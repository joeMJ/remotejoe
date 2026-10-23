//! Zugangsdaten prüfen (Kontextmenü „Zugangsdaten prüfen …“): je Ziel und Zugangsweg nur die Anmeldung, danach
//! sofort trennen. Mehrere Prüfungen laufen gleichzeitig; Ziele mit denselben Zugangsdaten nacheinander, und nach
//! einer Ablehnung („Kennwort falsch“, „gesperrt“, „abgelaufen“) wird mit diesen Zugangsdaten nichts weiter versucht
//! – Fehlversuche zählen z. B. im AD gegen die Sperrschwelle.
//!
//! Zertifikate und Host-Schlüssel: bekannte gelten; ein unbekannter SSH-Host-Schlüssel wird wie beim Inventar
//! übernommen und gemeldet, ein geänderter nie. Unbekannte RDP-, VNC- und Proxmox-Zertifikate werden nicht
//! angenommen (Ergebnis „Zertifikat unbekannt“), damit keine Zugangsdaten an einen fremden Server gehen.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rj_core::inherit::{resolve_credential, CredentialResolution, ResolvedCredential};
use rj_core::model::{Node, WayCredential};
use rj_core::{edit, Protocol, Secret};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::app::App;
use crate::patch::JobSpec;

/// Höchstens so viele Sätze gleicher Zugangsdaten gleichzeitig.
pub const PARALLEL: usize = 8;

#[derive(Debug, Clone)]
pub struct CheckRow {
    pub target: Uuid,
    pub name: String,
    pub path: String,
    /// Hauptweg oder Name des Zugangswegs.
    pub way: String,
    pub protocol: &'static str,
    pub user: String,
    /// Herkunft der Zugangsdaten (Ziel, Ordner, Anmeldeobjekt „…“).
    pub origin: String,
    /// wartet, läuft, ok, abgelehnt, gesperrt, abgelaufen, nicht erreichbar, zertifikat, nicht geprüft,
    /// nicht prüfbar, übersprungen, fehler, abgebrochen
    pub state: &'static str,
    pub detail: String,
}

impl CheckRow {
    fn json(&self) -> Value {
        json!({ "id": self.target, "name": self.name, "path": self.path, "way": self.way, "protocol": self.protocol, "user": self.user, "origin": self.origin, "state": self.state, "detail": self.detail })
    }
}

#[derive(Default)]
pub struct CheckState {
    pub rows: Arc<Mutex<Vec<CheckRow>>>,
    pub done: Arc<AtomicUsize>,
    pub job: Option<u64>,
    pub title: String,
}

/// Eine vorbereitete Prüfung (Zugangsdaten schon aus der KDBX geholt).
struct Work {
    row: usize,
    spec: JobSpec,
}

/// Prüfungen mit denselben Zugangsdaten, nacheinander.
struct Group {
    items: Vec<Work>,
}

/// Ergebnis einer einzelnen Anmeldung: Zustand und Text.
type Outcome = (&'static str, String);

/// Nach diesen Ergebnissen nichts mehr mit denselben Zugangsdaten versuchen.
fn stops_group(state: &str) -> bool {
    matches!(state, "abgelehnt" | "gesperrt" | "abgelaufen")
}

/// SSH-Fehlertext (`remote::ssh_open`) einordnen.
pub fn classify_ssh(e: &str) -> &'static str {
    if e.contains("Anmeldung fehlgeschlagen") {
        "abgelehnt"
    } else if e.contains("Host-Schlüssel") {
        "zertifikat"
    } else if e.contains("Verbindung zu") || e.contains("Zeitüberschreitung") || e.contains("nicht erreichbar") {
        "nicht erreichbar"
    } else {
        "fehler"
    }
}

/// PowerShell-Fehlertext (New-PSSession) einordnen.
pub fn classify_ps(e: &str) -> &'static str {
    let l = e.to_lowercase();
    if l.contains("pwsh nicht gefunden") {
        "nicht prüfbar"
    } else if l.contains("locked out") || l.contains("account is locked") || l.contains("konto ist gesperrt") || l.contains("account has been locked") || l.contains("disabled") {
        "gesperrt"
    } else if l.contains("password has expired") || l.contains("kennwort ist abgelaufen") || l.contains("must change") {
        "abgelaufen"
    } else if l.contains("access is denied") || l.contains("zugriff verweigert") || l.contains("logon failure") || l.contains("user name or password is incorrect") || l.contains("benutzername oder kennwort") || l.contains("permission denied") {
        "abgelehnt"
    } else if l.contains("cannot connect") || l.contains("cannot find the computer") || l.contains("unreachable") || l.contains("timed out") || l.contains("nicht erreichbar") || l.contains("verbindung") {
        "nicht erreichbar"
    } else {
        "fehler"
    }
}

fn check_ssh(spec: &JobSpec, known: rj_ssh::known_hosts::KnownHosts) -> Outcome {
    match crate::overview::remote::ssh_open(spec, spec.port, known) {
        Ok((rt, conn, note, _, _)) => {
            rt.block_on(conn.disconnect());
            ("ok", note.unwrap_or_default())
        }
        Err(e) => (classify_ssh(&e), e),
    }
}

fn check_rdp(spec: &JobSpec, worker_dir: &std::path::Path) -> Outcome {
    use std::io::Write;
    let exe = worker_dir.join(if cfg!(windows) { "rj-worker-rdp.exe" } else { "rj-worker-rdp" });
    let password = String::from_utf8_lossy(&spec.credentials.password.0).into_owned();
    let mut line = json!({ "host": spec.host, "port": spec.port, "username": spec.credentials.username, "domain": spec.credentials.domain, "password": password }).to_string().into_bytes();
    drop(password);
    line.push(b'\n');
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("--auth-check").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return ("fehler", format!("{} nicht startbar: {e}", exe.display())),
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&line);
    }
    zeroize::Zeroize::zeroize(&mut line);
    let out = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => return ("fehler", e.to_string()),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let Some(v) = text.lines().rev().find_map(|l| serde_json::from_str::<Value>(l).ok()) else { return ("fehler", "Keine Antwort der RDP-Prüfung".into()) };
    let message = v["message"].as_str().unwrap_or_default().to_string();
    let state = match v["result"].as_str().unwrap_or("other") {
        "ok" => "ok",
        "auth" => "abgelehnt",
        "locked" => "gesperrt",
        "expired" => "abgelaufen",
        "unreachable" => "nicht erreichbar",
        "cert" => "zertifikat",
        _ => "fehler",
    };
    let message = if state == "zertifikat" { "Zertifikat unbekannt oder geändert – erst einmal verbinden bzw. über „Schlüssel und Zertifikate prüfen“ annehmen".to_string() } else { message };
    (state, message)
}

fn check_ps(spec: &JobSpec) -> Outcome {
    let transport = rj_core::model::PsrpTransport::effective(spec.transport.clone(), None);
    let t = rj_psrp::PsTarget { host: spec.host.clone(), username: spec.credentials.username.clone(), domain: spec.credentials.domain.clone(), transport };
    match rj_psrp::PsSession::start(&t, &Secret::from_bytes(spec.credentials.password.0.clone())) {
        Ok(s) => {
            drop(s);
            ("ok", String::new())
        }
        Err(e) => {
            let text = rj_psrp::explain(e.to_string());
            (classify_ps(&text), text)
        }
    }
}

fn check_proxmox(spec: &JobSpec) -> Outcome {
    let key = format!("{}:{}", spec.host, spec.port);
    let accepted = crate::hyper::known_proxmox_certs().get(&key).cloned();
    let unknown = Arc::new(AtomicBool::new(false));
    let flag = unknown.clone();
    let check: rj_proxmox::CertCheck = Arc::new(move |fp: &str| {
        let ok = accepted.as_deref() == Some(fp);
        if !ok {
            flag.store(true, Ordering::SeqCst);
        }
        ok
    });
    let host = if spec.host.contains(':') && !spec.host.starts_with('[') { format!("[{}]", spec.host) } else { spec.host.clone() };
    let client = match rj_proxmox::Client::new(&format!("https://{host}:{}", spec.port), crate::hyper::proxmox_auth(&spec.credentials), check, Duration::from_secs(10)) {
        Ok(c) => c,
        Err(e) => return ("fehler", e.to_string()),
    };
    match client.guests() {
        Ok(_) => ("ok", String::new()),
        Err(rj_proxmox::ProxmoxError::Unauthorized) => ("abgelehnt", "Anmeldung abgelehnt".into()),
        Err(e) if unknown.load(Ordering::SeqCst) => ("zertifikat", format!("Zertifikat unbekannt oder geändert – erst einmal verbinden ({e})")),
        Err(rj_proxmox::ProxmoxError::Io(e)) => ("nicht erreichbar", e.to_string()),
        Err(e) => ("fehler", e.to_string()),
    }
}

fn check_vnc(spec: &JobSpec) -> Outcome {
    let known = rj_core::config::data_dir().map(|d| rj_vnc::client::KnownCerts::new(d.join("vnc_certs")));
    let p = rj_vnc::client::VncParams {
        host: spec.host.clone(),
        port: spec.port,
        username: Some(spec.credentials.username.clone()).filter(|u| !u.is_empty()),
        password: Some(Secret::from_bytes(spec.credentials.password.0.clone())),
        shared: true,
        view_only: true,
        clipboard: rj_core::ClipboardDirection::Off,
        clipboard_limit: 0,
        prefer_tls: true,
        known_certs: known,
        connect_timeout: Duration::from_secs(10),
        extended_clipboard: false,
    };
    // Nur gespeicherte Zertifikate gelten (die Prüfung über `known_certs`); Unbekanntes wird abgelehnt.
    match rj_vnc::client::VncClient::connect(&p, Arc::new(|_| false)) {
        Ok(c) => {
            drop(c);
            ("ok", String::new())
        }
        Err(rj_vnc::client::VncError::AuthFailed(m)) => ("abgelehnt", m),
        Err(rj_vnc::client::VncError::CertificateRejected) => ("zertifikat", "Zertifikat unbekannt oder geändert – erst einmal verbinden".into()),
        Err(rj_vnc::client::VncError::Io(e)) => ("nicht erreichbar", e.to_string()),
        Err(e) => ("fehler", e.to_string()),
    }
}

fn check_one(spec: &JobSpec, known: &rj_ssh::known_hosts::KnownHosts, worker_dir: &std::path::Path) -> Outcome {
    match spec.protocol {
        Protocol::Ssh => check_ssh(spec, known.clone()),
        Protocol::Rdp => check_rdp(spec, worker_dir),
        Protocol::Psrp | Protocol::Hyperv => check_ps(spec),
        Protocol::Proxmox => check_proxmox(spec),
        Protocol::Vnc => check_vnc(spec),
        Protocol::Web => ("nicht prüfbar", "Web-Anmeldungen lassen sich nicht verlässlich prüfen".into()),
    }
}

fn run(groups: Vec<Group>, rows: Arc<Mutex<Vec<CheckRow>>>, done: Arc<AtomicUsize>, known: rj_ssh::known_hosts::KnownHosts, worker_dir: std::path::PathBuf, cancel: Arc<AtomicBool>) {
    let queue = Arc::new(Mutex::new(VecDeque::from(groups)));
    let mut threads = Vec::new();
    for _ in 0..PARALLEL {
        let (queue, rows, done, known, worker_dir, cancel) = (queue.clone(), rows.clone(), done.clone(), known.clone(), worker_dir.clone(), cancel.clone());
        threads.push(std::thread::spawn(move || loop {
            let Some(group) = queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front() else { break };
            let mut stopped: Option<String> = None;
            for w in group.items {
                let set = |state: &'static str, detail: String| {
                    let mut r = rows.lock().unwrap_or_else(|e| e.into_inner());
                    r[w.row].state = state;
                    r[w.row].detail = detail;
                };
                if cancel.load(Ordering::SeqCst) {
                    set("abgebrochen", String::new());
                } else if let Some(first) = &stopped {
                    set("nicht geprüft", format!("Dieselben Zugangsdaten wurden bei {first} abgelehnt – kein weiterer Versuch (Kontosperre)"));
                } else {
                    set("läuft", String::new());
                    let (state, detail) = check_one(&w.spec, &known, &worker_dir);
                    if stops_group(state) {
                        let name = rows.lock().unwrap_or_else(|e| e.into_inner())[w.row].name.clone();
                        stopped = Some(format!("„{name}“"));
                    }
                    set(state, detail);
                }
                done.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    for t in threads {
        let _ = t.join();
    }
}

impl App {
    /// Zugangsdaten eines Wegs bzw. des Hauptwegs auflösen; `Err` mit Grund fürs Überspringen.
    fn check_credential(&self, t: &rj_core::Target, way: Option<&rj_core::model::AccessWay>) -> Result<(ResolvedCredential, String), String> {
        let target_cred = || -> Result<(ResolvedCredential, String), String> {
            match resolve_credential(&self.index, t.id).map_err(|e| e.to_string())? {
                CredentialResolution::Found(r) => {
                    let origin = match &r.origin {
                        rj_core::inherit::Origin::Target => "Ziel".to_string(),
                        _ => "Ordner".to_string(),
                    };
                    let origin = match self.index.logins.iter().find(|l| l.entry.entry == r.reference.entry) {
                        Some(l) => format!("Anmeldeobjekt „{}“", l.name),
                        None => origin,
                    };
                    Ok((r, origin))
                }
                CredentialResolution::Ambiguous(_) => Err("Mehrere Zugangsdaten kommen in Frage".into()),
                _ => Err("Keine Zugangsdaten hinterlegt".into()),
            }
        };
        let Some(w) = way else { return target_cred() };
        if let Some(own) = &w.own {
            return Ok((ResolvedCredential { name: rj_core::DEFAULT_CREDENTIAL.into(), reference: own.clone(), origin: rj_core::inherit::Origin::Target, overrides: Default::default() }, "Weg".into()));
        }
        match w.credential {
            WayCredential::Target => target_cred(),
            WayCredential::Login(id) => {
                let l = self.index.login(id).ok_or("Das Anmeldeobjekt gibt es nicht mehr")?;
                Ok((ResolvedCredential { name: l.name.clone(), reference: l.entry.clone(), origin: rj_core::inherit::Origin::Target, overrides: Default::default() }, format!("Anmeldeobjekt „{}“", l.name)))
            }
            WayCredential::None => Err("Weg ohne Zugangsdaten".into()),
        }
    }

    /// Startet die Prüfung für die gewählten Ordner und Ziele (`ids`); nur ein Lauf zur Zeit.
    pub fn credcheck_start(&mut self, c: &Value) -> Result<Value, String> {
        if self.credcheck.job.is_some_and(|j| self.jobs.is_pending(j)) {
            return Err("Eine Prüfung der Zugangsdaten läuft bereits".into());
        }
        if self.demo {
            return Err("In der Vorführung nicht verfügbar".into());
        }
        let ids: Vec<Uuid> = c.get("ids").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().and_then(|s| Uuid::parse_str(s).ok())).collect()).unwrap_or_default();
        let mut targets = Vec::new();
        for id in ids {
            match edit::node(&self.index, id) {
                Some(Node::Target(_)) => targets.push(id),
                Some(Node::Folder(_)) => targets.extend(edit::targets_below(&self.index, id)),
                None => {}
            }
        }
        let mut seen = std::collections::HashSet::new();
        targets.retain(|t| seen.insert(*t));
        if targets.is_empty() {
            return Err("Keine Ziele in der Auswahl".into());
        }
        let mut rows: Vec<CheckRow> = Vec::new();
        // Gruppen gleicher Zugangsdaten (Benutzer, Domäne, Kennwort); nur im Speicher verglichen.
        let mut groups: Vec<((String, Option<String>, Vec<u8>), Group)> = Vec::new();
        for id in targets {
            let Some(t) = self.target_ref(id).cloned() else { continue };
            let path = self.index.path_to(id).unwrap_or_default().iter().rev().skip(1).rev().map(|n| rj_core::charset::mark_visible(n.name(), false).into_owned()).collect::<Vec<_>>().join(" › ");
            let name = rj_core::charset::mark_visible(&t.name, false).into_owned();
            let mut ways: Vec<Option<&rj_core::model::AccessWay>> = vec![None];
            ways.extend(t.ways.iter().map(Some));
            for w in ways {
                let view = match w {
                    None => t.clone(),
                    Some(w) => match t.via(w.id) {
                        Some(v) => v,
                        None => continue,
                    },
                };
                let mut row = CheckRow {
                    target: id,
                    name: name.clone(),
                    path: path.clone(),
                    way: w.map(|w| w.label()).unwrap_or_else(|| "Hauptweg".into()),
                    protocol: view.protocol.label(),
                    user: String::new(),
                    origin: String::new(),
                    state: "wartet",
                    detail: String::new(),
                };
                if view.host.trim().is_empty() {
                    row.state = "übersprungen";
                    row.detail = "Keine Adresse".into();
                    rows.push(row);
                    continue;
                }
                if view.protocol == Protocol::Web {
                    row.state = "nicht prüfbar";
                    row.detail = "Web-Anmeldungen lassen sich nicht verlässlich prüfen".into();
                    rows.push(row);
                    continue;
                }
                match self.check_credential(&t, w).and_then(|(r, origin)| self.credential_payload(&r, view.protocol).map(|p| (p, origin))) {
                    Ok((creds, origin)) => {
                        row.user = match &creds.domain {
                            Some(d) if !d.is_empty() => format!("{d}\\{}", creds.username),
                            _ => creds.username.clone(),
                        };
                        row.origin = origin;
                        let key = (creds.username.clone(), creds.domain.clone(), creds.password.0.clone());
                        let spec = JobSpec {
                            protocol: view.protocol,
                            host: view.host.clone(),
                            port: view.effective_port(),
                            credentials: creds,
                            transport: rj_core::inherit::resolve_psrp_transport(&self.index, id).ok().flatten(),
                            legacy: self.target_legacy_of(id),
                        };
                        let work = Work { row: rows.len(), spec };
                        match groups.iter_mut().find(|g| g.0 == key) {
                            Some(g) => g.1.items.push(work),
                            None => groups.push((key, Group { items: vec![work] })),
                        }
                    }
                    Err(e) => {
                        row.state = "übersprungen";
                        row.detail = e;
                    }
                }
                rows.push(row);
            }
        }
        let total = rows.len();
        let pending: usize = groups.iter().map(|g| g.1.items.len()).sum();
        let groups: Vec<Group> = groups.into_iter().map(|(_, g)| g).collect();
        let shared = Arc::new(Mutex::new(rows));
        let done = Arc::new(AtomicUsize::new(total - pending));
        self.credcheck.rows = shared.clone();
        self.credcheck.done = done.clone();
        self.credcheck.title = c.get("title").and_then(Value::as_str).unwrap_or_default().to_string();
        let known = self.known_hosts_ref();
        let worker_dir = crate::sessions::Manager::default_worker_dir();
        let job = self.jobs.spawn_cancellable(format!("Zugangsdaten prüfen ({pending})"), move |job, cancel| {
            run(groups, shared, done, known, worker_dir, cancel.clone());
            Box::new(move |_app: &mut App, _cancelled: bool| vec![crate::sessions::UiEvent::JobResult { job, kind: "credcheck".into(), target: String::new(), json: "{}".into() }])
        });
        self.credcheck.job = Some(job);
        Ok(json!({ "job": job, "total": total, "pending": pending }))
    }

    pub fn credcheck_rows(&self) -> Value {
        let rows = self.credcheck.rows.lock().unwrap_or_else(|e| e.into_inner());
        let running = self.credcheck.job.is_some_and(|j| self.jobs.is_pending(j));
        json!({
            "rows": rows.iter().map(CheckRow::json).collect::<Vec<_>>(),
            "done": self.credcheck.done.load(Ordering::Relaxed),
            "total": rows.len(),
            "running": running,
            "title": self.credcheck.title,
        })
    }

    pub fn credcheck_cancel(&mut self) -> Value {
        if let Some(j) = self.credcheck.job {
            self.jobs.cancel(j);
        }
        json!({ "ok": true })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rj_ipc::CredentialPayload;

    #[test]
    fn errors_are_classified() {
        assert_eq!(classify_ssh("Anmeldung fehlgeschlagen (versucht: Kennwort)"), "abgelehnt");
        assert_eq!(classify_ssh("Verbindung zu 192.0.2.1:22 fehlgeschlagen: refused"), "nicht erreichbar");
        assert_eq!(classify_ssh("Host-Schlüssel von 192.0.2.1:22 hat sich geändert"), "zertifikat");
        assert_eq!(classify_ps("Access is denied."), "abgelehnt");
        assert_eq!(classify_ps("The referenced account is currently locked out"), "gesperrt");
        assert_eq!(classify_ps("WinRM cannot complete the operation … cannot connect"), "nicht erreichbar");
        assert_eq!(classify_ps("pwsh nicht gefunden oder nicht startbar"), "nicht prüfbar");
        assert!(stops_group("abgelehnt") && stops_group("gesperrt") && !stops_group("nicht erreichbar") && !stops_group("ok"));
    }

    #[test]
    fn same_credentials_stop_after_the_first_rejection() {
        // Zwei Arbeiten mit derselben Gruppe gegen eine nicht vorhandene SSH-Gegenstelle mit falscher Anmeldung lassen
        // sich ohne Server nicht nachstellen; geprüft wird hier die Abbruchlogik über eine ausgedachte Gruppe.
        let rows = Arc::new(Mutex::new(vec![
            CheckRow { target: Uuid::new_v4(), name: "a".into(), path: String::new(), way: "Hauptweg".into(), protocol: "Web", user: "u".into(), origin: "Ziel".into(), state: "wartet", detail: String::new() },
            CheckRow { target: Uuid::new_v4(), name: "b".into(), path: String::new(), way: "Hauptweg".into(), protocol: "Web", user: "u".into(), origin: "Ziel".into(), state: "wartet", detail: String::new() },
        ]));
        let spec = |host: &str| JobSpec { protocol: Protocol::Web, host: host.into(), port: 443, credentials: CredentialPayload { username: "u".into(), domain: None, password: rj_ipc::SecretBytes(b"p".to_vec()), private_key: None }, transport: None, legacy: false };
        let groups = vec![Group { items: vec![Work { row: 0, spec: spec("192.0.2.1") }, Work { row: 1, spec: spec("192.0.2.2") }] }];
        let done = Arc::new(AtomicUsize::new(0));
        let d = tempfile::tempdir().unwrap();
        run(groups, rows.clone(), done.clone(), rj_ssh::known_hosts::KnownHosts::new(d.path().join("kh")), d.path().to_path_buf(), Arc::new(AtomicBool::new(false)));
        let r = rows.lock().unwrap();
        assert_eq!(done.load(Ordering::Relaxed), 2);
        assert_eq!((r[0].state, r[1].state), ("nicht prüfbar", "nicht prüfbar"), "Web wird nicht geprüft, stoppt die Gruppe aber nicht");
    }
}
