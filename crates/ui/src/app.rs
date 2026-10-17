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

use crate::sessions::{ConnectSpec, Manager, Reconnect, UiEvent};
use crate::tree::{self, Light};
use crate::workspace::{Arrangement, SessionId, Workspace};

pub struct App {
    pub config: Config,
    pub config_path: Option<PathBuf>,
    pub vault: Option<Vault>,
    /// Index zur Anzeige (aus dem Tresor oder Vorführdaten).
    pub index: Index,
    pub demo: bool,
    pub sessions: Manager,
    pub workspace: Workspace,
    unlocking: Option<Receiver<Result<Vault, String>>>,
    lights: std::collections::HashMap<Uuid, Light>,
}

impl App {
    pub fn new(demo: bool) -> Self {
        let config_path = rj_core::config::default_config_path();
        let config = config_path.as_deref().map(Config::load).and_then(Result::ok).unwrap_or_default();
        let reconnect = Reconnect { auto: config.connection.auto_reconnect, max: Duration::from_secs(config.connection.reconnect_max_secs as u64) };
        App {
            sessions: Manager::new(Manager::default_worker_dir(), reconnect),
            config,
            config_path,
            vault: None,
            index: if demo { tree::demo_index(500) } else { Index::default() },
            demo,
            workspace: Workspace::default(),
            unlocking: None,
            lights: Default::default(),
        }
    }

    pub fn kdbx_path(&self) -> Option<String> {
        self.config.kdbx_path.as_ref().map(|p| p.display().to_string())
    }

    /// Entsperren im Hintergrund; das Ergebnis kommt über `poll`.
    pub fn unlock(&mut self, path: String, password: Secret, key_file: Option<String>) {
        let storage = self.config.index_storage;
        let (tx, rx) = mpsc::channel();
        self.unlocking = Some(rx);
        std::thread::spawn(move || {
            let mut key = MasterKey { password: Some(password), key_file: None };
            if let Some(k) = key_file.filter(|k| !k.is_empty()) {
                match key.clone().with_key_file(std::path::Path::new(&k)) {
                    Ok(k2) => key = k2,
                    Err(e) => {
                        let _ = tx.send(Err(format!("Schlüsseldatei: {e}")));
                        return;
                    }
                }
            }
            let _ = tx.send(Vault::open(std::path::Path::new(&path), key, storage).map_err(|e| e.to_string()));
        });
        if let Some(p) = self.config_path.clone() {
            let _ = p;
        }
    }

    pub fn lock(&mut self) {
        self.vault = None;
        if !self.demo {
            self.index = Index::default();
        }
    }

    pub fn save(&mut self) -> Result<bool, String> {
        match self.vault.as_mut() {
            Some(v) => v.save(None).map_err(|e| e.to_string()),
            None => Ok(false),
        }
    }

    pub fn tree_json(&self, filter: &str) -> String {
        let lights = |id: Uuid| self.lights.get(&id).copied();
        serde_json::to_string(&tree::tree(&self.index, filter, &lights)).unwrap_or_else(|_| "[]".into())
    }

    pub fn set_light(&mut self, id: Uuid, light: Light) {
        self.lights.insert(id, light);
    }

    fn target(&self, id: Uuid) -> Option<&Target> {
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
            if let Ok(Some((tunnel, _))) = rj_core::inherit::resolve_tunnel(&self.index, id) {
                v["tunnel"] = json!(self.index.tunnels.iter().find(|t| t.id == tunnel).map(|t| t.name.clone()));
            }
        }
        v.to_string()
    }

    fn payload(&self, r: &ResolvedCredential, protocol: Protocol) -> Result<CredentialPayload, String> {
        let vault = self.vault.as_ref().ok_or("KDBX ist gesperrt")?;
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

    fn options_json(&self, t: &Target) -> String {
        let mut v = match &t.options {
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
    pub fn connect(&mut self, target: Uuid, credential: Option<&str>, remember: bool, width: u32, height: u32) -> String {
        let Some(t) = self.target(target).cloned() else { return json!({ "error": "Ziel nicht gefunden" }).to_string() };
        let resolved = match credential {
            Some(name) => match resolve_credential_named(&self.index, target, name) {
                Ok(Some(r)) => r,
                _ => return json!({ "error": format!("Zugangsdatensatz „{name}“ nicht gefunden") }).to_string(),
            },
            None => match resolve_credential(&self.index, target) {
                Ok(CredentialResolution::Found(r)) => r,
                Ok(CredentialResolution::Ambiguous(list)) => {
                    let choices: Vec<_> = list
                        .iter()
                        .map(|c| json!({ "name": c.name, "title": c.reference.last_title }))
                        .collect();
                    return json!({ "ambiguous": choices }).to_string();
                }
                Ok(CredentialResolution::NotFound { name }) => {
                    return json!({ "error": format!("Kein Zugangsdatensatz „{name}“ am Ziel oder in einem Elternordner") }).to_string();
                }
                Err(e) => return json!({ "error": e.to_string() }).to_string(),
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
        let payload = match self.payload(&resolved, t.protocol) {
            Ok(p) => p,
            Err(e) => {
                return json!({ "missing": { "name": resolved.name, "title": resolved.reference.last_title, "reason": e } }).to_string();
            }
        };
        let (w, h) = if t.protocol == Protocol::Ssh { (width.clamp(20, 500), height.clamp(5, 300)) } else { (width.max(200), height.max(200)) };
        let spec = ConnectSpec {
            target,
            title: t.name.clone(),
            protocol: t.protocol,
            request: ConnectRequest {
                host: t.host.clone(),
                port: t.effective_port(),
                width: w,
                height: h,
                options_json: self.options_json(&t),
                connect_timeout_secs: self.config.connection.connect_timeout_secs,
                keepalive_secs: self.config.connection.keepalive_secs,
            },
            credentials: payload,
        };
        let s = self.sessions.open(spec);
        self.workspace.open(s, &rj_core::charset::mark_visible(&t.name, false));
        json!({ "session": s }).to_string()
    }

    /// Vorführsitzungen für Bildschirmfotos.
    pub fn open_demo_sessions(&mut self) {
        let sample = "\x1b[1;32mrjtest@srv-01\x1b[0m:\x1b[1;34m~\x1b[0m$ apt list --upgradable\r\n\
            Auflistung… Fertig\r\n\x1b[32mopenssl\x1b[0m/noble-updates 3.0.13-0ubuntu3.5 amd64 [aktualisierbar]\r\n\
            \x1b[32mlibssl3\x1b[0m/noble-updates 3.0.13-0ubuntu3.5 amd64 [aktualisierbar]\r\n\
            Grüße aus dem Terminal: äöü ß € 🚀 日本\r\n\x1b[1;32mrjtest@srv-01\x1b[0m:\x1b[1;34m~\x1b[0m$ ";
        let a = self.sessions.open_demo_terminal("ssh-001", sample, 90, 20);
        let b = self.sessions.open_demo_frame("rdp-002", 800, 600, 120);
        let c = self.sessions.open_demo_frame("vnc-003", 800, 600, 30);
        let d = self.sessions.open_demo_terminal("ssh-004", "root@labor:~# uptime\r\n 10:15:02 up 12 days,  3:04,  1 user,  load average: 0,08, 0,05, 0,01\r\nroot@labor:~# ", 90, 20);
        let t1 = self.workspace.open(a, "ssh-001");
        self.workspace.open(b, "rdp-002");
        self.workspace.tile(b, t1);
        self.workspace.open(c, "vnc-003");
        self.workspace.tile(c, t1);
        self.workspace.open(d, "ssh-004");
        self.workspace.tile(d, t1);
        self.workspace.set_arrangement(t1, Arrangement::TwoByTwo);
        let e = self.sessions.open_demo_terminal("ssh-005", "admin@netz-01:~$ ip -br addr\r\nlo   UNKNOWN 127.0.0.1/8\r\neth0 UP      192.0.2.15/24\r\nadmin@netz-01:~$ ", 90, 20);
        self.workspace.open(e, "ssh-005");
        self.workspace.activate(t1);
    }

    pub fn poll(&mut self) -> Vec<UiEvent> {
        let mut out = self.sessions.poll(Duration::from_millis(0));
        if let Some(rx) = &self.unlocking {
            if let Ok(r) = rx.try_recv() {
                self.unlocking = None;
                match r {
                    Ok(v) => {
                        self.index = v.index().index.clone();
                        let mut n = 0;
                        self.index.for_each_target(|_| n += 1);
                        self.vault = Some(v);
                        out.push(UiEvent::Unlocked { targets: n });
                    }
                    Err(reason) => out.push(UiEvent::UnlockFailed { reason }),
                }
            }
        }
        out
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
        assert_eq!(app.workspace.tabs.len(), 2);
        assert_eq!(app.workspace.tabs[0].sessions.len(), 4);
        assert!(app.tree_json("").len() > 10_000);
    }
}
