//! Schlanker HTTP/1.1-Client für die Update-Funktion: nur `https` mit
//! Zertifikatsprüfung gegen die Zertifizierungsstellen des Systems (nicht
//! abschaltbar), `http` nur für `127.0.0.1`/`localhost` (Tests). Größen- und
//! Zeitlimit, Abbruch über eine Marke, Weiterleitungen nur innerhalb dieser
//! Regeln; beim Wechsel des Hosts entfällt die Anmeldung.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
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
}

impl<'a> Control<'a> {
    pub fn new(limits: Limits, cancel: Option<&'a AtomicBool>) -> Self {
        Control { limits, cancel, deadline: Instant::now() + limits.timeout }
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

fn connect(u: &Url, ctl: &Control) -> Result<Stream> {
    ctl.check()?;
    let addr = (u.host.as_str(), u.port).to_socket_addrs().map_err(net)?.next().ok_or_else(|| UpdateError::Net(format!("{} nicht auflösbar", u.host)))?;
    let left = ctl.deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(1));
    let sock = TcpStream::connect_timeout(&addr, left.min(Duration::from_secs(30))).map_err(net)?;
    // Kurze Lesefrist, damit Abbruch und Zeitlimit greifen.
    sock.set_read_timeout(Some(Duration::from_millis(250))).map_err(net)?;
    sock.set_write_timeout(Some(Duration::from_secs(30))).map_err(net)?;
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
        if out.len() as u64 > ctl.limits.max_bytes {
            return Err(UpdateError::TooLarge(ctl.limits.max_bytes));
        }
        n -= got as u64;
    }
    Ok(())
}

fn once(method: &str, u: &Url, headers: &[(String, String)], body: &[u8], ctl: &Control) -> Result<Response> {
    let mut s = connect(u, ctl)?;
    let host = if u.host.contains(':') { format!("[{}]", u.host) } else { u.host.clone() };
    let mut req = format!("{method} {} HTTP/1.1\r\nHost: {host}:{}\r\nConnection: close\r\nUser-Agent: remotejoe/{}\r\nContent-Length: {}\r\n", u.path, u.port, crate::CURRENT_VERSION, body.len());
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
    s.write_all(req.as_bytes()).and_then(|_| s.write_all(body)).and_then(|_| s.flush()).map_err(net)?;
    drop(req);
    let mut r = BufReader::new(s);
    let first = read_line(&mut r, ctl)?;
    let status: u16 = first.split_whitespace().nth(1).and_then(|x| x.parse().ok()).ok_or_else(|| UpdateError::Parse("keine HTTP-Antwort".into()))?;
    let mut hs = Vec::new();
    loop {
        let l = read_line(&mut r, ctl)?;
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            hs.push((k.trim().to_string(), v.trim().to_string()));
        }
        if hs.len() > 200 {
            return Err(UpdateError::Parse("zu viele Kopfzeilen".into()));
        }
    }
    let get = |n: &str| hs.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.clone());
    let mut out = Vec::new();
    if (300..400).contains(&status) || method == "HEAD" {
        return Ok(Response { status, headers: hs, body: out });
    }
    if get("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        loop {
            let size = read_line(&mut r, ctl)?;
            let n = u64::from_str_radix(size.split(';').next().unwrap_or("").trim(), 16).map_err(|_| UpdateError::Parse("Blockgröße".into()))?;
            if n == 0 {
                break;
            }
            if out.len() as u64 + n > ctl.limits.max_bytes {
                return Err(UpdateError::TooLarge(ctl.limits.max_bytes));
            }
            read_exact(&mut r, n, &mut out, ctl)?;
            read_line(&mut r, ctl)?;
        }
    } else if let Some(n) = get("content-length").and_then(|v| v.parse::<u64>().ok()) {
        if n > ctl.limits.max_bytes {
            return Err(UpdateError::TooLarge(ctl.limits.max_bytes));
        }
        read_exact(&mut r, n, &mut out, ctl)?;
    } else {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let got = read_retry(&mut r, &mut buf, ctl)?;
            if got == 0 {
                break;
            }
            out.extend_from_slice(&buf[..got]);
            if out.len() as u64 > ctl.limits.max_bytes {
                return Err(UpdateError::TooLarge(ctl.limits.max_bytes));
            }
        }
    }
    Ok(Response { status, headers: hs, body: out })
}

/// Anfrage mit bis zu fünf Weiterleitungen (nur bei GET).
pub fn request(method: &str, url: &str, headers: &[(String, String)], body: &[u8], ctl: &Control) -> Result<Response> {
    let mut u = Url::parse(url)?;
    let origin = u.clone();
    for _ in 0..6 {
        // Anmeldung nur an den ursprünglichen Host (Downloads leiten oft auf einen Speicherdienst um).
        let hs: Vec<(String, String)> = headers.iter().filter(|(k, _)| u.same_origin(&origin) || !k.eq_ignore_ascii_case("authorization")).cloned().collect();
        let r = once(method, &u, &hs, body, ctl)?;
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
pub fn get(url: &str, headers: &[(String, String)], what: &str, ctl: &Control) -> Result<Vec<u8>> {
    let r = request("GET", url, headers, &[], ctl)?;
    if r.status != 200 {
        return Err(UpdateError::Status { status: r.status, what: what.into() });
    }
    Ok(r.body)
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
}
