//! Import mit erfundenen Beispieldateien.

use std::io::Write;
use std::path::PathBuf;

use rj_core::inherit::{resolve_credential, CredentialResolution};
use rj_core::{Node, Protocol};
use rj_importer::plan::{apply, plan};
use rj_importer::*;
use rj_kdbx::{KdfChoice, MasterKey, Vault};

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)
}

fn names(f: &rj_core::Folder) -> Vec<String> {
    f.children.iter().map(|c| c.name().to_string()).collect()
}

fn folder<'a>(f: &'a rj_core::Folder, name: &str) -> &'a rj_core::Folder {
    f.children.iter().find_map(|c| match c {
        Node::Folder(x) if x.name == name => Some(x),
        _ => None,
    }).unwrap_or_else(|| panic!("Ordner {name} fehlt in {:?}", names(f)))
}

#[test]
fn rdm_xml_structure_report_and_credentials() {
    let doc = read_file(&data("beispiel.rdm")).unwrap();
    let p = plan(&doc, "Import RDM");
    let r = &p.report;
    eprintln!("{}", r.summary());
    assert_eq!(r.targets, 6, "TeamViewer wird übersprungen");
    assert_eq!(r.folders, 4, "Standort Nord, Server, Linux, Standort Süd");
    assert_eq!(r.skipped.len(), 1);
    assert!(r.skipped[0].1.contains("TeamViewer"));
    assert!(r.unknown_fields.iter().any(|(f, _)| f == "MeinFeld"));
    let w = r.warnings.join("\n");
    assert!(w.contains("web⟦U+200B⟧01"), "{w}");
    assert!(w.contains("Kassen⟦U+202E⟧txt.exe"), "{w}");
    assert!(w.contains("verschlüsselt"), "SafePassword gemeldet");
    assert!(w.contains("„Druckserver“ kommt") || !w.contains("Druckserver"), "doppelter Name nur je Ordner");
    // Reihenfolge und Ebenen.
    assert_eq!(names(&p.folder), ["Standort Nord", "Standort Süd"]);
    let server = folder(folder(&p.folder, "Standort Nord"), "Server");
    assert_eq!(names(server), ["Dateiserver Süd 🗄", "Druckserver", "Linux"]);
    // Zugangsdaten: admin (3×), root, dienst (Verweis), fwadmin (Kennwort verschlüsselt, ohne
    // Kennwort übernommen) → 4 Einträge; admin einmal am Ordner „Server“.
    assert_eq!(r.credentials_in_source, 6);
    assert_eq!(r.credentials_unique, 4);
    assert!(p.refs.iter().any(|(id, _)| *id == server.id), "gemeinsamer Verweis am Ordner");
    assert_eq!(r.references, 4, "Ordner Server + root, Dienstkonto, fwadmin je am Ziel");
}

#[test]
fn rdm_json_and_royal() {
    let doc = read_file(&data("beispiel-rdm.json")).unwrap();
    let p = plan(&doc, "Import JSON");
    assert_eq!(p.report.targets, 3);
    assert_eq!(p.report.credentials_unique, 1);
    assert_eq!(p.report.references, 1, "ein Verweis am Ordner „Labor“ genügt");
    assert!(p.report.unknown_fields.iter().any(|(f, _)| f == "Farbe"));
    assert!(p.report.warnings.join("\n").contains("Sprung⟦U+200D⟧server"));

    // Royal TS .rtsz (ZIP mit JSON) wird hier aus der Beispieldatei gebaut.
    let dir = tempfile::tempdir().unwrap();
    let rtsz = dir.path().join("beispiel.rtsz");
    {
        let f = std::fs::File::create(&rtsz).unwrap();
        let mut z = zip::ZipWriter::new(f);
        z.start_file("Document.json", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(&std::fs::read(data("beispiel-royal.json")).unwrap()).unwrap();
        z.finish().unwrap();
    }
    let doc = read_file(&rtsz).unwrap();
    let p = plan(&doc, "Import Royal");
    assert_eq!(p.report.targets, 3);
    assert_eq!(p.report.skipped.len(), 1, "Aufgabenobjekt übersprungen");
    let kunden = folder(&p.folder, "Kunden 🧾");
    assert_eq!(names(kunden), ["Größere Filiale", "Router"], "Position bestimmt die Reihenfolge");
    assert_eq!(names(folder(kunden, "Größere Filiale")), ["Terminalserver A", "Terminalserver B"]);
    let Node::Target(ts) = &folder(kunden, "Größere Filiale").children[1] else { panic!() };
    assert_eq!(ts.port, Some(3390));
    assert_eq!(p.report.credentials_unique, 2);

    let doc = read_file(&data("beispiel.rtsx")).unwrap();
    let p = plan(&doc, "Import rtsx");
    let a = folder(&p.folder, "Außenstelle");
    let Node::Target(v) = &a.children[0] else { panic!() };
    assert_eq!(v.name, "Anzeige & Kasse");
    assert_eq!(v.protocol, Protocol::Vnc);
}

#[test]
fn apply_roundtrip_and_no_passwords_in_index() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ziel.kdbx");
    let key = MasterKey::password("Haupt-Kennwort");
    let mut vault = Vault::create(&path, key.clone(), KdfChoice::Weak, rj_core::config::IndexStorage::KdbxCustomData);
    let doc = read_file(&data("beispiel.rdm")).unwrap();
    let p = plan(&doc, "Import RDM");
    let base = vault.index().index.clone();
    let idx = apply(&p, &mut vault, &base).unwrap();
    vault.set_index(idx).unwrap();
    vault.save(None).unwrap();

    let v = Vault::open(&path, key, rj_core::config::IndexStorage::KdbxCustomData).unwrap();
    let idx = &v.index().index;
    let json = String::from_utf8(rj_core::schema::to_json(idx).unwrap()).unwrap();
    for pw in ["Geheim-Nord-ä1", "Anderes-Kennwort-🚀", "Dienst-Kennwort-ß"] {
        assert!(!json.contains(pw), "Kennwort {pw} darf nicht im Index stehen");
    }
    // Jedes Ziel löst die richtigen Zugangsdaten auf.
    let mut checked = 0;
    idx.for_each_target(|t| {
        let expected = match t.host.as_str() {
            "fs01.example.test" | "ps01.example.test" | "192.0.2.21" => Some(("EXAMPLE\\admin", "Geheim-Nord-ä1")),
            "192.0.2.22" => Some(("root", "Anderes-Kennwort-🚀")),
            "kasse.example.test" => Some(("EXAMPLE\\dienst", "Dienst-Kennwort-ß")),
            _ => None,
        };
        if let Some((user, pw)) = expected {
            let CredentialResolution::Found(r) = resolve_credential(idx, t.id).unwrap() else { panic!("{}", t.name) };
            let c = v.resolve(&r.reference).unwrap();
            assert_eq!(c.data.username, user, "{}", t.name);
            assert_eq!(c.data.password.as_bytes(), pw.as_bytes(), "{}", t.name);
            assert_eq!(c.group_path, vec!["remotejoe".to_string()]);
            checked += 1;
        }
    });
    assert_eq!(checked, 5);
    assert_eq!(v.entries().len(), 4, "zusammengelegt: vier KDBX-Einträge aus sechs Angaben");
}

#[test]
fn invalid_utf8_is_reported_with_location() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("kaputt.rdm");
    std::fs::write(&p, b"<?xml version=\"1.0\"?>\n<ArrayOfConnection>\n<Connection><Name>a\xff</Name></Connection></ArrayOfConnection>").unwrap();
    let e = read_file(&p).unwrap_err().to_string();
    assert!(e.contains("Zeile 3"), "{e}");
}

#[test]
fn rdm_encrypted_passwords_reported_per_entry_and_in_total() {
    let doc = read_file(&data("rdm-verschluesselt.rdm")).unwrap();
    let p = plan(&doc, "Import verschlüsselt");
    let r = &p.report;
    assert_eq!(r.encrypted_passwords, 3, "Datensatz, RDP-Block und Terminal-Block; Klartext zählt nicht");
    let w = r.warnings.join("\n");
    assert!(w.contains("Konto verschlüsselt") && w.contains("verschlüsselt"), "{w}");
    assert!(w.contains("RDP mit eigenem Konto"), "{w}");
    assert!(w.contains("SSH mit eigenem Konto"), "{w}");
    assert!(!w.contains("SSH im Klartext"), "Klartext-Kennwort ist kein Fund: {w}");
    let summary = r.summary();
    assert!(summary.contains("3 Einträge mit verschlüsselt exportiertem Kennwort"), "{summary}");
}
