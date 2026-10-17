//! Hintergrundaufgaben, Sperre und Aufwachen:
//! - langsame Gegenstelle (15 s) blockiert die Oberfläche nicht,
//! - nach dem Sperren ist kein Klartext-Kennwort mehr im Speicher,
//! - simuliertes Aufwachen sperrt nicht und verbindet abgebrochene
//!   Sitzungen neu (Tunnel zuerst).

use std::io::Read;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime};

use rj_core::{CredentialRef, Folder, Index, Node, Protocol, Secret, SshHop, Target, TunnelDef, TunnelKind, TunnelSetting};
use rj_kdbx::{CredentialData, KdfChoice, MasterKey, Vault};
use rj_ui::app::App;
use rj_ui::sessions::{Manager, Reconnect, UiEvent};

#[global_allocator]
static ALLOC: rj_ui::alloc::ZeroizingAllocator = rj_ui::alloc::ZeroizingAllocator;

/// Gemeinsames Daten- und Konfigurationsverzeichnis (Umgebung ist prozessweit).
fn env() -> &'static std::path::Path {
    static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
    let d = DIR.get_or_init(|| tempfile::tempdir().unwrap());
    std::env::set_var("XDG_DATA_HOME", d.path().join("data"));
    std::env::set_var("XDG_CONFIG_HOME", d.path().join("config"));
    d.path()
}

fn workers() -> std::path::PathBuf {
    std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_path_buf()
}

/// Gegenstelle, die Verbindungen annimmt und nie antwortet.
fn silent_peer() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let mut keep = Vec::new();
        for s in l.incoming().flatten() {
            keep.push(s);
        }
    });
    port
}

fn vault_with(dir: &std::path::Path, user: &str, password: &str) -> (Vault, uuid::Uuid) {
    let mut v = Vault::create(&dir.join(format!("t-{}.kdbx", uuid::Uuid::new_v4())), MasterKey::password("pw"), KdfChoice::Weak, rj_core::config::IndexStorage::KdbxCustomData);
    let e = v
        .add_credential(CredentialData { title: "Konto".into(), username: user.into(), password: Secret::from_str_exact(password), ..Default::default() })
        .unwrap();
    (v, e)
}

#[test]
fn slow_peer_does_not_block_ui() {
    let dir = env();
    let slow = silent_peer();
    let (vault, entry) = vault_with(dir, "benutzer", "geheim");
    let mut idx = Index::default();
    let tid = uuid::Uuid::new_v4();
    idx.tunnels.push(TunnelDef { id: tid, name: "Langsam".into(), kind: TunnelKind::Ssh { hops: vec![SshHop { host: "127.0.0.1".into(), port: Some(slow), credential: None }] } });
    let mut f = Folder::new("Langsam");
    f.settings.credentials.insert("default".into(), CredentialRef { entry, last_title: None });
    let mut t = Target::new("hinter-tunnel", Protocol::Ssh, "srv.example.test");
    t.settings.tunnel = Some(TunnelSetting::Use(tid));
    let target = t.id;
    let mut pve = Target::new("pve", Protocol::Proxmox, "127.0.0.1");
    pve.port = Some(slow);
    let pve_id = pve.id;
    f.children.push(Node::Target(t));
    f.children.push(Node::Target(pve));
    idx.root.children.push(Node::Folder(f));

    let mut app = App::new(false);
    app.index = idx;
    app.vault = Some(vault);
    app.sessions = Manager::new(workers(), Reconnect { auto: false, max: Duration::from_secs(60) });

    let t0 = Instant::now();
    let r: serde_json::Value = serde_json::from_str(&app.connect(target, None, false, 80, 24)).unwrap();
    let connect_ms = t0.elapsed().as_millis();
    let job = r["pending"].as_u64().unwrap_or_else(|| panic!("{r}"));
    let t1 = Instant::now();
    let g: serde_json::Value = serde_json::from_str(&app.guests_json(pve_id)).unwrap();
    let guests_ms = t1.elapsed().as_millis();
    assert!(g["pending"].is_u64(), "{g}");
    // Ein dritter Auftrag wird sofort abgebrochen.
    let c: serde_json::Value = serde_json::from_str(&app.connect(target, None, false, 80, 24)).unwrap();
    let cancelled = c["pending"].as_u64().unwrap();
    assert!(app.edit(&serde_json::json!({ "op": "cancel_job", "job": cancelled }).to_string()).contains("ok"));
    assert!(!app.jobs.json().to_string().contains(&cancelled.to_string()));

    // Bedienbarkeit messen, bis beide Aufgaben fertig sind.
    let mut worst = Duration::ZERO;
    let mut events = Vec::new();
    let start = Instant::now();
    while !app.jobs.is_empty() {
        let s = Instant::now();
        events.extend(app.poll());
        let _ = app.tree_json("");
        let _ = app.edit(r#"{"op":"views"}"#);
        worst = worst.max(s.elapsed());
        assert!(start.elapsed() < Duration::from_secs(40), "Aufgaben enden nicht");
        std::thread::sleep(Duration::from_millis(16));
    }
    let total = start.elapsed();
    println!(
        "MESSWERT langsame Gegenstelle: Verbinden kehrt nach {connect_ms} ms zurück, Gäste-Abruf nach {guests_ms} ms; \
         längster Takt der Oberfläche {} ms während {:.1} s Wartezeit",
        worst.as_millis(),
        total.as_secs_f64()
    );
    assert!(connect_ms < 200 && guests_ms < 200, "Aufrufe kehren sofort zurück");
    assert!(worst < Duration::from_millis(250), "Takt blockiert ({worst:?})");
    assert!(total >= Duration::from_secs(10), "Gegenstelle war nicht langsam ({total:?})");
    assert!(events.iter().any(|e| matches!(e, UiEvent::JobFailed { job: j, .. } if *j == job)), "{events:?}");
    assert!(events.iter().any(|e| matches!(e, UiEvent::JobResult { kind, json, .. } if kind == "guests" && json.contains("error"))), "{events:?}");
    assert!(!events.iter().any(|e| matches!(e, UiEvent::JobFailed { job: j, .. } | UiEvent::SessionOpened { job: j, .. } if *j == cancelled)), "abgebrochen bleibt still");
    assert!(app.sessions.ids().is_empty());
}

/// Durchsucht den lesbaren Speicher des eigenen Prozesses nach einem Muster.
/// Das Muster liegt nur mit 0x5A verknüpft vor, damit die Suche es nicht
/// selbst in den Speicher legt.
fn find_in_memory(masked: &[u8]) -> Vec<String> {
    let maps = std::fs::read_to_string("/proc/self/maps").unwrap();
    let mut mem = std::fs::File::open("/proc/self/mem").unwrap();
    let mut hits = Vec::new();
    let mut buf = vec![0u8; 1 << 20];
    for line in maps.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 2 || !cols[1].starts_with('r') {
            continue;
        }
        let name = cols.get(5).copied().unwrap_or("");
        if name.starts_with("[vvar") || name.starts_with("[vsyscall") {
            continue;
        }
        let (a, b) = cols[0].split_once('-').unwrap();
        let (start, end) = (u64::from_str_radix(a, 16).unwrap(), u64::from_str_radix(b, 16).unwrap());
        let mut pos = start;
        while pos < end {
            let len = ((end - pos) as usize).min(buf.len());
            use std::io::Seek;
            if mem.seek(std::io::SeekFrom::Start(pos)).is_err() {
                break;
            }
            let n = match mem.read(&mut buf[..len]) {
                Ok(n) if n > 0 => n,
                _ => break,
            };
            if n >= masked.len() {
                for i in 0..=n - masked.len() {
                    if buf[i] ^ 0x5A == masked[0] && buf[i..i + masked.len()].iter().zip(masked).all(|(x, m)| x ^ 0x5A == *m) {
                        hits.push(format!("{:#x} {name}", pos + i as u64));
                    }
                }
            }
            buf[..n].fill(0);
            // Überlappung, damit Treffer an Blockgrenzen nicht verloren gehen.
            pos += (n as u64).saturating_sub(masked.len() as u64).max(1);
        }
    }
    hits
}

#[test]
#[cfg(target_os = "linux")]
fn lock_removes_plaintext_passwords_from_memory() {
    use zeroize::Zeroize;
    let dir = env();
    let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().subsec_nanos();
    // Kennwörter erst zur Laufzeit zusammensetzen (nicht als Ganzes im Programm).
    let mut entry_pw = format!("Kw{}-{:08x}-{}", "Prüfung", nanos, "ß🚀");
    let mut master_pw = format!("Hs{}-{:08x}", "Schlüssel", nanos ^ 0x55AA);
    let entry_mask: Vec<u8> = entry_pw.bytes().map(|b| b ^ 0x5A).collect();
    let master_mask: Vec<u8> = master_pw.bytes().map(|b| b ^ 0x5A).collect();
    let path = dir.join("speicher.kdbx");
    {
        let mut v = Vault::create(&path, MasterKey::password(&master_pw), KdfChoice::Weak, rj_core::config::IndexStorage::KdbxCustomData);
        let e = v
            .add_credential(CredentialData { title: "Konto".into(), username: "benutzer".into(), password: Secret::from_str_exact(&entry_pw), ..Default::default() })
            .unwrap();
        let mut idx = Index::default();
        let mut f = Folder::new("Server");
        f.settings.credentials.insert("default".into(), CredentialRef { entry: e, last_title: None });
        f.children.push(Node::Target(Target::new("srv", Protocol::Ssh, "127.0.0.1")));
        idx.root.children.push(Node::Folder(f));
        v.set_index(idx).unwrap();
        v.save(None).unwrap();
    }
    entry_pw.zeroize();
    let mut app = App::new(false);
    // Der Test darf die echte Konfiguration des Benutzers nicht verändern.
    app.config_path = None;
    // Worker gibt es hier nicht: die Sitzung behält nur ihre Zugangsdaten.
    app.sessions = Manager::new(std::path::PathBuf::from("/nicht/vorhanden"), Reconnect { auto: false, max: Duration::from_secs(60) });
    app.unlock(path.display().to_string(), Secret::from_str_exact(&master_pw), None);
    master_pw.zeroize();
    let start = Instant::now();
    while !app.poll().iter().any(|e| matches!(e, UiEvent::Unlocked { .. })) {
        assert!(start.elapsed() < Duration::from_secs(20), "nicht entsperrt");
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut target = None;
    app.index.for_each_target(|t| target = Some(t.id));
    let r = app.connect(target.unwrap(), None, false, 80, 24);
    assert!(r.contains("session"), "{r}");
    for _ in 0..5 {
        app.poll();
    }
    let before = find_in_memory(&entry_mask);
    assert!(!before.is_empty(), "Prüfung findet das Kennwort vor dem Sperren nicht (Suche wirkungslos?)");
    app.lock(false);
    for _ in 0..5 {
        app.poll();
    }
    let entry_hits = find_in_memory(&entry_mask);
    let master_hits = find_in_memory(&master_mask);
    println!("MESSWERT Speicherprüfung: vor dem Sperren {} Fundstellen, danach Eintragskennwort {:?}, Hauptkennwort {:?}", before.len(), entry_hits, master_hits);
    assert!(entry_hits.is_empty(), "Eintragskennwort nach dem Sperren im Speicher: {entry_hits:?}");
    assert!(master_hits.is_empty(), "Hauptkennwort nach dem Sperren im Speicher: {master_hits:?}");
}

#[test]
fn simulated_wake_does_not_lock_and_resets_timer() {
    env();
    let mut app = App::new(true);
    // Sperre nach 1 Minute; 70 s Leerlauf, davor eine Stunde Standby
    // (Systemuhr springt, monotone Uhr nicht).
    let now = Instant::now();
    app.idle = rj_ui::idle::IdleWatch::new(1, now - Duration::from_secs(70), SystemTime::now() - Duration::from_secs(70 + 3600));
    let ev = app.poll();
    assert!(ev.iter().any(|e| matches!(e, UiEvent::Resumed)), "{ev:?}");
    assert!(!ev.iter().any(|e| matches!(e, UiEvent::Locked { .. })), "nach dem Aufwachen nie sperren");
    assert!(app.idle.idle_for(Instant::now()) < Duration::from_secs(1), "Zeitgeber neu gestartet");
    // Meldung des Systems ebenso.
    app.idle = rj_ui::idle::IdleWatch::new(1, now - Duration::from_secs(59), SystemTime::now() - Duration::from_secs(59));
    let ev = app.on_resume();
    assert!(matches!(ev[..], [UiEvent::Resumed]));
    assert!(!app.poll().iter().any(|e| matches!(e, UiEvent::Locked { .. })));
    assert!(app.idle.idle_for(Instant::now()) < Duration::from_secs(1));
}

#[test]
fn wake_reconnects_dropped_session_tunnel_first() {
    let dir = env();
    let sshd = match rj_testkit::sshd::Sshd::start() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ÜBERSPRUNGEN: {}", e.0);
            return;
        }
    };
    let kh = rj_ssh::known_hosts::KnownHosts::default_location().unwrap();
    kh.remember_openssh("127.0.0.1", sshd.port, &sshd.host_key_pub()).unwrap();
    let (vault, entry) = vault_with(dir, rj_testkit::sshd::USER, rj_testkit::sshd::PASSWORD);
    // Ziel: derselbe sshd, erreicht über einen SSH-Tunnel mit ihm als Sprungserver.
    let mut idx = Index::default();
    let tid = uuid::Uuid::new_v4();
    idx.tunnels.push(TunnelDef { id: tid, name: "Sprung".into(), kind: TunnelKind::Ssh { hops: vec![SshHop { host: "127.0.0.1".into(), port: Some(sshd.port), credential: None }] } });
    let mut f = Folder::new("Hinter Sprung");
    f.settings.credentials.insert("default".into(), CredentialRef { entry, last_title: None });
    f.settings.tunnel = Some(TunnelSetting::Use(tid));
    let mut t = Target::new("ziel", Protocol::Ssh, "127.0.0.1");
    t.port = Some(sshd.port);
    let target = t.id;
    f.children.push(Node::Target(t));
    idx.root.children.push(Node::Folder(f));
    let mut app = App::new(false);
    app.index = idx;
    app.vault = Some(vault);
    app.sessions = Manager::new(workers(), Reconnect { auto: false, max: Duration::from_secs(60) });

    let wait = |app: &mut App, what: &str, events: &mut Vec<UiEvent>, cond: &dyn Fn(&App, &[UiEvent]) -> bool| {
        let start = Instant::now();
        loop {
            events.extend(app.poll());
            if cond(app, events) {
                return;
            }
            assert!(start.elapsed() < Duration::from_secs(30), "{what}: Zeitüberschreitung\n{events:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let r: serde_json::Value = serde_json::from_str(&app.connect(target, None, false, 80, 24)).unwrap();
    assert!(r["pending"].is_u64(), "{r}");
    let mut events = Vec::new();
    wait(&mut app, "verbinden", &mut events, &|a, _| a.sessions.ids().first().is_some_and(|s| a.sessions.state(*s) == Some("connected")));
    let s = app.sessions.ids()[0];

    // Aufwachen melden, dann bricht die Verbindung ab (Netz kommt verzögert zurück).
    let ev = app.on_resume();
    assert!(matches!(ev[..], [UiEvent::Resumed]));
    events.clear();
    sshd.drop_sessions();
    wait(&mut app, "neu verbunden", &mut events, &|a, ev| {
        a.sessions.state(s) == Some("connected") && ev.iter().any(|e| matches!(e, UiEvent::State { state, .. } if state == "disconnected"))
    });
    let pos = |pred: &dyn Fn(&UiEvent) -> bool| events.iter().position(pred);
    let dropped = pos(&|e| matches!(e, UiEvent::State { state, .. } if state == "disconnected")).unwrap();
    let tunnel_done = pos(&|e| matches!(e, UiEvent::JobsChanged)).expect("Tunnel wurde neu aufgebaut");
    let reconnecting = events.iter().enumerate().skip(dropped).find(|(_, e)| matches!(e, UiEvent::State { state, .. } if state == "connecting")).map(|(i, _)| i).unwrap();
    assert!(tunnel_done < reconnecting, "Tunnel zuerst: {events:?}");
    wait(&mut app, "Hinweis", &mut events, &|_, ev| ev.iter().any(|e| matches!(e, UiEvent::Notice { text, .. } if text == "Verbindung neu aufgebaut")));
    app.close_session(s);
}
