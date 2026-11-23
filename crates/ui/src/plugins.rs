//! Anschlussstellen des Kerns für Erweiterungen (Plugins). Der Kern kennt keine Hersteller und keine Fachfunktionen;
//! er fragt hier nach, und die angemeldeten Erweiterungen antworten über die Schnittstelle `rj_plugin_api`.
//! Eingebaute Erweiterungen (`modules::builtin`, Bauschalter) und geladene aus `<Datenordner>/plugins/<id>/`
//! (Bibliothek, `manifest.json` und signierte Prüfsummen; Signatur mit dem Update-Schlüssel, Schnittstelle und
//! Mindestversion laut Manifest, Ladereihenfolge nach Abhängigkeiten).

use std::sync::Arc;

use rj_core::{Folder, Target};
use serde_json::{Map, Value};
use uuid::Uuid;

pub use rj_plugin_api::{CliTicket, DeviceInfo, Found, Host, HostShared, Manifest, Plugin, VirtualChild};

use crate::app::App;

/// Übergang: das Programm hinter einem Host (solange Erweiterungen eingebaut sind und noch direkt zugreifen).
pub fn app(host: &mut dyn Host) -> &mut App {
    host.as_any_mut().downcast_mut::<App>().expect("Host ist die App")
}

pub fn app_ref(host: &dyn Host) -> &App {
    host.as_any().downcast_ref::<App>().expect("Host ist die App")
}

fn host(app: &mut App) -> &mut dyn Host {
    app
}

/// Name des Programms (Pakete, Dateien, Texte). Es gibt nur eine Fassung.
pub const APP_NAME: &str = "remotejoe";

/// Anzeigename für Fenstertitel und Über-Dialog: „remotejoe advanced“, sobald mindestens eine Erweiterung
/// freigeschaltet ist, sonst „remotejoe basic“ (Wunsch des Nutzers, 22.11.2026).
pub fn edition() -> String {
    if any_active() { format!("{APP_NAME} advanced") } else { format!("{APP_NAME} basic") }
}

/// Alle angemeldeten Erweiterungen (auch nicht freigeschaltete): eingebaute, dann geladene Bibliotheken (eine
/// Bibliothek mit der Kennung einer eingebauten Erweiterung wird übergangen).
pub fn registered() -> Vec<&'static dyn Plugin> {
    let mut v = crate::modules::builtin();
    for p in loaded().plugins.iter() {
        let id = p.info().id;
        if !v.iter().any(|b| b.info().id == id) {
            v.push(&**p);
        }
    }
    v
}

/// Stand einer Bibliothek im Ordner `plugins/` (geladen oder mit Grund abgelehnt) – für Lizenzseite und Über.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LibStatus {
    pub id: String,
    pub name: String,
    pub version: String,
    pub pfad: std::path::PathBuf,
    pub geladen: bool,
    /// Grund, wenn nicht geladen.
    pub fehler: Option<String>,
    #[serde(skip)]
    pub manifest: Option<Manifest>,
}

/// Ergebnis des Ladens aus dem Datenordner (einmal je Programmlauf).
#[derive(Default)]
pub struct LoadedLibs {
    pub plugins: Vec<Box<rj_plugin_api::ffi::host::RemotePlugin>>,
    /// Alle gefundenen Bibliotheken in Ladereihenfolge: geladene und abgelehnte (mit Grund).
    pub libs: Vec<LibStatus>,
    /// Einsprungpunkte der Oberflächen (`rj_plugin_ui`) der geladenen Bibliotheken.
    pub uis: Vec<usize>,
    /// Kennungen der Erweiterungen (`Plugin::info().id`) je geladener Bibliothek (Kennung der Bibliothek).
    pub owners: Vec<(String, String)>,
}

impl LoadedLibs {
    /// Gefundene, aber nicht geladene Bibliotheken (Pfad, Grund).
    pub fn errors(&self) -> Vec<(String, String)> {
        self.libs.iter().filter(|l| !l.geladen).map(|l| (l.pfad.display().to_string(), l.fehler.clone().unwrap_or_default())).collect()
    }
    pub fn is_loaded(&self, id: &str) -> bool {
        self.libs.iter().any(|l| l.geladen && l.id == id)
    }
    /// Manifest der geladenen Bibliothek, zu der die Erweiterung `plugin_id` gehört.
    pub fn manifest_of(&self, plugin_id: &str) -> Option<&Manifest> {
        let lib = self.owners.iter().find(|(p, _)| p == plugin_id).map(|(_, l)| l.as_str())?;
        self.libs.iter().find(|l| l.geladen && l.id == lib).and_then(|l| l.manifest.as_ref())
    }
}

/// Einsprungpunkte der Oberflächen als JSON-Liste (für den C++-Lader rjplugins.cpp).
pub fn ui_entries_json() -> String {
    serde_json::to_string(&loaded().uis).unwrap_or_else(|_| "[]".into())
}

/// Ordner der ladbaren Erweiterungen: `<Datenordner>/plugins/`.
pub fn plugins_dir() -> Option<std::path::PathBuf> {
    rj_core::config::data_dir().map(|d| d.join("plugins"))
}

/// Dateiname der Bibliothek einer Erweiterung auf dieser Plattform.
pub fn library_name(id: &str) -> String {
    if cfg!(windows) { format!("rj_{id}_plugin.dll") } else { format!("librj_{id}_plugin.so") }
}

pub fn loaded() -> &'static LoadedLibs {
    static L: std::sync::OnceLock<LoadedLibs> = std::sync::OnceLock::new();
    L.get_or_init(load_all)
}

/// Marke des Absturzwächters im Ordner der Erweiterungen: liegt sie beim Start noch, endete der letzte Start mit
/// Erweiterungen unerwartet (vor dem stabilen Lauf bzw. ohne reguläres Beenden).
pub const START_MARK: &str = "start.marke";

/// Warum dieser Start ohne Erweiterungen läuft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SafeMode {
    /// Der letzte Start mit Erweiterungen endete unerwartet; zuletzt geladen wurden diese (Kennungen).
    Crash(Vec<String>),
    /// Startparameter `--ohne-erweiterungen`.
    Requested,
}

static SAFE_MODE: std::sync::OnceLock<SafeMode> = std::sync::OnceLock::new();

/// Läuft dieser Start ohne Erweiterungen, und warum?
pub fn safe_mode() -> Option<&'static SafeMode> {
    SAFE_MODE.get()
}

/// Vor dem Laden: Ohne Erweiterungen starten (`Some`), wenn der Startparameter es verlangt oder die Marke des
/// letzten Starts noch liegt (die Marke wird dabei entfernt – der nächste Start versucht es wieder mit
/// Erweiterungen). Sonst die Marke mit den Kennungen setzen, die gleich geladen werden.
pub fn guard_before_load(dir: &std::path::Path, requested: bool) -> Option<SafeMode> {
    let mark = dir.join(START_MARK);
    let last: Option<Vec<String>> = std::fs::read_to_string(&mark).ok().map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect());
    if requested || last.is_some() {
        let _ = std::fs::remove_file(&mark);
        return Some(if requested { SafeMode::Requested } else { SafeMode::Crash(last.unwrap_or_default()) });
    }
    let ids: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .filter(|id| !id.ends_with(".alt") && (dir.join(id).join(library_name(id)).is_file() || id.strip_suffix(".neu").is_some_and(|b| dir.join(id).join(library_name(b)).is_file())))
                .map(|id| id.trim_end_matches(".neu").to_string())
                .collect()
        })
        .unwrap_or_default();
    if !ids.is_empty() {
        let mut ids = ids;
        ids.sort();
        ids.dedup();
        let _ = std::fs::write(&mark, ids.join("\n") + "\n");
    }
    None
}

/// Stabiler Start bzw. reguläres Beenden: Marke des Absturzwächters entfernen.
pub fn start_ok() {
    if let Some(dir) = plugins_dir() {
        let _ = std::fs::remove_file(dir.join(START_MARK));
    }
}

/// Alle Unterordner von `plugins/` prüfen und laden. Ohne gültige Signatur (Prüfsummen mit Update-Schlüssel) wird
/// keine Bibliothek geöffnet.
fn load_all() -> LoadedLibs {
    // Tests laden nie aus dem Datenordner des Nutzers (nur ausdrücklich über `load_all_with`).
    if cfg!(test) || std::env::var_os("REMOTEJOE_NO_PLUGINS").is_some() {
        return LoadedLibs::default();
    }
    // Absturzwächter und Startparameter `--ohne-erweiterungen` (nicht bei Vorführung und Bildschirmfotos).
    let guarded = std::env::var_os("RJ_DEMO").is_none() && std::env::var_os("RJ_SCREENSHOTS").is_none();
    if guarded {
        if let Some(dir) = plugins_dir() {
            if let Some(mode) = guard_before_load(&dir, std::env::var_os("RJ_NO_PLUGINS_ARG").is_some()) {
                let _ = SAFE_MODE.set(mode);
                return LoadedLibs::default();
            }
        }
    }
    // Nur Entwicklungsbauten: anderer Ordner und anderer öffentlicher Schlüssel (Datei) über die Umgebung, für
    // Probeläufe mit einer selbst signierten Bibliothek. Veröffentlichte Bauten kennen nur Datenordner und Update-Schlüssel.
    let (dir, key_text) = if cfg!(debug_assertions) {
        let d = std::env::var_os("REMOTEJOE_PLUGIN_DIR").map(std::path::PathBuf::from).or_else(plugins_dir);
        let k = std::env::var_os("REMOTEJOE_PLUGIN_KEY").and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_else(|| crate::update::UPDATE_KEY.to_string());
        (d, k)
    } else {
        (plugins_dir(), crate::update::UPDATE_KEY.to_string())
    };
    let Some(dir) = dir else { return LoadedLibs::default() };
    let key = match rj_update::sign::public_key_from_text(&key_text) {
        Ok(k) => k,
        Err(e) => {
            let st = LibStatus { id: String::new(), name: String::new(), version: String::new(), pfad: dir.clone(), geladen: false, fehler: Some(format!("Update-Schlüssel unlesbar: {e}")), manifest: None };
            return LoadedLibs { libs: vec![st], ..Default::default() };
        }
    };
    load_all_with(&dir, &key, rj_update::CURRENT_VERSION)
}

/// Manifest eines Erweiterungsordners lesen: Signatur der Prüfsummen, Prüfsumme der `manifest.json`, Inhalt.
fn read_manifest(key: &rj_update::sign::VerifyingKey, dir: &std::path::Path) -> Result<Manifest, String> {
    let sums = std::fs::read(dir.join(rj_update::sign::SUMS)).map_err(|e| format!("{}: {e}", rj_update::sign::SUMS))?;
    let sig = std::fs::read(dir.join(rj_update::sign::SIG)).map_err(|e| format!("{}: {e}", rj_update::sign::SIG))?;
    let text = std::fs::read(dir.join(rj_plugin_api::MANIFEST_FILE)).map_err(|e| format!("{}: {e}", rj_plugin_api::MANIFEST_FILE))?;
    rj_update::sign::verify_package(key, &sums, &sig, rj_plugin_api::MANIFEST_FILE, &text).map_err(|e| format!("Signatur oder Prüfsumme des Manifests: {e}"))?;
    Manifest::parse(&String::from_utf8_lossy(&text))
}

/// Prüft ein Manifest gegen diesen Kern: Schnittstelle und Mindestversion.
pub fn check_manifest(m: &Manifest, current: &str) -> Result<(), String> {
    if m.api != rj_plugin_api::API_VERSION {
        return Err(format!("Schnittstelle {}, der Kern hat {}", m.api, rj_plugin_api::API_VERSION));
    }
    let min = rj_update::Version::parse(&m.min_remotejoe).map_err(|_| format!("min_remotejoe „{}“ ist keine Version", m.min_remotejoe))?;
    let cur = rj_update::Version::parse(current).map_err(|_| format!("Programmversion „{current}“ unlesbar"))?;
    if cur < min {
        return Err(format!("braucht remotejoe {} oder neuer, dies ist {current}", m.min_remotejoe));
    }
    Ok(())
}

/// Verträglichkeit der Erweiterungen untereinander, ohne eine Bibliothek zu öffnen: Mindestversion aus `braucht`
/// („standorte>=1.2.0“) und Fassung jeder genutzten Anschlussstelle (`nutzt` „adapter@3“ gegen `stellt_bereit` des
/// Anbieters; ohne Angabe Fassung 1). Liefert die Erweiterungen, die nicht geladen werden dürfen, mit Grund – so
/// lädt eine alte Erweiterung nie gegen eine neue mit geändertem Aufbau.
pub fn check_compat(manifests: &[Manifest]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for m in manifests {
        let mut why = Vec::new();
        for b in &m.braucht {
            let (id, min) = rj_plugin_api::dependency(b);
            let (Some(min), Some(p)) = (min, manifests.iter().find(|x| x.id == id)) else { continue };
            let older = match (rj_update::Version::parse(&p.version), rj_update::Version::parse(min)) {
                (Ok(have), Ok(need)) => have < need,
                _ => true,
            };
            if older {
                why.push(format!("braucht {} {min} oder neuer, vorhanden ist {} – {} aktualisieren", p.name, p.version, p.name));
            }
        }
        for n in &m.nutzt {
            let (name, want) = rj_plugin_api::interface(n);
            for p in manifests.iter().filter(|p| p.id != m.id) {
                for (pn, have) in p.stellt_bereit.iter().map(|a| rj_plugin_api::interface(a)) {
                    if pn == name && have != want {
                        why.push(format!(
                            "{} {} passt nicht zu {} {} (Anschlussstelle „{name}“: Fassung {want}, angeboten wird {have}) – beide aktualisieren",
                            m.name, m.version, p.name, p.version
                        ));
                    }
                }
            }
        }
        if !why.is_empty() {
            out.push((m.id.clone(), why.join("; ")));
        }
    }
    out
}

/// Ladereihenfolge nach Abhängigkeiten: eine Erweiterung kommt erst, wenn alles, was sie braucht, angenommen ist.
/// Liefert die angenommenen Kennungen in Reihenfolge und die abgelehnten mit Grund (fehlende oder abgelehnte
/// Abhängigkeit, Zyklus).
pub fn resolve_order(manifests: &[Manifest]) -> (Vec<String>, Vec<(String, String)>) {
    let mut accepted: Vec<String> = Vec::new();
    let mut rejected: Vec<(String, String)> = Vec::new();
    let mut open: Vec<&Manifest> = manifests.iter().collect();
    loop {
        let before = open.len();
        let mut rest = Vec::new();
        for m in open {
            if m.braucht_ids().iter().all(|b| accepted.contains(b)) {
                accepted.push(m.id.clone());
            } else {
                rest.push(m);
            }
        }
        open = rest;
        if open.is_empty() || open.len() == before {
            break;
        }
    }
    for m in open {
        let missing: Vec<String> = m.braucht_ids().into_iter().filter(|b| !accepted.contains(b)).map(|b| {
            if manifests.iter().any(|x| x.id == b) { format!("„{b}“ (nicht geladen)") } else { format!("„{b}“ (nicht vorhanden)") }
        }).collect();
        rejected.push((m.id.clone(), format!("braucht {}", missing.join(", "))));
    }
    (accepted, rejected)
}

/// Alle Unterordner von `dir` mit dem Schlüssel `key` prüfen und laden (`current` = Programmversion für die
/// Mindestversion der Erweiterungen).
pub fn load_all_with(dir: &std::path::Path, key: &rj_update::sign::VerifyingKey, current: &str) -> LoadedLibs {
    let mut out = LoadedLibs::default();
    // Seit dem letzten Lauf geholte Pakete (`<id>.neu`) jetzt übernehmen, bevor etwas geladen ist.
    crate::plugin_fetch::swap_pending(dir);
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    let mut entries: Vec<_> = rd.flatten().filter(|e| e.path().is_dir()).collect();
    entries.sort_by_key(|e| e.file_name());
    // 1. Manifeste lesen und gegen den Kern prüfen (ohne eine Bibliothek zu öffnen).
    let mut candidates: Vec<(Manifest, std::path::PathBuf)> = Vec::new();
    for e in entries {
        let id = e.file_name().to_string_lossy().into_owned();
        if id.ends_with(".neu") || id.ends_with(".alt") {
            continue;
        }
        let sub = e.path();
        let lib = sub.join(library_name(&id));
        if !lib.is_file() {
            continue;
        }
        let r = read_manifest(key, &sub).and_then(|m| {
            if m.id != id {
                return Err(format!("Manifest nennt „{}“, der Ordner heißt „{id}“", m.id));
            }
            check_manifest(&m, current).map(|_| m)
        });
        match r {
            Ok(m) => candidates.push((m, lib)),
            Err(err) => {
                eprintln!("remotejoe: Erweiterung {} nicht geladen: {err}", lib.display());
                out.libs.push(LibStatus { id, name: String::new(), version: String::new(), pfad: lib, geladen: false, fehler: Some(err), manifest: None });
            }
        }
    }
    // 2. Reihenfolge nach Abhängigkeiten.
    let manifests: Vec<Manifest> = candidates.iter().map(|(m, _)| m.clone()).collect();
    let (order, rejected) = resolve_order(&manifests);
    for (id, why) in rejected {
        let (m, lib) = candidates.iter().find(|(m, _)| m.id == id).expect("abgelehnte Kennung stammt aus den Kandidaten");
        eprintln!("remotejoe: Erweiterung {} nicht geladen: {why}", lib.display());
        out.libs.push(LibStatus { id: id.clone(), name: m.name.clone(), version: m.version.clone(), pfad: lib.clone(), geladen: false, fehler: Some(why), manifest: Some(m.clone()) });
    }
    // Verträglichkeit untereinander (Mindestversionen, Fassungen der Anschlussstellen), ebenfalls vor dem Öffnen.
    let incompatible = check_compat(&manifests);
    // 3. Laden in Reihenfolge; fällt eine aus, fallen ihre Abhängigen mit.
    for id in order {
        let (m, lib) = candidates.iter().find(|(m, _)| m.id == id).expect("Kennung stammt aus den Kandidaten");
        let missing: Vec<String> = m.braucht_ids().into_iter().filter(|b| !out.is_loaded(b)).collect();
        let r = if let Some((_, why)) = incompatible.iter().find(|(x, _)| x == &id) {
            Err(why.clone())
        } else if missing.is_empty() {
            load_one(key, m, lib)
        } else { Err(format!("braucht {}", missing.iter().map(|b| format!("„{b}“ (nicht geladen)")).collect::<Vec<_>>().join(", "))) };
        match r {
            Ok(l) => {
                eprintln!("remotejoe: Erweiterung {id} {} geladen ({}; Oberfläche: {})", m.version, lib.display(), if l.ui.is_some() { "ja" } else { "nein" });
                out.libs.push(LibStatus { id: id.clone(), name: m.name.clone(), version: m.version.clone(), pfad: lib.clone(), geladen: true, fehler: None, manifest: Some(l.manifest) });
                out.uis.extend(l.ui);
                for p in &l.plugins {
                    out.owners.push((p.info().id, id.clone()));
                }
                out.plugins.extend(l.plugins);
            }
            Err(err) => {
                eprintln!("remotejoe: Erweiterung {} nicht geladen: {err}", lib.display());
                out.libs.push(LibStatus { id: id.clone(), name: m.name.clone(), version: m.version.clone(), pfad: lib.clone(), geladen: false, fehler: Some(err), manifest: Some(m.clone()) });
            }
        }
    }
    out
}

fn load_one(key: &rj_update::sign::VerifyingKey, m: &Manifest, lib: &std::path::Path) -> Result<rj_plugin_api::ffi::host::Loaded, String> {
    let dir = lib.parent().ok_or("Ordner fehlt")?;
    let sums = std::fs::read(dir.join(rj_update::sign::SUMS)).map_err(|e| format!("{}: {e}", rj_update::sign::SUMS))?;
    let sig = std::fs::read(dir.join(rj_update::sign::SIG)).map_err(|e| format!("{}: {e}", rj_update::sign::SIG))?;
    let data = std::fs::read(lib).map_err(|e| e.to_string())?;
    rj_update::sign::verify_package(key, &sums, &sig, &library_name(&m.id), &data).map_err(|e| format!("Signatur oder Prüfsumme: {e}"))?;
    // SAFETY: Signatur und Prüfsumme der Bibliothek sind geprüft; sie stammt aus der Quelle der Lizenz.
    unsafe { rj_plugin_api::ffi::host::load(lib, Some(m)) }
}

/// Manifeste aller angemeldeten Bibliotheken (eingebaute zuerst), nur geladene.
pub fn manifests() -> Vec<Manifest> {
    let mut v = crate::modules::builtin_manifests();
    v.extend(loaded().libs.iter().filter(|l| l.geladen).filter_map(|l| l.manifest.clone()));
    v
}

/// Manifest der Bibliothek, zu der die Erweiterung `plugin_id` gehört (eingebaut oder geladen).
fn manifest_of(plugin_id: &str) -> Option<Manifest> {
    // Eingebaute Erweiterungen: eine Bibliothek je Erweiterung, Kennung gleich der Modulkennung.
    if let Some(m) = crate::modules::builtin_manifests().into_iter().find(|m| m.id == plugin_id) {
        return Some(m);
    }
    loaded().manifest_of(plugin_id).cloned()
}

/// Ist die Erweiterung freigeschaltet? Mindestens ein Modul laut Lizenz bzw. Vorführung aktiv **und** alles, was
/// ihre Bibliothek braucht, selbst freigeschaltet (eine Hersteller-Erweiterung ohne lizenzierte Oberfunktion bleibt
/// aus). Abhängigkeitsketten sind beliebig tief; Zyklen verhindert der Lader.
fn plugin_active(p: &dyn Plugin) -> bool {
    plugin_active_depth(p, 0)
}

fn plugin_active_depth(p: &dyn Plugin, depth: usize) -> bool {
    let info = p.info();
    if depth > 16 || !info.modules.iter().any(|m| crate::license::modul_aktiv(&m.id)) {
        return false;
    }
    match manifest_of(&info.id) {
        Some(m) => m.braucht_ids().iter().all(|b| registered().into_iter().filter(|q| q.info().modules.iter().any(|x| &x.id == b)).any(|q| plugin_active_depth(q, depth + 1))),
        None => true,
    }
}

/// Abhängigkeiten einer Erweiterung (Kennung `plugin_id`), die nicht freigeschaltet sind – für die Lizenzseite
/// („fehlt Oberfunktion …“).
pub fn missing_dependencies(plugin_id: &str) -> Vec<String> {
    let Some(m) = manifest_of(plugin_id) else { return Vec::new() };
    m.braucht_ids().into_iter().filter(|b| !registered().into_iter().filter(|q| q.info().modules.iter().any(|x| &x.id == b)).any(|q| plugin_active(q))).collect()
}

/// Module aller Erweiterungen (Kennung, Name) für Lizenzseite und Über.
pub fn modules() -> Vec<rj_plugin_api::ModuleInfo> {
    registered().into_iter().flat_map(|p| p.info().modules).collect()
}

/// Freigeschaltete Erweiterungen (Lizenz bzw. Vorführung, Abhängigkeiten erfüllt). Die Freischaltung entscheidet
/// der Kern; die Erweiterung erfährt sie über `configure`.
pub fn all() -> Vec<&'static dyn Plugin> {
    registered().into_iter().filter(|p| plugin_active(*p)).collect()
}

/// Ist das Modul `id` freigeschaltet (lizenziert und seine Erweiterung mit allen Abhängigkeiten aktiv)?
pub fn module_active(id: &str) -> bool {
    crate::license::modul_aktiv(id) && all().into_iter().any(|p| p.info().modules.iter().any(|m| m.id == id))
}

/// Operation einer anderen Erweiterung über den Kern als Vermittler (`to` = Kennung der Erweiterung oder eines
/// ihrer Module).
pub fn call_other(app: &mut App, to: &str, op: &str, c: &Value) -> Result<Value, String> {
    let Some(p) = all().into_iter().find(|p| { let i = p.info(); i.id == to || i.modules.iter().any(|m| m.id == to) }) else {
        return Err(format!("Erweiterung „{to}“ ist nicht geladen oder nicht freigeschaltet"));
    };
    let h = host(app);
    p.handle(h, op, c).unwrap_or_else(|| Err(format!("Erweiterung „{to}“ kennt die Operation „{op}“ nicht")))
}

/// Kennungen der freigeschalteten Erweiterungen, die die Anschlussstelle `anschluss` anbieten (Ladereihenfolge).
pub fn providers(anschluss: &str) -> Vec<String> {
    let active: Vec<String> = all().into_iter().map(|p| p.info().id).collect();
    let mut v: Vec<String> = manifests().into_iter().filter(|m| m.stellt_bereit.iter().any(|a| rj_plugin_api::interface(a).0 == anschluss)).map(|m| m.id).filter(|id| active.contains(id) || loaded().owners.iter().any(|(p, l)| l == id && active.contains(p))).collect();
    v.dedup();
    v
}

pub fn any_active() -> bool {
    !all().is_empty()
}

/// Freischaltung an die Erweiterungen melden (nach Start, Lizenz- und Vorführungswechsel).
pub fn configure(demo: bool) {
    for p in registered() {
        let on = plugin_active(p);
        let active: Vec<String> = p.info().modules.into_iter().map(|m| m.id).filter(|id| on && crate::license::modul_aktiv(id)).collect();
        p.configure(&active, demo);
    }
}

/// Nach `configure`: Erweiterungen in Ladereihenfolge anbinden (`Plugin::attach`), z. B. Hersteller-Erweiterungen
/// bei ihrer Oberfunktion. Fehler landen auf der Lizenzseite (`attach_errors`).
pub fn attach(app: &mut App) {
    let mut errors = Vec::new();
    for p in all() {
        let id = p.info().id;
        if let Err(e) = p.attach(app) {
            eprintln!("remotejoe: Erweiterung {id} nicht angebunden: {e}");
            errors.push((id, e));
        }
    }
    *ATTACH_ERRORS.lock().unwrap_or_else(|e| e.into_inner()) = errors;
}

static ATTACH_ERRORS: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

/// Fehler beim Anbinden (Kennung, Grund) – für die Lizenzseite.
pub fn attach_errors() -> Vec<(String, String)> {
    ATTACH_ERRORS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Beim Sperren: Geheimnisse der Erweiterungen verwerfen.
pub fn wipe() {
    for p in registered() {
        p.wipe();
    }
}

pub fn dispatch(app: &mut App, op: &str, c: &Value) -> Option<Result<Value, String>> {
    let h = host(app);
    all().into_iter().find_map(|p| p.handle(h, op, c))
}

pub fn settings_json(app: &mut App, m: &mut Map<String, Value>) {
    let h = host(app);
    for p in all() {
        p.settings_json(h, m);
    }
}

pub fn set_settings(app: &mut App, c: &Value) {
    let h = host(app);
    for p in all() {
        p.set_settings(h, c);
    }
}

pub fn folder_marked(f: &Folder) -> bool {
    all().into_iter().any(|p| p.folder_marked(f))
}

pub fn folder_virtual_children(f: &Folder) -> Vec<VirtualChild> {
    all().into_iter().flat_map(|p| p.folder_virtual_children(f)).collect()
}

pub fn folder_editor_json(app: &mut App, f: &Folder, v: &mut Value) {
    let h = host(app);
    for p in all() {
        p.folder_editor_json(h, f, v);
    }
}

pub fn folder_update(app: &mut App, id: Uuid, form: &Value) -> Result<(), String> {
    let h = host(app);
    for p in all() {
        p.folder_update(h, id, form)?;
    }
    Ok(())
}

pub fn target_saving(t: &mut Target) {
    for p in all() {
        p.target_saving(t);
    }
}

pub fn overview_extra(app: &mut App, key: &str) -> Map<String, Value> {
    let h = host(app);
    let mut m = Map::new();
    for p in all() {
        m.extend(p.overview_extra(h, key));
    }
    m
}

pub fn login_kinds() -> Vec<(&'static str, &'static str, bool)> {
    registered().into_iter().flat_map(|p| p.login_kinds()).collect()
}

pub fn device_info(system: &str) -> Option<DeviceInfo> {
    all().into_iter().find_map(|p| p.device_info(system))
}

pub fn texts() -> Vec<&'static str> {
    registered().into_iter().flat_map(|p| p.texts()).collect()
}

pub fn mcp_tools() -> Vec<Value> {
    all().into_iter().flat_map(|p| p.mcp_tools()).collect()
}

pub fn mcp_actions() -> Vec<(&'static str, &'static str)> {
    all().into_iter().flat_map(|p| p.mcp_actions()).collect()
}

pub fn mcp_call(app: &mut App, tool: &str, args: &Value) -> Option<Result<Value, String>> {
    let h = host(app);
    all().into_iter().find_map(|p| p.mcp_call(h, tool, args))
}

pub fn add_kinds() -> Vec<&'static str> {
    all().into_iter().flat_map(|p| p.add_kinds()).collect()
}

pub fn add_target(app: &mut App, kind: &str, parent: Uuid, c: &Value) -> Option<Result<Value, String>> {
    let h = host(app);
    all().into_iter().find_map(|p| p.add_target(h, kind, parent, c))
}

pub fn after_inventory(app: &mut App, id: Uuid) -> bool {
    let h = host(app);
    let mut changed = false;
    for p in all() {
        changed |= p.after_inventory(h, id);
    }
    changed
}

/// Inventar über eine offene SSH-Verbindung des Kerns: die Erweiterungen bekommen sie nacheinander (als Schein),
/// die speziellste zuerst (Hersteller vor Oberfunktion, also umgekehrte Ladereihenfolge), bis eine antwortet;
/// danach trennt der Kern. `shared`/`cli` kommen aus dem Faden der Aufgabe.
pub fn ssh_inventory(shared: &Arc<dyn HostShared>, cli: &crate::cli_service::CliService, open: crate::cli_service::OpenSsh, ip: Option<&str>) -> Result<(Found, &'static str, Option<String>), String> {
    let plugins = all();
    if plugins.is_empty() {
        open.rt.block_on(open.conn.disconnect());
        return Err("Keine verwertbare Antwort über SSH".into());
    }
    let t = cli.issue_open(open);
    let mut r = Err("Keine verwertbare Antwort über SSH".to_string());
    for p in plugins.into_iter().rev() {
        match p.ssh_inventory(shared, t, ip) {
            Some(Ok(x)) => {
                r = Ok(x);
                break;
            }
            Some(Err(e)) => r = Err(e),
            None => {}
        }
    }
    if let Some(o) = cli.take_open(t) {
        o.rt.block_on(o.conn.disconnect());
    }
    r
}

/// Inventar eines Switches über seine Befehlszeile (Sammelinventur): Schein aus Zugangsdaten des Kerns.
pub fn switch_inventory(shared: &Arc<dyn HostShared>, cli: &crate::cli_service::CliService, spec: &crate::patch::JobSpec, port: u16, known: &rj_ssh::known_hosts::KnownHosts) -> Result<Found, String> {
    let spec = crate::patch::JobSpec { protocol: spec.protocol, host: spec.host.clone(), port, credentials: spec.credentials.clone(), transport: spec.transport.clone(), legacy: spec.legacy };
    let t = cli.issue(spec, known.clone(), None);
    let r = all().into_iter().find_map(|p| p.switch_inventory(shared, t)).unwrap_or_else(|| Err("Keine verwertbare Antwort über SSH".into()));
    cli.revoke(t);
    r
}

pub fn inventory_query(app: &mut App, t: &Target, key: &str) -> Option<Result<Value, String>> {
    let h = host(app);
    all().into_iter().find_map(|p| p.inventory_query(h, t, key))
}

pub fn port_label(port: u16) -> Option<&'static str> {
    all().into_iter().find_map(|p| p.port_label(port))
}

#[cfg(test)]
mod lib_tests {
    use super::*;

    fn mf(id: &str, braucht: &[&str]) -> Manifest {
        Manifest { id: id.into(), name: id.to_uppercase(), version: "1.0.0".into(), api: rj_plugin_api::API_VERSION, min_remotejoe: "0.1.60".into(), braucht: braucht.iter().map(|b| b.to_string()).collect(), ..Default::default() }
    }

    /// Signierter Erweiterungsordner: Bibliothek (Bytes), Manifest, Prüfsummen, Signatur.
    fn write_dir(dir: &std::path::Path, m: &Manifest, lib_bytes: &[u8], sk: &rj_update::sign::SigningKey) {
        std::fs::create_dir_all(dir).unwrap();
        let lib = library_name(&m.id);
        std::fs::write(dir.join(&lib), lib_bytes).unwrap();
        let mtext = serde_json::to_string_pretty(m).unwrap();
        std::fs::write(dir.join(rj_plugin_api::MANIFEST_FILE), &mtext).unwrap();
        let sums = rj_update::sign::sums_text(&[(lib, rj_update::sign::sha256_hex(lib_bytes)), (rj_plugin_api::MANIFEST_FILE.into(), rj_update::sign::sha256_hex(mtext.as_bytes()))]);
        std::fs::write(dir.join(rj_update::sign::SUMS), &sums).unwrap();
        std::fs::write(dir.join(rj_update::sign::SIG), rj_update::sign::sign(sk, sums.as_bytes())).unwrap();
    }

    #[test]
    fn manifest_checks_api_and_minimum_version() {
        let m = mf("a", &[]);
        assert!(check_manifest(&m, "0.1.60").is_ok());
        assert!(check_manifest(&m, "0.2.0").is_ok());
        assert!(check_manifest(&m, "0.1.59").unwrap_err().contains("0.1.60 oder neuer"));
        let mut old = m.clone();
        old.api = 99;
        assert!(check_manifest(&old, "0.1.60").unwrap_err().contains("Schnittstelle"));
        assert!(Manifest::parse(r#"{"id":"Gross","name":"x","version":"1","api":1,"min_remotejoe":"0.1.60"}"#).unwrap_err().contains("Kennung"));
        assert!(Manifest::parse(r#"{"id":"a","name":"x","version":"1","api":1,"min_remotejoe":"0.1.60","braucht":["a"]}"#).unwrap_err().contains("sich selbst"));
    }

    /// Reihenfolge: Oberfunktion vor Kind vor Enkel; fehlende oder zyklische Abhängigkeiten werden mit Grund
    /// abgelehnt, der Rest bleibt unberührt.
    #[test]
    fn order_follows_dependencies_and_rejects_missing_or_cyclic_ones() {
        let (ok, bad) = resolve_order(&[mf("enkel", &["zweig"]), mf("kind", &["basis"]), mf("zweig", &["basis"]), mf("basis", &[]), mf("eigen", &[])]);
        assert_eq!(ok, ["basis", "eigen", "kind", "zweig", "enkel"]);
        assert!(bad.is_empty());
        let (ok, bad) = resolve_order(&[mf("kind", &["basis"]), mf("x", &["y"]), mf("y", &["x"]), mf("eigen", &[])]);
        assert_eq!(ok, ["eigen"]);
        assert_eq!(bad.len(), 3);
        assert!(bad.iter().any(|(id, why)| id == "kind" && why.contains("„basis“ (nicht vorhanden)")), "{bad:?}");
        assert!(bad.iter().any(|(id, why)| id == "x" && why.contains("„y“ (nicht geladen)")), "{bad:?}");
    }

    /// Ordner ohne ladbare Bibliothek: Manifest, Signatur, Kennung und Mindestversion werden vor dem Öffnen
    /// geprüft; Abgelehntes steht mit Grund in `libs`.
    #[test]
    fn folders_are_checked_before_any_library_is_opened() {
        let sk = rj_update::sign::generate().unwrap();
        let base = tempfile::tempdir().unwrap();
        // Zu neu für diesen Kern.
        let mut neu = mf("neu", &[]);
        neu.min_remotejoe = "99.0.0".into();
        write_dir(&base.path().join("neu"), &neu, b"x", &sk);
        // Ordnername passt nicht zur Kennung (Bibliothek trägt den Ordnernamen, das Manifest einen anderen).
        write_dir(&base.path().join("falsch"), &mf("anders", &[]), b"x", &sk);
        std::fs::rename(base.path().join("falsch").join(library_name("anders")), base.path().join("falsch").join(library_name("falsch"))).unwrap();
        // Braucht etwas, das es nicht gibt.
        write_dir(&base.path().join("kind"), &mf("kind", &["eltern"]), b"x", &sk);
        // Ohne Manifest.
        let ohne = base.path().join("ohne");
        std::fs::create_dir_all(&ohne).unwrap();
        std::fs::write(ohne.join(library_name("ohne")), b"x").unwrap();
        let l = load_all_with(base.path(), &sk.verifying_key(), "0.1.60");
        assert!(l.plugins.is_empty());
        let why = |id: &str| l.libs.iter().find(|x| x.id == id).map(|x| x.fehler.clone().unwrap_or_default()).unwrap_or_else(|| format!("{id} fehlt in {:?}", l.libs));
        assert!(why("neu").contains("99.0.0 oder neuer"), "{}", why("neu"));
        assert!(why("falsch").contains("Ordner heißt"), "{}", why("falsch"));
        assert!(why("kind").contains("„eltern“ (nicht vorhanden)"), "{}", why("kind"));
        assert!(why("ohne").contains(rj_update::sign::SUMS), "{}", why("ohne"));
        assert_eq!(l.errors().len(), 4);
    }


    fn mfv(id: &str, name: &str, version: &str, braucht: &[&str], bereit: &[&str], nutzt: &[&str]) -> Manifest {
        let v = |l: &[&str]| l.iter().map(|x| x.to_string()).collect();
        Manifest { name: name.into(), version: version.into(), braucht: v(braucht), stellt_bereit: v(bereit), nutzt: v(nutzt), ..mf(id, &[]) }
    }

    /// Fassungen der Anschlussstellen und Mindestversionen: der Fall vom 23.11. (alte Hersteller-Erweiterung ohne
    /// Fassungsangabe neben einer Oberfunktion mit neuer Fassung) wird abgelehnt, ein passendes Paar nicht.
    #[test]
    fn compat_rejects_old_users_of_a_newer_interface() {
        let basis3 = mfv("basis", "Basis", "1.3.0", &[], &["adapter@3"], &[]);
        let alt = mfv("netz", "Netz", "1.0.0", &["basis"], &["controller"], &["adapter"]);
        let neu = mfv("netz", "Netz", "1.3.0", &["basis>=1.3.0"], &["controller"], &["adapter@3"]);
        let bad = check_compat(&[basis3.clone(), alt]);
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0].0, "netz");
        assert!(bad[0].1.contains("Netz 1.0.0 passt nicht zu Basis 1.3.0") && bad[0].1.contains("Fassung 1, angeboten wird 3") && bad[0].1.contains("beide aktualisieren"), "{}", bad[0].1);
        assert!(check_compat(&[basis3.clone(), neu.clone()]).is_empty());
        // Beide ohne Angabe (heutiger Stand): Fassung 1 zu Fassung 1 passt.
        assert!(check_compat(&[mfv("basis", "Basis", "1.2.2", &[], &["adapter"], &[]), mfv("netz", "Netz", "1.0.0", &["basis"], &[], &["adapter"])]).is_empty());
        // Mindestversion nicht erfüllt.
        let basis_alt = mfv("basis", "Basis", "1.2.9", &[], &["adapter@3"], &[]);
        let bad = check_compat(&[basis_alt, neu]);
        assert!(bad[0].1.contains("braucht Basis 1.3.0 oder neuer, vorhanden ist 1.2.9"), "{:?}", bad);
        // Anbieter fehlt: Sache von `braucht`, nicht der Fassungsprüfung.
        assert!(check_compat(&[mfv("netz", "Netz", "1.3.0", &["basis>=1.3.0"], &[], &["adapter@3"])]).is_empty());
        // Angaben im Manifest.
        assert_eq!(rj_plugin_api::interface("adapter@3"), ("adapter", 3));
        assert_eq!(rj_plugin_api::interface("adapter"), ("adapter", 1));
        assert_eq!(rj_plugin_api::dependency("basis>=1.3.0"), ("basis", Some("1.3.0")));
        assert!(Manifest::parse(r#"{"id":"a","name":"x","version":"1","api":1,"min_remotejoe":"0.1.60","nutzt":["adapter@x"]}"#).unwrap_err().contains("unlesbar"));
        assert!(Manifest::parse(r#"{"id":"a","name":"x","version":"1","api":1,"min_remotejoe":"0.1.60","braucht":["a>=1.0.0"]}"#).unwrap_err().contains("sich selbst"));
    }

    /// Beim Laden: Unverträgliches wird vor dem Öffnen der Bibliothek abgelehnt (mit Grund in `libs`), Abhängige
    /// fallen mit; die Anbieter-Suche kennt Anschlussstellen auch mit Fassungsangabe.
    #[test]
    fn incompatible_extensions_are_not_opened() {
        let sk = rj_update::sign::generate().unwrap();
        let base = tempfile::tempdir().unwrap();
        // Bibliotheken sind keine echten: geöffnet werden darf nur „basis“ (scheitert dann beim Öffnen, nicht hier).
        write_dir(&base.path().join("basis"), &mfv("basis", "Basis", "1.3.0", &[], &["adapter@3"], &[]), b"x", &sk);
        write_dir(&base.path().join("netz"), &mfv("netz", "Netz", "1.0.0", &["basis"], &[], &["adapter"]), b"x", &sk);
        write_dir(&base.path().join("kind"), &mf("kind", &["netz"]), b"x", &sk);
        let l = load_all_with(base.path(), &sk.verifying_key(), "0.1.60");
        let why = |id: &str| l.libs.iter().find(|x| x.id == id).and_then(|x| x.fehler.clone()).unwrap_or_default();
        assert!(why("netz").contains("passt nicht zu Basis 1.3.0"), "{}", why("netz"));
        assert!(why("kind").contains("„netz“ (nicht geladen)"), "{}", why("kind"));
        assert!(!why("basis").contains("passt nicht"), "{}", why("basis"));
    }


    /// Absturzwächter: Marke wird vor dem Laden gesetzt; liegt sie beim nächsten Start noch, läuft er ohne
    /// Erweiterungen (Marke weg, der übernächste versucht es wieder); Startparameter ebenso; `start_ok`-Weg entfernt sie.
    #[test]
    fn crash_guard_marks_and_falls_back_once() {
        let base = tempfile::tempdir().unwrap();
        let d = base.path();
        for id in ["basis", "netz"] {
            std::fs::create_dir_all(d.join(id)).unwrap();
            std::fs::write(d.join(id).join(library_name(id)), b"x").unwrap();
        }
        std::fs::create_dir_all(d.join("neu.neu")).unwrap();
        std::fs::write(d.join("neu.neu").join(library_name("neu")), b"x").unwrap();
        std::fs::create_dir_all(d.join("leer")).unwrap();
        // Erster Start: Marke mit allem, was gleich geladen wird (auch wartende Pakete).
        assert_eq!(guard_before_load(d, false), None);
        assert_eq!(std::fs::read_to_string(d.join(START_MARK)).unwrap(), "basis\nnetz\nneu\n");
        // Absturz (Marke bleibt): nächster Start ohne Erweiterungen, Marke entfernt.
        assert_eq!(guard_before_load(d, false), Some(SafeMode::Crash(vec!["basis".into(), "netz".into(), "neu".into()])));
        assert!(!d.join(START_MARK).exists());
        // Übernächster Start: wieder mit Erweiterungen.
        assert_eq!(guard_before_load(d, false), None);
        assert!(d.join(START_MARK).exists());
        // Stabiler Start bzw. reguläres Beenden: Marke weg (wie `start_ok`).
        std::fs::remove_file(d.join(START_MARK)).unwrap();
        assert_eq!(guard_before_load(d, true), Some(SafeMode::Requested), "Startparameter");
        assert!(!d.join(START_MARK).exists());
        // Ohne Erweiterungen auf der Platte keine Marke.
        let leer = tempfile::tempdir().unwrap();
        assert_eq!(guard_before_load(leer.path(), false), None);
        assert!(!leer.path().join(START_MARK).exists());
    }

}
