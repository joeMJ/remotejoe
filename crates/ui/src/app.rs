//! Zustand der Anwendung hinter der Oberfläche: Konfiguration, Tresor
//! (KDBX mit Index), Sitzungen und Arbeitsbereich.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use rj_core::config::Config;
use rj_core::inherit::{resolve_credential, resolve_credential_named, CredentialResolution, ResolvedCredential};
use rj_core::{Index, Node, Protocol, ProtocolOptions, Secret, Target};
use rj_ipc::{ConnectRequest, CredentialPayload, MouseButton, SecretBytes};
use rj_kdbx::{MasterKey, Vault};
use serde_json::json;
use uuid::Uuid;

use rj_ssh::client::{Auth, SshParams};
use rj_ssh::known_hosts::{HostKeyQuestion, KnownHosts};
use rj_tunnel::{TunnelManager, TunnelSpec};

use crate::sessions::{ConnectSpec, Manager, Reconnect, UiEvent};
use crate::tree::{self, Light};
use crate::workspace::{Arrangement, SessionId, Workspace};

/// Unsichtbare Web-Sitzung, die die Felder einer Anmeldeseite erfasst.
struct FieldProbe {
    s: SessionId,
    target: Uuid,
    started: std::time::Instant,
    saw_loading: bool,
    /// Nächster Zeitpunkt für die Abfrage der Felder (nach dem Laden, mit Wiederholung für Seiten, die spät aufbauen).
    next_detect: Option<std::time::Instant>,
    attempts: u32,
    /// Letzter Stand der gefundenen Felder (wird bei Zeitablauf geliefert, wenn nie ein Kennwortfeld erschien).
    last: Option<serde_json::Value>,
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

pub struct App {
    pub config: Config,
    pub config_path: Option<PathBuf>,
    pub vault: Option<Vault>,
    /// Index zur Anzeige (aus dem Tresor oder Vorführdaten).
    pub index: Index,
    pub demo: bool,
    pub sessions: Manager,
    pub workspace: Workspace,
    unlocking: Option<Receiver<Result<(Vault, crate::vaultlock::VaultLock), String>>>,
    /// Ergebnis des Ablegens im Schlüsselbund (vom Hintergrundfaden), wird nach dem Entsperren abgeholt.
    keyring_result: std::sync::Arc<std::sync::Mutex<Option<Result<(), String>>>>,
    /// Hinweis zum Schlüsselbund für die Statuszeile (einmal abholbar).
    keyring_note: Option<String>,
    /// Schlüsseldatei des laufenden Entsperrens bzw. Anlegens; wird nach Erfolg in der Konfiguration
    /// gemerkt. `None`: die gemerkte Angabe bleibt, wie sie ist (Entsperren aus dem Schlüsselbund).
    unlock_key_file: Option<Option<PathBuf>>,
    /// Ergebnis des letzten Prüflaufs „Schlüssel und Zertifikate prüfen“.
    pub(crate) cert_rows: Vec<crate::certcheck::CheckRow>,
    /// Objektübersicht: zwischengespeicherte Prüfungen, laufender Ping/Traceroute, Inventarablage.
    pub(crate) overview: crate::overview::OverviewState,
    pub(crate) mass: crate::overview::mass::MassState,
    /// Fortschritt der laufenden Schlüsselprüfung: fertig geprüfte Ziele und Gesamtzahl.
    pub(crate) cert_done: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub(crate) cert_total: usize,
    /// Sperre der geöffneten Schlüsseldatenbank (fällt mit dem Tresor weg).
    vault_lock: Option<crate::vaultlock::VaultLock>,
    /// Laufendes „Felder erkennen“ (unsichtbare Web-Sitzung ohne Tab).
    probe: Option<FieldProbe>,
    /// Laufende Makros und ihr Eingabekanal an die Sitzungen.
    pub(crate) macro_runs: Vec<crate::macro_run::RunningMacro>,
    pub(crate) macro_input_tx: mpsc::Sender<(SessionId, Vec<u8>)>,
    macro_input_rx: Receiver<(SessionId, Vec<u8>)>,
    lights: std::collections::HashMap<Uuid, Light>,
    _rt: tokio::runtime::Runtime,
    /// Tunnel; aufgebaut wird in Hintergrundaufgaben.
    pub(crate) tunnels: std::sync::Arc<std::sync::Mutex<TunnelManager>>,
    /// Freigaben, die warten, weil gerade ein Tunnel aufgebaut wird.
    deferred_releases: Vec<(Uuid, String, u16)>,
    /// Hintergrundaufgaben und deren Ergebnis (Sitzung oder Fehler).
    pub jobs: crate::jobs::Jobs,
    pub(crate) job_results: std::collections::HashMap<u64, Result<SessionId, String>>,
    /// Sperre nach Leerlauf und Erkennung des Aufwachens.
    pub idle: crate::idle::IdleWatch,
    /// Gesperrt (von Hand oder nach Leerlauf).
    pub locked: bool,
    /// Proxmox-Konsolen: Ziel und Gast je Sitzung (zum Neuöffnen).
    pub(crate) console_sources: std::collections::HashMap<SessionId, (Uuid, String)>,
    known_hosts: KnownHosts,
    /// Zuletzt abgelehnte Rückfrage zu einem Host-Schlüssel eines Sprungservers.
    hop_question: std::sync::Arc<std::sync::Mutex<Option<HostKeyQuestion>>>,
    session_tunnels: std::collections::HashMap<SessionId, (Uuid, String, u16)>,
    pub patchday: Option<rj_tasks::patchday::Patchday>,
    /// Update-Funktion (gefundene Version, laufende Aufgabe).
    pub update: crate::update::UpdateState,
    /// Zugangsdaten des laufenden Patchday-Laufs (nur bis zur Übernahme durch die Aufgaben).
    pub patch_creds: Option<std::sync::Arc<crate::patch::CredStore>>,
    import_plan: Option<rj_importer::plan::ImportPlan>,
    snmp_rx: Option<Receiver<(Uuid, rj_snmp::Light, serde_json::Value)>>,
    snmp_stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Verlauf und Zustand je SNMP-Gerät (für die Eigenschaften).
    snmp_state: std::collections::HashMap<Uuid, serde_json::Value>,
    /// Lokale Brücken für Proxmox-Konsolen (je Sitzung).
    pub(crate) console_bridges: std::collections::HashMap<SessionId, rj_proxmox::ConsoleBridge>,
    /// Verlauf für „Rückgängig“.
    pub(crate) history: rj_core::edit::History,
    /// Laufendes „Alle im Ordner verbinden“ und das zuletzt beendete.
    pub(crate) stagger: Option<crate::editor::Stagger>,
    pub stagger_done: Option<crate::editor::Stagger>,
    pub(crate) connect_size: (u32, u32),
    /// Gewählte Ansicht im Baum: leer = alles, `favorites` oder UUID einer Ansicht.
    pub view: String,
}

impl App {
    pub fn new(demo: bool) -> Self {
        let config_path = rj_core::config::default_config_path();
        let config = config_path.as_deref().map(Config::load).and_then(Result::ok).unwrap_or_default();
        let reconnect = Reconnect { auto: config.connection.auto_reconnect, max: Duration::from_secs(config.connection.reconnect_max_secs as u64) };
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("Laufzeit");
        let known_hosts = KnownHosts::default_location().unwrap_or_else(|| KnownHosts::new(std::env::temp_dir().join("remotejoe-known_hosts")));
        let hop_question = std::sync::Arc::new(std::sync::Mutex::new(None));
        // Rückfragen zu Sprungservern: hier ablehnen und merken; die Oberfläche
        // fragt den Benutzer und versucht es nach Zustimmung erneut.
        let (ask, mut questions) = tokio::sync::mpsc::unbounded_channel::<(HostKeyQuestion, tokio::sync::oneshot::Sender<bool>)>();
        let hq = hop_question.clone();
        rt.spawn(async move {
            while let Some((q, reply)) = questions.recv().await {
                *hq.lock().unwrap_or_else(|e| e.into_inner()) = Some(q);
                let _ = reply.send(false);
            }
        });
        let tunnels = std::sync::Arc::new(std::sync::Mutex::new(TunnelManager::new(rt.handle().clone(), known_hosts.clone(), ask)));
        let idle = crate::idle::IdleWatch::new(config.security.lock_after_idle_mins, std::time::Instant::now(), std::time::SystemTime::now());
        let (macro_input_tx, macro_input_rx) = mpsc::channel();
        let mut app = App {
            _rt: rt,
            macro_runs: Vec::new(),
            macro_input_tx,
            macro_input_rx,
            tunnels,
            deferred_releases: Vec::new(),
            jobs: Default::default(),
            job_results: Default::default(),
            idle,
            locked: false,
            console_sources: Default::default(),
            known_hosts,
            hop_question,
            session_tunnels: Default::default(),
            patchday: None,
            patch_creds: None,
            import_plan: None,
            snmp_rx: None,
            snmp_stop: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            snmp_state: Default::default(),
            console_bridges: Default::default(),
            history: Default::default(),
            stagger: None,
            stagger_done: None,
            connect_size: (1024, 768),
            view: String::new(),
            sessions: Manager::new(Manager::default_worker_dir(), reconnect),
            config,
            config_path,
            vault: None,
            index: if demo { tree::demo_index(500) } else { Index::default() },
            demo,
            workspace: Workspace::default(),
            unlocking: None,
            keyring_result: Default::default(),
            keyring_note: None,
            unlock_key_file: None,
            cert_rows: Vec::new(),
            overview: Default::default(),
            mass: Default::default(),
            cert_done: Default::default(),
            cert_total: 0,
            vault_lock: None,
            probe: None,
            lights: Default::default(),
            update: Default::default(),
        };
        // Gespeicherte Farben gelten schon für das erste Terminal.
        let scheme = app.terminal_scheme();
        app.sessions.set_scheme(scheme);
        app.sessions.set_scrollback(app.config.terminal.clamped_scrollback());
        app
    }

    /// Laufzeit für Netzarbeit in Hintergrundaufgaben.
    pub(crate) fn rt_handle(&self) -> tokio::runtime::Handle {
        self._rt.handle().clone()
    }

    pub fn kdbx_path(&self) -> Option<String> {
        self.config.kdbx_path.as_ref().map(|p| p.display().to_string())
    }

    /// Entsperren im Hintergrund; das Ergebnis kommt über `poll`. `remember`: nach Erfolg das
    /// Hauptkennwort im Schlüsselbund des Systems ablegen (nur auf ausdrücklichen Wunsch).
    pub fn unlock(&mut self, path: String, password: Secret, key_file: Option<String>, remember: bool) {
        let storage = self.config.index_storage;
        self.unlock_key_file = Some(key_file.as_deref().filter(|k| !k.is_empty()).map(PathBuf::from));
        let (tx, rx) = mpsc::channel();
        self.unlocking = Some(rx);
        let outcome = self.keyring_result.clone();
        std::thread::spawn(move || {
            let r = open_vault(&path, password.clone(), key_file.clone(), storage);
            if remember && r.is_ok() {
                let pw = password.as_str().map(str::to_string).unwrap_or_default();
                let res = crate::keyring::store(&path, &pw, key_file.as_deref().filter(|k| !k.is_empty()));
                *outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(res);
            }
            let _ = tx.send(r);
        });
    }

    /// Beim Programmstart: mit dem im Schlüsselbund gemerkten Hauptkennwort entsperren (nur wenn der
    /// Nutzer das eingeschaltet hat). Gibt es keinen Eintrag oder ist der Schlüsselbund nicht erreichbar,
    /// meldet `poll` das als Hinweis und die Entsperrmaske bleibt stehen.
    pub fn unlock_from_keyring(&mut self) -> bool {
        let Some(path) = self.config.kdbx_path.as_ref().map(|p| p.display().to_string()) else { return false };
        if self.demo || !self.config.security.keyring_unlock {
            return false;
        }
        let storage = self.config.index_storage;
        let (tx, rx) = mpsc::channel();
        self.unlocking = Some(rx);
        self.unlock_key_file = None;
        std::thread::spawn(move || {
            let r = match crate::keyring::load(&path) {
                Ok(Some(f)) => open_vault(&path, Secret::from_str_exact(&f.password), f.key_file.clone(), storage),
                Ok(None) => Err("Im Schlüsselbund ist für diese Schlüsseldatenbank nichts gespeichert. Bitte das Hauptkennwort eingeben.".to_string()),
                Err(e) => Err(format!("Der Schlüsselbund ist nicht erreichbar ({e}). Bitte das Hauptkennwort eingeben.")),
            };
            let _ = tx.send(r);
        });
        true
    }

    /// Entfernt das gemerkte Hauptkennwort der geöffneten Datenbank aus dem Schlüsselbund.
    pub fn keyring_forget(&mut self) -> String {
        let Some(path) = self.config.kdbx_path.as_ref().map(|p| p.display().to_string()) else { return "Es ist keine Schlüsseldatenbank eingetragen".into() };
        let msg = match crate::keyring::forget(&path) {
            Ok(true) => "Das gemerkte Hauptkennwort wurde aus dem Schlüsselbund entfernt".to_string(),
            Ok(false) => "Im Schlüsselbund war für diese Schlüsseldatenbank nichts gespeichert".to_string(),
            Err(e) => return format!("Schlüsselbund nicht erreichbar: {e}"),
        };
        self.config.security.keyring_unlock = false;
        if !self.demo {
            if let Some(cp) = self.config_path.clone() {
                let _ = self.config.save(&cp);
            }
        }
        msg
    }

    pub fn keyring_enabled(&self) -> bool {
        self.config.security.keyring_unlock && !self.demo
    }

    /// Hinweis zum Schlüsselbund für die Statuszeile (wird beim Abholen gelöscht).
    pub fn take_keyring_note(&mut self) -> String {
        self.keyring_note.take().unwrap_or_default()
    }

    /// Legt eine neue KDBX an und öffnet sie; das Ergebnis kommt wie beim
    /// Entsperren über `poll`. Eine vorhandene Datei wird nie überschrieben.
    /// `make_key_file`: Die Schlüsseldatei unter `key_file` wird neu erzeugt.
    pub fn create_vault(&mut self, path: String, password: Secret, key_file: Option<String>, make_key_file: bool, remember: bool) {
        let storage = self.config.index_storage;
        let outcome = self.keyring_result.clone();
        let (tx, rx) = mpsc::channel();
        if self.vault.as_ref().is_some_and(|v| v.is_dirty()) {
            let _ = tx.send(Err("Es gibt ungespeicherte Änderungen. Bitte zuerst speichern (Datei → Speichern).".to_string()));
            self.unlocking = Some(rx);
            return;
        }
        self.unlocking = Some(rx);
        self.unlock_key_file = Some(key_file.as_deref().filter(|k| !k.is_empty()).map(PathBuf::from));
        std::thread::spawn(move || {
            let result = (|| -> Result<(Vault, crate::vaultlock::VaultLock), String> {
                if path.trim().is_empty() {
                    return Err("Bitte eine Datei angeben.".into());
                }
                let p = std::path::Path::new(&path);
                if p.exists() {
                    return Err(format!("Die Datei „{}“ gibt es schon; sie wird nicht überschrieben.", p.display()));
                }
                let kf = key_file.filter(|k| !k.is_empty());
                if password.as_bytes().is_empty() && kf.is_none() {
                    return Err("Bitte ein Hauptkennwort oder eine Schlüsseldatei angeben.".into());
                }
                if make_key_file && kf.is_none() {
                    return Err("Der Pfad der neuen Schlüsseldatei fehlt.".into());
                }
                if let Some(dir) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
                    std::fs::create_dir_all(dir).map_err(|e| format!("Ordner: {e}"))?;
                }
                let pw_copy = password.as_str().map(str::to_string).unwrap_or_default();
                let mut key = MasterKey { password: Some(password), key_file: None };
                if let Some(k) = &kf {
                    let kp = std::path::Path::new(k);
                    if make_key_file {
                        rj_kdbx::generate_key_file(kp).map_err(|e| format!("Schlüsseldatei: {e}"))?;
                    } else if !kp.is_file() {
                        return Err(KEY_FILE_MISSING.into());
                    }
                    key = key.with_key_file(kp).map_err(|e| format!("Schlüsseldatei: {e}"))?;
                }
                let lock = crate::vaultlock::VaultLock::acquire(p)?;
                let mut v = Vault::create(p, key, rj_kdbx::KdfChoice::Argon2idDefault, storage);
                v.save(None).map_err(|e| e.to_string())?;
                if remember {
                    let res = crate::keyring::store(&path, &pw_copy, kf.as_deref());
                    *outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(res);
                }
                Ok((v, lock))
            })();
            let _ = tx.send(result);
        });
    }

    /// Übersetzt die Einstellungen in das Farbschema des Terminal-Moduls.
    fn terminal_scheme(&self) -> rj_ssh::terminal::Scheme {
        let e = self.config.terminal.effective();
        let rgb = |c: [u8; 3]| alacritty_terminal::vte::ansi::Rgb { r: c[0], g: c[1], b: c[2] };
        rj_ssh::terminal::Scheme { fg: rgb(e.fg), bg: rgb(e.bg), cursor: rgb(e.cursor), ansi: e.ansi.map(rgb) }
    }

    /// Wirksame Einstellungen für die Darstellung (Schrift und Farben) als JSON.
    pub fn terminal_settings_json(&self) -> String {
        use rj_core::termcolors::to_hex;
        let t = &self.config.terminal;
        let e = t.effective();
        json!({
            "font_family": t.font_family.clone().unwrap_or_default(),
            "font_size": t.clamped_size(),
            "scrollback": t.clamped_scrollback(),
            "copy_on_select": t.copy_on_select,
            "confirm_paste": t.confirm_paste,
            "scheme": t.scheme,
            "custom": t.has_overrides(),
            "fg": to_hex(e.fg),
            "bg": to_hex(e.bg),
            "cursor": to_hex(e.cursor),
            "selection": to_hex(e.selection),
        })
        .to_string()
    }

    /// Eingebaute Schemata für die Auswahl (Kennung, Name, Farben).
    pub fn terminal_schemes_json() -> String {
        use rj_core::termcolors::{presets, to_hex};
        let list: Vec<_> = presets()
            .iter()
            .map(|s| json!({ "id": s.id, "label": s.label, "fg": to_hex(s.fg), "bg": to_hex(s.bg), "cursor": to_hex(s.cursor), "selection": to_hex(s.selection), "ansi": s.ansi.iter().map(|c| to_hex(*c)).collect::<Vec<_>>() }))
            .collect();
        json!(list).to_string()
    }

    /// Ändert Schrift und Farben (nur die gelieferten Felder), wendet sie auf alle
    /// Terminals an und speichert sie in der Konfiguration. Ergebnis: wirksame
    /// Einstellungen als JSON oder `{"error": …}`.
    pub fn set_terminal_settings(&mut self, change: &str) -> String {
        use rj_core::termcolors::{self, FONT_SIZE_MAX, FONT_SIZE_MIN};
        let Ok(v) = serde_json::from_str::<serde_json::Value>(change) else { return json!({ "error": "Ungültige Eingabe" }).to_string() };
        let mut t = self.config.terminal.clone();
        if let Some(f) = v.get("font_family").and_then(|x| x.as_str()) {
            let f = f.trim();
            t.font_family = (!f.is_empty()).then(|| f.to_string());
        }
        if let Some(sz) = v.get("font_size").and_then(|x| x.as_f64()) {
            if !sz.is_finite() || sz < FONT_SIZE_MIN as f64 || sz > FONT_SIZE_MAX as f64 {
                return json!({ "error": format!("Die Schriftgröße muss zwischen {} und {} liegen.", FONT_SIZE_MIN, FONT_SIZE_MAX) }).to_string();
            }
            t.font_size = sz as f32;
        }
        if let Some(n) = v.get("scrollback").and_then(|x| x.as_u64()) {
            if n < termcolors::SCROLLBACK_MIN as u64 || n > termcolors::SCROLLBACK_MAX as u64 {
                return json!({ "error": format!("Der Rücklauf muss zwischen {} und {} Zeilen liegen.", termcolors::SCROLLBACK_MIN, termcolors::SCROLLBACK_MAX) }).to_string();
            }
            t.scrollback = n as usize;
        }
        if let Some(b) = v.get("copy_on_select").and_then(|x| x.as_bool()) {
            t.copy_on_select = b;
        }
        if let Some(b) = v.get("confirm_paste").and_then(|x| x.as_bool()) {
            t.confirm_paste = b;
        }
        if let Some(id) = v.get("scheme").and_then(|x| x.as_str()) {
            if termcolors::preset(id).is_none() {
                return json!({ "error": format!("Unbekanntes Farbschema „{id}“") }).to_string();
            }
            if id != t.scheme {
                // Ein anderes Schema löscht eigene Abweichungen, außer sie kommen im selben Aufruf mit.
                t.foreground = None;
                t.background = None;
                t.cursor = None;
                t.selection = None;
            }
            t.scheme = id.to_string();
        }
        for (key, slot) in [("fg", &mut t.foreground), ("bg", &mut t.background), ("cursor", &mut t.cursor), ("selection", &mut t.selection)] {
            if let Some(c) = v.get(key).and_then(|x| x.as_str()) {
                if c.trim().is_empty() {
                    *slot = None;
                } else if let Some(rgb) = termcolors::parse_hex(c) {
                    *slot = Some(termcolors::to_hex(rgb));
                } else {
                    return json!({ "error": format!("„{c}“ ist keine Farbe im Format #rrggbb.") }).to_string();
                }
            }
        }
        self.config.terminal = t;
        let scheme = self.terminal_scheme();
        self.sessions.set_scheme(scheme);
        self.sessions.set_scrollback(self.config.terminal.clamped_scrollback());
        if !self.demo {
            if let Some(cp) = self.config_path.clone() {
                let _ = self.config.save(&cp);
            }
        }
        self.terminal_settings_json()
    }

    /// Schrift in Schritten ändern (Strg+Mausrad); `0` setzt die Vorgabe zurück.
    pub fn terminal_font_step(&mut self, delta: i32) -> String {
        use rj_core::termcolors::{FONT_SIZE_DEFAULT, FONT_SIZE_MAX, FONT_SIZE_MIN};
        let now = self.config.terminal.clamped_size();
        let next = if delta == 0 { FONT_SIZE_DEFAULT } else { (now + delta as f32).clamp(FONT_SIZE_MIN, FONT_SIZE_MAX) };
        self.set_terminal_settings(&json!({ "font_size": next }).to_string())
    }

    /// Merkt sich den Pfad der geöffneten KDBX und ihrer Schlüsseldatei in der Konfiguration (nicht im
    /// Demomodus). `key_file`: `None` lässt die gemerkte Schlüsseldatei unverändert.
    fn remember_kdbx_path(&mut self, path: &std::path::Path, key_file: Option<Option<PathBuf>>) {
        if self.demo {
            return;
        }
        let mut changed = false;
        if self.config.kdbx_path.as_deref() != Some(path) {
            self.config.kdbx_path = Some(path.to_path_buf());
            changed = true;
        }
        if let Some(k) = key_file {
            if self.config.key_file != k {
                self.config.key_file = k;
                changed = true;
            }
        }
        if changed {
            if let Some(cp) = self.config_path.clone() {
                let _ = self.config.save(&cp);
            }
        }
    }

    /// Gemerkte Schlüsseldatei, falls sie zu dieser KDBX gehört (zum Vorbelegen der Entsperrmaske).
    pub fn key_file_for(&self, path: &str) -> Option<String> {
        let same = self.config.kdbx_path.as_deref().is_some_and(|p| p == std::path::Path::new(path));
        same.then(|| self.config.key_file.as_ref().map(|k| k.display().to_string())).flatten()
    }

    /// Sperren: KDBX-Schlüssel und alle Kennwortpuffer verwerfen
    /// (überschrieben beim Freigeben). Laufende Sitzungen bleiben verbunden,
    /// außer `disconnect` ist gesetzt.
    pub fn lock(&mut self, disconnect: bool) {
        for r in &self.macro_runs {
            r.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.snmp_stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.vault = None;
        self.vault_lock = None;
        self.unlocking = None;
        self.import_plan = None;
        self.history.clear();
        self.jobs.cancel_all();
        self.stagger = None;
        if let Some(c) = &self.patch_creds {
            c.wipe_on_lock();
        }
        if disconnect {
            for s in self.sessions.ids() {
                self.close_session(s);
            }
        }
        self.sessions.lock();
        self.locked = true;
        if !self.demo {
            self.index = Index::default();
        }
    }

    /// Nach dem Entsperren: Zugangsdaten der laufenden Sitzungen wieder
    /// herstellen (für das Neuverbinden).
    fn restore_session_credentials(&mut self) {
        let mut found: std::collections::HashMap<Uuid, Option<CredentialPayload>> = Default::default();
        let mut ids = Vec::new();
        self.index.for_each_target(|t| ids.push((t.id, t.protocol)));
        for (id, proto) in ids {
            let p = match resolve_credential(&self.index, id) {
                Ok(CredentialResolution::Found(r)) => self.payload(&r, proto).ok(),
                _ => None,
            };
            found.insert(id, p);
        }
        let protocols: std::collections::HashMap<Uuid, Protocol> = {
            let mut m = std::collections::HashMap::new();
            self.index.for_each_target(|t| {
                m.insert(t.id, t.protocol);
            });
            m
        };
        // Konsolen von Gästen (anderes Protokoll als das Ziel) bleiben unberührt.
        self.sessions.unlock(|spec| {
            if protocols.get(&spec.target) != Some(&spec.protocol) {
                return None;
            }
            found.get(&spec.target).cloned().flatten()
        });
    }

    /// Eingabe des Benutzers (für die Sperre nach Leerlauf).
    pub fn activity(&mut self) {
        self.idle.activity(std::time::Instant::now());
    }

    /// Aufwachen aus dem Standby (Meldung des Systems oder Sprung der Uhr):
    /// nie sperren, Leerlauf neu beginnen, abgebrochene Sitzungen sofort neu
    /// verbinden – Tunnel zuerst.
    pub fn on_resume(&mut self) -> Vec<UiEvent> {
        let now = std::time::Instant::now();
        self.idle.resumed(now, std::time::SystemTime::now());
        self.sessions.set_resume_window(Some(now + Duration::from_secs(120)));
        for s in self.sessions.ids() {
            if matches!(self.sessions.state(s), Some("disconnected") | Some("failed") | Some("crashed")) {
                self.resume_reconnect(s);
            }
        }
        vec![UiEvent::Resumed]
    }

    /// Neu verbinden (Benutzer oder Rückstau). Proxmox-Konsolen werden neu
    /// geöffnet, Tunnel zuerst geprüft und bei Bedarf neu aufgebaut.
    pub fn reconnect_session(&mut self, s: SessionId) {
        if self.console_sources.contains_key(&s) {
            self.reopen_console(s);
        } else if self.session_tunnels.contains_key(&s) {
            self.resume_reconnect(s);
        } else {
            self.sessions.reconnect(s);
        }
    }

    /// Eine Sitzung nach dem Aufwachen neu verbinden; läuft sie über einen
    /// Tunnel, wird dieser zuerst (im Hintergrund) neu aufgebaut.
    pub(crate) fn resume_reconnect(&mut self, s: SessionId) {
        if self.console_sources.contains_key(&s) {
            self.sessions.mark_resumed(s);
            self.reopen_console(s);
            return;
        }
        let Some((tid, host, port)) = self.session_tunnels.get(&s).cloned() else {
            self.sessions.reconnect_resumed(s);
            return;
        };
        let Some(target) = self.sessions.target(s) else { return };
        let spec = self.index.tunnels.iter().find(|t| t.id == tid).cloned().ok_or_else(|| "Tunnel nicht mehr im Index".to_string()).and_then(|def| self.tunnel_spec(&def, target));
        let tunnels = self.tunnels.clone();
        let name = self.index.tunnels.iter().find(|t| t.id == tid).map(|t| t.name.clone()).unwrap_or_default();
        self.jobs.spawn_cancellable(format!("Tunnel „{name}“ neu aufbauen"), move |_, cancel| {
            let r: Result<Option<rj_tunnel::Endpoint>, String> = spec.and_then(|spec| {
                let mut specs = std::collections::HashMap::new();
                specs.insert(tid, spec);
                let mut tm = tunnels.lock().unwrap_or_else(|e| e.into_inner());
                let rebuilt = tm.check_cancellable(&specs, &cancel);
                match rebuilt.into_iter().find(|(id, h, p, _)| *id == tid && *h == host && *p == port) {
                    Some((_, _, _, Ok(ep))) => Ok(Some(ep)),
                    Some((_, _, _, Err(e))) => Err(e),
                    None => Ok(None),
                }
            });
            Box::new(move |app: &mut App, cancelled| {
                if cancelled {
                    return Vec::new();
                }
                match r {
                    Ok(ep) => {
                        if let Some(ep) = ep {
                            app.sessions.set_endpoint(s, &ep.host, ep.port);
                        }
                        if app.sessions.in_resume_window() {
                            app.sessions.reconnect_resumed(s);
                        } else {
                            app.sessions.reconnect(s);
                        }
                        Vec::new()
                    }
                    Err(e) => vec![UiEvent::Notice { s, text: format!("Tunnel „{name}“: {e}") }],
                }
            })
        });
    }

    fn release_tunnel(&mut self, id: Uuid, host: String, port: u16) {
        match self.tunnels.try_lock() {
            Ok(mut tm) => tm.release(id, &host, port),
            Err(_) => self.deferred_releases.push((id, host, port)),
        }
    }

    /// Gespeicherte Fensterlage als JSON (`saved` false, solange noch nie gespeichert wurde).
    pub fn window_state_json(&self) -> String {
        let w = &self.config.window;
        json!({ "saved": w.saved, "x": w.x, "y": w.y, "width": w.width, "height": w.height, "maximized": w.maximized, "tree_width": w.tree_width, "compact": w.compact, "tree_hidden": w.tree_hidden }).to_string()
    }

    /// Merkt Größe, Lage und Baumbreite des Hauptfensters (beim Beenden); nicht im Demomodus.
    pub fn set_window_state(&mut self, state: &str) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(state) else { return };
        let n = |k: &str| v.get(k).and_then(|x| x.as_i64());
        let (Some(x), Some(y), Some(w), Some(h)) = (n("x"), n("y"), n("width"), n("height")) else { return };
        if !(400..=20000).contains(&w) || !(300..=20000).contains(&h) {
            return;
        }
        let tree = n("tree_width").unwrap_or(280).clamp(100, 2000) as u32;
        self.config.window = rj_core::config::WindowState {
            x: x.clamp(-20000, 20000) as i32,
            y: y.clamp(-20000, 20000) as i32,
            width: w as u32,
            height: h as u32,
            maximized: v.get("maximized").and_then(|x| x.as_bool()).unwrap_or(false),
            tree_width: tree,
            saved: true,
            compact: v.get("compact").and_then(|x| x.as_bool()).unwrap_or(false),
            tree_hidden: v.get("tree_hidden").and_then(|x| x.as_bool()).unwrap_or(false),
        };
        if !self.demo {
            if let Some(cp) = self.config_path.clone() {
                let _ = self.config.save(&cp);
            }
        }
    }

    /// Baut den Exportbaum aus dem Index: der ganze Baum oder die gewählten Knoten, mit den
    /// wirksamen Zugangsdaten je Ziel (Kennwort nur, wenn `passwords`).
    fn export_nodes(&self, selection: Option<&[Uuid]>, passwords: bool, stats: &mut (usize, usize, usize)) -> Vec<rj_importer::export::ExportNode> {
        use rj_importer::export::ExportNode;
        fn conv(app: &App, n: &Node, passwords: bool, stats: &mut (usize, usize, usize)) -> ExportNode {
            match n {
                Node::Folder(f) => {
                    stats.1 += 1;
                    ExportNode::Folder { name: f.name.clone(), children: f.children.iter().map(|c| conv(app, c, passwords, stats)).collect() }
                }
                Node::Target(t) => {
                    stats.0 += 1;
                    let (mut username, mut domain, mut password) = (String::new(), None, None);
                    match resolve_credential(&app.index, t.id) {
                        Ok(CredentialResolution::Found(r)) => match app.payload(&r, t.protocol) {
                            Ok(p) => {
                                username = p.username.clone();
                                domain = p.domain.clone();
                                if passwords {
                                    password = Some(Secret::from_bytes(p.password.0.clone()));
                                }
                            }
                            Err(_) => stats.2 += 1,
                        },
                        _ => stats.2 += 1,
                    }
                    ExportNode::Target { name: t.name.clone(), protocol: t.protocol, host: t.host.clone(), port: t.port, options: t.options.clone(), username, domain, password }
                }
            }
        }
        match selection {
            None => self.index.root.children.iter().map(|c| conv(self, c, passwords, stats)).collect(),
            Some(ids) => {
                // Liegt ein gewählter Knoten schon in einem gewählten Ordner, wird er nicht doppelt aufgenommen.
                let inside_selected = |id: Uuid| self.index.path_to(id).is_some_and(|p| p.iter().rev().skip(1).any(|a| ids.contains(&a.id())));
                ids.iter().filter(|id| !inside_selected(**id)).filter_map(|id| self.index.path_to(*id).and_then(|p| p.last().map(|n| conv(self, n, passwords, stats)))).collect()
            }
        }
    }

    /// Exportiert Struktur und Ziele als JSON oder CSV, auf Wunsch mit Kennwörtern und als
    /// verschlüsseltes ZIP (AES-256). Mit Kennwörtern ist beim JSON das ZIP Pflicht.
    pub fn export_run(&mut self, c: &serde_json::Value) -> Result<serde_json::Value, String> {
        use std::io::Write;
        let fmt = c.get("format").and_then(|v| v.as_str()).unwrap_or("json");
        if fmt != "json" && fmt != "csv" {
            return Err("Unbekanntes Format".into());
        }
        let passwords = c.get("passwords").and_then(|v| v.as_bool()).unwrap_or(false);
        let zip = c.get("zip").and_then(|v| v.as_bool()).unwrap_or(false);
        let zip_pw = c.get("zip_password").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if self.vault.is_none() {
            return Err("Zum Export muss die Schlüsseldatenbank entsperrt sein".into());
        }
        if passwords && fmt == "json" && !zip {
            return Err("Mit Kennwörtern gibt es den JSON-Export nur als verschlüsseltes ZIP".into());
        }
        if zip && zip_pw.chars().count() < 8 {
            return Err("Das Kennwort für das ZIP braucht mindestens 8 Zeichen".into());
        }
        let mut path = c.get("path").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        if path.is_empty() {
            return Err("Bitte eine Zieldatei angeben".into());
        }
        let ext = if zip { "zip" } else { fmt };
        if !path.to_ascii_lowercase().ends_with(&format!(".{ext}")) {
            path.push('.');
            path.push_str(ext);
        }
        if std::path::Path::new(&path).exists() {
            return Err(format!("Die Datei „{path}“ gibt es schon; sie wird nicht überschrieben"));
        }
        let selection: Option<Vec<Uuid>> = match c.get("scope").and_then(|v| v.as_str()) {
            Some("selection") => Some(c.get("ids").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok())).collect()).unwrap_or_default()),
            _ => None,
        };
        if selection.as_ref().is_some_and(|s| s.is_empty()) {
            return Err("Im Baum ist nichts gewählt".into());
        }
        let mut stats = (0usize, 0usize, 0usize);
        let nodes = self.export_nodes(selection.as_deref(), passwords, &mut stats);
        let text = if fmt == "json" { rj_importer::export::to_json(&nodes, passwords) } else { rj_importer::export::to_csv(&nodes, passwords) };
        let bytes: Vec<u8> = if zip {
            rj_importer::export::encrypted_zip(&format!("remotejoe-export.{fmt}"), text.as_bytes(), &zip_pw).map_err(|e| e.to_string())?
        } else {
            text.as_bytes().to_vec()
        };
        if let Some(dir) = std::path::Path::new(&path).parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("Ordner: {e}"))?;
        }
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&path).map_err(|e| format!("Datei: {e}"))?;
        f.write_all(&bytes).map_err(|e| format!("Datei: {e}"))?;
        f.sync_all().ok();
        Ok(json!({ "ok": true, "path": path, "targets": stats.0, "folders": stats.1, "without_credentials": stats.2, "passwords": passwords, "zip": zip }))
    }

    /// Benutzername oder Kennwort eines Ziels für die Zwischenablage (`kind`: `user` oder `password`).
    /// Ergebnis als JSON `{ "text", "clear_secs" }` oder `{ "error" }`; der Wert steht nie im Protokoll.
    pub fn credential_copy_json(&self, id: &str, kind: &str) -> String {
        let err = |m: &str| json!({ "error": m }).to_string();
        let Ok(uuid) = Uuid::parse_str(id) else { return err("Ungültiges Ziel") };
        let Some(t) = self.target(uuid) else { return err("Das ist kein Verbindungsziel") };
        if self.vault.is_none() {
            return err("Die Schlüsseldatenbank ist gesperrt");
        }
        let r = match resolve_credential(&self.index, uuid) {
            Ok(CredentialResolution::Found(r)) => r,
            Ok(CredentialResolution::Ambiguous(_)) => return err("Mehrere Zugangsdaten kommen in Frage. Bitte im Ziel einen Datensatz festlegen."),
            _ => return err("Für dieses Ziel sind keine Zugangsdaten hinterlegt"),
        };
        let p = match self.payload(&r, t.protocol) {
            Ok(p) => p,
            Err(e) => return err(&e),
        };
        let text = match kind {
            "user" => p.username.clone(),
            "password" => String::from_utf8_lossy(&p.password.0).into_owned(),
            _ => return err("Unbekannte Angabe"),
        };
        if text.is_empty() {
            return err(if kind == "user" { "Kein Benutzername hinterlegt" } else { "Kein Kennwort hinterlegt" });
        }
        json!({ "text": text, "clear_secs": self.config.security.clipboard_clear_secs }).to_string()
    }

    /// Frist für das Löschen eines kopierten Kennworts (Sekunden, 0 = nie); wird gespeichert.
    pub fn set_clipboard_clear_secs(&mut self, secs: u32) {
        self.config.security.clipboard_clear_secs = secs.min(3600);
        if !self.demo {
            if let Some(cp) = self.config_path.clone() {
                let _ = self.config.save(&cp);
            }
        }
    }

    pub fn clipboard_clear_secs(&self) -> u32 {
        self.config.security.clipboard_clear_secs
    }

    /// Es gibt Änderungen, die noch nicht in die Datei geschrieben sind.
    pub fn has_unsaved_changes(&self) -> bool {
        !self.demo && self.vault.as_ref().is_some_and(|v| v.is_dirty())
    }

    pub fn save(&mut self) -> Result<bool, String> {
        match self.vault.as_mut() {
            Some(v) => v.save(None).map_err(|e| e.to_string()),
            None => Ok(false),
        }
    }

    pub fn tree_json(&self, filter: &str) -> String {
        let lights = |id: Uuid| self.lights.get(&id).copied();
        let mut nodes = match self.view.as_str() {
            "" => tree::tree(&self.index, filter, &lights),
            "favorites" => tree::listed(&self.index, &self.index.favorites, filter, &lights),
            v => match self.index.views.iter().find(|x| x.id.to_string() == v) {
                Some(view) => tree::listed(&self.index, &view.targets, filter, &lights),
                None => tree::tree(&self.index, filter, &lights),
            },
        };
        tree::mark_devices(&mut nodes, &|id| self.overview.devices.get(&id).copied());
        serde_json::to_string(&nodes).unwrap_or_else(|_| "[]".into())
    }

    pub fn set_light(&mut self, id: Uuid, light: Light) {
        self.lights.insert(id, light);
    }

    pub(crate) fn target_ref(&self, id: Uuid) -> Option<&Target> {
        self.target(id)
    }

    pub(crate) fn target_legacy_of(&self, id: Uuid) -> bool {
        self.target_legacy(id)
    }

    pub(crate) fn known_hosts_ref(&self) -> KnownHosts {
        self.known_hosts.clone()
    }

    pub(crate) fn credential_payload(&self, r: &ResolvedCredential, protocol: Protocol) -> Result<CredentialPayload, String> {
        self.payload(r, protocol)
    }

    pub(crate) fn target(&self, id: Uuid) -> Option<&Target> {
        match self.index.path_to(id)?.last()? {
            Node::Target(t) => Some(t),
            _ => None,
        }
    }

    /// Eigenschaften eines Knotens als JSON (ohne Geheimnisse).
    pub fn properties_json(&self, id: Uuid) -> String {
        let Some(path) = self.index.path_to(id) else { return "{}".into() };
        let names: Vec<&str> = path.iter().map(|n| n.name()).collect();
        let mut v = json!({ "path": names.join(" / ") });
        if let Some(Node::Target(t)) = path.last() {
            v["protocol"] = json!(t.protocol.as_str());
            v["host"] = json!(rj_core::charset::mark_visible(&t.host, false));
            v["port"] = json!(t.effective_port());
            v["credential"] = json!(t.credential_name());
            match resolve_credential(&self.index, id) {
                Ok(CredentialResolution::Found(r)) => {
                    let title = self.vault.as_ref().and_then(|vl| vl.credential(r.reference.entry)).map(|c| c.data.title).or(r.reference.last_title.clone());
                    v["credential_entry"] = json!(title);
                }
                Ok(CredentialResolution::Ambiguous(list)) => v["credential_choices"] = json!(list.iter().map(|c| c.name.clone()).collect::<Vec<_>>()),
                _ => {}
            }
            if let Ok(Some(dev)) = rj_core::inherit::resolve_snmp(&self.index, id) {
                v["snmp"] = self.snmp_state.get(&dev).cloned().unwrap_or_else(|| json!({ "light": "grey" }));
            }
            if let Ok(Some((tunnel, _))) = rj_core::inherit::resolve_tunnel(&self.index, id) {
                v["tunnel"] = json!(self.index.tunnels.iter().find(|t| t.id == tunnel).map(|t| t.name.clone()));
            }
        }
        v.to_string()
    }

    fn payload(&self, r: &ResolvedCredential, protocol: Protocol) -> Result<CredentialPayload, String> {
        let vault = self.vault.as_ref().ok_or("Die Schlüsseldatenbank ist gesperrt")?;
        let c = vault.resolve(&r.reference).map_err(|e| e.to_string())?;
        let (dom, user) = c.split_domain();
        let username = r.overrides.username.clone().unwrap_or_else(|| user.to_string());
        let domain = r.overrides.domain.clone().or(dom.map(str::to_string));
        let private_key = if protocol == Protocol::Ssh {
            c.attachments.iter().find(|a| a.starts_with("id_") || a.ends_with(".key") || a.ends_with(".pem")).and_then(|a| vault.attachment(c.uuid, a))
        } else {
            None
        };
        Ok(CredentialPayload {
            username,
            domain,
            password: SecretBytes(c.data.password.as_bytes().to_vec()),
            private_key: private_key.map(|k| SecretBytes(k.as_bytes().to_vec())),
        })
    }

    /// Host und Port zum Verbinden: bei Web-Zielen aus der Adresse, sonst aus dem Ziel.
    pub(crate) fn endpoint_of(t: &Target) -> (String, u16) {
        if t.protocol == Protocol::Web {
            if let ProtocolOptions::Web(o) = &t.options {
                if let Some(ep) = o.url.as_deref().filter(|u| !u.is_empty()).and_then(rj_core::weburl::endpoint) {
                    return ep;
                }
            }
        }
        (t.host.clone(), t.effective_port())
    }

    pub(crate) fn options_json(&self, t: &Target) -> String {
        let mut v = match &t.options {
            // Der Web-Worker erwartet eigene Optionsnamen: Anmeldefelder unter `login`, Profil je Ziel.
            ProtocolOptions::Web(o) if t.protocol == Protocol::Web => {
                let sel = |s: &Option<String>| s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
                let mut w = json!({ "profile": t.id.to_string(), "ignore_certificate": o.ignore_certificate });
                if let Some(u) = sel(&o.url) {
                    w["url"] = json!(u);
                }
                // Leere Selektoren: Der Worker erkennt die Felder selbst (Automatik).
                let (user, pw, submit) = (sel(&o.user_selector), sel(&o.password_selector), sel(&o.submit_selector));
                w["login"] = json!({ "username_selector": user, "password_selector": pw, "submit_selector": submit });
                w
            }
            ProtocolOptions::Rdp(o) => serde_json::to_value(o).unwrap_or_default(),
            ProtocolOptions::Ssh(o) => serde_json::to_value(o).unwrap_or_default(),
            ProtocolOptions::Vnc(o) => serde_json::to_value(o).unwrap_or_default(),
            ProtocolOptions::Web(o) => serde_json::to_value(o).unwrap_or_default(),
            ProtocolOptions::None => json!({}),
        };
        if v.get("clipboard_limit").is_none() {
            v["clipboard_limit"] = json!(self.config.connection.clipboard_limit);
        }
        v.to_string()
    }

    /// Verbindet ein Ziel. Ergebnis als JSON: `{"session":n}`,
    /// `{"ambiguous":[…]}`, `{"missing":{…}}` oder `{"error":"…"}`.
    /// „Felder erkennen“: öffnet das Web-Ziel unsichtbar (ohne Tab, ohne Zugangsdaten), liest die
    /// Eingabefelder und Knöpfe der Seite aus und meldet sie als `FieldsDetected`. Adresse und
    /// Zertifikatsoption kommen aus dem Formular (noch nicht gespeichert).
    pub fn detect_fields(&mut self, target: Uuid, url: &str, ignore_certificate: bool) -> String {
        let Some(Node::Target(t)) = rj_core::edit::node(&self.index, target).cloned() else {
            return json!({ "error": "Ziel nicht gefunden" }).to_string();
        };
        if t.protocol != Protocol::Web {
            return json!({ "error": "Felder erkennen gibt es nur für Web-Ziele" }).to_string();
        }
        if url.trim().is_empty() {
            return json!({ "error": "Bitte zuerst die Adresse (URL) eintragen" }).to_string();
        }
        if rj_tunnel::plan(&self.index, target).is_some() {
            return json!({ "error": "Das Ziel verwendet einen Tunnel; Felder erkennen geht nur ohne Tunnel. Die Felder lassen sich von Hand eintragen." }).to_string();
        }
        if let Some(old) = self.probe.take() {
            self.sessions.close(old.s);
        }
        let mut options: serde_json::Value = serde_json::from_str(&self.options_json(&t)).unwrap_or_else(|_| json!({}));
        options["url"] = json!(url.trim());
        options["ignore_certificate"] = json!(ignore_certificate);
        options["profile"] = json!(format!("{target}-erkennen"));
        // Ohne Fenster im Hauptprogramm (unter Windows legt der Worker dann ein unsichtbares Elternfenster an).
        options["headless"] = json!(true);
        let (host, port) = Self::endpoint_of(&t);
        let spec = ConnectSpec {
            target,
            title: t.name.clone(),
            protocol: Protocol::Web,
            request: ConnectRequest {
                host,
                port,
                width: 1024,
                height: 768,
                options_json: options.to_string(),
                connect_timeout_secs: self.config.connection.connect_timeout_secs,
                keepalive_secs: self.config.connection.keepalive_secs,
            },
            credentials: CredentialPayload { username: String::new(), domain: None, password: SecretBytes(Vec::new()), private_key: None },
            named_credentials: Default::default(),
        };
        let s = self.sessions.open(spec);
        self.probe = Some(FieldProbe { s, target, started: std::time::Instant::now(), saw_loading: false, next_detect: None, attempts: 0, last: None });
        json!({ "ok": true }).to_string()
    }

    /// Wertet die Ereignisse der unsichtbaren Erkennungssitzung aus und blendet sie vor der Oberfläche aus.
    fn process_probe(&mut self, out: &mut Vec<UiEvent>) {
        let Some(mut p) = self.probe.take() else { return };
        let mut done: Option<(Option<String>, serde_json::Value)> = None;
        let mut keep = Vec::with_capacity(out.len());
        for ev in out.drain(..) {
            match ev {
                UiEvent::WebFields { s, fields } if s == p.s => {
                    // Erst ein Kennwortfeld beendet die Suche; Knöpfe oder Textfelder allein können vor dem
                    // Aufbau des Formulars stehen (Seiten, die ihr Formular nachladen).
                    let has_password = fields.get("passwords").and_then(|v| v.as_array()).is_some_and(|a| !a.is_empty());
                    if !has_password && p.attempts < 7 {
                        p.last = Some(fields);
                        p.next_detect = Some(std::time::Instant::now() + Duration::from_millis(1500));
                    } else {
                        done = Some((None, fields));
                    }
                }
                UiEvent::WebStatus { s, loading, error, .. } if s == p.s => {
                    if let Some(e) = error {
                        done = Some((Some(e), json!({})));
                    } else if loading {
                        p.saw_loading = true;
                    } else if p.saw_loading && p.next_detect.is_none() && p.attempts == 0 {
                        // Kurz warten: Seiten bauen ihr Formular oft erst nach dem Laden auf.
                        p.next_detect = Some(std::time::Instant::now() + Duration::from_millis(600));
                    }
                }
                UiEvent::State { s, state, detail } if s == p.s => {
                    if matches!(state.as_str(), "failed" | "disconnected" | "crashed") {
                        done = Some((Some(if detail.is_empty() { "Die Seite konnte nicht geladen werden".to_string() } else { detail }), json!({})));
                    }
                }
                UiEvent::Question { s, .. } if s == p.s => {
                    done = Some((Some("Das Zertifikat der Seite ist nicht vertrauenswürdig. Zertifikatsprüfung abschalten oder die Seite zuerst normal verbinden und das Zertifikat annehmen.".into()), json!({})));
                }
                // Alles Übrige der Erkennungssitzung (Bilder, Titel, Hinweise …) ist für die Oberfläche ohne Belang.
                UiEvent::SharedFrame { s, .. } | UiEvent::Frame { s, .. } | UiEvent::Title { s, .. } | UiEvent::Notice { s, .. } | UiEvent::Terminal { s } if s == p.s => {}
                other => keep.push(other),
            }
        }
        *out = keep;
        if done.is_none() {
            if let Some(at) = p.next_detect.filter(|at| std::time::Instant::now() >= *at) {
                let _ = at;
                p.next_detect = None;
                p.attempts += 1;
                let _ = self.sessions.web_command(p.s, &json!({ "op": "detect" }));
            }
            if done.is_none() && p.started.elapsed() > PROBE_TIMEOUT {
                done = Some(match p.last.take() {
                    Some(fields) => (None, fields),
                    None => (Some("Zeitüberschreitung: Die Seite hat nicht rechtzeitig geantwortet".into()), json!({})),
                });
            }
        }
        match done {
            Some((error, fields)) => {
                self.sessions.close(p.s);
                out.push(UiEvent::FieldsDetected { target: p.target.to_string(), error, fields });
            }
            None => self.probe = Some(p),
        }
    }

    pub fn connect(&mut self, target: Uuid, credential: Option<&str>, remember: bool, width: u32, height: u32) -> String {
        self.connect_way(target, WaySel::Default, credential, remember, width, height)
    }

    /// Wie [`connect`], mit Wahl des Zugangswegs: Hauptweg, Weg für den Doppelklick oder ein bestimmter Weg.
    pub fn connect_way(&mut self, target: Uuid, sel: WaySel, credential: Option<&str>, remember: bool, width: u32, height: u32) -> String {
        let Some(base) = self.target(target).cloned() else { return json!({ "error": "Ziel nicht gefunden" }).to_string() };
        let way = match sel {
            WaySel::Main => None,
            WaySel::Default => base.open_with,
            WaySel::Way(w) => Some(w),
        };
        let (t, way_cred, way_own) = match way {
            None => (base, None, None),
            Some(w) => match (base.via(w), base.ways.iter().find(|x| x.id == w)) {
                (Some(v), Some(x)) => (v, Some(x.credential), x.own.clone().map(|r| (x.label(), r))),
                // Gelöschter Weg als Doppelklick-Weg: Hauptweg nehmen statt zu scheitern.
                _ if sel == WaySel::Default => (base, None, None),
                _ => return json!({ "error": "Zugangsweg nicht gefunden" }).to_string(),
            },
        };
        let resolved = match (way_cred, credential) {
            // Eigene Zugangsdaten des Wegs (eigener KDBX-Eintrag) gehen vor.
            _ if way_own.is_some() => way_own.map(|(name, reference)| ResolvedCredential { name, reference, origin: rj_core::inherit::Origin::Target, overrides: Default::default() }),
            (Some(rj_core::model::WayCredential::None), _) => None,
            (Some(rj_core::model::WayCredential::Login(lid)), _) => match self.index.login(lid) {
                Some(l) => Some(ResolvedCredential { name: l.name.clone(), reference: l.entry.clone(), origin: rj_core::inherit::Origin::Target, overrides: Default::default() }),
                None => return json!({ "error": "Das Anmeldeobjekt dieses Zugangswegs gibt es nicht mehr" }).to_string(),
            },
            (_, credential) => match credential {
            Some(name) => match resolve_credential_named(&self.index, target, name) {
                Ok(Some(r)) => Some(r),
                _ => return json!({ "error": format!("Zugangsdatensatz „{name}“ nicht gefunden") }).to_string(),
            },
            None => match resolve_credential(&self.index, target) {
                Ok(CredentialResolution::Found(r)) => Some(r),
                Ok(CredentialResolution::Disabled) => None,
                Ok(CredentialResolution::Ambiguous(list)) => {
                    let choices: Vec<_> = list
                        .iter()
                        .map(|c| json!({ "name": c.name, "title": c.reference.last_title }))
                        .collect();
                    return json!({ "ambiguous": choices }).to_string();
                }
                // Zusätzlicher Weg „wie das Ziel“, das Ziel hat aber keine: ohne Zugangsdaten verbinden (Anmeldung von Hand).
                Ok(CredentialResolution::NotFound { .. }) if way_cred.is_some() => None,
                Ok(CredentialResolution::NotFound { name }) => {
                    return json!({ "error": format!("Kein Zugangsdatensatz „{name}“ am Ziel oder in einem Elternordner") }).to_string();
                }
                Err(e) => return json!({ "error": e.to_string() }).to_string(),
            },
            },
        };
        if remember {
            if let Some(name) = credential {
                let mut idx = self.index.clone();
                set_target_credential(&mut idx, target, name);
                self.index = idx.clone();
                if let Some(v) = self.vault.as_mut() {
                    let _ = v.set_index(idx);
                }
            }
        }
        let payload = match &resolved {
            None => CredentialPayload { username: String::new(), domain: None, password: SecretBytes(Vec::new()), private_key: None },
            Some(resolved) => match self.payload(resolved, t.protocol) {
                Ok(p) => p,
                Err(e) => {
                    return json!({ "missing": { "name": resolved.name, "title": resolved.reference.last_title, "reason": e } }).to_string();
                }
            },
        };
        // Eingetragene Breite und Höhe (RDP) gelten fest; sonst richtet sich die Sitzung nach dem Fenster.
        let fixed = match &t.options {
            ProtocolOptions::Rdp(o) => o.width.zip(o.height),
            _ => None,
        };
        let (w, h) = if t.protocol == Protocol::Ssh {
            (width.clamp(20, 500), height.clamp(5, 300))
        } else if let Some((fw, fh)) = fixed {
            (fw.clamp(200, 8192), fh.clamp(200, 8192))
        } else {
            (width.clamp(200, 8192), height.clamp(200, 8192))
        };
        let (ep_host, ep_port) = Self::endpoint_of(&t);
        let spec = ConnectSpec {
            target,
            title: t.name.clone(),
            protocol: t.protocol,
            request: ConnectRequest {
                host: ep_host.clone(),
                port: ep_port,
                width: w,
                height: h,
                options_json: self.options_json(&t),
                connect_timeout_secs: self.config.connection.connect_timeout_secs,
                keepalive_secs: self.config.connection.keepalive_secs,
            },
            credentials: payload,
            named_credentials: self.auto_key_credentials(&t),
        };
        if let Some(def) = rj_tunnel::plan(&self.index, target).cloned() {
            let tspec = match self.tunnel_spec(&def, target) {
                Ok(s) => s,
                Err(e) => return json!({ "error": format!("Tunnel „{}“: {e}", def.name) }).to_string(),
            };
            // Tunnelaufbau im Hintergrund; die Oberfläche bleibt bedienbar.
            let tunnels = self.tunnels.clone();
            let hq = self.hop_question.clone();
            let (host, port, proto) = (ep_host, ep_port, t.protocol);
            let title = rj_core::charset::mark_visible(&t.name, false).into_owned();
            let job = self.jobs.spawn_cancellable(format!("Tunnel „{}“ für {}", def.name, title), move |job, cancel| {
                *hq.lock().unwrap_or_else(|e| e.into_inner()) = None;
                let r = tunnels.lock().unwrap_or_else(|e| e.into_inner()).acquire_cancellable(def.id, &tspec, &host, port, &cancel);
                let q = if r.is_err() { hq.lock().unwrap_or_else(|e| e.into_inner()).take() } else { None };
                Box::new(move |app: &mut App, cancelled| match r {
                    Ok(ep) => {
                        if cancelled {
                            app.release_tunnel(def.id, host, port);
                            app.job_results.insert(job, Err("abgebrochen".into()));
                            return Vec::new();
                        }
                        let mut spec = spec;
                        if proto == Protocol::Ssh {
                            let mut o: serde_json::Value = serde_json::from_str(&spec.request.options_json).unwrap_or_else(|_| json!({}));
                            o["host_key_alias"] = json!(format!("{host}:{port}"));
                            spec.request.options_json = o.to_string();
                        }
                        spec.request.host = ep.host;
                        spec.request.port = ep.port;
                        let s = app.open_session(spec, Some((def.id, host, port)));
                        app.job_results.insert(job, Ok(s));
                        vec![UiEvent::SessionOpened { s, job }]
                    }
                    Err(e) => {
                        let error = format!("Tunnel „{}“: {e}", def.name);
                        app.job_results.insert(job, Err(error.clone()));
                        if cancelled {
                            return Vec::new();
                        }
                        let hostkey = q.map(|q| {
                            let info = q.info();
                            json!({ "host": q.host, "port": q.port, "key": q.key, "text": q.text(), "detail": q.detail(),
                                    "kind": info.kind, "key_type": info.key_type, "fingerprint": info.fingerprint, "previous": info.previous })
                        });
                        vec![UiEvent::JobFailed { job, error, hostkey }]
                    }
                })
            });
            return json!({ "pending": job }).to_string();
        }
        let s = self.open_session(spec, None);
        json!({ "session": s }).to_string()
    }

    /// Öffnet eine Sitzung samt Tab; `tunnel` merkt die Tunnelnutzung.
    pub(crate) fn open_session(&mut self, spec: ConnectSpec, tunnel: Option<(Uuid, String, u16)>) -> SessionId {
        let title = rj_core::charset::mark_visible(&spec.title, false).into_owned();
        let s = self.sessions.open(spec);
        self.workspace.open(s, &title);
        if let Some(u) = tunnel {
            self.sessions.set_hold(s, true);
            self.session_tunnels.insert(s, u);
        }
        s
    }

    /// Benannte Zugangsdatensätze, die die Tastenfolge des Ziels mit `{cred:Name:feld}` anspricht; nur gefundene.
    pub(crate) fn auto_key_credentials(&self, t: &Target) -> std::collections::BTreeMap<String, CredentialPayload> {
        let script = match &t.options {
            ProtocolOptions::Rdp(o) => o.auto_keys.as_ref(),
            ProtocolOptions::Ssh(o) => o.auto_keys.as_ref(),
            _ => None,
        }
        .filter(|a| a.enabled)
        .map(|a| a.script.clone())
        .unwrap_or_default();
        let mut out = std::collections::BTreeMap::new();
        for name in rj_core::autokeys::named_credentials(&script) {
            if let Ok(Some(r)) = resolve_credential_named(&self.index, t.id, &name) {
                if let Ok(p) = self.payload(&r, t.protocol) {
                    out.insert(name, p);
                }
            }
        }
        out
    }

    /// Hat das Ziel (SSH) „Ältere Verfahren“ eingeschaltet?
    fn target_legacy(&self, id: Uuid) -> bool {
        self.target(id).is_some_and(|t| matches!(&t.options, rj_core::ProtocolOptions::Ssh(o) if o.legacy_algorithms))
    }

    /// Tunnelbeschreibung mit Zugangsdaten aus der KDBX.
    fn tunnel_spec(&self, def: &rj_core::TunnelDef, target: Uuid) -> Result<TunnelSpec, String> {
        match &def.kind {
            rj_core::TunnelKind::System { probe_host, probe_port } => Ok(TunnelSpec::System { probe_host: probe_host.clone(), probe_port: *probe_port }),
            rj_core::TunnelKind::Ssh { hops } => {
                // Hat das Ziel hinter dem Tunnel „Ältere Verfahren“ eingeschaltet, gilt das auch für die Stufen
                // (die älteren Verfahren stehen in der Rangfolge ganz hinten; neuere werden weiter bevorzugt).
                let target_legacy = self.target_legacy(target);
                let mut out = Vec::new();
                for hop in hops {
                    let name = hop.credential.as_deref().unwrap_or(rj_core::DEFAULT_CREDENTIAL);
                    let r = resolve_credential_named(&self.index, target, name)
                        .map_err(|e| e.to_string())?
                        .ok_or_else(|| format!("Zugangsdatensatz „{name}“ für {} nicht gefunden", hop.host))?;
                    let p = self.payload(&r, Protocol::Ssh)?;
                    let mut auth = Vec::new();
                    if let Some(k) = &p.private_key {
                        auth.push(Auth::Key { pem: Secret::from_bytes(k.0.clone()), passphrase: None });
                    }
                    let pw = Secret::from_bytes(p.password.0.clone());
                    auth.push(Auth::Password(pw.clone()));
                    auth.push(Auth::KeyboardInteractive(pw));
                    out.push(SshParams {
                        host: hop.host.clone(),
                        port: hop.port.unwrap_or(22),
                        user: p.username,
                        auth,
                        connect_timeout: Duration::from_secs(self.config.connection.connect_timeout_secs as u64),
                        keepalive: Duration::from_secs(self.config.connection.keepalive_secs as u64),
                        legacy: hop.legacy_algorithms || target_legacy,
                    });
                }
                Ok(TunnelSpec::Ssh { hops: out })
            }
            rj_core::TunnelKind::Wireguard { config_entry } => {
                let vault = self.vault.as_ref().ok_or("Die Schlüsseldatenbank ist gesperrt")?;
                let c = vault.resolve(config_entry).map_err(|e| e.to_string())?;
                let text = c
                    .attachments
                    .iter()
                    .find(|a| a.ends_with(".conf"))
                    .and_then(|a| vault.attachment(c.uuid, a))
                    .or_else(|| c.data.notes.as_deref().map(rj_core::Secret::from_str_exact))
                    .ok_or("WireGuard-Konfiguration fehlt (Anhang *.conf oder Notizen)")?;
                let text = text.as_str().ok_or("WireGuard-Konfiguration ist kein gültiges UTF-8")?;
                Ok(TunnelSpec::Wireguard { config: rj_tunnel::wireguard::WgConfig::parse(text).map_err(|e| e.to_string())? })
            }
        }
    }

    /// Patchday für mehrere Ziele starten (Trockenlauf, wenn `execute` falsch).
    pub fn start_patchday(&mut self, ids: &[Uuid], execute: bool) -> Result<usize, String> {
        let mut targets = Vec::new();
        let mut jobs = std::collections::HashMap::new();
        for id in ids {
            let Some(t) = self.target(*id).cloned() else { continue };
            targets.push((t.id, rj_core::charset::mark_visible(&t.name, false).into_owned()));
            if self.demo {
                continue;
            }
            if let Ok(CredentialResolution::Found(r)) = resolve_credential(&self.index, t.id) {
                if let Ok(c) = self.payload(&r, t.protocol) {
                    let transport = rj_core::inherit::resolve_psrp_transport(&self.index, t.id).ok().flatten();
                    jobs.insert(t.id, crate::patch::JobSpec { protocol: t.protocol, host: t.host.clone(), port: t.effective_port(), credentials: c, transport, legacy: self.target_legacy(t.id) });
                }
            }
        }
        if targets.is_empty() {
            return Err("Keine Ziele ausgewählt".into());
        }
        let n = targets.len();
        let exec: std::sync::Arc<dyn rj_tasks::patchday::Executor> = if self.demo {
            std::sync::Arc::new(crate::patch::DemoExecutor)
        } else {
            let creds = std::sync::Arc::new(crate::patch::CredStore::new(jobs));
            self.patch_creds = Some(creds.clone());
            std::sync::Arc::new(crate::patch::AppExecutor { creds, execute, full_upgrade: false, known_hosts: self.known_hosts.clone() })
        };
        self.patchday = Some(rj_tasks::patchday::Patchday::start(targets, 4, exec));
        Ok(n)
    }

    /// Patchday: angeforderte Zugangsdaten nachliefern (nur entsperrt).
    fn provide_patch_credentials(&mut self) {
        let Some(store) = self.patch_creds.clone() else { return };
        if self.locked || self.vault.is_none() {
            return;
        }
        store.set_unlocked();
        for id in store.wanted() {
            let spec = self.target(id).cloned().and_then(|t| match resolve_credential(&self.index, t.id) {
                Ok(CredentialResolution::Found(r)) => self.payload(&r, t.protocol).ok().map(|c| crate::patch::JobSpec {
                    protocol: t.protocol,
                    host: t.host.clone(),
                    port: t.effective_port(),
                    credentials: c,
                    transport: rj_core::inherit::resolve_psrp_transport(&self.index, t.id).ok().flatten(),
                    legacy: self.target_legacy(t.id),
                }),
                _ => None,
            });
            store.provide(id, spec);
        }
    }

    /// Patchday-Zeilen für die Anzeige: ohne Protokolle (außer für das
    /// gewählte Ziel), mit letzter Protokollzeile. Hat sich seit `since`
    /// nichts geändert, nur `{"unchanged":true}`.
    pub fn patchday_view(&self, selected: Option<Uuid>, since: u64) -> serde_json::Value {
        let Some(p) = &self.patchday else { return json!({ "version": 0, "rows": [] }) };
        let v = p.version();
        if v == since {
            return json!({ "unchanged": true, "version": v });
        }
        let rows: Vec<_> = p
            .rows()
            .into_iter()
            .map(|r| {
                let mut o = serde_json::to_value(&r).unwrap_or_default();
                o["last"] = json!(r.log.last());
                o["finished"] = json!(r.state.finished());
                if Some(r.target) != selected {
                    o.as_object_mut().map(|m| m.remove("log"));
                }
                o
            })
            .collect();
        json!({ "version": v, "rows": rows })
    }

    pub fn patchday_json(&self) -> String {
        self.patchday.as_ref().map(|p| serde_json::to_string(&p.rows()).unwrap_or_default()).unwrap_or_else(|| "[]".into())
    }

    /// Importvorschau: liest die Datei und liefert den Bericht (nichts wird geschrieben).
    pub fn import_preview(&mut self, path: &str, password: &str) -> Result<(String, serde_json::Value), String> {
        let pw = (!password.is_empty()).then_some(password);
        let doc = rj_importer::read_file_with_password(std::path::Path::new(path), pw).map_err(|e| match e {
            // Kennungen für die Oberfläche: sie fragt das Kennwort des ZIP ab und versucht es erneut.
            rj_importer::ImportError::PasswordNeeded => "PASSWORD_NEEDED".to_string(),
            rj_importer::ImportError::PasswordWrong => "PASSWORD_WRONG".to_string(),
            other => other.to_string(),
        })?;
        let title = format!("Import {}", std::path::Path::new(path).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default());
        let plan = rj_importer::plan::plan(&doc, &title);
        let summary = plan.report.summary();
        let report = serde_json::to_value(&plan.report).map_err(|e| e.to_string())?;
        self.import_plan = Some(plan);
        Ok((summary, report))
    }

    /// Übernimmt die Vorschau in KDBX und Index (gespeichert wird erst auf Befehl).
    pub fn import_apply(&mut self) -> Result<String, String> {
        let plan = self.import_plan.take().ok_or("Keine Importvorschau vorhanden")?;
        let vault = self.vault.as_mut().ok_or("Zum Import muss die Schlüsseldatenbank entsperrt sein")?;
        let idx = rj_importer::plan::apply(&plan, vault, &self.index).map_err(|e| e.to_string())?;
        vault.set_index(idx.clone()).map_err(|e| e.to_string())?;
        self.index = idx;
        Ok(format!("Import übernommen ({} Ziele) – noch nicht gespeichert", plan.report.targets))
    }

    /// Host-Schlüssel eines Sprungservers nach Zustimmung merken.
    pub fn accept_host_key(&self, host: &str, port: u16, key: &str) -> bool {
        self.known_hosts.remember_openssh(host, port, key).is_ok()
    }

    /// Sitzung schließen; Tunnel nach der letzten Sitzung abbauen.
    pub fn close_session(&mut self, s: SessionId) {
        self.sessions.close(s);
        self.workspace.remove_session(s);
        self.console_bridges.remove(&s);
        self.console_sources.remove(&s);
        if let Some((id, host, port)) = self.session_tunnels.remove(&s) {
            self.release_tunnel(id, host, port);
        }
    }

    /// Vorführsitzungen für Bildschirmfotos.
    pub fn open_demo_sessions(&mut self) {
        let mut ids = Vec::new();
        self.index.for_each_target(|t| ids.push(t.id));
        for (i, id) in ids.iter().take(40).enumerate() {
            self.lights.insert(*id, [Light::Green, Light::Green, Light::Yellow, Light::Green, Light::Red, Light::Grey][i % 6]);
        }
        let sample = "\x1b[1;32mrjtest@srv-01\x1b[0m:\x1b[1;34m~\x1b[0m$ apt list --upgradable\r\n\
            Auflistung… Fertig\r\n\x1b[32mopenssl\x1b[0m/noble-updates 3.0.13-0ubuntu3.5 amd64 [aktualisierbar]\r\n\
            \x1b[32mlibssl3\x1b[0m/noble-updates 3.0.13-0ubuntu3.5 amd64 [aktualisierbar]\r\n\
            Grüße aus dem Terminal: äöü ß € 🚀 日本\r\n\x1b[1;32mrjtest@srv-01\x1b[0m:\x1b[1;34m~\x1b[0m$ ";
        let a = self.sessions.open_demo_terminal("ssh-001", sample, 90, 20);
        let b = self.sessions.open_demo_frame("rdp-002", 800, 600, 120);
        let c = self.sessions.open_demo_frame("vnc-003", 800, 600, 30);
        let d = self.sessions.open_demo_terminal("ssh-004", "root@labor:~# uptime\r\n 10:15:02 up 12 days,  3:04,  1 user,  load average: 0,08, 0,05, 0,01\r\nroot@labor:~# ", 90, 20);
        // Vorführsitzungen den gleichnamigen Zielen zuordnen (Punkt am Symbol im Baum).
        let mut by_name = std::collections::HashMap::new();
        self.index.for_each_target(|t| {
            by_name.entry(t.name.clone()).or_insert(t.id);
        });
        for (s, name) in [(a, "ssh-001"), (b, "rdp-002"), (c, "vnc-003"), (d, "ssh-004")] {
            if let Some(id) = by_name.get(name) {
                self.sessions.set_demo_target(s, *id);
            }
        }
        // Vorne eine einzelne SSH-Sitzung, die anderen als Tabs im Hintergrund.
        let t1 = self.workspace.open(a, "ssh-001");
        self.workspace.open(b, "rdp-002");
        self.workspace.open(c, "vnc-003");
        self.workspace.open(d, "ssh-004");
        // Echte Web-Sitzungen für das Bildschirmfoto des Web-Tabs (Adressen aus
        // RJ_DEMO_WEB, durch Komma getrennt), nebeneinander gekachelt.
        let urls = std::env::var("RJ_DEMO_WEB").unwrap_or_default();
        let mut web_tab = None;
        for (i, url) in urls.split(',').filter(|u| !u.trim().is_empty()).take(2).enumerate() {
            let title = format!("web-{:03}", i + 1);
            let spec = ConnectSpec {
                target: Uuid::nil(),
                title: title.clone(),
                protocol: Protocol::Web,
                request: ConnectRequest {
                    host: "127.0.0.1".into(),
                    port: 0,
                    width: 640,
                    height: 560,
                    options_json: json!({ "url": url.trim(), "allow_http": true, "profile": title }).to_string(),
                    connect_timeout_secs: 15,
                    keepalive_secs: 30,
                },
                credentials: CredentialPayload { username: String::new(), domain: None, password: SecretBytes(Vec::new()), private_key: None },
                named_credentials: Default::default(),
            };
            let s = self.sessions.open(spec);
            match web_tab {
                None => web_tab = Some(self.workspace.open(s, &title)),
                Some(t) => {
                    self.workspace.open(s, &title);
                    self.workspace.tile(s, t);
                    self.workspace.set_arrangement(t, Arrangement::OneByTwo);
                }
            }
        }
        self.workspace.activate(t1);
    }

    pub fn poll(&mut self) -> Vec<UiEvent> {
        let mut out = self.sessions.poll(Duration::from_millis(0));
        self.process_probe(&mut out);
        // Eingaben laufender Makros an die Sitzungen weitergeben, beendete Makros melden.
        while let Ok((s, bytes)) = self.macro_input_rx.try_recv() {
            self.sessions.send_bytes(s, bytes);
        }
        let mut finished = Vec::new();
        self.macro_runs.retain(|r| match r.done.try_recv() {
            Ok(o) => {
                finished.push((r.session, r.name.clone(), o));
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => {
                finished.push((r.session, r.name.clone(), crate::macro_run::Outcome { ok: false, message: format!("Makro „{}“ wurde unerwartet beendet.", r.name) }));
                false
            }
        });
        for (s, name, o) in finished {
            self.sessions.detach_macro_tap(s);
            out.push(UiEvent::MacroDone { s, name, ok: o.ok, message: o.message });
        }
        self.provide_patch_credentials();
        // Gehaltene Sitzungen (Tunnel, Konsolenbrücke): erst Tunnel bzw. Brücke, dann neu verbinden.
        for s in self.sessions.take_reconnect_requests() {
            self.reconnect_session(s);
        }
        // Wartende Tunnel-Freigaben.
        if !self.deferred_releases.is_empty() {
            if let Ok(mut tm) = self.tunnels.try_lock() {
                for (id, host, port) in self.deferred_releases.drain(..) {
                    tm.release(id, &host, port);
                }
            }
        }
        // Fertige Hintergrundaufgaben.
        let finished = self.jobs.take_finished();
        if !finished.is_empty() {
            for (job, cancelled, cont) in finished {
                match cont {
                    Some(c) => out.extend(c(self, cancelled)),
                    None => {
                        self.job_results.insert(job, Err("Hintergrundaufgabe abgebrochen".into()));
                        out.push(UiEvent::JobFailed { job, error: "Hintergrundaufgabe unerwartet beendet".into(), hostkey: None });
                    }
                }
            }
            out.push(UiEvent::JobsChanged);
        }
        // Leerlauf und Aufwachen.
        match self.idle.tick(std::time::Instant::now(), std::time::SystemTime::now()) {
            crate::idle::IdleEvent::Lock if !self.locked && (self.vault.is_some() || self.demo) => {
                self.lock(self.config.security.disconnect_on_lock);
                out.push(UiEvent::Locked { reason: "idle".into() });
            }
            crate::idle::IdleEvent::Resumed => out.extend(self.on_resume()),
            _ => {}
        }
        if let Some(before) = self.stagger.as_ref().map(|s| s.results.len()) {
            self.stagger_tick();
            let after = self.stagger.as_ref().or(self.stagger_done.as_ref()).map(|s| s.results.len()).unwrap_or(before);
            if self.stagger.is_none() {
                out.push(UiEvent::ConnectAllDone);
            } else if after > before {
                out.push(UiEvent::ConnectAllProgress);
            }
        }
        if let Some(rx) = &self.unlocking {
            if let Ok(r) = rx.try_recv() {
                self.unlocking = None;
                match r {
                    Ok((v, lock)) => {
                        self.vault_lock = Some(lock);
                        self.index = v.index().index.clone();
                        self.history.clear();
                        self.locked = false;
                        self.idle.activity(std::time::Instant::now());
                        let mut n = 0;
                        self.index.for_each_target(|_| n += 1);
                        let kdbx = v.path().to_path_buf();
                        self.vault = Some(v);
                        // Web-Ziele aus älteren Importen (ganze Adresse im Host-Feld) reparieren.
                        let repaired = self.index.repair_web_targets();
                        if repaired > 0 {
                            if let Some(v) = self.vault.as_mut() {
                                let _ = v.set_index(self.index.clone());
                            }
                            let note = format!("{repaired} Web-Ziele aus einem älteren Import wurden repariert (Adresse in den Optionen); bitte speichern.");
                            self.keyring_note = Some(match self.keyring_note.take() {
                                Some(k) => format!("{k}. {note}"),
                                None => note,
                            });
                        }
                        // Bisherige „vorhandene Datensätze“ in Anmeldeobjekte übernehmen.
                        let converted = match self.vault.as_ref() {
                            Some(v) => rj_core::inherit::convert_entries_to_logins(&mut self.index, &|e| v.is_own_entry(e), &|e| v.credential(e).map(|c| c.data.title)),
                            None => 0,
                        };
                        if converted > 0 {
                            if let Some(v) = self.vault.as_mut() {
                                let _ = v.set_index(self.index.clone());
                            }
                            let note = format!("{converted} Zugangsdaten mit „vorhandenem Datensatz“ wurden in Anmeldeobjekte übernommen; bitte speichern.");
                            self.keyring_note = Some(match self.keyring_note.take() {
                                Some(k) => format!("{k}. {note}"),
                                None => note,
                            });
                        }
                        let key_file = self.unlock_key_file.take();
                        self.remember_kdbx_path(&kdbx, key_file);
                        self.restore_session_credentials();
                        self.start_snmp();
                        // Hauptkennwort im Schlüsselbund gemerkt? (Ergebnis vom Hintergrundfaden)
                        if let Some(r) = self.keyring_result.lock().unwrap_or_else(|e| e.into_inner()).take() {
                            match r {
                                Ok(()) => {
                                    self.config.security.keyring_unlock = true;
                                    if !self.demo {
                                        if let Some(cp) = self.config_path.clone() {
                                            let _ = self.config.save(&cp);
                                        }
                                    }
                                    self.keyring_note = Some("Hauptkennwort im Schlüsselbund des Systems gemerkt; beim nächsten Start wird damit entsperrt".into());
                                }
                                Err(e) => self.keyring_note = Some(format!("Hauptkennwort nicht im Schlüsselbund gemerkt: {e}")),
                            }
                        }
                        out.push(UiEvent::Unlocked { targets: n });
                        self.maybe_auto_check();
                    }
                    Err(reason) => {
                        self.unlock_key_file = None;
                        out.push(UiEvent::UnlockFailed { reason });
                    }
                }
            }
        }
        let mut changed = false;
        if let Some(rx) = &self.snmp_rx {
            while let Ok((dev, light, state)) = rx.try_recv() {
                self.snmp_state.insert(dev, state);
                let l = match light {
                    rj_snmp::Light::Grey => Light::Grey,
                    rj_snmp::Light::Green => Light::Green,
                    rj_snmp::Light::Yellow => Light::Yellow,
                    rj_snmp::Light::Red => Light::Red,
                };
                // Alle Ziele, die das Gerät erben, und Ordner, die es nennen.
                let mut ids = Vec::new();
                self.index.for_each_target(|t| {
                    if rj_core::inherit::resolve_snmp(&self.index, t.id).ok().flatten() == Some(dev) {
                        ids.push(t.id);
                    }
                });
                fn folders(f: &rj_core::Folder, dev: Uuid, out: &mut Vec<Uuid>) {
                    if f.settings.snmp == Some(rj_core::SnmpSetting::Device(dev)) {
                        out.push(f.id);
                    }
                    for c in &f.children {
                        if let Node::Folder(sub) = c {
                            folders(sub, dev, out);
                        }
                    }
                }
                folders(&self.index.root, dev, &mut ids);
                for id in ids {
                    if self.lights.insert(id, l) != Some(l) {
                        changed = true;
                    }
                }
            }
        }
        if changed {
            out.push(UiEvent::TreeChanged);
        }
        out
    }

    /// SNMP-Überwachung aller Geräte im Index (Hintergrund, alle 30 s).
    pub fn start_snmp(&mut self) {
        self.snmp_stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let Some(vault) = &self.vault else { return };
        let mut monitors = Vec::new();
        for d in &self.index.snmp_devices {
            let Some(credentials) = snmp_credentials(vault, d) else { continue };
            let dev = rj_snmp::Device { host: d.host.clone(), port: d.port.unwrap_or(161), credentials, timeout: Duration::from_secs(3) };
            monitors.push((d.id, rj_snmp::Monitor::new(dev)));
        }
        if monitors.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.snmp_rx = Some(rx);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.snmp_stop = stop.clone();
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                for (id, m) in &mut monitors {
                    let light = m.poll(start.elapsed().as_secs());
                    let state = json!({
                        "light": light,
                        "error": m.last_error,
                        "cpu": m.cpu.hour.iter().map(|x| x.1).collect::<Vec<_>>(),
                        "mem": m.mem.hour.iter().map(|x| x.1).collect::<Vec<_>>(),
                        "in": m.throughput_in.hour.iter().map(|x| x.1).collect::<Vec<_>>(),
                        "out": m.throughput_out.hour.iter().map(|x| x.1).collect::<Vec<_>>(),
                    });
                    if tx.send((*id, light, state)).is_err() {
                        return;
                    }
                }
                for _ in 0..300 {
                    if stop.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        });
    }

    pub fn mouse(&mut self, s: SessionId, x: i32, y: i32, button: i32, down: bool) {
        let b = match button {
            1 => Some(MouseButton::Left),
            2 => Some(MouseButton::Right),
            3 => Some(MouseButton::Middle),
            4 => Some(MouseButton::WheelUp),
            5 => Some(MouseButton::WheelDown),
            _ => None,
        };
        self.sessions.mouse(s, x, y, b, down);
    }

    /// Nicht zugeordnete KDBX-Einträge als JSON.
    pub fn unassigned_json(&self) -> String {
        let Some(v) = &self.vault else { return "[]".into() };
        let list: Vec<_> = v.unassigned().into_iter().map(|e| json!({ "uuid": e.uuid, "title": e.title, "group": e.group_path.join("/") })).collect();
        serde_json::Value::Array(list).to_string()
    }
}

/// Zugang zu einem SNMP-Gerät aus der KDBX: v2c Community; v3 Benutzer,
/// Kennwort zur Authentisierung und – getrennt – zur Verschlüsselung.
pub fn snmp_credentials(vault: &Vault, d: &rj_core::SnmpDevice) -> Option<rj_snmp::Credentials> {
    let c = vault.resolve(&d.credential).ok()?;
    Some(match d.version {
        rj_core::SnmpVersion::V2c => rj_snmp::Credentials::V2c { community: c.data.password.clone() },
        rj_core::SnmpVersion::V3 => {
            let auth = match d.auth_proto.unwrap_or(rj_core::SnmpAuth::Sha1) {
                rj_core::SnmpAuth::Md5 => rj_snmp::AuthProto::Md5,
                rj_core::SnmpAuth::Sha1 => rj_snmp::AuthProto::Sha1,
                rj_core::SnmpAuth::Sha256 => rj_snmp::AuthProto::Sha256,
            };
            let privacy = match d.priv_proto.unwrap_or(rj_core::SnmpPriv::Aes128) {
                rj_core::SnmpPriv::None => None,
                p => {
                    let proto = if p == rj_core::SnmpPriv::Des { rj_snmp::PrivProto::Des } else { rj_snmp::PrivProto::Aes128 };
                    let pw = vault.field(c.uuid, rj_core::SNMP_PRIV_FIELD).unwrap_or_else(|| c.data.password.clone());
                    Some((proto, pw))
                }
            };
            rj_snmp::Credentials::V3 { user: c.data.username.clone(), auth, auth_password: c.data.password.clone(), privacy }
        }
    })
}

fn set_target_credential(idx: &mut Index, target: Uuid, name: &str) {
    fn walk(f: &mut rj_core::Folder, target: Uuid, name: &str) -> bool {
        for n in &mut f.children {
            match n {
                Node::Target(t) if t.id == target => {
                    t.credential = Some(name.to_string());
                    return true;
                }
                Node::Folder(sub) => {
                    if walk(sub, target, name) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }
    walk(&mut idx.root, target, name);
}

/// Wahl des Zugangswegs beim Verbinden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaySel {
    /// Weg für den Doppelklick (fehlt er, der Hauptweg).
    Default,
    /// Immer der Hauptweg.
    Main,
    /// Ein bestimmter weiterer Weg.
    Way(Uuid),
}

impl WaySel {
    /// Aus der Ziel-ID der Oberfläche: `<id>`, `<id>#main` oder `<id>#<Weg-ID>`.
    pub fn split(id: &str) -> (&str, WaySel) {
        match id.split_once('#') {
            Some((t, "main")) => (t, WaySel::Main),
            Some((t, w)) => (t, Uuid::parse_str(w).map(WaySel::Way).unwrap_or(WaySel::Default)),
            None => (id, WaySel::Default),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rj_core::CredentialRef;

    #[test]
    fn connect_reports_ambiguity_and_missing() {
        let mut app = App::new(false);
        let mut idx = Index::default();
        let mut f = rj_core::Folder::new("Server");
        f.settings.credentials.insert("admin".into(), CredentialRef { entry: Uuid::new_v4(), last_title: Some("Admin-Konto".into()) });
        f.settings.credentials.insert("user".into(), CredentialRef { entry: Uuid::new_v4(), last_title: None });
        let t = Target::new("srv", Protocol::Ssh, "srv.example.test");
        let tid = t.id;
        f.children.push(Node::Target(t));
        idx.root.children.push(Node::Folder(f));
        app.index = idx;
        let r: serde_json::Value = serde_json::from_str(&app.connect(tid, None, false, 80, 24)).unwrap();
        assert_eq!(r["ambiguous"].as_array().unwrap().len(), 2);
        // Gewählt, aber KDBX gesperrt → fehlt mit zuletzt bekanntem Titel.
        let r: serde_json::Value = serde_json::from_str(&app.connect(tid, Some("admin"), true, 80, 24)).unwrap();
        assert_eq!(r["missing"]["title"], "Admin-Konto");
        // Wahl gemerkt.
        let Node::Folder(f) = &app.index.root.children[0] else { panic!() };
        let Node::Target(t) = &f.children[0] else { panic!() };
        assert_eq!(t.credential.as_deref(), Some("admin"));
    }

    #[test]
    fn demo_workspace() {
        let mut app = App::new(true);
        app.open_demo_sessions();
        // Vorne eine einzelne SSH-Sitzung, drei weitere Tabs im Hintergrund.
        assert_eq!(app.workspace.tabs.len(), 4);
        assert!(app.workspace.tabs.iter().all(|t| t.sessions.len() == 1));
        assert!(app.tree_json("").len() > 10_000);
    }
}

#[cfg(test)]
mod tunnel_tests {
    use super::*;
    use rj_core::{CredentialRef, Folder, TunnelDef, TunnelKind, TunnelSetting};
    use rj_kdbx::{CredentialData, KdfChoice};

    #[test]
    fn connect_uses_inherited_system_tunnel() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = Vault::create(&dir.path().join("t.kdbx"), MasterKey::password("pw"), KdfChoice::Weak, rj_core::config::IndexStorage::KdbxCustomData);
        let entry = vault
            .add_credential(CredentialData { title: "Konto".into(), username: "benutzer".into(), password: Secret::from_str_exact("geheim"), ..Default::default() })
            .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let probe = listener.local_addr().unwrap().port();
        let mut idx = Index::default();
        let tid = Uuid::new_v4();
        idx.tunnels.push(TunnelDef { id: tid, name: "VPN".into(), kind: TunnelKind::System { probe_host: "127.0.0.1".into(), probe_port: probe } });
        let mut f = Folder::new("Hinter VPN");
        f.settings.tunnel = Some(TunnelSetting::Use(tid));
        f.settings.credentials.insert("default".into(), CredentialRef { entry, last_title: Some("Konto".into()) });
        let t = Target::new("srv", Protocol::Ssh, "srv.example.test");
        let target = t.id;
        f.children.push(Node::Target(t));
        idx.root.children.push(Node::Folder(f));
        let mut app = App::new(false);
        app.index = idx;
        app.vault = Some(vault);
        let r: serde_json::Value = serde_json::from_str(&app.connect(target, None, false, 80, 24)).unwrap();
        // Tunnelaufbau läuft im Hintergrund; die Sitzung folgt über `poll`.
        let job = r["pending"].as_u64().unwrap_or_else(|| panic!("{r}"));
        let start = std::time::Instant::now();
        let s = loop {
            let ev = app.poll();
            if let Some(s) = ev.iter().find_map(|e| match e {
                UiEvent::SessionOpened { s, job: j } if *j == job => Some(*s),
                _ => None,
            }) {
                break s;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "Tunnel nicht aufgebaut");
            std::thread::sleep(Duration::from_millis(10));
        };
        app.close_session(s);
        drop(listener);
        let r: serde_json::Value = serde_json::from_str(&app.connect(target, None, false, 80, 24)).unwrap();
        let job = r["pending"].as_u64().unwrap_or_else(|| panic!("{r}"));
        let start = std::time::Instant::now();
        let error = loop {
            if let Some(e) = app.poll().into_iter().find_map(|e| match e {
                UiEvent::JobFailed { job: j, error, .. } if j == job => Some(error),
                _ => None,
            }) {
                break e;
            }
            assert!(start.elapsed() < Duration::from_secs(20), "kein Fehler gemeldet");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(error.contains("Tunnel „VPN“"), "{error}");
    }

    /// Die Tastenfolge kann einen zweiten, benannten Zugangsdatensatz einsetzen (anderes Kennwort für die Sitzung).
    #[test]
    fn auto_keys_resolve_named_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = Vault::create(&dir.path().join("t.kdbx"), MasterKey::password("pw"), KdfChoice::Weak, rj_core::config::IndexStorage::KdbxCustomData);
        let main = vault.add_credential(CredentialData { title: "RDP".into(), username: "anna".into(), password: Secret::from_str_exact("erstes"), ..Default::default() }).unwrap();
        let second = vault.add_credential(CredentialData { title: "Sitzung".into(), username: "anna2".into(), password: Secret::from_str_exact("zweites Kennwort"), ..Default::default() }).unwrap();
        let mut idx = Index::default();
        let mut f = Folder::new("Ubuntu");
        f.settings.credentials.insert("default".into(), CredentialRef { entry: main, last_title: Some("RDP".into()) });
        f.settings.credentials.insert("Sitzung".into(), CredentialRef { entry: second, last_title: Some("Sitzung".into()) });
        let mut t = Target::new("desktop", Protocol::Rdp, "desktop.example.test");
        t.options = ProtocolOptions::Rdp(rj_core::model::RdpOptions {
            auto_keys: Some(rj_core::autokeys::AutoKeys { enabled: true, script: "Taste Enter\nText {cred:Sitzung:password}\nText {cred:Gibtsnicht:password}".into(), ..Default::default() }),
            ..Default::default()
        });
        let target = t.id;
        f.children.push(Node::Target(t));
        idx.root.children.push(Node::Folder(f));
        let mut app = App::new(false);
        app.index = idx;
        app.vault = Some(vault);
        let t = app.target(target).cloned().unwrap();
        let named = app.auto_key_credentials(&t);
        // Gefunden wird nur, was es gibt; für „Gibtsnicht“ meldet die Tastenfolge beim Senden den Fehler.
        assert_eq!(named.keys().collect::<Vec<_>>(), vec!["Sitzung"]);
        assert_eq!(named["Sitzung"].password.0, b"zweites Kennwort");
        assert_eq!(named["Sitzung"].username, "anna2");
        // Ausgeschaltet: nichts auflösen.
        let mut off = t.clone();
        if let ProtocolOptions::Rdp(o) = &mut off.options {
            o.auto_keys.as_mut().unwrap().enabled = false;
        }
        assert!(app.auto_key_credentials(&off).is_empty());
    }

    /// „Ältere Verfahren“: gilt für die Stufe selbst und, wenn das Ziel sie eingeschaltet hat, für alle Stufen.
    #[test]
    fn tunnel_hops_follow_legacy_option() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = Vault::create(&dir.path().join("t.kdbx"), MasterKey::password("pw"), KdfChoice::Weak, rj_core::config::IndexStorage::KdbxCustomData);
        let entry = vault
            .add_credential(CredentialData { title: "Konto".into(), username: "benutzer".into(), password: Secret::from_str_exact("geheim"), ..Default::default() })
            .unwrap();
        let mut idx = Index::default();
        let tid = Uuid::new_v4();
        let def = TunnelDef {
            id: tid,
            name: "Sprung".into(),
            kind: TunnelKind::Ssh {
                hops: vec![
                    rj_core::SshHop { host: "neu.example.test".into(), port: None, credential: None, legacy_algorithms: false },
                    rj_core::SshHop { host: "alt.example.test".into(), port: None, credential: None, legacy_algorithms: true },
                ],
            },
        };
        idx.tunnels.push(def.clone());
        let mut f = Folder::new("Hinter Sprung");
        f.settings.tunnel = Some(TunnelSetting::Use(tid));
        f.settings.credentials.insert("default".into(), CredentialRef { entry, last_title: Some("Konto".into()) });
        let t = Target::new("srv", Protocol::Ssh, "srv.example.test");
        let target = t.id;
        f.children.push(Node::Target(t));
        idx.root.children.push(Node::Folder(f));
        let mut app = App::new(false);
        app.index = idx;
        app.vault = Some(vault);
        let flags = |app: &App| match app.tunnel_spec(&def, target).unwrap() {
            TunnelSpec::Ssh { hops } => hops.iter().map(|h| h.legacy).collect::<Vec<_>>(),
            _ => panic!("kein SSH-Tunnel"),
        };
        assert_eq!(flags(&app), vec![false, true], "nur die Stufe mit eigener Einstellung");
        // Ziel schaltet „Ältere Verfahren“ ein: gilt für alle Stufen.
        for n in app.index.root.children.iter_mut() {
            if let Node::Folder(f) = n {
                for c in f.children.iter_mut() {
                    if let Node::Target(t) = c {
                        t.options = rj_core::ProtocolOptions::Ssh(rj_core::SshOptions { legacy_algorithms: true, ..Default::default() });
                    }
                }
            }
        }
        assert_eq!(flags(&app), vec![true, true], "Option des Ziels gilt für alle Stufen");
    }
}

#[cfg(test)]
mod patchday_tests {
    use super::*;

    /// 25 Ziele mit Attrappe: Anzeige-Daten bleiben schnell und klein,
    /// unveränderte Stände werden nicht neu geliefert, Abbruch je Ziel.
    #[test]
    fn view_is_cheap_and_cancel_per_target() {
        let mut app = App::new(true);
        let mut ids = Vec::new();
        app.index.for_each_target(|t| ids.push(t.id));
        ids.truncate(25);
        assert_eq!(app.start_patchday(&ids, false).unwrap(), 25);
        let victim = ids[20];
        app.patchday.as_ref().unwrap().cancel(victim);
        let mut since = 0;
        let mut worst = Duration::ZERO;
        let mut unchanged = 0;
        let mut largest = 0;
        let start = std::time::Instant::now();
        while !app.patchday.as_ref().unwrap().finished() {
            let t = std::time::Instant::now();
            let v = app.patchday_view(Some(ids[0]), since);
            worst = worst.max(t.elapsed());
            if v["unchanged"] == true {
                unchanged += 1;
            } else {
                since = v["version"].as_u64().unwrap();
                largest = largest.max(v.to_string().len());
                // Protokoll nur für das gewählte Ziel.
                let rows = v["rows"].as_array().unwrap();
                assert_eq!(rows.iter().filter(|r| r.get("log").is_some()).count(), 1);
            }
            assert!(start.elapsed() < Duration::from_secs(60));
            std::thread::sleep(Duration::from_millis(25));
        }
        println!("MESSWERT Patchday 25 Ziele: längste Aufbereitung {} µs, größte Antwort {largest} Bytes, {unchanged} Takte ohne Änderung, Dauer {:.1} s", worst.as_micros(), start.elapsed().as_secs_f64());
        assert!(worst < Duration::from_millis(20), "Aufbereitung zu langsam: {worst:?}");
        let rows = app.patchday.as_ref().unwrap().rows();
        let r = rows.iter().find(|r| r.target == victim).unwrap();
        assert_eq!(r.state, rj_tasks::patchday::JobState::Cancelled);
        assert!(rows.iter().filter(|r| r.target != victim).all(|r| r.state != rj_tasks::patchday::JobState::Cancelled));
    }
}

#[cfg(test)]
mod terminal_settings_tests {
    use super::*;

    fn parse(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn scheme_font_and_overrides() {
        let mut app = App::new(true);
        app.config_path = None;
        let d = parse(&app.terminal_settings_json());
        assert_eq!(d["scheme"], "standard-dunkel");
        assert_eq!(d["font_size"], 11.0);

        // Schema wählen: wirksame Farben ändern sich.
        let r = parse(&app.set_terminal_settings(r##"{"scheme":"catppuccin-mocha"}"##));
        assert_eq!(r["bg"], "#1e1e2e");
        assert_eq!(r["fg"], "#cdd6f4");
        assert_eq!(r["custom"], false);

        // Eigene Farbe, Schriftart mit Leerzeichen, Größe.
        let r = parse(&app.set_terminal_settings(r##"{"bg":"#102030","font_family":"  DejaVu Sans Mono ","font_size":14}"##));
        assert_eq!(r["bg"], "#102030");
        assert_eq!(r["custom"], true);
        assert_eq!(r["font_family"], "DejaVu Sans Mono");
        assert_eq!(r["font_size"], 14.0);

        // Anderes Schema verwirft die eigenen Abweichungen.
        let r = parse(&app.set_terminal_settings(r##"{"scheme":"catppuccin-latte"}"##));
        assert_eq!(r["bg"], "#eff1f5");
        assert_eq!(r["custom"], false);

        // Leere Schriftart und leere Farbe setzen zurück.
        let r = parse(&app.set_terminal_settings(r##"{"font_family":"","bg":"#000001"}"##));
        assert_eq!(r["font_family"], "");
        let r = parse(&app.set_terminal_settings(r##"{"bg":""}"##));
        assert_eq!(r["custom"], false);
    }

    #[test]
    fn invalid_input_is_rejected_without_change() {
        let mut app = App::new(true);
        app.config_path = None;
        let before = app.terminal_settings_json();
        for bad in [r#"{"font_size":3}"#, r#"{"font_size":99}"#, r#"{"scheme":"gibt-es-nicht"}"#, r##"{"fg":"rot"}"##, "kein json"] {
            let r = parse(&app.set_terminal_settings(bad));
            assert!(r.get("error").is_some(), "{bad}");
            assert_eq!(app.terminal_settings_json(), before, "{bad}");
        }
    }

    #[test]
    fn font_steps_are_clamped_and_reset() {
        let mut app = App::new(true);
        app.config_path = None;
        assert_eq!(parse(&app.terminal_font_step(2))["font_size"], 13.0);
        assert_eq!(parse(&app.terminal_font_step(-3))["font_size"], 10.0);
        assert_eq!(parse(&app.terminal_font_step(100))["font_size"], 40.0);
        assert_eq!(parse(&app.terminal_font_step(-100))["font_size"], 6.0);
        assert_eq!(parse(&app.terminal_font_step(0))["font_size"], 11.0);
    }

    #[test]
    fn schemes_list_contains_catppuccin() {
        let l = parse(&App::terminal_schemes_json());
        let ids: Vec<_> = l.as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_string()).collect();
        for id in ["standard-dunkel", "standard-hell", "hoher-kontrast", "catppuccin-latte", "catppuccin-frappe", "catppuccin-macchiato", "catppuccin-mocha"] {
            assert!(ids.contains(&id.to_string()), "{id}");
        }
        assert_eq!(l[0]["ansi"].as_array().unwrap().len(), 16);
    }
}

#[cfg(test)]
mod web_options_tests {
    use super::*;
    use rj_core::{Protocol, Target, WebOptions};

    #[test]
    fn web_options_use_the_names_of_the_web_worker() {
        let app = App::new(true);
        let mut t = Target::new("ilo", Protocol::Web, "192.0.2.10");
        t.options = ProtocolOptions::Web(WebOptions {
            url: Some("https://192.0.2.10/".into()),
            user_selector: Some(" #user ".into()),
            password_selector: Some("#pw".into()),
            submit_selector: Some("".into()),
            ignore_certificate: true,
        });
        let v: serde_json::Value = serde_json::from_str(&app.options_json(&t)).unwrap();
        assert_eq!(v["url"], "https://192.0.2.10/");
        assert_eq!(v["ignore_certificate"], true);
        assert_eq!(v["profile"], t.id.to_string());
        assert_eq!(v["login"]["username_selector"], "#user");
        assert_eq!(v["login"]["password_selector"], "#pw");
        assert!(v["login"]["submit_selector"].is_null());
        // Ohne Selektoren und Adresse: `login` ohne Selektoren (Automatik), keine `url`.
        t.options = ProtocolOptions::Web(WebOptions::default());
        let v: serde_json::Value = serde_json::from_str(&app.options_json(&t)).unwrap();
        assert!(v["login"]["password_selector"].is_null() && v["login"]["username_selector"].is_null() && v.get("url").is_none());
        assert_eq!(v["ignore_certificate"], false);
    }
}

/// Öffnet die Schlüsseldatenbank (Sperrdatei, Schlüsseldatei, Vault).
/// Meldung, wenn die angegebene Schlüsseldatei fehlt. Nennt den eingegebenen Text bewusst nicht:
/// Dort steht manchmal versehentlich das Kennwort.
const KEY_FILE_MISSING: &str = "Die Schlüsseldatei wurde nicht gefunden. Bitte über „Wählen …“ die richtige Datei auswählen oder den Haken „Schlüsseldatei verwenden“ entfernen.";

fn open_vault(path: &str, password: Secret, key_file: Option<String>, storage: rj_core::config::IndexStorage) -> Result<(Vault, crate::vaultlock::VaultLock), String> {
    let key_file = key_file.filter(|k| !k.is_empty());
    if key_file.as_deref().is_some_and(|k| !std::path::Path::new(k).is_file()) {
        return Err(KEY_FILE_MISSING.into());
    }
    let lock = crate::vaultlock::VaultLock::acquire(std::path::Path::new(path))?;
    let mut key = MasterKey { password: Some(password), key_file: None };
    if let Some(k) = key_file {
        key = key.with_key_file(std::path::Path::new(&k)).map_err(|e| format!("Schlüsseldatei: {e}"))?;
    }
    Vault::open(std::path::Path::new(path), key, storage).map(|v| (v, lock)).map_err(|e| e.to_string())
}
