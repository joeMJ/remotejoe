//! Das verschlüsselte ZIP des Exports lässt sich mit 7-Zip öffnen (AES-256); ohne `7z` übersprungen.

use std::process::Command;

#[test]
fn encrypted_zip_opens_with_7zip() {
    let Ok(seven) = ["7z", "7za", "7zr"].into_iter().find(|b| Command::new(b).arg("--help").output().is_ok()).ok_or(()) else {
        eprintln!("ÜBERSPRUNGEN: 7z nicht vorhanden");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let zip = rj_importer::export::encrypted_zip("remotejoe-export.json", "{\"inhalt\": \"Prüfung ä\"}".as_bytes(), "Zip Kennwort ä 1").unwrap();
    let path = dir.path().join("e.zip");
    std::fs::write(&path, &zip).unwrap();
    let out = dir.path().join("aus");
    let ok = Command::new(seven).arg("x").arg("-pZip Kennwort ä 1").arg(format!("-o{}", out.display())).arg("-y").arg(&path).output().unwrap();
    assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stdout));
    assert_eq!(std::fs::read_to_string(out.join("remotejoe-export.json")).unwrap(), "{\"inhalt\": \"Prüfung ä\"}");
    let bad = Command::new(seven).arg("t").arg("-pfalsch").arg(&path).output().unwrap();
    assert!(!bad.status.success(), "falsches Kennwort muss scheitern");
}
