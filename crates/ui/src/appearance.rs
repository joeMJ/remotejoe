//! Darstellung: Farbschema (System mit je einer Palette für hell und dunkel, feste Palette:
//! Standard, mitgeliefert oder eigene Datei) und Fokusrahmen.

use serde_json::{json, Value};

use crate::app::App;
use crate::theme::{self, PaletteData, DARK, LIGHT};

/// Ist das eine gültige Kennung einer Palette (ohne `system`)?
fn known_palette(s: &str) -> bool {
    matches!(s, "hell" | "dunkel")
        || s.strip_prefix("eingebaut:").is_some_and(|id| theme::BUILTIN.iter().any(|(b, ..)| *b == id))
        || s.strip_prefix("datei:").is_some_and(|id| !id.is_empty() && !id.contains(['/', '\\', '.']))
}

impl App {
    /// Auswahlmöglichkeiten und aktuelle Werte für den Tab „Darstellung“.
    pub fn appearance_json(&self) -> Value {
        let (custom, notes) = theme::custom_dir().map(|d| theme::load_custom(&d)).unwrap_or_default();
        let palettes: Vec<Value> = theme::choices(&custom)
            .into_iter()
            .map(|c| json!({ "id": c.id, "name": c.name, "dark": c.dark, "own": c.own, "colors": c.colors }))
            .collect();
        let a = &self.config.appearance;
        json!({
            "scheme": a.scheme,
            "light_theme": a.light_theme,
            "dark_theme": a.dark_theme,
            "system_dark": theme::system_prefers_dark(),
            "focus_frame": a.focus_frame,
            "palettes": palettes,
            "dir": theme::custom_dir().map(|d| d.display().to_string()),
            "notes": notes,
        })
    }

    /// Schema, Zuordnung hell/dunkel und Fokusrahmen ändern und speichern; mit `save_palette`
    /// (`name`, `dark`, `colors`) wird zusätzlich eine eigene Palette gespeichert.
    pub fn set_appearance(&mut self, c: &Value) -> Result<Value, String> {
        if let Some(p) = c.get("save_palette") {
            let dir = theme::custom_dir().ok_or("kein Ordner für eigene Paletten")?;
            let name = p.get("name").and_then(Value::as_str).unwrap_or_default();
            let dark = p.get("dark").and_then(Value::as_bool).unwrap_or(false);
            let id = theme::save_custom(&dir, name, dark, p.get("colors").unwrap_or(&Value::Null))?;
            let mut v = self.appearance_json();
            v["saved"] = json!(id);
            return Ok(v);
        }
        if let Some(s) = c.get("scheme").and_then(Value::as_str) {
            if s != "system" && !known_palette(s) {
                return Err(format!("Unbekanntes Farbschema „{s}“"));
            }
            self.config.appearance.scheme = s.to_string();
        }
        for (key, slot) in [("light_theme", 0), ("dark_theme", 1)] {
            if let Some(s) = c.get(key).and_then(Value::as_str) {
                if s != "system" && !known_palette(s) {
                    return Err(format!("Unbekanntes Farbschema „{s}“"));
                }
                let a = &mut self.config.appearance;
                *(if slot == 0 { &mut a.light_theme } else { &mut a.dark_theme }) = s.to_string();
            }
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

/// Farben einer Palette nach Kennung; `None`, wenn es sie nicht (mehr) gibt.
fn resolve(id: &str) -> Option<(bool, PaletteData)> {
    match id {
        "hell" => Some((false, LIGHT.into())),
        "dunkel" => Some((true, DARK.into())),
        s if s.starts_with("eingebaut:") => theme::BUILTIN.iter().find(|(b, ..)| *b == &s["eingebaut:".len()..]).map(|(_, _, d, p)| (*d, (*p).into())),
        s if s.starts_with("datei:") => {
            let id = &s["datei:".len()..];
            theme::custom_dir().map(|d| theme::load_custom(&d).0).unwrap_or_default().into_iter().find(|p| p.id == id).map(|p| (p.dark, p.colors))
        }
        _ => None,
    }
}

/// Palette für das Zeichnen: `mode` ist `system` (Farben aus der Qt-Stilpalette, die übrigen aus dem
/// passenden Standardschema) oder `fixed` (alle Farben aus der Palette).
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
    let Some(a) = app.map(|a| &a.config.appearance) else { return system(dark_sys) };
    // „System“: je nach hellem oder dunklem Modus die zugeordnete Palette (oder die Desktopfarben).
    let id = if a.scheme == "system" { if dark_sys { a.dark_theme.as_str() } else { a.light_theme.as_str() } } else { a.scheme.as_str() };
    match resolve(id) {
        Some((dark, colors)) => fixed(dark, colors),
        None => system(dark_sys),
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
        assert!(app.set_appearance(&json!({ "scheme": "eingebaut:gibtsnicht" })).is_err());
        assert!(app.set_appearance(&json!({ "dark_theme": "datei:a/b" })).is_err());
        assert!(app.set_appearance(&json!({ "scheme": "datei:mein-schema" })).is_ok());
        assert_eq!(palette_json(Some(&app))["mode"], "system", "fehlende Datei: Rückfall auf System");
        app.set_appearance(&json!({ "scheme": "hell" })).unwrap();
        let p = palette_json(Some(&app));
        assert_eq!((p["mode"].as_str(), p["dark"].as_bool(), p["window"].as_str()), (Some("fixed"), Some(false), Some(LIGHT.window)));
        app.set_appearance(&json!({ "scheme": "eingebaut:catppuccin-mocha" })).unwrap();
        let p = palette_json(Some(&app));
        assert_eq!((p["mode"].as_str(), p["dark"].as_bool(), p["base"].as_str()), (Some("fixed"), Some(true), Some("#1e1e2e")));
    }

    #[test]
    fn system_mode_uses_the_palette_assigned_to_light_or_dark() {
        let mut app = App::new(false);
        app.config_path = None;
        app.set_appearance(&json!({ "scheme": "system", "light_theme": "eingebaut:catppuccin-latte", "dark_theme": "eingebaut:catppuccin-mocha" })).unwrap();
        let r = app.appearance_json();
        assert_eq!((r["light_theme"].as_str(), r["dark_theme"].as_str()), (Some("eingebaut:catppuccin-latte"), Some("eingebaut:catppuccin-mocha")));
        assert!(r["palettes"].as_array().unwrap().iter().any(|p| p["id"] == "eingebaut:dracula" && p["dark"] == true));
        // Welche greift, hängt vom System ab; beide Fälle ergeben eine feste Palette.
        let p = palette_json(Some(&app));
        assert_eq!(p["mode"], "fixed");
        assert_eq!(p["base"].as_str(), Some(if theme::system_prefers_dark() { "#1e1e2e" } else { "#eff1f5" }));
        app.set_appearance(&json!({ "light_theme": "system", "dark_theme": "system" })).unwrap();
        assert_eq!(palette_json(Some(&app))["mode"], "system");
    }
}
