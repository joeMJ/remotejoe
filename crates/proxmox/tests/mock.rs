//! Proxmox-Attrappe: REST über einfaches HTTP, Konsole als RFB über Websocket.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use base64::Engine;
use rj_core::{ClipboardDirection, Secret};
use rj_proxmox::net::{read_frame, sha1_ws, write_frame};
use rj_proxmox::{Action, Auth, CertCheck, Client, GuestKind, ProxmoxError};
use rj_vnc::client::{vnc_auth_response, VncClient, VncParams};
use serde_json::json;

const PASSWORD: &str = "Prüf-ä-Kennwort&=+";
const TICKET: &str = "PVE:root@pam:4EEC61E2::Attrappe";
const CSRF: &str = "4EEC61E2:Attrappe-CSRF";
const TOKEN: &str = "root@pam!rj=0b0b0b0b-1111-2222-3333-444444444444";
const VNCTICKET: &str = "PVEVNC:Attr+appe/=";

#[derive(Default)]
struct Log {
    /// Offene Konsolen-Websockets (für die Prüfung auf Reste).
    open_ws: usize,
    actions: Vec<String>,
    logins: usize,
    expire_next: bool,
}

struct Req {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Req {
    fn header(&self, n: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.as_str())
    }
}

fn read_req(r: &mut BufReader<TcpStream>) -> Option<Req> {
    let mut line = String::new();
    r.read_line(&mut line).ok()?;
    let mut it = line.split_whitespace();
    let method = it.next()?.to_string();
    let path = it.next()?.to_string();
    let mut headers = Vec::new();
    loop {
        let mut l = String::new();
        r.read_line(&mut l).ok()?;
        let l = l.trim_end();
        if l.is_empty() {
            break;
        }
        let (k, v) = l.split_once(':')?;
        headers.push((k.trim().to_string(), v.trim().to_string()));
    }
    let n = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    let mut body = vec![0; n];
    r.read_exact(&mut body).ok()?;
    Some(Req { method, path, headers, body })
}

fn respond(s: &mut TcpStream, status: u16, body: serde_json::Value) {
    let b = body.to_string();
    // Gestückelte Übertragung wie bei pveproxy.
    let mut out = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n").into_bytes();
    for c in b.as_bytes().chunks(7) {
        out.extend_from_slice(format!("{:x}\r\n", c.len()).as_bytes());
        out.extend_from_slice(c);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"0\r\n\r\n");
    let _ = s.write_all(&out);
}

fn form_value<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    body.split('&').find_map(|kv| kv.strip_prefix(&format!("{key}=")))
}

fn handle(mut s: TcpStream, log: Arc<Mutex<Log>>) {
    let mut r = BufReader::new(s.try_clone().unwrap());
    let Some(req) = read_req(&mut r) else { return };
    if req.path == "/api2/json/access/ticket" {
        let body = String::from_utf8(req.body.clone()).unwrap();
        let ok = form_value(&body, "username") == Some("root%40pam")
            && form_value(&body, "password") == Some(&rj_proxmox::api::form_encode(PASSWORD.as_bytes())[..]);
        if ok {
            log.lock().unwrap().logins += 1;
            respond(&mut s, 200, json!({"data": {"ticket": TICKET, "CSRFPreventionToken": CSRF, "username": "root@pam"}}));
        } else {
            respond(&mut s, 401, json!({"data": null}));
        }
        return;
    }
    let cookie = req.header("cookie") == Some(&format!("PVEAuthCookie={TICKET}")[..]);
    let token = req.header("authorization") == Some(&format!("PVEAPIToken={TOKEN}")[..]);
    if cookie && std::mem::take(&mut log.lock().unwrap().expire_next) {
        respond(&mut s, 401, json!({"data": null}));
        return;
    }
    if !cookie && !token {
        respond(&mut s, 401, json!({"data": null}));
        return;
    }
    if req.method == "POST" && cookie && req.header("csrfpreventiontoken") != Some(CSRF) {
        respond(&mut s, 401, json!({"message": "CSRF fehlt"}));
        return;
    }
    let p = req.path.as_str();
    if p == "/api2/json/cluster/resources?type=vm" {
        respond(
            &mut s,
            200,
            json!({"data": [
                {"id": "lxc/200", "type": "lxc", "vmid": 200, "name": "behälter-1", "node": "pve1", "status": "stopped", "maxmem": 536870912},
                {"id": "qemu/100", "type": "qemu", "vmid": 100, "name": "web-01", "node": "pve1", "status": "running", "cpu": 0.25, "mem": 1073741824u64, "maxmem": 2147483648u64},
                {"id": "storage/pve1/local", "type": "storage", "node": "pve1"},
                {"id": "qemu/9000", "type": "qemu", "vmid": 9000, "name": "vorlage", "node": "pve2", "status": "stopped", "template": 1}
            ]}),
        );
    } else if let Some(rest) = p.strip_prefix("/api2/json/nodes/pve1/") {
        if let Some((guest, action)) = rest.split_once("/status/") {
            log.lock().unwrap().actions.push(format!("{guest}:{action}"));
            respond(&mut s, 200, json!({"data": format!("UPID:pve1:0001:{action}:")}));
        } else if rest == "qemu/100/vncproxy" && req.method == "POST" {
            assert_eq!(req.body, b"websocket=1");
            respond(&mut s, 200, json!({"data": {"port": "5900", "ticket": VNCTICKET, "user": "root@pam"}}));
        } else if let Some(q) = rest.strip_prefix("qemu/100/vncwebsocket?") {
            let want = format!("port=5900&vncticket={}", rj_proxmox::api::form_encode(VNCTICKET.as_bytes()));
            assert_eq!(q, want);
            let key = req.header("sec-websocket-key").unwrap().to_string();
            let accept = base64::engine::general_purpose::STANDARD.encode(sha1_ws(&key));
            let _ = write!(s, "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\nSec-WebSocket-Protocol: binary\r\n\r\n");
            log.lock().unwrap().open_ws += 1;
            rfb_over_ws(r, s);
            log.lock().unwrap().open_ws -= 1;
        } else {
            respond(&mut s, 404, json!({"message": "unbekannt"}));
        }
    } else {
        respond(&mut s, 404, json!({"message": "unbekannt"}));
    }
}

/// Liest Websocket-Rahmen als Bytestrom.
struct WsIn {
    r: BufReader<TcpStream>,
    buf: Vec<u8>,
    pos: usize,
}

impl Read for WsIn {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.pos >= self.buf.len() {
            let (op, p) = read_frame(&mut self.r)?;
            if op == 8 {
                return Ok(0);
            }
            self.buf = p;
            self.pos = 0;
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

fn rfb_over_ws(r: BufReader<TcpStream>, mut s: TcpStream) {
    let mut i = WsIn { r, buf: Vec::new(), pos: 0 };
    let send = |s: &mut TcpStream, b: &[u8]| write_frame(s, 2, b, false).unwrap();
    send(&mut s, b"RFB 003.008\n");
    let mut v = [0u8; 12];
    i.read_exact(&mut v).unwrap();
    send(&mut s, &[1, 2]);
    let mut choice = [0u8; 1];
    i.read_exact(&mut choice).unwrap();
    assert_eq!(choice[0], 2);
    let challenge = [7u8; 16];
    send(&mut s, &challenge);
    let mut resp = [0u8; 16];
    i.read_exact(&mut resp).unwrap();
    if resp != vnc_auth_response(VNCTICKET.as_bytes(), &challenge) {
        send(&mut s, &[0, 0, 0, 1]);
        return;
    }
    send(&mut s, &[0, 0, 0, 0]);
    let mut shared = [0u8; 1];
    i.read_exact(&mut shared).unwrap();
    let mut init = vec![0, 4, 0, 2, 32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0];
    init.extend_from_slice(&5u32.to_be_bytes());
    init.extend_from_slice(b"vm100");
    send(&mut s, &init);
    let mut answered = false;
    loop {
        let mut t = [0u8; 1];
        if i.read_exact(&mut t).is_err() {
            return;
        }
        let skip = match t[0] {
            0 => 19,
            2 => {
                let mut h = [0u8; 3];
                i.read_exact(&mut h).unwrap();
                4 * u16::from_be_bytes([h[1], h[2]]) as usize
            }
            3 => 9,
            4 => 7,
            5 => 5,
            6 => {
                let mut h = [0u8; 7];
                i.read_exact(&mut h).unwrap();
                u32::from_be_bytes([h[3], h[4], h[5], h[6]]) as usize
            }
            _ => return,
        };
        let mut b = vec![0u8; skip];
        if i.read_exact(&mut b).is_err() {
            return;
        }
        if t[0] == 3 && !answered {
            answered = true;
            let mut m = vec![0u8, 0, 0, 1, 0, 0, 0, 0, 0, 4, 0, 2, 0, 0, 0, 0];
            for _ in 0..8 {
                m.extend_from_slice(&[0x10, 0x20, 0x30, 0]);
            }
            send(&mut s, &m);
        }
    }
}

fn server() -> (String, Arc<Mutex<Log>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    let log = Arc::new(Mutex::new(Log::default()));
    let log2 = log.clone();
    thread::spawn(move || {
        for s in l.incoming().flatten() {
            let log = log2.clone();
            thread::spawn(move || handle(s, log));
        }
    });
    (url, log)
}

fn no_tls() -> CertCheck {
    Arc::new(|_| false)
}

fn password_client(url: &str, pw: &str) -> Client {
    let auth = Auth::Password { username: "root".into(), realm: "pam".into(), password: Secret::from_str_exact(pw) };
    Client::new(url, auth, no_tls(), Duration::from_secs(15)).unwrap()
}

#[test]
fn ticket_login_lists_guests_and_runs_actions() {
    let (url, log) = server();
    let c = password_client(&url, PASSWORD);
    let guests = c.guests().unwrap();
    assert_eq!(guests.len(), 3, "Speicher wird ausgelassen");
    assert_eq!((guests[0].vmid, guests[0].kind, guests[0].name.as_str()), (100, GuestKind::Qemu, "web-01"));
    assert_eq!((guests[1].vmid, guests[1].kind, guests[1].name.as_str()), (200, GuestKind::Lxc, "behälter-1"));
    assert!(guests[2].template);
    assert_eq!(guests[0].status, "running");
    assert_eq!(guests[0].maxmem, 2147483648);

    assert!(c.action(&guests[1], Action::Start).unwrap().starts_with("UPID:"));
    c.action(&guests[0], Action::Reboot).unwrap();
    // Abgelaufenes Ticket: genau eine neue Anmeldung, Aufruf gelingt.
    log.lock().unwrap().expire_next = true;
    c.action(&guests[0], Action::Shutdown).unwrap();
    c.action(&guests[0], Action::Stop).unwrap();
    let l = log.lock().unwrap();
    assert_eq!(l.actions, ["lxc/200:start", "qemu/100:reboot", "qemu/100:shutdown", "qemu/100:stop"]);
    assert_eq!(l.logins, 2);
}

#[test]
fn wrong_password_is_rejected() {
    let (url, _) = server();
    // Gleiche Zeichen, aber zerlegt (NFD): darf nicht passen.
    let c = password_client(&url, &PASSWORD.replace('ä', "a\u{308}"));
    assert!(matches!(c.guests(), Err(ProxmoxError::Unauthorized)));
}

#[test]
fn api_token_works_without_csrf() {
    let (url, log) = server();
    let auth = Auth::Token { id: "root@pam!rj".into(), secret: Secret::from_str_exact("0b0b0b0b-1111-2222-3333-444444444444") };
    let c = Client::new(&url, auth, no_tls(), Duration::from_secs(15)).unwrap();
    let g = c.guests().unwrap();
    c.action(&g[0], Action::Start).unwrap();
    assert_eq!(log.lock().unwrap().logins, 0);
}

#[test]
fn console_bridge_carries_rfb() {
    let (url, _) = server();
    let c = password_client(&url, PASSWORD);
    let g = c.guests().unwrap();
    let bridge = c.console(&g[0]).unwrap();
    let p = VncParams {
        host: "127.0.0.1".into(),
        port: bridge.port,
        username: None,
        password: Some(bridge.password.clone()),
        shared: true,
        view_only: false,
        clipboard: ClipboardDirection::Off,
        clipboard_limit: 1 << 20,
        prefer_tls: false,
        known_certs: None,
        connect_timeout: Duration::from_secs(15),
        extended_clipboard: false,
    };
    let vnc = VncClient::connect(&p, Arc::new(|_| false)).unwrap();
    assert_eq!(vnc.name, "vm100");
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let px = vnc.with_frame(|f| (f.width, f.height, f.data[..4].to_vec()));
        if px.2[..3] == [0x10, 0x20, 0x30] {
            assert_eq!((px.0, px.1), (4, 2));
            break;
        }
        assert!(std::time::Instant::now() < deadline, "kein Bild über die Brücke: {px:?}");
        thread::sleep(Duration::from_millis(50));
    }
}

fn vnc_params(port: u16, password: Secret) -> VncParams {
    VncParams {
        host: "127.0.0.1".into(),
        port,
        username: None,
        password: Some(password),
        shared: true,
        view_only: false,
        clipboard: ClipboardDirection::Off,
        clipboard_limit: 1 << 20,
        prefer_tls: false,
        known_certs: None,
        connect_timeout: Duration::from_secs(15),
        extended_clipboard: false,
    }
}

/// „Neu verbinden“ einer Konsole: alte Brücke verwerfen (schließt Websocket
/// und Client-Verbindung sofort), Konsole neu öffnen, neu verbinden.
#[test]
fn reconnect_reopens_console_without_leftovers() {
    let (url, log) = server();
    let c = password_client(&url, PASSWORD);
    let g = c.guests().unwrap();
    let first = c.console(&g[0]).unwrap();
    let vnc1 = VncClient::connect(&vnc_params(first.port, first.password.clone()), Arc::new(|_| false)).unwrap();
    assert_eq!(vnc1.name, "vm100");
    assert_eq!(log.lock().unwrap().open_ws, 1);
    let (port1, done1) = (first.port, first.finished_flag());
    drop(first);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !(done1.load(std::sync::atomic::Ordering::SeqCst) && log.lock().unwrap().open_ws == 0) {
        assert!(std::time::Instant::now() < deadline, "Reste der alten Brücke: Faden fertig {}, offene Websockets {}", done1.load(std::sync::atomic::Ordering::SeqCst), log.lock().unwrap().open_ws);
        thread::sleep(Duration::from_millis(20));
    }
    assert!(TcpStream::connect(("127.0.0.1", port1)).is_err(), "alter Brückenport nimmt noch Verbindungen an");
    let second = c.console(&g[0]).unwrap();
    let vnc2 = VncClient::connect(&vnc_params(second.port, second.password.clone()), Arc::new(|_| false)).unwrap();
    assert_eq!(vnc2.name, "vm100");
    assert_eq!(log.lock().unwrap().open_ws, 1, "genau eine Konsole offen");
}
