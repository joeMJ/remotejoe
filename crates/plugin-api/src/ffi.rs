//! C-ABI zwischen dem Kern und einer Erweiterungs-Bibliothek. Alles läuft als JSON-Zeichenketten über wenige
//! `extern "C"`-Funktionen, damit Kern und Bibliothek mit verschiedenen Übersetzern (GitHub-Bau des Kerns, eigener
//! Bau der Erweiterung) zusammenpassen; weder Rust-Typen noch Trait-Objekte überqueren die Grenze.
//!
//! Bibliothek (Erweiterung): `export_plugins!(fn() -> Vec<&'static dyn Plugin>)` legt die Einsprungpunkte an
//! (`rj_plugin_api_version`, `rj_plugin_manifest`, `rj_plugin_count`, `rj_plugin_call`, `rj_plugin_free`).
//! Kern: `host::load` (Feature `host`) öffnet die Bibliothek und liefert je Erweiterung ein `dyn Plugin`, das jede
//! Methode über `rj_plugin_call` weiterreicht; der Kern selbst steht der Bibliothek als `RjHostVt`/`RjSharedVt`
//! (Funktionszeiger mit Kontext) zur Verfügung.
//!
//! Zeichenketten: UTF-8 mit Nullbyte. Wer eine Zeichenkette anlegt, gibt sie frei (`free` der jeweiligen Seite).
//! Ergebnisse: `{"ok": …}`, `{"err": "…"}`, für `Option<Result<…>>` zusätzlich `null` = nicht zuständig.

use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::Arc;

use serde_json::{json, Map, Value};

use crate::{Host, HostShared, Plugin};

/// Zugang zum Kern im Faden der Oberfläche (gültig während eines Aufrufs von `rj_plugin_call`).
#[repr(C)]
pub struct RjHostVt {
    pub ctx: *mut c_void,
    /// Operation des Kerns (`Host::call`): Ergebnis als JSON, freizugeben mit `free`.
    pub call: unsafe extern "C" fn(ctx: *mut c_void, op: *const c_char, args: *const c_char) -> *mut c_char,
    pub free: unsafe extern "C" fn(s: *mut c_char),
    /// Nebenläufiger Zugang (`Host::shared`), eigener Besitz (mit `drop` freigeben).
    pub shared: unsafe extern "C" fn(ctx: *mut c_void) -> RjSharedVt,
}

/// Nebenläufiger Zugang zum Kern (`HostShared`), aus beliebigen Fäden; Besitz zählt (`clone`/`drop`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RjSharedVt {
    pub ctx: *mut c_void,
    pub call: unsafe extern "C" fn(ctx: *mut c_void, op: *const c_char, args: *const c_char) -> *mut c_char,
    pub free: unsafe extern "C" fn(s: *mut c_char),
    pub clone: unsafe extern "C" fn(ctx: *mut c_void) -> *mut c_void,
    pub drop: unsafe extern "C" fn(ctx: *mut c_void),
}

// --- Hilfen -------------------------------------------------------------------------------------------------------

/// JSON als C-Zeichenkette (Besitz an den Aufrufer).
pub fn to_c(v: &Value) -> *mut c_char {
    CString::new(v.to_string()).unwrap_or_default().into_raw()
}

/// # Safety
/// `p` muss von `to_c` dieser Bibliothek stammen oder null sein.
pub unsafe extern "C" fn free_c(p: *mut c_char) {
    if !p.is_null() {
        drop(CString::from_raw(p));
    }
}

/// # Safety
/// `p` muss null oder eine gültige, nullterminierte Zeichenkette sein.
unsafe fn from_c(p: *const c_char) -> Value {
    if p.is_null() {
        return Value::Null;
    }
    serde_json::from_slice(CStr::from_ptr(p).to_bytes()).unwrap_or(Value::Null)
}

fn ok(v: Value) -> Value {
    json!({ "ok": v })
}

fn err(e: impl ToString) -> Value {
    json!({ "err": e.to_string() })
}

fn res(r: Result<Value, String>) -> Value {
    match r {
        Ok(v) => ok(v),
        Err(e) => err(e),
    }
}

fn opt(r: Option<Result<Value, String>>) -> Value {
    r.map(res).unwrap_or(Value::Null)
}

/// `{"ok":…}` / `{"err":…}` zurück in ein Ergebnis.
fn unres(v: Value) -> Result<Value, String> {
    match v {
        Value::Object(mut m) => {
            if let Some(e) = m.remove("err") {
                return Err(e.as_str().map(str::to_string).unwrap_or_else(|| e.to_string()));
            }
            Ok(m.remove("ok").unwrap_or(Value::Null))
        }
        _ => Err("Antwort ohne Ergebnis".into()),
    }
}

#[cfg_attr(not(feature = "host"), allow(dead_code))]
fn unopt(v: Value) -> Option<Result<Value, String>> {
    if v.is_null() {
        None
    } else {
        Some(unres(v))
    }
}

fn de<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| e.to_string())
}

/// Einmal angelegte, nie freigegebene Zeichenketten (für `&'static str` in Rückgaben der Schnittstelle).
#[cfg_attr(not(feature = "host"), allow(dead_code))]
fn intern(s: &str) -> &'static str {
    use std::collections::HashSet;
    use std::sync::Mutex;
    static POOL: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut g = POOL.lock().unwrap_or_else(|e| e.into_inner());
    let set = g.get_or_insert_with(HashSet::new);
    if let Some(x) = set.get(s) {
        return x;
    }
    let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
    set.insert(leaked);
    leaked
}

// --- Seite der Erweiterung (Bibliothek) ---------------------------------------------------------------------------

/// Der Kern aus Sicht der Bibliothek, nur während eines Aufrufs gültig.
pub struct FfiHost {
    vt: *const RjHostVt,
    index: std::cell::OnceCell<rj_core::Index>,
}

/// Operationen, nach denen der zwischengespeicherte Index weiter gilt.
const READ_ONLY_OPS: &[&str] = &[
    "index", "target", "folder", "node_path", "settings_extra", "settings_core", "data_dir", "text", "demo", "vault_open", "read_only",
    "device", "inventory", "inventory_web_way", "services_ip", "cli_ticket", "folder_has_slot", "snmp_ticket", "busy", "busy_set",
    "busy_clear", "cfgstore_state", "light", "redact", "mcp_target", "login_kinds", "edition", "tree_changed",
];

impl FfiHost {
    /// # Safety
    /// `vt` muss für die Lebensdauer des `FfiHost` gültig bleiben.
    pub unsafe fn new(vt: *const RjHostVt) -> Self {
        FfiHost { vt, index: std::cell::OnceCell::new() }
    }

    fn raw(&self, op: &str, args: Value) -> Result<Value, String> {
        let vt = unsafe { &*self.vt };
        let op_c = CString::new(op).map_err(|e| e.to_string())?;
        let args_c = CString::new(args.to_string()).map_err(|e| e.to_string())?;
        let p = unsafe { (vt.call)(vt.ctx, op_c.as_ptr(), args_c.as_ptr()) };
        if p.is_null() {
            return Err(format!("{op}: keine Antwort des Kerns"));
        }
        let v = unsafe { from_c(p) };
        unsafe { (vt.free)(p) };
        unres(v)
    }
}

impl Host for FfiHost {
    fn call(&mut self, op: &str, args: Value) -> Result<Value, String> {
        if !READ_ONLY_OPS.contains(&op) {
            self.index.take();
        }
        self.raw(op, args)
    }

    fn shared(&self) -> Arc<dyn HostShared> {
        let vt = unsafe { &*self.vt };
        Arc::new(FfiShared { vt: unsafe { (vt.shared)(vt.ctx) } })
    }

    fn index_ref(&self) -> &rj_core::Index {
        self.index.get_or_init(|| self.raw("index", Value::Null).ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default())
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Nebenläufiger Zugang zum Kern aus der Bibliothek.
pub struct FfiShared {
    vt: RjSharedVt,
}

// SAFETY: Der Kern stellt `call`, `clone` und `drop` fadensicher bereit (sie sprechen einen `Arc<dyn HostShared>` an).
unsafe impl Send for FfiShared {}
unsafe impl Sync for FfiShared {}

impl Drop for FfiShared {
    fn drop(&mut self) {
        unsafe { (self.vt.drop)(self.vt.ctx) }
    }
}

impl HostShared for FfiShared {
    fn call(&self, op: &str, args: Value) -> Result<Value, String> {
        let op_c = CString::new(op).map_err(|e| e.to_string())?;
        let args_c = CString::new(args.to_string()).map_err(|e| e.to_string())?;
        let p = unsafe { (self.vt.call)(self.vt.ctx, op_c.as_ptr(), args_c.as_ptr()) };
        if p.is_null() {
            return Err(format!("{op}: keine Antwort des Kerns"));
        }
        let v = unsafe { from_c(p) };
        unsafe { (self.vt.free)(p) };
        unres(v)
    }
}

/// Eine Methode der Erweiterung über ihren Namen aufrufen (Seite der Bibliothek). `host`/`shared`: je nach
/// Methode vorhanden.
pub fn plugin_call(p: &dyn Plugin, host: Option<&mut dyn Host>, shared: Option<Arc<dyn HostShared>>, op: &str, a: Value) -> Value {
    let no_host = || err(format!("{op}: Aufruf ohne Host"));
    let no_shared = || err(format!("{op}: Aufruf ohne nebenläufigen Zugang"));
    let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let id = |k: &str| a.get(k).and_then(Value::as_str).and_then(|x| uuid::Uuid::parse_str(x).ok());
    match op {
        "info" => ok(serde_json::to_value(p.info()).unwrap_or(Value::Null)),
        "configure" => {
            let active: Vec<String> = a.get("active").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
            p.configure(&active, a.get("demo").and_then(Value::as_bool).unwrap_or(false));
            ok(Value::Null)
        }
        "attach" => match host {
            Some(h) => match p.attach(h) {
                Ok(()) => ok(Value::Null),
                Err(e) => err(e),
            },
            None => no_host(),
        },
        "active_modules" => ok(json!(p.active_modules())),
        "handle" => match host {
            Some(h) => opt(p.handle(h, &s("op"), a.get("c").unwrap_or(&Value::Null))),
            None => no_host(),
        },
        "texts" => ok(json!(p.texts())),
        "settings_json" => match host {
            Some(h) => {
                let mut m: Map<String, Value> = a.get("m").and_then(Value::as_object).cloned().unwrap_or_default();
                p.settings_json(h, &mut m);
                ok(Value::Object(m))
            }
            None => no_host(),
        },
        "set_settings" => match host {
            Some(h) => {
                p.set_settings(h, a.get("c").unwrap_or(&Value::Null));
                ok(Value::Null)
            }
            None => no_host(),
        },
        "folder_marked" => match de::<rj_core::model::Folder>(a.get("f").cloned().unwrap_or(Value::Null)) {
            Ok(f) => ok(json!(p.folder_marked(&f))),
            Err(e) => err(e),
        },
        "folder_virtual_children" => match de::<rj_core::model::Folder>(a.get("f").cloned().unwrap_or(Value::Null)) {
            Ok(f) => ok(json!(p.folder_virtual_children(&f).into_iter().map(|c| json!({ "id": c.id, "name": c.name })).collect::<Vec<_>>())),
            Err(e) => err(e),
        },
        "folder_editor_json" => match (host, de::<rj_core::model::Folder>(a.get("f").cloned().unwrap_or(Value::Null))) {
            (Some(h), Ok(f)) => {
                let mut v = a.get("v").cloned().unwrap_or(Value::Null);
                p.folder_editor_json(h, &f, &mut v);
                ok(v)
            }
            (None, _) => no_host(),
            (_, Err(e)) => err(e),
        },
        "folder_update" => match (host, id("id")) {
            (Some(h), Some(i)) => res(p.folder_update(h, i, a.get("form").unwrap_or(&Value::Null)).map(|_| Value::Null)),
            (None, _) => no_host(),
            _ => err("Kennung fehlt"),
        },
        "target_saving" => match de::<rj_core::model::Target>(a.get("t").cloned().unwrap_or(Value::Null)) {
            Ok(mut t) => {
                p.target_saving(&mut t);
                ok(serde_json::to_value(t).unwrap_or(Value::Null))
            }
            Err(e) => err(e),
        },
        "overview_extra" => match host {
            Some(h) => ok(Value::Object(p.overview_extra(h, &s("key")))),
            None => no_host(),
        },
        "login_kinds" => ok(json!(p.login_kinds())),
        "device_info" => ok(p.device_info(&s("system")).map(|d| serde_json::to_value(d).unwrap_or(Value::Null)).unwrap_or(Value::Null)),
        "mcp_tools" => ok(json!(p.mcp_tools())),
        "mcp_actions" => ok(json!(p.mcp_actions())),
        "mcp_call" => match host {
            Some(h) => opt(p.mcp_call(h, &s("tool"), a.get("args").unwrap_or(&Value::Null))),
            None => no_host(),
        },
        "add_kinds" => ok(json!(p.add_kinds())),
        "add_target" => match (host, id("parent")) {
            (Some(h), Some(parent)) => opt(p.add_target(h, &s("kind"), parent, a.get("c").unwrap_or(&Value::Null))),
            (None, _) => no_host(),
            _ => err("Ordner fehlt"),
        },
        "after_inventory" => match (host, id("id")) {
            (Some(h), Some(i)) => ok(json!(p.after_inventory(h, i))),
            (None, _) => no_host(),
            _ => err("Kennung fehlt"),
        },
        "ssh_inventory" => match (shared, de::<crate::CliTicket>(a.get("ticket").cloned().unwrap_or(Value::Null))) {
            (Some(sh), Ok(t)) => {
                let ip = a.get("ip").and_then(Value::as_str).map(str::to_string);
                opt(p.ssh_inventory(&sh, t, ip.as_deref()).map(|r| r.map(|(found, kind, extra)| json!({ "found": found, "kind": kind, "extra": extra }))))
            }
            (None, _) => no_shared(),
            (_, Err(e)) => err(e),
        },
        "switch_inventory" => match (shared, de::<crate::CliTicket>(a.get("ticket").cloned().unwrap_or(Value::Null))) {
            (Some(sh), Ok(t)) => opt(p.switch_inventory(&sh, t).map(|r| r.map(|found| json!(found)))),
            (None, _) => no_shared(),
            (_, Err(e)) => err(e),
        },
        "inventory_query" => match (host, de::<rj_core::model::Target>(a.get("target").cloned().unwrap_or(Value::Null))) {
            (Some(h), Ok(t)) => opt(p.inventory_query(h, &t, &s("key"))),
            (None, _) => no_host(),
            (_, Err(e)) => err(e),
        },
        "port_label" => ok(json!(p.port_label(a.get("port").and_then(Value::as_u64).unwrap_or(0) as u16))),
        "wipe" => {
            p.wipe();
            ok(Value::Null)
        }
        _ => err(format!("Unbekannte Methode der Schnittstelle: {op}")),
    }
}

/// Einsprungpunkte einer Erweiterungs-Bibliothek. `$plugins`: `fn() -> Vec<&'static dyn Plugin>`;
/// `$manifest`: Text der `manifest.json` (z. B. `include_str!("../manifest.json")`), dieselbe Datei liegt im Paket.
#[macro_export]
macro_rules! export_plugins {
    ($plugins:path, $manifest:expr) => {
        fn __rj_plugins() -> &'static Vec<&'static dyn $crate::Plugin> {
            static P: ::std::sync::OnceLock<Vec<&'static dyn $crate::Plugin>> = ::std::sync::OnceLock::new();
            P.get_or_init($plugins)
        }

        /// Fassung der Schnittstelle, gegen die diese Bibliothek gebaut ist.
        #[no_mangle]
        pub extern "C" fn rj_plugin_api_version() -> u32 {
            $crate::API_VERSION
        }

        /// Manifest der Bibliothek als JSON (Kennung, Name, eigene Version, api, min_remotejoe, braucht, …);
        /// der Kern vergleicht es mit der `manifest.json` des Pakets.
        #[no_mangle]
        pub extern "C" fn rj_plugin_manifest() -> *const ::std::ffi::c_char {
            static M: ::std::sync::OnceLock<::std::ffi::CString> = ::std::sync::OnceLock::new();
            M.get_or_init(|| ::std::ffi::CString::new($manifest).unwrap_or_default()).as_ptr()
        }

        #[no_mangle]
        pub extern "C" fn rj_plugin_count() -> u32 {
            __rj_plugins().len() as u32
        }

        /// Methode `op` der Erweiterung `idx` mit JSON-Argumenten; Ergebnis als JSON (freizugeben mit
        /// `rj_plugin_free`), null bei Panik.
        ///
        /// # Safety
        /// `host` und `shared` dürfen null sein; sonst gültig für die Dauer des Aufrufs.
        #[no_mangle]
        pub unsafe extern "C" fn rj_plugin_call(
            idx: u32,
            host: *const $crate::ffi::RjHostVt,
            shared: *const $crate::ffi::RjSharedVt,
            op: *const ::std::ffi::c_char,
            args: *const ::std::ffi::c_char,
        ) -> *mut ::std::ffi::c_char {
            $crate::ffi::library_call(__rj_plugins(), idx, host, shared, op, args)
        }

        /// # Safety
        /// `p` muss von `rj_plugin_call` stammen oder null sein.
        #[no_mangle]
        pub unsafe extern "C" fn rj_plugin_free(p: *mut ::std::ffi::c_char) {
            $crate::ffi::free_c(p)
        }
    };
}

/// Kern von `rj_plugin_call` (Seite der Bibliothek).
///
/// # Safety
/// Siehe `rj_plugin_call`.
pub unsafe fn library_call(plugins: &[&'static dyn Plugin], idx: u32, host: *const RjHostVt, shared: *const RjSharedVt, op: *const c_char, args: *const c_char) -> *mut c_char {
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let Some(p) = plugins.get(idx as usize) else { return err("Unbekannte Erweiterung") };
        let op = if op.is_null() { String::new() } else { CStr::from_ptr(op).to_string_lossy().into_owned() };
        let a = from_c(args);
        let mut h = (!host.is_null()).then(|| FfiHost::new(host));
        let sh: Option<Arc<dyn HostShared>> = if shared.is_null() {
            None
        } else {
            let vt = *shared;
            Some(Arc::new(FfiShared { vt: RjSharedVt { ctx: (vt.clone)(vt.ctx), ..vt } }))
        };
        plugin_call(*p, h.as_mut().map(|x| x as &mut dyn Host), sh, &op, a)
    }));
    match r {
        Ok(v) => to_c(&v),
        Err(_) => std::ptr::null_mut(),
    }
}

// --- Seite des Kerns ---------------------------------------------------------------------------------------------

#[cfg(feature = "host")]
pub mod host {
    //! Der Kern öffnet eine Bibliothek und spricht ihre Erweiterungen als `dyn Plugin` an.
    use super::*;
    use crate::{CliTicket, DeviceInfo, Found, PluginInfo, VirtualChild, API_VERSION};

    type HostCtx = *mut dyn Host;

    unsafe extern "C" fn host_call(ctx: *mut c_void, op: *const c_char, args: *const c_char) -> *mut c_char {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let h: &mut dyn Host = &mut **(ctx as *mut HostCtx);
            let op = CStr::from_ptr(op).to_string_lossy().into_owned();
            res(h.call(&op, from_c(args)))
        }));
        match r {
            Ok(v) => to_c(&v),
            Err(_) => to_c(&err("Panik im Kern")),
        }
    }

    unsafe extern "C" fn host_shared(ctx: *mut c_void) -> RjSharedVt {
        let h: &mut dyn Host = &mut **(ctx as *mut HostCtx);
        shared_vt(h.shared())
    }

    unsafe extern "C" fn shared_call(ctx: *mut c_void, op: *const c_char, args: *const c_char) -> *mut c_char {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let s: &Arc<dyn HostShared> = &*(ctx as *const Arc<dyn HostShared>);
            let op = CStr::from_ptr(op).to_string_lossy().into_owned();
            res(s.call(&op, from_c(args)))
        }));
        match r {
            Ok(v) => to_c(&v),
            Err(_) => to_c(&err("Panik im Kern")),
        }
    }

    unsafe extern "C" fn shared_clone(ctx: *mut c_void) -> *mut c_void {
        let s: &Arc<dyn HostShared> = &*(ctx as *const Arc<dyn HostShared>);
        Box::into_raw(Box::new(s.clone())) as *mut c_void
    }

    unsafe extern "C" fn shared_drop(ctx: *mut c_void) {
        drop(Box::from_raw(ctx as *mut Arc<dyn HostShared>));
    }

    fn shared_vt(s: Arc<dyn HostShared>) -> RjSharedVt {
        RjSharedVt { ctx: Box::into_raw(Box::new(s)) as *mut c_void, call: shared_call, free: free_c, clone: shared_clone, drop: shared_drop }
    }

    type CallFn = unsafe extern "C" fn(u32, *const RjHostVt, *const RjSharedVt, *const c_char, *const c_char) -> *mut c_char;
    type FreeFn = unsafe extern "C" fn(*mut c_char);

    /// Eine Erweiterung aus einer Bibliothek; jede Methode läuft über `rj_plugin_call`.
    pub struct RemotePlugin {
        /// Nie entladen (Fäden und Fadenlokales der Bibliothek könnten noch laufen).
        _lib: &'static libloading::Library,
        call: CallFn,
        free: FreeFn,
        idx: u32,
        info: PluginInfo,
        texts: Vec<&'static str>,
    }

    // SAFETY: Die Bibliothek verlangt von ihren Einsprungpunkten Fadensicherheit (wie `Plugin: Send + Sync`).
    unsafe impl Send for RemotePlugin {}
    unsafe impl Sync for RemotePlugin {}

    impl RemotePlugin {
        fn raw(&self, host: Option<&mut dyn Host>, shared: Option<&Arc<dyn HostShared>>, op: &str, args: Value) -> Value {
            let op_c = CString::new(op).unwrap_or_default();
            let args_c = CString::new(args.to_string()).unwrap_or_default();
            // SAFETY: Der Zeiger lebt nur für diesen Aufruf; die Bibliothek darf ihn nicht behalten (Vertrag der Schnittstelle).
            let mut fat: Option<HostCtx> = host.map(|h| unsafe { std::mem::transmute::<*mut (dyn Host + '_), HostCtx>(h as *mut (dyn Host + '_)) });
            let vt = fat.as_mut().map(|f| RjHostVt { ctx: f as *mut HostCtx as *mut c_void, call: host_call, free: free_c, shared: host_shared });
            let svt = shared.map(|s| shared_vt(s.clone()));
            let p = unsafe {
                (self.call)(
                    self.idx,
                    vt.as_ref().map_or(std::ptr::null(), |v| v as *const RjHostVt),
                    svt.as_ref().map_or(std::ptr::null(), |v| v as *const RjSharedVt),
                    op_c.as_ptr(),
                    args_c.as_ptr(),
                )
            };
            if let Some(s) = svt {
                unsafe { (s.drop)(s.ctx) };
            }
            if p.is_null() {
                return err(format!("{op}: Panik in der Erweiterung „{}“", self.info.id));
            }
            let v = unsafe { from_c(p) };
            unsafe { (self.free)(p) };
            v
        }

        fn get(&self, host: Option<&mut dyn Host>, op: &str, args: Value) -> Value {
            unres(self.raw(host, None, op, args)).unwrap_or(Value::Null)
        }

        fn get_de<T: serde::de::DeserializeOwned + Default>(&self, host: Option<&mut dyn Host>, op: &str, args: Value) -> T {
            serde_json::from_value(self.get(host, op, args)).unwrap_or_default()
        }
    }

    impl Plugin for RemotePlugin {
        fn info(&self) -> PluginInfo {
            self.info.clone()
        }
        fn configure(&self, active: &[String], demo: bool) {
            self.get(None, "configure", json!({ "active": active, "demo": demo }));
        }
        fn attach(&self, host: &mut dyn Host) -> Result<(), String> {
            unres(self.raw(Some(host), None, "attach", Value::Null)).map(|_| ())
        }
        fn active_modules(&self) -> Vec<String> {
            self.get_de(None, "active_modules", Value::Null)
        }
        fn handle(&self, host: &mut dyn Host, op: &str, c: &Value) -> Option<Result<Value, String>> {
            unopt(self.raw(Some(host), None, "handle", json!({ "op": op, "c": c })))
        }
        fn texts(&self) -> Vec<&'static str> {
            self.texts.clone()
        }
        fn settings_json(&self, host: &mut dyn Host, m: &mut Map<String, Value>) {
            if let Value::Object(r) = self.get(Some(host), "settings_json", json!({ "m": Value::Object(m.clone()) })) {
                *m = r;
            }
        }
        fn set_settings(&self, host: &mut dyn Host, c: &Value) {
            self.get(Some(host), "set_settings", json!({ "c": c }));
        }
        fn folder_marked(&self, f: &rj_core::model::Folder) -> bool {
            self.get(None, "folder_marked", json!({ "f": f })).as_bool().unwrap_or(false)
        }
        fn folder_virtual_children(&self, f: &rj_core::model::Folder) -> Vec<VirtualChild> {
            let v = self.get(None, "folder_virtual_children", json!({ "f": f }));
            v.as_array().map(|a| a.iter().filter_map(|c| Some(VirtualChild { id: uuid::Uuid::parse_str(c.get("id")?.as_str()?).ok()?, name: c.get("name")?.as_str()?.to_string() })).collect()).unwrap_or_default()
        }
        fn folder_editor_json(&self, host: &mut dyn Host, f: &rj_core::model::Folder, v: &mut Value) {
            let r = self.get(Some(host), "folder_editor_json", json!({ "f": f, "v": &*v }));
            if !r.is_null() {
                *v = r;
            }
        }
        fn folder_update(&self, host: &mut dyn Host, id: uuid::Uuid, form: &Value) -> Result<(), String> {
            unres(self.raw(Some(host), None, "folder_update", json!({ "id": id, "form": form }))).map(|_| ())
        }
        fn target_saving(&self, t: &mut rj_core::model::Target) {
            if let Ok(n) = serde_json::from_value::<rj_core::model::Target>(self.get(None, "target_saving", json!({ "t": &*t }))) {
                *t = n;
            }
        }
        fn overview_extra(&self, host: &mut dyn Host, key: &str) -> Map<String, Value> {
            self.get(Some(host), "overview_extra", json!({ "key": key })).as_object().cloned().unwrap_or_default()
        }
        fn login_kinds(&self) -> Vec<(&'static str, &'static str, bool)> {
            let v: Vec<(String, String, bool)> = self.get_de(None, "login_kinds", Value::Null);
            v.into_iter().map(|(k, l, n)| (intern(&k), intern(&l), n)).collect()
        }
        fn device_info(&self, system: &str) -> Option<DeviceInfo> {
            serde_json::from_value(self.get(None, "device_info", json!({ "system": system }))).ok()
        }
        fn mcp_tools(&self) -> Vec<Value> {
            self.get_de(None, "mcp_tools", Value::Null)
        }
        fn mcp_actions(&self) -> Vec<(&'static str, &'static str)> {
            let v: Vec<(String, String)> = self.get_de(None, "mcp_actions", Value::Null);
            v.into_iter().map(|(a, b)| (intern(&a), intern(&b))).collect()
        }
        fn mcp_call(&self, host: &mut dyn Host, tool: &str, args: &Value) -> Option<Result<Value, String>> {
            unopt(self.raw(Some(host), None, "mcp_call", json!({ "tool": tool, "args": args })))
        }
        fn add_kinds(&self) -> Vec<&'static str> {
            let v: Vec<String> = self.get_de(None, "add_kinds", Value::Null);
            v.iter().map(|k| intern(k)).collect()
        }
        fn add_target(&self, host: &mut dyn Host, kind: &str, parent: uuid::Uuid, c: &Value) -> Option<Result<Value, String>> {
            unopt(self.raw(Some(host), None, "add_target", json!({ "kind": kind, "parent": parent, "c": c })))
        }
        fn after_inventory(&self, host: &mut dyn Host, id: uuid::Uuid) -> bool {
            self.get(Some(host), "after_inventory", json!({ "id": id })).as_bool().unwrap_or(false)
        }
        fn ssh_inventory(&self, shared: &Arc<dyn HostShared>, ticket: CliTicket, ip: Option<&str>) -> Option<Result<(Found, &'static str, Option<String>), String>> {
            let r = unopt(self.raw(None, Some(shared), "ssh_inventory", json!({ "ticket": ticket, "ip": ip })))?;
            Some(r.and_then(|v| {
                let found: Vec<(String, String)> = serde_json::from_value(v.get("found").cloned().unwrap_or(Value::Null)).map_err(|e| e.to_string())?;
                let kind = intern(v.get("kind").and_then(Value::as_str).unwrap_or_default());
                let extra = v.get("extra").and_then(Value::as_str).map(str::to_string);
                Ok((found.into_iter().map(|(k, x)| (intern(&k), x)).collect(), kind, extra))
            }))
        }
        fn switch_inventory(&self, shared: &Arc<dyn HostShared>, ticket: CliTicket) -> Option<Result<Found, String>> {
            let r = unopt(self.raw(None, Some(shared), "switch_inventory", json!({ "ticket": ticket })))?;
            Some(r.and_then(|v| {
                let found: Vec<(String, String)> = serde_json::from_value(v).map_err(|e| e.to_string())?;
                Ok(found.into_iter().map(|(k, x)| (intern(&k), x)).collect())
            }))
        }
        fn inventory_query(&self, host: &mut dyn Host, target: &rj_core::model::Target, key: &str) -> Option<Result<Value, String>> {
            unopt(self.raw(Some(host), None, "inventory_query", json!({ "target": target, "key": key })))
        }
        fn port_label(&self, port: u16) -> Option<&'static str> {
            self.get(None, "port_label", json!({ "port": port })).as_str().map(intern)
        }
        fn wipe(&self) {
            self.get(None, "wipe", Value::Null);
        }
    }

    /// Angaben einer Bibliothek vor dem Laden ihrer Erweiterungen.
    pub struct Loaded {
        /// Manifest aus der Bibliothek selbst (`rj_plugin_manifest`).
        pub manifest: crate::Manifest,
        pub plugins: Vec<Box<RemotePlugin>>,
        /// Adresse des Einsprungpunkts der Oberfläche (`rj_plugin_ui`), falls die Bibliothek eine hat.
        pub ui: Option<usize>,
    }

    /// Bibliothek öffnen und ihre Erweiterungen anbinden. Prüft die Fassung der Schnittstelle und, wenn `expect`
    /// gegeben ist, dass Kennung und Version des eingebauten Manifests zur `manifest.json` des Pakets passen.
    /// Mindestversion und Abhängigkeiten prüft der Kern vorher anhand der Datei. Die Bibliothek bleibt bis zum
    /// Programmende geladen.
    ///
    /// # Safety
    /// Lädt fremden Code in den Prozess: nur nach geprüfter Signatur aufrufen.
    pub unsafe fn load(path: &std::path::Path, expect: Option<&crate::Manifest>) -> Result<Loaded, String> {
        let lib = libloading::Library::new(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let api: libloading::Symbol<unsafe extern "C" fn() -> u32> = lib.get(b"rj_plugin_api_version\0").map_err(|e| e.to_string())?;
        let api = api();
        if api != API_VERSION {
            return Err(format!("{}: Schnittstelle {api}, der Kern erwartet {API_VERSION}", path.display()));
        }
        let mf: libloading::Symbol<unsafe extern "C" fn() -> *const c_char> = lib.get(b"rj_plugin_manifest\0").map_err(|e| e.to_string())?;
        let manifest = crate::Manifest::parse(&CStr::from_ptr(mf()).to_string_lossy()).map_err(|e| format!("{}: {e}", path.display()))?;
        if let Some(e) = expect {
            if e.id != manifest.id || e.version != manifest.version {
                return Err(format!("{}: Bibliothek meldet {} {}, das Paket nennt {} {}", path.display(), manifest.id, manifest.version, e.id, e.version));
            }
        }
        let count: libloading::Symbol<unsafe extern "C" fn() -> u32> = lib.get(b"rj_plugin_count\0").map_err(|e| e.to_string())?;
        let count = count();
        let call: libloading::Symbol<CallFn> = lib.get(b"rj_plugin_call\0").map_err(|e| e.to_string())?;
        let free: libloading::Symbol<FreeFn> = lib.get(b"rj_plugin_free\0").map_err(|e| e.to_string())?;
        let (call, free) = (*call, *free);
        let ui = lib.get::<unsafe extern "C" fn(*mut c_void, i32) -> *mut c_void>(b"rj_plugin_ui\0").ok().map(|s| *s as usize);
        let lib: &'static libloading::Library = Box::leak(Box::new(lib));
        let mut plugins = Vec::new();
        for idx in 0..count {
            let mut p = RemotePlugin { _lib: lib, call, free, idx, info: PluginInfo { id: String::new(), modules: Vec::new(), api }, texts: Vec::new() };
            p.info = serde_json::from_value(p.get(None, "info", Value::Null)).map_err(|e| format!("{}: Angaben der Erweiterung {idx}: {e}", path.display()))?;
            let texts: Vec<String> = p.get_de(None, "texts", Value::Null);
            p.texts = texts.into_iter().map(|t| Box::leak(t.into_boxed_str()) as &'static str).collect();
            plugins.push(Box::new(p));
        }
        Ok(Loaded { manifest, plugins, ui })
    }
}
