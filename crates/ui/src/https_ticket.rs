//! HTTPS für Erweiterungen: Der Kern ruft eine HTTPS-Adresse mit den Zugangsdaten eines Ziels (bzw. eines
//! Zugangswegs, Anmeldeobjekts oder benannten Zugangsdatensatzes) ab. Die Erweiterung bekommt nur ein Ticket (Nummer)
//! und die Antwort; Kennwort oder Schlüssel verlassen den Kern nie. Das Zertifikat wird über den Fingerabdruck
//! geprüft (beim ersten Mal Rückfrage in der Oberfläche der Erweiterung, angenommene Abdrücke je `host:port`).

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use rj_ipc::msg::CredentialPayload;
use serde_json::{json, Value};

/// Wie die Zugangsdaten in die Anfrage kommen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpAuth {
    /// Kennwort des Zugangsdatensatzes als Wert eines Kopffelds (z. B. API-Schlüssel).
    Header(String),
    /// `Authorization: Bearer <Kennwort>`.
    Bearer,
    /// `Authorization: Basic` aus Benutzer und Kennwort.
    Basic,
    /// Ohne Zugangsdaten.
    None,
}

impl HttpAuth {
    /// Aus der Angabe der Erweiterung: `{"kind":"header","name":"X-API-KEY"}`, `{"kind":"bearer"}`,
    /// `{"kind":"basic"}`, `{"kind":"none"}`.
    pub fn from_json(v: &Value) -> Result<HttpAuth, String> {
        match v.get("kind").and_then(Value::as_str).unwrap_or("none") {
            "header" => {
                let name = v.get("name").and_then(Value::as_str).unwrap_or_default().trim();
                let valid = !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
                // Keine Kopffelder, die die Verbindung selbst betreffen.
                let reserved = ["host", "content-length", "connection", "transfer-encoding"].iter().any(|r| name.eq_ignore_ascii_case(r));
                if !valid || reserved {
                    return Err(format!("Ungültiges Kopffeld „{name}“"));
                }
                Ok(HttpAuth::Header(name.to_string()))
            }
            "bearer" => Ok(HttpAuth::Bearer),
            "basic" => Ok(HttpAuth::Basic),
            "none" => Ok(HttpAuth::None),
            other => Err(format!("Unbekannte Anmeldeart „{other}“")),
        }
    }
}

/// Angaben hinter einem HTTPS-Ticket.
pub struct HttpSpec {
    pub host: String,
    pub port: u16,
    pub auth: HttpAuth,
    pub credentials: CredentialPayload,
}

/// Ablage der angenommenen Fingerabdrücke (`host:port` → SHA-256).
fn known_path() -> Option<PathBuf> {
    rj_core::config::data_dir().map(|d| d.join("https-zertifikate.json"))
}

pub fn known() -> BTreeMap<String, String> {
    known_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Zuletzt vorgelegte, noch nicht angenommene Fingerabdrücke je `host:port` (nur diese lassen sich annehmen).
static SEEN: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn note_seen(host_port: &str, fp: &str) {
    SEEN.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(host_port.to_string(), fp.to_string());
}

/// Fingerabdruck für `host:port` annehmen (nach Bestätigung durch den Anwender). Nur der Abdruck, den der Server
/// zuletzt über ein HTTPS-Ticket tatsächlich vorgelegt hat – eine Erweiterung kann keinen beliebigen festlegen.
pub fn accept(host_port: &str, fingerprint: &str) -> Result<(), String> {
    let fp = fingerprint.trim().to_ascii_lowercase();
    let ok = fp.split(':').count() == 32 && fp.split(':').all(|b| b.len() == 2 && b.bytes().all(|c| c.is_ascii_hexdigit()));
    if !ok || host_port.trim().is_empty() {
        return Err("Ungültiger Fingerabdruck".into());
    }
    let seen = SEEN.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(host_port.trim()).cloned());
    if seen.as_deref() != Some(fp.as_str()) {
        return Err("Dieser Fingerabdruck wurde vom Server nicht vorgelegt".into());
    }
    let p = known_path().ok_or("Kein Datenordner")?;
    let mut k = known();
    k.insert(host_port.trim().to_string(), fp);
    std::fs::create_dir_all(p.parent().unwrap_or(&p)).map_err(|e| e.to_string())?;
    rj_core::fsutil::atomic_write_with_backup(&p, &serde_json::to_vec_pretty(&k).unwrap_or_default()).map_err(|e| e.to_string())
}

fn header_value(spec: &HttpSpec) -> Result<Option<(String, String)>, String> {
    if spec.auth == HttpAuth::None {
        return Ok(None);
    }
    // Steuerzeichen (Zeilenumbruch) im Geheimnis würden weitere Kopffelder einschleusen.
    let secret = String::from_utf8(spec.credentials.password.0.clone()).map_err(|_| "Das Kennwort des Zugangsdatensatzes ist kein gültiger Text".to_string())?;
    if secret.chars().any(char::is_control) || spec.credentials.username.chars().any(char::is_control) {
        return Err("Kennwort bzw. Benutzer des Zugangsdatensatzes enthält Steuerzeichen".into());
    }
    Ok(header_value_unchecked(spec, secret))
}

fn header_value_unchecked(spec: &HttpSpec, secret: String) -> Option<(String, String)> {
    match &spec.auth {
        HttpAuth::Header(name) => Some((name.clone(), secret)),
        HttpAuth::Bearer => Some(("Authorization".into(), format!("Bearer {secret}"))),
        HttpAuth::Basic => {
            let user = match &spec.credentials.domain {
                Some(d) if !d.is_empty() => format!("{d}\\{}", spec.credentials.username),
                _ => spec.credentials.username.clone(),
            };
            Some(("Authorization".into(), format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{user}:{secret}")))))
        }
        HttpAuth::None => None,
    }
}

/// Eine Anfrage. `a`: `method` (GET/POST/PUT/PATCH/DELETE), `path` (mit `/` am Anfang, samt Abfrage), `body`
/// (JSON-Wert, wird als JSON gesendet) oder `body_text`, `headers` (weitere Kopffelder, ohne Zugangsdaten),
/// `timeout_secs`. Antwort: `status`, `headers` (Kleinbuchstaben), `body` (Text) bzw. `body_b64` (Binärdaten), `json`
/// (falls JSON). Unbekanntes oder geändertes Zertifikat: Fehler mit `{"certificate":{"host","fingerprint","old"}}`.
pub fn request(spec: &HttpSpec, a: &Value) -> Result<Value, String> {
    let method = a.get("method").and_then(Value::as_str).unwrap_or("GET").to_ascii_uppercase();
    if !["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&method.as_str()) {
        return Err(format!("Methode „{method}“ nicht erlaubt"));
    }
    let path = a.get("path").and_then(Value::as_str).unwrap_or("/");
    if !path.starts_with('/') || !path.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err("Ungültiger Pfad".into());
    }
    let timeout = Duration::from_secs(a.get("timeout_secs").and_then(Value::as_u64).unwrap_or(30).clamp(1, 600));
    let key = format!("{}:{}", spec.host, spec.port);
    let accepted = known().get(&key).cloned();
    let (body, ctype): (Vec<u8>, Option<&str>) = match (a.get("body"), a.get("body_text").and_then(Value::as_str)) {
        (Some(b), _) if !b.is_null() => (b.to_string().into_bytes(), Some("application/json")),
        (_, Some(t)) => (t.as_bytes().to_vec(), Some("text/plain")),
        _ => (Vec::new(), None),
    };
    let mut headers: Vec<(String, String)> = vec![("Accept".into(), "application/json".into())];
    if let Some(c) = ctype {
        headers.push(("Content-Type".into(), c.into()));
    }
    for (k, v) in a.get("headers").and_then(Value::as_object).into_iter().flatten() {
        let v = v.as_str().unwrap_or_default();
        let bad = k.is_empty() || !k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') || v.chars().any(char::is_control);
        let reserved = ["host", "content-length", "connection", "transfer-encoding", "authorization"].iter().any(|r| k.eq_ignore_ascii_case(r));
        let auth_name = matches!(&spec.auth, HttpAuth::Header(n) if n.eq_ignore_ascii_case(k));
        if bad || reserved || auth_name {
            return Err(format!("Kopffeld „{k}“ nicht erlaubt"));
        }
        headers.push((k.clone(), v.to_string()));
    }
    if let Some(h) = header_value(spec)? {
        headers.push(h);
    }
    let seen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let r = match send(spec, &method, path, &headers, &body, timeout, accepted.clone(), seen.clone()) {
        Ok(r) => r,
        Err(e) => {
            if let Some(fp) = seen.lock().unwrap_or_else(|e| e.into_inner()).take() {
                if accepted.as_deref() != Some(fp.as_str()) {
                    note_seen(&key, &fp);
                    return Err(json!({ "certificate": { "host": key, "fingerprint": fp, "old": accepted } }).to_string());
                }
            }
            return Err(e.to_string());
        }
    };
    let mut out = json!({ "status": r.status });
    out["headers"] = Value::Object(r.headers.iter().filter(|(k, _)| !k.eq_ignore_ascii_case("set-cookie")).map(|(k, v)| (k.to_ascii_lowercase(), json!(v))).collect());
    match String::from_utf8(r.body) {
        Ok(text) => {
            if let Ok(j) = serde_json::from_str::<Value>(&text) {
                out["json"] = j;
            } else {
                out["body"] = json!(text);
            }
        }
        Err(e) => out["body_b64"] = json!(base64::engine::general_purpose::STANDARD.encode(e.into_bytes())),
    }
    Ok(out)
}


/// Obergrenze einer Antwort (Kopf und Körper); größere Antworten werden abgelehnt, statt Speicher zu belegen.
pub const MAX_RESPONSE: usize = 64 * 1024 * 1024;

pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Prüft das Serverzertifikat nur über den angenommenen Fingerabdruck (SHA-256 des Zertifikats).
#[derive(Debug)]
struct Pinned {
    provider: Arc<rustls::crypto::CryptoProvider>,
    accepted: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
    host: String,
    port: u16,
}

impl rustls::client::danger::ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end: &rustls::pki_types::CertificateDer<'_>,
        _i: &[rustls::pki_types::CertificateDer<'_>],
        _n: &rustls::pki_types::ServerName<'_>,
        _o: &[u8],
        _t: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        use sha2::Digest;
        let fp = sha2::Sha256::digest(end.as_ref()).iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":");
        *self.seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(fp.clone());
        let ok = self.accepted.as_deref() == Some(fp.as_str());
        // „Bei Schlüsselfehlern trotzdem fortfahren“: nur für diese Verbindung annehmen und melden.
        if ok || crate::keyerrors::accept() {
            if !ok {
                crate::keyerrors::warn(crate::keyerrors::text("certificate", &self.host, self.port, "X.509", &fp, self.accepted.is_some()));
            }
            return Ok(rustls::client::danger::ServerCertVerified::assertion());
        }
        Err(rustls::Error::General("Zertifikat abgelehnt".into()))
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

fn bad(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg.to_string())
}

/// Eine HTTPS-Anfrage (eine Verbindung, `Connection: close`) mit Zeitgrenze je Lesen und Schreiben und
/// Größengrenze der Antwort.
#[allow(clippy::too_many_arguments)]
fn send(spec: &HttpSpec, method: &str, path: &str, headers: &[(String, String)], body: &[u8], timeout: Duration, accepted: Option<String>, seen: Arc<Mutex<Option<String>>>) -> std::io::Result<Response> {
    use std::net::ToSocketAddrs;
    let host = spec.host.trim().trim_start_matches('[').trim_end_matches(']').to_string();
    let addr = (host.as_str(), spec.port).to_socket_addrs()?.next().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "Adresse nicht auflösbar"))?;
    let sock = std::net::TcpStream::connect_timeout(&addr, timeout)?;
    sock.set_read_timeout(Some(timeout))?;
    sock.set_write_timeout(Some(timeout))?;
    sock.set_nodelay(true).ok();
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let verifier = Arc::new(Pinned { provider: provider.clone(), accepted, seen, host: spec.host.clone(), port: spec.port });
    let config = rustls::ClientConfig::builder_with_provider(provider).with_safe_default_protocol_versions().map_err(std::io::Error::other)?.dangerous().with_custom_certificate_verifier(verifier).with_no_client_auth();
    let name = match host.parse::<std::net::IpAddr>() {
        Ok(ip) => rustls::pki_types::ServerName::IpAddress(ip.into()),
        Err(_) => rustls::pki_types::ServerName::try_from(host.clone()).map_err(std::io::Error::other)?,
    };
    let conn = rustls::ClientConnection::new(Arc::new(config), name).map_err(std::io::Error::other)?;
    let mut tls = rustls::StreamOwned::new(conn, sock);
    let host_header = if host.contains(':') { format!("[{host}]:{}", spec.port) } else { format!("{host}:{}", spec.port) };
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {host_header}\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    tls.write_all(req.as_bytes())?;
    tls.write_all(body)?;
    tls.flush()?;
    let mut r = BufReader::new(tls.take(MAX_RESPONSE as u64 + 1));
    let mut line = String::new();
    r.read_line(&mut line)?;
    let status = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or_else(|| bad("keine HTTP-Antwort"))?;
    let mut headers = Vec::new();
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            break;
        }
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let header = |n: &str| headers.iter().find(|(k, _): &&(String, String)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.clone());
    let mut out = Vec::new();
    if header("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        loop {
            let mut size = String::new();
            r.read_line(&mut size)?;
            let n = usize::from_str_radix(size.trim().split(';').next().unwrap_or("0"), 16).map_err(|_| bad("Blockgröße"))?;
            if n == 0 {
                break;
            }
            if out.len() + n > MAX_RESPONSE {
                return Err(bad("Antwort zu groß"));
            }
            let start = out.len();
            out.resize(start + n, 0);
            r.read_exact(&mut out[start..])?;
            let mut crlf = String::new();
            r.read_line(&mut crlf)?;
        }
    } else if let Some(n) = header("content-length").and_then(|v| v.parse::<usize>().ok()) {
        if n > MAX_RESPONSE {
            return Err(bad("Antwort zu groß"));
        }
        out.resize(n, 0);
        r.read_exact(&mut out)?;
    } else {
        // Ohne Längenangabe bis zum Verbindungsende; eine TLS-Verbindung ohne sauberen Abschluss zählt als Ende.
        match r.read_to_end(&mut out) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {}
            Err(e) => return Err(e),
        }
        if out.len() > MAX_RESPONSE {
            return Err(bad("Antwort zu groß"));
        }
    }
    Ok(Response { status, headers, body: out })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rj_ipc::msg::SecretBytes;

    fn spec(auth: HttpAuth) -> HttpSpec {
        HttpSpec { host: "192.0.2.1".into(), port: 443, auth, credentials: CredentialPayload { username: "u".into(), domain: None, password: SecretBytes(b"geheim".to_vec()), private_key: None } }
    }

    #[test]
    fn auth_kinds_are_parsed_and_checked() {
        assert_eq!(HttpAuth::from_json(&json!({ "kind": "header", "name": "X-API-KEY" })).unwrap(), HttpAuth::Header("X-API-KEY".into()));
        assert!(HttpAuth::from_json(&json!({ "kind": "header", "name": "Host" })).is_err());
        assert!(HttpAuth::from_json(&json!({ "kind": "header", "name": "X\r\nY" })).is_err());
        assert!(HttpAuth::from_json(&json!({ "kind": "magic" })).is_err());
        assert_eq!(HttpAuth::from_json(&json!({})).unwrap(), HttpAuth::None);
    }

    #[test]
    fn secret_goes_only_into_the_auth_header() {
        assert_eq!(header_value(&spec(HttpAuth::Header("X-API-KEY".into()))).unwrap(), Some(("X-API-KEY".into(), "geheim".into())));
        assert_eq!(header_value(&spec(HttpAuth::Bearer)).unwrap().unwrap().1, "Bearer geheim");
        assert_eq!(header_value(&spec(HttpAuth::Basic)).unwrap().unwrap().1, format!("Basic {}", base64::engine::general_purpose::STANDARD.encode("u:geheim")));
        assert_eq!(header_value(&spec(HttpAuth::None)).unwrap(), None);
        let mut s = spec(HttpAuth::Header("X-API-KEY".into()));
        s.credentials.password = SecretBytes(b"geheim\r\nX-Evil: 1".to_vec());
        assert!(header_value(&s).is_err(), "Steuerzeichen im Geheimnis");
    }

    #[test]
    fn requests_with_bad_method_path_or_headers_are_refused_before_connecting() {
        let s = spec(HttpAuth::Header("X-API-KEY".into()));
        assert!(request(&s, &json!({ "method": "CONNECT", "path": "/" })).unwrap_err().contains("nicht erlaubt"));
        assert!(request(&s, &json!({ "path": "kein-schraegstrich" })).unwrap_err().contains("Pfad"));
        assert!(request(&s, &json!({ "path": "/a\r\nb" })).unwrap_err().contains("Pfad"));
        assert!(request(&s, &json!({ "path": "/a\tb" })).unwrap_err().contains("Pfad"));
        assert!(request(&s, &json!({ "path": "/", "headers": { "x-api-key": "fremd" } })).unwrap_err().contains("nicht erlaubt"));
        assert!(request(&s, &json!({ "path": "/", "headers": { "Authorization": "x" } })).unwrap_err().contains("nicht erlaubt"));
    }

    #[test]
    fn only_wellformed_and_presented_fingerprints_are_accepted() {
        assert!(accept("h:1", "zz").is_err());
        assert!(accept("", &vec!["ab"; 32].join(":")).is_err());
        assert!(accept("192.0.2.9:443", &vec!["ab"; 32].join(":")).unwrap_err().contains("nicht vorgelegt"), "nicht vom Server vorgelegt");
    }
}
