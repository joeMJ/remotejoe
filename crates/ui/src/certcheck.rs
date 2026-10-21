//! „Schlüssel und Zertifikate prüfen“: Für viele Ziele den Server-Schlüssel bzw. das Zertifikat
//! abholen, ohne sich anzumelden, und als Liste zeigen (neu, bekannt, geändert, gültig, nicht erreichbar).
//! Nach Bestätigung werden nur die **neuen** als vertrauenswürdig gespeichert; geänderte nie
//! automatisch – die bleiben beim nächsten Verbinden einzeln zu prüfen.
//!
//! Gespeichert wird jeweils dort, wo das Verbinden später nachsieht:
//! SSH `known_hosts`, RDP der Zertifikatspeicher von FreeRDP (`freerdp/server/<host>_<port>.pem`),
//! VNC `vnc_certs`, Web `web-profile/<profil>/zertifikate.json`, Proxmox `proxmox-zertifikate.json`.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rj_core::Protocol;
use rj_ssh::known_hosts::{fingerprint, HostKeyStatus, KnownHosts};
use serde_json::{json, Value};
use sha2::Digest;
use uuid::Uuid;

/// Zeitlimit je Ziel für Verbindungsaufbau und Handshake.
const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
/// Höchstens so viele Ziele gleichzeitig.
const PARALLEL: usize = 8;

/// Was für ein Ziel geprüft wird.
#[derive(Debug, Clone)]
pub struct ProbeSpec {
    pub target: Uuid,
    pub name: String,
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub legacy: bool,
    /// Optionen wie an den Worker (für Web: Adresse, Profil, Zertifikatsprüfung abgeschaltet).
    pub options_json: String,
    /// Ziel wird nicht geprüft (Grund für die Liste), z. B. Verbindung über einen Tunnel.
    pub skip: Option<String>,
    /// Zusätzliche Prüfung (SSH auf Port 22 bei RDP- und PowerShell-Zielen, für „Inventar abfragen“):
    /// antwortet dort niemand, entfällt die Zeile.
    pub extra: bool,
}

/// Wo gespeichert und nachgesehen wird (im Betrieb der Datenordner; in Tests eigene Orte).
#[derive(Debug, Clone)]
pub struct Stores {
    pub known_hosts: Option<KnownHosts>,
    pub data_dir: Option<PathBuf>,
}

impl Stores {
    pub fn default_location() -> Stores {
        Stores { known_hosts: KnownHosts::default_location(), data_dir: rj_core::config::data_dir() }
    }

    fn file(&self, name: &str) -> Option<PathBuf> {
        self.data_dir.as_ref().map(|d| d.join(name))
    }
}

/// Zustand einer Zeile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckState {
    New,
    Known,
    Changed { previous: String },
    /// Öffentlich anerkanntes Zertifikat: Es kommt ohnehin keine Warnung.
    Valid,
    Unreachable(String),
    Skipped(String),
    /// Nach dem Übernehmen.
    Saved,
}

/// Was zum Speichern gebraucht wird.
#[derive(Debug, Clone)]
pub enum SaveData {
    None,
    /// SSH-Host-Schlüssel im OpenSSH-Format (`alg base64`).
    Ssh(String),
    /// RDP: PEM-Datei im Zertifikatspeicher von FreeRDP.
    Rdp { path: PathBuf, pem: String },
    /// VNC: Zeile `host:port fp` in `vnc_certs`.
    Vnc { id: String, fp: String },
    /// Web: angenommenes Zertifikat im Profil des Ziels.
    Web { profile: PathBuf, host: String, der: Vec<u8> },
    /// Proxmox: `host:port → fp` in `proxmox-zertifikate.json`.
    Proxmox { key: String, fp: String },
}

#[derive(Debug, Clone)]
pub struct CheckRow {
    pub spec: ProbeSpec,
    pub kind: &'static str,
    pub fingerprint: String,
    pub state: CheckState,
    pub save: SaveData,
}

impl CheckRow {
    pub fn json(&self) -> Value {
        let (state, detail) = match &self.state {
            CheckState::New => ("new", String::new()),
            CheckState::Known => ("known", String::new()),
            CheckState::Changed { previous } => ("changed", previous.clone()),
            CheckState::Valid => ("valid", String::new()),
            CheckState::Unreachable(e) => ("unreachable", e.clone()),
            CheckState::Skipped(e) => ("skipped", e.clone()),
            CheckState::Saved => ("saved", String::new()),
        };
        json!({
            "target": self.spec.target,
            "name": self.spec.name,
            "protocol": self.spec.protocol.as_str(),
            "address": format!("{}:{}", self.spec.host, self.spec.port),
            "kind": self.kind,
            "fingerprint": self.fingerprint,
            "state": state,
            "detail": detail,
        })
    }
}

/// Protokolle, die dieser Prüflauf kennt.
pub fn supported(p: Protocol) -> bool {
    matches!(p, Protocol::Ssh | Protocol::Rdp | Protocol::Vnc | Protocol::Web | Protocol::Proxmox)
}

fn kind_of(p: Protocol) -> &'static str {
    match p {
        Protocol::Ssh => "SSH-Host-Schlüssel",
        Protocol::Rdp => "RDP-Zertifikat",
        Protocol::Vnc => "VNC-Zertifikat (VeNCrypt)",
        Protocol::Web => "Web-Zertifikat",
        Protocol::Proxmox => "Proxmox-Zertifikat",
        _ => "Zertifikat",
    }
}

/// Prüft alle Ziele (läuft im Hintergrundfaden, blockiert bis zum Ende oder Abbruch).
pub fn run(rt: &tokio::runtime::Handle, specs: Vec<ProbeSpec>, cancel: Arc<std::sync::atomic::AtomicBool>, stores: Stores) -> Vec<CheckRow> {
    run_with_progress(rt, specs, cancel, stores, Arc::new(std::sync::atomic::AtomicUsize::new(0)))
}

/// Wie `run`; `done` zählt die fertig geprüften Ziele (Fortschritt „n von x“).
pub fn run_with_progress(
    rt: &tokio::runtime::Handle,
    specs: Vec<ProbeSpec>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    stores: Stores,
    done: Arc<std::sync::atomic::AtomicUsize>,
) -> Vec<CheckRow> {
    // Zertifizierungsstellen des Systems nur laden, wenn ein Web-Ziel dabei ist.
    let roots = specs.iter().any(|s| s.protocol == Protocol::Web && s.skip.is_none()).then(system_roots).flatten();
    rt.block_on(async move {
        let sem = Arc::new(tokio::sync::Semaphore::new(PARALLEL));
        let mut set = tokio::task::JoinSet::new();
        for (i, spec) in specs.into_iter().enumerate() {
            let (sem, cancel, stores, roots) = (sem.clone(), cancel.clone(), stores.clone(), roots.clone());
            set.spawn(async move {
                let _permit = sem.acquire_owned().await;
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    let kind = kind_of(spec.protocol);
                    return (i, row(spec, kind, String::new(), CheckState::Skipped("abgebrochen".into()), SaveData::None));
                }
                (i, probe(spec, stores, roots).await)
            });
        }
        let mut rows: Vec<(usize, CheckRow)> = Vec::new();
        while let Some(r) = set.join_next().await {
            done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if let Ok(r) = r {
                rows.push(r);
            }
        }
        rows.sort_by_key(|(i, _)| *i);
        rows.into_iter().map(|(_, r)| r).filter(|r| !(r.spec.extra && matches!(r.state, CheckState::Unreachable(_)))).collect()
    })
}

fn row(spec: ProbeSpec, kind: &'static str, fingerprint: String, state: CheckState, save: SaveData) -> CheckRow {
    CheckRow { spec, kind, fingerprint, state, save }
}

async fn probe(spec: ProbeSpec, stores: Stores, roots: Option<Arc<rustls::RootCertStore>>) -> CheckRow {
    let kind = kind_of(spec.protocol);
    if let Some(why) = spec.skip.clone() {
        return row(spec, kind, String::new(), CheckState::Skipped(why), SaveData::None);
    }
    if spec.protocol == Protocol::Ssh {
        return probe_ssh(spec, stores.known_hosts.as_ref()).await;
    }
    // Die übrigen Prüfungen arbeiten mit blockierenden Sockets.
    let s2 = spec.clone();
    match tokio::task::spawn_blocking(move || probe_blocking(s2, &stores, roots)).await {
        Ok(r) => r,
        Err(e) => row(spec, kind, String::new(), CheckState::Unreachable(e.to_string()), SaveData::None),
    }
}

async fn probe_ssh(spec: ProbeSpec, known: Option<&KnownHosts>) -> CheckRow {
    let kind = kind_of(spec.protocol);
    let key = match rj_ssh::client::probe_host_key(&spec.host, spec.port, PROBE_TIMEOUT, spec.legacy).await {
        Ok(k) => k,
        Err(e) => return row(spec, kind, String::new(), CheckState::Unreachable(e.to_string()), SaveData::None),
    };
    let fp = fingerprint(&key);
    let state = match known.map(|k| k.check(&spec.host, spec.port, &key)) {
        Some(HostKeyStatus::Known) => CheckState::Known,
        Some(HostKeyStatus::Changed { previous_fingerprint }) => CheckState::Changed { previous: previous_fingerprint },
        Some(HostKeyStatus::Unknown) | None => CheckState::New,
    };
    let openssh = key.to_openssh().unwrap_or_default();
    row(spec, kind, fp, state, SaveData::Ssh(openssh))
}

fn probe_blocking(spec: ProbeSpec, stores: &Stores, roots: Option<Arc<rustls::RootCertStore>>) -> CheckRow {
    let kind = kind_of(spec.protocol);
    let r = match spec.protocol {
        Protocol::Rdp => probe_rdp(&spec, stores),
        Protocol::Vnc => probe_vnc(&spec, stores),
        Protocol::Web => probe_web(&spec, stores, roots),
        Protocol::Proxmox => probe_proxmox(&spec, stores),
        _ => Err(Outcome::Skip("Protokoll wird nicht geprüft".into())),
    };
    match r {
        Ok((fp, state, save)) => row(spec, kind, fp, state, save),
        Err(Outcome::Skip(why)) => row(spec, kind, String::new(), CheckState::Skipped(why), SaveData::None),
        Err(Outcome::Fail(e)) => row(spec, kind, String::new(), CheckState::Unreachable(e), SaveData::None),
    }
}

/// Kein Ergebnis: nichts zu prüfen (`Skip`) oder Fehler beim Abholen (`Fail`).
enum Outcome {
    Skip(String),
    Fail(String),
}

type Probed = Result<(String, CheckState, SaveData), Outcome>;

fn fail(e: impl std::fmt::Display) -> Outcome {
    Outcome::Fail(e.to_string())
}

fn hex(der: &[u8], upper: bool) -> String {
    sha2::Sha256::digest(der).iter().map(|b| if upper { format!("{b:02X}") } else { format!("{b:02x}") }).collect::<Vec<_>>().join(":")
}

fn connect(host: &str, port: u16) -> Result<TcpStream, Outcome> {
    let addr = (host, port).to_socket_addrs().map_err(fail)?.next().ok_or_else(|| fail("Adresse nicht auflösbar"))?;
    let s = TcpStream::connect_timeout(&addr, PROBE_TIMEOUT).map_err(fail)?;
    s.set_read_timeout(Some(PROBE_TIMEOUT)).map_err(fail)?;
    s.set_write_timeout(Some(PROBE_TIMEOUT)).map_err(fail)?;
    Ok(s)
}

// ---------------------------------------------------------------- TLS

/// Prüfer, der nur mitschreibt: das Serverzertifikat und – mit `roots` – ob es öffentlich anerkannt ist.
/// Er lehnt immer ab und beendet so den Handshake, bevor irgendetwas anderes passiert.
struct Capture {
    provider: Arc<rustls::crypto::CryptoProvider>,
    roots: Option<Arc<rustls::client::WebPkiServerVerifier>>,
    seen: Arc<Mutex<Option<(Vec<u8>, bool)>>>,
}

impl std::fmt::Debug for Capture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Capture")
    }
}

impl rustls::client::danger::ServerCertVerifier for Capture {
    fn verify_server_cert(
        &self,
        end: &rustls::pki_types::CertificateDer<'_>,
        inter: &[rustls::pki_types::CertificateDer<'_>],
        name: &rustls::pki_types::ServerName<'_>,
        ocsp: &[u8],
        now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let trusted = self.roots.as_ref().is_some_and(|v| v.verify_server_cert(end, inter, name, ocsp, now).is_ok());
        *self.seen.lock().unwrap_or_else(|e| e.into_inner()) = Some((end.as_ref().to_vec(), trusted));
        Err(rustls::Error::General("nur Zertifikat abgeholt".into()))
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

fn system_roots() -> Option<Arc<rustls::RootCertStore>> {
    let mut roots = rustls::RootCertStore::empty();
    for c in rustls_native_certs::load_native_certs().certs {
        let _ = roots.add(c);
    }
    (!roots.is_empty()).then(|| Arc::new(roots))
}

/// TLS-Handshake auf einer offenen Verbindung bis zum Serverzertifikat. Gibt (DER, öffentlich anerkannt) zurück.
fn tls_certificate(mut sock: TcpStream, host: &str, roots: Option<Arc<rustls::RootCertStore>>) -> Result<(Vec<u8>, bool), Outcome> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let verifier = match roots {
        Some(r) => Some(rustls::client::WebPkiServerVerifier::builder_with_provider(r, provider.clone()).build().map_err(fail)?),
        None => None,
    };
    let seen: Arc<Mutex<Option<(Vec<u8>, bool)>>> = Arc::default();
    let capture = Arc::new(Capture { provider: provider.clone(), roots: verifier, seen: seen.clone() });
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(fail)?
        .dangerous()
        .with_custom_certificate_verifier(capture)
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(host.trim_start_matches('[').trim_end_matches(']').to_string()).map_err(fail)?;
    let mut conn = rustls::ClientConnection::new(Arc::new(config), name).map_err(fail)?;
    while conn.is_handshaking() {
        if let Err(e) = conn.complete_io(&mut sock) {
            if seen.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
                break;
            }
            return Err(fail(format!("TLS: {e}")));
        }
    }
    let got = seen.lock().unwrap_or_else(|e| e.into_inner()).take();
    got.ok_or_else(|| fail("kein Zertifikat erhalten"))
}

// ---------------------------------------------------------------- RDP

/// RDP: X.224-Verbindungsanfrage mit Wunsch nach TLS bzw. NLA, dann TLS-Handshake bis zum Zertifikat.
/// Es werden keine Zugangsdaten gesendet (NLA beginnt erst nach dem TLS-Handshake).
fn probe_rdp(spec: &ProbeSpec, stores: &Stores) -> Probed {
    let mut sock = connect(&spec.host, spec.port)?;
    // TPKT (4) + X.224 Connection Request (7) + RDP_NEG_REQ (8): requestedProtocols = SSL | HYBRID.
    let req: [u8; 19] = [0x03, 0x00, 0x00, 0x13, 0x0e, 0xe0, 0, 0, 0, 0, 0, 0x01, 0x00, 0x08, 0x00, 0x03, 0x00, 0x00, 0x00];
    sock.write_all(&req).map_err(fail)?;
    let mut head = [0u8; 4];
    sock.read_exact(&mut head).map_err(fail)?;
    if head[0] != 0x03 {
        return Err(fail("keine RDP-Antwort"));
    }
    let len = u16::from_be_bytes([head[2], head[3]]) as usize;
    if !(11..=512).contains(&len) {
        return Err(fail("unerwartete RDP-Antwort"));
    }
    let mut rest = vec![0u8; len - 4];
    sock.read_exact(&mut rest).map_err(fail)?;
    if rest.get(1).map(|c| c & 0xf0) != Some(0xd0) {
        return Err(fail("RDP-Verbindung abgelehnt"));
    }
    let neg = rest.get(7..15).ok_or_else(|| Outcome::Skip("Server ohne TLS (nur Standard-RDP-Sicherheit)".into()))?;
    let value = u32::from_le_bytes([neg[4], neg[5], neg[6], neg[7]]);
    match neg[0] {
        0x02 if value != 0 => {}
        0x02 => return Err(Outcome::Skip("Server ohne TLS (nur Standard-RDP-Sicherheit)".into())),
        0x03 => return Err(Outcome::Skip(format!("Server lehnt TLS ab (Code {value})"))),
        _ => return Err(fail("unerwartete RDP-Antwort")),
    }
    let (der, _) = tls_certificate(sock, &spec.host, None)?;
    let fp = hex(&der, false);
    let pem = pem_encode(&der);
    let path = rdp_cert_path(stores, &spec.host, spec.port).ok_or_else(|| fail("kein Datenordner"))?;
    let state = match std::fs::read_to_string(&path) {
        Ok(old) if normalize_pem(&old) == normalize_pem(&pem) => CheckState::Known,
        Ok(old) => CheckState::Changed { previous: pem_decode(&old).map(|d| hex(&d, false)).unwrap_or_else(|| "anderes Zertifikat gespeichert".into()) },
        Err(_) => CheckState::New,
    };
    Ok((fp, state, SaveData::Rdp { path, pem }))
}

/// Ort im Zertifikatspeicher von FreeRDP 3 (`<ConfigPath>/server/<host>_<port>.pem`, Host klein geschrieben).
fn rdp_cert_path(stores: &Stores, host: &str, port: u16) -> Option<PathBuf> {
    stores.file("freerdp").map(|d| d.join("server").join(format!("{}_{port}.pem", host.to_ascii_lowercase())))
}

fn normalize_pem(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// PEM wie OpenSSL bzw. FreeRDP es schreibt (64 Zeichen je Zeile, Zeilenende `\n`).
fn pem_encode(der: &[u8]) -> String {
    let mut b = String::with_capacity(der.len() * 4 / 3 + 4);
    for c in der.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                b.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                b.push('=');
            }
        }
    }
    let mut out = String::from("-----BEGIN CERTIFICATE-----\n");
    for line in b.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).unwrap_or_default());
        out.push('\n');
    }
    out.push_str("-----END CERTIFICATE-----\n");
    out
}

fn pem_decode(pem: &str) -> Option<Vec<u8>> {
    let body: String = pem.lines().filter(|l| !l.starts_with("-----")).collect::<String>().chars().filter(|c| !c.is_whitespace()).collect();
    let mut out = Vec::new();
    let mut acc = 0u32;
    let mut bits = 0;
    for ch in body.bytes().take_while(|b| *b != b'=') {
        let v = B64.iter().position(|b| *b == ch)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits & 0xff) as u8);
        }
    }
    Some(out)
}

// ---------------------------------------------------------------- VNC, Web, Proxmox

fn probe_vnc(spec: &ProbeSpec, stores: &Stores) -> Probed {
    let der = match rj_vnc::client::probe_certificate(&spec.host, spec.port, PROBE_TIMEOUT) {
        Ok(Some(d)) => d,
        Ok(None) => return Err(Outcome::Skip("kein VeNCrypt mit Zertifikat (nichts zu prüfen)".into())),
        Err(e) => return Err(fail(e)),
    };
    let fp = rj_vnc::client::cert_fingerprint(&der);
    let id = format!("{}:{}", spec.host, spec.port);
    let old = stores.file("vnc_certs").and_then(|p| rj_vnc::client::KnownCerts::new(p).lookup(&id));
    let state = match old {
        Some(o) if o == fp => CheckState::Known,
        Some(o) => CheckState::Changed { previous: o },
        None => CheckState::New,
    };
    Ok((fp.clone(), state, SaveData::Vnc { id, fp }))
}

fn probe_web(spec: &ProbeSpec, stores: &Stores, roots: Option<Arc<rustls::RootCertStore>>) -> Probed {
    let opts: rj_web::WebOptions = serde_json::from_str(&spec.options_json).unwrap_or_default();
    let url = rj_web::start_url(&opts, &spec.host, spec.port).map_err(fail)?;
    if !url.https {
        return Err(Outcome::Skip("unverschlüsselt (HTTP), kein Zertifikat".into()));
    }
    if opts.ignore_certificate {
        return Err(Outcome::Skip("Zertifikatsprüfung bei diesem Ziel abgeschaltet".into()));
    }
    let sock = connect(&url.host, url.port)?;
    let (der, trusted) = tls_certificate(sock, &url.host, roots)?;
    let fp = rj_web::AcceptedCerts::fingerprint(&der);
    if trusted {
        return Ok((fp, CheckState::Valid, SaveData::None));
    }
    let base = stores.data_dir.clone().ok_or_else(|| fail("kein Datenordner"))?;
    let key = opts.profile.clone().unwrap_or_else(|| format!("{}-{}", url.host, url.port));
    let profile = rj_web::profile_dir(&base, &key);
    let key = rj_web::AcceptedCerts::key(&url.host, url.port);
    let state = match rj_web::AcceptedCerts::load(&profile).decide(&key, &der, &["?".to_string()]) {
        rj_web::CertDecision::Accepted => CheckState::Known,
        rj_web::CertDecision::Ask { previous: Some(o), .. } => CheckState::Changed { previous: o },
        _ => CheckState::New,
    };
    Ok((fp, state, SaveData::Web { profile, host: key, der }))
}

fn proxmox_known(stores: &Stores) -> BTreeMap<String, String> {
    stores.file("proxmox-zertifikate.json").and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn probe_proxmox(spec: &ProbeSpec, stores: &Stores) -> Probed {
    let sock = connect(&spec.host, spec.port)?;
    let (der, _) = tls_certificate(sock, &spec.host, None)?;
    let fp = rj_proxmox::net::fingerprint(&der);
    let key = format!("{}:{}", spec.host, spec.port);
    let state = match proxmox_known(stores).get(&key) {
        Some(o) if *o == fp => CheckState::Known,
        Some(o) => CheckState::Changed { previous: o.clone() },
        None => CheckState::New,
    };
    Ok((fp.clone(), state, SaveData::Proxmox { key, fp }))
}

// ---------------------------------------------------------------- Speichern

/// Speichert alle neuen Zeilen. Gibt (gespeichert, Fehlermeldungen) zurück; gespeicherte Zeilen
/// stehen danach auf `Saved`. Geänderte, bekannte, gültige und nicht erreichbare bleiben unberührt.
pub fn accept_new(rows: &mut [CheckRow], stores: &Stores) -> (usize, Vec<String>) {
    // Je Speicherdatei sammeln und einmal schreiben: Hunderte Einträge einzeln zu schreiben (lesen,
    // sichern, auf die Platte erzwingen) dauert unter Windows Minuten und blockiert die Oberfläche.
    let mut ssh: Vec<(usize, (String, u16, String))> = Vec::new();
    let mut vnc: Vec<(usize, (String, String))> = Vec::new();
    let mut web: BTreeMap<PathBuf, Vec<(usize, String, Vec<u8>)>> = BTreeMap::new();
    let mut prox: Vec<(usize, String, String)> = Vec::new();
    let mut done: Vec<(usize, Result<(), String>)> = Vec::new();
    for (i, r) in rows.iter().enumerate().filter(|(_, r)| r.state == CheckState::New) {
        match &r.save {
            SaveData::None => {}
            SaveData::Ssh(line) => ssh.push((i, (r.spec.host.clone(), r.spec.port, line.clone()))),
            SaveData::Rdp { path, pem } => done.push((i, save_file(path, pem.as_bytes()))),
            SaveData::Vnc { id, fp } => vnc.push((i, (id.clone(), fp.clone()))),
            SaveData::Web { profile, host, der } => web.entry(profile.clone()).or_default().push((i, host.clone(), der.clone())),
            SaveData::Proxmox { key, fp } => prox.push((i, key.clone(), fp.clone())),
        }
    }
    if !ssh.is_empty() {
        let res = match &stores.known_hosts {
            Some(k) => k.remember_openssh_many(&ssh.iter().map(|(_, e)| e.clone()).collect::<Vec<_>>()).map_err(|e| e.to_string()),
            None => Err("kein Datenordner".into()),
        };
        done.extend(ssh.iter().map(|(i, _)| (*i, res.clone())));
    }
    if !vnc.is_empty() {
        let res = match stores.file("vnc_certs") {
            Some(p) => rj_vnc::client::KnownCerts::new(p).remember_many(&vnc.iter().map(|(_, e)| e.clone()).collect::<Vec<_>>()).map_err(|e| e.to_string()),
            None => Err("kein Datenordner".into()),
        };
        done.extend(vnc.iter().map(|(i, _)| (*i, res.clone())));
    }
    for (profile, list) in web {
        let mut a = rj_web::AcceptedCerts::load(&profile);
        for (_, host, der) in &list {
            a.accept(host, der);
        }
        let res = a.save(&profile).map_err(|e| e.to_string());
        done.extend(list.iter().map(|(i, ..)| (*i, res.clone())));
    }
    if !prox.is_empty() {
        let res = match stores.file("proxmox-zertifikate.json") {
            Some(p) => {
                let mut k = proxmox_known(stores);
                for (_, key, fp) in &prox {
                    k.insert(key.clone(), fp.clone());
                }
                save_file(&p, &serde_json::to_vec_pretty(&k).unwrap_or_default())
            }
            None => Err("kein Datenordner".into()),
        };
        done.extend(prox.iter().map(|(i, ..)| (*i, res.clone())));
    }
    let mut saved = 0;
    let mut errors = Vec::new();
    done.sort_by_key(|(i, _)| *i);
    for (i, res) in done {
        match res {
            Ok(()) => {
                rows[i].state = CheckState::Saved;
                saved += 1;
            }
            Err(e) => errors.push(format!("{}: {e}", rows[i].spec.name)),
        }
    }
    (saved, errors)
}

fn save_file(path: &Path, data: &[u8]) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    rj_core::fsutil::atomic_write_with_backup(path, data).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(protocol: Protocol, host: &str, port: u16) -> ProbeSpec {
        ProbeSpec { target: Uuid::new_v4(), name: "t".into(), protocol, host: host.into(), port, legacy: false, options_json: String::new(), skip: None, extra: false }
    }

    fn stores(dir: &Path) -> Stores {
        Stores { known_hosts: Some(KnownHosts::new(dir.join("known_hosts"))), data_dir: Some(dir.to_path_buf()) }
    }

    #[test]
    fn accepting_hundreds_of_new_keys_writes_each_file_once_and_fast() {
        let dir = tempfile::tempdir().unwrap();
        let s = stores(dir.path());
        let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIKyAnCq2Gk7BPfqYFQiNcGYgWfm4W8KibIapJ/jZJJGt";
        let mut rows: Vec<CheckRow> = (0..600)
            .map(|i| row(spec(Protocol::Ssh, &format!("10.0.{}.{}", i / 250, i % 250), 22), "SSH-Host-Schlüssel", "fp".into(), CheckState::New, SaveData::Ssh(key.into())))
            .collect();
        rows.extend((0..300).map(|i| {
            row(spec(Protocol::Vnc, &format!("10.1.0.{i}"), 5900), "VNC", "fp".into(), CheckState::New, SaveData::Vnc { id: format!("10.1.0.{i}:5900"), fp: format!("aa:{i:02x}") })
        }));
        let profile = dir.path().join("web-profile/x");
        rows.extend((0..300).map(|i| {
            row(spec(Protocol::Web, &format!("10.2.0.{i}"), 443), "Web", "fp".into(), CheckState::New, SaveData::Web { profile: profile.clone(), host: format!("10.2.0.{i}:443"), der: vec![i as u8; 8] })
        }));
        let t = std::time::Instant::now();
        let (saved, errors) = accept_new(&mut rows, &s);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(saved, 1200);
        assert!(t.elapsed() < std::time::Duration::from_secs(5), "zu langsam: {:?}", t.elapsed());
        assert!(rows.iter().all(|r| r.state == CheckState::Saved));
        let kh = std::fs::read_to_string(dir.path().join("known_hosts")).unwrap();
        assert_eq!(kh.lines().count(), 600);
        assert_eq!(std::fs::read_to_string(dir.path().join("vnc_certs")).unwrap().lines().count(), 300);
        // Erneut übernehmen ändert nichts doppelt.
        let mut again: Vec<CheckRow> = (0..3).map(|i| row(spec(Protocol::Ssh, &format!("10.0.0.{i}"), 22), "SSH", "fp".into(), CheckState::New, SaveData::Ssh(key.into()))).collect();
        accept_new(&mut again, &s);
        assert_eq!(std::fs::read_to_string(dir.path().join("known_hosts")).unwrap().lines().count(), 600);
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
    }

    #[test]
    fn closed_port_is_unreachable_and_skips_are_reported() {
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut tunnel = spec(Protocol::Ssh, "10.0.0.1", 22);
        tunnel.skip = Some("über Tunnel".into());
        let mut http = spec(Protocol::Web, "127.0.0.1", 80);
        http.options_json = r#"{"url":"http://127.0.0.1:80/"}"#.into();
        let rows = run(rt().handle(), vec![spec(Protocol::Ssh, "127.0.0.1", closed), tunnel, spec(Protocol::Rdp, "127.0.0.1", closed), http], Default::default(), Stores { known_hosts: None, data_dir: None });
        assert_eq!(rows.len(), 4);
        assert!(matches!(rows[0].state, CheckState::Unreachable(_)), "{:?}", rows[0].state);
        assert_eq!(rows[1].state, CheckState::Skipped("über Tunnel".into()));
        assert_eq!(rows[1].json()["state"], "skipped");
        assert!(matches!(rows[2].state, CheckState::Unreachable(_)), "{:?}", rows[2].state);
        assert!(matches!(&rows[3].state, CheckState::Skipped(w) if w.contains("HTTP")), "{:?}", rows[3].state);
    }

    #[test]
    fn extra_ssh_rows_only_when_someone_answers() {
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut extra = spec(Protocol::Ssh, "127.0.0.1", closed);
        extra.extra = true;
        let rows = run(rt().handle(), vec![spec(Protocol::Rdp, "127.0.0.1", closed), extra], Default::default(), Stores { known_hosts: None, data_dir: None });
        assert_eq!(rows.len(), 1, "zusätzliche SSH-Zeile ohne Antwort entfällt");
        assert_eq!(rows[0].spec.protocol, Protocol::Rdp);
    }

    #[test]
    fn only_new_rows_are_saved() {
        let dir = tempfile::tempdir().unwrap();
        let s = stores(dir.path());
        let mut rows = vec![
            row(spec(Protocol::Ssh, "a", 22), "x", "fp".into(), CheckState::Changed { previous: "alt".into() }, SaveData::Ssh("x".into())),
            row(spec(Protocol::Proxmox, "b", 8006), "x", "fp".into(), CheckState::Known, SaveData::Proxmox { key: "b:8006".into(), fp: "fp".into() }),
            row(spec(Protocol::Proxmox, "c", 8006), "x", "fp-c".into(), CheckState::New, SaveData::Proxmox { key: "c:8006".into(), fp: "fp-c".into() }),
        ];
        let (n, errors) = accept_new(&mut rows, &s);
        assert_eq!((n, errors.len()), (1, 0));
        assert!(matches!(rows[0].state, CheckState::Changed { .. }));
        assert_eq!(rows[2].state, CheckState::Saved);
        assert_eq!(proxmox_known(&s).get("c:8006").map(String::as_str), Some("fp-c"));
        assert!(proxmox_known(&s).get("b:8006").is_none());
    }

    /// Das PEM muss Zeichen für Zeichen dem von OpenSSL (und damit FreeRDP) entsprechen.
    #[test]
    fn pem_matches_openssl_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let (key, cert) = (dir.path().join("k.pem"), dir.path().join("c.pem"));
        let ok = std::process::Command::new("openssl")
            .args(["req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-days", "1", "-subj", "/CN=probe.example.test", "-keyout"])
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            eprintln!("ÜBERSPRUNGEN: openssl fehlt");
            return;
        }
        // openssl aus Git für Windows (MSYS) schreibt CRLF; verglichen wird der Inhalt.
        let text = std::fs::read_to_string(&cert).unwrap().replace("\r\n", "\n");
        let der = pem_decode(&text).unwrap();
        assert_eq!(pem_encode(&der), text);
        for n in [1usize, 2, 3, 4, 5, 47, 48, 49] {
            let d: Vec<u8> = (0..n as u8).collect();
            assert_eq!(pem_decode(&pem_encode(&d)).unwrap(), d, "Länge {n}");
        }
    }

    /// Echter `sshd` als normaler Benutzer (nur Schlüsselaustausch, keine Anmeldung nötig).
    struct UserSshd {
        child: std::process::Child,
        port: u16,
        fingerprint: String,
        _dir: tempfile::TempDir,
    }

    impl Drop for UserSshd {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn user_sshd() -> Option<UserSshd> {
        if !Path::new("/usr/sbin/sshd").exists() {
            eprintln!("ÜBERSPRUNGEN: /usr/sbin/sshd fehlt");
            return None;
        }
        let dir = tempfile::tempdir().ok()?;
        let key = dir.path().join("hostkey");
        let ok = std::process::Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(&key).status().ok()?.success();
        if !ok {
            return None;
        }
        let port = std::net::TcpListener::bind("127.0.0.1:0").ok()?.local_addr().ok()?.port();
        let cfg = dir.path().join("cfg");
        std::fs::write(&cfg, format!("Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nPidFile none\nUsePAM no\n", key.display())).ok()?;
        let child = std::process::Command::new("/usr/sbin/sshd").args(["-D", "-e", "-f"]).arg(&cfg).stderr(std::process::Stdio::null()).spawn().ok()?;
        // Fingerabdruck wie OpenSSH ihn zeigt („256 SHA256:… Kommentar“).
        let out = std::process::Command::new("ssh-keygen").arg("-lf").arg(dir.path().join("hostkey.pub")).output().ok()?;
        let fp = String::from_utf8_lossy(&out.stdout).split_whitespace().nth(1)?.to_string();
        let s = UserSshd { child, port, fingerprint: fp, _dir: dir };
        for _ in 0..50 {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return Some(s);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!("ÜBERSPRUNGEN: sshd als Benutzer startet nicht");
        None
    }

    #[test]
    fn ssh_key_is_new_then_saved_then_known_and_a_different_key_is_changed() {
        let Some(sshd) = user_sshd() else { return };
        let rt = rt();
        let dir = tempfile::tempdir().unwrap();
        let s = stores(dir.path());
        let specs = || vec![spec(Protocol::Ssh, "127.0.0.1", sshd.port)];

        let mut rows = run(rt.handle(), specs(), Default::default(), s.clone());
        assert_eq!(rows[0].state, CheckState::New, "{:?}", rows[0].state);
        assert_eq!(rows[0].fingerprint, sshd.fingerprint);
        let (n, errors) = accept_new(&mut rows, &s);
        assert_eq!((n, errors.len()), (1, 0));
        assert_eq!(rows[0].state, CheckState::Saved);

        let rows = run(rt.handle(), specs(), Default::default(), s.clone());
        assert_eq!(rows[0].state, CheckState::Known);

        // Anderer Schlüssel unter derselben Adresse gespeichert: „geändert“, und Übernehmen lässt ihn stehen.
        let other_dir = tempfile::tempdir().unwrap();
        let s2 = stores(other_dir.path());
        s2.known_hosts.as_ref().unwrap().remember_openssh("127.0.0.1", sshd.port, "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGq6lxXqVOFtYi0gm3VYw6c1BZrfF1t6wEXeL1qD5xQ9").unwrap();
        let before = std::fs::read(other_dir.path().join("known_hosts")).unwrap();
        let mut rows = run(rt.handle(), specs(), Default::default(), s2.clone());
        assert!(matches!(rows[0].state, CheckState::Changed { .. }), "{:?}", rows[0].state);
        let (n, _) = accept_new(&mut rows, &s2);
        assert_eq!(n, 0);
        assert_eq!(std::fs::read(other_dir.path().join("known_hosts")).unwrap(), before);
    }

    /// TLS-Server mit selbst ausgestelltem Zertifikat (`openssl s_server`), für Web und Proxmox.
    fn tls_server(dir: &Path) -> Option<(std::process::Child, u16, String)> {
        let (key, cert) = (dir.join("k.pem"), dir.join("c.pem"));
        let ok = std::process::Command::new("openssl")
            .args(["req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-days", "1", "-subj", "/CN=localhost", "-keyout"])
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            return None;
        }
        let port = std::net::TcpListener::bind("127.0.0.1:0").ok()?.local_addr().ok()?.port();
        let child = std::process::Command::new("openssl")
            .args(["s_server", "-quiet", "-accept"])
            .arg(format!("127.0.0.1:{port}"))
            .arg("-cert")
            .arg(&cert)
            .arg("-key")
            .arg(&key)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let der = pem_decode(&std::fs::read_to_string(&cert).ok()?)?;
        for _ in 0..50 {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return Some((child, port, hex(&der, false)));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }

    #[test]
    fn self_signed_web_and_proxmox_certificates_are_new_then_known() {
        let dir = tempfile::tempdir().unwrap();
        let Some((mut child, port, fp)) = tls_server(dir.path()) else {
            eprintln!("ÜBERSPRUNGEN: openssl fehlt");
            return;
        };
        let rt = rt();
        let data = tempfile::tempdir().unwrap();
        let s = stores(data.path());
        let mut web = spec(Protocol::Web, "localhost", port);
        web.options_json = format!(r#"{{"url":"https://localhost:{port}/","profile":"ziel-1"}}"#);
        let px = spec(Protocol::Proxmox, "127.0.0.1", port);
        let specs = || vec![web.clone(), px.clone()];
        let mut rows = run(rt.handle(), specs(), Default::default(), s.clone());
        // Windows: „localhost“ löst zuerst zu ::1 auf, openssl s_server lauscht nur auf 127.0.0.1, und die Prüfung
        // versucht nur die erste Adresse. Dann ist der Prüfserver für das Web-Ziel nicht erreichbar.
        if cfg!(windows) && matches!(rows[0].state, CheckState::Unreachable(_)) {
            eprintln!("ÜBERSPRUNGEN (Windows): Prüfserver über „localhost“ nicht erreichbar: {:?}", rows[0].state);
            let _ = child.kill();
            let _ = child.wait();
            return;
        }
        assert_eq!(rows[0].state, CheckState::New, "{:?}", rows[0].state);
        assert_eq!(rows[0].fingerprint, fp.to_uppercase());
        assert_eq!(rows[1].state, CheckState::New, "{:?}", rows[1].state);
        assert_eq!(rows[1].fingerprint, fp);
        assert_eq!(accept_new(&mut rows, &s).0, 2);
        // Gespeichert genau dort, wo der Web-Worker und die Proxmox-Anbindung nachsehen.
        let saved = rj_web::AcceptedCerts::load(&rj_web::profile_dir(data.path(), "ziel-1"));
        assert_eq!(saved.hosts.get(&rj_web::AcceptedCerts::key("localhost", port)), Some(&fp.to_uppercase()), "Schlüssel mit Port");
        assert_eq!(proxmox_known(&s).get(&format!("127.0.0.1:{port}")), Some(&fp));
        let rows = run(rt.handle(), specs(), Default::default(), s.clone());
        assert_eq!((rows[0].state.clone(), rows[1].state.clone()), (CheckState::Known, CheckState::Known));
        let _ = child.kill();
        let _ = child.wait();
    }

    /// Nachahmer eines RDP- bzw. VNC-Servers (Python): beantwortet die Begrüßung, dann TLS mit Testzertifikat.
    const FAKE: &str = r#"
import socket, ssl, struct, sys
mode, port, cert, key = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4]
srv = socket.socket(); srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", port)); srv.listen(4)
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER); ctx.load_cert_chain(cert, key)
def exact(c, n):
    b = b""
    while len(b) < n:
        d = c.recv(n - len(b))
        if not d: raise EOFError
        b += d
    return b
while True:
    c, _ = srv.accept()
    try:
        if mode == "rdp":
            exact(c, 19)
            # X.224 Connection Confirm + RDP_NEG_RSP (PROTOCOL_HYBRID)
            c.sendall(bytes([3,0,0,19, 14, 0xd0, 0,0, 0,0, 0, 2,0, 8,0, 2,0,0,0]))
        else:
            c.sendall(b"RFB 003.008\n"); exact(c, 12)
            c.sendall(bytes([1, 19])); exact(c, 1)
            c.sendall(bytes([0, 2])); exact(c, 2)
            c.sendall(bytes([0, 1]) + struct.pack(">I", 260)); exact(c, 4)
            c.sendall(bytes([1]))
        t = ctx.wrap_socket(c, server_side=True)
        t.close()
    except Exception:
        c.close()
"#;

    fn fake_server(mode: &str, dir: &Path) -> Option<(std::process::Child, u16, Vec<u8>)> {
        let (key, cert) = (dir.join("k.pem"), dir.join("c.pem"));
        let ok = std::process::Command::new("openssl")
            .args(["req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-days", "1", "-subj", "/CN=localhost", "-keyout"])
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            return None;
        }
        let script = dir.join("fake.py");
        std::fs::write(&script, FAKE).ok()?;
        let port = std::net::TcpListener::bind("127.0.0.1:0").ok()?.local_addr().ok()?.port();
        let child = std::process::Command::new("python3")
            .arg("-I")
            .arg(&script)
            .arg(mode)
            .arg(port.to_string())
            .arg(&cert)
            .arg(&key)
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let der = pem_decode(&std::fs::read_to_string(&cert).ok()?)?;
        for _ in 0..50 {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return Some((child, port, der));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }

    #[test]
    fn rdp_certificate_is_fetched_without_login_and_stored_like_freerdp() {
        let dir = tempfile::tempdir().unwrap();
        let Some((mut child, port, der)) = fake_server("rdp", dir.path()) else {
            eprintln!("ÜBERSPRUNGEN: openssl oder python3 fehlt");
            return;
        };
        let data = tempfile::tempdir().unwrap();
        let s = stores(data.path());
        let rt = rt();
        let mut rows = run(rt.handle(), vec![spec(Protocol::Rdp, "127.0.0.1", port)], Default::default(), s.clone());
        assert_eq!(rows[0].state, CheckState::New, "{:?}", rows[0].state);
        assert_eq!(rows[0].fingerprint, hex(&der, false));
        assert_eq!(accept_new(&mut rows, &s).0, 1);
        let file = data.path().join("freerdp").join("server").join(format!("127.0.0.1_{port}.pem"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), std::fs::read_to_string(dir.path().join("c.pem")).unwrap());
        let rows = run(rt.handle(), vec![spec(Protocol::Rdp, "127.0.0.1", port)], Default::default(), s.clone());
        assert_eq!(rows[0].state, CheckState::Known);
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn vnc_vencrypt_certificate_is_fetched_and_stored_in_vnc_certs() {
        let dir = tempfile::tempdir().unwrap();
        let Some((mut child, port, der)) = fake_server("vnc", dir.path()) else {
            eprintln!("ÜBERSPRUNGEN: openssl oder python3 fehlt");
            return;
        };
        let data = tempfile::tempdir().unwrap();
        let s = stores(data.path());
        let rt = rt();
        let mut rows = run(rt.handle(), vec![spec(Protocol::Vnc, "127.0.0.1", port)], Default::default(), s.clone());
        assert_eq!(rows[0].state, CheckState::New, "{:?}", rows[0].state);
        assert_eq!(rows[0].fingerprint, hex(&der, false));
        assert_eq!(accept_new(&mut rows, &s).0, 1);
        let known = rj_vnc::client::KnownCerts::new(data.path().join("vnc_certs"));
        assert_eq!(known.lookup(&format!("127.0.0.1:{port}")), Some(hex(&der, false)));
        let rows = run(rt.handle(), vec![spec(Protocol::Vnc, "127.0.0.1", port)], Default::default(), s.clone());
        assert_eq!(rows[0].state, CheckState::Known);
        let _ = child.kill();
        let _ = child.wait();
    }
}
