//! RFB-Client (RFB 3.3/3.7/3.8, RFC 6143) mit VNC-Kennwort und VeNCrypt.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use des::cipher::{BlockEncrypt, KeyInit};
use rj_core::{ClipboardDirection, Secret};
use sha2::{Digest, Sha256};

use crate::codec::{self, extclip, Framebuffer, Zrle};

const ENC_RAW: i32 = 0;
const ENC_COPYRECT: i32 = 1;
const ENC_HEXTILE: i32 = 5;
const ENC_ZRLE: i32 = 16;
const ENC_DESKTOP_SIZE: i32 = -223;
const ENC_LAST_RECT: i32 = -224;
const ENC_EXT_DESKTOP_SIZE: i32 = -308;

#[derive(Debug, thiserror::Error)]
pub enum VncError {
    #[error("Verbindung: {0}")]
    Io(#[from] io::Error),
    #[error("Gegenstelle spricht kein RFB: {0:?}")]
    NotRfb(String),
    #[error("Gegenstelle lehnt ab: {0}")]
    Refused(String),
    #[error("Kein passendes Sicherheitsverfahren (angeboten: {0:?})")]
    NoSecurity(Vec<u8>),
    #[error("Anmeldung fehlgeschlagen: {0}")]
    AuthFailed(String),
    #[error("Kennwort erforderlich")]
    PasswordRequired,
    #[error("TLS: {0}")]
    Tls(String),
    #[error("Zertifikat abgelehnt")]
    CertificateRejected,
}

/// Angaben zum Serverzertifikat bei VeNCrypt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertInfo {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    pub old_fingerprint: Option<String>,
}

pub type CertCheck = Arc<dyn Fn(&CertInfo) -> bool + Send + Sync>;

/// Bekannte Zertifikate (Fingerabdruck SHA-256 je Host und Port).
#[derive(Debug, Clone)]
pub struct KnownCerts {
    path: std::path::PathBuf,
}

impl KnownCerts {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        KnownCerts { path: path.into() }
    }

    /// Gespeicherter Fingerabdruck für `host:port`.
    pub fn lookup(&self, id: &str) -> Option<String> {
        let text = std::fs::read_to_string(&self.path).ok()?;
        text.lines().find_map(|l| l.split_once(' ').filter(|(h, _)| *h == id).map(|(_, f)| f.trim().to_string()))
    }

    /// Fingerabdruck für `host:port` speichern (ersetzt einen alten).
    pub fn remember(&self, id: &str, fp: &str) -> io::Result<()> {
        if let Some(d) = self.path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let _lock = rj_core::fsutil::WriteLock::acquire_wait(&self.path, Duration::from_secs(5))?;
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        let mut out: String = text.lines().filter(|l| !l.starts_with(&format!("{id} "))).map(|l| format!("{l}\n")).collect();
        out.push_str(&format!("{id} {fp}\n"));
        rj_core::fsutil::atomic_write_locked(&self.path, out.as_bytes())
    }
}

/// SHA-256-Fingerabdruck eines Zertifikats in der Schreibweise von `vnc_certs` (kleine Hex-Ziffern mit `:`).
pub fn cert_fingerprint(der: &[u8]) -> String {
    Sha256::digest(der).iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
}

/// Holt nur das Serverzertifikat (VeNCrypt mit X.509), ohne Anmeldung: RFB-Begrüßung, VeNCrypt-Auswahl,
/// TLS-Handshake bis zur Zertifikatsprüfung, dann Abbruch. `Ok(None)`: Der Server bietet kein
/// VeNCrypt mit Zertifikat an (dann gibt es nichts zu prüfen).
pub fn probe_certificate(host: &str, port: u16, timeout: Duration) -> Result<Option<Vec<u8>>, VncError> {
    use std::net::ToSocketAddrs;
    let addr = (host, port).to_socket_addrs()?.next().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Adresse nicht auflösbar"))?;
    let mut sock = TcpStream::connect_timeout(&addr, timeout)?;
    sock.set_read_timeout(Some(timeout))?;
    sock.set_write_timeout(Some(timeout))?;
    let mut ver = [0u8; 12];
    sock.read_exact(&mut ver)?;
    let vs = String::from_utf8_lossy(&ver).into_owned();
    if !vs.starts_with("RFB ") {
        return Err(VncError::NotRfb(vs));
    }
    if &vs[4..11] == "003.003" {
        // RFB 3.3 kennt kein VeNCrypt.
        return Ok(None);
    }
    sock.write_all(b"RFB 003.008\n")?;
    let n = read_u8(&mut sock)?;
    if n == 0 {
        return Err(VncError::Refused(read_reason(&mut sock)));
    }
    let types = read_vec(&mut sock, n as usize)?;
    if !types.contains(&19) {
        return Ok(None);
    }
    sock.write_all(&[19])?;
    let (maj, min) = (read_u8(&mut sock)?, read_u8(&mut sock)?);
    if (maj, min) < (0, 2) {
        return Ok(None);
    }
    sock.write_all(&[0, 2])?;
    if read_u8(&mut sock)? != 0 {
        return Err(VncError::Refused("VeNCrypt-Version abgelehnt".into()));
    }
    let k = read_u8(&mut sock)?;
    let subs: Vec<u32> = (0..k).map(|_| read_u32(&mut sock)).collect::<io::Result<_>>()?;
    // Nur X.509-Verfahren haben ein prüfbares Zertifikat (X509None, X509Vnc, X509Plain).
    let Some(sub) = [260u32, 261, 262].into_iter().find(|s| subs.contains(s)) else { return Ok(None) };
    sock.write_all(&sub.to_be_bytes())?;
    if read_u8(&mut sock)? != 1 {
        return Err(VncError::Refused("VeNCrypt-Verfahren abgelehnt".into()));
    }
    let seen: Arc<Mutex<Option<Vec<u8>>>> = Arc::default();
    let seen2 = seen.clone();
    let check: Box<dyn FnMut(&[u8]) -> bool + Send> = Box::new(move |der: &[u8]| {
        *seen2.lock().unwrap_or_else(|e| e.into_inner()) = Some(der.to_vec());
        // Ablehnen beendet den Handshake vor jeder Anmeldung.
        false
    });
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let verifier = Arc::new(FingerprintVerifier { provider: provider.clone(), check: Arc::new(Mutex::new(Some(check))) });
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| VncError::Tls(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|e| VncError::Tls(e.to_string()))?;
    let mut conn = rustls::ClientConnection::new(Arc::new(config), name).map_err(|e| VncError::Tls(e.to_string()))?;
    while conn.is_handshaking() {
        if let Err(e) = conn.complete_io(&mut sock) {
            if seen.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
                break;
            }
            return Err(VncError::Tls(e.to_string()));
        }
    }
    let der = seen.lock().unwrap_or_else(|e| e.into_inner()).take();
    der.map(Some).ok_or_else(|| VncError::Tls("kein Zertifikat erhalten".into()))
}

/// Verbindungsparameter.
#[derive(Clone)]
pub struct VncParams {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<Secret>,
    pub shared: bool,
    pub view_only: bool,
    pub clipboard: ClipboardDirection,
    pub clipboard_limit: u32,
    /// VeNCrypt mit TLS bevorzugen, wenn angeboten.
    pub prefer_tls: bool,
    pub known_certs: Option<KnownCerts>,
    pub connect_timeout: Duration,
    /// Erweiterte Zwischenablage (UTF-8) anbieten; abschaltbar für alte Gegenstellen.
    pub extended_clipboard: bool,
}

/// Ereignisse für den Worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VncEvent {
    Frame { x: u32, y: u32, w: u32, h: u32 },
    Resized { width: u32, height: u32 },
    ClipText(String),
    Notice(String),
    Bell,
    Closed(String),
}

type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

/// Verbundener Client.
pub struct VncClient {
    writer: SharedWriter,
    pub events: Receiver<VncEvent>,
    fb: Arc<Mutex<Framebuffer>>,
    state: Arc<Mutex<ClipState>>,
    pub name: String,
    pub security: String,
    direction: ClipboardDirection,
    limit: u32,
    ext_desktop: Arc<Mutex<bool>>,
}

#[derive(Default)]
struct ClipState {
    /// Gegenstelle beherrscht die erweiterte Zwischenablage (UTF-8).
    extended: bool,
    /// Eigener Text, der auf Anfrage geliefert wird.
    pending: Option<String>,
    latin1_warned: bool,
}

fn read_u8(r: &mut (impl Read + ?Sized)) -> io::Result<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}
fn read_u16(r: &mut (impl Read + ?Sized)) -> io::Result<u16> {
    let mut b = [0u8; 2];
    r.read_exact(&mut b)?;
    Ok(u16::from_be_bytes(b))
}
fn read_u32(r: &mut (impl Read + ?Sized)) -> io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_be_bytes(b))
}
fn read_vec(r: &mut (impl Read + ?Sized), n: usize) -> io::Result<Vec<u8>> {
    let mut v = vec![0u8; n];
    r.read_exact(&mut v)?;
    Ok(v)
}
/// Überspringt `n` Bytes des Stroms.
fn skip(r: &mut (impl Read + ?Sized), n: u64) -> io::Result<()> {
    let got = io::copy(&mut Read::take(&mut *r, n), &mut io::sink())?;
    if got < n {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Verbindung während der Zwischenablage beendet"));
    }
    Ok(())
}

fn read_reason(r: &mut (impl Read + ?Sized)) -> String {
    match read_u32(r) {
        Ok(n) if n < 65536 => read_vec(r, n as usize).map(|v| String::from_utf8_lossy(&v).into_owned()).unwrap_or_default(),
        _ => String::new(),
    }
}

/// VNC-Kennwort-Antwort: DES mit bitweise gespiegeltem Schlüssel (höchstens 8 Bytes).
pub fn vnc_auth_response(password: &[u8], challenge: &[u8; 16]) -> [u8; 16] {
    let mut key = [0u8; 8];
    for (i, b) in password.iter().take(8).enumerate() {
        key[i] = b.reverse_bits();
    }
    let cipher = des::Des::new_from_slice(&key).expect("8 Bytes");
    let mut out = *challenge;
    for chunk in out.chunks_mut(8) {
        cipher.encrypt_block(chunk.into());
    }
    out
}

/// TLS-Prüfung über Fingerabdruck (Vertrauen bei erster Nutzung mit Nachfrage).
struct FingerprintVerifier {
    provider: Arc<rustls::crypto::CryptoProvider>,
    check: Arc<Mutex<Option<Box<dyn FnMut(&[u8]) -> bool + Send>>>>,
}

impl std::fmt::Debug for FingerprintVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FingerprintVerifier")
    }
}

impl rustls::client::danger::ServerCertVerifier for FingerprintVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let mut guard = self.check.lock().unwrap_or_else(|e| e.into_inner());
        let ok = guard.as_mut().map(|f| f(end_entity.as_ref())).unwrap_or(false);
        if ok {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("Zertifikat abgelehnt".into()))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// Lesender Teil einer TLS-Verbindung (wird von einem Faden befüllt).
struct PipeReader {
    rx: Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
}

impl Read for PipeReader {
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

/// Wechselt auf TLS; liefert Leser und Schreiber.
fn start_tls(sock: TcpStream, p: &VncParams, cert_check: &CertCheck) -> Result<(Box<dyn Read + Send>, Box<dyn Write + Send>), VncError> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let id = format!("{}:{}", p.host, p.port);
    let known = p.known_certs.clone();
    let check_cb = cert_check.clone();
    let host = p.host.clone();
    let port = p.port;
    let rejected = Arc::new(Mutex::new(false));
    let rej2 = rejected.clone();
    let check: Box<dyn FnMut(&[u8]) -> bool + Send> = Box::new(move |der: &[u8]| {
        let fp: String = Sha256::digest(der).iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":");
        let old = known.as_ref().and_then(|k| k.lookup(&id));
        if old.as_deref() == Some(fp.as_str()) {
            return true;
        }
        let ok = check_cb(&CertInfo { host: host.clone(), port, fingerprint: fp.clone(), old_fingerprint: old });
        if ok {
            if let Some(k) = &known {
                let _ = k.remember(&id, &fp);
            }
        } else {
            *rej2.lock().unwrap_or_else(|e| e.into_inner()) = true;
        }
        ok
    });
    let verifier = Arc::new(FingerprintVerifier { provider: provider.clone(), check: Arc::new(Mutex::new(Some(check))) });
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| VncError::Tls(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(p.host.clone()).map_err(|e| VncError::Tls(e.to_string()))?;
    let mut conn = rustls::ClientConnection::new(Arc::new(config), name).map_err(|e| VncError::Tls(e.to_string()))?;
    let mut s = sock;
    while conn.is_handshaking() {
        if let Err(e) = conn.complete_io(&mut s) {
            if *rejected.lock().unwrap_or_else(|e| e.into_inner()) {
                return Err(VncError::CertificateRejected);
            }
            return Err(VncError::Tls(e.to_string()));
        }
    }
    let conn = Arc::new(Mutex::new(conn));
    let (tx, rx) = mpsc::channel();
    let mut rsock = s.try_clone()?;
    let rconn = conn.clone();
    std::thread::Builder::new().name("vnc-tls".into()).spawn(move || {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = match rsock.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let mut c = rconn.lock().unwrap_or_else(|e| e.into_inner());
            let mut slice = &buf[..n];
            while !slice.is_empty() {
                if c.read_tls(&mut slice).is_err() {
                    return;
                }
                if c.process_new_packets().is_err() {
                    return;
                }
            }
            let mut plain = Vec::new();
            let _ = c.reader().read_to_end(&mut plain);
            while c.wants_write() {
                if c.write_tls(&mut rsock).is_err() {
                    return;
                }
            }
            drop(c);
            if !plain.is_empty() && tx.send(plain).is_err() {
                return;
            }
        }
    })?;
    Ok((Box::new(PipeReader { rx, buf: Vec::new(), pos: 0 }), Box::new(TlsWriter { conn, sock: s })))
}

fn vnc_auth(r: &mut dyn Read, w: &mut dyn Write, p: &VncParams) -> Result<(), VncError> {
    let pw = p.password.as_ref().ok_or(VncError::PasswordRequired)?;
    let mut ch = [0u8; 16];
    r.read_exact(&mut ch)?;
    w.write_all(&vnc_auth_response(pw.as_bytes(), &ch))?;
    w.flush()?;
    Ok(())
}

fn security_result(r: &mut dyn Read, minor: u8) -> Result<(), VncError> {
    let res = read_u32(r)?;
    if res != 0 {
        let reason = if minor >= 8 { read_reason(r) } else { String::new() };
        return Err(VncError::AuthFailed(if reason.is_empty() { "Kennwort falsch oder Zugang verweigert".into() } else { reason }));
    }
    Ok(())
}

impl VncClient {
    /// Baut die Verbindung auf und meldet an.
    pub fn connect(p: &VncParams, cert_check: CertCheck) -> Result<VncClient, VncError> {
        use std::net::ToSocketAddrs;
        let addr = (p.host.as_str(), p.port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Adresse nicht auflösbar"))?;
        let sock = TcpStream::connect_timeout(&addr, p.connect_timeout)?;
        sock.set_nodelay(true).ok();
        sock.set_read_timeout(Some(p.connect_timeout))?;
        let mut r: Box<dyn Read + Send> = Box::new(sock.try_clone()?);
        let mut w: Box<dyn Write + Send> = Box::new(sock.try_clone()?);

        let mut ver = [0u8; 12];
        r.read_exact(&mut ver)?;
        let vs = String::from_utf8_lossy(&ver).into_owned();
        if !vs.starts_with("RFB ") {
            return Err(VncError::NotRfb(vs));
        }
        let minor: u8 = match &vs[4..11] {
            "003.003" => 3,
            "003.007" => 7,
            _ => 8,
        };
        w.write_all(format!("RFB 003.00{minor}\n").as_bytes())?;
        let mut notices = Vec::new();
        let mut security = String::from("keine");

        let types: Vec<u8> = if minor == 3 {
            // RFB 3.3: Typ 0 heißt Ablehnung, danach folgt der Grund.
            match read_u32(&mut r)? {
                0 => return Err(VncError::Refused(read_reason(&mut r))),
                t => vec![t as u8],
            }
        } else {
            let n = read_u8(&mut r)?;
            if n == 0 {
                return Err(VncError::Refused(read_reason(&mut r)));
            }
            read_vec(&mut r, n as usize)?
        };
        let choice = if p.prefer_tls && types.contains(&19) {
            19
        } else if types.contains(&2) && p.password.is_some() {
            2
        } else if types.contains(&1) {
            1
        } else if types.contains(&19) {
            19
        } else if types.contains(&2) {
            return Err(VncError::PasswordRequired);
        } else {
            return Err(VncError::NoSecurity(types));
        };
        if minor >= 7 {
            w.write_all(&[choice])?;
        }
        match choice {
            1 => {
                if minor >= 8 {
                    security_result(&mut r, minor)?;
                }
            }
            2 => {
                security = "VNC-Kennwort".into();
                if p.password.as_ref().is_some_and(|pw| pw.len() > 8) {
                    notices.push("VNC-Kennwörter sind auf 8 Bytes begrenzt; nur der Anfang wird verwendet.".to_string());
                }
                vnc_auth(&mut r, &mut w, p)?;
                security_result(&mut r, minor)?;
            }
            19 => {
                let (maj, min) = (read_u8(&mut r)?, read_u8(&mut r)?);
                if (maj, min) < (0, 2) {
                    return Err(VncError::NoSecurity(vec![19]));
                }
                w.write_all(&[0, 2])?;
                if read_u8(&mut r)? != 0 {
                    return Err(VncError::Refused("VeNCrypt-Version abgelehnt".into()));
                }
                let n = read_u8(&mut r)?;
                let subs: Vec<u32> = (0..n).map(|_| read_u32(&mut r)).collect::<io::Result<_>>()?;
                // X509Vnc, X509Plain, X509None, TLSVnc, TLSNone; TLS ohne Zertifikat (anonym) nicht unterstützt.
                let pref: &[u32] = if p.password.is_some() { &[261, 262, 260] } else { &[260] };
                let sub = *pref.iter().find(|s| subs.contains(s)).ok_or_else(|| VncError::NoSecurity(subs.iter().map(|s| *s as u8).collect()))?;
                w.write_all(&sub.to_be_bytes())?;
                w.flush()?;
                if read_u8(&mut r)? != 1 {
                    return Err(VncError::Refused("VeNCrypt-Verfahren abgelehnt".into()));
                }
                drop(r);
                drop(w);
                let (tr, tw) = start_tls(sock.try_clone()?, p, &cert_check)?;
                r = tr;
                w = tw;
                security = match sub {
                    261 => "VeNCrypt X.509 + VNC-Kennwort",
                    262 => "VeNCrypt X.509 + Benutzer/Kennwort",
                    _ => "VeNCrypt X.509",
                }
                .into();
                match sub {
                    261 => vnc_auth(&mut r, &mut w, p)?,
                    262 => {
                        let user = p.username.clone().unwrap_or_default();
                        let pw = p.password.as_ref().ok_or(VncError::PasswordRequired)?;
                        w.write_all(&(user.len() as u32).to_be_bytes())?;
                        w.write_all(&(pw.len() as u32).to_be_bytes())?;
                        w.write_all(user.as_bytes())?;
                        w.write_all(pw.as_bytes())?;
                    }
                    _ => {}
                }
                security_result(&mut r, minor)?;
            }
            _ => unreachable!(),
        }

        // ClientInit / ServerInit.
        w.write_all(&[p.shared as u8])?;
        let width = read_u16(&mut r)? as u32;
        let height = read_u16(&mut r)? as u32;
        let _pf = read_vec(&mut r, 16)?;
        let name_len = read_u32(&mut r)? as usize;
        let name = String::from_utf8_lossy(&read_vec(&mut r, name_len.min(4096))?).into_owned();
        sock.set_read_timeout(None)?;

        // Bildpunktformat und Kodierungen.
        let mut msg = vec![0u8, 0, 0, 0, 32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0];
        let mut encs = vec![ENC_ZRLE, ENC_HEXTILE, ENC_COPYRECT, ENC_RAW, ENC_EXT_DESKTOP_SIZE, ENC_DESKTOP_SIZE, ENC_LAST_RECT];
        if p.extended_clipboard {
            encs.push(extclip::PSEUDO_ENCODING);
        }
        msg.extend_from_slice(&[2, 0]);
        msg.extend_from_slice(&(encs.len() as u16).to_be_bytes());
        for e in encs {
            msg.extend_from_slice(&e.to_be_bytes());
        }
        msg.extend_from_slice(&update_request(false, width, height));
        w.write_all(&msg)?;
        w.flush()?;

        let writer: SharedWriter = Arc::new(Mutex::new(w));
        let fb = Arc::new(Mutex::new(Framebuffer::new(width, height)));
        let state = Arc::new(Mutex::new(ClipState::default()));
        let ext_desktop = Arc::new(Mutex::new(false));
        let (tx, rx) = mpsc::channel();
        for n in notices {
            let _ = tx.send(VncEvent::Notice(n));
        }
        let reader = Reader {
            r,
            w: writer.clone(),
            fb: fb.clone(),
            state: state.clone(),
            tx,
            zrle: Zrle::default(),
            direction: p.clipboard,
            limit: p.clipboard_limit,
            ext_desktop: ext_desktop.clone(),
        };
        std::thread::Builder::new().name("vnc-lesen".into()).spawn(move || reader.run())?;
        Ok(VncClient { writer, events: rx, fb, state, name, security, direction: p.clipboard, limit: p.clipboard_limit, ext_desktop })
    }

    fn send(&self, data: &[u8]) -> io::Result<()> {
        let mut w = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        w.write_all(data)?;
        w.flush()
    }

    pub fn with_frame<R>(&self, f: impl FnOnce(&Framebuffer) -> R) -> R {
        f(&self.fb.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn key(&self, keysym: u32, down: bool) -> io::Result<()> {
        let mut m = vec![4, down as u8, 0, 0];
        m.extend_from_slice(&keysym.to_be_bytes());
        self.send(&m)
    }

    /// Text als Tastenfolge (Unicode-Keysyms).
    pub fn type_text(&self, text: &str) -> io::Result<()> {
        for c in text.chars() {
            let k = codec::keysym_for(c);
            self.key(k, true)?;
            self.key(k, false)?;
        }
        Ok(())
    }

    pub fn pointer(&self, mask: u8, x: u16, y: u16) -> io::Result<()> {
        let mut m = vec![5, mask];
        m.extend_from_slice(&x.to_be_bytes());
        m.extend_from_slice(&y.to_be_bytes());
        self.send(&m)
    }

    /// Größenänderung (ExtendedDesktopSize); `false`, wenn nicht unterstützt.
    pub fn resize(&self, width: u32, height: u32) -> io::Result<bool> {
        if !*self.ext_desktop.lock().unwrap_or_else(|e| e.into_inner()) {
            return Ok(false);
        }
        let mut m = vec![251, 0];
        m.extend_from_slice(&(width as u16).to_be_bytes());
        m.extend_from_slice(&(height as u16).to_be_bytes());
        m.extend_from_slice(&[1, 0]);
        m.extend_from_slice(&0u32.to_be_bytes());
        m.extend_from_slice(&[0, 0, 0, 0]);
        m.extend_from_slice(&(width as u16).to_be_bytes());
        m.extend_from_slice(&(height as u16).to_be_bytes());
        m.extend_from_slice(&0u32.to_be_bytes());
        self.send(&m)?;
        Ok(true)
    }

    /// Text in die Zwischenablage des Ziels. Ohne erweiterte Zwischenablage
    /// nur Latin-1; sonst Fehler mit sichtbarer Warnung statt Zeichenverlust.
    pub fn send_clipboard(&self, text: &str) -> Result<(), String> {
        if !matches!(self.direction, ClipboardDirection::Both | ClipboardDirection::HostToTarget) {
            return Err("Zwischenablage Host → Ziel ist für diese Verbindung abgeschaltet".into());
        }
        if text.len() as u64 > self.limit as u64 {
            return Err(format!("Inhalt überschreitet das Größenlimit ({} Bytes)", self.limit));
        }
        let extended = {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if st.extended {
                st.pending = Some(text.to_string());
            }
            st.extended
        };
        let r = if extended {
            self.send(&ext_message(&extclip::simple(extclip::NOTIFY)))
        } else {
            let latin = codec::to_latin1(text).map_err(|bad| {
                format!("Die Gegenstelle unterstützt nur Latin-1; nicht darstellbare Zeichen ({bad}) – nichts gesendet")
            })?;
            let mut m = vec![6, 0, 0, 0];
            m.extend_from_slice(&(latin.len() as u32).to_be_bytes());
            m.extend_from_slice(&latin);
            self.send(&m)
        };
        r.map_err(|e| e.to_string())
    }

    pub fn extended_clipboard(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).extended
    }
}

fn update_request(incremental: bool, w: u32, h: u32) -> Vec<u8> {
    let mut m = vec![3, incremental as u8, 0, 0, 0, 0];
    m.extend_from_slice(&(w as u16).to_be_bytes());
    m.extend_from_slice(&(h as u16).to_be_bytes());
    m
}

/// ClientCutText mit negativer Länge (erweiterte Zwischenablage).
fn ext_message(payload: &[u8]) -> Vec<u8> {
    let mut m = vec![6, 0, 0, 0];
    m.extend_from_slice(&(-(payload.len() as i32)).to_be_bytes());
    m.extend_from_slice(payload);
    m
}

struct Reader {
    r: Box<dyn Read + Send>,
    w: SharedWriter,
    fb: Arc<Mutex<Framebuffer>>,
    state: Arc<Mutex<ClipState>>,
    tx: Sender<VncEvent>,
    zrle: Zrle,
    direction: ClipboardDirection,
    limit: u32,
    ext_desktop: Arc<Mutex<bool>>,
}

impl Reader {
    fn send(&self, data: &[u8]) -> io::Result<()> {
        let mut w = self.w.lock().unwrap_or_else(|e| e.into_inner());
        w.write_all(data)?;
        w.flush()
    }

    fn run(mut self) {
        let reason = loop {
            if let Err(e) = self.message() {
                break if e.kind() == io::ErrorKind::UnexpectedEof { "Verbindung vom Server beendet".to_string() } else { e.to_string() };
            }
        };
        let _ = self.tx.send(VncEvent::Closed(reason));
    }

    fn message(&mut self) -> io::Result<()> {
        let t = read_u8(&mut self.r)?;
        match t {
            0 => self.update(),
            1 => {
                let _ = read_u8(&mut self.r)?;
                let _first = read_u16(&mut self.r)?;
                let n = read_u16(&mut self.r)?;
                read_vec(&mut self.r, n as usize * 6)?;
                Ok(())
            }
            2 => {
                let _ = self.tx.send(VncEvent::Bell);
                Ok(())
            }
            3 => self.cut_text(),
            150 => Ok(()),
            248 => {
                read_vec(&mut self.r, 3)?;
                let _flags = read_u32(&mut self.r)?;
                let n = read_u8(&mut self.r)?;
                read_vec(&mut self.r, n as usize)?;
                Ok(())
            }
            other => Err(io::Error::new(io::ErrorKind::InvalidData, format!("Unbekannte Servernachricht {other}"))),
        }
    }

    fn update(&mut self) -> io::Result<()> {
        let _ = read_u8(&mut self.r)?;
        let n = read_u16(&mut self.r)?;
        let mut dirty: Option<(u32, u32, u32, u32)> = None;
        for _ in 0..n {
            let x = read_u16(&mut self.r)? as u32;
            let y = read_u16(&mut self.r)? as u32;
            let w = read_u16(&mut self.r)? as u32;
            let h = read_u16(&mut self.r)? as u32;
            let enc = read_u32(&mut self.r)? as i32;
            let mut fb = self.fb.lock().unwrap_or_else(|e| e.into_inner());
            match enc {
                ENC_RAW => {
                    let buf = read_vec(&mut self.r, w as usize * h as usize * 4)?;
                    fb.raw(x, y, w, h, &buf)?;
                }
                ENC_COPYRECT => {
                    let sx = read_u16(&mut self.r)? as u32;
                    let sy = read_u16(&mut self.r)? as u32;
                    fb.copy_rect(x, y, w, h, sx, sy)?;
                }
                ENC_HEXTILE => codec::hextile(&mut fb, &mut self.r, x, y, w, h)?,
                ENC_ZRLE => {
                    let len = read_u32(&mut self.r)? as usize;
                    let data = read_vec(&mut self.r, len)?;
                    self.zrle.decode(&mut fb, &data, x, y, w, h)?;
                }
                ENC_DESKTOP_SIZE => {
                    fb.resize(w, h);
                    let _ = self.tx.send(VncEvent::Resized { width: w, height: h });
                    dirty = Some((0, 0, w, h));
                    continue;
                }
                ENC_EXT_DESKTOP_SIZE => {
                    let screens = read_u8(&mut self.r)?;
                    read_vec(&mut self.r, 3 + screens as usize * 16)?;
                    *self.ext_desktop.lock().unwrap_or_else(|e| e.into_inner()) = true;
                    // y = Status; 0 = Erfolg. Nur bei Größenwechsel neu anlegen.
                    if y == 0 && (w != fb.width || h != fb.height) {
                        fb.resize(w, h);
                        let _ = self.tx.send(VncEvent::Resized { width: w, height: h });
                        dirty = Some((0, 0, w, h));
                    } else if y != 0 {
                        let _ = self.tx.send(VncEvent::Notice(format!("Gegenstelle lehnt Größenänderung ab (Status {y})")));
                    }
                    continue;
                }
                ENC_LAST_RECT => break,
                other => {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Kodierung {other} nicht angefordert")));
                }
            }
            dirty = Some(match dirty {
                None => (x, y, x + w, y + h),
                Some((a, b, c, d)) => (a.min(x), b.min(y), c.max(x + w), d.max(y + h)),
            });
        }
        if let Some((a, b, c, d)) = dirty {
            let _ = self.tx.send(VncEvent::Frame { x: a, y: b, w: c - a, h: d - b });
        }
        let (fw, fh) = {
            let fb = self.fb.lock().unwrap_or_else(|e| e.into_inner());
            (fb.width, fb.height)
        };
        self.send(&update_request(true, fw, fh))
    }

    fn cut_text(&mut self) -> io::Result<()> {
        read_vec(&mut self.r, 3)?;
        let len = read_u32(&mut self.r)? as i32;
        let allowed = matches!(self.direction, ClipboardDirection::Both | ClipboardDirection::TargetToHost);
        if len >= 0 {
            if !allowed || len as u32 > self.limit {
                // Nicht gewünscht oder zu groß: überspringen, ohne den Inhalt
                // im Speicher anzulegen.
                skip(&mut self.r, len as u64)?;
                return Ok(());
            }
            let data = read_vec(&mut self.r, len as usize)?;
            {
                let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
                if !st.extended && !st.latin1_warned {
                    st.latin1_warned = true;
                    let _ = self.tx.send(VncEvent::Notice(
                        "Die Gegenstelle überträgt die Zwischenablage nur als Latin-1; Zeichen außerhalb gehen dort verloren.".into(),
                    ));
                }
                drop(st);
                let _ = self.tx.send(VncEvent::ClipText(codec::from_latin1(&data)));
            }
            return Ok(());
        }
        let n = (-(len as i64)) as usize;
        if n < 4 || n > 64 * 1024 * 1024 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Erweiterte Zwischenablage: Länge ungültig"));
        }
        let data = read_vec(&mut self.r, n)?;
        let flags = u32::from_be_bytes(data[..4].try_into().unwrap());
        let body = &data[4..];
        if flags & extclip::CAPS != 0 {
            self.state.lock().unwrap_or_else(|e| e.into_inner()).extended = true;
            self.send(&ext_message(&extclip::caps(self.limit)))?;
        } else if flags & extclip::REQUEST != 0 {
            let text = self.state.lock().unwrap_or_else(|e| e.into_inner()).pending.clone();
            if let Some(t) = text.filter(|_| matches!(self.direction, ClipboardDirection::Both | ClipboardDirection::HostToTarget)) {
                self.send(&ext_message(&extclip::provide_text(&t)?))?;
            }
        } else if flags & extclip::PEEK != 0 {
            let has = self.state.lock().unwrap_or_else(|e| e.into_inner()).pending.is_some();
            let f = if has { extclip::NOTIFY | extclip::FORMAT_TEXT } else { extclip::NOTIFY };
            self.send(&ext_message(&f.to_be_bytes()))?;
        } else if flags & extclip::NOTIFY != 0 {
            if allowed && flags & extclip::FORMAT_TEXT != 0 {
                self.send(&ext_message(&extclip::simple(extclip::REQUEST)))?;
            }
        } else if flags & extclip::PROVIDE != 0 && allowed {
            match extclip::parse_provide_text(flags, body, self.limit as usize) {
                Ok(Some(t)) => {
                    let _ = self.tx.send(VncEvent::ClipText(t));
                }
                Ok(None) => {}
                Err(e) => {
                    let _ = self.tx.send(VncEvent::Notice(format!("Zwischenablage der Gegenstelle nicht übernommen: {e}")));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vnc_auth_known_vector() {
        // Gegenprobe: Schlüssel "password", Herausforderung 0..16; Ergebnis
        // muss mit sich selbst und anderer Herausforderung unterschiedlich sein.
        let ch: [u8; 16] = core::array::from_fn(|i| i as u8);
        let a = vnc_auth_response(b"password", &ch);
        let b = vnc_auth_response(b"passwore", &ch);
        assert_ne!(a, b);
        assert_eq!(a, vnc_auth_response(b"password-lang", &ch), "nur 8 Bytes zählen");
    }
}
