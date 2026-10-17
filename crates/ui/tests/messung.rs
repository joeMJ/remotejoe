//! Messungen (von Hand aufzurufen):
//! `cargo test --release -p remotejoe-ui --test messung -- --ignored --nocapture`
//!
//! Speicher je Worker (RSS samt Kindprozessen), Aufbauzeit bis „verbunden“,
//! 25 gleichzeitige Sitzungen gegen lokale Server. Die Worker werden aus dem
//! Zielordner des Builds genommen (vorher mit demselben Profil bauen).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use rj_ipc::*;
use rj_supervisor::*;

fn worker(name: &str) -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.parent().unwrap().parent().unwrap().join(name)
}

fn rss_kib(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)?.parse().ok()))
        .unwrap_or(0)
}

/// RSS eines Prozesses und aller Nachkommen.
fn tree_rss_kib(pid: u32) -> u64 {
    let mut total = rss_kib(pid);
    if let Ok(rd) = std::fs::read_dir("/proc") {
        for e in rd.flatten() {
            let Ok(p) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
            let ppid = std::fs::read_to_string(format!("/proc/{p}/stat"))
                .ok()
                .and_then(|s| s.rsplit_once(") ").and_then(|(_, r)| r.split_whitespace().nth(1)?.parse::<u32>().ok()));
            if ppid == Some(pid) {
                total += tree_rss_kib(p);
            }
        }
    }
    total
}

struct Result {
    connect_ms: Vec<u128>,
    rss_kib: Vec<u64>,
}

fn open_many(program: &str, n: usize, memory_limit: Option<u64>, env: &[(&str, String)], req: impl Fn(usize) -> (ConnectRequest, CredentialPayload)) -> Option<Result> {
    let path = worker(program);
    if !path.exists() {
        eprintln!("ÜBERSPRUNGEN: {} fehlt", path.display());
        return None;
    }
    let mut sup = Supervisor::new();
    let mut ids = Vec::new();
    let start = Instant::now();
    for i in 0..n {
        let mut spec = WorkerSpec::new(&path);
        spec.memory_limit = memory_limit;
        spec.env = env.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        spec.restart = RestartPolicy { enabled: false, ..Default::default() };
        let (id, pid) = sup.spawn(spec).unwrap();
        let (r, c) = req(i);
        sup.send(id, &ToWorker::Connect(r)).unwrap();
        sup.send(id, &ToWorker::Credentials(c)).unwrap();
        ids.push((id, pid));
    }
    let mut connected: std::collections::HashMap<WorkerId, u128> = Default::default();
    let deadline = Instant::now() + Duration::from_secs(90);
    while connected.len() < n && Instant::now() < deadline {
        for ev in sup.tick(Duration::from_millis(20)) {
            match ev {
                SupervisorEvent::Message { id: worker, msg: FromWorker::State(SessionState::Connected) } => {
                    connected.entry(worker).or_insert(start.elapsed().as_millis());
                }
                SupervisorEvent::Message { id: worker, msg: FromWorker::State(SessionState::Failed { reason }) } => {
                    eprintln!("Worker {worker:?} fehlgeschlagen: {reason}");
                }
                SupervisorEvent::Message { id: worker, msg: FromWorker::Question { id, .. } } => {
                    let _ = sup.send(worker, &ToWorker::Answer { question: id, accept: true });
                }
                _ => {}
            }
        }
    }
    // Kurz laufen lassen, damit sich der Speicher einpendelt.
    let settle = Instant::now() + Duration::from_secs(3);
    while Instant::now() < settle {
        sup.tick(Duration::from_millis(50));
    }
    let rss: Vec<u64> = ids.iter().map(|(_, pid)| tree_rss_kib(*pid)).collect();
    let mut ms: Vec<u128> = connected.values().copied().collect();
    ms.sort();
    sup.shutdown();
    Some(Result { connect_ms: ms, rss_kib: rss })
}

fn report(name: &str, n: usize, r: &Result) {
    let total: u64 = r.rss_kib.iter().sum();
    let max = r.rss_kib.iter().max().copied().unwrap_or(0);
    let avg = if r.rss_kib.is_empty() { 0 } else { total / r.rss_kib.len() as u64 };
    println!(
        "MESSWERT {name}: {}/{n} verbunden; letzte Verbindung nach {} ms (erste {} ms); RSS je Worker Ø {} MiB, max {} MiB; gesamt {} MiB",
        r.connect_ms.len(),
        r.connect_ms.last().copied().unwrap_or(0),
        r.connect_ms.first().copied().unwrap_or(0),
        avg / 1024,
        max / 1024,
        total / 1024
    );
}

fn req(host: &str, port: u16, options: serde_json::Value) -> ConnectRequest {
    ConnectRequest { host: host.into(), port, width: 1024, height: 768, options_json: options.to_string(), connect_timeout_secs: 15, keepalive_secs: 30 }
}

fn cred(user: &str, pw: &str) -> CredentialPayload {
    CredentialPayload { username: user.into(), domain: None, password: SecretBytes(pw.as_bytes().to_vec()), private_key: None }
}

#[test]
#[ignore]
fn messung_sitzungen_und_speicher() {
    let dir = tempfile::tempdir().unwrap();

    // Hauptprogramm im Vorführbetrieb (500 Ziele, offscreen).
    let ui = worker("remotejoe");
    if ui.exists() {
        let mut c = std::process::Command::new(&ui).arg("--demo").env("QT_QPA_PLATFORM", "offscreen").spawn().unwrap();
        std::thread::sleep(Duration::from_secs(4));
        println!("MESSWERT Hauptprogramm (--demo, 500 Ziele, 5 Vorführsitzungen): RSS {} MiB", tree_rss_kib(c.id()) / 1024);
        let _ = c.kill();
        let _ = c.wait();
    }

    // 25 SSH-Sitzungen gegen einen lokalen sshd.
    match rj_testkit::sshd::Sshd::start_with(&["MaxStartups 100", "MaxSessions 100"]) {
        Ok(s) => {
            let kh = dir.path().join("known_hosts");
            if let Some(r) = open_many("rj-worker-ssh", 25, Some(1 << 30), &[], |_| {
                (req("127.0.0.1", s.port, serde_json::json!({ "known_hosts": kh })), cred(rj_testkit::sshd::USER, rj_testkit::sshd::PASSWORD))
            }) {
                report("SSH × 25", 25, &r);
            }
        }
        Err(e) => eprintln!("ÜBERSPRUNGEN SSH: {}", e.0),
    }

    // 25 VNC-Sitzungen gegen einen Xvnc (geteilt).
    match rj_testkit::xvnc::Xvnc::start(rj_testkit::xvnc::VncSecurity::VncAuth) {
        Ok(x) => {
            if let Some(r) = open_many("rj-worker-vnc", 25, Some(1 << 30), &[], |_| {
                (req("127.0.0.1", x.port, serde_json::json!({ "known_certs": dir.path().join("vnc_certs") })), cred("", rj_testkit::xvnc::PASSWORD))
            }) {
                report("VNC × 25", 25, &r);
            }
        }
        Err(e) => eprintln!("ÜBERSPRUNGEN VNC: {}", e.0),
    }

    // RDP gegen xrdp: eine Sitzung (Speicher je Worker).
    match rj_testkit::xrdp::Xrdp::start() {
        Ok(x) => {
            if let Some(r) = open_many("rj-worker-rdp", 1, Some(1 << 30), &[], |_| {
                (req("127.0.0.1", x.port, serde_json::json!({ "security": "tls", "config_path": dir.path().join("freerdp") })), cred(rj_testkit::xrdp::USER, rj_testkit::xrdp::PASSWORD))
            }) {
                report("RDP × 1", 1, &r);
            }
        }
        Err(e) => eprintln!("ÜBERSPRUNGEN RDP: {}", e.0),
    }

    // Web-Worker (WebKitGTK) unter Xvfb gegen eine lokale Seite: 3 Sitzungen.
    if let Ok(x) = rj_testkit::Xvfb::start("1280x1024x24") {
        let display = x.display.clone();
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for mut s in l.incoming().flatten() {
                let mut b = [0u8; 4096];
                let _ = s.read(&mut b);
                let page = "<html><body><h1>Messseite</h1></body></html>";
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len());
            }
        });
        let env = [
            ("DISPLAY", display.clone()),
            ("XDG_DATA_HOME", dir.path().display().to_string()),
            ("WEBKIT_DISABLE_COMPOSITING_MODE", "1".to_string()),
            ("WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS", "1".to_string()),
        ];
        if let Some(r) = open_many("rj-worker-web", 3, None, &env, |i| {
            (
                req("127.0.0.1", port, serde_json::json!({ "url": format!("http://127.0.0.1:{port}/"), "allow_http": true, "profile": format!("messung-{i}") })),
                cred("", ""),
            )
        }) {
            report("Web × 3 (inkl. WebKit-Kindprozesse)", 3, &r);
        }
        drop(x);
    }
}
