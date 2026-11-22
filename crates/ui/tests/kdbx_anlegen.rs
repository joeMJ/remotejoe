//! Neue KDBX im Programm anlegen: Datei entsteht, wird geöffnet, lässt sich nach dem
//! Sperren wieder entsperren; eine vorhandene Datei wird nie überschrieben.

use std::time::{Duration, Instant};

use rj_core::Secret;
use rj_ui::app::App;
use rj_ui::sessions::UiEvent;

fn warte(app: &mut App) -> UiEvent {
    let start = Instant::now();
    loop {
        for e in app.poll() {
            if matches!(e, UiEvent::Unlocked { .. } | UiEvent::UnlockFailed { .. }) {
                return e;
            }
        }
        assert!(start.elapsed() < Duration::from_secs(60), "keine Antwort");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn create_open_lock_unlock_and_never_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Ordner ä").join("neu.kdbx");
    let mut app = App::new(false);
    // Der Test darf die echte Konfiguration des Benutzers nicht verändern.
    app.config_path = None;

    // 1. Anlegen: Datei und Unterordner entstehen, die KDBX ist danach geöffnet.
    app.create_vault(path.display().to_string(), Secret::from_str_exact("Prüf-Kennwort ß 1234"), None, false, false);
    assert!(matches!(warte(&mut app), UiEvent::Unlocked { targets: 0 }));
    assert!(path.exists());

    // 2. Sperren und mit demselben Kennwort wieder entsperren.
    app.lock(false);
    app.unlock(path.display().to_string(), Secret::from_str_exact("Prüf-Kennwort ß 1234"), None, false);
    assert!(matches!(warte(&mut app), UiEvent::Unlocked { .. }));

    // 3. Falsches Kennwort schlägt fehl.
    app.lock(false);
    app.unlock(path.display().to_string(), Secret::from_str_exact("falsch"), None, false);
    assert!(matches!(warte(&mut app), UiEvent::UnlockFailed { .. }));

    // 4. Eine vorhandene Datei wird nicht überschrieben.
    let before = std::fs::read(&path).unwrap();
    app.create_vault(path.display().to_string(), Secret::from_str_exact("anderes Kennwort"), None, false, false);
    match warte(&mut app) {
        UiEvent::UnlockFailed { reason } => assert!(reason.contains("schon"), "{reason}"),
        e => panic!("unerwartet: {e:?}"),
    }
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn create_with_generated_key_file_needs_both() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mit-schluessel.kdbx");
    let key = dir.path().join("mit-schluessel.key");
    let mut app = App::new(false);
    app.config_path = None;

    app.create_vault(path.display().to_string(), Secret::from_str_exact("Kennwort 1"), Some(key.display().to_string()), true, false);
    assert!(matches!(warte(&mut app), UiEvent::Unlocked { .. }));
    assert!(key.exists());

    app.lock(false);
    // Ohne Schlüsseldatei geht es nicht, mit ihr schon.
    app.unlock(path.display().to_string(), Secret::from_str_exact("Kennwort 1"), None, false);
    assert!(matches!(warte(&mut app), UiEvent::UnlockFailed { .. }));
    app.unlock(path.display().to_string(), Secret::from_str_exact("Kennwort 1"), Some(key.display().to_string()), false);
    assert!(matches!(warte(&mut app), UiEvent::Unlocked { .. }));
}

#[test]
fn key_file_is_remembered_and_a_missing_one_is_reported_without_echo() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gemerkt.kdbx");
    let key = dir.path().join("gemerkt.key");
    let (p, k) = (path.display().to_string(), key.display().to_string());
    let mut app = App::new(false);
    app.config_path = None;

    // Nach dem Anlegen mit Schlüsseldatei ist sie zu dieser KDBX gemerkt (Vorbelegung der Maske).
    app.create_vault(p.clone(), Secret::from_str_exact("Kennwort 2"), Some(k.clone()), true, false);
    assert!(matches!(warte(&mut app), UiEvent::Unlocked { .. }));
    assert_eq!(app.key_file_for(&p).as_deref(), Some(k.as_str()));
    assert_eq!(app.key_file_for("/anderswo/andere.kdbx"), None);

    // Versehentlich das Kennwort ins Feld der Schlüsseldatei getippt: klare Meldung, der Text taucht nicht auf.
    app.lock(false);
    app.unlock(p.clone(), Secret::from_str_exact("Kennwort 2"), Some("Kennwort 2".into()), false);
    match warte(&mut app) {
        UiEvent::UnlockFailed { reason } => {
            assert!(reason.contains("Schlüsseldatei wurde nicht gefunden"), "{reason}");
            assert!(!reason.contains("Kennwort 2"), "{reason}");
        }
        e => panic!("unerwartet: {e:?}"),
    }
    // Der Fehlversuch ändert die gemerkte Schlüsseldatei nicht.
    assert_eq!(app.key_file_for(&p).as_deref(), Some(k.as_str()));
    app.unlock(p.clone(), Secret::from_str_exact("Kennwort 2"), Some(k.clone()), false);
    assert!(matches!(warte(&mut app), UiEvent::Unlocked { .. }));

    // Eine KDBX ohne Schlüsseldatei entfernt die gemerkte Angabe.
    let plain = dir.path().join("ohne.kdbx").display().to_string();
    app.lock(false);
    app.create_vault(plain.clone(), Secret::from_str_exact("Kennwort 3"), None, false, false);
    assert!(matches!(warte(&mut app), UiEvent::Unlocked { .. }));
    assert_eq!(app.key_file_for(&plain), None);
}

#[test]
fn create_rejects_missing_input() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(false);
    app.config_path = None;
    // Weder Kennwort noch Schlüsseldatei.
    app.create_vault(dir.path().join("leer.kdbx").display().to_string(), Secret::from_str_exact(""), None, false, false);
    assert!(matches!(warte(&mut app), UiEvent::UnlockFailed { .. }));
    assert!(!dir.path().join("leer.kdbx").exists());
    // Neue Schlüsseldatei ohne Pfad.
    app.create_vault(dir.path().join("x.kdbx").display().to_string(), Secret::from_str_exact("pw"), None, true, false);
    assert!(matches!(warte(&mut app), UiEvent::UnlockFailed { .. }));
}
