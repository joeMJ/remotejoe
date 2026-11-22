//! REST-Aufrufe und Konsolenbrücke.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use rj_core::Secret;
use serde_json::Value;

use crate::net::{self, Base, CertCheck};

#[derive(Debug, thiserror::Error)]
pub enum ProxmoxError {
    #[error("Adresse ungültig: {0}")]
    Url(String),
    #[error("Netzwerk: {0}")]
    Io(#[from] io::Error),
    #[error("Anmeldung abgelehnt")]
    Unauthorized,
    #[error("Proxmox meldet HTTP {status}: {message}")]
    Http { status: u16, message: String },
    #[error("Antwort unverständlich: {0}")]
    Format(String),
}

/// Anmeldung: API-Token (`benutzer@realm!name` + Geheimnis) oder Kennwort (Ticket).
#[derive(Clone)]
pub enum Auth {
    Token { id: String, secret: Secret },
    Password { username: String, realm: String, password: Secret },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestKind {
    Qemu,
    Lxc,
}

impl GuestKind {
    fn path(self) -> &'static str {
        match self {
            GuestKind::Qemu => "qemu",
            GuestKind::Lxc => "lxc",
        }
    }
}

/// Eine VM oder ein Container aus `cluster/resources`.
#[derive(Debug, Clone, PartialEq)]
pub struct Guest {
    pub vmid: u32,
    pub name: String,
    pub node: String,
    pub kind: GuestKind,
    pub status: String,
    pub template: bool,
    /// CPU-Last 0..1 und Speicher in Bytes (wie geliefert).
    pub cpu: f64,
    pub mem: u64,
    pub maxmem: u64,
}

/// Inventar eines Gasts (Konfiguration und, wenn er läuft, Adressen).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuestInfo {
    pub name: Option<String>,
    pub vcpu: Option<u64>,
    pub mem_bytes: Option<u64>,
    pub macs: Vec<String>,
    pub ips: Vec<String>,
}

fn num(v: Option<&Value>) -> Option<u64> {
    match v? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn looks_like_mac(s: &str) -> bool {
    s.len() == 17 && s.split(':').count() == 6 && s.split(':').all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()))
}

/// `GET …/config`: VM (`cores`×`sockets` bzw. `vcpus`, `memory` in MiB, `netN` = `virtio=MAC,…`) oder
/// Container (`cores`, `memory`, `netN` = `name=eth0,hwaddr=MAC,ip=…`).
pub fn parse_guest_config(cfg: &Value, kind: GuestKind) -> GuestInfo {
    let mut info = GuestInfo::default();
    info.name = cfg.get(if kind == GuestKind::Lxc { "hostname" } else { "name" }).and_then(Value::as_str).map(str::to_string);
    info.vcpu = match kind {
        GuestKind::Qemu => num(cfg.get("vcpus")).or_else(|| Some(num(cfg.get("cores")).unwrap_or(1) * num(cfg.get("sockets")).unwrap_or(1))),
        GuestKind::Lxc => num(cfg.get("cores")),
    };
    info.mem_bytes = num(cfg.get("memory")).map(|m| m * 1024 * 1024);
    let mut nets: Vec<(&String, &Value)> = cfg.as_object().into_iter().flatten().filter(|(k, _)| k.starts_with("net") && k[3..].parse::<u32>().is_ok()).collect();
    nets.sort_by_key(|(k, _)| k[3..].parse::<u32>().unwrap_or(0));
    for (_, v) in nets {
        for part in v.as_str().unwrap_or_default().split(',') {
            let Some((k, val)) = part.split_once('=') else { continue };
            if looks_like_mac(val) && (k == "hwaddr" || kind == GuestKind::Qemu) {
                info.macs.push(val.to_ascii_lowercase());
            } else if k == "ip" && val.contains('.') {
                info.ips.push(val.split('/').next().unwrap_or(val).to_string());
            }
        }
    }
    info
}

fn usable_ip(ip: &str) -> bool {
    !(ip.is_empty() || ip.starts_with("127.") || ip == "::1" || ip.starts_with("fe80:"))
}

/// Gast-Agent: `{"result":[{"name":"eth0","ip-addresses":[{"ip-address":"…"}]}]}`.
pub fn parse_agent_interfaces(v: &Value) -> Vec<String> {
    let list = v.get("result").unwrap_or(v);
    list.as_array()
        .into_iter()
        .flatten()
        .filter(|i| i.get("name").and_then(Value::as_str) != Some("lo"))
        .flat_map(|i| i.get("ip-addresses").and_then(Value::as_array).cloned().unwrap_or_default())
        .filter_map(|a| a.get("ip-address").and_then(Value::as_str).map(str::to_string))
        .filter(|ip| usable_ip(ip))
        .collect()
}

/// Container: `[{"name":"eth0","inet":"192.0.2.5/24","inet6":"…"}]`.
pub fn parse_lxc_interfaces(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter(|i| i.get("name").and_then(Value::as_str) != Some("lo"))
        .flat_map(|i| ["inet", "inet6"].into_iter().filter_map(|k| i.get(k).and_then(Value::as_str).map(|a| a.split('/').next().unwrap_or(a).to_string())).collect::<Vec<_>>())
        .filter(|ip| usable_ip(ip))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Start,
    Stop,
    Shutdown,
    Reboot,
}

impl Action {
    fn path(self) -> &'static str {
        match self {
            Action::Start => "start",
            Action::Stop => "stop",
            Action::Shutdown => "shutdown",
            Action::Reboot => "reboot",
        }
    }
}

struct Ticket {
    cookie: String,
    csrf: String,
}

/// Prozentkodierung für Formularfelder (`application/x-www-form-urlencoded`).
/// Die Bytes werden unverändert kodiert, nichts wird normalisiert.
pub fn form_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 3);
    for &b in bytes {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

/// Proxmox-Client (eine HTTP-Verbindung je Aufruf).
pub struct Client {
    base: Base,
    check: CertCheck,
    auth: Auth,
    timeout: Duration,
    ticket: Mutex<Option<Ticket>>,
}

impl Client {
    pub fn new(url: &str, auth: Auth, check: CertCheck, timeout: Duration) -> Result<Client, ProxmoxError> {
        let base = Base::parse(url).ok_or_else(|| ProxmoxError::Url(url.to_string()))?;
        Ok(Client { base, check, auth, timeout, ticket: Mutex::new(None) })
    }

    fn login(&self) -> Result<(), ProxmoxError> {
        let Auth::Password { username, realm, password } = &self.auth else { return Ok(()) };
        let mut body = format!("username={}&password=", form_encode(format!("{username}@{realm}").as_bytes())).into_bytes();
        body.extend_from_slice(form_encode(password.as_bytes()).as_bytes());
        let headers = [("Content-Type", "application/x-www-form-urlencoded".to_string())];
        let r = net::request(&self.base, &self.check, "POST", "/api2/json/access/ticket", &headers, &body, self.timeout)?;
        body.iter_mut().for_each(|b| *b = 0);
        if r.status == 401 || r.status == 403 {
            return Err(ProxmoxError::Unauthorized);
        }
        let v = Self::data(r)?;
        let cookie = v.get("ticket").and_then(Value::as_str).ok_or_else(|| ProxmoxError::Format("kein Ticket".into()))?;
        let csrf = v.get("CSRFPreventionToken").and_then(Value::as_str).unwrap_or_default();
        *self.ticket.lock().expect("Sperre") = Some(Ticket { cookie: cookie.to_string(), csrf: csrf.to_string() });
        Ok(())
    }

    fn auth_headers(&self, write: bool) -> Result<Vec<(&'static str, String)>, ProxmoxError> {
        match &self.auth {
            Auth::Token { id, secret } => {
                let secret = secret.as_str().ok_or_else(|| ProxmoxError::Format("Token-Geheimnis ist kein UTF-8".into()))?;
                Ok(vec![("Authorization", format!("PVEAPIToken={id}={secret}"))])
            }
            Auth::Password { .. } => {
                if self.ticket.lock().expect("Sperre").is_none() {
                    self.login()?;
                }
                let g = self.ticket.lock().expect("Sperre");
                let t = g.as_ref().expect("angemeldet");
                let mut h = vec![("Cookie", format!("PVEAuthCookie={}", t.cookie))];
                if write {
                    h.push(("CSRFPreventionToken", t.csrf.clone()));
                }
                Ok(h)
            }
        }
    }

    fn data(r: net::Response) -> Result<Value, ProxmoxError> {
        if r.status == 401 {
            return Err(ProxmoxError::Unauthorized);
        }
        let text = String::from_utf8(r.body).map_err(|e| ProxmoxError::Format(format!("ungültiges UTF-8 bei Byte {}", e.utf8_error().valid_up_to())))?;
        if !(200..300).contains(&r.status) {
            let message = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_else(|| text.chars().take(200).collect());
            return Err(ProxmoxError::Http { status: r.status, message });
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| ProxmoxError::Format(e.to_string()))?;
        Ok(v.get("data").cloned().unwrap_or(Value::Null))
    }

    fn call(&self, method: &str, path: &str, form: &str) -> Result<Value, ProxmoxError> {
        let write = method != "GET";
        for attempt in 0..2 {
            let mut headers = self.auth_headers(write)?;
            if write {
                headers.push(("Content-Type", "application/x-www-form-urlencoded".to_string()));
            }
            let r = net::request(&self.base, &self.check, method, path, &headers, form.as_bytes(), self.timeout)?;
            // Abgelaufenes Ticket: einmal neu anmelden.
            if r.status == 401 && attempt == 0 && matches!(self.auth, Auth::Password { .. }) {
                *self.ticket.lock().expect("Sperre") = None;
                continue;
            }
            return Self::data(r);
        }
        Err(ProxmoxError::Unauthorized)
    }

    /// Alle VMs und Container des Clusters.
    pub fn guests(&self) -> Result<Vec<Guest>, ProxmoxError> {
        let v = self.call("GET", "/api2/json/cluster/resources?type=vm", "")?;
        let list = v.as_array().ok_or_else(|| ProxmoxError::Format("Liste erwartet".into()))?;
        let mut out = Vec::new();
        for g in list {
            let kind = match g.get("type").and_then(Value::as_str) {
                Some("qemu") => GuestKind::Qemu,
                Some("lxc") => GuestKind::Lxc,
                _ => continue,
            };
            let Some(vmid) = g.get("vmid").and_then(Value::as_u64) else { continue };
            out.push(Guest {
                vmid: vmid as u32,
                name: g.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                node: g.get("node").and_then(Value::as_str).unwrap_or_default().to_string(),
                kind,
                status: g.get("status").and_then(Value::as_str).unwrap_or_default().to_string(),
                template: g.get("template").and_then(Value::as_u64).unwrap_or(0) == 1,
                cpu: g.get("cpu").and_then(Value::as_f64).unwrap_or(0.0),
                mem: g.get("mem").and_then(Value::as_u64).unwrap_or(0),
                maxmem: g.get("maxmem").and_then(Value::as_u64).unwrap_or(0),
            });
        }
        out.sort_by(|a, b| a.node.cmp(&b.node).then(a.vmid.cmp(&b.vmid)));
        Ok(out)
    }

    /// Inventar eines Gasts aus seiner Konfiguration; bei laufenden Gästen zusätzlich die Adressen
    /// (VM: über den Gast-Agenten, falls installiert; Container: über die Schnittstellen).
    pub fn guest_info(&self, g: &Guest) -> Result<GuestInfo, ProxmoxError> {
        let path = Self::guest_path(g);
        let cfg = self.call("GET", &format!("{path}/config"), "")?;
        let mut info = parse_guest_config(&cfg, g.kind);
        if g.status == "running" {
            let ifs = match g.kind {
                GuestKind::Qemu => self.call("GET", &format!("{path}/agent/network-get-interfaces"), "").ok().map(|v| parse_agent_interfaces(&v)),
                GuestKind::Lxc => self.call("GET", &format!("{path}/interfaces"), "").ok().map(|v| parse_lxc_interfaces(&v)),
            };
            if let Some(ips) = ifs.filter(|i| !i.is_empty()) {
                info.ips = ips;
            }
        }
        Ok(info)
    }

    fn guest_path(g: &Guest) -> String {
        format!("/api2/json/nodes/{}/{}/{}", form_encode(g.node.as_bytes()), g.kind.path(), g.vmid)
    }

    /// Statusaktion; liefert die Aufgaben-Kennung (UPID).
    pub fn action(&self, g: &Guest, a: Action) -> Result<String, ProxmoxError> {
        let v = self.call("POST", &format!("{}/status/{}", Self::guest_path(g), a.path()), "")?;
        Ok(v.as_str().unwrap_or_default().to_string())
    }

    /// Öffnet die Konsole: holt ein VNC-Ticket und stellt eine lokale
    /// Brücke bereit, an die sich der eigene VNC-Client verbindet.
    /// Das VNC-Kennwort ist das gelieferte Ticket.
    pub fn console(&self, g: &Guest) -> Result<ConsoleBridge, ProxmoxError> {
        let v = self.call("POST", &format!("{}/vncproxy", Self::guest_path(g)), "websocket=1")?;
        let port = match v.get("port") {
            Some(Value::String(s)) => s.parse::<u16>().ok(),
            Some(Value::Number(n)) => n.as_u64().map(|n| n as u16),
            _ => None,
        }
        .ok_or_else(|| ProxmoxError::Format("kein Konsolen-Port".into()))?;
        let ticket = v.get("ticket").and_then(Value::as_str).ok_or_else(|| ProxmoxError::Format("kein Konsolen-Ticket".into()))?.to_string();
        let path = format!("{}/vncwebsocket?port={port}&vncticket={}", Self::guest_path(g), form_encode(ticket.as_bytes()));
        let headers = self.auth_headers(false)?;
        let ws = net::websocket(&self.base, &self.check, &path, &headers, self.timeout)?;
        ConsoleBridge::start(ws, Secret::from_str_exact(&ticket))
    }
}

/// Lokale TCP-Brücke zwischen VNC-Client und Websocket (eine Verbindung).
pub struct ConsoleBridge {
    pub port: u16,
    /// Kennwort für die VNC-Anmeldung.
    pub password: Secret,
    stop: Arc<AtomicBool>,
    /// Sockets der Brücke (Websocket zum Host, Verbindung des VNC-Clients),
    /// damit das Verwerfen der Brücke beide sofort schließt.
    sockets: Arc<Mutex<Vec<TcpStream>>>,
    done: Arc<AtomicBool>,
}

impl ConsoleBridge {
    fn start(ws: net::WebSocket, password: Secret) -> Result<ConsoleBridge, ProxmoxError> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let sockets = Arc::new(Mutex::new(ws.socket.as_ref().and_then(|s| s.try_clone().ok()).into_iter().collect::<Vec<_>>()));
        let (stop2, done2, sockets2) = (stop.clone(), done.clone(), sockets.clone());
        thread::spawn(move || {
            let accepted = listener.accept();
            drop(listener);
            if let Ok((sock, _)) = accepted {
                if !stop2.load(Ordering::SeqCst) {
                    let _ = sock.set_nodelay(true);
                    if let Ok(c) = sock.try_clone() {
                        sockets2.lock().unwrap_or_else(|e| e.into_inner()).push(c);
                    }
                    bridge(sock, ws, stop2);
                }
            }
            done2.store(true, Ordering::SeqCst);
        });
        Ok(ConsoleBridge { port, password, stop, sockets, done })
    }

    /// Ist die Brücke vollständig beendet (Faden fertig, Sockets geschlossen)?
    pub fn finished(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }

    /// Beobachter für das Ende der Brücke (bleibt nach dem Verwerfen gültig).
    pub fn finished_flag(&self) -> Arc<AtomicBool> {
        self.done.clone()
    }
}

impl Drop for ConsoleBridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Laufende Verbindungen schließen, damit keine Reste bleiben.
        for s in self.sockets.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
        // Wartenden accept() lösen.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

fn bridge(sock: TcpStream, ws: net::WebSocket, stop: Arc<AtomicBool>) {
    let net::WebSocket { mut reader, writer, .. } = ws;
    let Ok(mut sock_r) = sock.try_clone() else { return };
    let mut sock_w = sock;
    let w2 = writer.clone();
    // Kurze Lesefrist und Stopp-Marke: Unter Windows weckt `shutdown` von einem anderen Faden einen schon
    // blockierten `recv` nicht zuverlässig auf. Bleibt der VNC-Client verbunden (z. B. beim Neuverbinden), hinge
    // der Faden sonst samt Socket, bis der Client trennt.
    let _ = sock_r.set_read_timeout(Some(std::time::Duration::from_millis(200)));
    let stop_up = stop.clone();
    let up = thread::spawn(move || {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match sock_r.read(&mut buf) {
                Ok(0) => break,
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted) => {
                    if stop_up.load(Ordering::SeqCst) {
                        break;
                    }
                }
                Err(_) => break,
                Ok(n) => {
                    let mut w = w2.lock().expect("Sperre");
                    if net::write_frame(&mut **w, 2, &buf[..n], true).is_err() {
                        break;
                    }
                }
            }
        }
        let mut w = w2.lock().expect("Sperre");
        let _ = net::write_frame(&mut **w, 8, &[], true);
    });
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        match net::read_frame(&mut reader) {
            Ok((1 | 2 | 0, p)) => {
                if sock_w.write_all(&p).is_err() {
                    break;
                }
            }
            Ok((9, p)) => {
                let mut w = writer.lock().expect("Sperre");
                let _ = net::write_frame(&mut **w, 10, &p, true);
            }
            Ok((10, _)) => {}
            _ => break,
        }
    }
    let _ = sock_w.shutdown(std::net::Shutdown::Both);
    let _ = up.join();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_encoding_keeps_bytes() {
        assert_eq!(form_encode("a b&c=ß".as_bytes()), "a%20b%26c%3D%C3%9F");
        // Zerlegtes „ä“ bleibt zerlegt.
        assert_eq!(form_encode("a\u{308}".as_bytes()), "a%CC%88");
    }
}

#[cfg(test)]
mod guest_info_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn vm_and_container_configs_are_read() {
        let vm = json!({ "name": "web01", "cores": 2, "sockets": 2, "memory": "4096", "net0": "virtio=BC:24:11:AA:BB:CC,bridge=vmbr0,firewall=1", "net1": "e1000=bc:24:11:00:00:01,bridge=vmbr1" });
        let i = parse_guest_config(&vm, GuestKind::Qemu);
        assert_eq!((i.name.as_deref(), i.vcpu, i.mem_bytes), (Some("web01"), Some(4), Some(4096 * 1024 * 1024)));
        assert_eq!(i.macs, vec!["bc:24:11:aa:bb:cc", "bc:24:11:00:00:01"]);
        assert_eq!(parse_guest_config(&json!({ "vcpus": 3, "cores": 4 }), GuestKind::Qemu).vcpu, Some(3));
        assert_eq!(parse_guest_config(&json!({}), GuestKind::Qemu).vcpu, Some(1));
        let ct = json!({ "hostname": "dns1", "cores": 1, "memory": 512, "net0": "name=eth0,bridge=vmbr0,hwaddr=BC:24:11:0A:0B:0C,ip=192.0.2.53/24,type=veth" });
        let i = parse_guest_config(&ct, GuestKind::Lxc);
        assert_eq!((i.name.as_deref(), i.vcpu, i.mem_bytes), (Some("dns1"), Some(1), Some(512 * 1024 * 1024)));
        assert_eq!((i.macs, i.ips), (vec!["bc:24:11:0a:0b:0c".to_string()], vec!["192.0.2.53".to_string()]));
        assert_eq!(parse_guest_config(&json!({ "hostname": "x" }), GuestKind::Lxc).vcpu, None, "ohne Begrenzung keine Angabe");
    }

    #[test]
    fn addresses_from_agent_and_container_skip_loopback_and_link_local() {
        let agent = json!({ "result": [
            { "name": "lo", "ip-addresses": [{ "ip-address": "127.0.0.1" }] },
            { "name": "eth0", "ip-addresses": [{ "ip-address": "192.0.2.10" }, { "ip-address": "fe80::1" }, { "ip-address": "2001:db8::10" }] }
        ] });
        assert_eq!(parse_agent_interfaces(&agent), vec!["192.0.2.10", "2001:db8::10"]);
        let ct = json!([{ "name": "lo", "inet": "127.0.0.1/8" }, { "name": "eth0", "inet": "192.0.2.53/24", "inet6": "fe80::2/64" }]);
        assert_eq!(parse_lxc_interfaces(&ct), vec!["192.0.2.53"]);
    }
}
