//! Update über das Hauptprogramm gegen die Gitea-Attrappe: Zugriffsschlüssel
//! aus einem Eintrag der Schlüsseldatenbank, Prüfen und Installieren als
//! Hintergrundaufgabe, Rückfrage bei offenen Sitzungen.

use std::time::{Duration, Instant};

use rj_core::Secret;
use rj_kdbx::{CredentialData, KdfChoice, MasterKey, Vault};
use rj_testkit::gitea::GiteaMock;
use rj_ui::app::App;
use rj_ui::sessions::UiEvent;
use rj_update::sign::{self, SIG, SUMS};
use rj_update::{archive, asset_name, install};
use serde_json::{json, Value};

const TOKEN: &str = "gitea-Prüfschlüssel ä\"'\\";

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
fn check_and_install_with_token_from_vault() {
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
        st.token = Some(TOKEN.into());
        st.files.insert(name.clone(), data);
        st.files.insert(SUMS.into(), sums.clone().into_bytes());
        st.files.insert(SIG.into(), sign::sign(&key, sums.as_bytes()).into_bytes());
    }
    mock.set_release("v9.0.0", "Änderungen\n- Update-Tab", &[&name, SUMS, SIG]);

    let mut vault = Vault::create(&dir.path().join("t.kdbx"), MasterKey::password("pw"), KdfChoice::Weak, rj_core::config::IndexStorage::KdbxCustomData);
    let entry = vault
        .add_credential(CredentialData { title: "Gitea-Zugriff".into(), username: "update".into(), password: Secret::from_str_exact(TOKEN), ..Default::default() })
        .unwrap();
    let mut app = App::new(false);
    app.config_path = None;
    let target = dir.path().join("remotejoe");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join(install::main_program()), b"alt").unwrap();
    app.update.dir_override = Some(target.clone());
    app.update.key_override = Some(sign::public_key_text(&key.verifying_key()));

    // Ohne Quelle: Fehler; nur https außer 127.0.0.1.
    assert!(edit(&mut app, json!({"op": "update_check"}))["error"].as_str().unwrap().contains("Quelle"));
    let r = edit(&mut app, json!({"op": "update_settings", "source": "gitea", "base_url": "http://git.example.test", "repo": "team/remotejoe"}));
    assert!(r["error"].as_str().unwrap().contains("https"), "{r}");
    let r = edit(&mut app, json!({"op": "update_settings", "source": "gitea", "base_url": mock.base, "repo": "team/remotejoe", "token_entry": entry.to_string()}));
    assert_eq!(r["source"], "gitea", "{r}");
    assert!(!r.to_string().contains(TOKEN));

    // Gesperrte Schlüsseldatenbank: klare Meldung, keine Anfrage.
    assert!(edit(&mut app, json!({"op": "update_check"}))["error"].as_str().unwrap().contains("gesperrt"));
    app.vault = Some(vault);
    assert_eq!(app.update_status()["token_title"], "Gitea-Zugriff");

    edit(&mut app, json!({"op": "update_check"}));
    let r = wait_result(&mut app, "update_check");
    assert_eq!(r["state"], "newer", "{r}");
    assert_eq!(r["found"]["version"], "9.0.0");
    assert_eq!(r["found"]["notes"], "Änderungen\n- Update-Tab");
    assert!(mock.state.lock().unwrap().requests.iter().any(|(p, h)| p.ends_with("/releases/latest") && h.contains(&("authorization".into(), format!("token {TOKEN}")))));

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
