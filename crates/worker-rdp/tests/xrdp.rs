//! Tests gegen einen lokal gestarteten xrdp mit Xvnc-Sitzung (Testkit).
//! Ohne xrdp oder ohne Root-Rechte werden die Tests mit Hinweis übersprungen.
#![cfg(unix)] // setzt xrdp und libc::kill voraus; unter Windows nicht übersetzbar

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use rj_core::{ClipboardDirection, Secret};
use rj_rdp::session::*;
use rj_testkit::xrdp::{Xrdp, PASSWORD, USER, USER2};

static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn params(port: u16, user: &str) -> RdpParams {
    RdpParams {
        host: "127.0.0.1".into(),
        port,
        username: user.into(),
        domain: None,
        password: Secret::from_str_exact(PASSWORD),
        width: 1024,
        height: 768,
        security: Security::Tls,
        ignore_certificate: false,
        clipboard: ClipboardDirection::Both,
        clipboard_limit: 16 * 1024 * 1024,
        keyboard_layout: 0x0407,
        config_path: Some(tempfile::tempdir().unwrap().keep().display().to_string()),
        timeout_ms: 15000,
        gfx: false,
    }
}

fn pump(s: &mut RdpSession, ms: u64, mut until: impl FnMut(&RdpEvent) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_millis(ms);
    let mut ev = Vec::new();
    while Instant::now() < deadline {
        ev.clear();
        assert!(s.poll(30, &mut ev), "Verbindung unerwartet getrennt: {ev:?}");
        for e in &ev {
            if until(e) {
                return true;
            }
        }
    }
    false
}

/// Läuft `f` in einem Faden, während die Sitzung weiter bedient wird.
fn while_pumping<T: Send + 'static>(s: &mut RdpSession, f: impl FnOnce() -> T + Send + 'static) -> T {
    let h = std::thread::spawn(f);
    while !h.is_finished() {
        pump(s, 50, |_| false);
    }
    h.join().unwrap()
}

fn start() -> Option<Arc<Xrdp>> {
    match Xrdp::start() {
        Ok(x) => Some(Arc::new(x)),
        Err(e) => {
            eprintln!("ÜBERSPRUNGEN: {e}");
            None
        }
    }
}

fn session(x: &Xrdp, user: &str) -> RdpSession {
    let mut s = RdpSession::new(|_| CertDecision::AcceptOnce).unwrap();
    s.connect(&params(x.port, user)).expect("Verbindung");
    let deadline = Instant::now() + Duration::from_secs(30);
    while x.display_of(user).is_none() && Instant::now() < deadline {
        pump(&mut s, 500, |_| false);
    }
    assert!(x.display_of(user).is_some(), "Sitzung startet nicht");
    pump(&mut s, 3000, |_| false);
    s
}

#[test]
fn certificate_rejected_means_no_connection() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(x) = start() else { return };
    let asked = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let a2 = asked.clone();
    let mut s = RdpSession::new(move |c| {
        assert!(!c.fingerprint.is_empty());
        a2.store(true, std::sync::atomic::Ordering::SeqCst);
        CertDecision::Reject
    })
    .unwrap();
    assert!(s.connect(&params(x.port, USER)).is_err());
    assert!(asked.load(std::sync::atomic::Ordering::SeqCst), "Zertifikat wurde abgefragt");
}

#[test]
fn frame_keys_and_clipboard_matrix() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(x) = start() else { return };
    let mut s = session(&x, USER);

    // Bild: Inhalt ist nicht leer.
    let nonzero = s.with_frame(|buf, w, h| {
        assert_eq!((w, h), (1024, 768));
        buf.iter().filter(|b| **b != 0).count()
    });
    assert!(nonzero > 1000, "Bild enthält Bildpunkte");

    // Tastatur: Maus ins xterm, Unicode-Text, Eingabetaste als Scancode.
    s.send_mouse(rj_rdp::keymap::mouse::MOVE, 100, 100);
    s.send_text("Grüße ß äöü ÄÖÜ €\tTab");
    s.send_key(0xff0d, true);
    s.send_key(0xff0d, false);
    s.send_text("🚀");
    s.send_key(0xff0d, true);
    s.send_key(0xff0d, false);
    pump(&mut s, 2000, |_| false);
    let typed = x.typed_text();
    eprintln!("MATRIX Tastatur: {typed:?}");
    let mut lines = typed.lines();
    assert_eq!(lines.next(), Some("Grüße ß äöü ÄÖÜ €\tTab"), "Tasteneingabe kommt im Ziel an");
    let emoji_line = lines.next().unwrap_or("<fehlt>");
    eprintln!("MATRIX Tastatur Emoji (Ersatzpaar): {}", if emoji_line == "🚀" { "angekommen" } else { "von der Gegenstelle nicht umgesetzt" });

    // Zwischenablage-Matrix.
    let mut big = String::new();
    let mut i = 0;
    while big.len() < 100 * 1024 {
        big.push_str(&format!("Zeile {i:05}: Grüße ß äöü\tTab\n"));
        i += 1;
    }
    let cases: Vec<(&str, String)> = vec![
        ("Umlaute", "äöü ÄÖÜ".into()),
        ("ß", "Straße ẞ".into()),
        ("Emoji", "🚀👍🏽 €".into()),
        ("Tab", "a\tb\t\tc".into()),
        ("CRLF", "eins\r\nzwei\r\ndrei".into()),
        ("LF", "eins\nzwei".into()),
        ("100 kB", big),
    ];
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut failures = Vec::new();
    for (name, text) in &cases {
        // Host → Ziel: angeboten wird CF_UNICODETEXT mit CRLF.
        s.offer_text(text).unwrap();
        pump(&mut s, 500, |_| false);
        let x2 = x.clone();
        let got = while_pumping(&mut s, move || x2.x_run("timeout 10 xclip -o -selection clipboard | base64 -w0", None));
        let got = got.ok().and_then(|g| b64.decode(g.trim()).ok()).unwrap_or_default();
        let expected = text.replace("\r\n", "\n").replace('\n', "\r\n");
        let ok_ht = got == expected.as_bytes();
        eprintln!("MATRIX Host→Ziel {name}: {}", if ok_ht { "grün".to_string() } else { format!("abweichend ({} statt {} Bytes)", got.len(), expected.len()) });

        // Ziel → Host: xclip setzt die Zwischenablage der Sitzung.
        let x3 = x.clone();
        let data = text.clone().into_bytes();
        let h = std::thread::spawn(move || x3.x_run("timeout 15 xclip -i -selection clipboard -loops 3 -quiet", Some(&data)));
        let mut received = None;
        pump(&mut s, 12_000, |e| {
            if let RdpEvent::ClipText(t) = e {
                received = Some(t.clone());
                return true;
            }
            if let RdpEvent::Notice(n) = e {
                eprintln!("Hinweis: {n}");
            }
            false
        });
        let _ = while_pumping(&mut s, move || h.join());
        let expected_th = text.replace("\r\n", "\n");
        let ok_th = received.as_deref() == Some(expected_th.as_str());
        eprintln!("MATRIX Ziel→Host {name}: {}", if ok_th { "grün".to_string() } else { format!("abweichend: {:?}", received.map(|r| r.chars().take(40).collect::<String>())) });
        if *name != "Emoji" && !(ok_ht && ok_th) {
            failures.push(*name);
        }
    }
    assert!(failures.is_empty(), "Zwischenablage-Matrix: {failures:?}");

    // Dateien Ziel → Host: xclip bietet eine Dateiliste (text/uri-list) an.
    let path = x.put_file("Prüfdatei ä.txt", "Inhalt der Datei 🚀\n".as_bytes()).unwrap();
    let uri = format!("file://{}", path.display().to_string().replace(' ', "%20").replace('ä', "%C3%A4"));
    let x4 = x.clone();
    let h = std::thread::spawn(move || x4.x_run("timeout 15 xclip -i -selection clipboard -t text/uri-list -loops 5 -quiet", Some(uri.as_bytes())));
    let mut files = None;
    pump(&mut s, 12_000, |e| {
        if let RdpEvent::ClipFiles(f) = e {
            files = Some(f.clone());
            return true;
        }
        false
    });
    let _ = while_pumping(&mut s, move || h.join());
    eprintln!(
        "MATRIX Datei Ziel→Host: {}",
        match &files {
            Some(f) if f.len() == 1 && f[0].1 == "Inhalt der Datei 🚀\n".as_bytes() => format!("grün ({})", f[0].0.name),
            Some(f) => format!("abweichend: {:?}", f.iter().map(|(d, c)| (d.name.clone(), c.len())).collect::<Vec<_>>()),
            None => "keine Dateiliste von der Gegenstelle".into(),
        }
    );

    // Dateien Host → Ziel.
    s.offer_files(vec![("Übertragung 🚀.txt".into(), b"Hallo Ziel\n".to_vec())]).unwrap();
    pump(&mut s, 500, |_| false);
    let x5 = x.clone();
    let targets = while_pumping(&mut s, move || x5.x_run("timeout 10 xclip -o -selection clipboard -t TARGETS", None));
    let x6 = x.clone();
    let uris = while_pumping(&mut s, move || x6.x_run("timeout 10 xclip -o -selection clipboard -t text/uri-list", None));
    eprintln!("MATRIX Datei Host→Ziel: Formate {:?}, Liste {:?}", targets.map(|t| t.replace('\n', " ")), uris);

    // Größenänderung über den Anzeige-Steuerkanal.
    let supported = s.resize(1280, 800);
    let mut resized = false;
    if supported {
        resized = pump(&mut s, 5000, |e| matches!(e, RdpEvent::Resized { width: 1280, height: 800 }));
    }
    eprintln!("MATRIX Größenänderung im Betrieb: {}", if resized { "grün" } else if supported { "Kanal vorhanden, Gegenstelle ändert nicht" } else { "Kanal nicht angeboten (Worker verbindet neu)" });
}

/// Zwei Worker-Prozesse, einer wird hart beendet; der andere läuft weiter.
#[test]
fn pointer_shape_of_the_remote_side_arrives() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(x) = start() else { return };
    let mut s = RdpSession::new(|_| CertDecision::AcceptOnce).unwrap();
    s.connect(&params(x.port, USER)).expect("Verbindung");
    // Der Zeiger kommt beim Aufbau und bei Bewegungen über Fenster mit eigener Zeigerform.
    let mut shape = None;
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut step = 0u16;
    while shape.is_none() && Instant::now() < deadline {
        step = (step + 37) % 700;
        s.send_mouse(rj_rdp::keymap::mouse::MOVE, 50 + step, 50 + step / 2);
        pump(&mut s, 300, |e| {
            if let RdpEvent::Pointer(p) = e {
                if p.kind == 2 {
                    shape = Some(p.clone());
                    return true;
                }
            }
            false
        });
    }
    let p = shape.expect("kein Mauszeiger der Gegenstelle empfangen");
    assert!(p.width > 0 && p.height > 0 && p.width <= 384, "{}×{}", p.width, p.height);
    assert_eq!(p.bgra.len(), (p.width * p.height * 4) as usize);
    assert!(p.hot_x < p.width && p.hot_y < p.height);
    assert!(p.bgra.chunks(4).any(|px| px[3] != 0), "Zeiger ist nicht völlig durchsichtig");
}

#[test]
fn worker_crash_isolation() {
    use rj_ipc::*;
    use rj_supervisor::*;
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(x) = start() else { return };
    let mut sup = Supervisor::new();
    let mut ids = Vec::new();
    for user in [USER, USER2] {
        let (id, _) = sup.spawn(WorkerSpec::new(env!("CARGO_BIN_EXE_rj-worker-rdp"))).unwrap();
        let cfg = tempfile::tempdir().unwrap().keep();
        sup.send(id, &ToWorker::Connect(ConnectRequest {
            host: "127.0.0.1".into(),
            port: x.port,
            width: 800,
            height: 600,
            options_json: serde_json::json!({ "security": "tls", "config_path": cfg }).to_string(),
            connect_timeout_secs: 15,
            keepalive_secs: 30,
        }))
        .unwrap();
        sup.send(id, &ToWorker::Credentials(CredentialPayload {
            username: user.into(),
            domain: None,
            password: SecretBytes(PASSWORD.as_bytes().to_vec()),
            private_key: None,
        }))
        .unwrap();
        ids.push(id);
    }
    let mut connected = std::collections::HashSet::new();
    let mut frames = std::collections::HashMap::<WorkerId, u32>::new();
    let mut shared = std::collections::HashMap::<WorkerId, rj_ipc::shm::FrameBufferReader>::new();
    let deadline = Instant::now() + Duration::from_secs(40);
    while (connected.len() < 2 || frames.values().filter(|n| **n > 2).count() < 2) && Instant::now() < deadline {
        for ev in sup.tick(Duration::from_millis(50)) {
            if let SupervisorEvent::Message { id, msg } = ev {
                match msg {
                    FromWorker::Question { id: q, .. } => sup.send(id, &ToWorker::Answer { question: q, accept: true }).unwrap(),
                    FromWorker::SharedFrame { handle, width, height } => {
                        // Versiegelter Bildpuffer über den Socket übergeben.
                        let h = sup.take_shared(id, handle).expect("Bildpuffer übergeben");
                        shared.insert(id, rj_ipc::shm::FrameBufferReader::open(h, width, height).expect("versiegelt"));
                    }
                    FromWorker::State(SessionState::Connected) => {
                        connected.insert(id);
                    }
                    FromWorker::State(SessionState::Failed { reason }) => panic!("Verbindung {id}: {reason}"),
                    FromWorker::Frame(_) => *frames.entry(id).or_default() += 1,
                    _ => {}
                }
            }
        }
    }
    assert_eq!(connected.len(), 2, "beide Sitzungen verbunden");
    assert_eq!(shared.len(), 2, "beide Bildpuffer angekommen");
    // Das xterm der Sitzung erscheint erst nach den ersten Bildern.
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until && !shared.values().all(|r| r.pixels().iter().any(|b| *b != 0)) {
        sup.tick(Duration::from_millis(50));
    }
    for r in shared.values() {
        assert!(r.pixels().iter().any(|b| *b != 0), "Bild im geteilten Speicher");
    }
    // Worker A hart beenden.
    let pid = sup.pid(ids[0]).unwrap();
    // SAFETY: Signal an den selbst gestarteten Worker.
    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    let mut exited = None;
    let before = frames.get(&ids[1]).copied().unwrap_or(0);
    let deadline = Instant::now() + Duration::from_secs(10);
    while exited.is_none() && Instant::now() < deadline {
        for ev in sup.tick(Duration::from_millis(50)) {
            match ev {
                SupervisorEvent::Exited { id, reason, .. } => exited = Some((id, reason)),
                SupervisorEvent::Message { id, msg: FromWorker::Frame(_) } => *frames.entry(id).or_default() += 1,
                _ => {}
            }
        }
    }
    let (id, reason) = exited.expect("Absturz wird gemeldet");
    assert_eq!(id, ids[0]);
    eprintln!("Worker A beendet: {}", reason.describe());
    // Worker B lebt, antwortet und liefert nach Eingabe weiter Bilder.
    assert!(sup.is_running(ids[1]));
    sup.send(ids[1], &ToWorker::Ping).unwrap();
    sup.send(ids[1], &ToWorker::Input(InputEvent::MouseMove { x: 50, y: 50 })).unwrap();
    sup.send(ids[1], &ToWorker::Input(InputEvent::Text("weiter".into()))).unwrap();
    let mut pong = false;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !(pong && frames.get(&ids[1]).copied().unwrap_or(0) > before) && Instant::now() < deadline {
        for ev in sup.tick(Duration::from_millis(50)) {
            match ev {
                SupervisorEvent::Message { msg: FromWorker::Pong, .. } => pong = true,
                SupervisorEvent::Message { id, msg: FromWorker::Frame(_) } => *frames.entry(id).or_default() += 1,
                SupervisorEvent::Message { msg: FromWorker::State(SessionState::Disconnected { reason }), .. } => panic!("B getrennt: {reason}"),
                _ => {}
            }
        }
    }
    assert!(pong, "Worker B antwortet");
    assert!(frames[&ids[1]] > before, "Worker B liefert weiter Bilder");
    sup.shutdown();
}
