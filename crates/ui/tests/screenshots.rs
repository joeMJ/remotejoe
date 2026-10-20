//! Oberflächentest im Offscreen-Betrieb: Vorführdaten mit 500 Zielen,
//! Bildschirmfotos der Hauptansichten, Startzeit.

use std::process::Command;

#[test]
fn offscreen_screenshots_and_start_time() {
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_remotejoe"));
    cmd.args(["--screenshots", dir.path().to_str().unwrap()]).env("QT_QPA_PLATFORM", "offscreen");
    // Offscreen findet unter Windows keine Schriften von selbst (nur Kästchen in den Bildern).
    #[cfg(windows)]
    if std::env::var_os("QT_QPA_FONTDIR").is_none() {
        if let Some(w) = std::env::var_os("WINDIR") {
            cmd.env("QT_QPA_FONTDIR", std::path::Path::new(&w).join("Fonts"));
        }
    }
    let out = cmd.output().unwrap();
    let log = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{log}");
    for f in ["hauptfenster-hell.png", "hauptfenster-dunkel.png", "hauptfenster-kompakt-ohne-baum.png", "suche.png", "hypervisor.png", "verbinden-zugangsdaten.png", "hostschluessel-unbekannt.png", "hostschluessel-geaendert.png", "import-vorschau.png", "patchday.png", "abgerissenes-fenster.png", "eigenschaften.png", "zugangsdaten-auswahl.png", "hinzufuegen-menue.png", "entsperren.png", "mehrere-ziele.png"] {
        let len = std::fs::metadata(dir.path().join(f)).map(|m| m.len()).unwrap_or(0);
        assert!(len > 5000, "{f} fehlt oder ist leer ({len} Bytes)\n{log}");
    }
    let m = std::fs::read_to_string(dir.path().join("messwerte.txt")).unwrap();
    eprintln!("{m}");
    let get = |k: &str| m.lines().find_map(|l| l.strip_prefix(k)).and_then(|v| v.trim().parse::<i64>().ok()).unwrap();
    assert_eq!(get("baum_eintraege="), 525, "500 Ziele, 20 Gruppen, 5 Standorte");
    let bulk = m.lines().find_map(|l| l.strip_prefix("mehrere_ziele=")).unwrap_or_default();
    assert!(bulk.starts_with("4 Ziele werden angelegt, 3 Zeilen mit Fehlern"), "{bulk}");
    let ms = get("erstes_zeichnen_ms=");
    assert!(ms > 0 && ms < 3000, "erstes Zeichnen nach {ms} ms");
}
