//! Anbindung an Qt über cxx-qt: das QObject `RjBackend` liefert der
//! Oberfläche Baum, Arbeitsbereich und Sitzungsereignisse (als JSON) und
//! nimmt Befehle entgegen.

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use uuid::Uuid;

use crate::app::App;
use crate::workspace::Arrangement;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        type RjBackend = super::BackendRust;

        #[qinvokable]
        fn init(self: Pin<&mut RjBackend>, demo: bool);
        #[qinvokable]
        #[cxx_name = "isDemo"]
        fn is_demo(self: &RjBackend) -> bool;
        /// Auswahl und Werte der Darstellung (Farbschema, Fokusrahmen) als JSON.
        #[qinvokable]
        #[cxx_name = "appearanceJson"]
        fn appearance_json(self: &RjBackend) -> QString;
        /// Farbschema und/oder Fokusrahmen ändern (JSON), speichern; liefert `appearanceJson`.
        #[qinvokable]
        #[cxx_name = "setAppearance"]
        fn set_appearance(self: Pin<&mut RjBackend>, change: &QString) -> QString;
        /// Programmversion (aus `Cargo.toml`), z. B. `0.1.1`.
        #[qinvokable]
        #[cxx_name = "appVersion"]
        fn app_version(self: &RjBackend) -> QString;
        /// Ist eine Schlüsseldatenbank entsperrt (oder läuft die Vorführung)?
        /// Ungespeicherte Änderungen in der Schlüsseldatenbank (Index, Zugangsdaten, Import).
        /// Gemerkte Fensterlage (JSON) lesen bzw. beim Beenden speichern.
        #[qinvokable]
        #[cxx_name = "windowStateJson"]
        fn window_state_json(self: &RjBackend) -> QString;
        #[qinvokable]
        #[cxx_name = "saveWindowState"]
        fn save_window_state(self: Pin<&mut RjBackend>, state: &QString);
        #[qinvokable]
        #[cxx_name = "hasUnsavedChanges"]
        fn has_unsaved_changes(self: &RjBackend) -> bool;
        #[qinvokable]
        #[cxx_name = "isVaultOpen"]
        fn is_vault_open(self: &RjBackend) -> bool;
        #[qinvokable]
        #[cxx_name = "treeJson"]
        fn tree_json(self: &RjBackend, filter: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "propertiesJson"]
        fn properties_json(self: &RjBackend, id: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "connectTarget"]
        fn connect_target(self: Pin<&mut RjBackend>, id: &QString, credential: &QString, remember: bool, width: i32, height: i32) -> QString;
        #[qinvokable]
        fn poll(self: Pin<&mut RjBackend>) -> QString;
        #[qinvokable]
        #[cxx_name = "workspaceJson"]
        fn workspace_json(self: &RjBackend) -> QString;
        #[qinvokable]
        #[cxx_name = "tileSession"]
        fn tile_session(self: Pin<&mut RjBackend>, session: u64, tab: u64) -> bool;
        #[qinvokable]
        #[cxx_name = "setArrangement"]
        fn set_arrangement(self: Pin<&mut RjBackend>, tab: u64, mode: &QString) -> bool;
        #[qinvokable]
        #[cxx_name = "detachTab"]
        fn detach_tab(self: Pin<&mut RjBackend>, tab: u64) -> u64;
        #[qinvokable]
        #[cxx_name = "attachTab"]
        fn attach_tab(self: Pin<&mut RjBackend>, tab: u64) -> bool;
        #[qinvokable]
        #[cxx_name = "activateTab"]
        fn activate_tab(self: Pin<&mut RjBackend>, tab: u64);
        #[qinvokable]
        #[cxx_name = "closeSession"]
        fn close_session(self: Pin<&mut RjBackend>, session: u64);
        #[qinvokable]
        #[cxx_name = "detectFields"]
        fn detect_fields(self: Pin<&mut RjBackend>, id: &QString, url: &QString, ignore_certificate: bool) -> QString;
        #[qinvokable]
        #[cxx_name = "importPreview"]
        fn import_preview(self: Pin<&mut RjBackend>, path: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "importApply"]
        fn import_apply(self: Pin<&mut RjBackend>) -> QString;
        #[qinvokable]
        #[cxx_name = "startPatchday"]
        fn start_patchday(self: Pin<&mut RjBackend>, ids_json: &QString, execute: bool) -> QString;
        #[qinvokable]
        #[cxx_name = "patchdayJson"]
        fn patchday_json(self: &RjBackend) -> QString;
        #[qinvokable]
        #[cxx_name = "cancelPatchTarget"]
        fn cancel_patch_target(self: &RjBackend, id: &QString);
        #[qinvokable]
        #[cxx_name = "acceptHostKey"]
        fn accept_host_key(self: &RjBackend, host: &QString, port: u32, key: &QString) -> bool;
        #[qinvokable]
        #[cxx_name = "keyEvent"]
        fn key_event(self: Pin<&mut RjBackend>, session: u64, key: u32, modifiers: u32, text: &QString, down: bool);
        #[qinvokable]
        #[cxx_name = "mouseEvent"]
        fn mouse_event(self: Pin<&mut RjBackend>, session: u64, x: i32, y: i32, button: i32, down: bool);
        #[qinvokable]
        #[cxx_name = "resizeSession"]
        fn resize_session(self: Pin<&mut RjBackend>, session: u64, width: u32, height: u32);
        #[qinvokable]
        fn paste(self: Pin<&mut RjBackend>, session: u64, text: &QString);
        #[qinvokable]
        #[cxx_name = "offerClipboard"]
        fn offer_clipboard(self: Pin<&mut RjBackend>, session: u64, text: &QString);
        #[qinvokable]
        #[cxx_name = "clipboardReadAnswer"]
        fn clipboard_read_answer(self: Pin<&mut RjBackend>, session: u64, allow: bool, text: &QString);
        #[qinvokable]
        fn answer(self: Pin<&mut RjBackend>, session: u64, question: u32, accept: bool);
        /// Frist für Sicherheitsrückfragen in Sekunden.
        #[qinvokable]
        #[cxx_name = "questionTimeout"]
        fn question_timeout(self: &RjBackend) -> i32;
        #[qinvokable]
        #[cxx_name = "terminalSettings"]
        fn terminal_settings(self: &RjBackend) -> QString;
        #[qinvokable]
        #[cxx_name = "terminalSchemes"]
        fn terminal_schemes(self: &RjBackend) -> QString;
        #[qinvokable]
        #[cxx_name = "setTerminalSettings"]
        fn set_terminal_settings(self: Pin<&mut RjBackend>, change: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "terminalFontStep"]
        fn terminal_font_step(self: Pin<&mut RjBackend>, delta: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "sessionInfo"]
        fn session_info(self: &RjBackend, session: u64) -> QString;
        #[qinvokable]
        #[cxx_name = "terminalJson"]
        fn terminal_json(self: &RjBackend, session: u64) -> QString;
        /// Terminal zurück- (positiv) oder vorblättern, in Zeilen.
        #[qinvokable]
        #[cxx_name = "terminalScroll"]
        fn terminal_scroll(self: Pin<&mut RjBackend>, session: u64, lines: i32);
        #[qinvokable]
        #[cxx_name = "terminalScrollPage"]
        fn terminal_scroll_page(self: Pin<&mut RjBackend>, session: u64, back: bool);
        #[qinvokable]
        #[cxx_name = "selectText"]
        fn select_text(self: Pin<&mut RjBackend>, session: u64, r0: u32, c0: u32, r1: u32, c1: u32) -> QString;
        #[qinvokable]
        fn disconnect(self: Pin<&mut RjBackend>, session: u64);
        #[qinvokable]
        fn reconnect(self: Pin<&mut RjBackend>, session: u64);
        #[qinvokable]
        fn unlock(self: Pin<&mut RjBackend>, path: &QString, password: &QString, key_file: &QString);
        #[qinvokable]
        #[cxx_name = "createVault"]
        fn create_vault(self: Pin<&mut RjBackend>, path: &QString, password: &QString, key_file: &QString, make_key_file: bool);
        #[qinvokable]
        fn lock(self: Pin<&mut RjBackend>);
        /// Eingabe des Benutzers (Leerlauf-Zeitgeber).
        #[qinvokable]
        fn activity(self: Pin<&mut RjBackend>);
        /// Meldung des Systems: Standby beginnt (`true`) oder ist beendet (`false`).
        #[qinvokable]
        #[cxx_name = "systemSleep"]
        fn system_sleep(self: Pin<&mut RjBackend>, sleeping: bool) -> QString;
        #[qinvokable]
        fn save(self: Pin<&mut RjBackend>) -> QString;
        /// Bearbeitungsbefehl als JSON (siehe `editor.rs`).
        #[qinvokable]
        fn edit(self: Pin<&mut RjBackend>, cmd: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "kdbxPath"]
        fn kdbx_path(self: &RjBackend) -> QString;
        #[qinvokable]
        fn text(self: &RjBackend, key: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "paletteJson"]
        fn palette_json(self: &RjBackend) -> QString;
        #[qinvokable]
        #[cxx_name = "unassignedJson"]
        fn unassigned_json(self: &RjBackend) -> QString;
        #[qinvokable]
        #[cxx_name = "guestsJson"]
        fn guests_json(self: Pin<&mut RjBackend>, target: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "guestAction"]
        fn guest_action(self: Pin<&mut RjBackend>, target: &QString, guest: &QString, action: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "guestConsole"]
        fn guest_console(self: Pin<&mut RjBackend>, target: &QString, guest: &QString, title: &QString, console_host: &QString, width: i32, height: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "acceptProxmoxCertificate"]
        fn accept_proxmox_certificate(self: &RjBackend, host: &QString, fingerprint: &QString) -> bool;
    }

    unsafe extern "C++" {
        include!("app.h");
        #[cxx_name = "rjRunApplication"]
        fn run_application() -> i32;
    }
}

#[derive(Default)]
pub struct BackendRust {
    app: Option<App>,
}

fn q(s: impl AsRef<str>) -> QString {
    QString::from(s.as_ref())
}

fn uuid(s: &QString) -> Option<Uuid> {
    Uuid::parse_str(&s.to_string()).ok()
}

impl qobject::RjBackend {
    fn app(&self) -> Option<&App> {
        self.rust().app.as_ref()
    }

    fn app_mut(self: Pin<&mut Self>) -> Option<&mut App> {
        // SAFETY: `BackendRust` ist kein selbstreferenzierender Typ.
        unsafe { self.rust_mut().get_unchecked_mut() }.app.as_mut()
    }

    fn init(self: Pin<&mut Self>, demo: bool) {
        // SAFETY: siehe `app_mut`.
        let r = unsafe { self.rust_mut().get_unchecked_mut() };
        if r.app.is_none() {
            let mut app = App::new(demo);
            if demo {
                app.open_demo_sessions();
            }
            app.maybe_auto_check();
            r.app = Some(app);
        }
    }

    fn is_demo(&self) -> bool {
        self.app().is_some_and(|a| a.demo)
    }

    fn app_version(&self) -> QString {
        q(env!("CARGO_PKG_VERSION").to_string())
    }

    fn window_state_json(&self) -> QString {
        q(self.app().map(|a| a.window_state_json()).unwrap_or_else(|| "{}".into()))
    }

    fn save_window_state(self: Pin<&mut Self>, state: &QString) {
        let s = state.to_string();
        if let Some(a) = self.app_mut() {
            a.set_window_state(&s);
        }
    }

    fn has_unsaved_changes(&self) -> bool {
        self.app().is_some_and(|a| a.has_unsaved_changes())
    }

    fn is_vault_open(&self) -> bool {
        self.app().is_some_and(|a| a.demo || (a.vault.is_some() && !a.locked))
    }

    fn tree_json(&self, filter: &QString) -> QString {
        q(self.app().map(|a| a.tree_json(&filter.to_string())).unwrap_or_else(|| "[]".into()))
    }

    fn properties_json(&self, id: &QString) -> QString {
        q(match (self.app(), uuid(id)) {
            (Some(a), Some(u)) => a.properties_json(u),
            _ => "{}".into(),
        })
    }

    fn connect_target(self: Pin<&mut Self>, id: &QString, credential: &QString, remember: bool, width: i32, height: i32) -> QString {
        let cred = credential.to_string();
        let Some(u) = uuid(id) else { return q(r#"{"error":"Ziel ungültig"}"#) };
        q(match self.app_mut() {
            Some(a) => a.connect(u, (!cred.is_empty()).then_some(cred.as_str()), remember, width.max(1) as u32, height.max(1) as u32),
            None => r#"{"error":"nicht bereit"}"#.into(),
        })
    }

    fn poll(self: Pin<&mut Self>) -> QString {
        q(match self.app_mut() {
            Some(a) => serde_json::to_string(&a.poll()).unwrap_or_else(|_| "[]".into()),
            None => "[]".into(),
        })
    }

    fn terminal_settings(&self) -> QString {
        q(self.app().map(|a| a.terminal_settings_json()).unwrap_or_default())
    }

    fn terminal_schemes(&self) -> QString {
        q(crate::app::App::terminal_schemes_json())
    }

    fn set_terminal_settings(self: Pin<&mut Self>, change: &QString) -> QString {
        let c = change.to_string();
        q(self.app_mut().map(|a| a.set_terminal_settings(&c)).unwrap_or_default())
    }

    fn terminal_font_step(self: Pin<&mut Self>, delta: i32) -> QString {
        q(self.app_mut().map(|a| a.terminal_font_step(delta)).unwrap_or_default())
    }

    fn session_info(&self, session: u64) -> QString {
        q(self.app().and_then(|a| a.sessions.info(session)).map(|v| v.to_string()).unwrap_or_else(|| "{}".into()))
    }

    fn workspace_json(&self) -> QString {
        let Some(a) = self.app() else { return q("{}") };
        let sessions: serde_json::Map<String, serde_json::Value> = a
            .workspace
            .tabs
            .iter()
            .flat_map(|t| t.sessions.iter())
            .map(|s| {
                (
                    s.to_string(),
                    serde_json::json!({
                        "title": a.sessions.title(*s),
                        "protocol": a.sessions.protocol(*s).map(|p| p.as_str()),
                        "state": a.sessions.state(*s),
                    }),
                )
            })
            .collect();
        let tabs: Vec<_> = a
            .workspace
            .tabs
            .iter()
            .map(|t| {
                let mut v = serde_json::to_value(t).unwrap_or_default();
                v["cells"] = serde_json::to_value(crate::workspace::cells(t.arrangement, t.sessions.len())).unwrap_or_default();
                v
            })
            .collect();
        q(serde_json::json!({ "windows": a.workspace.windows, "tabs": tabs, "sessions": sessions }).to_string())
    }

    fn tile_session(self: Pin<&mut Self>, session: u64, tab: u64) -> bool {
        self.app_mut().is_some_and(|a| a.workspace.tile(session, tab))
    }

    fn set_arrangement(self: Pin<&mut Self>, tab: u64, mode: &QString) -> bool {
        let Some(m) = Arrangement::parse(&mode.to_string()) else { return false };
        self.app_mut().is_some_and(|a| a.workspace.set_arrangement(tab, m))
    }

    fn detach_tab(self: Pin<&mut Self>, tab: u64) -> u64 {
        self.app_mut().and_then(|a| a.workspace.detach(tab)).unwrap_or(0)
    }

    fn attach_tab(self: Pin<&mut Self>, tab: u64) -> bool {
        self.app_mut().is_some_and(|a| a.workspace.attach(tab))
    }

    fn activate_tab(self: Pin<&mut Self>, tab: u64) {
        if let Some(a) = self.app_mut() {
            a.workspace.activate(tab);
        }
    }

    fn close_session(self: Pin<&mut Self>, session: u64) {
        if let Some(a) = self.app_mut() {
            a.close_session(session);
        }
    }

    fn detect_fields(self: Pin<&mut Self>, id: &QString, url: &QString, ignore_certificate: bool) -> QString {
        let (id, url) = (id.to_string(), url.to_string());
        q(match (uuid::Uuid::parse_str(&id), self.app_mut()) {
            (Ok(t), Some(a)) => a.detect_fields(t, &url, ignore_certificate),
            _ => serde_json::json!({ "error": "Ziel nicht gefunden" }).to_string(),
        })
    }

    fn import_preview(self: Pin<&mut Self>, path: &QString) -> QString {
        let p = path.to_string();
        q(match self.app_mut().map(|a| a.import_preview(&p)) {
            Some(Ok((s, report))) => serde_json::json!({ "summary": s, "report": report }).to_string(),
            Some(Err(e)) => serde_json::json!({ "error": e }).to_string(),
            None => "{}".into(),
        })
    }

    fn import_apply(self: Pin<&mut Self>) -> QString {
        q(match self.app_mut().map(|a| a.import_apply()) {
            Some(Ok(s)) | Some(Err(s)) => s,
            None => String::new(),
        })
    }

    fn start_patchday(self: Pin<&mut Self>, ids_json: &QString, execute: bool) -> QString {
        let ids: Vec<Uuid> = serde_json::from_str::<Vec<String>>(&ids_json.to_string()).unwrap_or_default().iter().filter_map(|s| Uuid::parse_str(s).ok()).collect();
        q(match self.app_mut().map(|a| a.start_patchday(&ids, execute)) {
            Some(Ok(n)) => format!("{n} Ziele gestartet"),
            Some(Err(e)) => e,
            None => String::new(),
        })
    }

    fn patchday_json(&self) -> QString {
        q(self.app().map(|a| a.patchday_json()).unwrap_or_else(|| "[]".into()))
    }

    fn cancel_patch_target(&self, id: &QString) {
        if let (Some(a), Some(u)) = (self.app(), uuid(id)) {
            if let Some(p) = &a.patchday {
                p.cancel(u);
            }
        }
    }

    fn guests_json(self: Pin<&mut Self>, target: &QString) -> QString {
        let Some(id) = uuid(target) else { return q("{\"error\":\"Ziel ungültig\"}") };
        q(self.app_mut().map(|a| a.guests_json(id)).unwrap_or_else(|| "{\"error\":\"gesperrt\"}".into()))
    }

    fn guest_action(self: Pin<&mut Self>, target: &QString, guest: &QString, action: &QString) -> QString {
        let Some(id) = uuid(target) else { return q("{\"error\":\"Ziel ungültig\"}") };
        let (g, a) = (guest.to_string(), action.to_string());
        q(self.app_mut().map(|x| x.guest_action(id, &g, &a)).unwrap_or_else(|| "{\"error\":\"gesperrt\"}".into()))
    }

    fn guest_console(self: Pin<&mut Self>, target: &QString, guest: &QString, title: &QString, console_host: &QString, width: i32, height: i32) -> QString {
        let Some(id) = uuid(target) else { return q("{\"error\":\"Ziel ungültig\"}") };
        let (g, t, h) = (guest.to_string(), title.to_string(), console_host.to_string());
        q(self
            .app_mut()
            .map(|x| x.guest_console(id, &g, &t, &h, width.max(0) as u32, height.max(0) as u32))
            .unwrap_or_else(|| "{\"error\":\"gesperrt\"}".into()))
    }

    fn accept_proxmox_certificate(&self, host: &QString, fingerprint: &QString) -> bool {
        self.app().is_some_and(|a| a.accept_proxmox_certificate(&host.to_string(), &fingerprint.to_string()))
    }

    fn accept_host_key(&self, host: &QString, port: u32, key: &QString) -> bool {
        self.app().is_some_and(|a| a.accept_host_key(&host.to_string(), port as u16, &key.to_string()))
    }

    fn key_event(self: Pin<&mut Self>, session: u64, key: u32, modifiers: u32, text: &QString, down: bool) {
        let t = text.to_string();
        if let Some(a) = self.app_mut() {
            a.sessions.key(session, key, modifiers, &t, down);
        }
    }

    fn mouse_event(self: Pin<&mut Self>, session: u64, x: i32, y: i32, button: i32, down: bool) {
        if let Some(a) = self.app_mut() {
            a.mouse(session, x, y, button, down);
        }
    }

    fn resize_session(self: Pin<&mut Self>, session: u64, width: u32, height: u32) {
        if let Some(a) = self.app_mut() {
            a.sessions.resize(session, width, height);
        }
    }

    fn paste(self: Pin<&mut Self>, session: u64, text: &QString) {
        let t = text.to_string();
        if let Some(a) = self.app_mut() {
            a.sessions.paste(session, &t);
        }
    }

    fn offer_clipboard(self: Pin<&mut Self>, session: u64, text: &QString) {
        let t = text.to_string();
        if let Some(a) = self.app_mut() {
            a.sessions.offer_clipboard(session, &t);
        }
    }

    fn clipboard_read_answer(self: Pin<&mut Self>, session: u64, allow: bool, text: &QString) {
        let t = text.to_string();
        if let Some(a) = self.app_mut() {
            a.sessions.clipboard_read_answer(session, allow, &t);
        }
    }

    fn question_timeout(&self) -> i32 {
        self.app().map(|a| a.sessions.question_timeout.as_secs() as i32).unwrap_or(120)
    }

    fn answer(self: Pin<&mut Self>, session: u64, question: u32, accept: bool) {
        if let Some(a) = self.app_mut() {
            a.sessions.answer(session, question, accept);
        }
    }

    fn terminal_json(&self, session: u64) -> QString {
        q(self.app().and_then(|a| a.sessions.terminal(session)).map(|s| serde_json::to_string(&s).unwrap_or_default()).unwrap_or_default())
    }

    fn terminal_scroll(self: Pin<&mut Self>, session: u64, lines: i32) {
        if let Some(a) = self.app_mut() {
            a.sessions.scroll_terminal(session, lines);
        }
    }

    fn terminal_scroll_page(self: Pin<&mut Self>, session: u64, back: bool) {
        if let Some(a) = self.app_mut() {
            a.sessions.scroll_terminal_page(session, back);
        }
    }

    fn select_text(self: Pin<&mut Self>, session: u64, r0: u32, c0: u32, r1: u32, c1: u32) -> QString {
        q(self.app_mut().and_then(|a| a.sessions.select(session, (r0 as usize, c0 as usize), (r1 as usize, c1 as usize))).unwrap_or_default())
    }

    fn disconnect(self: Pin<&mut Self>, session: u64) {
        if let Some(a) = self.app_mut() {
            a.sessions.disconnect(session);
        }
    }

    fn reconnect(self: Pin<&mut Self>, session: u64) {
        if let Some(a) = self.app_mut() {
            a.reconnect_session(session);
        }
    }

    fn unlock(self: Pin<&mut Self>, path: &QString, password: &QString, key_file: &QString) {
        let (p, k) = (path.to_string(), key_file.to_string());
        let pw = rj_core::Secret::from_str_exact(&password.to_string());
        if let Some(a) = self.app_mut() {
            a.unlock(p, pw, (!k.is_empty()).then_some(k));
        }
    }

    fn create_vault(self: Pin<&mut Self>, path: &QString, password: &QString, key_file: &QString, make_key_file: bool) {
        let (p, k) = (path.to_string(), key_file.to_string());
        let pw = rj_core::Secret::from_str_exact(&password.to_string());
        if let Some(a) = self.app_mut() {
            a.create_vault(p, pw, (!k.is_empty()).then_some(k), make_key_file);
        }
    }

    fn lock(self: Pin<&mut Self>) {
        if let Some(a) = self.app_mut() {
            let d = a.config.security.disconnect_on_lock;
            a.lock(d);
        }
    }

    fn save(self: Pin<&mut Self>) -> QString {
        q(match self.app_mut().map(|a| a.save()) {
            Some(Ok(true)) => "Gespeichert".to_string(),
            Some(Ok(false)) => "Keine Änderungen".to_string(),
            Some(Err(e)) => format!("Speichern fehlgeschlagen: {e}"),
            None => String::new(),
        })
    }

    fn kdbx_path(&self) -> QString {
        q(self.app().and_then(|a| a.kdbx_path()).unwrap_or_default())
    }

    fn activity(self: Pin<&mut Self>) {
        if let Some(a) = self.app_mut() {
            a.activity();
        }
    }

    fn system_sleep(self: Pin<&mut Self>, sleeping: bool) -> QString {
        match self.app_mut() {
            Some(a) if !sleeping => q(serde_json::to_string(&a.on_resume()).unwrap_or_default()),
            _ => q("[]"),
        }
    }

    fn edit(self: Pin<&mut Self>, cmd: &QString) -> QString {
        let mut c = cmd.to_string();
        let r = self.app_mut().map(|a| a.edit(&c)).unwrap_or_else(|| r#"{"error":"nicht bereit"}"#.into());
        zeroize::Zeroize::zeroize(&mut c);
        q(r)
    }

    fn text(&self, key: &QString) -> QString {
        q(crate::texts::text(&key.to_string()))
    }

    fn palette_json(&self) -> QString {
        q(crate::appearance::palette_json(self.app()).to_string())
    }

    fn appearance_json(&self) -> QString {
        q(self.app().map(|a| a.appearance_json().to_string()).unwrap_or_else(|| "{}".into()))
    }

    fn set_appearance(self: Pin<&mut Self>, change: &QString) -> QString {
        let c: serde_json::Value = serde_json::from_str(&change.to_string()).unwrap_or_default();
        q(match self.app_mut().map(|a| a.set_appearance(&c)) {
            Some(Ok(v)) => v.to_string(),
            Some(Err(e)) => serde_json::json!({ "error": e }).to_string(),
            None => "{}".into(),
        })
    }

    fn unassigned_json(&self) -> QString {
        q(self.app().map(|a| a.unassigned_json()).unwrap_or_else(|| "[]".into()))
    }
}
