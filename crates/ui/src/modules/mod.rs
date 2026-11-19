//! Module (Entwicklungsvorhaben S3, Etappe E4): Teile des Programms, die sich an- und später per Bauschalter bzw.
//! Lizenz abschalten lassen. Jedes Modul meldet seine Operationen an; die allgemeine Verteilung in `editor.rs` fragt
//! sie ab, wenn sie eine Operation nicht selbst kennt.

#[cfg(feature = "aruba")]
pub mod aruba;
#[cfg(feature = "standorte")]
pub mod standorte;

use serde_json::Value;

use crate::app::App;

pub trait Module: Sync {
    fn id(&self) -> &'static str;
    /// Operation ausführen; `None`, wenn das Modul sie nicht kennt.
    fn handle(&self, app: &mut App, op: &str, c: &Value) -> Option<Result<Value, String>>;
}

/// Angemeldete Module dieser Fassung (je nach Bauschaltern).
pub fn all() -> Vec<&'static dyn Module> {
    #[allow(unused_mut)]
    let mut v: Vec<&'static dyn Module> = Vec::new();
    #[cfg(feature = "standorte")]
    v.push(&standorte::Ops);
    #[cfg(feature = "aruba")]
    v.push(&aruba::Ops);
    v
}

/// Name der Fassung für Fenster, „Über“ und Pakete.
pub const EDITION: &str = if cfg!(feature = "advanced") { "remotejoe advanced" } else { "remotejoe basic" };

/// Einstellungen der Module in die Antwort „settings“.
#[allow(unused_variables)]
pub fn settings_json(app: &App, m: &mut serde_json::Map<String, Value>) {
    #[cfg(feature = "standorte")]
    standorte::settings_json(app, m);
    #[cfg(feature = "aruba")]
    aruba::settings_json(app, m);
}

/// Einstellungen der Module aus „set_settings“ übernehmen.
#[allow(unused_variables)]
pub fn set_settings(app: &mut App, c: &Value) {
    #[cfg(feature = "standorte")]
    standorte::set_settings(app, c);
    #[cfg(feature = "aruba")]
    aruba::set_settings(app, c);
}

/// Operation an das zuständige Modul geben.
pub fn dispatch(app: &mut App, op: &str, c: &Value) -> Option<Result<Value, String>> {
    all().into_iter().find_map(|m| m.handle(app, op, c))
}

#[cfg(all(test, feature = "advanced"))]
mod tests {
    use super::*;

    #[cfg(feature = "advanced")]
    #[test]
    fn modules_know_their_ops_and_unknown_ops_fall_through() {
        let mut app = App::new(true);
        app.config_path = None;
        assert!(dispatch(&mut app, "gibt_es_nicht", &Value::Null).is_none());
        assert!(dispatch(&mut app, "sites_overview", &serde_json::json!({})).is_some(), "Standorte");
        assert!(dispatch(&mut app, "aptools_status", &Value::Null).is_some(), "Aruba");
        let ids: Vec<&str> = all().iter().map(|m| m.id()).collect();
        assert_eq!(ids, ["standorte", "aruba"]);
    }
}

#[cfg(all(test, not(any(feature = "standorte", feature = "aruba"))))]
mod tests_core {
    use super::*;

    #[test]
    fn without_modules_no_module_op_is_known() {
        let mut app = App::new(true);
        app.config_path = None;
        assert!(all().is_empty());
        assert!(dispatch(&mut app, "sites_overview", &serde_json::json!({})).is_none());
        assert!(dispatch(&mut app, "aruba_cluster", &serde_json::json!({})).is_none());
        assert_eq!(EDITION, "remotejoe basic");
    }
}
