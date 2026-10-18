//! Farben der Symbole im Baum und in den Tabs: je Theme eine prägnante Farbe pro Zieltyp
//! (Catppuccin mit den offiziellen Akzentfarben), auf Wunsch durch eigene Farben ersetzt.

use std::collections::BTreeMap;

/// Arten mit eigener Farbe (Zieltypen, Makro, Ordner).
pub const KINDS: [&str; 10] = ["rdp", "ssh", "vnc", "web", "psrp", "proxmox", "hyperv", "macro", "folder", "session"];

/// Farben in der Reihenfolge von [`KINDS`].
type Set = [&'static str; 9];

const STANDARD_LIGHT: Set = ["#0b5cad", "#1b6e20", "#7a3e9d", "#a35f00", "#005f73", "#b03a00", "#3a4a8c", "#6a1b9a", "#b8901a"];
const STANDARD_DARK: Set = ["#5aa2f0", "#5fc463", "#c08ae0", "#f0a24a", "#3fb8c8", "#f07a4a", "#8f9ef0", "#d08ae8", "#e3c04f"];

/// Feste Zuordnung je eingebautem Theme: blau, grün, mauve, pfirsich, türkis, rot, lavendel, rosa, gelb.
const THEMES: [(&str, Set); 11] = [
    ("catppuccin-latte", ["#1e66f5", "#40a02b", "#8839ef", "#fe640b", "#179299", "#d20f39", "#7287fd", "#ea76cb", "#df8e1d"]),
    ("catppuccin-frappe", ["#8caaee", "#a6d189", "#ca9ee6", "#ef9f76", "#81c8be", "#e78284", "#babbf1", "#f4b8e4", "#e5c890"]),
    ("catppuccin-macchiato", ["#8aadf4", "#a6da95", "#c6a0f6", "#f5a97f", "#8bd5ca", "#ed8796", "#b7bdf8", "#f5bde6", "#eed49f"]),
    ("catppuccin-mocha", ["#89b4fa", "#a6e3a1", "#cba6f7", "#fab387", "#94e2d5", "#f38ba8", "#b4befe", "#f5c2e7", "#f9e2af"]),
    ("nord-hell", ["#5e81ac", "#5f7a4a", "#8f5f87", "#b4583c", "#3e7b7a", "#a3333c", "#4c5f8a", "#9a4f8c", "#a07a1e"]),
    ("nord", ["#88c0d0", "#a3be8c", "#b48ead", "#d08770", "#8fbcbb", "#bf616a", "#81a1c1", "#c895bf", "#ebcb8b"]),
    ("solarized-hell", ["#268bd2", "#859900", "#6c71c4", "#cb4b16", "#2aa198", "#dc322f", "#5b72c4", "#d33682", "#b58900"]),
    ("solarized-dunkel", ["#268bd2", "#859900", "#6c71c4", "#cb4b16", "#2aa198", "#dc322f", "#7a8fd6", "#d33682", "#b58900"]),
    ("gruvbox-hell", ["#076678", "#79740e", "#8f3f71", "#af3a03", "#427b58", "#9d0006", "#45588a", "#b16286", "#b57614"]),
    ("gruvbox-dunkel", ["#83a598", "#b8bb26", "#d3869b", "#fe8019", "#8ec07c", "#fb4934", "#a5b4e0", "#e3a0c0", "#fabd2f"]),
    ("dracula", ["#8be9fd", "#50fa7b", "#bd93f9", "#ffb86c", "#6be5c8", "#ff5555", "#a9b6ff", "#ff79c6", "#f1fa8c"]),
];

/// Farbstil: `einheitlich` (alle Ziele in der Akzentfarbe des Themes, Ordner gedämpft) oder `je_typ`
/// (eine Farbe je Zieltyp, in Helligkeit und Sättigung angeglichen).
pub const STYLES: [&str; 2] = ["einheitlich", "je_typ"];

/// Farben für das Theme mit der Kennung `id` (`eingebaut:<Name>`, `hell`, `dunkel`, eigene Palette …)
/// im Stil `style`; `accent` und `muted` sind Hervorhebungs- und Nebentextfarbe des Themes. Danach
/// gelten eigene Farben aus `own` (Art → `#rrggbb`).
pub fn colors(id: &str, dark: bool, style: &str, accent: &str, muted: &str, own: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let name = id.strip_prefix("eingebaut:").unwrap_or(id);
    let set = THEMES.iter().find(|(n, _)| *n == name).map(|(_, s)| *s).unwrap_or(if dark { STANDARD_DARK } else { STANDARD_LIGHT });
    let mut m: BTreeMap<String, String> = if style == "je_typ" {
        // Gleiche Helligkeit und höchstens mittlere Sättigung, damit keine Farbe heraussticht.
        KINDS.iter().zip(set).map(|(k, c)| (k.to_string(), harmonize(c, dark))).collect()
    } else {
        let accent = if valid(accent) { accent } else { set[0] };
        let folder = if valid(muted) { muted } else { set[8] };
        KINDS.iter().map(|k| (k.to_string(), if *k == "folder" { folder } else { accent }.to_string())).collect()
    };
    // Punkt für offene Sitzungen: das Grün des Themes, in beiden Stilen gut sichtbar.
    m.insert("session".into(), set[1].to_string());
    for (k, c) in own {
        if KINDS.contains(&k.as_str()) && valid(c) {
            m.insert(k.clone(), c.clone());
        }
    }
    m
}

/// Farbe auf einheitliche Helligkeit (hell: 42 %, dunkel: 72 %) und höchstens 60 % Sättigung bringen.
fn harmonize(c: &str, dark: bool) -> String {
    let Some((h, s, _)) = to_hsl(c) else { return c.to_string() };
    from_hsl(h, s.min(0.6), if dark { 0.72 } else { 0.42 })
}

fn to_hsl(c: &str) -> Option<(f64, f64, f64)> {
    if !valid(c) {
        return None;
    }
    let v = |i: usize| u8::from_str_radix(&c[i..i + 2], 16).map(|x| f64::from(x) / 255.0).ok();
    let (r, g, b) = (v(1)?, v(3)?, v(5)?);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    if (max - min).abs() < f64::EPSILON {
        return Some((0.0, 0.0, l));
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r { (g - b) / d + if g < b { 6.0 } else { 0.0 } } else if max == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 };
    Some((h / 6.0, s, l))
}

fn from_hsl(h: f64, s: f64, l: f64) -> String {
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let t = |mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let v = if t < 1.0 / 6.0 { p + (q - p) * 6.0 * t } else if t < 0.5 { q } else if t < 2.0 / 3.0 { p + (q - p) * (2.0 / 3.0 - t) * 6.0 } else { p };
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    format!("#{:02x}{:02x}{:02x}", t(h + 1.0 / 3.0), t(h), t(h - 1.0 / 3.0))
}

/// `#rrggbb`?
pub fn valid(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|ch| ch.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_colors_and_overrides() {
        let none = BTreeMap::new();
        // Einheitlich: Ziele in der Akzentfarbe, Ordner gedämpft.
        let m = colors("eingebaut:catppuccin-macchiato", true, "einheitlich", "#8aadf4", "#a5adcb", &none);
        assert_eq!((m["rdp"].as_str(), m["ssh"].as_str(), m["folder"].as_str()), ("#8aadf4", "#8aadf4", "#a5adcb"));
        // Je Typ: angeglichene Helligkeit.
        let m = colors("eingebaut:solarized-hell", false, "je_typ", "#268bd2", "#93a1a1", &none);
        let l = |c: &str| to_hsl(c).unwrap().2;
        assert!(KINDS.iter().filter(|k| **k != "session").all(|k| (l(&m[*k]) - 0.42).abs() < 0.01), "{m:?}");
        assert_ne!(m["rdp"], m["ssh"]);
        let own: BTreeMap<String, String> = [("ssh".to_string(), "#123456".to_string()), ("ssh2".into(), "#000000".into()), ("rdp".into(), "rot".into())].into();
        let m = colors("eingebaut:nord", true, "einheitlich", "#88c0d0", "#d8dee9", &own);
        assert_eq!((m["ssh"].as_str(), m["rdp"].as_str(), m.len()), ("#123456", "#88c0d0", KINDS.len()));
        assert_eq!(from_hsl(0.0, 1.0, 0.5), "#ff0000");
    }
}
