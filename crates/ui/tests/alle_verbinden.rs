//! „Alle im Ordner verbinden“ gegen 10 Ziele: 9 verweisen auf einen lokalen
//! sshd, eines auf einen geschlossenen Port. Der Aufbau ist gestaffelt
//! (höchstens 4 gleichzeitig), das ausfallende Ziel stört die übrigen nicht.

use std::time::{Duration, Instant};

use rj_core::{CredentialRef, Folder, Index, Node, Protocol, Secret, Target};
use rj_kdbx::{CredentialData, KdfChoice, MasterKey, Vault};
use rj_ui::app::App;
use rj_ui::sessions::{Manager, Reconnect};

#[test]
fn connect_all_ten_targets_staggered() {
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
    // Host-Schlüssel des Test-sshd vorab bekannt machen.
    let kh = rj_ssh::known_hosts::KnownHosts::default_location().unwrap();
    kh.remember_openssh("127.0.0.1", sshd.port, &sshd.host_key_pub()).unwrap();

    let mut vault = Vault::create(&dir.path().join("t.kdbx"), MasterKey::password("pw"), KdfChoice::Weak, rj_core::config::IndexStorage::KdbxCustomData);
    let entry = vault
        .add_credential(CredentialData {
            title: "Testkonto".into(),
            username: rj_testkit::sshd::USER.into(),
            password: Secret::from_str_exact(rj_testkit::sshd::PASSWORD),
            ..Default::default()
        })
        .unwrap();
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut idx = Index::default();
    let mut f = Folder::new("Zehn Ziele");
    f.settings.credentials.insert("default".into(), CredentialRef { entry, last_title: Some("Testkonto".into()) });
    let mut bad = None;
    for i in 0..10 {
        let mut t = Target::new(format!("ziel-{i:02}"), Protocol::Ssh, "127.0.0.1");
        t.port = Some(if i == 3 { closed } else { sshd.port });
        if i == 3 {
            bad = Some(t.id);
        }
        f.children.push(Node::Target(t));
    }
    let fid = f.id;
    idx.root.children.push(Node::Folder(f));

    let mut app = App::new(false);
    app.index = idx;
    app.vault = Some(vault);
    let workers = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_path_buf();
    app.sessions = Manager::new(workers, Reconnect { auto: false, max: Duration::from_secs(60) });

    let r = app.edit(&serde_json::json!({ "op": "connect_all", "id": fid, "parallel": 4 }).to_string());
    assert!(r.contains("\"count\":10"), "{r}");
    let start = Instant::now();
    loop {
        app.poll();
        let done = app.stagger_done.is_some();
        let pending = app.sessions.ids().iter().any(|s| matches!(app.sessions.state(*s), Some("") | Some("connecting")));
        if done && !pending {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "Zeitüberschreitung");
        std::thread::sleep(Duration::from_millis(20));
    }
    let st = app.stagger_done.as_ref().unwrap();
    eprintln!("höchstens gleichzeitig: {}, Dauer {:?}", st.peak, start.elapsed());
    assert!(st.peak <= 4, "höchstens 4 gleichzeitig, beobachtet {}", st.peak);
    let mut connected = 0;
    let mut failed = Vec::new();
    for s in app.sessions.ids() {
        match app.sessions.state(s) {
            Some("connected") => connected += 1,
            other => failed.push((app.sessions.target(s), other.map(str::to_string))),
        }
    }
    assert_eq!(connected, 9, "{failed:?}");
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].0, bad);
    // Alle trennen.
    let r = app.edit(&serde_json::json!({ "op": "connect_all", "id": fid, "mode": "disconnect" }).to_string());
    assert!(r.contains("\"count\":10"), "{r}");
}
