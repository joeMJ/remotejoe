//! Farben der Symbole im Baum und in den Tabs: je Theme eine prägnante Farbe pro Zieltyp
//! (Catppuccin mit den offiziellen Akzentfarben), auf Wunsch durch eigene Farben ersetzt.

use std::collections::BTreeMap;

/// Arten mit eigener Farbe (Zieltypen, Makro, Ordner).
pub const KINDS: [&str; 9] = ["rdp", "ssh", "vnc", "web", "psrp", "proxmox", "hyperv", "macro", "folder"];

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

/// Farben für das Theme mit der Kennung `id` (`eingebaut:<Name>`, `hell`, `dunkel`, eigene Palette …),
/// danach eigene Farben aus `own` (Art → `#rrggbb`).
pub fn colors(id: &str, dark: bool, own: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let name = id.strip_prefix("eingebaut:").unwrap_or(id);
    let set = THEMES.iter().find(|(n, _)| *n == name).map(|(_, s)| *s).unwrap_or(if dark { STANDARD_DARK } else { STANDARD_LIGHT });
    let mut m: BTreeMap<String, String> = KINDS.iter().zip(set).map(|(k, c)| (k.to_string(), c.to_string())).collect();
    for (k, c) in own {
        if KINDS.contains(&k.as_str()) && valid(c) {
            m.insert(k.clone(), c.clone());
        }
    }
    m
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
        let m = colors("eingebaut:catppuccin-macchiato", true, &none);
        assert_eq!(m["rdp"], "#8aadf4");
        assert_eq!(m["folder"], "#eed49f");
        assert_eq!(colors("hell", false, &none)["ssh"], "#1b6e20");
        assert_eq!(colors("datei:x", true, &none)["ssh"], "#5fc463");
        let own: BTreeMap<String, String> = [("ssh".to_string(), "#123456".to_string()), ("ssh2".into(), "#000000".into()), ("rdp".into(), "rot".into())].into();
        let m = colors("eingebaut:nord", true, &own);
        assert_eq!((m["ssh"].as_str(), m["rdp"].as_str(), m.len()), ("#123456", "#88c0d0", KINDS.len()));
    }
}
