//! Farbschemata hell und dunkel mit geprüftem Kontrast (WCAG 2.1 AA).

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Palette {
    pub window: &'static str,
    pub base: &'static str,
    pub alternate: &'static str,
    pub text: &'static str,
    pub muted: &'static str,
    pub highlight: &'static str,
    pub highlight_text: &'static str,
    pub button: &'static str,
    pub link: &'static str,
    pub warning: &'static str,
    pub error: &'static str,
    pub ok: &'static str,
}

pub const LIGHT: Palette = Palette {
    window: "#f3f3f3",
    base: "#ffffff",
    alternate: "#f0f4f8",
    text: "#1b1b1b",
    muted: "#555555",
    highlight: "#0b5cad",
    highlight_text: "#ffffff",
    button: "#e6e6e6",
    link: "#0b5cad",
    warning: "#8a5300",
    error: "#b00020",
    ok: "#1b6e20",
};

pub const DARK: Palette = Palette {
    window: "#1f1f1f",
    base: "#141414",
    alternate: "#1b2229",
    text: "#ececec",
    muted: "#b0b0b0",
    highlight: "#3d8fe0",
    highlight_text: "#000000",
    button: "#2d2d2d",
    link: "#7cb8f2",
    warning: "#f0b04a",
    error: "#ff8a8a",
    ok: "#7ed387",
};

/// Farben einer Palette mit eigenem Speicher (für Paletten aus Dateien).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaletteData {
    pub window: String,
    pub base: String,
    pub alternate: String,
    pub text: String,
    pub muted: String,
    pub highlight: String,
    pub highlight_text: String,
    pub button: String,
    pub link: String,
    pub warning: String,
    pub error: String,
    pub ok: String,
}

impl From<Palette> for PaletteData {
    fn from(p: Palette) -> Self {
        let s = |x: &str| x.to_string();
        PaletteData {
            window: s(p.window),
            base: s(p.base),
            alternate: s(p.alternate),
            text: s(p.text),
            muted: s(p.muted),
            highlight: s(p.highlight),
            highlight_text: s(p.highlight_text),
            button: s(p.button),
            link: s(p.link),
            warning: s(p.warning),
            error: s(p.error),
            ok: s(p.ok),
        }
    }
}

/// Eigene Palette aus `<Konfigurationsordner>/paletten/<Name>.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomPalette {
    /// Dateiname ohne `.json`; Auswahlkennung ist `datei:<id>`.
    pub id: String,
    pub name: String,
    pub dark: bool,
    pub colors: PaletteData,
}

fn valid_hex(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Liest die Paletten aus einem Ordner. Fehlende Farben kommen aus dem hellen bzw. dunklen
/// Standardschema (`"dark": true`); ungültige Dateien werden übersprungen und als Hinweis
/// gemeldet, ebenso zu geringer Kontrast (kein Zwang).
pub fn load_custom(dir: &std::path::Path) -> (Vec<CustomPalette>, Vec<String>) {
    let mut out = Vec::new();
    let mut notes = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return (out, notes) };
    let mut files: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    for path in files.into_iter().take(50) {
        let id = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) if t.len() <= 64 * 1024 => t,
            _ => {
                notes.push(format!("{id}.json: nicht lesbar oder zu groß"));
                continue;
            }
        };
        let v: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                notes.push(format!("{id}.json: kein gültiges JSON ({e})"));
                continue;
            }
        };
        let dark = v.get("dark").and_then(|x| x.as_bool()).unwrap_or(false);
        let mut c: PaletteData = if dark { DARK } else { LIGHT }.into();
        let mut ok = true;
        for (key, slot) in [
            ("window", &mut c.window),
            ("base", &mut c.base),
            ("alternate", &mut c.alternate),
            ("text", &mut c.text),
            ("muted", &mut c.muted),
            ("highlight", &mut c.highlight),
            ("highlight_text", &mut c.highlight_text),
            ("button", &mut c.button),
            ("link", &mut c.link),
            ("warning", &mut c.warning),
            ("error", &mut c.error),
            ("ok", &mut c.ok),
        ] {
            if let Some(val) = v.get(key) {
                match val.as_str().filter(|h| valid_hex(h)) {
                    Some(h) => *slot = h.to_lowercase(),
                    None => {
                        notes.push(format!("{id}.json: „{key}“ ist keine Farbe der Form #rrggbb"));
                        ok = false;
                    }
                }
            }
        }
        if !ok {
            continue;
        }
        for (fg, bg, what) in [(&c.text, &c.base, "Text/Grund"), (&c.text, &c.window, "Text/Fenster"), (&c.muted, &c.base, "Nebentext"), (&c.highlight_text, &c.highlight, "Auswahl")] {
            let ratio = contrast(fg, bg);
            if ratio < 4.5 {
                notes.push(format!("{id}.json: Hinweis, geringer Kontrast bei {what} ({ratio:.1} : 1, empfohlen mindestens 4,5 : 1)"));
            }
        }
        let name = v.get("name").and_then(|x| x.as_str()).map(|n| rj_core::charset::mark_visible(n, false).into_owned()).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| id.clone());
        out.push(CustomPalette { id, name, dark, colors: c });
    }
    (out, notes)
}

/// Ordner für eigene Paletten.
pub fn custom_dir() -> Option<std::path::PathBuf> {
    rj_core::config::config_dir().map(|d| d.join("paletten"))
}

fn channel(hex: &str, i: usize) -> f64 {
    let v = u8::from_str_radix(&hex[1 + i * 2..3 + i * 2], 16).unwrap_or(0) as f64 / 255.0;
    if v <= 0.03928 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Relative Leuchtdichte nach WCAG.
pub fn luminance(hex: &str) -> f64 {
    0.2126 * channel(hex, 0) + 0.7152 * channel(hex, 1) + 0.0722 * channel(hex, 2)
}

/// Kontrastverhältnis (1 … 21).
pub fn contrast(a: &str, b: &str) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Dunkles Schema nach Systemvorgabe? Linux: GNOME-Einstellung bzw.
/// `GTK_THEME`; Windows: Registrierung (`AppsUseLightTheme`).
pub fn system_prefers_dark() -> bool {
    if let Ok(v) = std::env::var("REMOTEJOE_THEME") {
        return v == "dark";
    }
    #[cfg(windows)]
    {
        let out = std::process::Command::new("reg")
            .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "/v", "AppsUseLightTheme"])
            .output();
        if let Ok(o) = out {
            return String::from_utf8_lossy(&o.stdout).contains("0x0");
        }
        false
    }
    #[cfg(not(windows))]
    {
        if std::env::var("GTK_THEME").is_ok_and(|t| t.to_lowercase().contains("dark")) {
            return true;
        }
        std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "color-scheme"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("dark"))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_meets_wcag_aa() {
        for (name, p) in [("hell", LIGHT), ("dunkel", DARK)] {
            for (fg, bg, min, what) in [
                (p.text, p.base, 4.5, "Text/Grund"),
                (p.text, p.window, 4.5, "Text/Fenster"),
                (p.text, p.alternate, 4.5, "Text/Wechselzeile"),
                (p.muted, p.base, 4.5, "Nebentext"),
                (p.highlight_text, p.highlight, 4.5, "Auswahl"),
                (p.link, p.base, 4.5, "Verweis"),
                (p.warning, p.base, 4.5, "Warnung"),
                (p.error, p.base, 4.5, "Fehler"),
                (p.ok, p.base, 4.5, "OK"),
                (p.text, p.button, 4.5, "Schaltfläche"),
            ] {
                let c = contrast(fg, bg);
                assert!(c >= min, "{name} {what}: {c:.2} < {min}");
            }
        }
    }

    #[test]
    fn custom_palettes_are_read_checked_and_bad_ones_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("gut.json"), r##"{"name":"Mein Schema","dark":true,"base":"#101010","highlight":"#ff8800"}"##).unwrap();
        std::fs::write(dir.path().join("kaputt.json"), "{ nicht json").unwrap();
        std::fs::write(dir.path().join("farbe.json"), r#"{"text":"rot"}"#).unwrap();
        std::fs::write(dir.path().join("flau.json"), r##"{"text":"#cccccc","base":"#ffffff"}"##).unwrap();
        std::fs::write(dir.path().join("anderes.txt"), "x").unwrap();
        let (list, notes) = load_custom(dir.path());
        let ids: Vec<_> = list.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["flau", "gut"], "{notes:?}");
        let gut = list.iter().find(|p| p.id == "gut").unwrap();
        assert_eq!((gut.name.as_str(), gut.dark, gut.colors.base.as_str(), gut.colors.text.as_str()), ("Mein Schema", true, "#101010", DARK.text));
        assert!(notes.iter().any(|n| n.starts_with("kaputt.json")) && notes.iter().any(|n| n.starts_with("farbe.json")), "{notes:?}");
        assert!(notes.iter().any(|n| n.starts_with("flau.json") && n.contains("Kontrast")), "{notes:?}");
        assert!(load_custom(&dir.path().join("fehlt")).0.is_empty());
    }

    #[test]
    fn contrast_formula() {
        assert!((contrast("#000000", "#ffffff") - 21.0).abs() < 0.01);
        assert!((contrast("#777777", "#777777") - 1.0).abs() < 0.01);
    }
}
