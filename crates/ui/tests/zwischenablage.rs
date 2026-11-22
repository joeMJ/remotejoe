//! „Benutzernamen/Kennwort kopieren“: wirksame Zugangsdaten eines Ziels (auch geerbt), Fehlerfälle.

use rj_core::{CredentialRef, Folder, Index, Node, Protocol, Secret, Target};
use rj_kdbx::{CredentialData, KdfChoice, MasterKey, Vault};
use rj_ui::app::App;

#[test]
fn credential_copy_returns_effective_values_and_reports_problems() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_DATA_HOME", dir.path().join("data"));
    std::env::set_var("XDG_CONFIG_HOME", dir.path().join("config"));
    let mut vault = Vault::create(&dir.path().join("t.kdbx"), MasterKey::password("pw"), KdfChoice::Weak, rj_core::config::IndexStorage::KdbxCustomData);
    let entry = vault
        .add_credential(CredentialData {
            title: "Konto".into(),
            username: "benutzer1".into(),
            password: Secret::from_str_exact("geheim-äöü-1"),
            ..Default::default()
        })
        .unwrap();
    let mut idx = Index::default();
    let mut mit = Folder::new("mit Zugang");
    mit.settings.credentials.insert("default".into(), CredentialRef { entry, last_title: Some("Konto".into()) });
    let t1 = Target::new("geerbt", Protocol::Ssh, "host1.example.test");
    let t1id = t1.id;
    mit.children.push(Node::Target(t1));
    let mut ohne = Folder::new("ohne Zugang");
    let t2 = Target::new("leer", Protocol::Ssh, "host2.example.test");
    let t2id = t2.id;
    ohne.children.push(Node::Target(t2));
    let fid = mit.id;
    idx.root.children.push(Node::Folder(mit));
    idx.root.children.push(Node::Folder(ohne));

    let mut app = App::new(true);
    app.index = idx;
    // Gesperrt: keine Werte.
    let v: serde_json::Value = serde_json::from_str(&app.credential_copy_json(&t1id.to_string(), "password")).unwrap();
    assert!(v["error"].as_str().unwrap().contains("gesperrt"), "{v}");

    app.vault = Some(vault);
    let get = |app: &App, id: &str, kind: &str| serde_json::from_str::<serde_json::Value>(&app.credential_copy_json(id, kind)).unwrap();
    let u = get(&app, &t1id.to_string(), "user");
    assert_eq!(u["text"], "benutzer1");
    let p = get(&app, &t1id.to_string(), "password");
    assert_eq!(p["text"], "geheim-äöü-1");
    assert_eq!(p["clear_secs"], 30, "Vorgabe: 30 Sekunden");
    // Ziel ohne Zugangsdaten, Ordner, unbekannte Angabe, ungültige Kennung.
    assert!(get(&app, &t2id.to_string(), "password")["error"].as_str().unwrap().contains("keine Zugangsdaten"));
    assert!(get(&app, &fid.to_string(), "user")["error"].as_str().unwrap().contains("kein Verbindungsziel"));
    assert!(get(&app, &t1id.to_string(), "pin")["error"].is_string());
    assert!(get(&app, "kein-uuid", "user")["error"].is_string());
    // Frist ändern.
    app.set_clipboard_clear_secs(0);
    assert_eq!(get(&app, &t1id.to_string(), "password")["clear_secs"], 0);
}
