//! Schlanker HTTP/1.1-Client für die Update-Funktion: nur `https` mit
//! Zertifikatsprüfung gegen die Zertifizierungsstellen des Systems (nicht
//! abschaltbar), `http` nur für `127.0.0.1`/`localhost` (Tests). Größen- und
//! Zeitlimit, Abbruch über eine Marke, Weiterleitungen nur innerhalb dieser
//! Regeln; beim Wechsel des Hosts entfällt die Anmeldung.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crate::{Limits, Result, UpdateError};

/// Geprüfte Adresse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    pub tls: bool,
    pub host: String,
    pub port: u16,
    /// Pfad mit Abfrage, beginnt mit `/`.
    pub path: String,
}

fn is_local(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

impl Url {
    pub fn parse(url: &str) -> Result<Url> {
        let (tls, rest) = if let Some(r) = url.strip_prefix("https://") {
            (true, r)
        } else if let Some(r) = url.strip_prefix("http://") {
            (false, r)
        } else {
            return Err(UpdateError::Insecure(url.into()));
        };
        let (authority, path) = match rest.find(['/', '?']) {
            Some(i) => (&rest[..i], rest[i..].to_string()),
            None => (rest, "/".to_string()),
        };
        let path = if path.starts_with('?') { format!("/{path}") } else { path };
        if authority.contains('@') {
            return Err(UpdateError::Url("Zugangsdaten in der Adresse sind nicht erlaubt".into()));
        }
        let (host, port) = if let Some(r) = authority.strip_prefix('[') {
            let (h, p) = r.split_once(']').ok_or_else(|| UpdateError::Url(url.into()))?;
            (h.to_string(), p.strip_prefix(':'))
        } else {
            match authority.rsplit_once(':') {
                Some((h, p)) => (h.to_string(), Some(p)),
                None => (authority.to_string(), None),
            }
        };
        if host.is_empty() || host.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(UpdateError::Url(url.into()));
        }
        let port = match port {
            Some(p) => p.parse().map_err(|_| UpdateError::Url(url.into()))?,
            None if tls => 443,
            None => 80,
        };
        if !tls && !is_local(&host) {
            return Err(UpdateError::Insecure(url.into()));
        }
        Ok(Url { tls, host, port, path })
    }

    /// Ziel einer Weiterleitung (absolut oder relativ zum Host).
    pub fn join(&self, location: &str) -> Result<Url> {
        if location.starts_with("https://") || location.starts_with("http://") {
            Url::parse(location)
        } else if location.starts_with('/') {
            Ok(Url { path: location.to_string(), ..self.clone() })
        } else {
            Err(UpdateError::Url(location.into()))
        }
    }

    fn same_origin(&self, o: &Url) -> bool {
        self.tls == o.tls && self.host.eq_ignore_ascii_case(&o.host) && self.port == o.port
    }
}

/// Antwort mit Kopfzeilen und Rumpf.
#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// Abbruch und Zeitlimit eines Abrufs.
pub struct Control<'a> {
    pub limits: Limits,
    pub cancel: Option<&'a AtomicBool>,
    pub deadline: Instant,
    /// Bisher gelesene Bytes des Inhalts (Fortschrittsanzeige); `None`: nicht gezählt.
    pub progress: Option<&'a AtomicU64>,
    /// Proxy-Einstellung nur für diesen Abruf (Tests); `None`: die mit [`crate::proxy::set_mode`] gesetzte.
    pub proxy: Option<crate::proxy::ProxyMode>,
}

impl<'a> Control<'a> {
    pub fn new(limits: Limits, cancel: Option<&'a AtomicBool>) -> Self {
        Control { limits, cancel, deadline: Instant::now() + limits.timeout, progress: None, proxy: None }
    }

    /// Proxy-Einstellung nur für diesen Abruf.
    pub fn with_proxy(mut self, mode: crate::proxy::ProxyMode) -> Self {
        self.proxy = Some(mode);
        self
    }

    /// Zählt die gelesenen Bytes des Inhalts mit (für eine Fortschrittsanzeige).
    pub fn with_progress(mut self, progress: &'a AtomicU64) -> Self {
        self.progress = Some(progress);
        self
    }

    fn note(&self, bytes: usize) {
        if let Some(p) = self.progress {
            p.store(bytes as u64, Ordering::Relaxed);
        }
    }

    fn check(&self) -> Result<()> {
        if self.cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(UpdateError::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return Err(UpdateError::Timeout);
        }
        Ok(())
    }
}

fn tls_config() -> Result<Arc<rustls::ClientConfig>> {
    static CFG: OnceLock<std::result::Result<Arc<rustls::ClientConfig>, String>> = OnceLock::new();
    CFG.get_or_init(|| {
        let mut roots = rustls::RootCertStore::empty();
        let found = rustls_native_certs::load_native_certs();
        for c in found.certs {
            let _ = roots.add(c);
        }
        if roots.is_empty() {
            return Err("keine Zertifizierungsstellen des Systems gefunden".into());
        }
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map(|b| Arc::new(b.with_root_certificates(roots).with_no_client_auth()))
            .map_err(|e| e.to_string())
    })
    .clone()
    .map_err(UpdateError::Net)
}

enum Stream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Read for Stream {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(s) => s.read(b),
            Stream::Tls(s) => s.read(b),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(s) => s.write(b),
            Stream::Tls(s) => s.write(b),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Plain(s) => s.flush(),
            Stream::Tls(s) => s.flush(),
        }
    }
}

fn net(e: io::Error) -> UpdateError {
    UpdateError::Net(e.to_string())
}

/// Text, wenn beim Verbindungsaufbau keine Adresse geantwortet hat.
const NO_ANSWER: &str = "keine Antwort";

/// Hat der Server nicht geantwortet (Verbindungsaufbau ohne Antwort oder Zeitlimit des Abrufs)?
pub fn no_answer(e: &UpdateError) -> bool {
    match e {
        UpdateError::Timeout => true,
        UpdateError::Net(m) => m.contains(NO_ANSWER) || m.contains("os error 10060") || m.contains("os error 110") || m.contains("timed out"),
        _ => false,
    }
}

/// Meldung für Nutzer: der Fehler, bei fehlender Antwort mit Hinweis auf Netz und Anbieter.
pub fn describe(e: &UpdateError) -> String {
    if no_answer(e) {
        format!("{e} – der Server hat nicht geantwortet. Das liegt meist am Netz oder am Internetanbieter (verworfene Verbindungen); bitte später erneut versuchen.")
    } else {
        e.to_string()
    }
}

/// Wiederholung des Verbindungsaufbaus: Frist je Versuch und Pausen vor den Wiederholungen.
struct Retry {
    attempt: Duration,
    pauses: &'static [Duration],
}

/// Bleibt ein Verbindungsaufbau 10 s ohne Antwort, bis zu dreimal wiederholen (Pausen 2, 5, 10 s). Unterwegs
/// verlorene Verbindungsversuche (z. B. Bremsen beim Anbieter) gehen so nicht gleich als Fehler durch.
const RETRY: Retry = Retry { attempt: Duration::from_secs(10), pauses: &[Duration::from_secs(2), Duration::from_secs(5), Duration::from_secs(10)] };

fn tcp(host: &str, port: u16, ctl: &Control) -> Result<TcpStream> {
    let addrs: Vec<SocketAddr> = (host, port).to_socket_addrs().map_err(net)?.collect();
    if addrs.is_empty() {
        return Err(UpdateError::Net(format!("{host} nicht auflösbar")));
    }
    let sock = connect_any(&addrs, ctl, &RETRY, &TcpStream::connect_timeout)?;
    // Kurze Lesefrist, damit Abbruch und Zeitlimit greifen.
    sock.set_read_timeout(Some(Duration::from_millis(250))).map_err(net)?;
    sock.set_write_timeout(Some(Duration::from_secs(30))).map_err(net)?;
    Ok(sock)
}

/// Alle Adressen der Reihe nach probieren. Kam von keiner eine Antwort (Zeitüberschreitung), nach einer Pause
/// erneut, so oft `retry` Pausen hat. Eine Ablehnung (Port zu, Netz unerreichbar) ist eine Antwort und wird nicht
/// wiederholt. Abbruch und Zeitlimit des Abrufs gelten auch während der Pausen.
fn connect_any(addrs: &[SocketAddr], ctl: &Control, retry: &Retry, dial: &dyn Fn(&SocketAddr, Duration) -> io::Result<TcpStream>) -> Result<TcpStream> {
    let mut round = 0;
    loop {
        let mut last = None;
        let mut timed_out = false;
        for addr in addrs {
            ctl.check()?;
            let left = ctl.deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(1));
            match dial(addr, left.min(retry.attempt)) {
                Ok(s) => return Ok(s),
                Err(e) => {
                    timed_out |= matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock);
                    last = Some(e);
                }
            }
        }
        let e = last.unwrap_or_else(|| io::Error::other("keine Adresse"));
        let Some(pause) = retry.pauses.get(round).filter(|_| timed_out) else {
            if timed_out {
                return Err(UpdateError::Net(format!("{NO_ANSWER} nach {} Versuch(en)", round + 1)));
            }
            return Err(net(e));
        };
        round += 1;
        let until = Instant::now() + *pause;
        while Instant::now() < until {
            ctl.check()?;
            std::thread::sleep(until.saturating_duration_since(Instant::now()).min(Duration::from_millis(100)));
        }
    }
}

/// Tunnel durch einen HTTP-Proxy (`CONNECT`). Die Antwort wird byteweise gelesen, damit nichts vom
/// anschließenden TLS-Austausch verloren geht.
fn tunnel(proxy: &(String, u16), u: &Url, ctl: &Control) -> Result<TcpStream> {
    let mut sock = tcp(&proxy.0, proxy.1, ctl).map_err(|e| {
        // Ohne erneutes „Netzwerk:“ (das setzt die Fehlerart selbst davor).
        let why = match e {
            UpdateError::Net(m) => m,
            other => other.to_string(),
        };
        UpdateError::Net(format!("Proxy {}:{} nicht erreichbar: {why}", proxy.0, proxy.1))
    })?;
    let target = if u.host.contains(':') { format!("[{}]:{}", u.host, u.port) } else { format!("{}:{}", u.host, u.port) };
    let req = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\nUser-Agent: remotejoe/{}\r\n\r\n", crate::CURRENT_VERSION);
    sock.write_all(req.as_bytes()).map_err(net)?;
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if read_retry(&mut sock, &mut b, ctl)? == 0 {
            return Err(UpdateError::Net("Proxy hat die Verbindung beendet".into()));
        }
        head.push(b[0]);
        if head.len() > 16 * 1024 {
            return Err(UpdateError::Parse("Antwort des Proxys zu lang".into()));
        }
    }
    let text = String::from_utf8_lossy(&head);
    let first = text.lines().next().unwrap_or("").trim().to_string();
    match first.split_whitespace().nth(1) {
        Some("200") => Ok(sock),
        Some("407") => Err(UpdateError::Net(format!("Proxy {}:{} verlangt eine Anmeldung; das wird nicht unterstützt", proxy.0, proxy.1))),
        _ => Err(UpdateError::Net(format!("Proxy {}:{} lehnt die Verbindung ab: {first}", proxy.0, proxy.1))),
    }
}

/// Proxy für dieses Ziel (`None`: direkt).
fn route(u: &Url, ctl: &Control) -> Option<(String, u16)> {
    let mode = ctl.proxy.clone().unwrap_or_else(crate::proxy::mode);
    let url = format!("{}://{}:{}{}", if u.tls { "https" } else { "http" }, u.host, u.port, u.path);
    crate::proxy::for_target(&mode, &url, &u.host)
}

fn connect(u: &Url, proxy: Option<&(String, u16)>, ctl: &Control) -> Result<Stream> {
    ctl.check()?;
    let sock = match proxy {
        Some(p) => tunnel(p, u, ctl)?,
        None => tcp(&u.host, u.port, ctl).map_err(|e| match e {
            UpdateError::Net(m) => UpdateError::Net(format!("{}:{}: {m}", u.host, u.port)),
            other => other,
        })?,
    };
    if !u.tls {
        return Ok(Stream::Plain(sock));
    }
    let name = rustls::pki_types::ServerName::try_from(u.host.clone()).map_err(|e| UpdateError::Url(e.to_string()))?;
    let conn = rustls::ClientConnection::new(tls_config()?, name).map_err(|e| UpdateError::Net(e.to_string()))?;
    Ok(Stream::Tls(Box::new(rustls::StreamOwned::new(conn, sock))))
}

/// Liest mit Wiederholung bei abgelaufener Lesefrist.
fn read_retry(r: &mut impl Read, buf: &mut [u8], ctl: &Control) -> Result<usize> {
    loop {
        ctl.check()?;
        match r.read(buf) {
            Ok(n) => return Ok(n),
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => continue,
            Err(e) => return Err(net(e)),
        }
    }
}

fn read_line(r: &mut BufReader<Stream>, ctl: &Control) -> Result<String> {
    let mut line = Vec::new();
    loop {
        ctl.check()?;
        match r.read_until(b'\n', &mut line) {
            Ok(_) => break,
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => continue,
            Err(e) => return Err(net(e)),
        }
    }
    if line.len() > 64 * 1024 {
        return Err(UpdateError::Parse("Kopfzeile zu lang".into()));
    }
    Ok(String::from_utf8_lossy(&line).trim_end().to_string())
}

fn read_exact(r: &mut BufReader<Stream>, mut n: u64, out: &mut Vec<u8>, ctl: &Control) -> Result<()> {
    let mut buf = vec![0u8; 64 * 1024];
    while n > 0 {
        let want = buf.len().min(n as usize);
        let got = read_retry(r, &mut buf[..want], ctl)?;
        if got == 0 {
            return Err(UpdateError::Net("Verbindung vorzeitig beendet".into()));
        }
        out.extend_from_slice(&buf[..got]);
        ctl.note(out.len());
        if out.len() as u64 > ctl.limits.max_bytes {
            return Err(UpdateError::TooLarge(ctl.limits.max_bytes));
        }
        n -= got as u64;
    }
    Ok(())
}

/// Längster Rumpf einer Weiterleitung, der zum Wiederverwenden der Verbindung noch gelesen wird.
const REDIRECT_BODY_MAX: u64 = 64 * 1024;

/// Offene Verbindung mit Ziel (Server und Proxy), für das sie gilt.
struct Conn {
    key: String,
    r: BufReader<Stream>,
}

/// Fehler eines Austauschs: `stale` heißt, die (wiederverwendete) Verbindung war schon zu, bevor eine Antwort kam.
struct Failed {
    stale: bool,
    err: UpdateError,
}

impl From<UpdateError> for Failed {
    fn from(err: UpdateError) -> Self {
        Failed { stale: false, err }
    }
}

/// HTTP-Client, der die Verbindung zum zuletzt benutzten Server offen hält und wiederverwendet (keep-alive).
/// Weiterleitungen auf denselben Server laufen über dieselbe Verbindung. Hat der Server eine wiederverwendete
/// Verbindung inzwischen geschlossen, wird einmal neu verbunden.
#[derive(Default)]
pub struct Client {
    conn: Option<Conn>,
    opened: u64,
}

fn request_head(method: &str, u: &Url, headers: &[(String, String)], body: &[u8]) -> Result<String> {
    let host = if u.host.contains(':') { format!("[{}]", u.host) } else { u.host.clone() };
    let mut req = format!("{method} {} HTTP/1.1\r\nHost: {host}:{}\r\nUser-Agent: remotejoe/{}\r\nContent-Length: {}\r\n", u.path, u.port, crate::CURRENT_VERSION, body.len());
    for (k, v) in headers {
        if k.contains(['\r', '\n']) || v.contains(['\r', '\n']) {
            return Err(UpdateError::Url("Kopfzeile mit Zeilenumbruch".into()));
        }
        req.push_str(k);
        req.push_str(": ");
        req.push_str(v);
        req.push_str("\r\n");
    }
    req.push_str("\r\n");
    Ok(req)
}

/// Rumpf mit Längenangabe oder in Blöcken lesen. `None`: der Rumpf endet erst mit dem Ende der Verbindung.
fn read_body(r: &mut BufReader<Stream>, hs: &[(String, String)], ctl: &Control) -> Result<Option<Vec<u8>>> {
    let get = |n: &str| hs.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.clone());
    let mut out = Vec::new();
    if get("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        loop {
            let size = read_line(r, ctl)?;
            let n = u64::from_str_radix(size.split(';').next().unwrap_or("").trim(), 16).map_err(|_| UpdateError::Parse("Blockgröße".into()))?;
            if n == 0 {
                break;
            }
            if out.len() as u64 + n > ctl.limits.max_bytes {
                return Err(UpdateError::TooLarge(ctl.limits.max_bytes));
            }
            read_exact(r, n, &mut out, ctl)?;
            read_line(r, ctl)?;
        }
        // Nachspann bis zur Leerzeile, damit die Verbindung sauber für die nächste Anfrage steht.
        for _ in 0..200 {
            if read_line(r, ctl)?.is_empty() {
                return Ok(Some(out));
            }
        }
        return Err(UpdateError::Parse("zu viele Kopfzeilen".into()));
    }
    if let Some(n) = get("content-length").and_then(|v| v.parse::<u64>().ok()) {
        if n > ctl.limits.max_bytes {
            return Err(UpdateError::TooLarge(ctl.limits.max_bytes));
        }
        read_exact(r, n, &mut out, ctl)?;
        return Ok(Some(out));
    }
    Ok(None)
}

/// Rumpf, der erst mit dem Ende der Verbindung endet.
fn read_to_end(r: &mut BufReader<Stream>, ctl: &Control) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let got = read_retry(r, &mut buf, ctl)?;
        if got == 0 {
            return Ok(out);
        }
        out.extend_from_slice(&buf[..got]);
        ctl.note(out.len());
        if out.len() as u64 > ctl.limits.max_bytes {
            return Err(UpdateError::TooLarge(ctl.limits.max_bytes));
        }
    }
}

/// Eine Anfrage über eine offene Verbindung. Liefert die Antwort und ob die Verbindung danach weiter benutzbar ist.
fn exchange(r: &mut BufReader<Stream>, method: &str, req: &str, body: &[u8], ctl: &Control) -> std::result::Result<(Response, bool), Failed> {
    let s = r.get_mut();
    if let Err(e) = s.write_all(req.as_bytes()).and_then(|_| s.write_all(body)).and_then(|_| s.flush()) {
        return Err(Failed { stale: true, err: net(e) });
    }
    let first = match read_line(r, ctl) {
        Ok(l) if l.is_empty() => return Err(Failed { stale: true, err: UpdateError::Net("Verbindung vom Server beendet".into()) }),
        Ok(l) => l,
        Err(e @ UpdateError::Net(_)) => return Err(Failed { stale: true, err: e }),
        Err(e) => return Err(e.into()),
    };
    let mut parts = first.split_whitespace();
    let version = parts.next().unwrap_or("");
    let status: u16 = parts.next().and_then(|x| x.parse().ok()).ok_or_else(|| UpdateError::Parse("keine HTTP-Antwort".into()))?;
    let mut hs = Vec::new();
    loop {
        let l = read_line(r, ctl)?;
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            hs.push((k.trim().to_string(), v.trim().to_string()));
        }
        if hs.len() > 200 {
            return Err(UpdateError::Parse("zu viele Kopfzeilen".into()).into());
        }
    }
    let close = !version.eq_ignore_ascii_case("HTTP/1.1")
        || hs.iter().any(|(k, v)| k.eq_ignore_ascii_case("connection") && v.split(',').any(|t| t.trim().eq_ignore_ascii_case("close")));
    // Ohne Rumpf: HEAD, 1xx, 204, 304.
    if method == "HEAD" || status < 200 || status == 204 || status == 304 {
        return Ok((Response { status, headers: hs, body: Vec::new() }, !close));
    }
    if (300..400).contains(&status) {
        // Weiterleitung: Rumpf nur lesen, wenn er klein ist und eine Länge hat – sonst Verbindung aufgeben.
        let len = hs.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse::<u64>().ok());
        let chunked = hs.iter().any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.eq_ignore_ascii_case("chunked"));
        let keep = !close && (chunked || len.is_some_and(|n| n <= REDIRECT_BODY_MAX));
        if keep {
            let small = Control { limits: Limits { max_bytes: REDIRECT_BODY_MAX, ..ctl.limits }, cancel: ctl.cancel, deadline: ctl.deadline, progress: None, proxy: None };
            read_body(r, &hs, &small)?;
        }
        return Ok((Response { status, headers: hs, body: Vec::new() }, keep));
    }
    match read_body(r, &hs, ctl)? {
        Some(out) => Ok((Response { status, headers: hs, body: out }, !close)),
        None => Ok((Response { status, headers: hs, body: read_to_end(r, ctl)? }, false)),
    }
}

impl Client {
    pub fn new() -> Self {
        Self::default()
    }

    /// Zahl der bisher geöffneten Verbindungen.
    pub fn connections(&self) -> u64 {
        self.opened
    }

    fn once(&mut self, method: &str, u: &Url, headers: &[(String, String)], body: &[u8], ctl: &Control) -> Result<Response> {
        let req = request_head(method, u, headers, body)?;
        let proxy = route(u, ctl);
        let key = format!("{}://{}:{} {:?}", if u.tls { "https" } else { "http" }, u.host.to_ascii_lowercase(), u.port, proxy);
        if let Some(mut c) = self.conn.take() {
            if c.key == key {
                ctl.check()?;
                match exchange(&mut c.r, method, &req, body, ctl) {
                    Ok((resp, keep)) => {
                        if keep {
                            self.conn = Some(c);
                        }
                        return Ok(resp);
                    }
                    // Der Server hat die Verbindung inzwischen geschlossen: einmal neu verbinden.
                    Err(Failed { stale: true, .. }) => {}
                    Err(f) => return Err(f.err),
                }
            }
        }
        let s = connect(u, proxy.as_ref(), ctl)?;
        self.opened += 1;
        let mut c = Conn { key, r: BufReader::new(s) };
        let (resp, keep) = exchange(&mut c.r, method, &req, body, ctl).map_err(|f| f.err)?;
        if keep {
            self.conn = Some(c);
        }
        Ok(resp)
    }

    /// Anfrage mit bis zu fünf Weiterleitungen (nur bei GET).
    pub fn request(&mut self, method: &str, url: &str, headers: &[(String, String)], body: &[u8], ctl: &Control) -> Result<Response> {
        let mut u = Url::parse(url)?;
        let origin = u.clone();
        for _ in 0..6 {
            // Anmeldung nur an den ursprünglichen Host (Downloads leiten oft auf einen Speicherdienst um).
            let hs: Vec<(String, String)> = headers.iter().filter(|(k, _)| u.same_origin(&origin) || !k.eq_ignore_ascii_case("authorization")).cloned().collect();
            let r = self.once(method, &u, &hs, body, ctl)?;
            if method == "GET" && matches!(r.status, 301 | 302 | 303 | 307 | 308) {
                let loc = r.header("location").ok_or_else(|| UpdateError::Parse("Weiterleitung ohne Ziel".into()))?;
                u = u.join(loc)?;
                continue;
            }
            return Ok(r);
        }
        Err(UpdateError::Net("zu viele Weiterleitungen".into()))
    }

    /// GET; Fehlerstatus wird zum Fehler (ohne Inhalt der Antwort).
    pub fn get(&mut self, url: &str, headers: &[(String, String)], what: &str, ctl: &Control) -> Result<Vec<u8>> {
        let r = self.request("GET", url, headers, &[], ctl)?;
        if r.status != 200 {
            return Err(UpdateError::Status { status: r.status, what: what.into() });
        }
        Ok(r.body)
    }
}

/// Anfrage mit bis zu fünf Weiterleitungen (nur bei GET); Weiterleitungen auf denselben Server über dieselbe Verbindung.
pub fn request(method: &str, url: &str, headers: &[(String, String)], body: &[u8], ctl: &Control) -> Result<Response> {
    Client::new().request(method, url, headers, body, ctl)
}

/// GET; Fehlerstatus wird zum Fehler (ohne Inhalt der Antwort).
pub fn get(url: &str, headers: &[(String, String)], what: &str, ctl: &Control) -> Result<Vec<u8>> {
    Client::new().get(url, headers, what, ctl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_except_local() {
        assert!(Url::parse("https://git.example.test/api").is_ok());
        assert_eq!(Url::parse("http://127.0.0.1:3000/x?y=1").unwrap().path, "/x?y=1");
        assert!(Url::parse("http://localhost/").is_ok());
        assert!(matches!(Url::parse("http://git.example.test/"), Err(UpdateError::Insecure(_))));
        assert!(matches!(Url::parse("ftp://git.example.test/"), Err(UpdateError::Insecure(_))));
        assert!(Url::parse("https://nutzer:geheim@git.example.test/").is_err());
        let u = Url::parse("https://git.example.test:8443").unwrap();
        assert_eq!((u.port, u.path.as_str()), (8443, "/"));
        assert!(u.join("http://dl.example.test/a").is_err(), "Weiterleitung auf http");
        assert_eq!(u.join("/b").unwrap().host, "git.example.test");
    }

    /// Testserver nach Art der Update-Quelle: Anmeldeseite, Anmeldung mit Weiterleitung, Ordnerliste, Datei über
    /// Weiterleitung, Antwort in Blöcken mit Nachspann. Zählt die angenommenen Verbindungen. `close_after`: Der Server
    /// schließt jede Verbindung nach so vielen Antworten, ohne es anzukündigen (wie ein abgelaufenes keep-alive).
    fn quelle(close_after: usize) -> (u16, Arc<std::sync::atomic::AtomicUsize>) {
        use std::net::TcpListener;
        use std::sync::atomic::AtomicUsize;
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let accepted = Arc::new(AtomicUsize::new(0));
        let acc = accepted.clone();
        std::thread::spawn(move || {
            for c in l.incoming() {
                let Ok(mut c) = c else { return };
                acc.fetch_add(1, Ordering::SeqCst);
                std::thread::spawn(move || {
                    let mut r = BufReader::new(c.try_clone().unwrap());
                    for _ in 0..close_after {
                        let mut first = String::new();
                        if r.read_line(&mut first).unwrap_or(0) == 0 {
                            return;
                        }
                        let mut len = 0usize;
                        loop {
                            let mut h = String::new();
                            r.read_line(&mut h).unwrap();
                            if h == "\r\n" {
                                break;
                            }
                            if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                                len = v.trim().parse().unwrap();
                            }
                        }
                        let mut body = vec![0u8; len];
                        r.read_exact(&mut body).unwrap();
                        let path = first.split_whitespace().nth(1).unwrap().to_string();
                        let resp: Vec<u8> = match path.as_str() {
                            "/d/abc/" if first.starts_with("POST") => b"HTTP/1.1 302 Found\r\nLocation: /d/abc/\r\nSet-Cookie: sessionid=s1\r\nContent-Length: 0\r\n\r\n".to_vec(),
                            "/d/abc/" => b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nform".to_vec(),
                            "/liste" => b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhallo\r\n5\r\n welt\r\n0\r\nX-Nachspann: 1\r\n\r\n".to_vec(),
                            "/datei" => b"HTTP/1.1 302 Found\r\nLocation: /speicher/datei\r\nContent-Length: 10\r\n\r\numgeleitet".to_vec(),
                            "/speicher/datei" => b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\ninhalt".to_vec(),
                            "/leer" => b"HTTP/1.1 204 No Content\r\n\r\n".to_vec(),
                            _ => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
                        };
                        if c.write_all(&resp).is_err() {
                            return;
                        }
                    }
                    // Ohne Ankündigung schließen.
                });
            }
        });
        (port, accepted)
    }

    fn ablauf(port: u16, client: &mut Client) {
        let limits = Limits { max_bytes: 1024 * 1024, timeout: Duration::from_secs(10) };
        let ctl = Control::new(limits, None).with_proxy(crate::proxy::ProxyMode::Off);
        let base = format!("http://127.0.0.1:{port}");
        assert_eq!(client.get(&format!("{base}/d/abc/"), &[], "Seite", &ctl).unwrap(), b"form");
        let r = client.request("POST", &format!("{base}/d/abc/"), &[], b"password=x", &ctl).unwrap();
        assert_eq!(r.status, 302);
        assert_eq!(client.get(&format!("{base}/liste"), &[], "Liste", &ctl).unwrap(), b"hallo welt");
        assert_eq!(client.request("GET", &format!("{base}/leer"), &[], &[], &ctl).unwrap().status, 204);
        assert_eq!(client.get(&format!("{base}/datei"), &[], "Datei", &ctl).unwrap(), b"inhalt");
        assert_eq!(client.get(&format!("{base}/liste"), &[], "Liste", &ctl).unwrap(), b"hallo welt");
    }

    /// Alle Anfragen eines Ablaufs (auch Weiterleitung und Blockantwort) über eine einzige Verbindung.
    #[test]
    fn keep_alive_uses_one_connection() {
        let (port, accepted) = quelle(usize::MAX);
        let mut c = Client::new();
        ablauf(port, &mut c);
        assert_eq!(c.connections(), 1);
        assert_eq!(accepted.load(Ordering::SeqCst), 1);
    }

    /// Schließt der Server die Verbindung nach jeder bzw. jeder zweiten Antwort, verbindet der Client neu und alle
    /// Antworten stimmen.
    #[test]
    fn keep_alive_reconnects_when_server_closes() {
        for n in [1, 2] {
            let (port, accepted) = quelle(n);
            let mut c = Client::new();
            ablauf(port, &mut c);
            assert_eq!(c.connections() as usize, accepted.load(Ordering::SeqCst), "nach {n} Antwort(en) geschlossen");
            assert!(c.connections() > 1);
        }
    }

    const SHORT: Retry = Retry { attempt: Duration::from_millis(300), pauses: &[Duration::from_millis(100), Duration::from_millis(100), Duration::from_millis(100)] };

    /// Adresse, die nie antwortet: wartet die Frist ab und meldet Zeitüberschreitung (wie ein unterwegs verlorener
    /// Verbindungsversuch). Zählt die Versuche; alle anderen Adressen werden echt gewählt.
    fn dialer(silent: SocketAddr, calls: &std::sync::atomic::AtomicUsize) -> impl Fn(&SocketAddr, Duration) -> io::Result<TcpStream> + '_ {
        move |a, t| {
            calls.fetch_add(1, Ordering::SeqCst);
            if *a == silent {
                std::thread::sleep(t);
                return Err(io::Error::from(io::ErrorKind::TimedOut));
            }
            TcpStream::connect_timeout(a, t)
        }
    }

    fn silent() -> SocketAddr {
        "192.0.2.1:9".parse().unwrap()
    }

    /// Antwortet keine Adresse, wird der Aufbau dreimal wiederholt (4 Versuche mit Pausen) und scheitert dann.
    #[test]
    fn connect_retries_when_nothing_answers() {
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let ctl = Control::new(Limits { max_bytes: 1024, timeout: Duration::from_secs(30) }, None);
        let t = Instant::now();
        let e = connect_any(&[silent()], &ctl, &SHORT, &dialer(silent(), &calls)).unwrap_err();
        assert_eq!(e.to_string(), "Netzwerk: keine Antwort nach 4 Versuch(en)");
        assert!(no_answer(&e) && describe(&e).contains("Internetanbieter"), "{}", describe(&e));
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        assert!(t.elapsed() >= Duration::from_millis(4 * 300 + 3 * 100), "{:?}", t.elapsed());
    }

    /// Hinweis auf Netz und Anbieter nur bei fehlender Antwort; eine Ablehnung zeigt Rechner und Port.
    #[test]
    fn messages_name_target_and_hint_only_without_answer() {
        assert!(describe(&UpdateError::Timeout).contains("Internetanbieter"));
        assert!(no_answer(&UpdateError::Net("Connection timed out (os error 10060)".into())));
        assert!(!no_answer(&UpdateError::Status { status: 404, what: "x".into() }));
        assert_eq!(describe(&UpdateError::Net("Connection refused".into())), "Netzwerk: Connection refused");
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let ctl = Control::new(Limits { max_bytes: 1024, timeout: Duration::from_secs(5) }, None).with_proxy(crate::proxy::ProxyMode::Off);
        let e = get(&format!("http://127.0.0.1:{port}/x"), &[], "Test", &ctl).unwrap_err().to_string();
        assert!(e.starts_with(&format!("Netzwerk: 127.0.0.1:{port}: ")), "{e}");
    }

    /// Erste Adresse antwortet nicht, die zweite schon: Verbindung über die zweite, ohne Wiederholungsrunde.
    #[test]
    fn connect_tries_all_addresses() {
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let ctl = Control::new(Limits { max_bytes: 1024, timeout: Duration::from_secs(30) }, None);
        let s = connect_any(&[silent(), l.local_addr().unwrap()], &ctl, &SHORT, &dialer(silent(), &calls)).unwrap();
        assert_eq!(s.peer_addr().unwrap(), l.local_addr().unwrap());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// Eine Ablehnung (Port zu) ist eine Antwort: kein erneuter Versuch. Windows meldet die Ablehnung erst nach rund
    /// 2 s (wiederholte Verbindungsversuche des Systems); die Frist je Versuch ist deshalb hier länger.
    #[test]
    fn connect_refused_is_not_retried() {
        const REFUSED: Retry = Retry { attempt: Duration::from_secs(5), pauses: &[Duration::from_millis(100), Duration::from_millis(100), Duration::from_millis(100)] };
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let ctl = Control::new(Limits { max_bytes: 1024, timeout: Duration::from_secs(30) }, None);
        assert!(connect_any(&[SocketAddr::from(([127, 0, 0, 1], port))], &ctl, &REFUSED, &dialer(silent(), &calls)).is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// Abbruch während der Wiederholungen greift sofort, auch in einer Pause; das Zeitlimit des Abrufs ebenso.
    #[test]
    fn connect_retry_respects_cancel_and_deadline() {
        const SLOW: Retry = Retry { attempt: Duration::from_millis(200), pauses: &[Duration::from_secs(10)] };
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let flag = Arc::new(AtomicBool::new(false));
        let f2 = flag.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            f2.store(true, Ordering::Relaxed);
        });
        let ctl = Control::new(Limits { max_bytes: 1024, timeout: Duration::from_secs(30) }, Some(&flag));
        let t = Instant::now();
        assert!(matches!(connect_any(&[silent()], &ctl, &SLOW, &dialer(silent(), &calls)), Err(UpdateError::Cancelled)));
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
        let ctl = Control::new(Limits { max_bytes: 1024, timeout: Duration::from_millis(700) }, None);
        let t = Instant::now();
        assert!(matches!(connect_any(&[silent()], &ctl, &SLOW, &dialer(silent(), &calls)), Err(UpdateError::Timeout)));
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    }

    /// Abruf über einen Proxy mit CONNECT: Der Proxy bekommt nur Rechner und Port, die Anfrage geht durch den Tunnel.
    #[test]
    fn request_through_connect_proxy() {
        use std::net::TcpListener;
        use std::sync::Mutex;
        let origin = TcpListener::bind("127.0.0.1:0").unwrap();
        let oport = origin.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut s, _) = origin.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut l = String::new();
            while r.read_line(&mut l).unwrap() > 0 && !l.ends_with("\r\n\r\n") {}
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhallo").unwrap();
        });
        let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
        let pport = proxy.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(String::new()));
        let seen2 = seen.clone();
        std::thread::spawn(move || {
            let (mut c, _) = proxy.accept().unwrap();
            let mut r = BufReader::new(c.try_clone().unwrap());
            let mut first = String::new();
            r.read_line(&mut first).unwrap();
            *seen2.lock().unwrap() = first.trim().to_string();
            let mut l = String::new();
            while r.read_line(&mut l).unwrap() > 0 && l != "\r\n" {
                l.clear();
            }
            let target = first.split_whitespace().nth(1).unwrap().to_string();
            let up = TcpStream::connect(target).unwrap();
            c.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n").unwrap();
            let (mut a, mut b) = (c.try_clone().unwrap(), up.try_clone().unwrap());
            std::thread::spawn(move || {
                let _ = io::copy(&mut b, &mut a);
            });
            let mut up = up;
            let _ = io::copy(&mut r, &mut up);
        });
        let limits = Limits { max_bytes: 1024, timeout: Duration::from_secs(10) };
        let ctl = Control::new(limits, None).with_proxy(crate::proxy::ProxyMode::Manual(format!("127.0.0.1:{pport}")));
        let body = get(&format!("http://127.0.0.1:{oport}/x"), &[], "Test", &ctl).unwrap();
        assert_eq!(body, b"hallo");
        assert_eq!(*seen.lock().unwrap(), format!("CONNECT 127.0.0.1:{oport} HTTP/1.1"));
        // Proxy verlangt Anmeldung: verständliche Meldung.
        let p407 = TcpListener::bind("127.0.0.1:0").unwrap();
        let port407 = p407.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut c, _) = p407.accept().unwrap();
            let mut r = BufReader::new(c.try_clone().unwrap());
            let mut l = String::new();
            while r.read_line(&mut l).unwrap() > 0 && !l.ends_with("\r\n\r\n") {}
            c.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n").unwrap();
        });
        let ctl = Control::new(limits, None).with_proxy(crate::proxy::ProxyMode::Manual(format!("127.0.0.1:{port407}")));
        let e = get("http://127.0.0.1:9/x", &[], "Test", &ctl).unwrap_err().to_string();
        assert!(e.contains("Anmeldung"), "{e}");
    }
}
