//! Oberflächentest im Offscreen-Betrieb: Vorführdaten mit 500 Zielen,
//! Bildschirmfotos der Hauptansichten, Startzeit.

use std::process::Command;

#[test]
fn offscreen_screenshots_and_start_time() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_remotejoe"))
        .args(["--screenshots", dir.path().to_str().unwrap()])
        .env("QT_QPA_PLATFORM", "offscreen")
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{log}");
    for f in ["hauptfenster-hell.png", "hauptfenster-dunkel.png", "suche.png", "dialog-zugangsdaten.png", "patchday.png", "abgerissenes-fenster.png"] {
        let len = std::fs::metadata(dir.path().join(f)).map(|m| m.len()).unwrap_or(0);
        assert!(len > 5000, "{f} fehlt oder ist leer ({len} Bytes)\n{log}");
    }
    let m = std::fs::read_to_string(dir.path().join("messwerte.txt")).unwrap();
    eprintln!("{m}");
    let get = |k: &str| m.lines().find_map(|l| l.strip_prefix(k)).and_then(|v| v.trim().parse::<i64>().ok()).unwrap();
    assert_eq!(get("baum_eintraege="), 525, "500 Ziele, 20 Gruppen, 5 Standorte");
    let ms = get("erstes_zeichnen_ms=");
    assert!(ms > 0 && ms < 3000, "erstes Zeichnen nach {ms} ms");
}
