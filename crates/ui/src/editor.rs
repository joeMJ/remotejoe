//! Bearbeiten von Ordnern, Zielen, Ansichten und Vorlagen aus der
//! Oberfläche. Befehle kommen als JSON (`{"op": …}`) und antworten mit JSON.
//!
//! Jede Änderung geht über den Index der App und wird an den Tresor
//! weitergegeben (`Vault::set_index`); gespeichert wird über `Vault::save`
//! mit Sicherungskopie, atomarem Schreiben, Sperre und Konfliktabfrage.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use rj_core::edit::{self, ApplyKind, EditError};
use rj_core::inherit::{credential_mode, reachable_credential_names, resolve_credential_at, CredentialMode, CredentialResolution, Origin};
use rj_core::{
    ClipboardDirection, CredentialRef, Folder, Index, MacroDef, Node, Protocol, ProtocolOptions, PsrpTransport, Secret, SnmpSetting, TaskDef,
    TunnelSetting,
};
use rj_kdbx::{ConflictChoice, CredentialData};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::app::App;
use crate::workspace::SessionId;

/// Gestaffelter Verbindungsaufbau für „Alle im Ordner verbinden“.
#[derive(Debug, Default)]
pub struct Stagger {
    limit: usize,
    queue: VecDeque<Uuid>,
    active: Vec<(Uuid, SessionId)>,
    /// Ergebnis je Ziel: Sitzung oder Fehlermeldung.
    pub results: Vec<(Uuid, Result<SessionId, String>)>,
    /// Größte beobachtete Zahl gleichzeitiger Verbindungsaufbauten.
    pub peak: usize,
}

impl Stagger {
    pub fn new(targets: Vec<Uuid>, limit: usize) -> Self {
        Stagger { limit: limit.max(1), queue: targets.into(), ..Default::default() }
    }

    pub fn is_done(&self) -> bool {
        self.queue.is_empty() && self.active.is_empty()
    }

    /// Ein Schritt: fertige Aufbauten austragen (Zustand nicht mehr
    /// „connecting“), dann bis zur Grenze neue starten. Ein Fehler bei einem
    /// Ziel hält die übrigen nicht auf.
    pub fn tick(&mut self, state: impl Fn(SessionId) -> Option<String>, mut start: impl FnMut(Uuid) -> Result<SessionId, String>) {
        self.active.retain(|(_, s)| matches!(state(*s).as_deref(), Some("") | Some("connecting")));
        while self.active.len() < self.limit {
            let Some(t) = self.queue.pop_front() else { break };
            match start(t) {
                Ok(s) => {
                    self.active.push((t, s));
                    self.results.push((t, Ok(s)));
                }
                Err(e) => self.results.push((t, Err(e))),
            }
        }
        self.peak = self.peak.max(self.active.len());
    }
}

fn err(e: impl std::fmt::Display) -> String {
    json!({ "error": e.to_string() }).to_string()
}

fn ok(v: Value) -> String {
    v.to_string()
}

fn id_of(v: &Value, key: &str) -> Option<Uuid> {
    v.get(key).and_then(Value::as_str).and_then(|s| Uuid::parse_str(s).ok())
}

/// Zu löschende Knoten: `ids`, sonst `id`.
fn delete_ids(c: &Value) -> Vec<Uuid> {
    let ids = ids_of(c, "ids");
    if ids.is_empty() {
        id_of(c, "id").into_iter().collect()
    } else {
        ids
    }
}

fn ids_of(v: &Value, key: &str) -> Vec<Uuid> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok())).collect())
        .unwrap_or_default()
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// Zeichensatzprüfung eines Eingabefelds: Anzeige mit Markierungen und Zahl
/// der Funde. Kennwörter werden nur geprüft, nie verändert.
pub fn check_text(text: &str, multiline: bool) -> Value {
    let findings = rj_core::charset::find_suspicious(text, multiline);
    json!({
        "marked": rj_core::charset::mark_visible(text, multiline),
        "issues": findings.len(),
    })
}

fn protocol_from(kind: &str) -> Option<Protocol> {
    Some(match kind {
        "rdp" => Protocol::Rdp,
        "ssh" => Protocol::Ssh,
        "vnc" => Protocol::Vnc,
        "web" => Protocol::Web,
        "proxmox" => Protocol::Proxmox,
        "hyperv" => Protocol::Hyperv,
        "psrp" => Protocol::Psrp,
        _ => return None,
    })
}

fn clip_str(c: ClipboardDirection) -> &'static str {
    match c {
        ClipboardDirection::Off => "off",
        ClipboardDirection::HostToTarget => "host_to_target",
        ClipboardDirection::TargetToHost => "target_to_host",
        ClipboardDirection::Both => "both",
    }
}

fn clip_parse(s: &str) -> Option<ClipboardDirection> {
    serde_json::from_value(json!(s)).ok()
}

/// Prüft die Felder der Registerkarte „Verbindung“. Liefert Fehler je Feld.
pub fn validate_connection(protocol: Protocol, host: &str, port: Option<&Value>, options: Option<&Value>) -> BTreeMap<String, String> {
    let mut e = BTreeMap::new();
    if host.is_empty() {
        e.insert("host".into(), "Host darf nicht leer sein".into());
    } else if host.chars().any(|c| c.is_whitespace() || c.is_control()) {
        e.insert("host".into(), "Host enthält Leer- oder Steuerzeichen".into());
    } else if rj_core::charset::find_suspicious(host, false).len() > 0 {
        e.insert("host".into(), "Host enthält unsichtbare oder richtungsändernde Zeichen".into());
    }
    if let Some(p) = port.filter(|p| !p.is_null() && p.as_str() != Some("")) {
        let n = p.as_u64().or_else(|| p.as_str().and_then(|s| s.parse().ok()));
        if !matches!(n, Some(1..=65535)) {
            e.insert("port".into(), "Port muss zwischen 1 und 65535 liegen".into());
        }
    }
    if let Some(o) = options {
        if matches!(protocol, Protocol::Web | Protocol::Proxmox) {
            if let Some(url) = o.get("url").and_then(Value::as_str).filter(|u| !u.is_empty()) {
                let lower = url.to_ascii_lowercase();
                if !(lower.starts_with("https://") || lower.starts_with("http://")) {
                    e.insert("url".into(), "Adresse muss mit https:// oder http:// beginnen".into());
                } else if url.split("://").nth(1).and_then(|r| r.split('/').next()).is_some_and(|auth| auth.contains('@')) {
                    e.insert("url".into(), "Zugangsdaten in der Adresse sind nicht erlaubt".into());
                }
            }
        }
        if matches!(protocol, Protocol::Rdp | Protocol::Ssh) {
            if let Some(a) = o.get("auto_keys").filter(|a| a.is_object()) {
                match serde_json::from_value::<rj_core::autokeys::AutoKeys>(a.clone()) {
                    Ok(a) => {
                        if let Err(m) = a.validate(protocol == Protocol::Ssh) {
                            e.insert("auto_keys".into(), m);
                        }
                    }
                    Err(x) => {
                        e.insert("auto_keys".into(), format!("Tastenfolge: {x}"));
                    }
                }
            }
        }
        if let Some(l) = o.get("clipboard_limit").filter(|l| !l.is_null()) {
            if l.as_u64().is_none() {
                e.insert("clipboard_limit".into(), "Größenlimit muss eine ganze Zahl sein".into());
            }
        }
    }
    e
}

fn options_from(protocol: Protocol, v: &Value) -> Result<ProtocolOptions, String> {
    // Zahlenfelder vorab prüfen: die Meldung von serde nennt weder das Feld noch ist sie deutsch.
    const NUMERIC: &[(&str, &str)] = &[("width", "Breite"), ("height", "Höhe"), ("keepalive_secs", "Keepalive"), ("clipboard_limit", "Größenlimit")];
    for (key, label) in NUMERIC {
        if let Some(x) = v.get(*key).filter(|x| !x.is_null()) {
            if x.as_u64().is_none_or(|n| n > u32::MAX as u64) {
                return Err(format!("Optionen: {label} muss eine ganze Zahl ab 0 sein (eingegeben: {})", x.as_str().map_or_else(|| x.to_string(), str::to_string)));
            }
        }
    }
    let r = match protocol {
        Protocol::Rdp | Protocol::Hyperv => serde_json::from_value(v.clone()).map(ProtocolOptions::Rdp),
        Protocol::Ssh => serde_json::from_value(v.clone()).map(ProtocolOptions::Ssh),
        Protocol::Vnc => serde_json::from_value(v.clone()).map(ProtocolOptions::Vnc),
        Protocol::Web | Protocol::Proxmox => serde_json::from_value(v.clone()).map(ProtocolOptions::Web),
        Protocol::Psrp => return Ok(ProtocolOptions::None),
    };
    r.map_err(|e| format!("Optionen: ungültiger Wert ({e})"))
}

fn options_value(o: &ProtocolOptions) -> Value {
    match o {
        ProtocolOptions::Rdp(x) => serde_json::to_value(x).unwrap_or_default(),
        ProtocolOptions::Ssh(x) => serde_json::to_value(x).unwrap_or_default(),
        ProtocolOptions::Vnc(x) => serde_json::to_value(x).unwrap_or_default(),
        ProtocolOptions::Web(x) => serde_json::to_value(x).unwrap_or_default(),
        ProtocolOptions::None => json!({}),
    }
}

impl App {
    fn folder_name(&self, id: Uuid) -> String {
        edit::folder(&self.index, id).map(|f| rj_core::charset::mark_visible(&f.name, false).into_owned()).unwrap_or_default()
    }

    fn origin_text(&self, origin: Origin, is_target: bool) -> String {
        match origin {
            Origin::Target => "am Ziel gesetzt".into(),
            Origin::Folder(_) if !is_target => "am Ordner gesetzt oder geerbt".into(),
            Origin::Folder(id) => format!("geerbt von Ordner „{}“", self.folder_name(id)),
            Origin::Root => "geerbt von der Wurzel".into(),
        }
    }

    /// Übernimmt den geänderten Index in den Tresor und merkt den vorherigen
    /// Zustand für „Rückgängig“.
    fn commit(&mut self, label: &str, before: Index) {
        self.history.record(label, &before);
        self.push_index();
    }

    fn push_index(&mut self) {
        if let Some(v) = self.vault.as_mut() {
            let _ = v.set_index(self.index.clone());
        }
    }

    /// Daten für den Eigenschaften-Tab eines Knotens (ohne Kennwörter).
    pub fn editor_json(&self, id: Uuid) -> String {
        let is_root = id == self.index.root.id;
        let node = if is_root { None } else { edit::node(&self.index, id) };
        if !is_root && node.is_none() {
            return err("Objekt nicht gefunden");
        }
        let (settings, meta, name) = match node {
            None => (&self.index.root.settings, &self.index.root.meta, self.index.root.name.as_str()),
            Some(Node::Folder(f)) => (&f.settings, &f.meta, f.name.as_str()),
            Some(Node::Target(t)) => (&t.settings, &t.meta, t.name.as_str()),
        };
        let mut v = json!({
            "id": id,
            "kind": match node { None => "root", Some(Node::Folder(_)) => "folder", Some(Node::Target(_)) => "target" },
            "name": name,
            "name_check": check_text(name, false),
            "description": meta.description,
            "notes": meta.notes,
            "fields": meta.fields,
            "parent": edit::parent_of(&self.index, id),
            "folders": edit::folder_list(&self.index).into_iter()
                .filter(|(f, _)| *f != id && !matches!(node, Some(Node::Folder(x)) if edit::folder(&self.index, *f).is_some() && x_contains(x, *f)))
                .map(|(f, p)| json!({ "id": f, "path": if p.is_empty() { "(Wurzel)".to_string() } else { p } })).collect::<Vec<_>>(),
            "favorite": self.index.favorites.contains(&id),
            "unlocked": self.vault.is_some(),
            "tunnels": self.index.tunnels.iter().map(|t| json!({ "id": t.id, "name": t.name })).collect::<Vec<_>>(),
            "devices": self.index.snmp_devices.iter().map(|d| json!({ "id": d.id, "name": d.name })).collect::<Vec<_>>(),
            "tasks": self.index.tasks.iter().map(|t| json!({ "id": t.id, "name": t.name, "kind": t.kind, "params": t.params, "linked": meta.tasks.contains(&t.id) })).collect::<Vec<_>>(),
            "macros": self.index.macros.iter().map(|m| json!({ "id": m.id, "name": m.name, "steps": m.steps, "linked": meta.macros.contains(&m.id) })).collect::<Vec<_>>(),
            "tunnel": match settings.tunnel { None => json!("inherit"), Some(TunnelSetting::None) => json!("none"), Some(TunnelSetting::Use(t)) => json!(t) },
            "snmp": match settings.snmp { None => json!("inherit"), Some(SnmpSetting::None) => json!("none"), Some(SnmpSetting::Device(d)) => json!(d) },
            "psrp_transport": settings.psrp_transport,
        });
        if let Some(Node::Target(t)) = node {
            v["protocol"] = json!(t.protocol.as_str());
            v["host"] = json!(t.host);
            if t.protocol == Protocol::Web && !t.host.is_empty() {
                v["default_url"] = json!(rj_core::weburl::default_url(&t.host, t.effective_port()));
            }
            v["port"] = json!(t.port);
            v["default_port"] = json!(t.protocol.default_port());
            v["options"] = options_value(&t.options);
            v["templates"] = json!(self.index.templates.iter().filter(|x| x.protocol == t.protocol).map(|x| json!({ "id": x.id, "name": x.name, "is_default": x.is_default })).collect::<Vec<_>>());
            if let Ok(Some((tid, origin))) = rj_core::inherit::resolve_tunnel(&self.index, id) {
                let name = self.index.tunnels.iter().find(|x| x.id == tid).map(|x| x.name.clone()).unwrap_or_default();
                v["tunnel_effective"] = json!(format!("{name} ({})", self.origin_text(origin, true)));
            }
        } else {
            v["clipboard"] = json!(settings.clipboard.map(clip_str).unwrap_or("inherit"));
            v["clipboard_effective"] = json!(clip_str(edit::folder_clipboard(&self.index, id)));
        }
        v["inventory"] = json!(match settings.inventory { None => "inherit", Some(true) => "on", Some(false) => "off" });
        if let Some(Node::Target(t)) = node {
            // Eigene Zugangsdaten eines Wegs: Benutzername zeigen, nie das Kennwort.
            v["ways"] = json!(t
                .ways
                .iter()
                .map(|w| {
                    let mut j = way_json(w);
                    if let Some(r) = &w.own {
                        j["username"] = json!(self.vault.as_ref().and_then(|v| v.credential(r.entry)).map(|c| c.data.username).unwrap_or_default());
                    }
                    j
                })
                .collect::<Vec<_>>());
            v["open_with"] = json!(t.open_with);
            // Web-Ziel: Rechner und Port aus der URL, wie beim Verbinden.
            let (host, port) = App::endpoint_of(t);
            v["endpoint"] = json!(format!("{host}:{port}"));
            v["main_way"] = json!(format!("{} · {host}:{port}", t.protocol.label()));
            v["way_logins"] = json!(self.index.logins.iter().map(|l| json!({ "id": l.id, "name": l.name })).collect::<Vec<_>>());
        }
        if let Some(Node::Target(t)) = node {
            // Text für „geerbt“: der Wert ohne eigene Festlegung am Ziel.
            if let Ok((on, src)) = rj_core::inherit::resolve_inventory_inherited(&self.index, t.id) {
                v["inventory_effective"] = json!(format!("{}, {}", if on { "an" } else { "aus" }, inventory_source_text(src, on)));
            }
        }
        v["credential"] = self.credential_section(id, node.is_some_and(|n| matches!(n, Node::Target(_))));
        // Ordner für eine neue Anmeldung aus diesem Tab: der Ordner selbst bzw. der Ordner des Ziels.
        v["folder_for_new"] = json!(match node {
            Some(Node::Target(_)) => edit::parent_of(&self.index, id).unwrap_or(self.index.root.id),
            _ => id,
        });
        // Zweiter Zugangsdatensatz „Sitzung“ (nur Benutzername; das Kennwort verlässt die Schlüsseldatenbank nie).
        let aux = settings.credentials.get(rj_core::autokeys::SESSION_CREDENTIAL).map(|r| {
            let username = self.vault.as_ref().and_then(|v| v.credential(r.entry)).map(|c| c.data.username).unwrap_or_default();
            json!({ "name": rj_core::autokeys::SESSION_CREDENTIAL, "exists": true, "username": username })
        });
        v["aux_credential"] = aux.unwrap_or_else(|| json!({ "name": rj_core::autokeys::SESSION_CREDENTIAL, "exists": false, "username": "" }));
        v.to_string()
    }

    /// Anmeldeobjekte mit Ordnerpfad, Benutzername und Zahl der Verwender (ohne Kennwörter).
    fn logins_list(&self) -> Vec<Value> {
        let mut list: Vec<Value> = self
            .index
            .logins
            .iter()
            .map(|l| {
                let username = self.vault.as_ref().and_then(|v| v.credential(l.entry.entry)).map(|c| c.data.username).unwrap_or_default();
                let path = if l.folder == self.index.root.id {
                    String::new()
                } else {
                    self.index.path_to(l.folder).map(|p| p.iter().map(|n| n.name().to_string()).collect::<Vec<_>>().join(" / ")).unwrap_or_default()
                };
                json!({ "id": l.id, "name": l.name, "folder": l.folder, "path": path, "username": username, "used": rj_core::inherit::login_users(&self.index, l.id).len() })
            })
            .collect();
        list.sort_by(|a, b| a["name"].as_str().unwrap_or_default().to_lowercase().cmp(&b["name"].as_str().unwrap_or_default().to_lowercase()));
        list
    }

    /// Anmeldeobjekt anlegen (ohne `id`) oder ändern: Name, Ordner, Benutzername, Kennwort (leer = unverändert).
    fn login_save(&mut self, c: &Value) -> Result<Value, String> {
        let name = str_of(c, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or("Name der Anmeldung fehlt")?.to_string();
        let folder = id_of(c, "folder").unwrap_or(self.index.root.id);
        if folder != self.index.root.id && edit::folder(&self.index, folder).is_none() {
            return Err("Ordner nicht gefunden".into());
        }
        let username = str_of(c, "username").map(str::to_string);
        // Kennwort byte-genau: kein Trimmen, keine Normalisierung.
        let password = str_of(c, "password").filter(|p| !p.is_empty()).map(Secret::from_str_exact);
        let title = format!("Anmeldung: {name}");
        let existing = id_of(c, "id").and_then(|id| self.index.login(id).cloned());
        let before = self.index.clone();
        let vault = self.vault.as_mut().ok_or("Die Schlüsseldatenbank ist gesperrt")?;
        let (id, entry) = match existing {
            Some(l) => {
                let old = vault.credential(l.entry.entry).ok_or("Eintrag nicht in der Schlüsseldatenbank")?;
                let data = CredentialData { title: title.clone(), username: username.unwrap_or(old.data.username.clone()), password: password.unwrap_or(old.data.password.clone()), ..old.data };
                vault.update_credential(l.entry.entry, data).map_err(|e| e.to_string())?;
                (l.id, l.entry.entry)
            }
            None => {
                let e = vault
                    .add_credential(CredentialData { title: title.clone(), username: username.unwrap_or_default(), password: password.unwrap_or_default(), ..Default::default() })
                    .map_err(|e| e.to_string())?;
                (Uuid::new_v4(), e)
            }
        };
        let login = rj_core::Login { id, name, folder, entry: CredentialRef { entry, last_title: Some(title) } };
        match self.index.logins.iter_mut().find(|l| l.id == id) {
            Some(l) => *l = login,
            None => self.index.logins.push(login),
        }
        rj_core::inherit::sync_logins(&mut self.index);
        self.commit("Anmeldung speichern", before);
        Ok(json!({ "id": id }))
    }

    fn credential_section(&self, id: Uuid, is_target: bool) -> Value {
        let mode = credential_mode(&self.index, id).unwrap_or(CredentialMode::Inherit);
        let mut c = match &mode {
            CredentialMode::Inherit => json!({ "mode": "inherit" }),
            CredentialMode::None => json!({ "mode": "none" }),
            CredentialMode::Named(n) => json!({ "mode": "named", "name": n }),
            CredentialMode::Login(l) => json!({ "mode": "login", "login": l, "login_name": self.index.login(*l).map(|l| l.name.clone()) }),
            CredentialMode::Entry { name, reference } => {
                let own = self.vault.as_ref().is_some_and(|v| v.is_own_entry(reference.entry));
                let username = self.vault.as_ref().and_then(|v| v.credential(reference.entry)).map(|c| c.data.username);
                json!({ "mode": if own { "direct" } else { "entry" }, "name": name, "entry": reference.entry, "title": reference.last_title, "username": username })
            }
        };
        let eff = match resolve_credential_at(&self.index, id) {
            Some(CredentialResolution::Found(r)) => {
                let title = self.vault.as_ref().and_then(|v| v.credential(r.reference.entry)).map(|c| c.data.title).or(r.reference.last_title.clone()).unwrap_or_default();
                json!({ "state": "found", "name": r.name, "title": title, "origin": self.origin_text(r.origin, is_target) })
            }
            Some(CredentialResolution::Disabled) => json!({ "state": "none", "origin": "keine Zugangsdaten" }),
            Some(CredentialResolution::Ambiguous(list)) => json!({ "state": "ambiguous", "names": list.iter().map(|r| r.name.clone()).collect::<Vec<_>>() }),
            Some(CredentialResolution::NotFound { name }) => json!({ "state": "missing", "name": name }),
            None => json!({ "state": "missing" }),
        };
        c["effective"] = eff;
        c["names"] = json!(reachable_credential_names(&self.index, id));
        c["logins"] = json!(self.logins_list());
        c["entries"] = json!(self
            .vault
            .as_ref()
            .map(|v| v.entries().into_iter().map(|e| json!({ "uuid": e.uuid, "title": e.title, "group": e.group_path.join("/") })).collect::<Vec<_>>())
            .unwrap_or_default());
        c
    }

    /// Führt einen Bearbeitungsbefehl aus.
    pub fn edit(&mut self, cmd: &str) -> String {
        let Ok(mut c) = serde_json::from_str::<Value>(cmd) else { return err("Befehl ist kein gültiges JSON") };
        let r = self.edit_value(&c);
        // Ein mitgegebenes Kennwort überschreiben, sobald es verarbeitet ist.
        if let Some(Value::String(pw)) = c.get_mut("credential").and_then(|x| x.get_mut("password")) {
            zeroize::Zeroize::zeroize(pw);
        }
        if let Some(Value::String(pw)) = c.get_mut("aux_credential").and_then(|x| x.get_mut("password")) {
            zeroize::Zeroize::zeroize(pw);
        }
        for k in ["community", "auth_password", "priv_password"] {
            if let Some(Value::String(pw)) = c.get_mut(k) {
                zeroize::Zeroize::zeroize(pw);
            }
        }
        r
    }

    /// Offene Sitzungen der Ziele unter den Knoten (ein Ziel selbst oder ein Ordner samt Unterbaum).
    fn sessions_under(&self, ids: &[Uuid]) -> Vec<SessionId> {
        fn collect(n: &Node, out: &mut HashSet<Uuid>) {
            match n {
                Node::Target(t) => {
                    out.insert(t.id);
                }
                Node::Folder(f) => f.children.iter().for_each(|c| collect(c, out)),
            }
        }
        let mut targets = HashSet::new();
        for id in ids {
            if let Some(n) = self.index.path_to(*id).and_then(|p| p.last().copied()) {
                collect(n, &mut targets);
            }
        }
        self.sessions.ids().into_iter().filter(|s| self.sessions.target(*s).is_some_and(|t| targets.contains(&t))).collect()
    }

    fn edit_value(&mut self, c: &Value) -> String {
        let c = c.clone();
        if str_of(&c, "op") == Some("set_view") {
            self.view = str_of(&c, "view").unwrap_or_default().to_string();
            return ok(json!({ "ok": true }));
        }
        if self.vault.as_ref().is_some_and(|v| v.index().read_only) && !matches!(str_of(&c, "op"), Some("patchday") | Some("snmp_list") | Some("editor") | Some("check_text") | Some("bulk_preview") | Some("apply_preview") | Some("jobs") | Some("cancel_job") | Some("settings") | Some("set_settings") | Some("views") | Some("templates") | Some("undo_label") | Some("connect_all") | Some("delete_preview") | Some("cert_check_start") | Some("cert_check_rows") | Some("cert_check_progress") | Some("cert_check_accept") | Some("logins") | Some("overview") | Some("ov_probe") | Some("ov_query") | Some("ov_field") | Some("ov_run") | Some("ov_stop") | Some("ov_output") | Some("mass_start") | Some("mass_rows") | Some("mass_cancel") | Some("merge_preview") | Some("merge_check") | Some("target_exists") | Some("shared_list") | Some("report_build")) {
            return err("Der Index ist schreibgeschützt (neuere Programmversion)");
        }
        let op = str_of(&c, "op").unwrap_or_default().to_string();
        let r = match op.as_str() {
            "editor" => return id_of(&c, "id").map(|id| self.editor_json(id)).unwrap_or_else(|| err("ID fehlt")),
            "check_text" => return ok(check_text(str_of(&c, "text").unwrap_or_default(), c.get("multiline").and_then(Value::as_bool).unwrap_or(false))),
            "add" => self.op_add(&c),
            "bulk_preview" => self.op_bulk(&c, false),
            "bulk_add" => self.op_bulk(&c, true),
            "update" => self.op_update(&c),
            "move" => self.simple("Verschieben", |idx| {
                let id = id_of(&c, "id").ok_or(EditError::NotFound)?;
                let parent = id_of(&c, "parent").unwrap_or(idx.root.id);
                let pos = c.get("pos").and_then(Value::as_u64).map(|p| p as usize);
                edit::move_node(idx, id, parent, pos).map(|_| json!({ "ok": true }))
            }),
            "reorder" => self.simple("Verschieben", |idx| {
                let id = id_of(&c, "id").ok_or(EditError::NotFound)?;
                edit::reorder(idx, id, c.get("delta").and_then(Value::as_i64).unwrap_or(0) as i32).map(|_| json!({ "ok": true }))
            }),
            "duplicate" => self.op_duplicate(&c),
            "rename" => self.simple("Umbenennen", |idx| {
                let id = id_of(&c, "id").ok_or(EditError::NotFound)?;
                edit::rename(idx, id, str_of(&c, "name").unwrap_or_default()).map(|_| json!({ "ok": true }))
            }),
            "delete_preview" => {
                let doomed = self.sessions_under(&delete_ids(&c));
                let active = doomed.iter().filter(|s| self.sessions.state(**s).is_some_and(|st| !matches!(st, "failed" | "disconnected" | "crashed"))).count();
                return ok(json!({ "sessions": doomed.len(), "active": active }));
            }
            "logins" => Ok(json!({
                "logins": self.logins_list(),
                "root": self.index.root.id,
                "folders": edit::folder_list(&self.index).into_iter()
                    .map(|(f, p)| json!({ "id": f, "path": if p.is_empty() { "(Wurzel)".to_string() } else { p } })).collect::<Vec<_>>(),
                "unlocked": self.vault.is_some(),
            })),
            "login_save" => self.login_save(&c),
            "login_delete" => match id_of(&c, "id") {
                Some(id) => self.simple("Anmeldung löschen", |idx| if rj_core::inherit::delete_login(idx, id) { Ok(json!({ "ok": true })) } else { Err(EditError::NotFound) }),
                None => Err("Anmeldung fehlt".into()),
            },
            "delete" => {
                let ids = delete_ids(&c);
                // Sitzungen der betroffenen Ziele (auch in Unterordnern) merken, bevor sie verschwinden.
                let doomed = self.sessions_under(&ids);
                let r = self.simple("Löschen", |idx| {
                    for id in &ids {
                        // Anmeldeobjekte direkt löschen (sie sind keine Knoten des Baums).
                        if idx.login(*id).is_some() {
                            rj_core::inherit::delete_login(idx, *id);
                            continue;
                        }
                        edit::delete(idx, *id)?;
                    }
                    // Anmeldeobjekte in gelöschten Ordnern entfallen mit (Verwender behalten den Eintrag).
                    let gone: Vec<Uuid> = idx.logins.iter().filter(|l| l.folder != idx.root.id && edit::folder(idx, l.folder).is_none()).map(|l| l.id).collect();
                    for l in gone {
                        rj_core::inherit::delete_login(idx, l);
                    }
                    Ok(json!({ "deleted": ids.len() }))
                });
                r.map(|mut v| {
                    for s in &doomed {
                        self.close_session(*s);
                    }
                    v["closed_sessions"] = json!(doomed);
                    v
                })
            }
            "favorite" => self.simple("Favorit", |idx| {
                let id = id_of(&c, "id").ok_or(EditError::NotFound)?;
                edit::set_favorite(idx, id, c.get("on").and_then(Value::as_bool).unwrap_or(true)).map(|_| json!({ "ok": true }))
            }),
            "view_create" => self.simple("Ansicht anlegen", |idx| {
                let v = edit::create_view(idx, str_of(&c, "name").unwrap_or_default())?;
                edit::add_to_view(idx, v, &ids_of(&c, "ids"))?;
                Ok(json!({ "id": v }))
            }),
            "view_rename" => self.simple("Ansicht umbenennen", |idx| {
                edit::rename_view(idx, id_of(&c, "view").ok_or(EditError::NotFound)?, str_of(&c, "name").unwrap_or_default()).map(|_| json!({ "ok": true }))
            }),
            "view_delete" => self.simple("Ansicht löschen", |idx| edit::delete_view(idx, id_of(&c, "view").ok_or(EditError::NotFound)?).map(|_| json!({ "ok": true }))),
            "view_add" => self.simple("Zur Ansicht hinzufügen", |idx| {
                edit::add_to_view(idx, id_of(&c, "view").ok_or(EditError::NotFound)?, &ids_of(&c, "ids")).map(|n| json!({ "added": n }))
            }),
            "view_remove" => self.simple("Aus Ansicht entfernen", |idx| {
                edit::remove_from_view(idx, id_of(&c, "view").ok_or(EditError::NotFound)?, &ids_of(&c, "ids")).map(|_| json!({ "ok": true }))
            }),
            "apply_preview" => {
                let (Some(id), Some(kind)) = (id_of(&c, "id"), str_of(&c, "kind").and_then(ApplyKind::parse)) else { return err("Angaben fehlen") };
                edit::apply_preview(&self.index, id, kind).map(|n| json!({ "count": n })).map_err(|e| e.to_string())
            }
            "apply" => self.simple("Auf Unterobjekte anwenden", |idx| {
                let id = id_of(&c, "id").ok_or(EditError::NotFound)?;
                let kind = str_of(&c, "kind").and_then(ApplyKind::parse).ok_or(EditError::NotFound)?;
                edit::apply_to_children(idx, id, kind).map(|n| json!({ "count": n }))
            }),
            "template_save" => self.simple("Als Vorlage speichern", |idx| {
                let id = id_of(&c, "id").ok_or(EditError::NotFound)?;
                let Some(Node::Target(t)) = edit::node(idx, id) else { return Err(EditError::NotFound) };
                let name = str_of(&c, "name").filter(|n| !n.is_empty()).ok_or(EditError::EmptyName)?;
                let mut tpl = edit::template_from_target(t, name);
                tpl.is_default = c.get("default").and_then(Value::as_bool).unwrap_or(false);
                let tid = tpl.id;
                edit::upsert_template(idx, tpl);
                Ok(json!({ "id": tid }))
            }),
            "template_delete" => self.simple("Vorlage löschen", |idx| {
                let id = id_of(&c, "id").ok_or(EditError::NotFound)?;
                idx.templates.retain(|t| t.id != id);
                Ok(json!({ "ok": true }))
            }),
            "templates" => Ok(json!(self.index.templates.iter().map(|t| json!({ "id": t.id, "name": t.name, "protocol": t.protocol.as_str(), "is_default": t.is_default })).collect::<Vec<_>>())),
            "views" => Ok(json!(self.index.views.iter().map(|v| json!({ "id": v.id, "name": v.name, "count": v.targets.len() })).collect::<Vec<_>>())),
            "undo" => {
                let before = self.index.referenced_entries();
                match self.history.undo(&mut self.index) {
                    Some(label) => {
                        // Von remotejoe in dieser Sitzung angelegte Einträge, die nach dem Rückgängigmachen niemand
                        // mehr verwendet, zurücknehmen (sonst blieben sie als „Nicht zugeordnet“ in der KDBX).
                        let after = self.index.referenced_entries();
                        let mut withdrawn = 0;
                        if let Some(v) = self.vault.as_mut() {
                            for u in before.difference(&after) {
                                if v.created_this_session(*u) && v.withdraw_created(*u).unwrap_or(false) {
                                    withdrawn += 1;
                                }
                            }
                        }
                        self.push_index();
                        Ok(json!({ "undone": label, "withdrawn": withdrawn }))
                    }
                    None => Err("Nichts rückgängig zu machen".into()),
                }
            }
            "undo_label" => Ok(json!({ "label": self.history.last_label() })),
            "jobs" => Ok(self.jobs.json()),
            "patchday" => Ok(self.patchday_view(id_of(&c, "selected"), c.get("since").and_then(Value::as_u64).unwrap_or(0))),
            "snmp_list" => Ok(json!(self
                .index
                .snmp_devices
                .iter()
                .map(|d| json!({
                    "id": d.id, "name": d.name, "host": d.host, "port": d.port, "version": d.version,
                    "auth_proto": d.auth_proto.unwrap_or(rj_core::SnmpAuth::Sha1),
                    "priv_proto": d.priv_proto.unwrap_or(rj_core::SnmpPriv::Aes128),
                    "username": self.vault.as_ref().and_then(|v| v.credential(d.credential.entry)).map(|c| c.data.username),
                }))
                .collect::<Vec<_>>())),
            "snmp_save" => self.op_snmp_save(&c),
            "snmp_delete" => self.simple("SNMP-Gerät löschen", |idx| {
                let id = id_of(&c, "id").ok_or(EditError::NotFound)?;
                idx.snmp_devices.retain(|d| d.id != id);
                Ok(json!({ "ok": true }))
            }),
            "cancel_job" => {
                let job = c.get("job").and_then(Value::as_u64).unwrap_or(0);
                if self.jobs.cancel(job) {
                    Ok(json!({ "ok": true }))
                } else {
                    Err("Aufgabe läuft nicht mehr".into())
                }
            }
            "export" => self.export_run(&c),
            "settings" => Ok(json!({
                "lock_after_idle_mins": self.config.security.lock_after_idle_mins,
                "disconnect_on_lock": self.config.security.disconnect_on_lock,
                "connect_all_parallel": self.config.connection.connect_all_parallel,
                "keyboard_capture": self.config.connection.keyboard_capture,
                "close_tab_key": self.config.connection.close_tab_key,
                "aruba_tools": self.config.features.aruba,
            })),
            "set_settings" => {
                if let Some(m) = c.get("lock_after_idle_mins").and_then(Value::as_u64) {
                    self.config.security.lock_after_idle_mins = m.min(24 * 60) as u32;
                    self.idle.set_minutes(self.config.security.lock_after_idle_mins, std::time::Instant::now());
                }
                if let Some(d) = c.get("disconnect_on_lock").and_then(Value::as_bool) {
                    self.config.security.disconnect_on_lock = d;
                }
                if let Some(n) = c.get("connect_all_parallel").and_then(Value::as_u64) {
                    self.config.connection.connect_all_parallel = n.clamp(1, 64) as u32;
                }
                if let Some(b) = c.get("keyboard_capture").and_then(Value::as_bool) {
                    self.config.connection.keyboard_capture = b;
                }
                if let Some(k) = c.get("close_tab_key").and_then(Value::as_str).filter(|k| matches!(*k, "outside" | "always" | "never")) {
                    self.config.connection.close_tab_key = k.to_string();
                }
                if let Some(b) = c.get("aruba_tools").and_then(Value::as_bool) {
                    self.config.features.aruba = b;
                }
                match &self.config_path {
                    Some(p) => self.config.save(p).map(|_| json!({ "ok": true })).map_err(|e| e.to_string()),
                    None => Ok(json!({ "ok": true })),
                }
            }
            "save" => return self.save_with(str_of(&c, "choice")),
            "overview" => self.overview_json(&c),
            "ov_probe" => self.overview_probe(&c),
            "ov_query" => self.overview_query(&c),
            "aruba_cluster" => self.aruba_cluster(&c),
            "aruba_ap" => self.aruba_ap(&c),
            "aruba_demo" => self.aruba_demo(&c),
            "ov_field" => self.overview_field(&c),
            "ov_run" => self.overview_run(&c),
            "ov_stop" => {
                self.overview_stop();
                Ok(json!({ "ok": true }))
            }
            "ov_output" => Ok(json!({ "run": self.overview_output(&c) })),
            "ov_notes" => self.overview_notes(&c),
            "ov_inventory" => self.overview_inventory_switch(&c),
            "ways_suggest" => self.ways_suggest(&c),
            "mass_start" => self.mass_start(&c),
            "merge_preview" => self.merge_preview(&c),
            "shared_list" => Ok(self.shared_list()),
            "report_build" => Ok(self.report_build(&c)),
            "shared_split" => self.shared_split(&c),
            "shared_to_login" => self.shared_to_login(&c),
            // Gibt es das Ziel noch? (Übersicht nach einem Fehler beim Auffrischen)
            "target_exists" => Ok(json!({ "exists": id_of(&c, "id").is_some_and(|id| self.target(id).is_some()) })),
            "merge_check" => self.merge_check(&c),
            "merge_apply" => self.merge_apply(&c),
            "mass_rows" => Ok(self.mass_rows()),
            "mass_cancel" => Ok(self.mass_cancel()),
            "connect_all" => self.op_connect_all(&c),
            "cert_check_start" => self.op_cert_check_start(&c),
            "cert_check_rows" => Ok(json!({ "rows": self.cert_rows.iter().map(|r| r.json()).collect::<Vec<_>>() })),
            "cert_check_progress" => Ok(json!({ "done": self.cert_done.load(std::sync::atomic::Ordering::Relaxed), "total": self.cert_total })),
            "cert_check_accept" => {
                // Im Hintergrund: Hunderte Dateien sicher zu schreiben dauert unter Windows Minuten.
                let mut rows = self.cert_rows.clone();
                let n = rows.iter().filter(|r| r.state == crate::certcheck::CheckState::New).count();
                let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
                self.cert_done = done.clone();
                self.cert_total = n;
                let job = self.jobs.spawn(&format!("Schlüssel übernehmen ({n})"), move |job| {
                    let (saved, errors) = crate::certcheck::accept_new_with_progress(&mut rows, &crate::certcheck::Stores::default_location(), &done);
                    Box::new(move |app: &mut App, _cancelled: bool| {
                        let json = json!({ "saved": saved, "errors": errors, "rows": rows.iter().map(|r| r.json()).collect::<Vec<_>>() }).to_string();
                        app.cert_rows = rows;
                        vec![crate::sessions::UiEvent::JobResult { job, kind: "cert_accept".into(), target: String::new(), json }]
                    })
                });
                Ok(json!({ "job": job, "count": n }))
            }
            "update_task" => self.simple("Aufgabe ändern", |idx| {
                let id = id_of(&c, "id").ok_or(EditError::NotFound)?;
                let t = idx.tasks.iter_mut().find(|t| t.id == id).ok_or(EditError::NotFound)?;
                if let Some(n) = str_of(&c, "name") {
                    if n.is_empty() {
                        return Err(EditError::EmptyName);
                    }
                    t.name = n.to_string();
                }
                if let Some(k) = str_of(&c, "kind") {
                    t.kind = k.to_string();
                }
                if let Some(p) = c.get("params").and_then(Value::as_object) {
                    t.params = p.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect();
                }
                Ok(json!({ "ok": true }))
            }),
            "update_macro" => {
                let steps = match c.get("steps") {
                    Some(Value::Array(a)) => Some(a.clone()),
                    Some(Value::String(s)) => match serde_json::from_str::<Vec<Value>>(s) {
                        Ok(a) => Some(a),
                        Err(e) => return err(format!("Schritte sind kein gültiges JSON: {e}")),
                    },
                    _ => None,
                };
                self.simple("Makro ändern", |idx| {
                    let id = id_of(&c, "id").ok_or(EditError::NotFound)?;
                    let m = idx.macros.iter_mut().find(|m| m.id == id).ok_or(EditError::NotFound)?;
                    if let Some(n) = str_of(&c, "name") {
                        if n.is_empty() {
                            return Err(EditError::EmptyName);
                        }
                        m.name = n.to_string();
                    }
                    if let Some(s) = steps {
                        m.steps = s;
                    }
                    Ok(json!({ "ok": true }))
                })
            }
            "macro_list" => Ok(self.macro_list()),
            "macro_get" => self.macro_get(&c),
            "macro_new" => self.simple("Makro anlegen", |idx| {
                let m = MacroDef { id: Uuid::new_v4(), name: "Neues Makro".into(), steps: Vec::new() };
                let id = m.id;
                idx.macros.push(m);
                Ok(json!({ "macro": id }))
            }),
            "macro_delete" => self.simple("Makro löschen", |idx| {
                let id = id_of(&c, "id").ok_or(EditError::NotFound)?;
                if !idx.macros.iter().any(|m| m.id == id) {
                    return Err(EditError::NotFound);
                }
                idx.macros.retain(|m| m.id != id);
                walk_meta(&mut idx.root, &mut |meta| meta.macros.retain(|x| *x != id));
                Ok(json!({ "ok": true }))
            }),
            "macro_duplicate" => self.simple("Makro duplizieren", |idx| {
                let id = id_of(&c, "id").ok_or(EditError::NotFound)?;
                let src = idx.macros.iter().find(|m| m.id == id).cloned().ok_or(EditError::NotFound)?;
                let copy = MacroDef { id: Uuid::new_v4(), name: format!("{} (Kopie)", src.name), steps: src.steps };
                let new = copy.id;
                idx.macros.push(copy);
                Ok(json!({ "macro": new }))
            }),
            "macro_link" | "macro_unlink" => {
                let add = op == "macro_link";
                self.simple(if add { "Makro zuordnen" } else { "Zuordnung des Makros aufheben" }, |idx| {
                    let mid = id_of(&c, "id").ok_or(EditError::NotFound)?;
                    let node = id_of(&c, "node").ok_or(EditError::NotFound)?;
                    if !idx.macros.iter().any(|m| m.id == mid) {
                        return Err(EditError::NotFound);
                    }
                    link(idx, node, |meta| {
                        if add {
                            if !meta.macros.contains(&mid) {
                                meta.macros.push(mid);
                            }
                        } else {
                            meta.macros.retain(|x| *x != mid);
                        }
                    })?;
                    Ok(json!({ "ok": true }))
                })
            }
            "session_macros" => Ok(self.session_macros(c.get("session").and_then(Value::as_u64).unwrap_or(0))),
            "run_macro" => self.run_macro(&c),
            "cancel_macro" => {
                let s = c.get("session").and_then(Value::as_u64).unwrap_or(0);
                for r in self.macro_runs.iter().filter(|r| r.session == s) {
                    r.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                Ok(json!({ "ok": true }))
            }
            "update_status" => Ok(self.update_status()),
            "update_settings" => self.update_settings(&c),
            "update_demo_source" => self.update_demo_source(str_of(&c, "base_url").unwrap_or_default()),
            "update_check" => self.update_check(),
            "update_install" => self.update_install(c.get("confirm").and_then(Value::as_bool).unwrap_or(false)),
            "update_cancel" => Ok(self.update_cancel()),
            "update_apply" => self.update_apply(c.get("confirm").and_then(Value::as_bool).unwrap_or(false)),
            "web_command" => {
                let s = c.get("session").and_then(Value::as_u64).unwrap_or(0);
                match c.get("cmd") {
                    Some(cmd) => self.sessions.web_command(s, cmd).map(|_| json!({ "ok": true })),
                    None => Err("Befehl fehlt".into()),
                }
            }
            _ => Err(format!("Unbekannter Befehl „{op}“")),
        };
        match r {
            Ok(v) => ok(v),
            Err(e) => err(e),
        }
    }

    /// Einfache Index-Änderung mit Verlauf.
    /// Alle Makros mit der Zahl der Ziele und Ordner, denen sie zugeordnet sind.
    fn macro_list(&self) -> Value {
        let mut counts: BTreeMap<Uuid, usize> = BTreeMap::new();
        count_meta(&self.index.root, &mut |meta| {
            for m in &meta.macros {
                *counts.entry(*m).or_default() += 1;
            }
        });
        json!(self.index.macros.iter().map(|m| json!({ "id": m.id, "name": m.name, "steps": m.steps.len(), "links": counts.get(&m.id).copied().unwrap_or(0) })).collect::<Vec<_>>())
    }

    fn macro_get(&self, c: &Value) -> Result<Value, String> {
        let id = id_of(c, "id").ok_or("Makro fehlt")?;
        let m = self.index.macros.iter().find(|m| m.id == id).ok_or("Makro nicht gefunden")?;
        let mut linked = Vec::new();
        fn walk(f: &rj_core::Folder, id: Uuid, root: bool, out: &mut Vec<Value>) {
            if f.meta.macros.contains(&id) {
                out.push(json!({ "id": f.id, "name": if root { "(Wurzel)".to_string() } else { f.name.clone() }, "kind": "folder" }));
            }
            for n in &f.children {
                match n {
                    Node::Folder(s) => walk(s, id, false, out),
                    Node::Target(t) if t.meta.macros.contains(&id) => out.push(json!({ "id": t.id, "name": t.name, "kind": "target" })),
                    Node::Target(_) => {}
                }
            }
        }
        walk(&self.index.root, id, true, &mut linked);
        Ok(json!({ "id": m.id, "name": m.name, "steps": m.steps, "linked": linked }))
    }

    /// Makros, die für eine Sitzung gelten: die des Ziels und der Ordner darüber (geerbt).
    fn session_macros(&self, s: SessionId) -> Value {
        let Some(target) = self.sessions.target(s) else { return json!([]) };
        let mut sources: Vec<(String, &rj_core::Meta)> = Vec::new();
        if let Some(path) = self.index.path_to(target) {
            for n in path.iter().rev() {
                match n {
                    Node::Target(t) => sources.push((String::new(), &t.meta)),
                    Node::Folder(f) => sources.push((f.name.clone(), &f.meta)),
                }
            }
        }
        sources.push(("(Wurzel)".into(), &self.index.root.meta));
        let mut seen = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        for (origin, meta) in sources {
            for id in &meta.macros {
                if let Some(m) = self.index.macros.iter().find(|m| m.id == *id) {
                    if seen.insert(m.id) {
                        out.push(json!({ "id": m.id, "name": m.name, "origin": origin, "steps": m.steps.len() }));
                    }
                }
            }
        }
        json!(out)
    }

    /// Startet ein Makro in einer verbundenen SSH-Sitzung.
    fn run_macro(&mut self, c: &Value) -> Result<Value, String> {
        use rj_core::Protocol;
        let s = c.get("session").and_then(Value::as_u64).ok_or("Sitzung fehlt")?;
        let mid = id_of(c, "macro").ok_or("Makro fehlt")?;
        if self.sessions.protocol(s) != Some(Protocol::Ssh) {
            return Err("Makros laufen zunächst nur in SSH-Sitzungen.".into());
        }
        if self.sessions.state(s) != Some("connected") {
            return Err("Die Sitzung ist nicht verbunden.".into());
        }
        if self.macro_runs.iter().any(|r| r.session == s) {
            return Err("In dieser Sitzung läuft schon ein Makro.".into());
        }
        let def = self.index.macros.iter().find(|m| m.id == mid).cloned().ok_or("Makro nicht gefunden")?;
        if def.steps.is_empty() {
            return Err("Das Makro hat noch keine Schritte.".into());
        }
        let mac: rj_tasks::macros::Macro = serde_json::from_value(json!({ "steps": def.steps })).map_err(|e| format!("Das Makro enthält ungültige Schritte: {e}"))?;
        let cred = self.sessions.credentials(s).ok_or("Sitzung nicht gefunden")?;
        let creds = crate::macro_run::Creds {
            password: (!cred.password.0.is_empty()).then(|| rj_core::Secret::from_bytes(cred.password.0.clone())),
            username: cred.username.clone(),
            domain: cred.domain.clone(),
        };
        let (tap_tx, tap_rx) = std::sync::mpsc::channel();
        // Die Eingabeaufforderung steht meist schon da: als „bisherige Ausgabe“ vorlegen.
        if let Some(text) = self.sessions.screen_text(s) {
            let _ = tap_tx.send(text.into_bytes());
        }
        self.sessions.attach_macro_tap(s, tap_tx);
        let run = crate::macro_run::spawn(s, def.name.clone(), mac, creds, tap_rx, self.macro_input_tx.clone());
        self.macro_runs.push(run);
        Ok(json!({ "started": true, "name": def.name }))
    }

    /// Vorschau „Gleiche Adressen zusammenfassen“: Gruppen unter der Auswahl mit vorgeschlagenem Hauptziel.
    fn merge_preview(&self, c: &Value) -> Result<Value, String> {
        use rj_core::merge;
        let scope = ids_of(c, "ids");
        // „Zu einem Ziel zusammenfassen“: die markierten Ziele als eine Gruppe, gleich welche Adresse.
        let found = if c.get("manual").and_then(Value::as_bool).unwrap_or(false) {
            let sel = merge::selection(&self.index, &scope);
            if sel.len() > 1 { vec![("Auswahl".to_string(), sel)] } else { Vec::new() }
        } else {
            merge::groups(&self.index, &scope)
        };
        let groups: Vec<Value> = found
            .into_iter()
            .map(|(address, members)| {
                let keep = merge::default_keep(&self.index, &members);
                let list: Vec<Value> = members
                    .iter()
                    .filter_map(|id| self.target(*id))
                    .map(|t| {
                        let (host, port) = App::endpoint_of(t);
                        let path: Vec<String> = self.index.path_to(t.id).unwrap_or_default().iter().map(|n| rj_core::charset::mark_visible(n.name(), false).into_owned()).collect();
                        json!({
                            "id": t.id,
                            "name": rj_core::charset::mark_visible(&t.name, false),
                            "protocol": t.protocol.as_str(),
                            "label": t.protocol.label(),
                            "endpoint": format!("{}:{port}", rj_core::charset::mark_visible(&host, false)),
                            "ways": t.ways.len(),
                            "path": path[..path.len().saturating_sub(1)].join(" › "),
                        })
                    })
                    .collect();
                let name = keep.and_then(|k| self.target(k)).map(|t| t.name.clone()).unwrap_or_default();
                json!({ "address": address, "members": list, "keep": keep, "name": name })
            })
            .collect();
        Ok(json!({ "groups": groups }))
    }

    /// Hinweise für eine Gruppe (Probelauf auf einer Kopie des Index).
    fn merge_check(&self, c: &Value) -> Result<Value, String> {
        let keep = id_of(c, "keep").ok_or("Hauptziel fehlt")?;
        let mut idx = self.index.clone();
        let r = rj_core::merge::merge(&mut idx, keep, &ids_of(c, "others"), "").map_err(|e| e.to_string())?;
        Ok(json!({ "ways": r.ways, "own": r.own, "notes": r.notes }))
    }

    /// Fasst die gewählten Gruppen zusammen; ein Schritt zum Rückgängigmachen.
    fn merge_apply(&mut self, c: &Value) -> Result<Value, String> {
        let groups: Vec<(Uuid, Vec<Uuid>, String)> = c
            .get("groups")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|g| Some((id_of(g, "keep")?, ids_of(g, "others"), str_of(g, "name").unwrap_or_default().to_string()))).collect())
            .unwrap_or_default();
        if groups.iter().all(|(_, o, _)| o.is_empty()) {
            return Err("Keine Gruppe gewählt".into());
        }
        // Angenommene Web-Zertifikate der aufgelösten Ziele: deren Port merken, solange es sie noch gibt.
        let carry: Vec<(Uuid, Uuid, u16)> = groups
            .iter()
            .flat_map(|(keep, others, _)| others.iter().filter(move |o| *o != keep).map(move |o| (*keep, *o)))
            .filter_map(|(keep, o)| self.target(o).map(|t| (keep, o, App::endpoint_of(t).1)))
            .collect();
        let r = self.simple("Ziele zusammenfassen", |idx| {
            let (mut targets, mut ways, mut own, mut notes) = (0, 0, 0, Vec::new());
            for (keep, others, name) in &groups {
                if others.is_empty() {
                    continue;
                }
                let r = rj_core::merge::merge(idx, *keep, others, name)?;
                targets += others.len();
                ways += r.ways;
                own += r.own;
                notes.extend(r.notes);
            }
            Ok(json!({ "merged": targets, "ways": ways, "own": own, "notes": notes }))
        });
        if r.is_ok() {
            carry_web_certs(&carry);
        }
        r
    }

    fn simple(&mut self, label: &str, f: impl FnOnce(&mut Index) -> Result<Value, EditError>) -> Result<Value, String> {
        let before = self.index.clone();
        match f(&mut self.index) {
            Ok(v) => {
                if self.index != before {
                    self.commit(label, before);
                }
                Ok(v)
            }
            Err(e) => {
                self.index = before;
                Err(e.to_string())
            }
        }
    }

    fn op_add(&mut self, c: &Value) -> Result<Value, String> {
        let parent = id_of(c, "parent").unwrap_or(self.index.root.id);
        let kind = str_of(c, "kind").unwrap_or_default();
        let before = self.index.clone();
        let template = id_of(c, "template").and_then(|t| self.index.templates.iter().find(|x| x.id == t).cloned());
        let result = match kind {
            "folder" => {
                let name = str_of(c, "name").filter(|n| !n.is_empty()).unwrap_or("Neuer Ordner");
                edit::insert(&mut self.index, parent, Node::Folder(Folder::new(name)), None).map(|id| json!({ "id": id }))
            }
            "task" => {
                let t = TaskDef { id: Uuid::new_v4(), name: "Neue Aufgabe".into(), kind: "apt".into(), params: BTreeMap::new() };
                let tid = t.id;
                self.index.tasks.push(t);
                link(&mut self.index, parent, |m| m.tasks.push(tid)).map(|_| json!({ "task": tid }))
            }
            // Aruba-AP: SSH als Hauptweg (Inventar), dazu die Web-Oberfläche auf 4343 mit denselben Zugangsdaten.
            // Leere Adresse am Weg: https://<Adresse des Ziels>:4343/ – folgt also der Adresse des Ziels.
            "aruba" => {
                let name = str_of(c, "name").filter(|n| !n.is_empty()).unwrap_or("Neuer Aruba-AP");
                let mut t = edit::new_target(&self.index, parent, Protocol::Ssh, name, None);
                let mut w = rj_core::model::AccessWay::new(Protocol::Web);
                w.name = "Web-Oberfläche".into();
                w.port = Some(4343);
                t.ways.push(w);
                edit::insert(&mut self.index, parent, Node::Target(t), None).map(|id| json!({ "id": id }))
            }
            "macro" => {
                let m = MacroDef { id: Uuid::new_v4(), name: "Neues Makro".into(), steps: Vec::new() };
                let mid = m.id;
                self.index.macros.push(m);
                link(&mut self.index, parent, |m| m.macros.push(mid)).map(|_| json!({ "macro": mid }))
            }
            other => {
                let protocol = template.as_ref().map(|t| t.protocol).or_else(|| protocol_from(other));
                match protocol {
                    Some(p) => {
                        let name = str_of(c, "name").filter(|n| !n.is_empty()).unwrap_or("Neues Ziel");
                        let t = edit::new_target(&self.index, parent, p, name, template.as_ref());
                        edit::insert(&mut self.index, parent, Node::Target(t), None).map(|id| json!({ "id": id }))
                    }
                    None => return Err(format!("Unbekannter Typ „{other}“")),
                }
            }
        };
        match result {
            Ok(v) => {
                self.commit("Hinzufügen", before);
                Ok(v)
            }
            Err(e) => {
                self.index = before;
                Err(e.to_string())
            }
        }
    }

    /// Mehrere Ziele: Vorschau (`apply` = false) oder Anlegen aller gültigen
    /// Zeilen als eine rückgängig machbare Aktion. Antwort: Zeilen mit
    /// Hinweisen, bei `apply` zusätzlich `created` und `remaining` (der Text
    /// der ungültigen Zeilen, der im Eingabefeld stehen bleibt).
    fn op_bulk(&mut self, c: &Value, apply: bool) -> Result<Value, String> {
        let parent = id_of(c, "parent").unwrap_or(self.index.root.id);
        let template = id_of(c, "template").and_then(|t| self.index.templates.iter().find(|x| x.id == t).cloned());
        let protocol = template.as_ref().map(|t| t.protocol).or_else(|| str_of(c, "kind").and_then(protocol_from)).ok_or("Zieltyp fehlt")?;
        if edit::folder(&self.index, parent).is_none() {
            return Err("Ordner nicht gefunden".into());
        }
        let pattern = str_of(c, "pattern").unwrap_or(rj_core::bulk::DEFAULT_PATTERN);
        let port_default = template.as_ref().and_then(|t| t.port).or_else(|| edit::default_template(&self.index, protocol).and_then(|t| t.port));
        let rows = rj_core::bulk::plan(&self.index, parent, protocol, str_of(c, "text").unwrap_or_default(), pattern, port_default).map_err(|e| e.to_string())?;
        let rows_json: Vec<Value> = rows
            .iter()
            .map(|r| {
                json!({
                    "line": r.line, "nr": r.nr,
                    "name": rj_core::charset::mark_visible(&r.name, false),
                    "host": rj_core::charset::mark_visible(&r.host, false),
                    "port": r.effective_port, "valid": r.is_valid(),
                    "errors": r.errors, "hints": r.hints,
                })
            })
            .collect();
        let valid = rows.iter().filter(|r| r.is_valid()).count();
        let mut out = json!({ "rows": rows_json, "valid": valid, "invalid": rows.len() - valid, "limit": rj_core::bulk::MAX_ROWS });
        if !apply {
            return Ok(out);
        }
        if valid == 0 {
            return Err("Keine gültige Zeile zum Anlegen".into());
        }
        let before = self.index.clone();
        let ids = match rj_core::bulk::apply(&mut self.index, parent, protocol, &rows, template.as_ref()) {
            Ok(ids) => ids,
            Err(e) => {
                self.index = before;
                return Err(e.to_string());
            }
        };
        if let Some(cred) = c.get("credential").filter(|v| str_of(v, "mode").is_some_and(|m| m != "inherit")) {
            // Bei „direkt“ entsteht ein gemeinsamer KDBX-Eintrag, den alle
            // neuen Ziele verwenden.
            let res = self.apply_credential(ids[0], cred).and_then(|_| {
                let mode = credential_mode(&self.index, ids[0]).ok_or("Objekt nicht gefunden")?;
                for id in &ids[1..] {
                    let m = match &mode {
                        CredentialMode::Entry { reference, .. } => {
                            let name = match edit::node(&self.index, *id) {
                                Some(Node::Target(t)) => t.credential_name().to_string(),
                                _ => rj_core::DEFAULT_CREDENTIAL.into(),
                            };
                            CredentialMode::Entry { name, reference: reference.clone() }
                        }
                        m => m.clone(),
                    };
                    rj_core::inherit::set_credential_mode(&mut self.index, *id, m);
                }
                Ok(())
            });
            if let Err(e) = res {
                self.index = before;
                return Err(e);
            }
        }
        self.commit("Mehrere Ziele anlegen", before);
        out["created"] = json!(ids);
        out["remaining"] = json!(rj_core::bulk::remaining_text(&rows));
        Ok(out)
    }

    /// Übernimmt die Felder des Eigenschaften-Tabs.
    fn op_update(&mut self, c: &Value) -> Result<Value, String> {
        let id = id_of(c, "id").ok_or("ID fehlt")?;
        let f = c.get("fields").cloned().unwrap_or(json!({}));
        let is_root = id == self.index.root.id;
        let node = if is_root { None } else { Some(edit::node(&self.index, id).ok_or("Objekt nicht gefunden")?.clone()) };
        // Prüfen, bevor etwas geändert wird.
        if let Some(Node::Target(t)) = &node {
            // Geprüft werden nur übergebene Felder.
            let mut errors = validate_connection(t.protocol, str_of(&f, "host").unwrap_or("-"), f.get("port"), f.get("options"));
            if f.get("host").is_none() {
                errors.remove("host");
            }
            // Web-Ziele: Die Adresse genügt; Host und Port werden daraus abgeleitet.
            if t.protocol == Protocol::Web {
                let url_now = match &t.options {
                    ProtocolOptions::Web(o) => o.url.clone().unwrap_or_default(),
                    _ => String::new(),
                };
                let url = f.get("options").and_then(|o| o.get("url")).and_then(Value::as_str).map(str::to_string).unwrap_or(url_now);
                let host_now = str_of(&f, "host").map(str::to_string).unwrap_or_else(|| t.host.clone());
                // Ein leerer Host ist bei Web in Ordnung, solange die Adresse da ist; ungültige Zeichen bleiben ein Fehler.
                if errors.get("host").is_some_and(|m| m == "Host darf nicht leer sein") {
                    errors.remove("host");
                }
                if url.is_empty() && host_now.is_empty() {
                    errors.insert("url".into(), "Die Adresse (URL) darf nicht leer sein".into());
                } else if !url.is_empty() && !errors.contains_key("url") && rj_core::weburl::endpoint(&url).is_none() {
                    errors.insert("url".into(), "Adresse ungültig; Beispiel: https://rechner/ oder http://rechner:8080/".into());
                }
            }
            if !errors.is_empty() {
                return Ok(json!({ "errors": errors }));
            }
        }
        // Zugangswege: vollständig prüfen, bevor etwas geändert wird.
        let ways = match (&node, f.get("ways")) {
            (Some(Node::Target(t)), Some(v)) => match ways_from(v, t, &self.index) {
                Ok((w, own)) => Some((w, own)),
                Err(e) => return Ok(json!({ "errors": { "ways": e } })),
            },
            _ => None,
        };
        let open_with: Option<Option<Uuid>> = match f.get("open_with") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(Value::String(s)) if s == "main" || s.is_empty() => Some(None),
            Some(Value::String(s)) => match Uuid::parse_str(s) {
                Ok(u) => Some(Some(u)),
                Err(_) => return Ok(json!({ "errors": { "ways": "Weg für den Doppelklick ungültig" } })),
            },
            Some(_) => return Ok(json!({ "errors": { "ways": "Weg für den Doppelklick ungültig" } })),
        };
        if let Some(n) = str_of(&f, "name") {
            if n.is_empty() && !is_root {
                return Ok(json!({ "errors": { "name": "Name darf nicht leer sein" } }));
            }
        }
        let before = self.index.clone();
        // Zugangsdaten zuerst: „direkt“ legt einen KDBX-Eintrag an.
        if let Some(cred) = c.get("credential") {
            if let Err(e) = self.apply_credential(id, cred) {
                self.index = before;
                return Err(e);
            }
        }
        // Zweiter Zugangsdatensatz „Sitzung“ für die Tastenfolge.
        if let Some(aux) = c.get("aux_credential") {
            if let Err(e) = self.apply_aux_credential(id, aux) {
                self.index = before;
                return Err(e);
            }
        }
        // Eigene Zugangsdaten an Zugangswegen: KDBX-Eintrag anlegen bzw. ändern.
        let ways = match ways {
            Some((mut list, own)) => {
                let target_name = edit::node(&self.index, id).map(|n| n.name().to_string()).unwrap_or_default();
                if let Err(e) = self.apply_way_credentials(&target_name, &mut list, own) {
                    self.index = before;
                    return Err(e);
                }
                Some(list)
            }
            None => None,
        };
        let res = (|| -> Result<(), String> {
            let protocol = match &node {
                Some(Node::Target(t)) => Some(t.protocol),
                _ => None,
            };
            let options = match (protocol, f.get("options")) {
                (Some(p), Some(o)) => Some(options_from(p, o)?),
                _ => None,
            };
            let tunnel = match f.get("tunnel").and_then(Value::as_str) {
                None => None,
                Some("inherit") => Some(None),
                Some("none") => Some(Some(TunnelSetting::None)),
                Some(s) => Some(Some(TunnelSetting::Use(Uuid::parse_str(s).map_err(|_| "Tunnel ungültig")?))),
            };
            let snmp = match f.get("snmp").and_then(Value::as_str) {
                None => None,
                Some("inherit") => Some(None),
                Some("none") => Some(Some(SnmpSetting::None)),
                Some(s) => Some(Some(SnmpSetting::Device(Uuid::parse_str(s).map_err(|_| "SNMP-Gerät ungültig")?))),
            };
            let psrp: Option<Option<PsrpTransport>> = match f.get("psrp_transport") {
                None => None,
                Some(Value::Null) => Some(None),
                Some(Value::String(s)) if s == "inherit" => Some(None),
                Some(v) => Some(Some(serde_json::from_value(v.clone()).map_err(|e| format!("PowerShell-Transport: {e}"))?)),
            };
            let inventory = match f.get("inventory").and_then(Value::as_str) {
                None => None,
                Some("inherit") => Some(None),
                Some("on") => Some(Some(true)),
                Some("off") => Some(Some(false)),
                Some(_) => return Err("Inventar-Einstellung ungültig".into()),
            };
            let clipboard = match f.get("clipboard").and_then(Value::as_str) {
                None => None,
                Some("inherit") => Some(None),
                Some(s) => Some(Some(clip_parse(s).ok_or("Zwischenablage-Richtung ungültig")?)),
            };
            let (settings, meta) = match edit_target(&mut self.index, id) {
                Some(x) => x,
                None => return Err("Objekt nicht gefunden".into()),
            };
            if let Some(t) = tunnel {
                settings.tunnel = t;
            }
            if let Some(s) = snmp {
                settings.snmp = s;
            }
            if let Some(p) = psrp {
                settings.psrp_transport = p;
            }
            if let Some(cl) = clipboard {
                settings.clipboard = cl;
            }
            if let Some(i) = inventory {
                settings.inventory = i;
            }
            if let Some(d) = str_of(&f, "description") {
                meta.description = d.to_string();
            }
            if let Some(n) = str_of(&f, "notes") {
                meta.notes = n.to_string();
            }
            if let Some(fields) = f.get("fields").and_then(Value::as_object) {
                meta.fields = fields.iter().filter(|(k, _)| !k.is_empty()).map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect();
            }
            if f.get("tasks").is_some() {
                meta.tasks = ids_of(&f, "tasks");
            }
            if f.get("macros").is_some() {
                meta.macros = ids_of(&f, "macros");
            }
            if is_root {
                return Ok(());
            }
            match rj_core::inherit::find_node_mut(&mut self.index.root, id) {
                Some(Node::Target(t)) => {
                    if let Some(w) = ways {
                        t.ways = w;
                    }
                    if let Some(o) = open_with {
                        t.open_with = o;
                    }
                    // Doppelklick-Weg muss es geben (gelöschter Weg → Hauptweg).
                    if t.open_with.is_some_and(|o| !t.ways.iter().any(|w| w.id == o)) {
                        t.open_with = None;
                    }
                    if let Some(n) = str_of(&f, "name") {
                        t.name = n.to_string();
                    }
                    if let Some(h) = str_of(&f, "host") {
                        t.host = h.to_string();
                    }
                    if let Some(p) = f.get("port") {
                        t.port = p.as_u64().or_else(|| p.as_str().and_then(|s| s.parse().ok())).map(|p| p as u16);
                    }
                    if let Some(o) = options {
                        t.options = o;
                    }
                    if t.protocol == Protocol::Web {
                        if let ProtocolOptions::Web(o) = &t.options {
                            if let Some((h, p)) = o.url.as_deref().filter(|u| !u.is_empty()).and_then(rj_core::weburl::endpoint) {
                                t.host = h;
                                t.port = (p != t.protocol.default_port()).then_some(p);
                            }
                        }
                    }
                    // Aruba-AP: Web-Weg auf 4343 ohne Adresse bekommt die URL, sobald das Ziel eine Adresse hat
                    // (nach der Übernahme der Adresse aus dem Formular, damit es schon beim ersten Speichern greift).
                    let host = t.host.trim().to_string();
                    if !host.is_empty() {
                        let h = if host.contains(':') && !host.starts_with('[') { format!("[{host}]") } else { host };
                        for w in t.ways.iter_mut().filter(|w| w.protocol == Protocol::Web && w.host.trim().is_empty() && w.port == Some(4343)) {
                            w.host = format!("https://{h}:4343/");
                        }
                    }
                }
                Some(Node::Folder(fo)) => {
                    if let Some(n) = str_of(&f, "name") {
                        fo.name = n.to_string();
                    }
                }
                None => return Err("Objekt nicht gefunden".into()),
            }
            if let Some(p) = id_of(&f, "parent") {
                if edit::parent_of(&self.index, id) != Some(p) {
                    edit::move_node(&mut self.index, id, p, None).map_err(|e| e.to_string())?;
                }
            }
            Ok(())
        })();
        if let Err(e) = res {
            self.index = before;
            return Err(e);
        }
        if self.index != before {
            self.commit("Eigenschaften ändern", before);
        }
        if c.get("save").and_then(Value::as_bool).unwrap_or(false) {
            return serde_json::from_str(&self.save_with(None)).map_err(|e| e.to_string());
        }
        Ok(json!({ "ok": true }))
    }

    /// Setzt die Zugangsdaten nach einer der fünf Möglichkeiten.
    fn apply_credential(&mut self, id: Uuid, cred: &Value) -> Result<(), String> {
        let name_for_entry = match edit::node(&self.index, id) {
            Some(Node::Target(t)) => t.credential_name().to_string(),
            _ => edit::folder(&self.index, id).and_then(|f| f.settings.credential_name.clone()).unwrap_or_else(|| rj_core::DEFAULT_CREDENTIAL.into()),
        };
        let mode = match str_of(cred, "mode").unwrap_or("inherit") {
            "inherit" => CredentialMode::Inherit,
            "none" => CredentialMode::None,
            "named" => CredentialMode::Named(str_of(cred, "name").filter(|n| !n.is_empty()).ok_or("Name des Datensatzes fehlt")?.to_string()),
            "login" => CredentialMode::Login(id_of(cred, "login").filter(|l| self.index.login(*l).is_some()).ok_or("Keine Anmeldung gewählt")?),
            "entry" => {
                let entry = id_of(cred, "entry").ok_or("Kein Eintrag gewählt")?;
                let vault = self.vault.as_ref().ok_or("Die Schlüsseldatenbank ist gesperrt")?;
                let title = vault.credential(entry).map(|c| c.data.title).ok_or("Eintrag nicht in der Schlüsseldatenbank")?;
                CredentialMode::Entry { name: name_for_entry, reference: CredentialRef { entry, last_title: Some(title) } }
            }
            "direct" => {
                let username = str_of(cred, "username").unwrap_or_default().to_string();
                // Kennwort byte-genau: kein Trimmen, keine Normalisierung.
                let password = str_of(cred, "password").map(Secret::from_str_exact);
                let title = match edit::node(&self.index, id) {
                    Some(n) => n.name().to_string(),
                    None => "Wurzel".into(),
                };
                let current = match credential_mode(&self.index, id) {
                    Some(CredentialMode::Entry { reference, .. }) => Some(reference.entry),
                    _ => None,
                };
                // Geteilter Eintrag (z. B. nach Duplizieren): nicht überschreiben, sondern einen eigenen anlegen.
                let current = current.filter(|e| !rj_core::shared::is_shared(&self.index, *e));
                let vault = self.vault.as_mut().ok_or("Die Schlüsseldatenbank ist gesperrt")?;
                let entry = match current.filter(|e| vault.is_own_entry(*e)) {
                    Some(e) => {
                        let old = vault.credential(e).ok_or("Eintrag nicht in der Schlüsseldatenbank")?;
                        let data = CredentialData { username, password: password.unwrap_or(old.data.password.clone()), ..old.data };
                        vault.update_credential(e, data).map_err(|e| e.to_string())?;
                        e
                    }
                    None => vault
                        .add_credential(CredentialData { title: title.clone(), username, password: password.unwrap_or_default(), ..Default::default() })
                        .map_err(|e| e.to_string())?,
                };
                CredentialMode::Entry { name: name_for_entry, reference: CredentialRef { entry, last_title: Some(title) } }
            }
            other => return Err(format!("Unbekannte Art der Zugangsdaten „{other}“")),
        };
        if !rj_core::inherit::set_credential_mode(&mut self.index, id, mode) {
            return Err("Objekt nicht gefunden".into());
        }
        Ok(())
    }

    /// Zweiten, benannten Zugangsdatensatz des Ziels anlegen oder ändern (Kennwort der Sitzung für die Tastenfolge).
    /// Leere Felder lassen den bisherigen Wert; es wird nichts angelegt, solange nichts eingegeben ist.
    fn apply_aux_credential(&mut self, id: Uuid, aux: &Value) -> Result<(), String> {
        let name = str_of(aux, "name").filter(|n| !n.is_empty()).unwrap_or(rj_core::autokeys::SESSION_CREDENTIAL).to_string();
        let username = str_of(aux, "username").map(str::to_string);
        // Kennwort byte-genau: kein Trimmen, keine Normalisierung.
        let password = str_of(aux, "password").filter(|p| !p.is_empty()).map(Secret::from_str_exact);
        let existing = rj_core::inherit::settings_mut(&mut self.index, id).and_then(|s| s.credentials.get(&name).cloned());
        if existing.is_none() && username.as_deref().unwrap_or_default().is_empty() && password.is_none() {
            return Ok(());
        }
        let title = match edit::node(&self.index, id) {
            Some(n) => format!("{} – {name}", n.name()),
            None => name.clone(),
        };
        let shared = existing.as_ref().is_some_and(|r| rj_core::shared::is_shared(&self.index, r.entry));
        let vault = self.vault.as_mut().ok_or("Die Schlüsseldatenbank ist gesperrt")?;
        let entry = match existing.as_ref().map(|r| r.entry).filter(|e| !shared && vault.is_own_entry(*e)) {
            Some(e) => {
                let old = vault.credential(e).ok_or("Eintrag nicht in der Schlüsseldatenbank")?;
                let data = CredentialData { username: username.unwrap_or(old.data.username.clone()), password: password.unwrap_or(old.data.password.clone()), ..old.data };
                vault.update_credential(e, data).map_err(|e| e.to_string())?;
                e
            }
            None => vault
                .add_credential(CredentialData { title: title.clone(), username: username.unwrap_or_default(), password: password.unwrap_or_default(), ..Default::default() })
                .map_err(|e| e.to_string())?,
        };
        let s = rj_core::inherit::settings_mut(&mut self.index, id).ok_or("Objekt nicht gefunden")?;
        s.credentials.insert(name, CredentialRef { entry, last_title: Some(title) });
        Ok(())
    }

    /// SNMP-Gerät anlegen oder ändern. Geheimnisse (Community, Kennwort zur
    /// Authentisierung, Kennwort zur Verschlüsselung) gehen in einen eigenen
    /// KDBX-Eintrag; leere Kennwortfelder lassen den bisherigen Wert.
    fn op_snmp_save(&mut self, c: &Value) -> Result<Value, String> {
        let name = str_of(c, "name").filter(|n| !n.is_empty()).ok_or("Name darf nicht leer sein")?.to_string();
        let host = str_of(c, "host").unwrap_or_default().to_string();
        if let Some(e) = validate_connection(Protocol::Ssh, &host, c.get("port"), None).into_values().next() {
            return Err(e);
        }
        let version: rj_core::SnmpVersion = serde_json::from_value(c.get("version").cloned().unwrap_or(json!("v2c"))).map_err(|_| "Version ungültig")?;
        let auth_proto: Option<rj_core::SnmpAuth> = c.get("auth_proto").and_then(|v| serde_json::from_value(v.clone()).ok());
        let priv_proto: Option<rj_core::SnmpPriv> = c.get("priv_proto").and_then(|v| serde_json::from_value(v.clone()).ok());
        let port = c.get("port").and_then(|p| p.as_u64().or_else(|| p.as_str().and_then(|s| s.parse().ok()))).map(|p| p as u16);
        let existing = id_of(c, "id").and_then(|id| self.index.snmp_devices.iter().find(|d| d.id == id).cloned());
        let vault = self.vault.as_mut().ok_or("Die Schlüsseldatenbank ist gesperrt")?;
        // Kennwörter byte-genau, nie getrimmt.
        let secret = |k: &str| str_of(c, k).filter(|s| !s.is_empty()).map(Secret::from_str_exact);
        let (main_pw, priv_pw) = match version {
            rj_core::SnmpVersion::V2c => (secret("community"), None),
            rj_core::SnmpVersion::V3 => (secret("auth_password"), secret("priv_password")),
        };
        let username = if version == rj_core::SnmpVersion::V3 { str_of(c, "username").unwrap_or_default().to_string() } else { String::new() };
        let title = format!("SNMP {name}");
        let entry = match existing.as_ref().map(|d| d.credential.entry).filter(|e| vault.is_own_entry(*e)) {
            Some(e) => {
                let old = vault.credential(e).ok_or("Eintrag nicht in der Schlüsseldatenbank")?;
                let data = CredentialData { title: title.clone(), username, password: main_pw.unwrap_or(old.data.password.clone()), ..old.data };
                vault.update_credential(e, data).map_err(|e| e.to_string())?;
                e
            }
            None => vault
                .add_credential(CredentialData { title: title.clone(), username, password: main_pw.unwrap_or_default(), ..Default::default() })
                .map_err(|e| e.to_string())?,
        };
        if let Some(p) = priv_pw {
            vault.set_protected_field(entry, rj_core::SNMP_PRIV_FIELD, p).map_err(|e| e.to_string())?;
        }
        let before = self.index.clone();
        let dev = rj_core::SnmpDevice {
            id: existing.as_ref().map(|d| d.id).unwrap_or_else(Uuid::new_v4),
            name,
            host,
            port,
            version,
            credential: CredentialRef { entry, last_title: Some(title) },
            interfaces: existing.as_ref().map(|d| d.interfaces.clone()).unwrap_or_default(),
            auth_proto,
            priv_proto,
        };
        let id = dev.id;
        match self.index.snmp_devices.iter_mut().find(|d| d.id == id) {
            Some(d) => *d = dev,
            None => self.index.snmp_devices.push(dev),
        }
        self.commit("SNMP-Gerät speichern", before);
        let saved: Value = serde_json::from_str(&self.save_with(None)).unwrap_or_default();
        self.start_snmp();
        Ok(json!({ "id": id, "save": saved }))
    }

    /// Speichern mit Konfliktbehandlung. Antwort: `{"saved":bool}`,
    /// `{"conflict":{"mine":r,"theirs":r}}` oder `{"error":…}`.
    /// Notizen eines Ziels ändern (mit „Rückgängig“) und den Tresor speichern.
    pub(crate) fn set_target_inventory(&mut self, id: Uuid, on: Option<bool>) -> Result<(), String> {
        self.simple("Inventar umschalten", |idx| match rj_core::inherit::find_node_mut(&mut idx.root, id) {
            Some(Node::Target(t)) => {
                t.settings.inventory = on;
                Ok(json!({ "ok": true }))
            }
            _ => Err(EditError::NotFound),
        })?;
        let r: Value = serde_json::from_str(&self.save_with(None)).map_err(|e| e.to_string())?;
        match r.get("error").and_then(Value::as_str) {
            Some(e) => Err(e.to_string()),
            None if r.get("conflict").is_some() => Err("Die Schlüsseldatenbank wurde inzwischen geändert – bitte über Datei → Speichern abgleichen".into()),
            None => Ok(()),
        }
    }

    /// Nach einer Inventur: beim ersten Erkennen als Aruba-AP den Weg „Web-Oberfläche“ (https, 4343) anlegen, sofern das
    /// Ziel noch keinen Web-Zugang hat. Gespeichert wird sofort, wenn sonst nichts ungespeichert ist. Liefert, ob ein Weg
    /// angelegt wurde.
    pub(crate) fn aruba_web_way(&mut self, id: Uuid) -> bool {
        if self.demo || self.vault.as_ref().map_or(true, |v| v.index().read_only) {
            return false;
        }
        let mut inv = self.overview.store.load(id);
        if inv.aruba_web_way || crate::overview::inventory::device_kind(&inv) != Some("aruba-ap") {
            return false;
        }
        inv.aruba_web_way = true;
        let _ = self.overview.store.save(id, &inv);
        let Some(t) = self.target(id) else { return false };
        if t.protocol == Protocol::Web || t.ways.iter().any(|w| w.protocol == Protocol::Web) {
            return false;
        }
        let was_dirty = self.has_unsaved_changes();
        let r = self.simple("Aruba-Web-Oberfläche anlegen", |idx| match rj_core::inherit::find_node_mut(&mut idx.root, id) {
            Some(Node::Target(t)) => {
                let mut w = rj_core::model::AccessWay::new(Protocol::Web);
                w.name = "Web-Oberfläche".into();
                w.port = Some(4343);
                // Adresse sichtbar als URL eintragen (leer wirkte der Weg unbrauchbar).
                let host = t.host.trim();
                if !host.is_empty() {
                    let h = if host.contains(':') && !host.starts_with('[') { format!("[{host}]") } else { host.to_string() };
                    w.host = format!("https://{h}:4343/");
                }
                t.ways.push(w);
                Ok(json!({ "ok": true }))
            }
            _ => Err(EditError::NotFound),
        });
        if r.is_err() {
            return false;
        }
        if !was_dirty {
            let _ = self.save_with(None);
        }
        true
    }

    /// Geteilte Zugangsdaten: eigene KDBX-Einträge, die direkt an mehreren Stellen eingetragen sind (Benutzer, nie
    /// Kennwörter).
    fn shared_list(&self) -> Value {
        let Some(vault) = self.vault.as_ref() else { return json!({ "groups": [], "locked": true }) };
        let groups: Vec<Value> = rj_core::shared::shared(&self.index)
            .into_iter()
            .filter(|(e, _)| vault.is_own_entry(*e))
            .map(|(e, uses)| {
                let c = vault.credential(e);
                let users: Vec<Value> = uses
                    .iter()
                    .map(|u| {
                        let node = u.node();
                        let path: Vec<String> = self.index.path_to(node).unwrap_or_default().iter().map(|n| rj_core::charset::mark_visible(n.name(), false).into_owned()).collect();
                        let name = path.last().cloned().unwrap_or_else(|| "(Wurzel)".into());
                        let what = match u {
                            rj_core::shared::Use::Settings { name, .. } if name == rj_core::DEFAULT_CREDENTIAL => "Zugangsdaten".to_string(),
                            rj_core::shared::Use::Settings { name, .. } => format!("Datensatz „{name}“"),
                            rj_core::shared::Use::Way { way, .. } => match self.target(node).and_then(|t| t.ways.iter().find(|w| w.id == *way)) {
                                Some(w) => format!("Weg „{}“", w.label()),
                                None => "Weg".into(),
                            },
                        };
                        json!({ "id": node, "name": name, "path": path[..path.len().saturating_sub(1)].join(" › "), "what": what })
                    })
                    .collect();
                json!({ "entry": e, "title": c.as_ref().map(|c| c.data.title.clone()), "username": c.map(|c| c.data.username), "users": users })
            })
            .collect();
        json!({ "groups": groups })
    }

    /// Trennen: Die erste Stelle behält den Eintrag, jede weitere bekommt eine eigene Kopie.
    fn shared_split(&mut self, c: &Value) -> Result<Value, String> {
        let entry = id_of(c, "entry").ok_or("Eintrag fehlt")?;
        let uses = rj_core::shared::direct_uses(&self.index).remove(&entry).unwrap_or_default();
        if uses.len() < 2 {
            return Err("Der Eintrag ist nicht (mehr) geteilt".into());
        }
        let before = self.index.clone();
        for u in &uses[1..] {
            match self.copy_entry(entry, u.node()) {
                Ok(Some(r)) => {
                    rj_core::shared::replace(&mut self.index, u, r);
                }
                Ok(None) => {}
                Err(e) => {
                    self.index = before;
                    return Err(e);
                }
            }
        }
        self.commit("Geteilte Zugangsdaten trennen", before);
        Ok(json!({ "split": uses.len() - 1 }))
    }

    /// In Anmeldeobjekt umwandeln: ein Anmeldeobjekt auf den Eintrag; Ziele und Ordner sowie Wege verwenden es.
    /// Zusätzliche Datensätze (z. B. „Sitzung“) bekommen eigene Kopien.
    fn shared_to_login(&mut self, c: &Value) -> Result<Value, String> {
        use rj_core::shared::Use;
        let entry = id_of(c, "entry").ok_or("Eintrag fehlt")?;
        let uses = rj_core::shared::direct_uses(&self.index).remove(&entry).unwrap_or_default();
        if uses.len() < 2 {
            return Err("Der Eintrag ist nicht (mehr) geteilt".into());
        }
        let title = self.vault.as_ref().and_then(|v| v.credential(entry)).map(|c| c.data.title).unwrap_or_else(|| "Gemeinsame Anmeldung".into());
        let first = uses[0].node();
        let folder = if edit::folder(&self.index, first).is_some() { first } else { edit::parent_of(&self.index, first).unwrap_or(self.index.root.id) };
        let before = self.index.clone();
        let login = rj_core::Login { id: Uuid::new_v4(), name: title.clone(), folder, entry: CredentialRef { entry, last_title: Some(title) } };
        let lid = login.id;
        self.index.logins.push(login);
        for u in &uses {
            let main_name = match edit::node(&self.index, u.node()) {
                Some(Node::Target(t)) => t.credential_name().to_string(),
                _ => rj_core::DEFAULT_CREDENTIAL.to_string(),
            };
            match u {
                Use::Settings { node, name } if *name == main_name => {
                    rj_core::inherit::set_credential_mode(&mut self.index, *node, rj_core::inherit::CredentialMode::Login(lid));
                }
                Use::Way { node, way } => {
                    if let Some(Node::Target(t)) = rj_core::inherit::find_node_mut(&mut self.index.root, *node) {
                        if let Some(w) = t.ways.iter_mut().find(|w| w.id == *way) {
                            w.own = None;
                            w.credential = rj_core::model::WayCredential::Login(lid);
                        }
                    }
                }
                other => match self.copy_entry(entry, other.node()) {
                    Ok(Some(r)) => {
                        rj_core::shared::replace(&mut self.index, other, r);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        self.index = before;
                        return Err(e);
                    }
                },
            }
        }
        rj_core::inherit::sync_logins(&mut self.index);
        self.commit("In Anmeldeobjekt umwandeln", before);
        Ok(json!({ "login": lid }))
    }

    /// „Fehler melden“: Text für ein GitHub-Issue zusammenstellen und bereinigen (Rechner-, Benutzer- und Zielnamen,
    /// Adressen, Mailadressen). Gesendet wird nichts.
    fn report_build(&self, c: &Value) -> Value {
        let mut words: Vec<(String, &str)> = Vec::new();
        for k in ["USER", "USERNAME", "LOGNAME"] {
            if let Ok(v) = std::env::var(k) {
                words.push((v, "<Benutzer>"));
            }
        }
        for k in ["HOSTNAME", "COMPUTERNAME"] {
            if let Ok(v) = std::env::var(k) {
                words.push((v, "<Rechner>"));
            }
        }
        if let Ok(h) = std::fs::read_to_string("/etc/hostname") {
            words.push((h.trim().to_string(), "<Rechner>"));
        }
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            words.push((home.to_string_lossy().into_owned(), "<Benutzerordner>"));
        }
        self.index.for_each_target(|t| {
            words.push((t.name.clone(), "<Ziel>"));
            words.push((t.host.clone(), "<Ziel-Adresse>"));
        });
        let clean = |s: &str| crate::report::sanitize(s, &words);
        let kind = str_of(c, "kind").unwrap_or("bug");
        let title = clean(str_of(c, "title").unwrap_or_default().trim());
        let text = clean(str_of(c, "text").unwrap_or_default().trim());
        let mut body = format!("### Beschreibung\n\n{text}\n\n### Umgebung\n\n- remotejoe {}\n- {}\n", str_of(c, "version").unwrap_or("?"), clean(str_of(c, "os").unwrap_or("?")));
        let diag: Vec<String> = c.get("diag").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(clean).collect()).unwrap_or_default();
        if !diag.is_empty() {
            body.push_str(&format!("\n### Diagnose (letzte Einträge, bereinigt)\n\n```\n{}\n```\n", diag.join("\n")));
        }
        let (url, cut) = crate::report::issue_url(&title, &body, if kind == "wish" { "enhancement" } else { "bug" });
        json!({ "url": url, "cut": cut, "title": title, "body": body })
    }

    fn op_duplicate(&mut self, c: &Value) -> Result<Value, String> {
        let id = id_of(c, "id").ok_or("Objekt fehlt")?;
        let before = self.index.clone();
        let r = edit::duplicate(&mut self.index, id, " (Kopie)").map_err(|e| e.to_string()).and_then(|n| self.unshare_copy(n).map(|_| n));
        match r {
            Ok(n) => {
                self.commit("Duplizieren", before);
                Ok(json!({ "id": n }))
            }
            Err(e) => {
                self.index = before;
                Err(e)
            }
        }
    }

    /// Nach dem Duplizieren: eigene (direkt eingetragene) KDBX-Einträge der Kopie bekommen eine eigene Kopie des
    /// Eintrags, damit Änderungen an der Kopie das Original nicht mehr treffen. Fremde Einträge und Anmeldeobjekte
    /// bleiben geteilt.
    fn unshare_copy(&mut self, copy: Uuid) -> Result<(), String> {
        let nodes = edit::subtree(&self.index, copy);
        let uses: Vec<(Uuid, rj_core::shared::Use)> = rj_core::shared::direct_uses(&self.index)
            .into_iter()
            .flat_map(|(e, u)| u.into_iter().map(move |x| (e, x)))
            .filter(|(_, u)| nodes.contains(&u.node()))
            .collect();
        let mut copies: HashMap<Uuid, CredentialRef> = HashMap::new();
        for (entry, u) in uses {
            let r = match copies.get(&entry) {
                Some(r) => r.clone(),
                None => match self.copy_entry(entry, u.node())? {
                    Some(r) => {
                        copies.insert(entry, r.clone());
                        r
                    }
                    None => continue,
                },
            };
            rj_core::shared::replace(&mut self.index, &u, r);
        }
        Ok(())
    }

    /// Kopie eines eigenen KDBX-Eintrags, betitelt nach dem Knoten `node` und mit Unix-Zeitstempel (Sekunden), damit
    /// zwei Einträge in der Anzeige nie gleich heißen; `None` bei fremden Einträgen.
    fn copy_entry(&mut self, entry: Uuid, node: Uuid) -> Result<Option<CredentialRef>, String> {
        let title = format!("{} · {}", edit::node(&self.index, node).map(|n| n.name().to_string()).unwrap_or_default(), unique_stamp());
        let vault = self.vault.as_mut().ok_or("Die Schlüsseldatenbank ist gesperrt")?;
        if !vault.is_own_entry(entry) {
            return Ok(None);
        }
        let old = vault.credential(entry).ok_or("Eintrag nicht in der Schlüsseldatenbank")?;
        let new = vault.add_credential(CredentialData { title: title.clone(), ..old.data }).map_err(|e| e.to_string())?;
        Ok(Some(CredentialRef { entry: new, last_title: Some(title) }))
    }

    /// Eigene Zugangsdaten der Wege: vorhandenen eigenen Eintrag ändern (leeres Kennwort = unverändert), sonst einen
    /// neuen Eintrag „Ziel – Weg“ anlegen.
    fn apply_way_credentials(&mut self, target_name: &str, list: &mut [rj_core::model::AccessWay], own: Vec<OwnInput>) -> Result<(), String> {
        if own.is_empty() {
            return Ok(());
        }
        let shared: Vec<Uuid> = rj_core::shared::shared(&self.index).into_iter().map(|(e, _)| e).collect();
        let vault = self.vault.as_mut().ok_or("Die Schlüsseldatenbank ist gesperrt")?;
        for (wid, username, password) in own {
            let Some(w) = list.iter_mut().find(|w| w.id == wid) else { continue };
            // Unverändert (gleicher Benutzer, kein neues Kennwort): Eintrag nicht anfassen.
            let current = w.own.as_ref().and_then(|r| vault.credential(r.entry)).map(|c| c.data.username);
            if password.is_none() && current.as_deref() == Some(username.as_str()) {
                continue;
            }
            let title = format!("{target_name} – {}", w.label());
            let entry = match w.own.as_ref().map(|r| r.entry).filter(|e| !shared.contains(e) && vault.is_own_entry(*e)) {
                Some(e) => {
                    let old = vault.credential(e).ok_or("Eintrag nicht in der Schlüsseldatenbank")?;
                    let data = CredentialData { title: title.clone(), username, password: password.unwrap_or(old.data.password.clone()), ..old.data };
                    vault.update_credential(e, data).map_err(|e| e.to_string())?;
                    e
                }
                None => {
                    // Fremder Eintrag (z. B. aus dem Zusammenfassen): ohne neues Kennwort das bisherige übernehmen.
                    let old_pw = w.own.as_ref().and_then(|r| vault.credential(r.entry)).map(|c| c.data.password);
                    vault
                        .add_credential(CredentialData { title: title.clone(), username, password: password.or(old_pw).unwrap_or_default(), ..Default::default() })
                        .map_err(|e| e.to_string())?
                }
            };
            w.own = Some(CredentialRef { entry, last_title: Some(title) });
        }
        Ok(())
    }

    pub(crate) fn set_target_notes(&mut self, id: Uuid, text: String) -> Result<(), String> {
        self.simple("Notizen ändern", |idx| match rj_core::inherit::find_node_mut(&mut idx.root, id) {
            Some(Node::Target(t)) => {
                t.meta.notes = text;
                Ok(json!({ "ok": true }))
            }
            _ => Err(EditError::NotFound),
        })?;
        let r: Value = serde_json::from_str(&self.save_with(None)).map_err(|e| e.to_string())?;
        match r.get("error").and_then(Value::as_str) {
            Some(e) => Err(e.to_string()),
            None if r.get("conflict").is_some() => Err("Die Schlüsseldatenbank wurde inzwischen geändert – bitte über Datei → Speichern abgleichen".into()),
            None => Ok(()),
        }
    }

    pub fn save_with(&mut self, choice: Option<&str>) -> String {
        let choice = match choice {
            Some("mine") => Some(ConflictChoice::KeepMine),
            Some("theirs") => Some(ConflictChoice::LoadTheirs),
            Some("both") => Some(ConflictChoice::KeepBoth),
            _ => None,
        };
        let Some(v) = self.vault.as_mut() else {
            return json!({ "saved": false, "notice": "Vorführdaten werden nicht gespeichert" }).to_string();
        };
        match v.save(choice) {
            Ok(saved) => {
                self.index = v.index().index.clone();
                json!({ "saved": saved }).to_string()
            }
            Err(rj_kdbx::VaultError::Conflict { mine, theirs }) => json!({ "conflict": { "mine": mine, "theirs": theirs } }).to_string(),
            Err(e) => err(format!("Speichern fehlgeschlagen: {e}")),
        }
    }

    /// „Schlüssel und Zertifikate prüfen“ für die markierten Ziele bzw. Ordner (`ids`) im Hintergrund
    /// starten; das Ergebnis kommt als `JobResult` mit `kind = "cert_check"`.
    fn op_cert_check_start(&mut self, c: &Value) -> Result<Value, String> {
        let mut seen = HashSet::new();
        let mut targets = Vec::new();
        for id in ids_of(c, "ids") {
            match edit::node(&self.index, id) {
                Some(Node::Target(_)) => targets.push(id),
                Some(Node::Folder(_)) => targets.extend(edit::targets_below(&self.index, id)),
                None => {}
            }
        }
        let mut specs = Vec::new();
        for id in targets.into_iter().filter(|i| seen.insert(*i)) {
            let Some(Node::Target(t)) = edit::node(&self.index, id) else { continue };
            let tunnel = rj_core::inherit::resolve_tunnel(&self.index, id).ok().flatten().is_some();
            if crate::certcheck::supported(t.protocol) {
                specs.push(crate::certcheck::ProbeSpec {
                    target: id,
                    name: t.name.clone(),
                    protocol: t.protocol,
                    host: t.host.clone(),
                    port: t.effective_port(),
                    legacy: matches!(&t.options, ProtocolOptions::Ssh(o) if o.legacy_algorithms),
                    options_json: self.options_json(t),
                    skip: tunnel.then(|| "Verbindung über einen Tunnel – wird nicht direkt geprüft".to_string()),
                    extra: false,
                });
            }
            // HTTPS-Dienste des Geräts: offen laut Dienstprüfung der Übersicht (443, 4343, 8443), bei einem erkannten
            // Aruba-AP immer 4343 (dorthin leitet die Web-Oberfläche weiter). Gleiches Browserprofil wie das Ziel, damit
            // der Web-Tab das Zertifikat danach kennt; antwortet dort niemand, entfällt die Zeile.
            let mut https: Vec<u16> = self.overview.open_ports(&id.to_string()).into_iter().filter(|p| matches!(p, 443 | 4343 | 8443)).collect();
            if self.overview.devices.get(&id) == Some(&"aruba-ap") && !https.contains(&4343) {
                https.push(4343);
            }
            let covered = |port: u16| {
                let main = t.protocol == Protocol::Web && Self::endpoint_of(t).1 == port;
                main || t.ways.iter().filter_map(|w| t.via(w.id)).any(|v| v.protocol == Protocol::Web && Self::endpoint_of(&v).1 == port && Self::endpoint_of(&v).0 == t.host)
            };
            for port in https.into_iter().filter(|p| !covered(*p)) {
                let host = t.host.trim();
                if host.is_empty() {
                    continue;
                }
                let h = if host.contains(':') && !host.starts_with('[') { format!("[{host}]") } else { host.to_string() };
                let url = if port == 443 { format!("https://{h}/") } else { format!("https://{h}:{port}/") };
                let mut v = t.clone();
                v.protocol = Protocol::Web;
                v.port = Some(port);
                v.options = ProtocolOptions::Web(rj_core::model::WebOptions { url: Some(url), ..Default::default() });
                specs.push(crate::certcheck::ProbeSpec {
                    target: id,
                    name: format!("{} – HTTPS :{port}", t.name),
                    protocol: Protocol::Web,
                    host: host.to_string(),
                    port,
                    legacy: false,
                    options_json: self.options_json(&v),
                    skip: tunnel.then(|| "Verbindung über einen Tunnel – wird nicht direkt geprüft".to_string()),
                    extra: true,
                });
            }
            // Zugangswege: jeder Weg wie ein eigenes Ziel (gleiches Browserprofil, Adresse und Port des Wegs).
            for w in &t.ways {
                let Some(v) = t.via(w.id) else { continue };
                if crate::certcheck::supported(v.protocol) {
                    specs.push(crate::certcheck::ProbeSpec {
                        target: id,
                        name: v.name.clone(),
                        protocol: v.protocol,
                        host: Self::endpoint_of(&v).0,
                        port: Self::endpoint_of(&v).1,
                        legacy: matches!(&v.options, ProtocolOptions::Ssh(o) if o.legacy_algorithms),
                        options_json: self.options_json(&v),
                        skip: tunnel.then(|| "Verbindung über einen Tunnel – wird nicht direkt geprüft".to_string()),
                        extra: false,
                    });
                }
            }
            // Windows-Ziele: zusätzlich der SSH-Schlüssel auf Port 22 (über ihn fragt die Übersicht das Inventar ab).
            if matches!(t.protocol, Protocol::Rdp | Protocol::Psrp) && !tunnel {
                specs.push(crate::certcheck::ProbeSpec {
                    target: id,
                    name: t.name.clone(),
                    protocol: Protocol::Ssh,
                    host: t.host.clone(),
                    port: 22,
                    legacy: false,
                    options_json: String::new(),
                    skip: None,
                    extra: true,
                });
            }
        }
        if specs.is_empty() {
            return Err("Keine prüfbaren Ziele in der Auswahl".into());
        }
        let n = specs.len();
        self.cert_rows.clear();
        let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        self.cert_done = done.clone();
        self.cert_total = n;
        let rt = self.rt_handle();
        let job = self.jobs.spawn_cancellable(format!("Schlüssel prüfen ({n} Ziele)"), move |job, cancel| {
            let rows = crate::certcheck::run_with_progress(&rt, specs, cancel, crate::certcheck::Stores::default_location(), done);
            Box::new(move |app: &mut App, cancelled: bool| {
                if cancelled {
                    return Vec::new();
                }
                let json = json!({ "rows": rows.iter().map(|r| r.json()).collect::<Vec<_>>() }).to_string();
                app.cert_rows = rows;
                vec![crate::sessions::UiEvent::JobResult { job, kind: "cert_check".into(), target: String::new(), json }]
            })
        });
        Ok(json!({ "job": job, "count": n }))
    }

    /// Alle Ziele eines Ordners (`id`) oder die markierten Ziele (`ids`, Ordner darin zählen nicht)
    /// verbinden, neu verbinden oder trennen. `count_only`: nur die Zahl der Ziele melden (für die Rückfrage).
    fn op_connect_all(&mut self, c: &Value) -> Result<Value, String> {
        let targets: Vec<Uuid> = if c.get("ids").is_some() {
            ids_of(c, "ids").into_iter().filter(|i| matches!(edit::node(&self.index, *i), Some(Node::Target(_)))).collect()
        } else {
            edit::targets_below(&self.index, id_of(c, "id").ok_or("ID fehlt")?)
        };
        if c.get("count_only").and_then(Value::as_bool).unwrap_or(false) {
            return Ok(json!({ "count": targets.len() }));
        }
        let mode = str_of(c, "mode").unwrap_or("connect");
        let open: Vec<SessionId> = self.sessions.ids().into_iter().filter(|s| self.sessions.target(*s).is_some_and(|t| targets.contains(&t))).collect();
        match mode {
            "disconnect" => {
                for s in &open {
                    self.sessions.disconnect(*s);
                }
                Ok(json!({ "count": open.len() }))
            }
            "reconnect" => {
                for s in &open {
                    self.sessions.reconnect(*s);
                }
                Ok(json!({ "count": open.len() }))
            }
            _ => {
                let limit = c.get("parallel").and_then(Value::as_u64).map(|n| n as usize).unwrap_or(self.config.connection.connect_all_parallel as usize);
                self.connect_size = (
                    c.get("width").and_then(Value::as_u64).unwrap_or(1024) as u32,
                    c.get("height").and_then(Value::as_u64).unwrap_or(768) as u32,
                );
                let n = targets.len();
                self.stagger = Some(Stagger::new(targets, limit));
                self.stagger_tick();
                Ok(json!({ "count": n }))
            }
        }
    }

    /// Ein Schritt des gestaffelten Verbindungsaufbaus (aus `poll`).
    pub(crate) fn stagger_tick(&mut self) {
        let Some(mut st) = self.stagger.take() else { return };
        let (w, h) = self.connect_size;
        let mut states: BTreeMap<SessionId, String> = self.sessions.ids().into_iter().map(|s| (s, self.sessions.state(s).unwrap_or_default().to_string())).collect();
        // Verbindungen über Tunnel laufen erst als Hintergrundaufgabe.
        for (job, r) in &self.job_results {
            let st = match r {
                Ok(s) => states.get(s).cloned().unwrap_or_else(|| "failed".into()),
                Err(_) => "failed".into(),
            };
            states.insert(*job, st);
        }
        for job in self.jobs.pending_ids() {
            states.insert(job, "connecting".into());
        }
        st.tick(
            |s| states.get(&s).cloned(),
            |t| {
                let ssh = matches!(edit::node(&self.index, t), Some(Node::Target(x)) if x.protocol == Protocol::Ssh);
                let (cw, ch) = if ssh { (120, 40) } else { (w, h) };
                let r: Value = serde_json::from_str(&self.connect(t, None, false, cw, ch)).unwrap_or_default();
                match r.get("session").or_else(|| r.get("pending")).and_then(Value::as_u64) {
                    Some(s) => Ok(s),
                    None => Err(r.get("error").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| r.to_string())),
                }
            },
        );
        if !st.is_done() {
            self.stagger = Some(st);
        } else {
            self.stagger_done = Some(st);
        }
    }
}

fn x_contains(f: &Folder, id: Uuid) -> bool {
    f.children.iter().any(|n| {
        n.id() == id
            || match n {
                Node::Folder(s) => x_contains(s, id),
                Node::Target(_) => false,
            }
    })
}

/// Einstellungen und Beschreibung eines Knotens (auch der Wurzel).
/// Ruft `f` für die Beschreibung der Wurzel, aller Ordner und Ziele auf.
fn walk_meta(f: &mut rj_core::Folder, g: &mut dyn FnMut(&mut rj_core::Meta)) {
    g(&mut f.meta);
    for n in &mut f.children {
        match n {
            Node::Folder(s) => walk_meta(s, g),
            Node::Target(t) => g(&mut t.meta),
        }
    }
}

fn count_meta(f: &rj_core::Folder, g: &mut dyn FnMut(&rj_core::Meta)) {
    g(&f.meta);
    for n in &f.children {
        match n {
            Node::Folder(s) => count_meta(s, g),
            Node::Target(t) => g(&t.meta),
        }
    }
}

fn edit_target(index: &mut Index, id: Uuid) -> Option<(&mut rj_core::Settings, &mut rj_core::Meta)> {
    if index.root.id == id {
        return Some((&mut index.root.settings, &mut index.root.meta));
    }
    match rj_core::inherit::find_node_mut(&mut index.root, id)? {
        Node::Folder(f) => Some((&mut f.settings, &mut f.meta)),
        Node::Target(t) => Some((&mut t.settings, &mut t.meta)),
    }
}

fn link(index: &mut Index, id: Uuid, f: impl FnOnce(&mut rj_core::Meta)) -> Result<(), EditError> {
    let (_, meta) = edit_target(index, id).ok_or(EditError::NotFound)?;
    f(meta);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rj_kdbx::{KdfChoice, MasterKey, Vault};

    fn app_with_vault(dir: &std::path::Path) -> (App, std::path::PathBuf, MasterKey) {
        let path = dir.join("t.kdbx");
        let key = MasterKey::password("Prüf ✓");
        let mut v = Vault::create(&path, key.clone(), KdfChoice::Weak, rj_core::config::IndexStorage::KdbxCustomData);
        v.save(None).unwrap();
        let mut app = App::new(false);
        app.index = v.index().index.clone();
        app.vault = Some(v);
        (app, path, key)
    }

    fn j(s: String) -> Value {
        serde_json::from_str(&s).unwrap()
    }

    fn cmd(app: &mut App, v: Value) -> Value {
        j(app.edit(&v.to_string()))
    }

    #[test]
    fn create_edit_save_reopen_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, path, key) = app_with_vault(dir.path());
        let root = app.index.root.id;
        let f = cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "folder", "name": "Standort Nord" }))["id"].as_str().unwrap().to_string();
        let t = cmd(&mut app, json!({ "op": "add", "parent": f, "kind": "rdp" }))["id"].as_str().unwrap().to_string();
        // Kennwort mit zerlegtem Umlaut, Leerzeichen am Ende und Emoji: byte-genau.
        let pw = "Kennwo\u{0308}rt 🚀 ";
        let r = cmd(&mut app, json!({
            "op": "update", "id": t,
            "fields": { "name": "srv-01", "host": "srv-01.example.test", "port": "3390", "description": "Testserver",
                        "options": { "clipboard": "target_to_host" }, "fields": { "Raum": "2.14" } },
            "credential": { "mode": "direct", "username": "EXAMPLE\\admin", "password": pw },
            "save": true
        }));
        assert_eq!(r["saved"], true, "{r}");
        // Neu öffnen und vergleichen.
        let v2 = Vault::open(&path, key, rj_core::config::IndexStorage::KdbxCustomData).unwrap();
        assert_eq!(v2.index().index.root, app.index.root);
        let tid = Uuid::parse_str(&t).unwrap();
        let Some(Node::Target(tt)) = edit::node(&v2.index().index, tid) else { panic!() };
        assert_eq!((tt.host.as_str(), tt.port), ("srv-01.example.test", Some(3390)));
        assert_eq!(tt.meta.fields["Raum"], "2.14");
        let entry = tt.settings.credentials["default"].entry;
        assert!(v2.is_own_entry(entry));
        assert_eq!(v2.credential(entry).unwrap().data.password.as_bytes(), pw.as_bytes());
        // Kennwort steht nie im Index.
        let json = String::from_utf8(rj_core::schema::to_json(&v2.index().index).unwrap()).unwrap();
        assert!(!json.contains("Kennwo"));
    }

    #[test]
    fn duplicate_gets_own_entry_and_shared_entries_are_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _path, _key) = app_with_vault(dir.path());
        let root = app.index.root.id;
        let a = cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "ssh", "name": "A" }))["id"].as_str().unwrap().to_string();
        let r = cmd(&mut app, json!({ "op": "update", "id": a, "fields": {}, "credential": { "mode": "direct", "username": "alt", "password": "eins" } }));
        assert!(r.get("error").is_none(), "{r}");
        let aid = Uuid::parse_str(&a).unwrap();
        let entry_of = |app: &App, id: Uuid| app.target(id).unwrap().settings.credentials[rj_core::DEFAULT_CREDENTIAL].entry;
        let ea = entry_of(&app, aid);
        // Duplizieren: die Kopie bekommt einen eigenen Eintrag mit denselben Daten.
        let b = Uuid::parse_str(cmd(&mut app, json!({ "op": "duplicate", "id": a }))["id"].as_str().unwrap()).unwrap();
        let eb = entry_of(&app, b);
        assert_ne!(ea, eb);
        let c = Uuid::parse_str(cmd(&mut app, json!({ "op": "duplicate", "id": a }))["id"].as_str().unwrap()).unwrap();
        let v = app.vault.as_ref().unwrap();
        let (tb, tc) = (v.credential(eb).unwrap().data.title, v.credential(entry_of(&app, c)).unwrap().data.title);
        assert!(tb.starts_with("A (Kopie) · ") && tc.starts_with("A (Kopie 2) · ") && tb != tc, "{tb} / {tc}");
        assert_eq!(app.vault.as_ref().unwrap().credential(eb).unwrap().data.password.as_bytes(), b"eins");
        // Altbestand: zwei Ziele teilen einen Eintrag (Duplikat aus einer früheren Version).
        if let Some(Node::Target(t)) = rj_core::inherit::find_node_mut(&mut app.index.root, b) {
            t.settings.credentials.get_mut(rj_core::DEFAULT_CREDENTIAL).unwrap().entry = ea;
        }
        assert!(rj_core::shared::is_shared(&app.index, ea));
        let r = cmd(&mut app, json!({ "op": "update", "id": b.to_string(), "fields": {}, "credential": { "mode": "direct", "username": "neu", "password": "zwei" } }));
        assert!(r.get("error").is_none(), "{r}");
        assert_ne!(entry_of(&app, b), ea, "geteilter Eintrag: eigener statt überschreiben");
        let c = app.vault.as_ref().unwrap().credential(ea).unwrap();
        assert_eq!((c.data.username.as_str(), c.data.password.as_bytes()), ("alt", b"eins".as_slice()), "Original unverändert");
    }

    #[test]
    fn way_options_are_saved_merged_and_validated() {
        let mut app = App::new(false);
        let root = app.index.root.id;
        let id = cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "hyperv", "name": "HV" }))["id"].as_str().unwrap().to_string();
        let tid = Uuid::parse_str(&id).unwrap();
        let wid = Uuid::new_v4().to_string();
        let save = |app: &mut App, opts: Value| cmd(app, json!({ "op": "update", "id": id, "fields": { "ways": [{ "id": wid, "protocol": "rdp", "options": opts }] } }));
        let r = save(&mut app, json!({ "gfx": "off", "width": 1600, "height": 900, "keyboard_layout": "" }));
        assert!(r.get("errors").is_none(), "{r}");
        let w = &app.target(tid).unwrap().ways[0];
        assert!(matches!(&w.options, ProtocolOptions::Rdp(o) if o.gfx == rj_core::model::GfxMode::Off && o.width == Some(1600)));
        // Eine Tastenfolge (nicht im Formular) bleibt beim nächsten Speichern erhalten.
        if let Some(Node::Target(t)) = rj_core::inherit::find_node_mut(&mut app.index.root, tid) {
            if let ProtocolOptions::Rdp(o) = &mut t.ways[0].options {
                o.auto_keys = Some(rj_core::autokeys::AutoKeys { enabled: true, script: "Taste Enter".into(), ..Default::default() });
            }
        }
        save(&mut app, json!({ "gfx": "on", "width": null }));
        let w = &app.target(tid).unwrap().ways[0];
        assert!(matches!(&w.options, ProtocolOptions::Rdp(o) if o.gfx == rj_core::model::GfxMode::On && o.width.is_none() && o.auto_keys.is_some()), "{:?}", w.options);
        let e = save(&mut app, json!({ "width": "breit" }));
        assert!(e["errors"]["ways"].as_str().is_some_and(|m| m.starts_with("Weg 1") && m.contains("Breite muss eine ganze Zahl")), "{e}");
    }

    #[test]
    fn aruba_web_way_gets_url_on_first_save() {
        let mut app = App::new(false);
        let root = app.index.root.id;
        let id = cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "ssh", "name": "AP" }))["id"].as_str().unwrap().to_string();
        let wid = Uuid::new_v4().to_string();
        // Wie „Hinzufügen → Aruba-AP“: Adresse und Web-Weg (4343, ohne Adresse) im selben Speichern.
        let r = cmd(&mut app, json!({ "op": "update", "id": id, "fields": { "host": "192.0.2.60", "ways": [{ "id": wid, "protocol": "web", "port": "4343", "host": "" }] } }));
        assert!(r.get("errors").is_none(), "{r}");
        let t = app.target(Uuid::parse_str(&id).unwrap()).unwrap();
        assert_eq!(t.ways[0].host, "https://192.0.2.60:4343/");
    }

    #[test]
    fn report_removes_target_names_and_addresses() {
        let mut app = App::new(false);
        let root = app.index.root.id;
        edit::insert(&mut app.index, root, Node::Target(rj_core::Target::new("ap-flur-2", Protocol::Ssh, "198.51.100.4")), None).unwrap();
        let r = cmd(&mut app, json!({ "op": "report_build", "kind": "bug", "title": "Inventar ap-flur-2", "text": "Fehler bei 198.51.100.4 und Version 8.10.0.23", "version": "0.1.18", "os": "Ubuntu", "diag": ["verbinde 198.51.100.4"] }));
        let all = r.to_string();
        assert!(!all.contains("ap-flur") && !all.contains("198.51.100.4"), "{all}");
        assert!(all.contains("Version 8.10.0.23") && all.contains("remotejoe 0.1.18"), "{all}");
        assert!(r["url"].as_str().unwrap().starts_with(crate::report::ISSUE_URL));
    }

    #[test]
    fn shared_check_lists_splits_and_converts_to_login() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _path, _key) = app_with_vault(dir.path());
        let root = app.index.root.id;
        let a = cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "ssh", "name": "A" }))["id"].as_str().unwrap().to_string();
        cmd(&mut app, json!({ "op": "update", "id": a, "fields": {}, "credential": { "mode": "direct", "username": "u", "password": "p" } }));
        let aid = Uuid::parse_str(&a).unwrap();
        let ea = app.target(aid).unwrap().settings.credentials[rj_core::DEFAULT_CREDENTIAL].entry;
        let share = |app: &mut App, name: &str| {
            let mut t = rj_core::Target::new(name, Protocol::Ssh, "192.0.2.9");
            t.settings.credentials.insert(rj_core::DEFAULT_CREDENTIAL.into(), CredentialRef { entry: ea, last_title: None });
            edit::insert(&mut app.index, root, Node::Target(t), None).unwrap()
        };
        let b = share(&mut app, "B");
        let l = cmd(&mut app, json!({ "op": "shared_list" }));
        assert_eq!(l["groups"].as_array().unwrap().len(), 1, "{l}");
        assert_eq!(l["groups"][0]["users"].as_array().unwrap().len(), 2);
        assert_eq!(l["groups"][0]["username"], "u");
        assert!(l.to_string().find("\"p\"").is_none(), "kein Kennwort");
        let r = cmd(&mut app, json!({ "op": "shared_split", "entry": ea }));
        assert_eq!(r["split"], 1, "{r}");
        assert!(cmd(&mut app, json!({ "op": "shared_list" }))["groups"].as_array().unwrap().is_empty());
        assert_eq!(app.target(aid).unwrap().settings.credentials[rj_core::DEFAULT_CREDENTIAL].entry, ea, "erste Stelle behält den Eintrag");
        assert_eq!(cmd(&mut app, json!({ "op": "undo" }))["undone"], "Geteilte Zugangsdaten trennen");
        let r = cmd(&mut app, json!({ "op": "shared_to_login", "entry": ea }));
        let lid = Uuid::parse_str(r["login"].as_str().unwrap()).unwrap();
        for id in [aid, b] {
            assert_eq!(rj_core::inherit::credential_mode(&app.index, id), Some(rj_core::inherit::CredentialMode::Login(lid)));
        }
        assert!(cmd(&mut app, json!({ "op": "shared_list" }))["groups"].as_array().unwrap().is_empty());
    }

    #[test]
    fn own_way_credentials_create_update_and_raise_schema() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _path, _key) = app_with_vault(dir.path());
        let root = app.index.root.id;
        let id = cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "rdp", "name": "HV" }))["id"].as_str().unwrap().to_string();
        let tid = Uuid::parse_str(&id).unwrap();
        let wid = Uuid::new_v4().to_string();
        let way = |user: &str, pw: Option<&str>| {
            let mut w = json!({ "id": wid, "protocol": "web", "name": "iLO", "host": "https://192.0.2.192/", "credential": "own", "username": user });
            if let Some(p) = pw {
                w["password"] = json!(p);
            }
            json!({ "op": "update", "id": id, "fields": { "ways": [w] } })
        };
        let r = cmd(&mut app, way("Administrator", Some("geheim ")));
        assert!(r.get("errors").is_none() && r.get("error").is_none(), "{r}");
        let own = app.target(tid).unwrap().ways[0].own.clone().expect("eigener Eintrag");
        let c = app.vault.as_ref().unwrap().credential(own.entry).unwrap();
        assert_eq!((c.data.username.as_str(), c.data.password.as_bytes()), ("Administrator", "geheim ".as_bytes()));
        assert_eq!(rj_core::schema::written_version(&app.index), rj_core::SCHEMA_VERSION, "eigene Zugangsdaten: Schema 5");
        // Unverändert gespeichert: derselbe Eintrag, Kennwort bleibt.
        cmd(&mut app, way("Administrator", None));
        assert_eq!(app.target(tid).unwrap().ways[0].own.as_ref().map(|r| r.entry), Some(own.entry));
        // Neues Kennwort: derselbe (eigene) Eintrag wird geändert.
        cmd(&mut app, way("Administrator", Some("neu")));
        assert_eq!(app.vault.as_ref().unwrap().credential(own.entry).unwrap().data.password.as_bytes(), b"neu");
        let e = j(app.editor_json(tid));
        assert_eq!((e["ways"][0]["credential"].as_str(), e["ways"][0]["username"].as_str()), (Some("own"), Some("Administrator")));
        assert!(e["ways"][0].get("password").is_none());
        // Zurück auf „wie das Ziel“: Schema 4.
        cmd(&mut app, json!({ "op": "update", "id": id, "fields": { "ways": [{ "id": wid, "protocol": "web", "credential": "target" }] } }));
        assert!(app.target(tid).unwrap().ways[0].own.is_none());
        assert_eq!(rj_core::schema::written_version(&app.index), rj_core::model::SCHEMA_VERSION_WAYS);
    }

    #[test]
    fn aruba_detection_adds_the_web_way_once() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _path, _key) = app_with_vault(dir.path());
        app.overview.store.dir = Some(dir.path().join("inventar"));
        let root = app.index.root.id;
        let id = Uuid::parse_str(cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "ssh", "name": "AP" }))["id"].as_str().unwrap()).unwrap();
        if let Some(Node::Target(t)) = rj_core::inherit::find_node_mut(&mut app.index.root, id) {
            t.host = "192.0.2.4".into();
        }
        assert!(!app.aruba_web_way(id), "ohne Inventar kein Weg");
        let mut inv = app.overview.store.load(id);
        inv.offer("system", "Aruba Instant 8.10.0.23 LSR (Modell 515)", "SSH", 1);
        app.overview.store.save(id, &inv).unwrap();
        assert!(app.aruba_web_way(id));
        let t = app.target(id).unwrap();
        assert_eq!((t.ways.len(), t.ways[0].protocol, t.ways[0].port), (1, Protocol::Web, Some(4343)));
        assert_eq!(t.ways[0].host, "https://192.0.2.4:4343/", "URL sichtbar eingetragen");
        // Weg gelöscht: Die nächste Inventur legt ihn nicht wieder an.
        if let Some(Node::Target(t)) = rj_core::inherit::find_node_mut(&mut app.index.root, id) {
            t.ways.clear();
        }
        assert!(!app.aruba_web_way(id));
        assert!(app.target(id).unwrap().ways.is_empty());
    }

    #[test]
    fn merge_same_address_preview_apply_and_undo() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _path, _key) = app_with_vault(dir.path());
        let root = app.index.root.id;
        let f = cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "folder", "name": "APs" }))["id"].as_str().unwrap().to_string();
        let text = "ap1.example.test\nap1.example.test:4343\nap2.example.test";
        let b = cmd(&mut app, json!({ "op": "bulk_add", "parent": f, "kind": "ssh", "text": text, "pattern": "{nr}-{host}" }));
        assert_eq!(b["created"].as_array().map(Vec::len), Some(3), "{b}");
        assert_eq!(j(app.save_with(None))["saved"], true);
        let before = app.index.clone();
        let p = cmd(&mut app, json!({ "op": "merge_preview", "ids": [f] }));
        let g = p["groups"].as_array().unwrap();
        assert_eq!(g.len(), 1, "{p}");
        assert_eq!(g[0]["address"], "ap1.example.test");
        let members: Vec<String> = g[0]["members"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap().to_string()).collect();
        assert_eq!(g[0]["keep"], members[0].as_str());
        let c = cmd(&mut app, json!({ "op": "merge_check", "keep": members[0], "others": [members[1]] }));
        assert_eq!((c["ways"].as_u64(), c["own"].as_u64()), (Some(1), Some(0)), "{c}");
        assert_eq!(app.index, before, "Vorschau und Prüfung ändern nichts");
        let r = cmd(&mut app, json!({ "op": "merge_apply", "groups": [{ "keep": members[0], "others": [members[1]], "name": "AP 1" }] }));
        assert_eq!(r["merged"], 1, "{r}");
        let keep = Uuid::parse_str(&members[0]).unwrap();
        let t = app.target(keep).unwrap();
        assert_eq!((t.name.as_str(), t.ways.len(), t.ways[0].port), ("AP 1", 1, Some(4343)));
        assert!(app.target(Uuid::parse_str(&members[1]).unwrap()).is_none());
        assert!(app.has_unsaved_changes());
        assert_eq!(cmd(&mut app, json!({ "op": "undo" }))["undone"], "Ziele zusammenfassen");
        assert_eq!(app.index.root, before.root);
        assert!(!app.has_unsaved_changes(), "Rückgängig bis zum gespeicherten Stand: nichts mehr ungespeichert");
    }

    #[test]
    fn bulk_add_roundtrip_and_single_undo() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, path, key) = app_with_vault(dir.path());
        let root = app.index.root.id;
        let f = cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "folder", "name": "Linux" }))["id"].as_str().unwrap().to_string();
        let text = "# Webserver\nweb1.example.test\nweb2.example.test:2222\n\nkaputt host\nweb3\u{200B}.example.test\n192.0.2.10";
        let p = cmd(&mut app, json!({ "op": "bulk_preview", "parent": f, "kind": "ssh", "text": text, "pattern": "{nr}-{host}" }));
        assert_eq!((p["valid"].as_u64(), p["invalid"].as_u64()), (Some(3), Some(2)), "{p}");
        assert!(p["rows"][3]["host"].as_str().unwrap().contains("⟦U+200B⟧"));
        // Vorschau ändert nichts.
        assert_eq!(edit::folder(&app.index, Uuid::parse_str(&f).unwrap()).unwrap().children.len(), 0);
        let pw = "geheim ";
        let r = cmd(&mut app, json!({ "op": "bulk_add", "parent": f, "kind": "ssh", "text": text, "pattern": "{nr}-{host}",
                                      "credential": { "mode": "direct", "username": "root", "password": pw } }));
        assert_eq!(r["created"].as_array().unwrap().len(), 3, "{r}");
        assert_eq!(r["remaining"], "kaputt host\nweb3\u{200B}.example.test");
        assert_eq!(app.history.last_label(), Some("Mehrere Ziele anlegen"));
        let s = j(app.save_with(None));
        assert_eq!(s["saved"], true, "{s}");
        let v2 = Vault::open(&path, key, rj_core::config::IndexStorage::KdbxCustomData).unwrap();
        assert_eq!(v2.index().index.root, app.index.root);
        let fid = Uuid::parse_str(&f).unwrap();
        let names: Vec<_> = edit::folder(&v2.index().index, fid).unwrap().children.iter().map(|n| n.name().to_string()).collect();
        assert_eq!(names, ["1-web1.example.test", "2-web2.example.test", "5-192.0.2.10"]);
        let entries: Vec<_> = edit::folder(&v2.index().index, fid)
            .unwrap()
            .children
            .iter()
            .map(|n| match n {
                Node::Target(t) => t.settings.credentials["default"].entry,
                _ => panic!(),
            })
            .collect();
        assert!(entries.iter().all(|e| *e == entries[0]));
        assert_eq!(v2.credential(entries[0]).unwrap().data.password.as_bytes(), pw.as_bytes());
        // Eine Aktion rückgängig: alle drei Ziele sind wieder weg.
        let u = cmd(&mut app, json!({ "op": "undo" }));
        assert_eq!(u["undone"], "Mehrere Ziele anlegen");
        assert!(edit::folder(&app.index, fid).unwrap().children.is_empty());
        // Ohne gültige Zeile wird nichts angelegt.
        let e = cmd(&mut app, json!({ "op": "bulk_add", "parent": f, "kind": "ssh", "text": "kaputt host" }));
        assert!(e["error"].is_string(), "{e}");
    }

    #[test]
    fn validation_reports_fields() {
        let mut app = App::new(false);
        let root = app.index.root.id;
        let t = cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "web" }))["id"].as_str().unwrap().to_string();
        let r = cmd(&mut app, json!({ "op": "update", "id": t, "fields": { "host": "a b", "port": "70000", "options": { "url": "https://u:p@example.test/" } } }));
        let e = &r["errors"];
        assert!(e["host"].is_string() && e["port"].is_string() && e["url"].is_string(), "{r}");
        let r = cmd(&mut app, json!({ "op": "update", "id": t, "fields": { "host": "web\u{200B}01.example.test" } }));
        assert!(r["errors"]["host"].as_str().unwrap().contains("unsichtbare"));
        let c = check_text("Kassen\u{202E}txt.exe", false);
        assert_eq!(c["issues"], 1);
        assert_eq!(c["marked"], "Kassen⟦U+202E⟧txt.exe");
    }

    #[test]
    fn auto_keys_are_validated_saved_and_shown_again() {
        let mut app = App::new(false);
        let root = app.index.root.id;
        let rdp = cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "rdp" }))["id"].as_str().unwrap().to_string();
        let ssh = cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "ssh" }))["id"].as_str().unwrap().to_string();
        // Ungültige Zeile: Fehler mit Zeilennummer, nichts gespeichert.
        let r = cmd(&mut app, json!({ "op": "update", "id": rdp, "fields": { "options": { "auto_keys": { "enabled": true, "script": "Taste Enter\nTaste Blubb" } } } }));
        assert!(r["errors"]["auto_keys"].as_str().unwrap().starts_with("Zeile 2:"), "{r}");
        // Windows-Taste ist im Terminal nicht darstellbar, bei RDP schon.
        let script = "Taste Win+R";
        let r = cmd(&mut app, json!({ "op": "update", "id": ssh, "fields": { "options": { "auto_keys": { "enabled": true, "script": script } } } }));
        assert!(r["errors"]["auto_keys"].as_str().unwrap().contains("Terminal"), "{r}");
        let r = cmd(&mut app, json!({ "op": "update", "id": rdp, "fields": { "options": { "auto_keys": { "enabled": true, "script": script, "trigger": "delay", "wait_ms": 1500, "on_reconnect": "first_only" } } } }));
        assert!(r["errors"].as_object().is_none_or(|e| e.is_empty()), "{r}");
        // Gespeichert und im Editor wieder sichtbar.
        let e = j(app.editor_json(Uuid::parse_str(&rdp).unwrap()));
        let a = &e["options"]["auto_keys"];
        assert_eq!((a["enabled"].as_bool(), a["script"].as_str(), a["trigger"].as_str(), a["wait_ms"].as_u64(), a["on_reconnect"].as_str()), (Some(true), Some(script), Some("delay"), Some(1500), Some("first_only")), "{e}");
    }

    #[test]
    fn second_session_credential_is_stored_in_the_vault_and_used_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _path, _key) = app_with_vault(dir.path());
        let root = app.index.root.id;
        let t = cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "rdp", "name": "desktop" }))["id"].as_str().unwrap().to_string();
        let tid = Uuid::parse_str(&t).unwrap();
        // Vorher: nichts angelegt.
        let e = j(app.editor_json(tid));
        assert_eq!((e["aux_credential"]["exists"].as_bool(), e["aux_credential"]["username"].as_str()), (Some(false), Some("")));
        // Leere Angaben legen nichts an.
        let r = cmd(&mut app, json!({ "op": "update", "id": t, "fields": {}, "aux_credential": { "name": "Sitzung", "username": "", "password": "" } }));
        assert!(r["error"].is_null(), "{r}");
        assert!(app.auto_key_credentials(&app.target(tid).cloned().unwrap()).is_empty());
        // Anlegen: Benutzer und Kennwort; das Kennwort steht nur in der Schlüsseldatenbank.
        let r = cmd(&mut app, json!({ "op": "update", "id": t, "fields": { "options": { "auto_keys": { "enabled": true, "script": "Text {cred:Sitzung:password}" } } }, "aux_credential": { "name": "Sitzung", "username": "anna2", "password": "zweites Kennwort" } }));
        assert!(r["error"].is_null() && r["errors"].as_object().is_none_or(|e| e.is_empty()), "{r}");
        let named = app.auto_key_credentials(&app.target(tid).cloned().unwrap());
        assert_eq!((named["Sitzung"].username.as_str(), named["Sitzung"].password.0.as_slice()), ("anna2", b"zweites Kennwort".as_slice()));
        let e = j(app.editor_json(tid));
        assert_eq!((e["aux_credential"]["exists"].as_bool(), e["aux_credential"]["username"].as_str()), (Some(true), Some("anna2")));
        assert!(!e.to_string().contains("zweites Kennwort"), "Kennwort darf nie im Editor-JSON stehen");
        let index = serde_json::to_string(&app.index).unwrap();
        assert!(!index.contains("zweites Kennwort"), "Kennwort darf nie im Index stehen");
        // Ändern: nur den Benutzer; das Kennwort bleibt.
        cmd(&mut app, json!({ "op": "update", "id": t, "fields": {}, "aux_credential": { "name": "Sitzung", "username": "anna3" } }));
        let named = app.auto_key_credentials(&app.target(tid).cloned().unwrap());
        assert_eq!((named["Sitzung"].username.as_str(), named["Sitzung"].password.0.as_slice()), ("anna3", b"zweites Kennwort".as_slice()));
    }

    #[test]
    fn login_object_create_assign_change_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _path, _key) = app_with_vault(dir.path());
        let root = app.index.root.id;
        let f = cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "folder", "name": "Netz" }))["id"].as_str().unwrap().to_string();
        let t = cmd(&mut app, json!({ "op": "add", "parent": f, "kind": "ssh", "name": "sw01" }))["id"].as_str().unwrap().to_string();
        let tid = Uuid::parse_str(&t).unwrap();
        // Anlegen im Ordner.
        let l = cmd(&mut app, json!({ "op": "login_save", "name": "Switch-Admin", "folder": f, "username": "admin", "password": "geheim-1" }));
        let lid = l["id"].as_str().unwrap_or_else(|| panic!("{l}")).to_string();
        let list = cmd(&mut app, json!({ "op": "logins" }));
        assert_eq!(list["logins"][0]["name"], "Switch-Admin");
        assert_eq!(list["logins"][0]["username"], "admin");
        assert!(!list.to_string().contains("geheim-1"), "Kennwort nie in der Liste");
        // Im Baum steht die Anmeldung im Ordner.
        let tree = crate::tree::tree(&app.index, "", &|_| None);
        assert!(tree[0].children.iter().any(|n| n.login && n.id.to_string() == lid));
        // Am Ordner wählen: das Ziel erbt sie.
        let r = cmd(&mut app, json!({ "op": "update", "id": f, "credential": { "mode": "login", "login": lid } }));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(j(app.editor_json(Uuid::parse_str(&f).unwrap()))["credential"]["mode"], "login");
        let creds = |app: &App| {
            let CredentialResolution::Found(r) = rj_core::inherit::resolve_credential(&app.index, tid).unwrap() else { panic!() };
            app.vault.as_ref().unwrap().credential(r.reference.entry).unwrap().data
        };
        assert_eq!((creds(&app).username.as_str(), creds(&app).password.as_str()), ("admin", Some("geheim-1")));
        // Kennwort an der Anmeldung ändern: gilt beim Ziel sofort.
        cmd(&mut app, json!({ "op": "login_save", "id": lid, "name": "Switch-Admin", "folder": f, "username": "admin2", "password": "geheim-2" }));
        assert_eq!((creds(&app).username.as_str(), creds(&app).password.as_str()), ("admin2", Some("geheim-2")));
        assert_eq!(cmd(&mut app, json!({ "op": "logins" }))["logins"][0]["used"], 1);
        // Löschen: das Ziel behält die Zugangsdaten.
        assert_eq!(cmd(&mut app, json!({ "op": "login_delete", "id": lid }))["ok"], true);
        assert!(app.index.logins.is_empty());
        assert_eq!(creds(&app).username, "admin2");
        // Ordner mit Anmeldung löschen nimmt sie mit.
        let l2 = cmd(&mut app, json!({ "op": "login_save", "name": "x", "folder": f, "username": "u" }))["id"].as_str().unwrap().to_string();
        assert!(app.index.login(Uuid::parse_str(&l2).unwrap()).is_some());
        cmd(&mut app, json!({ "op": "delete", "ids": [f] }));
        assert!(app.index.logins.is_empty());
    }

    #[test]
    fn inheritance_display_matches_core_and_undo() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _path, _key) = app_with_vault(dir.path());
        let root = app.index.root.id;
        let f = cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "folder", "name": "Linux" }))["id"].as_str().unwrap().to_string();
        let t = cmd(&mut app, json!({ "op": "add", "parent": f, "kind": "ssh", "name": "web01" }))["id"].as_str().unwrap().to_string();
        let r = cmd(&mut app, json!({ "op": "update", "id": f, "credential": { "mode": "direct", "username": "root", "password": "x" } }));
        assert_eq!(r["ok"], true, "{r}");
        let e = j(app.editor_json(Uuid::parse_str(&t).unwrap()));
        assert_eq!(e["credential"]["mode"], "inherit");
        assert_eq!(e["credential"]["effective"]["origin"], "geerbt von Ordner „Linux“");
        let CredentialResolution::Found(r) = rj_core::inherit::resolve_credential(&app.index, Uuid::parse_str(&t).unwrap()).unwrap() else { panic!() };
        assert_eq!(r.origin, Origin::Folder(Uuid::parse_str(&f).unwrap()));
        let fe = j(app.editor_json(Uuid::parse_str(&f).unwrap()));
        assert_eq!(fe["credential"]["mode"], "direct");
        // „Keine“ am Ziel, dann zurück auf Vererbung.
        cmd(&mut app, json!({ "op": "update", "id": t, "credential": { "mode": "none" } }));
        assert_eq!(j(app.editor_json(Uuid::parse_str(&t).unwrap()))["credential"]["effective"]["state"], "none");
        let u = cmd(&mut app, json!({ "op": "undo" }));
        assert_eq!(u["undone"], "Eigenschaften ändern");
        assert_eq!(j(app.editor_json(Uuid::parse_str(&t).unwrap()))["credential"]["effective"]["state"], "found");
        // Auf Unterobjekte anwenden mit Vorschau und Rückgängig.
        cmd(&mut app, json!({ "op": "update", "id": t, "fields": { "tunnel": "none" } }));
        assert_eq!(cmd(&mut app, json!({ "op": "apply_preview", "id": f, "kind": "tunnel" }))["count"], 1);
        assert_eq!(cmd(&mut app, json!({ "op": "apply", "id": f, "kind": "tunnel" }))["count"], 1);
        assert_eq!(cmd(&mut app, json!({ "op": "apply_preview", "id": f, "kind": "tunnel" }))["count"], 0);
        assert_eq!(cmd(&mut app, json!({ "op": "undo" }))["undone"], "Auf Unterobjekte anwenden");
        assert_eq!(cmd(&mut app, json!({ "op": "apply_preview", "id": f, "kind": "tunnel" }))["count"], 1);
        // Löschen lässt den KDBX-Eintrag bestehen: er erscheint unter „Nicht zugeordnet“.
        cmd(&mut app, json!({ "op": "delete", "id": f }));
        assert_eq!(app.vault.as_ref().unwrap().unassigned().len(), 1);
    }

    #[test]
    fn undo_withdraws_entries_created_for_the_undone_step() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _path, _key) = app_with_vault(dir.path());
        let root = app.index.root.id;
        let t = cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "ssh", "name": "web01" }))["id"].as_str().unwrap().to_string();
        let count = |app: &App| app.vault.as_ref().unwrap().entries().len();
        // Ungespeichert angelegt und rückgängig gemacht: kein Eintrag bleibt zurück.
        cmd(&mut app, json!({ "op": "update", "id": t, "credential": { "mode": "direct", "username": "root", "password": "x" } }));
        assert_eq!(count(&app), 1);
        let u = cmd(&mut app, json!({ "op": "undo" }));
        assert_eq!(u["withdrawn"], 1, "{u}");
        assert_eq!(count(&app), 0);
        assert!(app.vault.as_ref().unwrap().unassigned().is_empty());
        // Gespeichert und dann rückgängig: wandert in den Papierkorb, nicht unter „Nicht zugeordnet“.
        cmd(&mut app, json!({ "op": "update", "id": t, "credential": { "mode": "direct", "username": "root", "password": "y" } }));
        app.save().unwrap();
        assert_eq!(cmd(&mut app, json!({ "op": "undo" }))["withdrawn"], 1);
        assert!(app.vault.as_ref().unwrap().unassigned().is_empty());
        // Eintrag eines Anmeldeobjekts gilt als zugeordnet.
        cmd(&mut app, json!({ "op": "login_save", "name": "Switch", "folder": root, "username": "admin", "password": "p" }));
        assert!(app.vault.as_ref().unwrap().unassigned().is_empty(), "Anmeldeobjekt nicht unter „Nicht zugeordnet“");
    }

    #[test]
    fn delete_closes_sessions_of_deleted_targets() {
        let mut app = App::new(false);
        let root = app.index.root.id;
        let folder = cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "folder", "name": "Import" }))["id"].as_str().unwrap().to_string();
        let sub = cmd(&mut app, json!({ "op": "add", "parent": folder, "kind": "folder", "name": "Unter" }))["id"].as_str().unwrap().to_string();
        let t1 = Uuid::parse_str(cmd(&mut app, json!({ "op": "add", "parent": folder, "kind": "ssh", "name": "a" }))["id"].as_str().unwrap()).unwrap();
        let t2 = Uuid::parse_str(cmd(&mut app, json!({ "op": "add", "parent": sub, "kind": "ssh", "name": "b" }))["id"].as_str().unwrap()).unwrap();
        let other = Uuid::parse_str(cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "ssh", "name": "c" }))["id"].as_str().unwrap()).unwrap();
        let s1 = app.sessions.open_demo_for_target(t1, "connected");
        let s2 = app.sessions.open_demo_for_target(t2, "failed");
        let s3 = app.sessions.open_demo_for_target(other, "connected");
        // Vorschau zählt auch Unterordner; fehlgeschlagene Sitzungen sind nicht aktiv.
        let p = cmd(&mut app, json!({ "op": "delete_preview", "ids": [folder] }));
        assert_eq!((p["sessions"].as_u64(), p["active"].as_u64()), (Some(2), Some(1)), "{p}");
        let r = cmd(&mut app, json!({ "op": "delete", "ids": [folder] }));
        let mut closed: Vec<u64> = r["closed_sessions"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
        closed.sort_unstable();
        assert_eq!(closed, [s1, s2], "{r}");
        assert_eq!(app.sessions.ids(), [s3], "Sitzung eines anderen Ziels bleibt");
        // Ein einzelnes Ziel zu löschen beendet nur dessen Sitzung.
        let r = cmd(&mut app, json!({ "op": "delete", "id": other }));
        assert_eq!(r["closed_sessions"].as_array().unwrap().len(), 1);
        assert!(app.sessions.ids().is_empty());
        // Rückgängig bringt die Ziele zurück, nicht die Sitzungen.
        cmd(&mut app, json!({ "op": "undo" }));
        assert!(app.sessions.ids().is_empty());
    }

    #[test]
    fn templates_and_favorites_views() {
        let mut app = App::new(false);
        let root = app.index.root.id;
        let t = cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "ssh", "name": "a" }))["id"].as_str().unwrap().to_string();
        cmd(&mut app, json!({ "op": "update", "id": t, "fields": { "host": "a.example.test", "port": 2222, "tunnel": "none" }, "credential": { "mode": "named", "name": "admin" } }));
        let tpl = cmd(&mut app, json!({ "op": "template_save", "id": t, "name": "Linux-Standard", "default": true }))["id"].as_str().unwrap().to_string();
        let json = serde_json::to_string(&app.index.templates).unwrap();
        assert!(json.contains("admin") && !json.contains("entry"), "{json}");
        // Neues SSH-Ziel nimmt die Standardvorlage.
        let n = cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "ssh" }))["id"].as_str().unwrap().to_string();
        let Some(Node::Target(nt)) = edit::node(&app.index, Uuid::parse_str(&n).unwrap()) else { panic!() };
        assert_eq!((nt.port, nt.credential.as_deref()), (Some(2222), Some("admin")));
        let n2 = cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "x", "template": tpl }))["id"].as_str().unwrap().to_string();
        assert!(Uuid::parse_str(&n2).is_ok());
        cmd(&mut app, json!({ "op": "favorite", "id": t, "on": true }));
        let v = cmd(&mut app, json!({ "op": "view_create", "name": "Patchday", "ids": [t, n] }))["id"].as_str().unwrap().to_string();
        cmd(&mut app, json!({ "op": "view_create", "name": "Kunde", "ids": [t] }));
        assert_eq!(app.index.views.iter().filter(|x| x.targets.contains(&Uuid::parse_str(&t).unwrap())).count(), 2);
        cmd(&mut app, json!({ "op": "view_rename", "view": v, "name": "Patchday Juli" }));
        let d = cmd(&mut app, json!({ "op": "duplicate", "id": t }))["id"].as_str().unwrap().to_string();
        assert_ne!(d, t);
        assert_eq!(app.index.favorites.len(), 1, "Duplikat ist kein Favorit");
        cmd(&mut app, json!({ "op": "rename", "id": d, "name": "a-zwei" }));
        let f = cmd(&mut app, json!({ "op": "add", "parent": root, "kind": "folder" }))["id"].as_str().unwrap().to_string();
        cmd(&mut app, json!({ "op": "move", "id": d, "parent": f }));
        assert_eq!(edit::parent_of(&app.index, Uuid::parse_str(&d).unwrap()), Uuid::parse_str(&f).ok());
        let task = cmd(&mut app, json!({ "op": "add", "parent": f, "kind": "task" }))["task"].as_str().unwrap().to_string();
        cmd(&mut app, json!({ "op": "update_task", "id": task, "name": "apt-Aktualisierung", "params": { "dry_run": "true" } }));
        let m = cmd(&mut app, json!({ "op": "add", "parent": f, "kind": "macro" }))["macro"].as_str().unwrap().to_string();
        let r = cmd(&mut app, json!({ "op": "update_macro", "id": m, "steps": "[{\"send\":\"uptime\\n\"}]" }));
        assert_eq!(r["ok"], true, "{r}");
        let e = j(app.editor_json(Uuid::parse_str(&f).unwrap()));
        assert_eq!(e["tasks"][0]["linked"], true);
        assert_eq!(e["macros"][0]["linked"], true);
    }

    #[test]
    fn stagger_limits_parallelism_and_survives_failures() {
        use std::cell::RefCell;
        let targets: Vec<Uuid> = (0..10).map(|_| Uuid::new_v4()).collect();
        let bad = targets[3];
        let mut st = Stagger::new(targets.clone(), 4);
        // Simulierte Sitzungen: jede braucht drei Schritte bis „connected“.
        let states: RefCell<BTreeMap<SessionId, (u32, Uuid)>> = RefCell::new(BTreeMap::new());
        let mut next = 1;
        let mut max_active = 0;
        for _ in 0..50 {
            for (age, _) in states.borrow_mut().values_mut() {
                *age += 1;
            }
            st.tick(
                |s| states.borrow().get(&s).map(|(age, t)| if *age >= 3 { if *t == bad { "failed" } else { "connected" } } else { "connecting" }.to_string()),
                |t| {
                    let s = next;
                    next += 1;
                    states.borrow_mut().insert(s, (0, t));
                    Ok(s)
                },
            );
            let active = states.borrow().values().filter(|(a, _)| *a < 3).count();
            max_active = max_active.max(active);
            if st.is_done() {
                break;
            }
        }
        assert!(st.is_done());
        assert_eq!(st.results.len(), 10);
        assert!(max_active <= 4 && st.peak == 4, "höchstens 4 gleichzeitig ({max_active}/{})", st.peak);
        let ok = states.borrow().values().filter(|(a, t)| *a >= 3 && *t != bad).count();
        assert_eq!(ok, 9);
        // Startfehler (z. B. Zugangsdaten fehlen) halten die übrigen nicht auf.
        let mut st = Stagger::new(targets.clone(), 2);
        let mut n = 0;
        st.tick(|_| Some("connected".into()), |t| if t == targets[0] { Err("fehlt".into()) } else { n += 1; Ok(n) });
        assert_eq!(st.results.len(), 3);
    }
}

#[cfg(test)]
mod macro_tests {
    use super::*;

    fn cmd(app: &mut App, v: Value) -> Value {
        serde_json::from_str(&app.edit(&v.to_string())).unwrap()
    }

    #[test]
    fn create_link_list_duplicate_unlink_delete() {
        let mut app = App::new(true);
        app.config_path = None;
        let folder = cmd(&mut app, json!({ "op": "add", "kind": "folder" }))["id"].as_str().unwrap().to_string();
        let target = cmd(&mut app, json!({ "op": "add", "kind": "ssh", "parent": folder }))["id"].as_str().unwrap().to_string();

        let m = cmd(&mut app, json!({ "op": "macro_new" }))["macro"].as_str().unwrap().to_string();
        assert_eq!(cmd(&mut app, json!({ "op": "macro_list" }))[0]["links"], 0);
        cmd(&mut app, json!({ "op": "update_macro", "id": m, "name": "Updates", "steps": [{ "step": "send", "text": "uptime{key:enter}" }] }));

        // Zuordnen an den Ordner und an das Ziel; doppelt zuordnen ändert nichts.
        assert_eq!(cmd(&mut app, json!({ "op": "macro_link", "id": m, "node": folder }))["ok"], true);
        cmd(&mut app, json!({ "op": "macro_link", "id": m, "node": folder }));
        cmd(&mut app, json!({ "op": "macro_link", "id": m, "node": target }));
        let l = cmd(&mut app, json!({ "op": "macro_list" }));
        assert_eq!(l[0]["links"], 2);
        assert_eq!(l[0]["steps"], 1);
        let g = cmd(&mut app, json!({ "op": "macro_get", "id": m }));
        assert_eq!(g["name"], "Updates");
        assert_eq!(g["linked"].as_array().unwrap().len(), 2);

        // Duplizieren: neue ID, ohne Zuordnung.
        let d = cmd(&mut app, json!({ "op": "macro_duplicate", "id": m }))["macro"].as_str().unwrap().to_string();
        assert_ne!(d, m);
        let l = cmd(&mut app, json!({ "op": "macro_list" }));
        assert_eq!(l.as_array().unwrap().len(), 2);
        assert_eq!(l[1]["name"], "Updates (Kopie)");
        assert_eq!(l[1]["links"], 0);

        // Zuordnung aufheben, dann löschen (entfernt auch alle Zuordnungen).
        cmd(&mut app, json!({ "op": "macro_unlink", "id": m, "node": target }));
        assert_eq!(cmd(&mut app, json!({ "op": "macro_list" }))[0]["links"], 1);
        assert_eq!(cmd(&mut app, json!({ "op": "macro_delete", "id": m }))["ok"], true);
        let l = cmd(&mut app, json!({ "op": "macro_list" }));
        assert_eq!(l.as_array().unwrap().len(), 1);
        assert!(cmd(&mut app, json!({ "op": "macro_get", "id": m })).get("error").is_some());
        assert!(cmd(&mut app, json!({ "op": "macro_delete", "id": m })).get("error").is_some());
        // Rückgängig stellt das gelöschte Makro samt Zuordnung wieder her.
        cmd(&mut app, json!({ "op": "undo" }));
        assert_eq!(cmd(&mut app, json!({ "op": "macro_list" })).as_array().unwrap().len(), 2);
    }

    #[test]
    fn run_macro_refuses_without_ssh_session() {
        let mut app = App::new(true);
        app.config_path = None;
        let m = cmd(&mut app, json!({ "op": "macro_new" }))["macro"].as_str().unwrap().to_string();
        let r = cmd(&mut app, json!({ "op": "run_macro", "session": 99, "macro": m }));
        assert!(r["error"].as_str().unwrap().contains("SSH"), "{r}");
        assert_eq!(cmd(&mut app, json!({ "op": "session_macros", "session": 99 })), json!([]));
    }
}

#[cfg(test)]
mod web_tests {
    use super::*;

    fn cmd(app: &mut App, v: Value) -> Value {
        serde_json::from_str(&app.edit(&v.to_string())).unwrap()
    }

    fn host_port(app: &App, id: &str) -> (String, Option<u16>) {
        match edit::node(&app.index, Uuid::parse_str(id).unwrap()) {
            Some(Node::Target(t)) => (t.host.clone(), t.port),
            _ => panic!("kein Ziel"),
        }
    }

    #[test]
    fn web_target_needs_only_the_url() {
        let mut app = App::new(true);
        app.config_path = None;
        let t = cmd(&mut app, json!({ "op": "add", "kind": "web" }))["id"].as_str().unwrap().to_string();

        // Nur die Adresse: Host und Port werden abgeleitet, Standardport 443 bleibt leer.
        let r = cmd(&mut app, json!({ "op": "update", "id": t, "fields": { "options": { "url": "https://192.0.2.192/" } } }));
        assert!(r.get("errors").is_none(), "{r}");
        assert_eq!(host_port(&app, &t), ("192.0.2.192".into(), None));

        // Anderer Port und http.
        cmd(&mut app, json!({ "op": "update", "id": t, "fields": { "options": { "url": "https://ilo.example.test:8443/index.html" } } }));
        assert_eq!(host_port(&app, &t), ("ilo.example.test".into(), Some(8443)));
        cmd(&mut app, json!({ "op": "update", "id": t, "fields": { "options": { "url": "http://ilo.example.test/" } } }));
        assert_eq!(host_port(&app, &t), ("ilo.example.test".into(), Some(80)));

        // Ungültige Adresse und Zugangsdaten in der Adresse werden abgelehnt.
        let r = cmd(&mut app, json!({ "op": "update", "id": t, "fields": { "options": { "url": "ilo.example.test" } } }));
        assert!(r["errors"]["url"].is_string(), "{r}");
        let r = cmd(&mut app, json!({ "op": "update", "id": t, "fields": { "options": { "url": "https://u:p@x.example.test/" } } }));
        assert!(r["errors"]["url"].is_string(), "{r}");
        assert_eq!(host_port(&app, &t).0, "ilo.example.test");

        // Den Editor-Daten liegt für Ziele mit Host ein Vorschlag der Adresse bei.
        let e = cmd(&mut app, json!({ "op": "editor", "id": t }));
        assert_eq!(e["default_url"], "http://ilo.example.test/");
    }

    #[test]
    fn web_target_without_url_and_host_is_rejected() {
        let mut app = App::new(true);
        app.config_path = None;
        let t = cmd(&mut app, json!({ "op": "add", "kind": "web" }))["id"].as_str().unwrap().to_string();
        let r = cmd(&mut app, json!({ "op": "update", "id": t, "fields": { "host": "", "options": { "url": "" } } }));
        assert!(r["errors"]["url"].as_str().unwrap().contains("leer"), "{r}");
    }
}

/// Herkunft der Inventar-Einstellung für die Anzeige.
pub(crate) fn inventory_source_text(src: rj_core::inherit::InventorySource, on: bool) -> &'static str {
    match src {
        rj_core::inherit::InventorySource::Target => "am Ziel festgelegt",
        rj_core::inherit::InventorySource::Folder => "vom Ordner geerbt",
        rj_core::inherit::InventorySource::Default if on => "Vorgabe",
        rj_core::inherit::InventorySource::Default => "Vorgabe für Web-Ziele",
    }
}

/// Zugangsweg für die Oberfläche.
/// Unix-Zeitstempel in Sekunden, innerhalb des Programmlaufs streng steigend (mehrere Kopien in derselben Sekunde
/// bekommen so trotzdem verschiedene Titel).
fn unique_stamp() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut prev = LAST.load(Ordering::Relaxed);
    loop {
        let next = now.max(prev + 1);
        match LAST.compare_exchange(prev, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(p) => prev = p,
        }
    }
}

/// Zusammenfassen: angenommene Web-Zertifikate der aufgelösten Ziele (eigenes Browserprofil je Ziel) ins Profil des
/// Hauptziels übernehmen, damit dessen Wege nicht erneut fragen. Einträge ohne Port gehörten zum Port des alten
/// Ziels und bekommen ihn mit; vorhandene Einträge des Hauptziels bleiben.
fn carry_web_certs(carry: &[(Uuid, Uuid, u16)]) {
    let Some(base) = rj_core::config::data_dir() else { return };
    for (keep, other, port) in carry {
        let from = rj_web::AcceptedCerts::load(&rj_web::profile_dir(&base, &other.to_string()));
        if from.hosts.is_empty() {
            continue;
        }
        let dir = rj_web::profile_dir(&base, &keep.to_string());
        let mut to = rj_web::AcceptedCerts::load(&dir);
        for (k, fp) in from.hosts {
            let has_port = k.rsplit_once(':').is_some_and(|(h, p)| p.parse::<u16>().is_ok() && (h.starts_with('[') || !h.contains(':')));
            let key = if has_port { k } else { rj_web::AcceptedCerts::key(&k, *port) };
            to.hosts.entry(key).or_insert(fp);
        }
        let _ = to.save(&dir);
    }
}

pub(crate) fn way_json(w: &rj_core::model::AccessWay) -> Value {
    let cred = match w.credential {
        _ if w.own.is_some() => json!("own"),
        rj_core::model::WayCredential::Target => json!("target"),
        rj_core::model::WayCredential::None => json!("none"),
        rj_core::model::WayCredential::Login(id) => json!(id),
    };
    json!({ "id": w.id, "name": w.name, "label": w.label(), "protocol": w.protocol.as_str(), "host": w.host, "port": w.port, "credential": cred, "options": options_value(&w.options) })
}

/// Zugangswege aus dem Eigenschaften-Tab prüfen. Optionen bekannter Wege (gleiche ID, gleiches Protokoll) bleiben.
/// Eigene Zugangsdaten eines Wegs aus dem Eigenschaften-Tab: Weg, Benutzername, neues Kennwort (falls eingegeben).
type OwnInput = (Uuid, String, Option<Secret>);

fn ways_from(v: &Value, t: &rj_core::Target, index: &Index) -> Result<(Vec<rj_core::model::AccessWay>, Vec<OwnInput>), String> {
    use rj_core::model::{AccessWay, WayCredential};
    let list = v.as_array().ok_or("Zugangswege: Liste erwartet")?;
    let mut out = Vec::new();
    let mut own = Vec::new();
    for (n, w) in list.iter().enumerate() {
        let nr = n + 1;
        let protocol = match w.get("protocol").and_then(Value::as_str) {
            Some("web") => Protocol::Web,
            Some("ssh") => Protocol::Ssh,
            Some("rdp") => Protocol::Rdp,
            Some("vnc") => Protocol::Vnc,
            Some("psrp") => Protocol::Psrp,
            _ => return Err(format!("Weg {nr}: Protokoll fehlt oder wird nicht unterstützt")),
        };
        let host = w.get("host").and_then(Value::as_str).unwrap_or_default().trim().to_string();
        if host.contains("://") {
            if protocol != Protocol::Web {
                return Err(format!("Weg {nr}: Eine URL gibt es nur bei Web; sonst Rechnername oder IP eintragen"));
            }
            if rj_core::weburl::endpoint(&host).is_none() {
                return Err(format!("Weg {nr}: URL ungültig; Beispiel: https://192.0.2.4:4343/"));
            }
        } else if host.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(format!("Weg {nr}: Adresse enthält Leer- oder Steuerzeichen"));
        }
        let port = match w.get("port") {
            None | Some(Value::Null) => None,
            Some(p) => {
                let n = p.as_u64().or_else(|| p.as_str().filter(|s| !s.trim().is_empty()).and_then(|s| s.trim().parse().ok()));
                match (n, p.as_str().is_some_and(|s| s.trim().is_empty())) {
                    (_, true) => None,
                    (Some(n), _) if (1..=65535).contains(&n) => Some(n as u16),
                    _ => return Err(format!("Weg {nr}: Port muss zwischen 1 und 65535 liegen")),
                }
            }
        };
        let own_input = w.get("credential").and_then(Value::as_str) == Some("own");
        let credential = match w.get("credential").and_then(Value::as_str) {
            None | Some("target") | Some("own") => WayCredential::Target,
            Some("none") => WayCredential::None,
            Some(s) => match Uuid::parse_str(s) {
                Ok(id) if index.login(id).is_some() => WayCredential::Login(id),
                _ => return Err(format!("Weg {nr}: Anmeldeobjekt nicht gefunden")),
            },
        };
        let id = w.get("id").and_then(Value::as_str).and_then(|s| Uuid::parse_str(s).ok());
        let mut way = AccessWay::new(protocol);
        if let Some(id) = id {
            way.id = id;
            if let Some(old) = t.ways.iter().find(|x| x.id == id && x.protocol == protocol) {
                way.options = old.options.clone();
            }
            // Eigene Zugangsdaten: bisherigen Eintrag behalten (Benutzer und Kennwort setzt `apply_way_credentials`).
            if own_input {
                way.own = t.ways.iter().find(|x| x.id == id).and_then(|x| x.own.clone());
            }
        }
        if own_input {
            let username = w.get("username").and_then(Value::as_str).unwrap_or_default().to_string();
            // Kennwort byte-genau; leer = unverändert.
            let password = w.get("password").and_then(Value::as_str).filter(|p| !p.is_empty()).map(Secret::from_str_exact);
            own.push((way.id, username, password));
        }
        // Optionen des Wegs: Felder aus dem Formular über die bisherigen legen (z. B. eine Tastenfolge bleibt),
        // leere Felder entfernen den Wert; danach wie beim Hauptweg prüfen.
        if let Some(Value::Object(edit)) = w.get("options") {
            let mut base = match options_value(&way.options) {
                Value::Object(m) => m,
                _ => serde_json::Map::new(),
            };
            for (k, v) in edit {
                match v {
                    Value::Null => {
                        base.remove(k);
                    }
                    Value::String(s) if s.trim().is_empty() => {
                        base.remove(k);
                    }
                    _ => {
                        base.insert(k.clone(), v.clone());
                    }
                }
            }
            let errs = validate_connection(protocol, "weg.example", None, Some(&Value::Object(base.clone())));
            if let Some(e) = errs.iter().find(|(k, _)| k.as_str() != "host" && k.as_str() != "port").map(|(_, e)| e) {
                return Err(format!("Weg {nr}: {e}"));
            }
            way.options = options_from(protocol, &Value::Object(base)).map_err(|e| format!("Weg {nr}: {e}"))?;
        }
        way.name = w.get("name").and_then(Value::as_str).unwrap_or_default().trim().to_string();
        way.host = host;
        way.port = port;
        way.credential = credential;
        out.push(way);
    }
    Ok((out, own))
}

