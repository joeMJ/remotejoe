//! Ende-zu-Ende mit echtem WebView2 (Windows): Worker-Prozess gegen lokale Prüfseiten, ohne Oberfläche
//! (`headless`: der Worker legt ein unsichtbares Elternfenster an) und eingebettet in ein fremdes Fenster dieses
//! Testprozesses. Geprüft werden Laden und Zustand, Formular-Anmeldung, Zertifikatsfrage, Profil je Ziel, Downloads
//! (nur .ica wird geöffnet), Kurzbefehle und Einbetten in ein Fenster eines anderen Prozesses.
//! Nur mit Merkmal `webview`; ohne WebView2-Laufzeit werden die Tests übersprungen.
#![cfg(all(windows, feature = "webview"))]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use rj_ipc::*;
use rj_supervisor::*;

/// Kennwort mit Umlaut, Euro, Anführungszeichen, Apostroph, Gegenschrägstrich und Emoji.
const PASSWORD: &str = "Prüf-\"Kennwort\"&ä€'\\ 🚀";
const LOGIN: &str = "<html><head><meta charset='utf-8'><title>Anmeldung</title></head><body><form method='post' action='/login'>\
<input id='user' name='user'><input id='pw' name='pw' type='password'>\
<button id='go' type='submit'>Anmelden</button></form><a id='weiter' href='/zwei'>weiter</a></body></html>";

/// Antwort des Prüfservers: Inhaltsart, zusätzliche Kopfzeilen, Rumpf.
struct Reply {
    ctype: &'static str,
    headers: String,
    body: Vec<u8>,
}

fn html(s: &str) -> Reply {
    Reply { ctype: "text/html; charset=utf-8", headers: String::new(), body: s.as_bytes().to_vec() }
}

fn reply_for(path: &str) -> Reply {
    match path {
        "/login" => html("<html><head><meta charset='utf-8'><title>Angemeldet</title></head><body>angemeldet</body></html>"),
        "/zwei" => html("<html><head><meta charset='utf-8'><title>Seite zwei</title></head><body><p>zwei</p></body></html>"),
        "/titel" => html("<html><head><meta charset='utf-8'><title>Prüfseite ä</title></head><body>titel</body></html>"),
        // Neues Fenster: der Link mit target=_blank muss im selben Tab öffnen.
        "/neu" => html("<html><head><meta charset='utf-8'><title>Ausgang</title></head><body><a id='n' target='_blank' href='/zwei'>neu</a></body></html>"),
        // Citrix-Startdatei (Attrappe): wird heruntergeladen und danach geöffnet.
        "/start.ica" => Reply {
            ctype: "application/x-ica",
            headers: "Content-Disposition: attachment; filename=\"start.ica\"\r\n".into(),
            body: b"[Encoding]\r\nInputEncoding=UTF8\r\n[ApplicationServers]\r\nTest=\r\n".to_vec(),
        },
        // Andere Datei: wird heruntergeladen, aber nicht geöffnet.
        "/daten.bin" => Reply {
            ctype: "application/octet-stream",
            headers: "Content-Disposition: attachment; filename=\"daten.bin\"\r\n".into(),
            body: vec![7u8; 5000],
        },
        // Name mit Umkehr der Schreibrichtung: darf keine andere Endung vortäuschen.
        "/falsch" => Reply {
            ctype: "application/octet-stream",
            headers: "Content-Disposition: attachment; filename*=UTF-8''a%E2%80%AEgpj.dat\r\n".into(),
            body: vec![1u8; 10],
        },
        p if p.starts_with("/ev") => html(""),
        _ => html(LOGIN),
    }
}

fn serve(l: TcpListener, tx: mpsc::Sender<(String, String)>, tls: Option<Arc<rustls::ServerConfig>>) {
    for s in l.incoming().flatten() {
        let tx = tx.clone();
        let tls = tls.clone();
        std::thread::spawn(move || match tls {
            None => handle(s, &tx),
            Some(cfg) => {
                let Ok(conn) = rustls::ServerConnection::new(cfg) else { return };
                handle(rustls::StreamOwned::new(conn, s), &tx);
            }
        });
    }
}

fn handle<S: Read + Write>(s: S, tx: &mpsc::Sender<(String, String)>) {
    let mut r = BufReader::new(s);
    let mut first = String::new();
    if r.read_line(&mut first).is_err() {
        return;
    }
    let mut len = 0;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).unwrap_or(0) == 0 {
            break;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; len];
    let _ = r.read_exact(&mut body);
    let path = first.split_whitespace().nth(1).unwrap_or("").to_string();
    let rep = reply_for(&path);
    let s = r.get_mut();
    let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: {}\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n", rep.ctype, rep.headers, rep.body.len());
    let _ = s.write_all(&rep.body);
    let _ = s.flush();
    let _ = tx.send((path, String::from_utf8_lossy(&body).into_owned()));
}

fn http_server() -> (u16, mpsc::Receiver<(String, String)>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || serve(l, tx, None));
    (port, rx)
}

fn form_decode(s: &str) -> Vec<(String, String)> {
    s.split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), String::from_utf8(percent_decode(&v.replace('+', " "))).unwrap()))
        .collect()
}

fn percent_decode(v: &str) -> Vec<u8> {
    let b = v.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            out.push(u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap(), 16).unwrap());
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// Eine Web-Sitzung über Supervisor und Worker-Prozess.
struct Sess {
    sup: Supervisor,
    id: WorkerId,
    pid: u32,
    custom: Vec<serde_json::Value>,
    log: Vec<String>,
    questions: usize,
    accept: bool,
    connected: bool,
}

fn worker_spec(home: &Path, extra_env: &[(&str, String)]) -> WorkerSpec {
    let mut spec = WorkerSpec::new(env!("CARGO_BIN_EXE_rj-worker-web"));
    // Der Worker lädt sehr viel Adressraum; keine Speichergrenze.
    spec.memory_limit = None;
    spec.env = vec![("REMOTEJOE_HOME".into(), home.display().to_string())];
    spec.env.extend(extra_env.iter().map(|(k, v)| (k.to_string(), v.clone())));
    spec
}

impl Sess {
    fn start(home: &Path, extra_env: &[(&str, String)], opts: serde_json::Value, accept: bool) -> Sess {
        let mut sup = Supervisor::new();
        let (id, pid) = sup.spawn(worker_spec(home, extra_env)).unwrap();
        sup.send(id, &ToWorker::Connect(ConnectRequest {
            host: "127.0.0.1".into(),
            port: 0,
            width: 800,
            height: 600,
            options_json: opts.to_string(),
            connect_timeout_secs: 15,
            keepalive_secs: 30,
        }))
        .unwrap();
        sup.send(id, &ToWorker::Credentials(CredentialPayload {
            username: "admin".into(),
            domain: None,
            password: SecretBytes(PASSWORD.as_bytes().to_vec()),
            private_key: None,
        }))
        .unwrap();
        Sess { sup, id, pid, custom: Vec::new(), log: Vec::new(), questions: 0, accept, connected: false }
    }

    fn command(&mut self, json: serde_json::Value) {
        self.sup.send(self.id, &ToWorker::Custom(json.to_string().into_bytes())).unwrap();
    }

    fn pump(&mut self, wait: Duration) {
        for ev in self.sup.tick(wait) {
            match ev {
                SupervisorEvent::Message { msg, .. } => match msg {
                    FromWorker::Custom(b) => self.custom.push(serde_json::from_slice(&b).expect("JSON vom Worker")),
                    FromWorker::Question { id: q, text, .. } => {
                        self.questions += 1;
                        self.log.push(text);
                        let accept = self.accept;
                        self.sup.send(self.id, &ToWorker::Answer { question: q, accept }).unwrap();
                    }
                    FromWorker::State(SessionState::Connected) => self.connected = true,
                    FromWorker::State(SessionState::Failed { reason }) => panic!("Sitzung fehlgeschlagen: {reason}"),
                    other => self.log.push(format!("{other:?}")),
                },
                SupervisorEvent::Exited { reason, .. } => panic!("Worker beendet: {reason:?}; {:?}", self.log),
                other => self.log.push(format!("{other:?}")),
            }
        }
    }

    fn wait(&mut self, secs: u64, mut cond: impl FnMut(&Sess) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < deadline {
            self.pump(Duration::from_millis(20));
            if cond(self) {
                return true;
            }
        }
        false
    }

    /// Letzter Zustand `status` mit dem Titel.
    fn status_title(&self) -> Option<String> {
        self.custom.iter().rev().find(|c| c["ev"] == "status").and_then(|c| c["title"].as_str().map(String::from))
    }

    fn loaded(&self, title: &str) -> bool {
        self.custom.iter().rev().find(|c| c["ev"] == "status").is_some_and(|c| c["title"] == title && c["loading"] == false)
    }

    fn downloads(&self) -> Vec<&serde_json::Value> {
        self.custom.iter().filter(|c| c["ev"] == "download").collect()
    }
}

impl Drop for Sess {
    fn drop(&mut self) {
        self.sup.shutdown();
    }
}

/// Ist die WebView2-Laufzeit vorhanden? Sonst lassen sich die Tests nicht ausführen.
fn runtime_available() -> bool {
    let ok = ["C:\\Program Files (x86)\\Microsoft\\EdgeWebView\\Application", "C:\\Program Files\\Microsoft\\EdgeWebView\\Application"].iter().any(|p| Path::new(p).is_dir());
    if !ok {
        eprintln!("NICHT AUSGEFÜHRT: WebView2-Laufzeit fehlt");
    }
    ok
}

fn opts(url: &str) -> serde_json::Value {
    serde_json::json!({ "url": url, "allow_http": true, "headless": true, "profile": "6f1c0000-0000-4000-8000-0000000000aa" })
}

fn opts_login(url: &str) -> serde_json::Value {
    let mut o = opts(url);
    o["login"] = serde_json::json!({"username_selector": "#user", "password_selector": "#pw", "submit_selector": "#go"});
    o
}

fn profile_of(home: &Path) -> PathBuf {
    home.join("daten/web-profile/6f1c0000-0000-4000-8000-0000000000aa")
}

#[test]
fn page_status_title_and_navigation() {
    if !runtime_available() {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let (port, _rx) = http_server();
    let mut s = Sess::start(home.path(), &[], opts(&format!("http://127.0.0.1:{port}/titel")), true);
    assert!(s.wait(60, |s| s.loaded("Prüfseite ä")), "Titel nicht gemeldet: {:?} {:?}", s.custom, s.log);
    assert!(s.connected, "Zustand „verbunden“ fehlt");
    let st = s.custom.iter().rev().find(|c| c["ev"] == "status").unwrap().clone();
    assert!(st["uri"].as_str().unwrap().ends_with("/titel"), "{st}");
    assert_eq!((st["back"].as_bool(), st["forward"].as_bool(), st["error"].is_null()), (Some(false), Some(false), true), "{st}");
    drop(s);

    // Neu laden meldet Laden und Ende (Zurück/Vor brauchen einen Klick in der Seite: Prüfliste).
    let mut s = Sess::start(home.path(), &[], opts(&format!("http://127.0.0.1:{port}/neu")), true);
    assert!(s.wait(60, |s| s.loaded("Ausgang")));
    s.command(serde_json::json!({"op": "nav", "action": "reload"}));
    let n = s.custom.len();
    assert!(s.wait(30, |s| s.custom[n..].iter().any(|c| c["ev"] == "status" && c["loading"] == true)), "Neu laden meldet kein Laden");
    assert!(s.wait(30, |s| s.loaded("Ausgang")), "Neu laden endet nicht");
}

#[test]
fn unreachable_server_reports_an_error() {
    if !runtime_available() {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    // Freier Port ohne Zuhörer.
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let mut s = Sess::start(home.path(), &[], opts(&format!("http://127.0.0.1:{port}/")), true);
    assert!(s.wait(60, |s| s.custom.iter().any(|c| c["ev"] == "status" && c["error"].is_string())), "Fehler nicht gemeldet: {:?}", s.custom);
    let e = s.custom.iter().find(|c| c["ev"] == "status" && c["error"].is_string()).unwrap();
    assert!(e["error"].as_str().unwrap().contains("127.0.0.1"), "{e}");
}

/// Startet den Worker gegen `url` (Anmeldeseite) und wartet auf die Anmeldung.
fn login_run(home: &Path, url: &str, accept: bool, ignore: bool, rx: &mpsc::Receiver<(String, String)>) -> (Option<String>, Sess) {
    let mut o = opts_login(url);
    o["ignore_certificate"] = serde_json::json!(ignore);
    let mut s = Sess::start(home, &[], o, accept);
    let deadline = Instant::now() + Duration::from_secs(if accept { 60 } else { 10 });
    let mut posted = None;
    while Instant::now() < deadline && posted.is_none() {
        s.pump(Duration::from_millis(50));
        while let Ok((path, body)) = rx.try_recv() {
            if path == "/login" {
                posted = Some(body);
            }
        }
    }
    (posted, s)
}

fn expect_login(posted: &Option<String>, s: &Sess) {
    let body = posted.as_ref().unwrap_or_else(|| panic!("keine Anmeldung gesendet; Meldungen: {:#?} {:#?}", s.log, s.custom));
    assert_eq!(form_decode(body), [("user".to_string(), "admin".to_string()), ("pw".to_string(), PASSWORD.to_string())]);
}

#[test]
fn form_login_with_script_and_profile_per_target() {
    if !runtime_available() {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let (port, rx) = http_server();
    let (posted, mut s) = login_run(home.path(), &format!("http://127.0.0.1:{port}/"), true, false, &rx);
    expect_login(&posted, &s);
    assert_eq!(s.questions, 0);
    // Nach dem Abschicken lädt die Seite „Angemeldet“.
    assert!(s.wait(30, |s| s.loaded("Angemeldet")), "Nach der Anmeldung: {:?}", s.status_title());
    drop(s);
    let profile = profile_of(home.path());
    assert!(profile.join("EBWebView").is_dir(), "Profilordner mit Browserdaten fehlt: {}", profile.display());
    assert!(profile.join("protokoll.log").is_file());
    // Ein zweites Ziel hat sein eigenes Profil.
    let mut o = opts(&format!("http://127.0.0.1:{port}/titel"));
    o["profile"] = serde_json::json!("6f1c0000-0000-4000-8000-0000000000bb");
    let mut s = Sess::start(home.path(), &[], o, true);
    assert!(s.wait(60, |s| s.loaded("Prüfseite ä")));
    drop(s);
    assert!(home.path().join("daten/web-profile/6f1c0000-0000-4000-8000-0000000000bb/EBWebView").is_dir());
    // Kennwort gelangt nie ins Protokoll.
    let trace = std::fs::read_to_string(profile.join("protokoll.log")).unwrap();
    assert!(!trace.contains("Kennwort") || !trace.contains("Prüf-"), "{trace}");
    assert!(!trace.contains(PASSWORD));
}

#[test]
fn certificate_question_remembered_per_profile() {
    if !runtime_available() {
        return;
    }
    let pem = tempfile::tempdir().unwrap();
    let openssl = ["openssl", "C:\\Program Files\\Git\\usr\\bin\\openssl.exe", "C:\\Program Files\\Git\\mingw64\\bin\\openssl.exe"]
        .iter()
        .find(|p| Command::new(p).arg("version").stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false))
        .copied();
    let Some(openssl) = openssl else {
        eprintln!("NICHT AUSGEFÜHRT (TLS-Teil): openssl fehlt");
        return;
    };
    let ok = Command::new(openssl)
        .args(["req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes", "-days", "2", "-subj", "/CN=127.0.0.1", "-addext", "subjectAltName=IP:127.0.0.1"])
        .arg("-keyout")
        .arg(pem.path().join("key.pem"))
        .arg("-out")
        .arg(pem.path().join("cert.pem"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env("MSYS_NO_PATHCONV", "1")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(ok, "Zertifikat nicht erzeugt");
    use rustls::pki_types::pem::PemObject;
    let certs = rustls::pki_types::CertificateDer::pem_file_iter(pem.path().join("cert.pem")).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
    let key = rustls::pki_types::PrivateKeyDer::from_pem_file(pem.path().join("key.pem")).unwrap();
    let cfg = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap();
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || serve(l, tx, Some(Arc::new(cfg))));
    let url = format!("https://127.0.0.1:{port}/");
    let home = tempfile::tempdir().unwrap();

    // Erst Rückfrage, abgelehnt ⇒ keine Anmeldung.
    let (posted, s) = login_run(home.path(), &url, false, false, &rx);
    assert!(posted.is_none(), "abgelehntes Zertifikat darf nicht zur Anmeldung führen");
    assert!(s.questions >= 1, "Rückfrage erwartet: {:#?}", s.log);
    assert!(s.log.iter().any(|t| t.contains("nicht vertrauenswürdig")), "{:#?}", s.log);
    drop(s);
    assert!(!profile_of(home.path()).join("zertifikate.json").exists(), "abgelehnt darf nichts gemerkt werden");

    // Angenommen ⇒ Anmeldung; Fingerabdruck im Profil gespeichert.
    let (posted, s) = login_run(home.path(), &url, true, false, &rx);
    expect_login(&posted, &s);
    assert_eq!(s.questions, 1);
    assert!(profile_of(home.path()).join("zertifikate.json").is_file());
    drop(s);

    // Neuer Start mit demselben Profil: keine Rückfrage mehr.
    let (posted, s) = login_run(home.path(), &url, true, false, &rx);
    expect_login(&posted, &s);
    assert_eq!(s.questions, 0);
    drop(s);

    // Option „Zertifikatsprüfung abschalten“: frisches Profil, keine Rückfrage, nichts gespeichert.
    let fresh = tempfile::tempdir().unwrap();
    let (posted, s) = login_run(fresh.path(), &url, true, true, &rx);
    expect_login(&posted, &s);
    assert_eq!(s.questions, 0, "mit abgeschalteter Prüfung darf es keine Rückfrage geben: {:#?}", s.log);
    assert!(!profile_of(fresh.path()).join("zertifikate.json").exists());
}

/// Programm, das den Start belegt: hängt die übergebene Datei an eine Protokolldatei an.
fn write_opener(dir: &Path) -> (PathBuf, PathBuf) {
    let log = dir.join("opener.log");
    let cmd = dir.join("opener.cmd");
    std::fs::write(&cmd, format!("@echo off\r\necho gestartet: %1>> \"{}\"\r\n", log.display())).unwrap();
    (cmd, log)
}

#[test]
fn downloads_save_to_folder_and_open_only_ica() {
    if !runtime_available() {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let tools = tempfile::tempdir().unwrap();
    let (opener, log) = write_opener(tools.path());
    let env = [("RJ_ICA_OPENER", opener.display().to_string())];
    let (port, _rx) = http_server();
    let dir = home.path().join("daten/downloads");

    // .ica: gespeichert und mit dem Testprogramm gestartet.
    let mut s = Sess::start(home.path(), &env, opts(&format!("http://127.0.0.1:{port}/start.ica")), true);
    assert!(s.wait(60, |s| s.downloads().iter().any(|d| d["state"] == "done")), "Download nicht fertig: {:?} {:?}", s.custom, s.log);
    let d = s.downloads().into_iter().find(|d| d["state"] == "done").unwrap().clone();
    assert_eq!(d["name"], "start.ica");
    assert_eq!(d["opened"], true, "{d}");
    assert!(d["error"].is_null(), "{d}");
    assert!(s.downloads().iter().any(|d| d["state"] == "started"), "Beginn nicht gemeldet");
    let saved = dir.join("start.ica");
    assert!(std::fs::read_to_string(&saved).unwrap().contains("[Encoding]"), "Datei fehlt oder falsch: {}", saved.display());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !log.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    let started = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(started.contains("start.ica"), "Testprogramm nicht gestartet: {started:?}");
    drop(s);

    // Zweiter Download gleichen Namens überschreibt nicht.
    let mut s = Sess::start(home.path(), &env, opts(&format!("http://127.0.0.1:{port}/start.ica")), true);
    assert!(s.wait(60, |s| s.downloads().iter().any(|d| d["state"] == "done")), "{:?}", s.custom);
    assert!(dir.join("start (1).ica").is_file(), "Datei mit neuem Namen fehlt");
    drop(s);
    // Auch der zweite Download wird geöffnet (das Testprogramm läuft asynchron: erst warten, dann zählen).
    let deadline = Instant::now() + Duration::from_secs(10);
    while std::fs::read_to_string(&log).unwrap_or_default().lines().count() < 2 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(std::fs::read_to_string(&log).unwrap_or_default().lines().count(), 2, "zweite .ica nicht geöffnet");

    // Andere Datei: gespeichert, nicht geöffnet.
    let before = std::fs::read_to_string(&log).unwrap_or_default().lines().count();
    let mut s = Sess::start(home.path(), &env, opts(&format!("http://127.0.0.1:{port}/daten.bin")), true);
    assert!(s.wait(60, |s| s.downloads().iter().any(|d| d["state"] == "done")), "Download-Ereignisse: {:?}; Meldungen: {:?}", s.downloads(), s.log);
    let d = s.downloads().into_iter().find(|d| d["state"] == "done").unwrap().clone();
    assert_eq!((d["opened"].as_bool(), d["received"].as_u64()), (Some(false), Some(5000)), "{d}");
    assert_eq!(std::fs::metadata(dir.join("daten.bin")).unwrap().len(), 5000);
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(std::fs::read_to_string(&log).unwrap_or_default().lines().count(), before, ".bin darf nicht geöffnet werden");
    drop(s);

    // Name mit Umkehr der Schreibrichtung: Zeichen ersetzt, nicht als ausführbare Endung gespeichert.
    let mut s = Sess::start(home.path(), &env, opts(&format!("http://127.0.0.1:{port}/falsch")), true);
    assert!(s.wait(60, |s| s.downloads().iter().any(|d| d["state"] == "done")), "{:?}", s.custom);
    let d = s.downloads().into_iter().find(|d| d["state"] == "done").unwrap().clone();
    let name = d["name"].as_str().unwrap();
    assert!(!name.contains('\u{202e}'), "{name:?}");
    assert!(dir.join(name).is_file(), "{name}");
    // Ein Download ist kein Ladefehler: die Seite meldet keinen Fehler.
    assert!(s.custom.iter().rev().find(|c| c["ev"] == "status").is_some_and(|c| c["error"].is_null()), "Ladefehler gemeldet: {:?}", s.custom);
}

#[test]
fn download_without_associated_program_reports_error() {
    if !runtime_available() {
        return;
    }
    // Ohne Testprogramm: ShellExecute. Ist .ica auf diesem Rechner nicht zugeordnet, wird das gemeldet,
    // sonst startet das zugeordnete Programm; beides ist ein gültiges Ergebnis, die Datei liegt immer im Zielordner.
    let home = tempfile::tempdir().unwrap();
    let (port, _rx) = http_server();
    let mut s = Sess::start(home.path(), &[], opts(&format!("http://127.0.0.1:{port}/start.ica")), true);
    assert!(s.wait(60, |s| s.downloads().iter().any(|d| d["state"] == "done")), "{:?}", s.custom);
    let d = s.downloads().into_iter().find(|d| d["state"] == "done").unwrap().clone();
    eprintln!("Ergebnis ohne Testprogramm: {d}");
    assert!(d["opened"] == true || d["error"].is_string(), "{d}");
    assert!(home.path().join("daten/downloads/start.ica").is_file());
}

#[test]
fn field_detection_works_headless() {
    if !runtime_available() {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let (port, _rx) = http_server();
    // Wie „Felder erkennen“ im Editor: Sitzung ohne Fenster im Hauptprogramm, nur der Befehl `detect`.
    let mut s = Sess::start(home.path(), &[], opts(&format!("http://127.0.0.1:{port}/")), true);
    assert!(s.wait(60, |s| s.loaded("Anmeldung")), "Seite nicht geladen: {:?}", s.log);
    s.command(serde_json::json!({"op": "detect"}));
    assert!(s.wait(20, |s| s.custom.iter().any(|c| c["ev"] == "fields")), "keine Felder gemeldet: {:?}", s.custom);
    let f = s.custom.iter().find(|c| c["ev"] == "fields").unwrap().clone();
    let sel = |k: &str| -> Vec<String> { f[k].as_array().unwrap().iter().map(|e| e["selector"].as_str().unwrap().to_string()).collect() };
    assert_eq!(sel("users"), ["#user"], "{f}");
    assert_eq!(sel("passwords"), ["#pw"], "{f}");
    assert!(sel("submits").contains(&"#go".to_string()), "{f}");
    assert!(!f.to_string().contains(PASSWORD), "Werte der Felder werden nie gelesen: {f}");
}

/// Fenster dieses Testprozesses mit eigener Nachrichtenschleife (Elternfenster für das Einbetten).
struct HostWindow {
    hwnd: isize,
    tid: u32,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl HostWindow {
    fn open() -> HostWindow {
        use windows_sys::Win32::System::Threading::GetCurrentThreadId;
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || unsafe {
            let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            let h = CreateWindowExW(0, class.as_ptr(), std::ptr::null(), WS_OVERLAPPEDWINDOW | WS_VISIBLE, 40, 40, 900, 700, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null());
            tx.send((h as isize, GetCurrentThreadId())).unwrap();
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            DestroyWindow(h);
        });
        let (hwnd, tid) = rx.recv().unwrap();
        HostWindow { hwnd, tid, thread: Some(thread) }
    }
}

impl Drop for HostWindow {
    fn drop(&mut self) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
        unsafe { PostThreadMessageW(self.tid, WM_QUIT, 0, 0) };
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Kindfenster des Fensters `parent`, die einem bestimmten Prozess gehören: (Klasse, Breite, Höhe, sichtbar).
fn children_of_process(parent: isize, pid: u32) -> Vec<(String, i32, i32, bool)> {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    struct Ctx {
        pid: u32,
        out: Vec<(String, i32, i32, bool)>,
    }
    unsafe extern "system" fn each(h: HWND, l: LPARAM) -> BOOL {
        let ctx = &mut *(l as *mut Ctx);
        let mut p = 0u32;
        GetWindowThreadProcessId(h, &mut p);
        if p == ctx.pid {
            let mut buf = [0u16; 128];
            let n = GetClassNameW(h, buf.as_mut_ptr(), 128) as usize;
            let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            GetWindowRect(h, &mut r);
            ctx.out.push((String::from_utf16_lossy(&buf[..n]), r.right - r.left, r.bottom - r.top, IsWindowVisible(h) != 0));
        }
        1
    }
    let mut ctx = Ctx { pid, out: Vec::new() };
    unsafe { EnumChildWindows(parent as HWND, Some(each), &mut ctx as *mut Ctx as LPARAM) };
    ctx.out
}

#[test]
fn page_embeds_into_a_window_of_another_process() {
    if !runtime_available() {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let (port, _rx) = http_server();
    let host = HostWindow::open();
    // Nicht headless: der Worker wartet auf das Elternfenster aus dem Befehl.
    let mut o = opts(&format!("http://127.0.0.1:{port}/titel"));
    o["headless"] = serde_json::json!(false);
    let mut s = Sess::start(home.path(), &[], o, true);
    s.command(serde_json::json!({"op": "embed", "hwnd": host.hwnd}));
    s.command(serde_json::json!({"op": "bounds", "x": 0, "y": 0, "w": 640, "h": 480, "scale": 100}));
    assert!(s.wait(60, |s| s.loaded("Prüfseite ä")), "Seite nicht geladen: {:?} {:?}", s.custom, s.log);
    let pid = s.pid;
    let web = |s: &mut Sess| {
        s.pump(Duration::from_millis(50));
        children_of_process(host.hwnd, pid).into_iter().find(|c| c.0 == "WRY_WEBVIEW")
    };
    let w = web(&mut s).expect("Kindfenster des Workers im fremden Fenster");
    assert!(w.3, "sichtbar");
    // Größe folgt dem Befehl.
    s.command(serde_json::json!({"op": "bounds", "x": 0, "y": 0, "w": 500, "h": 300, "scale": 100}));
    assert!(s.wait(10, |_| children_of_process(host.hwnd, pid).iter().any(|c| c.0 == "WRY_WEBVIEW" && c.1 == 500 && c.2 == 300)), "Größe nicht übernommen: {:?}", children_of_process(host.hwnd, pid));
    // Ausblenden und wieder zeigen (Tabwechsel).
    s.command(serde_json::json!({"op": "visible", "on": false}));
    assert!(s.wait(10, |_| children_of_process(host.hwnd, pid).iter().any(|c| c.0 == "WRY_WEBVIEW" && !c.3)), "nicht ausgeblendet");
    s.command(serde_json::json!({"op": "visible", "on": true}));
    assert!(s.wait(10, |_| children_of_process(host.hwnd, pid).iter().any(|c| c.0 == "WRY_WEBVIEW" && c.3)), "nicht wieder gezeigt");
    // Zweites Elternfenster (Tab abgerissen): die Seite zieht um.
    let host2 = HostWindow::open();
    s.command(serde_json::json!({"op": "embed", "hwnd": host2.hwnd}));
    assert!(s.wait(10, |_| children_of_process(host2.hwnd, pid).iter().any(|c| c.0 == "WRY_WEBVIEW")), "Seite nicht umgezogen");
    assert!(children_of_process(host.hwnd, pid).iter().all(|c| c.0 != "WRY_WEBVIEW"), "Seite noch im alten Fenster");
}
