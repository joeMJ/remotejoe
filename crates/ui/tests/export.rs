//! Export: JSON und CSV, mit und ohne Kennwörter, verschlüsseltes ZIP, Rundlauf über den Import.

use rj_core::{CredentialRef, Folder, Index, Node, Protocol, Secret, Target};
use rj_kdbx::{CredentialData, KdfChoice, MasterKey, Vault};
use rj_ui::app::App;
use serde_json::json;

fn app_with_data(dir: &std::path::Path) -> (App, Vec<String>) {
    std::env::set_var("XDG_DATA_HOME", dir.join("data"));
    std::env::set_var("XDG_CONFIG_HOME", dir.join("config"));
    let mut vault = Vault::create(&dir.join("t.kdbx"), MasterKey::password("pw"), KdfChoice::Weak, rj_core::config::IndexStorage::KdbxCustomData);
    let entry = vault
        .add_credential(CredentialData { title: "Konto".into(), username: "benutzer1".into(), password: Secret::from_str_exact("geheim-äöü-1"), ..Default::default() })
        .unwrap();
    let mut idx = Index::default();
    let mut f = Folder::new("Nord");
    f.settings.credentials.insert("default".into(), CredentialRef { entry, last_title: Some("Konto".into()) });
    let mut t1 = Target::new("rechner", Protocol::Rdp, "host1.example.test");
    t1.port = Some(3390);
    let t2 = Target::new("switch", Protocol::Ssh, "host2.example.test");
    let ids = vec![t1.id.to_string(), t2.id.to_string()];
    f.children.push(Node::Target(t1));
    f.children.push(Node::Target(t2));
    idx.root.children.push(Node::Folder(f));
    let mut app = App::new(true);
    app.index = idx;
    app.vault = Some(vault);
    (app, ids)
}

#[test]
fn export_roundtrip_json_csv_and_encrypted_zip() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, ids) = app_with_data(dir.path());
    let p = |n: &str| dir.path().join(n).display().to_string();

    // JSON ohne Kennwörter: nirgends ein Kennwort, wieder importierbar.
    let r = app.export_run(&json!({ "format": "json", "scope": "all", "path": p("a") })).unwrap();
    assert_eq!((r["targets"].as_u64(), r["folders"].as_u64(), r["zip"].as_bool()), (Some(2), Some(1), Some(false)));
    let text = std::fs::read_to_string(p("a.json")).unwrap();
    assert!(!text.contains("geheim") && text.contains("benutzer1"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(p("a.json")).unwrap().permissions().mode() & 0o777, 0o600, "nur für den Benutzer lesbar");
    }
    let doc = rj_importer::read_file(std::path::Path::new(&p("a.json"))).unwrap();
    let plan = rj_importer::plan::plan(&doc, "Rundlauf");
    assert_eq!(plan.report.targets, 2);

    // JSON mit Kennwörtern nur als ZIP, mit langem Kennwort; Rundlauf über den Import mit Kennwort.
    assert!(app.export_run(&json!({ "format": "json", "scope": "all", "passwords": true, "path": p("b") })).unwrap_err().contains("verschlüsseltes ZIP"));
    assert!(app.export_run(&json!({ "format": "json", "scope": "all", "passwords": true, "zip": true, "zip_password": "kurz", "path": p("b") })).unwrap_err().contains("mindestens 8"));
    let r = app.export_run(&json!({ "format": "json", "scope": "all", "passwords": true, "zip": true, "zip_password": "Zip-Kennwort-1", "path": p("b") })).unwrap();
    assert_eq!(r["path"].as_str().unwrap(), p("b.zip"));
    let bytes = std::fs::read(p("b.zip")).unwrap();
    assert!(!bytes.windows(8).any(|w| w == b"geheim-\xc3"), "Kennwort nicht im Klartext im ZIP");
    assert!(matches!(rj_importer::read_file(std::path::Path::new(&p("b.zip"))), Err(rj_importer::ImportError::PasswordNeeded)));
    assert!(matches!(rj_importer::read_file_with_password(std::path::Path::new(&p("b.zip")), Some("falsch")), Err(rj_importer::ImportError::PasswordWrong)));
    let doc = rj_importer::read_file_with_password(std::path::Path::new(&p("b.zip")), Some("Zip-Kennwort-1")).unwrap();
    let pw = doc.items.iter().find_map(|i| match i { rj_importer::SourceItem::Connection { credential: Some(c), .. } => c.password.clone(), _ => None }).unwrap();
    assert_eq!(pw.as_str(), Some("geheim-äöü-1"));

    // CSV: ohne Kennwörter, mit Kennwörtern (Klartext erlaubt), Auswahl.
    app.export_run(&json!({ "format": "csv", "scope": "all", "path": p("c") })).unwrap();
    let csv = std::fs::read_to_string(p("c.csv")).unwrap();
    assert!(csv.contains("host1.example.test") && !csv.contains("geheim"));
    app.export_run(&json!({ "format": "csv", "scope": "selection", "ids": [ids[0]], "passwords": true, "path": p("d") })).unwrap();
    let csv = std::fs::read_to_string(p("d.csv")).unwrap();
    assert!(csv.contains("geheim-äöü-1") && csv.contains("rechner") && !csv.contains("switch"), "{csv}");

    // Fehlerfälle: vorhandene Datei, leere Auswahl, gesperrt.
    assert!(app.export_run(&json!({ "format": "json", "scope": "all", "path": p("a") })).unwrap_err().contains("gibt es schon"));
    assert!(app.export_run(&json!({ "format": "csv", "scope": "selection", "ids": [], "path": p("e") })).unwrap_err().contains("nichts gewählt"));
    app.vault = None;
    assert!(app.export_run(&json!({ "format": "csv", "scope": "all", "path": p("f") })).unwrap_err().contains("entsperrt"));
}
