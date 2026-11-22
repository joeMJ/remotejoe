//! Erweiterungen aus Sicht des übrigen Kerns: Freischaltung, Lizenz, Einstellungen. Die Erweiterungen selbst lädt
//! `plugins.rs`; die allgemeine Verteilung in `editor.rs` fragt sie, wenn sie eine Operation nicht selbst kennt.

use serde_json::Value;

use crate::app::App;

/// Eingebaute Erweiterungen: im Programm keine (Erweiterungen kommen als Bibliotheken aus `plugins/` des
/// Datenordners). Tests einer Erweiterung gegen den ganzen Kern melden sie hier an (`testing::register`).
type Builtin = (Vec<&'static dyn rj_plugin_api::Plugin>, Vec<rj_plugin_api::Manifest>);
static BUILTIN: std::sync::RwLock<Option<Builtin>> = std::sync::RwLock::new(None);

pub(crate) fn register_builtin(plugins: Vec<&'static dyn rj_plugin_api::Plugin>, manifest: rj_plugin_api::Manifest) {
    let mut b = BUILTIN.write().unwrap_or_else(|e| e.into_inner());
    let (p, m) = b.get_or_insert_with(|| (Vec::new(), Vec::new()));
    if m.iter().any(|x| x.id == manifest.id) {
        return;
    }
    p.extend(plugins);
    m.push(manifest);
}

pub fn builtin() -> Vec<&'static dyn rj_plugin_api::Plugin> {
    BUILTIN.read().unwrap_or_else(|e| e.into_inner()).as_ref().map(|b| b.0.clone()).unwrap_or_default()
}

/// Manifeste der eingebauten Erweiterungen (eines je Bibliothek).
pub fn builtin_manifests() -> Vec<rj_plugin_api::Manifest> {
    BUILTIN.read().unwrap_or_else(|e| e.into_inner()).as_ref().map(|b| b.1.clone()).unwrap_or_default()
}

/// Ist mindestens eine Erweiterung aktiv? Eingebaut oder geladen, und laut Lizenz bzw. Vorführung freigeschaltet
/// (sonst Rückfall auf die Funktionen ohne Erweiterungen, gespeicherte Daten bleiben unberührt). Entscheidet
/// zugleich über „advanced“ im Fenstertitel.
pub fn aktiv() -> bool {
    crate::plugins::any_active()
}

/// Beim Anlegen der App: Vorführung merken.
#[allow(unused_variables)]
pub fn set_demo(demo: bool) {
    crate::license::set_demo(demo);
}

/// Angaben für Einstellungen → Lizenz.
pub fn lizenz_json() -> Value {
    crate::license::json()
}

/// Eingefügte Lizenz prüfen und ablegen (gilt ab dem nächsten Start); bei geöffneter Schlüsseldatenbank auch dort.
#[allow(unused_variables)]
pub fn lizenz_speichern(app: Option<&mut App>, text: &str) -> Result<Value, String> {
    let mut v = crate::license::save(text)?;
    if let Some(app) = app {
        v["kdbx"] = match crate::license::in_kdbx(app, text) {
            Ok(()) => Value::Bool(true),
            Err(e) => Value::String(e),
        };
        // Erweiterungen der Lizenz gleich herunterladen (gelten mit dem nächsten Start).
        v["erweiterungen"] = app.plugin_fetch_after_license(text);
    }
    Ok(v)
}

/// Lizenz entfernen (Einstellungen → Lizenz, nach Rückfrage in der Oberfläche); gilt ab dem nächsten Start.
pub fn lizenz_entfernen(app: &mut App) -> Result<Value, String> {
    crate::license::remove(app)
}

/// Nach dem Entsperren: Lizenz aus der Schlüsseldatenbank übernehmen (gilt nach dem Neustart).
#[allow(unused_variables)]
pub fn lizenz_nach_entsperren(app: &App) -> bool {
    crate::license::aus_kdbx(app)
}


/// Einstellungen der Module in die Antwort „settings“.
pub fn settings_json(app: &mut App, m: &mut serde_json::Map<String, Value>) {
    m.insert("module_aktiv".into(), Value::Bool(aktiv()));
    crate::plugins::settings_json(app, m);
}

/// Einstellungen der Module aus „set_settings“ übernehmen.
pub fn set_settings(app: &mut App, c: &Value) {
    crate::plugins::set_settings(app, c);
}

#[cfg(test)]
mod tests_core {
    use super::*;

    #[test]
    fn without_modules_no_module_op_is_known() {
        let mut app = App::new(true);
        app.config_path = None;
        assert!(crate::plugins::all().is_empty());
        assert!(crate::plugins::dispatch(&mut app, "gibt_es_nicht", &serde_json::json!({})).is_none());
        assert_eq!(crate::plugins::edition(), "remotejoe basic");
    }
}
