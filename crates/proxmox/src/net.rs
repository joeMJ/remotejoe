//! Schlankes HTTP/1.1 und Websocket (RFC 6455) über TCP oder TLS. TLS-Zertifikate
//! werden über ihren SHA-256-Fingerabdruck geprüft (Nachfrage beim ersten Mal).

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use sha2::{Digest, Sha256};

/// Entscheidung über ein Zertifikat: Fingerabdruck → annehmen?
pub type CertCheck = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// Zieladresse (`https://host:port` oder für Tests `http://…`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base {
    pub tls: bool,
    pub host: String,
    pub port: u16,
}

impl Base {
    pub fn parse(url: &str) -> Option<Base> {
        let (tls, rest) = if let Some(r) = url.strip_prefix("https://") {
            (true, r)
        } else {
            (false, url.strip_prefix("http://")?)
        };
        let hostport = rest.split('/').next()?;
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().ok()?),
            None => (hostport.to_string(), if tls { 8006 } else { 80 }),
        };
        Some(Base { tls, host, port })
    }
}

struct Verifier {
    provider: Arc<rustls::crypto::CryptoProvider>,
    check: CertCheck,
}

impl std::fmt::Debug for Verifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Verifier")
    }
}

pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der).iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
}

impl rustls::client::danger::ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        end: &rustls::pki_types::CertificateDer<'_>,
        _i: &[rustls::pki_types::CertificateDer<'_>],
        _n: &rustls::pki_types::ServerName<'_>,
        _o: &[u8],
        _t: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        if (self.check)(&fingerprint(end.as_ref())) {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("Zertifikat abgelehnt".into()))
        }
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &rustls::pki_types::CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.provider.signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &rustls::pki_types::CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.provider.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// Getrennte Lese- und Schreibseite einer Verbindung.
pub struct Conn {
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
}

struct Pipe {
    rx: Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
}

impl Read for Pipe {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.pos >= self.buf.len() {
            match self.rx.recv() {
                Ok(b) => {
                    self.buf = b;
                    self.pos = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

struct TlsWriter {
    conn: Arc<Mutex<rustls::ClientConnection>>,
    sock: TcpStream,
}

impl Write for TlsWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let mut c = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        c.writer().write_all(data)?;
        while c.wants_write() {
            c.write_tls(&mut self.sock)?;
        }
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.sock.flush()
    }
}

/// Verbindung aufbauen (bei TLS mit Fingerabdruckprüfung).
pub fn connect(base: &Base, check: &CertCheck, timeout: Duration) -> io::Result<Conn> {
    use std::net::ToSocketAddrs;
    let addr = (base.host.as_str(), base.port).to_socket_addrs()?.next().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Adresse nicht auflösbar"))?;
    let sock = TcpStream::connect_timeout(&addr, timeout)?;
    sock.set_nodelay(true).ok();
    if !base.tls {
        return Ok(Conn { reader: Box::new(sock.try_clone()?), writer: Box::new(sock) });
    }
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let verifier = Arc::new(Verifier { provider: provider.clone(), check: check.clone() });
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(io::Error::other)?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(base.host.clone()).map_err(io::Error::other)?;
    let mut conn = rustls::ClientConnection::new(Arc::new(config), name).map_err(io::Error::other)?;
    let mut s = sock;
    s.set_read_timeout(Some(timeout))?;
    while conn.is_handshaking() {
        conn.complete_io(&mut s)?;
    }
    s.set_read_timeout(None)?;
    let conn = Arc::new(Mutex::new(conn));
    let (tx, rx) = mpsc::channel();
    let mut rsock = s.try_clone()?;
    let rconn = conn.clone();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = match rsock.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let mut c = rconn.lock().unwrap_or_else(|e| e.into_inner());
            let mut slice = &buf[..n];
            while !slice.is_empty() {
                if c.read_tls(&mut slice).is_err() || c.process_new_packets().is_err() {
                    return;
                }
            }
            let mut plain = Vec::new();
            let _ = c.reader().read_to_end(&mut plain);
            drop(c);
            if !plain.is_empty() && tx.send(plain).is_err() {
                return;
            }
        }
    });
    Ok(Conn { reader: Box::new(Pipe { rx, buf: Vec::new(), pos: 0 }), writer: Box::new(TlsWriter { conn, sock: s }) })
}

/// HTTP-Antwort.
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

fn read_head(r: &mut BufReader<Box<dyn Read + Send>>) -> io::Result<(u16, Vec<(String, String)>)> {
    let mut line = String::new();
    r.read_line(&mut line)?;
    let status = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "keine HTTP-Antwort"))?;
    let mut headers = Vec::new();
    loop {
        line.clear();
        r.read_line(&mut line)?;
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    Ok((status, headers))
}

/// Eine Anfrage (Verbindung je Anfrage, `Connection: close`).
pub fn request(base: &Base, check: &CertCheck, method: &str, path: &str, headers: &[(&str, String)], body: &[u8], timeout: Duration) -> io::Result<Response> {
    let mut c = connect(base, check, timeout)?;
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\nContent-Length: {}\r\n", base.host, base.port, body.len());
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    c.writer.write_all(req.as_bytes())?;
    c.writer.write_all(body)?;
    c.writer.flush()?;
    let mut r = BufReader::new(c.reader);
    let (status, headers) = read_head(&mut r)?;
    let mut body = Vec::new();
    let chunked = headers.iter().any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.eq_ignore_ascii_case("chunked"));
    if chunked {
        loop {
            let mut size = String::new();
            r.read_line(&mut size)?;
            let n = usize::from_str_radix(size.trim().split(';').next().unwrap_or("0"), 16).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Blockgröße"))?;
            if n == 0 {
                break;
            }
            let mut chunk = vec![0; n];
            r.read_exact(&mut chunk)?;
            body.extend_from_slice(&chunk);
            let mut crlf = String::new();
            r.read_line(&mut crlf)?;
        }
    } else if let Some(n) = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse::<usize>().ok()) {
        body.resize(n, 0);
        r.read_exact(&mut body)?;
    } else {
        r.read_to_end(&mut body)?;
    }
    Ok(Response { status, headers, body })
}

/// Websocket-Verbindung (Client): sendet maskierte Binärrahmen.
pub struct WebSocket {
    pub reader: BufReader<Box<dyn Read + Send>>,
    pub writer: Arc<Mutex<Box<dyn Write + Send>>>,
}

pub fn websocket(base: &Base, check: &CertCheck, path: &str, headers: &[(&str, String)], timeout: Duration) -> io::Result<WebSocket> {
    let mut c = connect(base, check, timeout)?;
    let mut key = [0u8; 16];
    getrandom::fill(&mut key).map_err(|_| io::Error::other("Zufall"))?;
    let key = base64::engine::general_purpose::STANDARD.encode(key);
    let mut req = format!(
        "GET {path} HTTP/1.1\r\nHost: {}:{}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: binary\r\n",
        base.host, base.port
    );
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    c.writer.write_all(req.as_bytes())?;
    c.writer.flush()?;
    let mut r = BufReader::new(c.reader);
    let (status, hs) = read_head(&mut r)?;
    if status != 101 {
        return Err(io::Error::new(io::ErrorKind::ConnectionRefused, format!("Websocket abgelehnt (HTTP {status})")));
    }
    let accept = base64::engine::general_purpose::STANDARD.encode(sha1_ws(&key));
    if hs.iter().find(|(k, _)| k.eq_ignore_ascii_case("sec-websocket-accept")).map(|(_, v)| v.as_str()) != Some(accept.as_str()) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Websocket-Antwort ungültig"));
    }
    Ok(WebSocket { reader: r, writer: Arc::new(Mutex::new(c.writer)) })
}

/// SHA-1 für den Websocket-Handschlag (RFC 6455, Abschnitt 4.2.2).
pub fn sha1_ws(key: &str) -> [u8; 20] {
    let data = format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    sha1(data.as_bytes())
}

/// Kleine SHA-1-Umsetzung (nur für den Websocket-Handschlag, keine Sicherheitsfunktion).
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

/// Rahmen schreiben (Client: maskiert).
pub fn write_frame(w: &mut dyn Write, opcode: u8, payload: &[u8], mask: bool) -> io::Result<()> {
    let mut h = vec![0x80 | opcode];
    let m = if mask { 0x80 } else { 0 };
    match payload.len() {
        n if n < 126 => h.push(m | n as u8),
        n if n < 65536 => {
            h.push(m | 126);
            h.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            h.push(m | 127);
            h.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    if mask {
        let mut key = [0u8; 4];
        getrandom::fill(&mut key).map_err(|_| io::Error::other("Zufall"))?;
        h.extend_from_slice(&key);
        let mut p = payload.to_vec();
        for (i, b) in p.iter_mut().enumerate() {
            *b ^= key[i % 4];
        }
        h.extend_from_slice(&p);
    } else {
        h.extend_from_slice(payload);
    }
    w.write_all(&h)?;
    w.flush()
}

/// Rahmen lesen: (Opcode, Nutzlast). Maskierte Rahmen werden entmaskiert.
pub fn read_frame(r: &mut dyn Read) -> io::Result<(u8, Vec<u8>)> {
    let mut h = [0u8; 2];
    r.read_exact(&mut h)?;
    let opcode = h[0] & 0x0f;
    let masked = h[1] & 0x80 != 0;
    let len = match h[1] & 0x7f {
        126 => {
            let mut b = [0u8; 2];
            r.read_exact(&mut b)?;
            u16::from_be_bytes(b) as u64
        }
        127 => {
            let mut b = [0u8; 8];
            r.read_exact(&mut b)?;
            u64::from_be_bytes(b)
        }
        n => n as u64,
    };
    if len > 64 * 1024 * 1024 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Websocket-Rahmen zu groß"));
    }
    let mut key = [0u8; 4];
    if masked {
        r.read_exact(&mut key)?;
    }
    let mut p = vec![0u8; len as usize];
    r.read_exact(&mut p)?;
    if masked {
        for (i, b) in p.iter_mut().enumerate() {
            *b ^= key[i % 4];
        }
    }
    Ok((opcode, p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_accept_example_from_rfc() {
        // RFC 6455, Abschnitt 1.3.
        let a = base64::engine::general_purpose::STANDARD.encode(sha1_ws("dGhlIHNhbXBsZSBub25jZQ=="));
        assert_eq!(a, "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn frames_roundtrip() {
        for n in [0usize, 5, 200, 70000] {
            let data: Vec<u8> = (0..n).map(|i| i as u8).collect();
            let mut buf = Vec::new();
            write_frame(&mut buf, 2, &data, true).unwrap();
            let (op, p) = read_frame(&mut &buf[..]).unwrap();
            assert_eq!((op, p), (2, data));
        }
    }

    #[test]
    fn base_url() {
        assert_eq!(Base::parse("https://pve.example.test"), Some(Base { tls: true, host: "pve.example.test".into(), port: 8006 }));
        assert_eq!(Base::parse("http://127.0.0.1:9000/x"), Some(Base { tls: false, host: "127.0.0.1".into(), port: 9000 }));
        assert_eq!(Base::parse("ftp://x"), None);
    }
}
