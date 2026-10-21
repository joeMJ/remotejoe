//! Objektübersicht beim Einfachklick: Zielname, angebotene Dienste (TCP), Inventar, Notizen und
//! Prüfungen (Ping, Traceroute).
//!
//! Ohne Anmeldung (beim Anzeigen): Namensauflösung, Portprüfung, MAC aus der Nachbartabelle und
//! – bei offenem RDP mit NLA – der Rechnername. Mit Anmeldung nur auf Knopfdruck:
//! SSH (Linux, Aruba `show version`) oder PowerShell (Windows). Notizen sind die Notizen des Ziels
//! (Index in der KDBX); das Inventar liegt als Datei im Datenordner.

pub mod inventory;
pub mod mass;
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

/// Anleitungen, auf die Fehlermeldungen von „Inventar abfragen“ verweisen.
pub const HELP_WINRM: &str = "https://github.com/joeMJ/remotejoe/blob/main/docs/ZIELE-EINRICHTEN.md";
pub const HELP_OPENSSH: &str = "https://github.com/joeMJ/remotejoe#host-f%C3%BCr-powershell-%C3%BCber-ssh-einrichten";

/// Verweise zu einer gescheiterten Abfrage: PowerShell → WinRM einrichten (bei Windows auch OpenSSH als
/// Alternative); SSH auf einem Windows-Ziel → OpenSSH einrichten.
fn help_links(windows: bool, windows_target: bool) -> Value {
    let winrm = json!({ "text": "Anleitung: WinRM auf dem Windows-Ziel einrichten", "url": HELP_WINRM });
    let ssh = json!({ "text": "Anleitung: OpenSSH (mit PowerShell) auf Windows einrichten", "url": HELP_OPENSSH });
    match (windows, windows_target) {
        (true, _) => json!([winrm, ssh]),
        (false, true) => json!([ssh, winrm]),
        (false, false) => json!([]),
    }
}

/// Scheiterte die Abfrage, weil der Dienst fehlt oder nicht erreichbar ist (und nicht an Schlüssel oder Anmeldung)?
fn service_missing(e: &str) -> bool {
    let l = e.to_lowercase();
    !["host-schlüssel", "anmeld", "auth", "kennwort", "passwor", "zugriff verweigert", "access is denied", "zugangsdaten"].iter().any(|w| l.contains(w))
}

/// Wie lange ein Ergebnis der Portprüfung gilt.
const SERVICES_TTL: Duration = Duration::from_secs(60);

/// Laufender Ping oder Traceroute.
struct Run {
    key: String,
    kind: &'static str,
    out: Arc<Mutex<Vec<u8>>>,
    done: Arc<AtomicBool>,
    child: Arc<Mutex<Option<std::process::Child>>>,
}

pub struct OverviewState {
    pub store: Store,
    /// Ergebnis der Portprüfung je Ziel bzw. Gast.
    services: HashMap<String, (Instant, Value)>,
    run: Option<Run>,
    /// Laufende Abfragen je Ziel bzw. Gast (keine doppelten Starts).
    busy: HashMap<String, u64>,
    /// Geräteart je Ziel aus dem Inventar (eigenes Symbol im Baum, z. B. Aruba-AP).
    pub devices: HashMap<Uuid, &'static str>,
}

impl Default for OverviewState {
    fn default() -> Self {
        let store = Store::default_location();
        let devices = store.scan_devices();
        OverviewState { store, services: HashMap::new(), run: None, busy: HashMap::new(), devices }
    }
}

impl OverviewState {
    /// Zuletzt als offen erkannte TCP-Ports eines Ziels (Dienstprüfung der Übersicht; ohne Ablaufzeit).
    pub fn open_ports(&self, key: &str) -> Vec<u16> {
        self.services
            .get(key)
            .and_then(|(_, v)| v["list"].as_array().map(|l| l.iter().filter(|s| s["open"] == true).filter_map(|s| s["port"].as_u64()).map(|p| p as u16).collect()))
            .unwrap_or_default()
    }

    /// Inventar speichern und die Geräteart des Ziels nachführen.
    pub fn save_inventory(&mut self, key: &str, inv: &Inventory) -> Result<(), String> {
        let r = self.store.save_key(key, inv);
        if let Ok(id) = Uuid::parse_str(key) {
            match inventory::device_kind(inv) {
                Some(k) => self.devices.insert(id, k),
                None => self.devices.remove(&id),
            };
        }
        r
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

/// Was die Übersicht zeigt: ein Ziel aus dem Baum oder ein Gast eines Hypervisors.
enum Subject {
    Target(rj_core::Target),
    Guest { hv: Uuid, hv_name: String, hv_protocol: Protocol, guest: String, name: String, running: bool, address: Option<String> },
}

impl Subject {
    fn key(&self) -> String {
        match self {
            Subject::Target(t) => t.id.to_string(),
            Subject::Guest { hv, guest, .. } => format!("gast-{hv}-{guest}"),
        }
    }
}

fn str_of<'a>(c: &'a Value, k: &str) -> Option<&'a str> {
    c.get(k).and_then(Value::as_str).filter(|s| !s.is_empty())
}

impl App {
    fn ov_subject(&self, c: &Value) -> Result<Subject, String> {
        if let Some(hv) = str_of(c, "hv") {
            let hv = Uuid::parse_str(hv).map_err(|_| "Hypervisor ungültig")?;
            let t = self.target_ref(hv).ok_or("Hypervisor nicht gefunden")?;
            return Ok(Subject::Guest {
                hv,
                hv_name: rj_core::charset::mark_visible(&t.name, false).into_owned(),
                hv_protocol: t.protocol,
                guest: str_of(c, "guest").ok_or("Gast fehlt")?.to_string(),
                name: str_of(c, "name").unwrap_or("Gast").to_string(),
                running: str_of(c, "state") == Some("running"),
                address: str_of(c, "address").map(str::to_string),
            });
        }
        let id = str_of(c, "id").and_then(|s| Uuid::parse_str(s).ok()).ok_or("ID fehlt")?;
        let mut t = self.target_ref(id).cloned().ok_or("Das ist kein Verbindungsziel")?;
        // Web-Ziel: Rechner und Port stehen in der URL; Übersicht, Ping und Portprüfung nehmen sie von dort.
        if t.protocol == Protocol::Web {
            let (host, port) = App::endpoint_of(&t);
            t.host = host;
            t.port = Some(port);
        }
        Ok(Subject::Target(t))
    }

    /// Adresse für Portprüfung und Ping: beim Ziel der Host, beim Gast die gemeldete bzw. ermittelte IP.
    fn ov_host(&self, s: &Subject) -> Option<String> {
        match s {
            Subject::Target(t) => Some(t.host.clone()).filter(|h| !h.trim().is_empty()),
            Subject::Guest { address, .. } => address.clone().or_else(|| self.overview.store.load_key(&s.key()).value("ip").map(str::to_string)),
        }
    }

    /// Daten für den Tab „Übersicht“.
    pub fn overview_json(&self, c: &Value) -> Result<Value, String> {
        let s = self.ov_subject(c)?;
        let key = s.key();
        let inv = self.overview.store.load_key(&key);
        let services = self.overview.services.get(&key).filter(|(at, _)| at.elapsed() < SERVICES_TTL).map(|(_, v)| v.clone());
        let read_only = self.vault.as_ref().is_some_and(|v| v.index().read_only);
        let mut v = json!({
            "key": key,
            "inventory": inventory_json(&inv),
            "aruba": inv.value("system").is_some_and(|s| s.starts_with("Aruba")),
            "services": services,
            "busy": self.overview.busy.contains_key(&key),
            "run": self.overview_output_key(&key),
        });
        // Brotkrumen: Ordner bis zum Ziel bzw. bis zum Hypervisor des Gasts (`path_to` beginnt unter der Wurzel).
        let crumb_of = |id: Uuid, with_self: bool| -> String {
            let names: Vec<String> = self.index.path_to(id).unwrap_or_default().iter().map(|n| rj_core::charset::mark_visible(n.name(), false).into_owned()).collect();
            let n = if with_self { names.len() } else { names.len().saturating_sub(1) };
            names[..n].join(" › ")
        };
        v["path"] = json!(match &s {
            Subject::Target(t) => crumb_of(t.id, false),
            Subject::Guest { hv, .. } => crumb_of(*hv, true),
        });
        match &s {
            Subject::Target(t) => {
                v["name"] = json!(rj_core::charset::mark_visible(&t.name, false));
                v["sub"] = json!(format!("{} · {}:{}", protocol_label(t.protocol), rj_core::charset::mark_visible(&t.host, false), t.effective_port()));
                v["notes"] = json!(t.meta.notes);
                // Zugangswege: Knopf je Weg in der Kopfkarte; hervorgehoben der Weg für den Doppelklick.
                v["main_way"] = json!(protocol_label(t.protocol));
                v["ways"] = json!(t.ways.iter().map(|w| json!({ "id": w.id, "label": w.label(), "protocol": w.protocol.as_str() })).collect::<Vec<_>>());
                v["open_with"] = json!(t.open_with);
                v["notes_editable"] = json!(!read_only && !self.demo);
                let (on, src) = rj_core::inherit::resolve_inventory(&self.index, t.id).unwrap_or((false, rj_core::inherit::InventorySource::Default));
                v["can_query"] = json!(on);
                v["inventory_on"] = json!(on);
                v["inventory_source"] = json!(crate::editor::inventory_source_text(src, on));
                v["inventory_switchable"] = json!(!read_only && !self.demo);
            }
            Subject::Guest { hv_name, hv_protocol, name, .. } => {
                v["name"] = json!(name);
                v["sub"] = json!(format!("Gast auf {} ({})", hv_name, protocol_label(*hv_protocol)));
                v["guest"] = json!(true);
                v["can_query"] = json!(false);
            }
        }
        Ok(v)
    }

    /// Ohne Anmeldung am Ziel: beim Gast zuerst das Inventar vom Hypervisor, dann (mit bekannter Adresse)
    /// Portprüfung, MAC aus der Nachbartabelle und bei RDP mit NLA Rechnername. `force` umgeht den Zwischenspeicher.
    pub fn overview_probe(&mut self, c: &Value) -> Result<Value, String> {
        let s = self.ov_subject(c)?;
        let key = s.key();
        let force = c.get("force").and_then(Value::as_bool).unwrap_or(false);
        if !force {
            if let Some((at, v)) = self.overview.services.get(&key) {
                if at.elapsed() < SERVICES_TTL {
                    return Ok(json!({ "services": v }));
                }
            }
        }
        if self.overview.busy.contains_key(&key) {
            return Ok(json!({ "pending": self.overview.busy[&key] }));
        }
        if self.demo {
            return Ok(json!({ "services": { "error": "In der Vorführung nicht verfügbar" } }));
        }
        let (extra, rdp_port, guest_task) = match &s {
            Subject::Target(t) => (Some((t.effective_port(), protocol_label(t.protocol))), if t.protocol == Protocol::Rdp { t.effective_port() } else { 3389 }, None),
            Subject::Guest { hv, guest, running, .. } => (None, 3389, Some(self.guest_inventory_task(*hv, guest, *running))),
        };
        let mut host = self.ov_host(&s);
        let job_key = key.clone();
        let job = self.jobs.spawn("Dienste prüfen", move |job| {
            let mut found: Vec<(&'static str, String, &'static str)> = Vec::new();
            let mut guest_error = None;
            if let Some(task) = guest_task {
                match task.and_then(|t| t()) {
                    Ok((f, src)) => {
                        for (k, v) in f {
                            if k == "ip" && host.is_none() {
                                host = Some(v.clone());
                            }
                            found.push((k, v, src));
                        }
                    }
                    Err(e) => guest_error = Some(e),
                }
            }
            let services = match host.as_deref().map(probe::resolve) {
                None => json!({ "error": guest_error.clone().unwrap_or_else(|| "Keine Adresse bekannt".into()) }),
                Some(Err(e)) => json!({ "error": e }),
                Some(Ok(ip)) => {
                    let list = probe::port_list(extra);
                    let svc = probe::scan(ip, &list, Duration::from_millis(900));
                    found.push(("ip", ip.to_string(), "DNS"));
                    // Die Portprüfung hat die Nachbartabelle gefüllt (falls im selben Segment).
                    if let Some(mac) = sys::neighbor_mac(ip) {
                        found.push(("mac", mac, "ARP"));
                    }
                    if svc.iter().any(|x| x.port == rdp_port && x.open) {
                        // Nur den Namen: Die Version in der NTLM-Antwort ist bei Nicht-Windows-Servern (FreeRDP,
                        // GNOME-Remote-Desktop, xrdp) eine feste Angabe; das System kommt aus „Inventar abfragen“.
                        if let Ok(i) = rdpname::query(std::net::SocketAddr::new(ip, rdp_port), host.as_deref().unwrap_or_default(), Duration::from_secs(4)) {
                            if let Some(n) = i.machine_name() {
                                found.push(("name", n, "RDP (NTLM)"));
                            }
                        }
                    }
                    json!({ "ip": ip.to_string(), "list": svc, "at": now(), "guest_error": guest_error })
                }
            };
            Box::new(move |app: &mut App, cancelled| {
                app.overview.busy.remove(&job_key);
                if cancelled {
                    return Vec::new();
                }
                let mut inv = app.overview.store.load_key(&job_key);
                let at = now();
                for (k, v, src) in found {
                    // Die IP aus der Namensauflösung ersetzt keinen vorhandenen Wert (mehrere Adressen möglich).
                    if k == "ip" && src == "DNS" && inv.value("ip").is_some() {
                        continue;
                    }
                    inv.offer(k, &v, src, at);
                }
                let saved = app.overview.save_inventory(&job_key, &inv);
                if services.get("list").is_some() {
                    app.overview.services.insert(job_key.clone(), (Instant::now(), services.clone()));
                }
                let json = json!({ "services": services, "inventory": inventory_json(&inv), "error": saved.err() });
                vec![UiEvent::JobResult { job, kind: "ov_probe".into(), target: job_key, json: json.to_string() }]
            })
        });
        self.overview.busy.insert(key, job);
        Ok(json!({ "pending": job }))
    }

    /// Mit Anmeldung am Ziel: SSH (Linux, Aruba) oder PowerShell (Windows), Zugangsdaten aus der KDBX.
    pub fn overview_query(&mut self, c: &Value) -> Result<Value, String> {
        let Subject::Target(t) = self.ov_subject(c)? else { return Err("Gäste liefern ihr Inventar über den Hypervisor".into()) };
        if !rj_core::inherit::resolve_inventory(&self.index, t.id).map(|(on, _)| on).unwrap_or(false) {
            return Err("Inventar ist für dieses Ziel abgeschaltet (Übersicht oder Einstellungen → Allgemein)".into());
        }
        if self.demo {
            return Err("In der Vorführung nicht verfügbar".into());
        }
        let (t, r) = self.inventory_view(&t)?;
        let key = t.id.to_string();
        let open = |p: u16| {
            self.overview.services.get(&key).and_then(|(_, v)| v["list"].as_array().map(|l| l.iter().any(|s| s["port"] == p && s["open"] == true)))
        };
        // Ist SSH offen, immer der eigene SSH-Client (Kennwort aus der KDBX; Windows über OpenSSH und PowerShell).
        // PowerShell-Remoting nur ohne SSH: unter Windows über WinRM; unter Linux kann pwsh kein WinRM.
        let ssh_open = open(22) == Some(true) || t.protocol == Protocol::Ssh;
        let windows_target = matches!(t.protocol, Protocol::Rdp | Protocol::Psrp) || open(3389) == Some(true) || open(5985) == Some(true) || open(5986) == Some(true);
        let windows = !ssh_open && matches!(t.protocol, Protocol::Rdp | Protocol::Psrp);
        if windows && !cfg!(windows) {
            return Ok(json!({ "error": "WinRM ist von Linux aus nicht nutzbar (PowerShell unter Linux hat keinen WinRM-Client). Bitte OpenSSH auf dem Windows-Ziel einrichten.", "help": help_links(false, true) }));
        }
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
        let help = help_links(windows, windows_target);
        let known = self.known_hosts_ref();
        let ip = self.overview.services.get(&key).and_then(|(_, v)| v["ip"].as_str().map(str::to_string));
        let job_key = key.clone();
        let job = self.jobs.spawn("Inventar abfragen", move |job| {
            let res = if windows { remote::ps_inventory(&spec, ip.as_deref()) } else { remote::ssh_inventory(&spec, ssh_port, ip.as_deref(), known, windows_target) };
            drop(spec);
            Box::new(move |app: &mut App, cancelled| {
                app.overview.busy.remove(&job_key);
                if cancelled {
                    return Vec::new();
                }
                let json = match res {
                    Ok((found, src, note)) => {
                        let mut inv = app.overview.store.load_key(&job_key);
                        let at = now();
                        for (k, v) in found {
                            inv.offer(k, &v, src, at);
                        }
                        let saved = app.overview.save_inventory(&job_key, &inv);
                        let way = Uuid::parse_str(&job_key).is_ok_and(|id| app.aruba_web_way(id));
                        json!({ "inventory": inventory_json(&inv), "error": saved.err(), "note": note, "ways_changed": way })
                    }
                    // Verweise nur, wenn der Dienst fehlt oder nicht erreichbar ist – nicht bei Host-Schlüssel oder Anmeldung.
                    Err(e) => json!({ "error": e, "help": if service_missing(&e) { help } else { json!([]) } }),
                };
                vec![UiEvent::JobResult { job, kind: "ov_query".into(), target: job_key, json: json.to_string() }]
            })
        });
        self.overview.busy.insert(key, job);
        Ok(json!({ "pending": job }))
    }

    /// Inventarfeld von Hand setzen (`value`) oder einen vorgemerkten Wert übernehmen/verwerfen (`accept`).
    pub fn overview_field(&mut self, c: &Value) -> Result<Value, String> {
        let key = self.ov_subject(c)?.key();
        let field = c.get("field").and_then(Value::as_str).ok_or("Feld fehlt")?;
        let mut inv = self.overview.store.load_key(&key);
        if let Some(a) = c.get("accept").and_then(Value::as_bool) {
            inv.decide(field, a);
        } else {
            inv.set_manual(field, c.get("value").and_then(Value::as_str).unwrap_or_default(), now())?;
        }
        self.overview.save_inventory(&key, &inv)?;
        Ok(json!({ "inventory": inventory_json(&inv) }))
    }

    /// Ping oder Traceroute starten (ein Lauf zur Zeit; ein neuer beendet den alten).
    pub fn overview_run(&mut self, c: &Value) -> Result<Value, String> {
        let s = self.ov_subject(c)?;
        let kind = match c.get("kind").and_then(Value::as_str) {
            Some("ping") => "ping",
            Some("trace") => "trace",
            _ => return Err("Unbekannte Prüfung".into()),
        };
        let host = self.ov_host(&s).map(|h| probe::bare_host(&h).to_string()).ok_or("Keine Adresse bekannt")?;
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
        self.overview.run = Some(Run { key: s.key(), kind, out, done, child });
        Ok(json!({ "running": kind }))
    }

    pub fn overview_stop(&mut self) {
        if let Some(r) = self.overview.run.take() {
            if let Some(ch) = r.child.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                let _ = ch.kill();
            }
        }
    }

    /// Ausgabe des laufenden oder letzten Laufs für dieses Ziel bzw. diesen Gast.
    pub fn overview_output(&self, c: &Value) -> Value {
        match self.ov_subject(c) {
            Ok(s) => self.overview_output_key(&s.key()),
            Err(_) => Value::Null,
        }
    }

    fn overview_output_key(&self, key: &str) -> Value {
        match &self.overview.run {
            Some(r) if r.key == key => json!({
                "kind": r.kind,
                "text": sys::decode_output(&r.out.lock().unwrap_or_else(|e| e.into_inner())),
                "done": r.done.load(Ordering::Relaxed),
            }),
            _ => Value::Null,
        }
    }

    /// Notizen des Ziels speichern (Index in der KDBX); Gäste haben keine.
    /// Für das Inventar: das Ziel über seinen SSH-Zugangsweg, wenn der Hauptweg kein SSH ist, sonst das Ziel selbst;
    /// dazu die passenden Zugangsdaten (die des Wegs bzw. des Ziels).
    pub(crate) fn inventory_view(&self, t: &rj_core::Target) -> Result<(rj_core::Target, rj_core::inherit::ResolvedCredential), String> {
        use rj_core::model::WayCredential;
        let target_cred = || match resolve_credential(&self.index, t.id).map_err(|e| e.to_string())? {
            CredentialResolution::Found(r) => Ok(r),
            CredentialResolution::Ambiguous(_) => Err("Mehrere Zugangsdaten kommen in Frage. Bitte im Ziel einen Datensatz festlegen.".to_string()),
            _ => Err("Für dieses Ziel sind keine Zugangsdaten hinterlegt".to_string()),
        };
        if t.protocol != Protocol::Ssh {
            if let Some(w) = t.ways.iter().find(|w| w.protocol == Protocol::Ssh) {
                let view = t.via(w.id).ok_or("Zugangsweg nicht gefunden")?;
                let r = match w.credential {
                    WayCredential::Target => target_cred()?,
                    WayCredential::Login(id) => {
                        let l = self.index.login(id).ok_or("Das Anmeldeobjekt des SSH-Wegs gibt es nicht mehr")?;
                        rj_core::inherit::ResolvedCredential { name: l.name.clone(), reference: l.entry.clone(), origin: rj_core::inherit::Origin::Target, overrides: Default::default() }
                    }
                    WayCredential::None => return Err("Der SSH-Weg dieses Ziels hat keine Zugangsdaten".into()),
                };
                return Ok((view, r));
            }
        }
        Ok((t.clone(), target_cred()?))
    }

    /// Vorschläge für Zugangswege aus den angebotenen Diensten (Portprüfung der Übersicht): Web auf 4343, 443
    /// bzw. 80, SSH, RDP, VNC – ohne den Hauptweg.
    pub fn ways_suggest(&self, c: &Value) -> Result<Value, String> {
        let Subject::Target(t) = self.ov_subject(c)? else { return Err("Nur für Ziele".into()) };
        let key = t.id.to_string();
        let Some((_, svc)) = self.overview.services.get(&key) else {
            return Err("Die angebotenen Dienste sind noch nicht bekannt. Bitte das Ziel einmal anklicken (Übersicht) und danach erneut vorschlagen.".into());
        };
        let open: Vec<u64> = svc["list"].as_array().map(|l| l.iter().filter(|s| s["open"] == true).filter_map(|s| s["port"].as_u64()).collect()).unwrap_or_default();
        let has = |p: u64| open.contains(&p);
        let main = (t.protocol, u64::from(t.effective_port()));
        let host = t.host.trim();
        let h = if host.contains(':') && !host.starts_with('[') { format!("[{host}]") } else { host.to_string() };
        let mut out = Vec::new();
        let mut web = |port: u64, url: String, name: &str| {
            if main != (Protocol::Web, port) {
                out.push(json!({ "protocol": "web", "name": name, "host": url, "credential": "target" }));
            }
        };
        if has(4343) {
            web(4343, format!("https://{h}:4343/"), "Web-Oberfläche (4343)");
        }
        if has(443) {
            web(443, format!("https://{h}/"), "Web-Oberfläche");
        } else if has(80) {
            web(80, format!("http://{h}/"), "Web-Oberfläche (HTTP)");
        }
        for (port, proto) in [(22u64, "ssh"), (3389, "rdp"), (5900, "vnc")] {
            let p = match proto { "ssh" => Protocol::Ssh, "rdp" => Protocol::Rdp, _ => Protocol::Vnc };
            if has(port) && main != (p, port) {
                out.push(json!({ "protocol": proto, "name": "", "host": "", "credential": "target" }));
            }
        }
        Ok(json!({ "ways": out }))
    }

    /// Schalter „Inventar abfragen“ in der Übersicht: legt die Einstellung am Ziel selbst fest und speichert.
    pub fn overview_inventory_switch(&mut self, c: &Value) -> Result<Value, String> {
        let Subject::Target(t) = self.ov_subject(c)? else { return Err("Gäste haben keine Inventar-Einstellung".into()) };
        let on = c.get("on").and_then(Value::as_bool).ok_or("Wert fehlt")?;
        self.set_target_inventory(t.id, Some(on))?;
        self.overview_json(c)
    }

    pub fn overview_notes(&mut self, c: &Value) -> Result<Value, String> {
        let Subject::Target(t) = self.ov_subject(c)? else { return Err("Gäste haben keine Notizen".into()) };
        let text = c.get("notes").and_then(Value::as_str).unwrap_or_default().to_string();
        self.set_target_notes(t.id, text)?;
        Ok(json!({ "ok": true }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_queries_link_the_matching_guide() {
        let h = help_links(true, true);
        assert_eq!((h[0]["url"].as_str(), h[1]["url"].as_str()), (Some(HELP_WINRM), Some(HELP_OPENSSH)));
        assert_eq!(help_links(false, true)[0]["url"], HELP_OPENSSH);
        assert!(help_links(false, false).as_array().unwrap().is_empty(), "Linux über SSH: keine Windows-Anleitung");
        assert!(service_missing("Verbindung abgelehnt (os error 111)") && service_missing("WinRM-Client nicht verfügbar"));
        assert!(!service_missing("Host-Schlüssel unbekannt – bitte zuerst einmal direkt verbinden") && !service_missing("Authentication failed"));
    }

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
        while !app.overview_output(&json!({ "id": id.to_string() }))["done"].as_bool().unwrap_or(false) && start.elapsed() < Duration::from_secs(15) {
            std::thread::sleep(Duration::from_millis(100));
        }
        let o = app.overview_output(&json!({ "id": id.to_string() }));
        assert_eq!(o["kind"], "ping");
        assert!(o["text"].as_str().unwrap().contains("127.0.0.1"), "{o}");
        assert!(app.overview_run(&json!({ "id": id.to_string(), "kind": "x" })).is_err());
        let v = app.overview_json(&json!({ "id": id.to_string() })).unwrap();
        assert_eq!((v["name"].as_str(), v["sub"].as_str(), v["inventory"].as_array().map(Vec::len)), (Some("lokal"), Some("SSH · 127.0.0.1:22"), Some(inventory::FIELDS.len())));
        assert_eq!(v["path"], "", "Ziel direkt unter der Wurzel");
        // Gast eines Ziels, das kein Hypervisor ist: Übersicht ja, Abfrage beim Hypervisor scheitert verständlich.
        let g = json!({ "hv": id.to_string(), "guest": "pve1/qemu/100", "name": "web-01", "address": "127.0.0.1" });
        let v = app.overview_json(&g).unwrap();
        assert_eq!((v["name"].as_str(), v["guest"].as_bool(), v["can_query"].as_bool()), (Some("web-01"), Some(true), Some(false)));
        assert!(app.overview_query(&g).is_err() && app.overview_notes(&g).is_err());
        app.overview_field(&json!({ "hv": id.to_string(), "guest": "pve1/qemu/100", "field": "name", "value": "web-01" })).unwrap();
        assert_eq!(app.overview.store.load_key(&format!("gast-{id}-pve1/qemu/100")).value("name"), Some("web-01"));
        let r = app.overview_field(&json!({ "id": id.to_string(), "field": "vcpu", "value": "4" })).unwrap();
        assert_eq!(r["inventory"][3]["value"], "4");
        assert_eq!(r["inventory"][3]["manual"], true);
    }
}
