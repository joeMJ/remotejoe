//! Makros in einer echten SSH-Sitzung (lokaler sshd): Eingaben, Warten auf Ausgabe,
//! Platzhalter für Zugangsdaten, Fehlschlag mit Grund, Abbruch.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use rj_core::{CredentialRef, Folder, Index, Node, Protocol, Secret, Target};
use rj_kdbx::{CredentialData, KdfChoice, MasterKey, Vault};
use rj_ui::app::App;
use rj_ui::sessions::{Manager, Reconnect, UiEvent};
use serde_json::{json, Value};

#[global_allocator]
static ALLOC: rj_ui::alloc::ZeroizingAllocator = rj_ui::alloc::ZeroizingAllocator;

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

fn cmd(app: &mut App, v: Value) -> Value {
    serde_json::from_str(&app.edit(&v.to_string())).unwrap()
}

/// Wartet, bis `cond` erfüllt ist; sammelt dabei alle Ereignisse.
fn wait(app: &mut App, what: &str, events: &mut Vec<UiEvent>, cond: &dyn Fn(&App, &[UiEvent]) -> bool) {
    let start = Instant::now();
    loop {
        events.extend(app.poll());
        if cond(app, events) {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(30), "{what}: Zeitüberschreitung\n{events:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn done(events: &[UiEvent], name: &str) -> Option<(bool, String)> {
    events.iter().find_map(|e| match e {
        UiEvent::MacroDone { name: n, ok, message, .. } if n == name => Some((*ok, message.clone())),
        _ => None,
    })
}

#[test]
fn macros_run_in_a_real_ssh_session() {
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
    let mut v = Vault::create(&dir.join("m.kdbx"), MasterKey::password("pw"), KdfChoice::Weak, rj_core::config::IndexStorage::KdbxCustomData);
    let entry = v
        .add_credential(CredentialData { title: "Konto".into(), username: rj_testkit::sshd::USER.into(), password: Secret::from_str_exact(rj_testkit::sshd::PASSWORD), ..Default::default() })
        .unwrap();
    let mut idx = Index::default();
    let mut f = Folder::new("Server");
    f.settings.credentials.insert("default".into(), CredentialRef { entry, last_title: None });
    let mut t = Target::new("ziel", Protocol::Ssh, "127.0.0.1");
    t.port = Some(sshd.port);
    let target = t.id;
    f.children.push(Node::Target(t));
    idx.root.children.push(Node::Folder(f));
    let mut app = App::new(false);
    app.config_path = None;
    app.index = idx;
    app.vault = Some(v);
    app.sessions = Manager::new(workers(), Reconnect { auto: false, max: Duration::from_secs(60) });

    // Makros anlegen und dem Ziel zuordnen.
    let ok_macro = cmd(&mut app, json!({ "op": "macro_new" }))["macro"].as_str().unwrap().to_string();
    cmd(&mut app, json!({ "op": "update_macro", "id": ok_macro, "name": "Rechnen", "steps": [
        { "step": "send", "text": "echo RJ_$((20+22)){key:enter}" },
        { "step": "wait", "pattern": { "text": "RJ_42" }, "timeout_ms": 10000 },
        { "step": "send", "text": "echo ICH_{cred:username}{key:enter}" },
        { "step": "wait", "pattern": { "text": format!("ICH_{}", rj_testkit::sshd::USER) }, "timeout_ms": 10000 }] }));
    let bad_macro = cmd(&mut app, json!({ "op": "macro_new" }))["macro"].as_str().unwrap().to_string();
    cmd(&mut app, json!({ "op": "update_macro", "id": bad_macro, "name": "Fehlt", "steps": [
        { "step": "wait", "pattern": { "text": "GIBT_ES_NIE" }, "timeout_ms": 500 }] }));
    let slow_macro = cmd(&mut app, json!({ "op": "macro_new" }))["macro"].as_str().unwrap().to_string();
    cmd(&mut app, json!({ "op": "update_macro", "id": slow_macro, "name": "Lang", "steps": [
        { "step": "wait", "pattern": { "text": "NIEMALS" }, "timeout_ms": 60000 }] }));
    for m in [&ok_macro, &bad_macro, &slow_macro] {
        cmd(&mut app, json!({ "op": "macro_link", "id": m, "node": target.to_string() }));
    }

    let r: Value = serde_json::from_str(&app.connect(target, None, false, 100, 30)).unwrap();
    assert!(r["session"].is_u64(), "{r}");
    let s = r["session"].as_u64().unwrap();
    let mut events = Vec::new();
    wait(&mut app, "verbinden", &mut events, &|a, _| a.sessions.state(s) == Some("connected"));
    // Eingabeaufforderung abwarten, damit die Eingaben des Makros nicht vor der Shell ankommen.
    wait(&mut app, "Shell bereit", &mut events, &|a, _| a.sessions.screen_text(s).is_some_and(|t| t.contains('$') || t.contains('#')));

    // Für die Sitzung gelten die drei zugeordneten Makros.
    let list = cmd(&mut app, json!({ "op": "session_macros", "session": s }));
    assert_eq!(list.as_array().unwrap().len(), 3, "{list}");

    // 1. Erfolgreicher Lauf mit Platzhalter für den Benutzernamen.
    let r = cmd(&mut app, json!({ "op": "run_macro", "session": s, "macro": ok_macro }));
    assert_eq!(r["started"], true, "{r}");
    // Zweiter Start während des Laufs wird abgelehnt.
    let again = cmd(&mut app, json!({ "op": "run_macro", "session": s, "macro": ok_macro }));
    assert!(again["error"].as_str().unwrap().contains("läuft schon"), "{again}");
    wait(&mut app, "Makro Rechnen", &mut events, &|_, ev| done(ev, "Rechnen").is_some());
    let (ok, msg) = done(&events, "Rechnen").unwrap();
    assert!(ok, "{msg}");
    let screen = app.sessions.screen_text(s).unwrap();
    assert!(screen.contains("RJ_42"), "{screen}");

    // 2. Fehlschlag mit Grund (Zeitüberschreitung).
    let r = cmd(&mut app, json!({ "op": "run_macro", "session": s, "macro": bad_macro }));
    assert_eq!(r["started"], true, "{r}");
    wait(&mut app, "Makro Fehlt", &mut events, &|_, ev| done(ev, "Fehlt").is_some());
    let (ok, msg) = done(&events, "Fehlt").unwrap();
    assert!(!ok && msg.contains("Zeitüberschreitung"), "{msg}");

    // 3. Abbruch eines langen Makros.
    let r = cmd(&mut app, json!({ "op": "run_macro", "session": s, "macro": slow_macro }));
    assert_eq!(r["started"], true, "{r}");
    std::thread::sleep(Duration::from_millis(300));
    cmd(&mut app, json!({ "op": "cancel_macro", "session": s }));
    wait(&mut app, "Makro Lang", &mut events, &|_, ev| done(ev, "Lang").is_some());
    let (ok, msg) = done(&events, "Lang").unwrap();
    assert!(!ok && msg.contains("abgebrochen"), "{msg}");

    // Das Kennwort taucht in keiner Meldung auf.
    for e in &events {
        if let UiEvent::MacroDone { message, .. } = e {
            assert!(!message.contains(rj_testkit::sshd::PASSWORD));
        }
    }
    app.close_session(s);
}
