//! Update über das Hauptprogramm gegen eine Attrappe: Prüfen und Installieren als
//! Hintergrundaufgabe, Rückfrage bei offenen Sitzungen. Die Quelle ist im Programm fest
//! GitHub; der Test setzt sie über `source_override` auf die Attrappe.

use std::time::{Duration, Instant};

use rj_testkit::gitea::GiteaMock;
use rj_ui::app::App;
use rj_ui::sessions::UiEvent;
use rj_update::release::{Source, SourceKind};
use rj_update::sign::{self, SIG, SUMS};
use rj_update::{archive, asset_name, install};
use serde_json::{json, Value};

fn wait_result(app: &mut App, kind: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        for e in app.poll() {
            if let UiEvent::JobResult { kind: k, json, .. } = e {
                if k == kind {
                    return serde_json::from_str(&json).unwrap();
                }
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("kein Ergebnis {kind}");
}

fn edit(app: &mut App, v: Value) -> Value {
    serde_json::from_str(&app.edit(&v.to_string())).unwrap()
}

#[test]
fn check_and_install_from_the_fixed_source() {
    let dir = tempfile::tempdir().unwrap();
    // Paket 9.0.0 mit eigenem Testschlüssel.
    let key = sign::generate().unwrap();
    let prog = tempfile::tempdir().unwrap();
    std::fs::write(prog.path().join(install::main_program()), b"neu").unwrap();
    let data = archive::pack_tar_zst(prog.path()).unwrap();
    let name = asset_name("9.0.0", &rj_update::platform());
    let sums = sign::sums_text(&[(name.clone(), sign::sha256_hex(&data))]);
    let mock = GiteaMock::start("team/remotejoe");
    {
        let mut st = mock.state.lock().unwrap();
        st.files.insert(name.clone(), data);
        st.files.insert(SUMS.into(), sums.clone().into_bytes());
        st.files.insert(SIG.into(), sign::sign(&key, sums.as_bytes()).into_bytes());
    }
    mock.set_release("v9.0.0", "Änderungen\n- Update-Tab", &[&name, SUMS, SIG]);

    let mut app = App::new(false);
    app.config_path = None;
    let target = dir.path().join("remotejoe");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join(install::main_program()), b"alt").unwrap();
    app.update.dir_override = Some(target.clone());
    app.update.key_override = Some(sign::public_key_text(&key.verifying_key()));

    // Die Quelle ist fest GitHub; im Status steht keine Quelle zur Auswahl mehr.
    let st = app.update_status();
    assert!(st.get("source").is_none() && st.get("repo").is_none() && st.get("token_entry").is_none(), "{st}");
    assert_eq!(rj_ui::update::github_source().repo, "joeMJ/remotejoe");
    assert_eq!(rj_ui::update::github_source().kind, SourceKind::Github);
    // Nur die Prüfung beim Start ist einstellbar.
    let r = edit(&mut app, json!({"op": "update_settings", "check_on_start": true, "source": "gitea", "repo": "fremd/repo"}));
    assert_eq!(r["check_on_start"], true, "{r}");
    assert!(app.config.update.check_on_start);
    // Die Vorführquelle gibt es nur im Vorführbetrieb.
    assert!(edit(&mut app, json!({"op": "update_demo_source", "base_url": mock.base}))["error"].is_string());
    app.update.source_override = Some(Source { kind: SourceKind::Gitea, base_url: mock.base.clone(), repo: "team/remotejoe".into() });

    edit(&mut app, json!({"op": "update_check"}));
    let r = wait_result(&mut app, "update_check");
    assert_eq!(r["state"], "newer", "{r}");
    assert_eq!(r["found"]["version"], "9.0.0");
    assert_eq!(r["found"]["notes"], "Änderungen\n- Update-Tab");
    assert!(mock.state.lock().unwrap().requests.iter().any(|(p, h)| p.ends_with("/releases/latest") && !h.iter().any(|(k, _)| k == "authorization")), "keine Zugangsdaten an die Quelle");

    // Offene Sitzung: erst Bestätigung.
    app.sessions.open_demo_frame("rdp-001", 8, 8, 1);
    let r = edit(&mut app, json!({"op": "update_install"}));
    assert_eq!(r["state"], "confirm", "{r}");
    assert_eq!(std::fs::read(target.join(install::main_program())).unwrap(), b"alt");
    edit(&mut app, json!({"op": "update_install", "confirm": true}));
    let r = wait_result(&mut app, "update_install");
    assert_eq!(r["state"], "installed", "{r}");
    assert_eq!(std::fs::read(target.join(install::main_program())).unwrap(), b"neu");
    assert_eq!(std::fs::read(dir.path().join("remotejoe.alt").join(install::main_program())).unwrap(), b"alt");
}
