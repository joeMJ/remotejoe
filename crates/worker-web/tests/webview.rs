//! Ende-zu-Ende mit echter WebView (WebKitGTK unter Xvfb) gegen eine lokale
//! Anmeldeseite. Nur mit Merkmal `webview`; ohne Xvfb wird übersprungen.
#![cfg(feature = "webview")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use rj_ipc::*;
use rj_supervisor::*;

const PASSWORD: &str = "Prüf-\"Kennwort\"&ä 🚀";
const PAGE: &str = "<html><body><form method='post' action='/login'>\
<input id='user' name='user'><input id='pw' name='pw' type='password'>\
<button id='go' type='submit'>Anmelden</button></form></body></html>";

fn serve(l: TcpListener, tx: mpsc::Sender<(String, String)>, tls: Option<Arc<rustls::ServerConfig>>) {
    for s in l.incoming().flatten() {
        let tx = tx.clone();
        match &tls {
            None => handle(s, &tx),
            Some(cfg) => {
                let Ok(conn) = rustls::ServerConnection::new(cfg.clone()) else { continue };
                handle(rustls::StreamOwned::new(conn, s), &tx);
            }
        }
    }
}

fn handle<S: Read + Write>(s: S, tx: &mpsc::Sender<(String, String)>) {
    {
        let mut r = BufReader::new(s);
        let mut first = String::new();
        if r.read_line(&mut first).is_err() {
            return;
        }
        let mut len = 0;
        loop {
            let mut h = String::new();
            r.read_line(&mut h).unwrap_or(0);
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
        let page = if path == "/login" { "<html><body>angemeldet</body></html>" } else { PAGE };
        let s = r.get_mut();
        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len());
        let _ = s.flush();
        let _ = tx.send((path, String::from_utf8_lossy(&body).into_owned()));
    }
}

fn form_decode(s: &str) -> Vec<(String, String)> {
    s.split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| {
            let mut out = Vec::new();
            let b = v.replace('+', " ").into_bytes();
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
            (k.to_string(), String::from_utf8(out).unwrap())
        })
        .collect()
}

struct Run {
    posted: Option<String>,
    questions: usize,
    log: Vec<String>,
}

/// Startet den Worker gegen `url` und beantwortet Rückfragen mit `accept`.
fn run(display: &str, data: &std::path::Path, url: &str, port: u16, accept: bool, ignore: bool, rx: &mpsc::Receiver<(String, String)>) -> Run {
    let mut spec = WorkerSpec::new(env!("CARGO_BIN_EXE_rj-worker-web"));
    // WebKit reserviert sehr viel Adressraum; eine RLIMIT_AS-Grenze bricht es.
    spec.memory_limit = None;
    spec.env = vec![
        ("DISPLAY".into(), display.to_string()),
        ("XDG_DATA_HOME".into(), data.display().to_string()),
        ("WEBKIT_DISABLE_COMPOSITING_MODE".into(), "1".into()),
        ("WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS".into(), "1".into()),
        ("LIBGL_ALWAYS_SOFTWARE".into(), "1".into()),
    ];
    let mut sup = Supervisor::new();
    let (id, _) = sup.spawn(spec).unwrap();
    let opts = serde_json::json!({
        "url": url,
        "allow_http": true,
        "ignore_certificate": ignore,
        "profile": "6f1c0000-0000-4000-8000-0000000000aa",
        "login": {"username_selector": "#user", "password_selector": "#pw", "submit_selector": "#go"}
    });
    sup.send(id, &ToWorker::Connect(ConnectRequest {
        host: "127.0.0.1".into(),
        port,
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
    let deadline = Instant::now() + Duration::from_secs(if accept { 60 } else { 8 });
    let mut r = Run { posted: None, questions: 0, log: Vec::new() };
    while Instant::now() < deadline && r.posted.is_none() {
        for ev in sup.tick(Duration::from_millis(100)) {
            match ev {
                SupervisorEvent::Message { msg: FromWorker::Question { id: q, text, .. }, .. } => {
                    r.questions += 1;
                    r.log.push(text);
                    sup.send(id, &ToWorker::Answer { question: q, accept }).unwrap();
                }
                SupervisorEvent::Message { msg, .. } => r.log.push(format!("{msg:?}")),
                other => r.log.push(format!("{other:?}")),
            }
        }
        while let Ok((path, body)) = rx.try_recv() {
            if path == "/login" {
                r.posted = Some(body);
            }
        }
    }
    sup.shutdown();
    r
}

fn xvfb() -> Option<rj_testkit::Xvfb> {
    match rj_testkit::Xvfb::start("1024x768x24") {
        Ok(x) => Some(x),
        Err(e) => {
            eprintln!("ÜBERSPRUNGEN: {}", e.0);
            None
        }
    }
}

fn expect_login(r: &Run) {
    let body = r.posted.as_ref().unwrap_or_else(|| panic!("keine Anmeldung gesendet; Meldungen: {:#?}", r.log));
    let fields = form_decode(body);
    assert_eq!(fields, [("user".to_string(), "admin".to_string()), ("pw".to_string(), PASSWORD.to_string())]);
}

#[test]
fn form_login_and_certificate_warning_in_real_webview() {
    let Some(x) = xvfb() else { return };
    let display = x.display.clone();
    let data = tempfile::tempdir().unwrap();

    // HTTP: Anmeldung über das eingespeiste Skript.
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || serve(l, tx, None));
    let r = run(&display, data.path(), &format!("http://127.0.0.1:{port}/"), port, true, false, &rx);
    expect_login(&r);
    assert_eq!(r.questions, 0);
    let profile = data.path().join("remotejoe/web-profile/6f1c0000-0000-4000-8000-0000000000aa");
    assert!(profile.is_dir());

    // HTTPS mit selbstsigniertem Zertifikat: erst Rückfrage, abgelehnt ⇒ keine Anmeldung.
    let pem = tempfile::tempdir().unwrap();
    let ok = Command::new("openssl")
        .args(["req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes", "-days", "2", "-subj", "/CN=web.example.test"])
        .arg("-keyout")
        .arg(pem.path().join("key.pem"))
        .arg("-out")
        .arg(pem.path().join("cert.pem"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        eprintln!("ÜBERSPRUNGEN (TLS-Teil): openssl fehlt");
        return;
    }
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
    let r = run(&display, data.path(), &url, port, false, false, &rx);
    assert!(r.posted.is_none(), "abgelehntes Zertifikat darf nicht zur Anmeldung führen");
    assert!(r.questions >= 1, "Rückfrage erwartet: {:#?}", r.log);
    assert!(r.log.iter().any(|t| t.contains("nicht vertrauenswürdig")), "{:#?}", r.log);

    // Angenommen ⇒ Anmeldung; Fingerabdruck im Profil gespeichert.
    let r = run(&display, data.path(), &url, port, true, false, &rx);
    expect_login(&r);
    assert_eq!(r.questions, 1);
    assert!(profile.join("zertifikate.json").is_file());

    // Neuer Start mit demselben Profil: keine Rückfrage mehr.
    let r = run(&display, data.path(), &url, port, true, false, &rx);
    expect_login(&r);
    assert_eq!(r.questions, 0);

    // Option „Zertifikatsprüfung abschalten“: frisches Profil, keine Rückfrage, nichts gespeichert.
    let fresh = tempfile::tempdir().unwrap();
    let r = run(&display, fresh.path(), &url, port, true, true, &rx);
    expect_login(&r);
    assert_eq!(r.questions, 0, "mit abgeschalteter Prüfung darf es keine Rückfrage geben: {:#?}", r.log);
    assert!(!fresh.path().join("remotejoe/web-profile/6f1c0000-0000-4000-8000-0000000000aa/zertifikate.json").exists());
    drop(x);
}
