//! Hilfen für Tests anderer Kisten gegen den ganzen Kern (die Erweiterungen in crates/plugin-*/tests/kern.rs):
//! Programm mit leerer Schlüsseldatenbank, Bearbeitungsbefehle als JSON. Die eingebauten Erweiterungen gelten dabei
//! als freigeschaltet (Vorführungsschalter der Lizenz), unabhängig von einer Lizenz auf dem Rechner.
#![doc(hidden)]

use rj_kdbx::{KdfChoice, MasterKey, Vault};
use serde_json::Value;

use crate::app::App;

/// Eingebaute Erweiterungen in Tests freischalten (vor `App::new`), unabhängig von einer Lizenz auf dem Rechner.
pub fn enable_plugins() {
    crate::license::set_demo(true);
}

/// Erweiterung einbauen (Tests einer Erweiterung gegen den ganzen Kern; vor `App::new`, einmal je Erweiterung).
/// Lädt dabei nie Bibliotheken aus dem Datenordner des Rechners (die eingebauten stehen für sie).
pub fn register(plugins: Vec<&'static dyn rj_plugin_api::Plugin>, manifest: rj_plugin_api::Manifest) {
    std::env::set_var("REMOTEJOE_NO_PLUGINS", "1");
    crate::modules::register_builtin(plugins, manifest);
}

/// Programm mit einer neuen, entsperrten Schlüsseldatenbank in `dir` (ohne Konfigurationsdatei).
pub fn app(dir: &std::path::Path) -> App {
    enable_plugins();
    let path = dir.join("t.kdbx");
    let mut v = Vault::create(&path, MasterKey::password("Prüf"), KdfChoice::Weak, rj_core::config::IndexStorage::KdbxCustomData);
    v.save(None).unwrap();
    let mut app = App::new(false);
    app.config_path = None;
    app.index = v.index().index.clone();
    app.vault = Some(v);
    app
}

/// Programm mit neuer Schlüsseldatenbank in `dir`, dazu Pfad und Hauptschlüssel (zum Wiederöffnen).
pub fn app_with_vault(dir: &std::path::Path) -> (App, std::path::PathBuf, MasterKey) {
    let path = dir.join("t.kdbx");
    let key = MasterKey::password("Prüf ✓");
    let mut v = Vault::create(&path, key.clone(), KdfChoice::Weak, rj_core::config::IndexStorage::KdbxCustomData);
    v.save(None).unwrap();
    let mut app = App::new(false);
    app.index = v.index().index.clone();
    app.vault = Some(v);
    (app, path, key)
}

/// Bearbeitungsbefehl (wie aus der Oberfläche) mit JSON-Antwort.
pub fn cmd(app: &mut App, v: Value) -> Value {
    serde_json::from_str(&app.edit(&v.to_string())).unwrap()
}
