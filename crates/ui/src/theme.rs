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

/// Mitgelieferte Paletten (Kennung, Name, dunkel, Farben). Alle Werte stammen aus den offiziellen
/// Quellen: Catppuccin `catppuccin/palette` (palette.json), Nord `nordtheme/nord` (nord.css),
/// Solarized `altercation/solarized` (README), Gruvbox `morhetz/gruvbox` (colors/gruvbox.vim),
/// Dracula `dracula/dracula-theme` (README). Zugeordnet sind sie den Rollen von remotejoe; die
/// Akzentfarben heller Paletten erreichen nicht überall 4,5 : 1 (so sind die Originale).
pub const BUILTIN: &[(&str, &str, bool, Palette)] = &[
    ("catppuccin-latte", "Catppuccin Latte", false, Palette { window: "#e6e9ef", base: "#eff1f5", alternate: "#e6e9ef", text: "#4c4f69", muted: "#5c5f77", highlight: "#1e66f5", highlight_text: "#eff1f5", button: "#ccd0da", link: "#1e66f5", warning: "#fe640b", error: "#d20f39", ok: "#40a02b" }),
    ("catppuccin-frappe", "Catppuccin Frappé", true, Palette { window: "#292c3c", base: "#303446", alternate: "#292c3c", text: "#c6d0f5", muted: "#a5adce", highlight: "#8caaee", highlight_text: "#232634", button: "#414559", link: "#8caaee", warning: "#e5c890", error: "#e78284", ok: "#a6d189" }),
    ("catppuccin-macchiato", "Catppuccin Macchiato", true, Palette { window: "#1e2030", base: "#24273a", alternate: "#1e2030", text: "#cad3f5", muted: "#a5adcb", highlight: "#8aadf4", highlight_text: "#181926", button: "#363a4f", link: "#8aadf4", warning: "#eed49f", error: "#ed8796", ok: "#a6da95" }),
    ("catppuccin-mocha", "Catppuccin Mocha", true, Palette { window: "#181825", base: "#1e1e2e", alternate: "#181825", text: "#cdd6f4", muted: "#a6adc8", highlight: "#89b4fa", highlight_text: "#11111b", button: "#313244", link: "#89b4fa", warning: "#f9e2af", error: "#f38ba8", ok: "#a6e3a1" }),
    ("nord-hell", "Nord hell", false, Palette { window: "#e5e9f0", base: "#eceff4", alternate: "#e5e9f0", text: "#2e3440", muted: "#4c566a", highlight: "#5e81ac", highlight_text: "#eceff4", button: "#d8dee9", link: "#5e81ac", warning: "#d08770", error: "#bf616a", ok: "#a3be8c" }),
    ("nord", "Nord", true, Palette { window: "#3b4252", base: "#2e3440", alternate: "#3b4252", text: "#eceff4", muted: "#d8dee9", highlight: "#88c0d0", highlight_text: "#2e3440", button: "#434c5e", link: "#88c0d0", warning: "#ebcb8b", error: "#bf616a", ok: "#a3be8c" }),
    ("solarized-hell", "Solarized hell", false, Palette { window: "#eee8d5", base: "#fdf6e3", alternate: "#eee8d5", text: "#073642", muted: "#586e75", highlight: "#268bd2", highlight_text: "#fdf6e3", button: "#eee8d5", link: "#268bd2", warning: "#cb4b16", error: "#dc322f", ok: "#859900" }),
    ("solarized-dunkel", "Solarized dunkel", true, Palette { window: "#073642", base: "#002b36", alternate: "#073642", text: "#93a1a1", muted: "#839496", highlight: "#268bd2", highlight_text: "#fdf6e3", button: "#073642", link: "#268bd2", warning: "#b58900", error: "#dc322f", ok: "#859900" }),
    ("gruvbox-hell", "Gruvbox hell", false, Palette { window: "#f2e5bc", base: "#fbf1c7", alternate: "#f2e5bc", text: "#3c3836", muted: "#665c54", highlight: "#076678", highlight_text: "#fbf1c7", button: "#ebdbb2", link: "#076678", warning: "#b57614", error: "#9d0006", ok: "#79740e" }),
    ("gruvbox-dunkel", "Gruvbox dunkel", true, Palette { window: "#1d2021", base: "#282828", alternate: "#32302f", text: "#ebdbb2", muted: "#a89984", highlight: "#83a598", highlight_text: "#1d2021", button: "#3c3836", link: "#83a598", warning: "#fabd2f", error: "#fb4934", ok: "#b8bb26" }),
    ("dracula", "Dracula", true, Palette { window: "#282a36", base: "#282a36", alternate: "#44475a", text: "#f8f8f2", muted: "#6272a4", highlight: "#bd93f9", highlight_text: "#282a36", button: "#44475a", link: "#8be9fd", warning: "#ffb86c", error: "#ff5555", ok: "#50fa7b" }),
];

/// Eine wählbare Palette: Standard, mitgeliefert oder eigene Datei.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// `hell`, `dunkel`, `eingebaut:<id>` oder `datei:<id>`.
    pub id: String,
    pub name: String,
    pub dark: bool,
    pub own: bool,
    pub colors: PaletteData,
}

/// Alle wählbaren Paletten: Standard hell und dunkel, die mitgelieferten, dann die eigenen.
pub fn choices(custom: &[CustomPalette]) -> Vec<Choice> {
    let mut v = vec![
        Choice { id: "hell".into(), name: "Standard hell".into(), dark: false, own: false, colors: LIGHT.into() },
        Choice { id: "dunkel".into(), name: "Standard dunkel".into(), dark: true, own: false, colors: DARK.into() },
    ];
    v.extend(BUILTIN.iter().map(|(id, name, dark, p)| Choice { id: format!("eingebaut:{id}"), name: name.to_string(), dark: *dark, own: false, colors: (*p).into() }));
    v.extend(custom.iter().map(|p| Choice { id: format!("datei:{}", p.id), name: p.name.clone(), dark: p.dark, own: true, colors: p.colors.clone() }));
    v
}

/// Dateiname für eine neue eigene Palette aus ihrem Namen (nur a–z, 0–9, `-`).
pub fn file_id(name: &str) -> String {
    let mut s = String::new();
    for c in name.to_lowercase().chars() {
        let c = match c {
            'ä' => "ae".to_string(),
            'ö' => "oe".to_string(),
            'ü' => "ue".to_string(),
            'ß' => "ss".to_string(),
            c if c.is_ascii_alphanumeric() => c.to_string(),
            _ => "-".to_string(),
        };
        s.push_str(&c);
    }
    let s: Vec<&str> = s.split('-').filter(|p| !p.is_empty()).collect();
    let s = s.join("-");
    if s.is_empty() { "palette".into() } else { s.chars().take(60).collect() }
}

/// Speichert eine eigene Palette als `<Ordner>/<id>.json` (überschreibt eine gleichnamige eigene).
pub fn save_custom(dir: &std::path::Path, name: &str, dark: bool, colors: &serde_json::Value) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Bitte einen Namen angeben".into());
    }
    let mut out = serde_json::Map::new();
    out.insert("name".into(), name.into());
    out.insert("dark".into(), dark.into());
    for key in ["window", "base", "alternate", "text", "muted", "highlight", "highlight_text", "button", "link", "warning", "error", "ok"] {
        let v = colors.get(key).and_then(|x| x.as_str()).ok_or_else(|| format!("Farbe „{key}“ fehlt"))?;
        if !valid_hex(v) {
            return Err(format!("„{key}“ ist keine Farbe der Form #rrggbb"));
        }
        out.insert(key.into(), v.to_lowercase().into());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let id = file_id(name);
    let text = serde_json::to_vec_pretty(&serde_json::Value::Object(out)).map_err(|e| e.to_string())?;
    rj_core::fsutil::atomic_write_with_backup(&dir.join(format!("{id}.json")), &text).map_err(|e| e.to_string())?;
    Ok(format!("datei:{id}"))
}

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
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: ohne aufblitzendes Konsolenfenster (das Hauptprogramm hat keine Konsole).
        let out = std::process::Command::new("reg")
            .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "/v", "AppsUseLightTheme"])
            .creation_flags(0x0800_0000)
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

    /// Mitgelieferte Paletten: Text auf allen Flächen und die Auswahl müssen lesbar sein.
    #[test]
    fn builtin_palettes_keep_text_readable() {
        let mut ids = std::collections::HashSet::new();
        for (id, name, dark, p) in BUILTIN {
            assert!(ids.insert(*id), "doppelte Kennung {id}");
            assert_eq!(*dark, luminance(p.base) < 0.2, "{name}: dunkel passt nicht zur Grundfarbe");
            for (fg, bg, min, what) in [
                (p.text, p.base, 4.5, "Text/Grund"),
                (p.text, p.window, 4.5, "Text/Fenster"),
                (p.text, p.alternate, 4.5, "Text/Wechselzeile"),
                (p.text, p.button, 4.5, "Schaltfläche"),
                (p.highlight_text, p.highlight, 3.0, "Auswahl"),
            ] {
                let c = contrast(fg, bg);
                assert!(c >= min, "{name} {what}: {c:.2} < {min}");
            }
            for c in [p.window, p.base, p.alternate, p.text, p.muted, p.highlight, p.highlight_text, p.button, p.link, p.warning, p.error, p.ok] {
                assert!(valid_hex(c), "{name}: {c}");
            }
        }
        assert_eq!(choices(&[]).len(), 2 + BUILTIN.len());
    }

    #[test]
    fn own_palette_is_saved_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let colors = serde_json::to_value(PaletteData::from(DARK)).unwrap();
        let id = save_custom(dir.path(), "Mein Grün äö", true, &colors).unwrap();
        assert_eq!(id, "datei:mein-gruen-aeoe");
        let (list, notes) = load_custom(dir.path());
        assert_eq!((list.len(), list[0].name.as_str(), list[0].dark), (1, "Mein Grün äö", true), "{notes:?}");
        assert!(save_custom(dir.path(), " ", true, &colors).is_err());
        let mut bad = colors.clone();
        bad["text"] = "rot".into();
        assert!(save_custom(dir.path(), "x", true, &bad).is_err());
        assert_eq!(file_id("../../etc"), "etc");
    }

    #[test]
    fn contrast_formula() {
        assert!((contrast("#000000", "#ffffff") - 21.0).abs() < 0.01);
        assert!((contrast("#777777", "#777777") - 1.0).abs() < 0.01);
    }
}
