//! Der eine Ordner für alles: neue Installation unter `~/remotejoe`, alte Ordner bleiben, wo sie sind.
//! (Eigene Testdatei, weil sie Umgebungsvariablen des Prozesses setzt.)

use rj_core::config::{config_dir, data_dir, default_config_path, home_dir, Config};
use std::path::Path;

/// Persönlicher Ordner für den Test. Unter Windows gelten `USERPROFILE`, `APPDATA` und `LOCALAPPDATA`;
/// sie zeigen auf dieselben Unterordner wie unter Linux, damit die Prüfungen für beide gelten.
fn set_home(home: &Path) {
    std::env::set_var("HOME", home);
    #[cfg(windows)]
    {
        std::env::set_var("USERPROFILE", home);
        std::env::set_var("APPDATA", home.join(".config"));
        std::env::set_var("LOCALAPPDATA", home.join(".local/share"));
    }
}

#[test]
fn fresh_install_uses_one_folder_under_home_and_old_installations_stay() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    set_home(&home);
    for k in ["XDG_CONFIG_HOME", "XDG_DATA_HOME", "REMOTEJOE_HOME"] {
        std::env::remove_var(k);
    }
    // Neue Installation.
    let new = home.join("remotejoe");
    assert_eq!(home_dir(), Some(new.clone()));
    assert_eq!(config_dir(), Some(new.clone()));
    assert_eq!(data_dir(), Some(new.join("daten")));
    assert_eq!(default_config_path(), Some(new.join("config.toml")));
    let kh = rj_ssh::known_hosts::KnownHosts::default_location().unwrap();
    assert!(kh.path().starts_with(new.join("daten")), "{:?}", kh.path());
    // Speichern legt den Ordner an; danach gilt er endgültig.
    let mut c = Config::default();
    c.kdbx_path = Some(new.join("remotejoe.kdbx"));
    c.save(&new.join("config.toml")).unwrap();
    assert!(new.join("config.toml").is_file());
    assert_eq!(home_dir(), Some(new.clone()));

    // Alte Installation (nur alter Ordner mit config.toml, kein neuer): bleibt unverändert.
    let home2 = dir.path().join("home2");
    std::fs::create_dir_all(home2.join(".config/remotejoe")).unwrap();
    std::fs::write(home2.join(".config/remotejoe/config.toml"), "").unwrap();
    set_home(&home2);
    assert_eq!(home_dir(), None);
    assert_eq!(config_dir(), Some(home2.join(".config/remotejoe")));
    assert_eq!(data_dir(), Some(home2.join(".local/share/remotejoe")));
    assert!(!home2.join("remotejoe").exists(), "nichts wird neu angelegt oder verschoben");

    // REMOTEJOE_HOME wählt den Ordner ausdrücklich.
    std::env::set_var("REMOTEJOE_HOME", dir.path().join("frei"));
    assert_eq!(config_dir(), Some(dir.path().join("frei")));
    assert_eq!(data_dir(), Some(dir.path().join("frei/daten")));
}
