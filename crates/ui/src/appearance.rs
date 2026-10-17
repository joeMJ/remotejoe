//! Darstellung: Farbschema (System, Hell, Dunkel, eigene Paletten aus Dateien) und Fokusrahmen.

use serde_json::{json, Value};

use crate::app::App;
use crate::theme::{self, PaletteData, DARK, LIGHT};

impl App {
    /// Auswahlmöglichkeiten und aktuelle Werte für den Tab „Darstellung“.
    pub fn appearance_json(&self) -> Value {
        let (custom, notes) = theme::custom_dir().map(|d| theme::load_custom(&d)).unwrap_or_default();
        let mut schemes = vec![
            json!({ "id": "system", "label": "System (automatisch hell oder dunkel)" }),
            json!({ "id": "hell", "label": "Hell" }),
            json!({ "id": "dunkel", "label": "Dunkel" }),
        ];
        schemes.extend(custom.iter().map(|p| json!({ "id": format!("datei:{}", p.id), "label": format!("{} (eigene Palette)", p.name) })));
        json!({
            "scheme": self.config.appearance.scheme,
            "focus_frame": self.config.appearance.focus_frame,
            "schemes": schemes,
            "dir": theme::custom_dir().map(|d| d.display().to_string()),
            "notes": notes,
        })
    }

    /// Schema und Fokusrahmen ändern und speichern.
    pub fn set_appearance(&mut self, c: &Value) -> Result<Value, String> {
        if let Some(s) = c.get("scheme").and_then(Value::as_str) {
            let known = matches!(s, "system" | "hell" | "dunkel") || s.strip_prefix("datei:").is_some_and(|id| !id.is_empty() && !id.contains(['/', '\\']));
            if !known {
                return Err(format!("Unbekanntes Farbschema „{s}“"));
            }
            self.config.appearance.scheme = s.to_string();
        }
        if let Some(b) = c.get("focus_frame").and_then(Value::as_bool) {
            self.config.appearance.focus_frame = b;
        }
        if !self.demo {
            if let Some(cp) = self.config_path.clone() {
                self.config.save(&cp).map_err(|e| e.to_string())?;
            }
        }
        Ok(self.appearance_json())
    }
}

/// Palette für das Zeichnen: `mode` ist `system` (Farben aus der Qt-Stilpalette, die übrigen aus dem
/// passenden Standardschema) oder `fixed` (alle Farben aus dem Schema).
pub fn palette_json(app: Option<&App>) -> Value {
    let dark_sys = theme::system_prefers_dark();
    let fixed = |dark: bool, colors: PaletteData| {
        let mut v = serde_json::to_value(colors).unwrap_or_default();
        v["mode"] = json!("fixed");
        v["dark"] = json!(dark);
        v
    };
    // Vorgabe für Bildschirmfotos: Umgebungsvariable erzwingt ein Standardschema.
    if std::env::var_os("REMOTEJOE_THEME").is_some() {
        return fixed(dark_sys, if dark_sys { DARK } else { LIGHT }.into());
    }
    let scheme = app.map(|a| a.config.appearance.scheme.clone()).unwrap_or_else(|| "system".into());
    match scheme.as_str() {
        "hell" => fixed(false, LIGHT.into()),
        "dunkel" => fixed(true, DARK.into()),
        s if s.starts_with("datei:") => {
            let id = &s["datei:".len()..];
            let found = theme::custom_dir().map(|d| theme::load_custom(&d).0).unwrap_or_default().into_iter().find(|p| p.id == id);
            match found {
                Some(p) => fixed(p.dark, p.colors),
                None => system(dark_sys),
            }
        }
        _ => system(dark_sys),
    }
}

fn system(dark: bool) -> Value {
    let mut v = serde_json::to_value(PaletteData::from(if dark { DARK } else { LIGHT })).unwrap_or_default();
    v["mode"] = json!("system");
    v["dark"] = json!(dark);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemes_are_validated_and_saved_in_the_config() {
        let mut app = App::new(false);
        app.config_path = None;
        assert_eq!(app.appearance_json()["scheme"], "system");
        assert_eq!(app.appearance_json()["focus_frame"], true);
        let r = app.set_appearance(&json!({ "scheme": "dunkel", "focus_frame": false })).unwrap();
        assert_eq!((r["scheme"].as_str(), r["focus_frame"].as_bool()), (Some("dunkel"), Some(false)));
        assert!(app.set_appearance(&json!({ "scheme": "../etc/passwd" })).is_err());
        assert!(app.set_appearance(&json!({ "scheme": "datei:../x" })).is_err());
        assert!(app.set_appearance(&json!({ "scheme": "datei:mein-schema" })).is_ok());
        assert_eq!(palette_json(Some(&app))["mode"], "system", "fehlende Datei: Rückfall auf System");
        app.set_appearance(&json!({ "scheme": "hell" })).unwrap();
        let p = palette_json(Some(&app));
        assert_eq!((p["mode"].as_str(), p["dark"].as_bool(), p["window"].as_str()), (Some("fixed"), Some(false), Some(LIGHT.window)));
    }
}
