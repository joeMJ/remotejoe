//! Farbschemata und Schrifteinstellungen des SSH-Terminals.
//!
//! Die Schemata „Catppuccin Latte/Frappé/Macchiato/Mocha“ verwenden die Farben der
//! Catppuccin-Palette (MIT-Lizenz, <https://github.com/catppuccin/catppuccin>).

use serde::{Deserialize, Serialize};

pub type Rgb = [u8; 3];

/// Ein vollständiges Farbschema: Vordergrund, Hintergrund, Cursor, Auswahl und die 16 ANSI-Farben.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalScheme {
    pub id: &'static str,
    pub label: &'static str,
    pub fg: Rgb,
    pub bg: Rgb,
    pub cursor: Rgb,
    pub selection: Rgb,
    /// 0–7 normal (schwarz, rot, grün, gelb, blau, magenta, cyan, weiß), 8–15 hell.
    pub ansi: [Rgb; 16],
}

const fn hex(v: u32) -> Rgb {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/// Schema „standard-dunkel“: wie bisher (xterm-Farben auf Schwarz).
pub const STANDARD_DARK: TerminalScheme = TerminalScheme {
    id: "standard-dunkel",
    label: "Standard dunkel",
    fg: [229, 229, 229],
    bg: [0, 0, 0],
    cursor: [229, 229, 229],
    selection: [48, 100, 160],
    ansi: [
        [0, 0, 0], [205, 0, 0], [0, 205, 0], [205, 205, 0], [0, 0, 238], [205, 0, 205], [0, 205, 205], [229, 229, 229],
        [127, 127, 127], [255, 0, 0], [0, 255, 0], [255, 255, 0], [92, 92, 255], [255, 0, 255], [0, 255, 255], [255, 255, 255],
    ],
};

pub const STANDARD_LIGHT: TerminalScheme = TerminalScheme {
    id: "standard-hell",
    label: "Standard hell",
    fg: hex(0x1f1f1f),
    bg: hex(0xffffff),
    cursor: hex(0x1f1f1f),
    selection: hex(0xb3d4fc),
    ansi: [
        hex(0x000000), hex(0xc91b00), hex(0x00a600), hex(0x998a00), hex(0x0225c7), hex(0xc930c7), hex(0x00a5aa), hex(0x6b6b6b),
        hex(0x5b5b5b), hex(0xe03a24), hex(0x00b800), hex(0xb39f00), hex(0x3a52e0), hex(0xdd4cdb), hex(0x00b4b9), hex(0x1f1f1f),
    ],
};

pub const HIGH_CONTRAST: TerminalScheme = TerminalScheme {
    id: "hoher-kontrast",
    label: "Hoher Kontrast",
    fg: hex(0xffffff),
    bg: hex(0x000000),
    cursor: hex(0xffff00),
    selection: hex(0x0050c8),
    ansi: [
        hex(0x000000), hex(0xff5555), hex(0x55ff55), hex(0xffff55), hex(0x7aa2ff), hex(0xff77ff), hex(0x55ffff), hex(0xe6e6e6),
        hex(0x8a8a8a), hex(0xff8080), hex(0x80ff80), hex(0xffff80), hex(0xa0b8ff), hex(0xff99ff), hex(0x80ffff), hex(0xffffff),
    ],
};

pub const CATPPUCCIN_LATTE: TerminalScheme = TerminalScheme {
    id: "catppuccin-latte",
    label: "Catppuccin Latte (hell)",
    fg: hex(0x4c4f69),
    bg: hex(0xeff1f5),
    cursor: hex(0xdc8a78),
    selection: hex(0xacb0be),
    ansi: [
        hex(0x5c5f77), hex(0xd20f39), hex(0x40a02b), hex(0xdf8e1d), hex(0x1e66f5), hex(0xea76cb), hex(0x179299), hex(0xacb0be),
        hex(0x6c6f85), hex(0xd20f39), hex(0x40a02b), hex(0xdf8e1d), hex(0x1e66f5), hex(0xea76cb), hex(0x179299), hex(0xbcc0cc),
    ],
};

pub const CATPPUCCIN_FRAPPE: TerminalScheme = TerminalScheme {
    id: "catppuccin-frappe",
    label: "Catppuccin Frappé (dunkel)",
    fg: hex(0xc6d0f5),
    bg: hex(0x303446),
    cursor: hex(0xf2d5cf),
    selection: hex(0x626880),
    ansi: [
        hex(0x51576d), hex(0xe78284), hex(0xa6d189), hex(0xe5c890), hex(0x8caaee), hex(0xf4b8e4), hex(0x81c8be), hex(0xb5bfe2),
        hex(0x626880), hex(0xe78284), hex(0xa6d189), hex(0xe5c890), hex(0x8caaee), hex(0xf4b8e4), hex(0x81c8be), hex(0xa5adce),
    ],
};

pub const CATPPUCCIN_MACCHIATO: TerminalScheme = TerminalScheme {
    id: "catppuccin-macchiato",
    label: "Catppuccin Macchiato (dunkel)",
    fg: hex(0xcad3f5),
    bg: hex(0x24273a),
    cursor: hex(0xf4dbd6),
    selection: hex(0x5b6078),
    ansi: [
        hex(0x494d64), hex(0xed8796), hex(0xa6da95), hex(0xeed49f), hex(0x8aadf4), hex(0xf5bde6), hex(0x8bd5ca), hex(0xb8c0e0),
        hex(0x5b6078), hex(0xed8796), hex(0xa6da95), hex(0xeed49f), hex(0x8aadf4), hex(0xf5bde6), hex(0x8bd5ca), hex(0xa5adcb),
    ],
};

pub const CATPPUCCIN_MOCHA: TerminalScheme = TerminalScheme {
    id: "catppuccin-mocha",
    label: "Catppuccin Mocha (dunkel)",
    fg: hex(0xcdd6f4),
    bg: hex(0x1e1e2e),
    cursor: hex(0xf5e0dc),
    selection: hex(0x585b70),
    ansi: [
        hex(0x45475a), hex(0xf38ba8), hex(0xa6e3a1), hex(0xf9e2af), hex(0x89b4fa), hex(0xf5c2e7), hex(0x94e2d5), hex(0xbac2de),
        hex(0x585b70), hex(0xf38ba8), hex(0xa6e3a1), hex(0xf9e2af), hex(0x89b4fa), hex(0xf5c2e7), hex(0x94e2d5), hex(0xa6adc8),
    ],
};

/// Alle eingebauten Schemata in der Reihenfolge der Auswahl.
pub fn presets() -> [&'static TerminalScheme; 7] {
    [&STANDARD_DARK, &STANDARD_LIGHT, &HIGH_CONTRAST, &CATPPUCCIN_MOCHA, &CATPPUCCIN_MACCHIATO, &CATPPUCCIN_FRAPPE, &CATPPUCCIN_LATTE]
}

pub fn preset(id: &str) -> Option<&'static TerminalScheme> {
    presets().into_iter().find(|s| s.id == id)
}

/// `#rrggbb` (auch ohne `#`) lesen.
pub fn parse_hex(s: &str) -> Option<Rgb> {
    let t = s.trim().trim_start_matches('#');
    if t.len() != 6 || !t.is_ascii() {
        return None;
    }
    let v = u32::from_str_radix(t, 16).ok()?;
    Some(hex(v))
}

pub fn to_hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

pub const FONT_SIZE_MIN: f32 = 6.0;
pub const FONT_SIZE_MAX: f32 = 40.0;
pub const FONT_SIZE_DEFAULT: f32 = 11.0;

/// Schrift und Farben des Terminals (Konfiguration, Abschnitt `[terminal]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalSettings {
    /// Schriftart; leer bedeutet die Festbreitenschrift des Systems.
    pub font_family: Option<String>,
    pub font_size: f32,
    /// Kennung eines eingebauten Schemas.
    pub scheme: String,
    /// Eigene Abweichungen (`#rrggbb`) vom gewählten Schema.
    pub foreground: Option<String>,
    pub background: Option<String>,
    pub cursor: Option<String>,
    pub selection: Option<String>,
    /// Zeilen im Rücklauf (Mausrad zurückblättern).
    pub scrollback: usize,
}

/// Grenzen und Vorgabe des Rücklaufs (Zeilen).
pub const SCROLLBACK_DEFAULT: usize = 10_000;
pub const SCROLLBACK_MIN: usize = 100;
pub const SCROLLBACK_MAX: usize = 200_000;

impl Default for TerminalSettings {
    fn default() -> Self {
        TerminalSettings {
            font_family: None,
            font_size: FONT_SIZE_DEFAULT,
            scheme: STANDARD_DARK.id.into(),
            foreground: None,
            background: None,
            cursor: None,
            selection: None,
            scrollback: SCROLLBACK_DEFAULT,
        }
    }
}

/// Wirksames Farbschema samt Überschreibungen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveScheme {
    pub fg: Rgb,
    pub bg: Rgb,
    pub cursor: Rgb,
    pub selection: Rgb,
    pub ansi: [Rgb; 16],
}

impl TerminalSettings {
    /// Rücklauf im erlaubten Bereich.
    pub fn clamped_scrollback(&self) -> usize {
        self.scrollback.clamp(SCROLLBACK_MIN, SCROLLBACK_MAX)
    }

    /// Schriftgröße im erlaubten Bereich.
    pub fn clamped_size(&self) -> f32 {
        if self.font_size.is_finite() { self.font_size.clamp(FONT_SIZE_MIN, FONT_SIZE_MAX) } else { FONT_SIZE_DEFAULT }
    }

    pub fn effective(&self) -> EffectiveScheme {
        let base = preset(&self.scheme).unwrap_or(&STANDARD_DARK);
        let pick = |own: &Option<String>, dflt: Rgb| own.as_deref().and_then(parse_hex).unwrap_or(dflt);
        EffectiveScheme {
            fg: pick(&self.foreground, base.fg),
            bg: pick(&self.background, base.bg),
            cursor: pick(&self.cursor, base.cursor),
            selection: pick(&self.selection, base.selection),
            ansi: base.ansi,
        }
    }

    /// Hat der Benutzer Farben des Schemas von Hand geändert?
    pub fn has_overrides(&self) -> bool {
        self.foreground.is_some() || self.background.is_some() || self.cursor.is_some() || self.selection.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_unique_and_complete() {
        let p = presets();
        let mut ids: Vec<_> = p.iter().map(|s| s.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), p.len());
        assert!(preset("catppuccin-mocha").is_some());
        assert!(preset("gibt-es-nicht").is_none());
        // Catppuccin Mocha: Hintergrund „Base“ und Vordergrund „Text“.
        assert_eq!(to_hex(CATPPUCCIN_MOCHA.bg), "#1e1e2e");
        assert_eq!(to_hex(CATPPUCCIN_MOCHA.fg), "#cdd6f4");
        assert_eq!(to_hex(CATPPUCCIN_LATTE.bg), "#eff1f5");
    }

    #[test]
    fn dark_scheme_keeps_the_old_xterm_colours() {
        assert_eq!(STANDARD_DARK.fg, [229, 229, 229]);
        assert_eq!(STANDARD_DARK.bg, [0, 0, 0]);
        assert_eq!(STANDARD_DARK.ansi[1], [205, 0, 0]);
        assert_eq!(STANDARD_DARK.ansi[12], [92, 92, 255]);
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(parse_hex("#1e1e2e"), Some([0x1e, 0x1e, 0x2e]));
        assert_eq!(parse_hex("ffffff"), Some([255, 255, 255]));
        assert_eq!(parse_hex("#12345"), None);
        assert_eq!(parse_hex("#gggggg"), None);
        assert_eq!(parse_hex("#ä12345"), None);
    }

    #[test]
    fn overrides_win_and_bad_values_fall_back() {
        let mut s = TerminalSettings { scheme: "catppuccin-latte".into(), background: Some("#102030".into()), cursor: Some("kaputt".into()), ..Default::default() };
        let e = s.effective();
        assert_eq!(e.bg, [0x10, 0x20, 0x30]);
        assert_eq!(e.cursor, CATPPUCCIN_LATTE.cursor);
        assert_eq!(e.fg, CATPPUCCIN_LATTE.fg);
        s.scheme = "unbekannt".into();
        assert_eq!(s.effective().fg, STANDARD_DARK.fg);
    }

    #[test]
    fn size_is_clamped() {
        let mut s = TerminalSettings::default();
        s.font_size = 2.0;
        assert_eq!(s.clamped_size(), FONT_SIZE_MIN);
        s.font_size = 400.0;
        assert_eq!(s.clamped_size(), FONT_SIZE_MAX);
        s.font_size = f32::NAN;
        assert_eq!(s.clamped_size(), FONT_SIZE_DEFAULT);
    }
}
