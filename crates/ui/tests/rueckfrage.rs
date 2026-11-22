//! Host-Schlüssel-Rückfrage: ohne Antwort wird nach Ablauf der Frist
//! abgelehnt, beim Schließen der Sitzung ebenfalls. Gegen einen lokalen sshd
//! (OpenSSH) mit leerer Liste bekannter Host-Schlüssel.

use std::time::{Duration, Instant};

use rj_core::{CredentialRef, Folder, Index, Node, Protocol, Secret, Target};
use rj_kdbx::{CredentialData, KdfChoice, MasterKey, Vault};
use rj_ui::app::App;
use rj_ui::sessions::{Manager, Reconnect, UiEvent};

fn wait(app: &mut App, what: &str, mut f: impl FnMut(&UiEvent) -> bool) -> UiEvent {
    let start = Instant::now();
    loop {
        for e in app.poll() {
            if f(&e) {
                return e;
            }
        }
        assert!(start.elapsed() < Duration::from_secs(30), "Zeitüberschreitung: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn host_key_question_rejects_on_timeout_and_on_close() {
    let sshd = match rj_testkit::sshd::Sshd::start() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ÜBERSPRUNGEN: {}", e.0);
            return;
        }
    };
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_DATA_HOME", dir.path().join("data"));
    std::env::set_var("XDG_CONFIG_HOME", dir.path().join("config"));
    let mut vault = Vault::create(&dir.path().join("t.kdbx"), MasterKey::password("pw"), KdfChoice::Weak, rj_core::config::IndexStorage::KdbxCustomData);
    let entry = vault
        .add_credential(CredentialData {
            title: "Testkonto".into(),
            username: rj_testkit::sshd::USER.into(),
            password: Secret::from_str_exact(rj_testkit::sshd::PASSWORD),
            ..Default::default()
        })
        .unwrap();
    let mut idx = Index::default();
    let mut f = Folder::new("Rückfrage");
    f.settings.credentials.insert("default".into(), CredentialRef { entry, last_title: Some("Testkonto".into()) });
    let mut t = Target::new("ziel", Protocol::Ssh, "127.0.0.1");
    t.port = Some(sshd.port);
    let tid = t.id;
    f.children.push(Node::Target(t));
    idx.root.children.push(Node::Folder(f));
    let mut app = App::new(false);
    app.index = idx;
    app.vault = Some(vault);
    let workers = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_path_buf();
    app.sessions = Manager::new(workers, Reconnect { auto: false, max: Duration::from_secs(60) });
    app.sessions.question_timeout = Duration::from_secs(2);
    let kh = rj_ssh::known_hosts::KnownHosts::default_location().unwrap();

    // 1. Keine Antwort: nach der Frist abgelehnt, Verbindung scheitert.
    let r = app.connect(tid, None, false, 80, 24);
    assert!(r.contains("session"), "{r}");
    let q = wait(&mut app, "Rückfrage", |e| matches!(e, UiEvent::Question { .. }));
    let UiEvent::Question { s, info, timeout_secs, .. } = q else { unreachable!() };
    assert_eq!(timeout_secs, 2);
    assert_eq!(info.kind, "hostkey");
    assert!(info.fingerprint.starts_with("SHA256:") && info.fingerprint.len() > 40, "{info:?}");
    assert!(!info.key_type.is_empty() && info.previous.is_none());
    let asked = Instant::now();
    let closed = wait(&mut app, "Frist", |e| matches!(e, UiEvent::QuestionClosed { .. }));
    assert!(matches!(&closed, UiEvent::QuestionClosed { s: cs, reason, .. } if *cs == s && reason == "timeout"), "{closed:?}");
    assert!(asked.elapsed() >= Duration::from_millis(1900));
    wait(&mut app, "Abbruch", |e| matches!(e, UiEvent::State { s: es, state, .. } if *es == s && state == "failed"));
    // Eine späte Zustimmung wirkt nicht mehr.
    app.sessions.answer(s, 1, true);
    assert_eq!(app.sessions.open_questions(), 0);
    assert!(!std::fs::read_to_string(kh.path()).unwrap_or_default().contains("ssh-"), "nichts gespeichert");
    app.close_session(s);

    // 2. Sitzung (Tab) schließen während der Rückfrage: abgelehnt.
    app.sessions.question_timeout = Duration::from_secs(60);
    let r = app.connect(tid, None, false, 80, 24);
    assert!(r.contains("session"), "{r}");
    let UiEvent::Question { s, .. } = wait(&mut app, "zweite Rückfrage", |e| matches!(e, UiEvent::Question { .. })) else { unreachable!() };
    app.close_session(s);
    let closed = wait(&mut app, "Schließen", |e| matches!(e, UiEvent::QuestionClosed { .. }));
    assert!(matches!(&closed, UiEvent::QuestionClosed { reason, .. } if reason == "closed"), "{closed:?}");
    assert_eq!(app.sessions.open_questions(), 0);
    assert!(!std::fs::read_to_string(kh.path()).unwrap_or_default().contains("ssh-"), "nichts gespeichert");
}
