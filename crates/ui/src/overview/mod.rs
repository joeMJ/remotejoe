//! Objektübersicht beim Einfachklick: Zielname, angebotene Dienste (TCP), Inventar, Notizen und
//! Prüfungen (Ping, Traceroute).
//!
//! Ohne Anmeldung (beim Anzeigen): Namensauflösung, Portprüfung, MAC aus der Nachbartabelle und
//! – bei offenem RDP mit NLA – Rechnername und Windows-Version. Mit Anmeldung nur auf Knopfdruck:
//! SSH (Linux, Aruba `show version`) oder PowerShell (Windows). Notizen sind die Notizen des Ziels
//! (Index in der KDBX); das Inventar liegt als Datei im Datenordner.

pub mod inventory;
pub mod probe;
pub mod rdpname;
pub mod remote;
#[cfg(not(windows))]
mod sys_linux;
#[cfg(windows)]
mod sys_windows;
#[cfg(not(windows))]
use sys_linux as sys;
#[cfg(windows)]
use sys_windows as sys;

use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rj_core::inherit::{resolve_credential, CredentialResolution};
use rj_core::Protocol;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::app::App;
use crate::sessions::UiEvent;
use inventory::{now, Inventory, Store};

/// Wie lange ein Ergebnis der Portprüfung gilt.
const SERVICES_TTL: Duration = Duration::from_secs(60);

/// Laufender Ping oder Traceroute.
struct Run {
    target: Uuid,
    kind: &'static str,
    out: Arc<Mutex<Vec<u8>>>,
    done: Arc<AtomicBool>,
    child: Arc<Mutex<Option<std::process::Child>>>,
}

pub struct OverviewState {
    pub store: Store,
    services: HashMap<Uuid, (Instant, Value)>,
    run: Option<Run>,
    /// Laufende Abfragen je Ziel (keine doppelten Starts).
    busy: HashMap<Uuid, u64>,
}

impl Default for OverviewState {
    fn default() -> Self {
        OverviewState { store: Store::default_location(), services: HashMap::new(), run: None, busy: HashMap::new() }
    }
}

/// Erlaubter Host für Ping und Traceroute: nichts, was als Schalter gelesen würde.
pub fn valid_host(h: &str) -> bool {
    let h = probe::bare_host(h);
    !h.is_empty() && !h.starts_with(['-', '/']) && h.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_' | '%'))
}

fn inventory_json(inv: &Inventory) -> Value {
    let fields: Vec<Value> = inventory::FIELDS
        .iter()
        .map(|k| {
            let f = inv.fields.get(*k).cloned().unwrap_or_default();
            json!({ "key": k, "value": f.value, "source": f.source, "at": f.at, "manual": f.manual, "pending": f.pending })
        })
        .collect();
    json!(fields)
}

fn protocol_label(p: Protocol) -> &'static str {
    match p {
        Protocol::Rdp => "RDP",
        Protocol::Ssh => "SSH",
        Protocol::Vnc => "VNC",
        Protocol::Web => "Web",
        Protocol::Proxmox => "Proxmox",
        Protocol::Hyperv => "Hyper-V",
        Protocol::Psrp => "PowerShell",
        #[allow(unreachable_patterns)]
        _ => "Ziel",
    }
}

impl App {
    fn ov_target(&self, c: &Value) -> Result<rj_core::Target, String> {
        let id = c.get("id").and_then(Value::as_str).and_then(|s| Uuid::parse_str(s).ok()).ok_or("ID fehlt")?;
        self.target_ref(id).cloned().ok_or_else(|| "Das ist kein Verbindungsziel".into())
    }

    /// Daten für den Tab „Übersicht“.
    pub fn overview_json(&self, c: &Value) -> Result<Value, String> {
        let t = self.ov_target(c)?;
        let inv = self.overview.store.load(t.id);
        let services = self.overview.services.get(&t.id).filter(|(at, _)| at.elapsed() < SERVICES_TTL).map(|(_, v)| v.clone());
        let read_only = self.vault.as_ref().is_some_and(|v| v.index().read_only);
        Ok(json!({
            "id": t.id.to_string(),
            "name": rj_core::charset::mark_visible(&t.name, false),
            "protocol": protocol_label(t.protocol),
            "host": rj_core::charset::mark_visible(&t.host, false),
            "port": t.effective_port(),
            "notes": t.meta.notes,
            "notes_editable": !read_only && !self.demo,
            "inventory": inventory_json(&inv),
            "aruba": inv.value("system").is_some_and(|s| s.starts_with("Aruba")),
            "services": services,
            "busy": self.overview.busy.contains_key(&t.id),
            "run": self.overview_output(t.id),
        }))
    }

    /// Ohne Anmeldung: Portprüfung, Adresse, MAC und (bei RDP mit NLA) Rechnername. `force` umgeht den Zwischenspeicher.
    pub fn overview_probe(&mut self, c: &Value) -> Result<Value, String> {
        let t = self.ov_target(c)?;
        let force = c.get("force").and_then(Value::as_bool).unwrap_or(false);
        if !force {
            if let Some((at, v)) = self.overview.services.get(&t.id) {
                if at.elapsed() < SERVICES_TTL {
                    return Ok(json!({ "services": v }));
                }
            }
        }
        if self.demo {
            return Ok(json!({ "services": { "error": "In der Vorführung nicht verfügbar" } }));
        }
        let host = t.host.clone();
        let extra = (t.effective_port(), protocol_label(t.protocol));
        let id = t.id;
        let rdp_port = if t.protocol == Protocol::Rdp { t.effective_port() } else { 3389 };
        let job = self.jobs.spawn("Dienste prüfen", move |job| {
            let res = (|| -> Result<(Value, Vec<(&'static str, String, &'static str)>), String> {
                let ip = probe::resolve(&host)?;
                let list = probe::port_list(Some(extra));
                let services = probe::scan(ip, &list, Duration::from_millis(900));
                let mut found: Vec<(&'static str, String, &'static str)> = vec![("ip", ip.to_string(), "DNS")];
                // Die Portprüfung hat die Nachbartabelle gefüllt (falls im selben Segment).
                if let Some(mac) = sys::neighbor_mac(ip) {
                    found.push(("mac", mac, "ARP"));
                }
                if services.iter().any(|s| s.port == rdp_port && s.open) {
                    if let Ok(i) = rdpname::query(std::net::SocketAddr::new(ip, rdp_port), &host, Duration::from_secs(4)) {
                        if let Some(n) = i.machine_name() {
                            found.push(("name", n, "RDP (NTLM)"));
                        }
                        if let Some(s) = i.system() {
                            found.push(("system", s, "RDP (NTLM)"));
                        }
                    }
                }
                Ok((json!({ "ip": ip.to_string(), "list": services, "at": now() }), found))
            })();
            Box::new(move |app: &mut App, cancelled| {
                app.overview.busy.remove(&id);
                if cancelled {
                    return Vec::new();
                }
                let json = match res {
                    Ok((services, found)) => {
                        app.overview.services.insert(id, (Instant::now(), services.clone()));
                        let mut inv = app.overview.store.load(id);
                        let at = now();
                        for (k, v, src) in found {
                            // Die IP aus der Namensauflösung ersetzt keinen vorhandenen Wert (mehrere Adressen möglich).
                            if k == "ip" && inv.value("ip").is_some() {
                                continue;
                            }
                            inv.offer(k, &v, src, at);
                        }
                        let saved = app.overview.store.save(id, &inv);
                        json!({ "services": services, "inventory": inventory_json(&inv), "error": saved.err() })
                    }
                    Err(e) => json!({ "services": { "error": e } }),
                };
                vec![UiEvent::JobResult { job, kind: "ov_probe".into(), target: id.to_string(), json: json.to_string() }]
            })
        });
        self.overview.busy.insert(id, job);
        Ok(json!({ "pending": job }))
    }

    /// Mit Anmeldung: SSH (Linux, Aruba) oder PowerShell (Windows), Zugangsdaten aus der KDBX.
    pub fn overview_query(&mut self, c: &Value) -> Result<Value, String> {
        let t = self.ov_target(c)?;
        if self.demo {
            return Err("In der Vorführung nicht verfügbar".into());
        }
        let r = match resolve_credential(&self.index, t.id).map_err(|e| e.to_string())? {
            CredentialResolution::Found(r) => r,
            CredentialResolution::Ambiguous(_) => return Err("Mehrere Zugangsdaten kommen in Frage. Bitte im Ziel einen Datensatz festlegen.".into()),
            _ => return Err("Für dieses Ziel sind keine Zugangsdaten hinterlegt".into()),
        };
        // Windows-Ziele über PowerShell; alles andere über SSH (bei Web-Zielen auf Port 22, z. B. Aruba-VC).
        let windows = matches!(t.protocol, Protocol::Rdp | Protocol::Psrp);
        let cred_protocol = if windows { t.protocol } else { Protocol::Ssh };
        let creds = self.credential_payload(&r, cred_protocol)?;
        let spec = crate::patch::JobSpec {
            protocol: t.protocol,
            host: t.host.clone(),
            port: t.effective_port(),
            credentials: creds,
            transport: rj_core::inherit::resolve_psrp_transport(&self.index, t.id).ok().flatten(),
            legacy: self.target_legacy_of(t.id),
        };
        let ssh_port = if t.protocol == Protocol::Ssh { t.effective_port() } else { 22 };
        let known = self.known_hosts_ref();
        let ip = self.overview.services.get(&t.id).and_then(|(_, v)| v["ip"].as_str().map(str::to_string));
        let id = t.id;
        let job = self.jobs.spawn("Inventar abfragen", move |job| {
            let res = if windows { remote::ps_inventory(&spec, ip.as_deref()) } else { remote::ssh_inventory(&spec, ssh_port, ip.as_deref(), known) };
            drop(spec);
            Box::new(move |app: &mut App, cancelled| {
                app.overview.busy.remove(&id);
                if cancelled {
                    return Vec::new();
                }
                let json = match res {
                    Ok((found, src)) => {
                        let mut inv = app.overview.store.load(id);
                        let at = now();
                        for (k, v) in found {
                            inv.offer(k, &v, src, at);
                        }
                        let saved = app.overview.store.save(id, &inv);
                        json!({ "inventory": inventory_json(&inv), "error": saved.err() })
                    }
                    Err(e) => json!({ "error": e }),
                };
                vec![UiEvent::JobResult { job, kind: "ov_query".into(), target: id.to_string(), json: json.to_string() }]
            })
        });
        self.overview.busy.insert(id, job);
        Ok(json!({ "pending": job }))
    }

    /// Inventarfeld von Hand setzen (`value`) oder einen vorgemerkten Wert übernehmen/verwerfen (`accept`).
    pub fn overview_field(&mut self, c: &Value) -> Result<Value, String> {
        let t = self.ov_target(c)?;
        let key = c.get("field").and_then(Value::as_str).ok_or("Feld fehlt")?;
        let mut inv = self.overview.store.load(t.id);
        if let Some(a) = c.get("accept").and_then(Value::as_bool) {
            inv.decide(key, a);
        } else {
            inv.set_manual(key, c.get("value").and_then(Value::as_str).unwrap_or_default(), now())?;
        }
        self.overview.store.save(t.id, &inv)?;
        Ok(json!({ "inventory": inventory_json(&inv) }))
    }

    /// Ping oder Traceroute starten (ein Lauf zur Zeit; ein neuer beendet den alten).
    pub fn overview_run(&mut self, c: &Value) -> Result<Value, String> {
        let t = self.ov_target(c)?;
        let kind = match c.get("kind").and_then(Value::as_str) {
            Some("ping") => "ping",
            Some("trace") => "trace",
            _ => return Err("Unbekannte Prüfung".into()),
        };
        let host = probe::bare_host(&t.host).to_string();
        if !valid_host(&host) {
            return Err(format!("Ungültige Adresse „{host}“"));
        }
        self.overview_stop();
        let mut cmd = if kind == "ping" { sys::ping_command(&host) } else { sys::trace_command(&host) };
        cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("{} lässt sich nicht starten: {e}", if kind == "ping" { "Ping" } else { "Traceroute" }))?;
        let out: Arc<Mutex<Vec<u8>>> = Arc::default();
        let done = Arc::new(AtomicBool::new(false));
        let readers: Vec<_> = [child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>), child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>)]
            .into_iter()
            .flatten()
            .map(|mut r| {
                let out = out.clone();
                std::thread::spawn(move || {
                    let mut buf = [0u8; 1024];
                    while let Ok(n) = r.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        out.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(&buf[..n]);
                    }
                })
            })
            .collect();
        let child = Arc::new(Mutex::new(Some(child)));
        let (c2, d2) = (child.clone(), done.clone());
        std::thread::spawn(move || {
            for r in readers {
                let _ = r.join();
            }
            if let Some(mut ch) = c2.lock().unwrap_or_else(|e| e.into_inner()).take() {
                let _ = ch.wait();
            }
            d2.store(true, Ordering::Relaxed);
        });
        self.overview.run = Some(Run { target: t.id, kind, out, done, child });
        Ok(json!({ "running": kind }))
    }

    pub fn overview_stop(&mut self) {
        if let Some(r) = self.overview.run.take() {
            if let Some(ch) = r.child.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                let _ = ch.kill();
            }
        }
    }

    /// Ausgabe des laufenden oder letzten Laufs für dieses Ziel.
    pub fn overview_output(&self, target: Uuid) -> Value {
        match &self.overview.run {
            Some(r) if r.target == target => json!({
                "kind": r.kind,
                "text": sys::decode_output(&r.out.lock().unwrap_or_else(|e| e.into_inner())),
                "done": r.done.load(Ordering::Relaxed),
            }),
            _ => Value::Null,
        }
    }

    /// Notizen des Ziels speichern (Index in der KDBX).
    pub fn overview_notes(&mut self, c: &Value) -> Result<Value, String> {
        let t = self.ov_target(c)?;
        let text = c.get("notes").and_then(Value::as_str).unwrap_or_default().to_string();
        self.set_target_notes(t.id, text)?;
        Ok(json!({ "ok": true }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_that_could_be_options_are_refused() {
        assert!(valid_host("192.0.2.1") && valid_host("srv01.example") && valid_host("[fe80::1%eth0]"));
        assert!(!valid_host("-c 1") && !valid_host("--help") && !valid_host("/x") && !valid_host("a b") && !valid_host(""));
    }

    #[test]
    fn ping_runs_and_its_output_arrives() {
        let mut app = App::new(false);
        let mut idx = rj_core::Index::default();
        let t = rj_core::Target::new("lokal", Protocol::Ssh, "127.0.0.1");
        let id = t.id;
        idx.root.children.push(rj_core::Node::Target(t));
        app.index = idx;
        let dir = tempfile::tempdir().unwrap();
        app.overview.store = Store { dir: Some(dir.path().to_path_buf()) };
        let c = json!({ "id": id.to_string(), "kind": "ping" });
        match app.overview_run(&c) {
            Ok(_) => {}
            // Ohne ping im Testsystem (z. B. schlanker Container) gibt es nichts zu prüfen.
            Err(e) if e.contains("starten") => return,
            Err(e) => panic!("{e}"),
        }
        let start = Instant::now();
        while !app.overview_output(id)["done"].as_bool().unwrap_or(false) && start.elapsed() < Duration::from_secs(15) {
            std::thread::sleep(Duration::from_millis(100));
        }
        let o = app.overview_output(id);
        assert_eq!(o["kind"], "ping");
        assert!(o["text"].as_str().unwrap().contains("127.0.0.1"), "{o}");
        assert!(app.overview_run(&json!({ "id": id.to_string(), "kind": "x" })).is_err());
        let v = app.overview_json(&json!({ "id": id.to_string() })).unwrap();
        assert_eq!((v["name"].as_str(), v["protocol"].as_str(), v["inventory"].as_array().map(Vec::len)), (Some("lokal"), Some("SSH"), Some(inventory::FIELDS.len())));
        let r = app.overview_field(&json!({ "id": id.to_string(), "field": "vcpu", "value": "4" })).unwrap();
        assert_eq!(r["inventory"][3]["value"], "4");
        assert_eq!(r["inventory"][3]["manual"], true);
    }
}
