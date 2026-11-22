//! Administrativer Tastaturmodus für RDP: Zeichen als feste Tastenfolgen (Scancodes) statt als Unicode-Ereignisse.
//!
//! Hintergrund: Windows nimmt Unicode-Tastenereignisse in Fenstern mit erhöhten Rechten (PowerShell oder CMD „als
//! Administrator“) nicht an. Unicode bleibt trotzdem die Vorgabe, weil Tastendrücke von der Belegung der Sitzung
//! abhängen; ein falsch übersetztes AltGr kann unsichtbare Zeichen erzeugen (Kennwort falsch, Konto gesperrt).
//!
//! Regeln im Modus:
//! - Jedes Zeichen wird einzeln zu einer vollständigen Folge (Umschaltstufe drücken, Taste drücken und loslassen,
//!   Umschaltstufe loslassen). AltGr ist die rechte Alt-Taste.
//! - Umschalt, Strg, Alt und AltGr allein erzeugen nichts; mit einer anderen Taste ergeben sie ein Kürzel, das
//!   ebenfalls als ganze Folge gesendet wird. Am Server bleibt so nie eine Umschalttaste hängen.
//! - Zeichen auf Tottasten und Zeichen ohne eigene Taste gehen weiter als Unicode.
//!
//! Die Tabellen stammen aus den Belegungsdateien von Windows (`res/tastaturbelegungen.json`, erzeugt mit
//! `tools/kbdgen`), weil die Gegenstellen Windows-Server sind.

use rj_ipc::InputEvent;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

/// Scancodes der Umschalttasten (Set 1, `0x100` = E0-Präfix).
pub const SC_LSHIFT: u32 = 0x2A;
pub const SC_LCTRL: u32 = 0x1D;
pub const SC_LALT: u32 = 0x38;
pub const SC_RALT: u32 = 0x138;
pub const SC_LWIN: u32 = 0x15B;

/// Umschaltstufe eines Zeichens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Plain,
    Shift,
    AltGr,
    ShiftAltGr,
}

impl Level {
    fn parse(s: &str) -> Option<Level> {
        Some(match s {
            "" => Level::Plain,
            "shift" => Level::Shift,
            "altgr" => Level::AltGr,
            "shift+altgr" => Level::ShiftAltGr,
            _ => return None,
        })
    }

    fn modifiers(self) -> &'static [u32] {
        match self {
            Level::Plain => &[],
            Level::Shift => &[SC_LSHIFT],
            Level::AltGr => &[SC_RALT],
            Level::ShiftAltGr => &[SC_LSHIFT, SC_RALT],
        }
    }
}

/// Eine Tastaturbelegung: Zeichen → Taste und Stufe; Tottasten.
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub klid: String,
    keys: HashMap<char, (u32, Level)>,
    dead: HashSet<char>,
}

#[derive(Deserialize)]
struct FileJson {
    belegungen: Vec<LayoutJson>,
}

#[derive(Deserialize)]
struct LayoutJson {
    klid: String,
    tasten: Vec<KeyJson>,
    #[serde(default)]
    tot: Vec<u32>,
}

#[derive(Deserialize)]
struct KeyJson {
    z: u32,
    sc: u32,
    #[serde(default)]
    e0: bool,
    #[serde(default)]
    stufe: String,
}

/// Alle Belegungen aus der Tabelle (JSON wie von `tools/kbdgen`).
pub fn parse_layouts(json: &str) -> Result<Vec<Layout>, String> {
    let f: FileJson = serde_json::from_str(json).map_err(|e| format!("Tastaturtabelle unlesbar: {e}"))?;
    Ok(f.belegungen
        .into_iter()
        .map(|l| Layout {
            klid: l.klid.to_ascii_uppercase(),
            keys: l
                .tasten
                .into_iter()
                .filter_map(|k| Some((char::from_u32(k.z)?, (k.sc | if k.e0 { 0x100 } else { 0 }, Level::parse(&k.stufe)?))))
                .collect(),
            dead: l.tot.into_iter().filter_map(char::from_u32).collect(),
        })
        .collect())
}

/// Angebotene Belegungen (KLID, Name) in der Reihenfolge der Auswahl.
pub const NAMES: [(&str, &str); 17] = [
    ("00000407", "Deutsch (Deutschland, Österreich)"),
    ("00000807", "Deutsch (Schweiz)"),
    ("0000040C", "Französisch (Frankreich)"),
    ("0000080C", "Französisch (Belgien)"),
    ("0000100C", "Französisch (Schweiz)"),
    ("00000809", "Englisch (Vereinigtes Königreich)"),
    ("00000409", "Englisch (USA)"),
    ("00000410", "Italienisch"),
    ("0000040A", "Spanisch"),
    ("00000816", "Portugiesisch"),
    ("00000413", "Niederländisch"),
    ("0000041D", "Schwedisch"),
    ("0000040B", "Finnisch"),
    ("00000414", "Norwegisch"),
    ("00000406", "Dänisch"),
    ("00000415", "Polnisch (Programmierer)"),
    ("00000405", "Tschechisch"),
];

/// Anzeigename einer Belegung.
pub fn name_of(klid: &str) -> String {
    NAMES.iter().find(|(k, _)| k.eq_ignore_ascii_case(klid)).map(|(_, n)| n.to_string()).unwrap_or_else(|| klid.to_string())
}

/// Mitgelieferte Belegungen (aus den Windows-Belegungsdateien).
pub fn builtin() -> &'static [Layout] {
    static L: std::sync::OnceLock<Vec<Layout>> = std::sync::OnceLock::new();
    L.get_or_init(|| parse_layouts(include_str!("../res/tastaturbelegungen.json")).unwrap_or_default())
}

/// Belegungskennung (KLID, „00000407“) aus der Angabe am Ziel („de“, „de-CH“, „0x0407“, „00000407“ …); leer = keine.
pub fn klid_of(spec: &str) -> Option<String> {
    let s = spec.trim();
    let id: u32 = match s {
        "" => return None,
        "de" | "de-DE" | "de-AT" => 0x0407,
        "de-CH" => 0x0807,
        "fr" | "fr-FR" => 0x040C,
        "fr-BE" => 0x080C,
        "fr-CH" => 0x100C,
        "en-GB" | "gb" => 0x0809,
        "en-US" | "us" => 0x0409,
        "it" | "it-IT" => 0x0410,
        "es" | "es-ES" => 0x040A,
        "pt" | "pt-PT" => 0x0816,
        "nl" | "nl-NL" => 0x0413,
        "sv" | "sv-SE" => 0x041D,
        "fi" | "fi-FI" => 0x040B,
        "no" | "nb" | "nb-NO" => 0x0414,
        "da" | "da-DK" => 0x0406,
        "pl" | "pl-PL" => 0x0415,
        "cs" | "cs-CZ" => 0x0405,
        _ => u32::from_str_radix(s.trim_start_matches("0x"), 16).ok()?,
    };
    // Die Kennung am Ziel ist eine Sprach-ID (0x0C07 Österreich); die Belegung dazu ist die deutsche.
    let id = if id == 0x0C07 { 0x0407 } else { id };
    Some(format!("{id:08X}"))
}

impl Layout {
    pub fn find<'a>(layouts: &'a [Layout], klid: &str) -> Option<&'a Layout> {
        layouts.iter().find(|l| l.klid.eq_ignore_ascii_case(klid))
    }

    /// Folge für ein Zeichen; `None` = als Unicode senden (Tottaste oder keine eigene Taste).
    pub fn sequence(&self, c: char) -> Option<Vec<(u32, bool)>> {
        if self.dead.contains(&c) {
            return None;
        }
        let (sc, level) = *self.keys.get(&c)?;
        Some(wrap(level.modifiers(), sc))
    }

    /// Scancode einer Taste ohne Umschaltstufe (für Kürzel wie Strg+Z: die Taste, die hier „z“ schreibt).
    pub fn plain_key(&self, c: char) -> Option<u32> {
        match self.keys.get(&c.to_lowercase().next()?)? {
            (sc, Level::Plain) => Some(*sc),
            _ => None,
        }
    }
}

/// Umschalttasten drücken, Taste drücken und loslassen, Umschalttasten in umgekehrter Reihenfolge loslassen.
pub fn wrap(mods: &[u32], key: u32) -> Vec<(u32, bool)> {
    let mut v: Vec<(u32, bool)> = mods.iter().map(|m| (*m, true)).collect();
    v.push((key, true));
    v.push((key, false));
    v.extend(mods.iter().rev().map(|m| (*m, false)));
    v
}

/// Ist der Keysym eine Umschalttaste (Umschalt, Strg, Alt, AltGr, Windows-Taste, Feststell)?
pub fn is_modifier_keysym(k: u32) -> bool {
    matches!(k, 0xffe1..=0xffee | 0xfe03 | 0xff7e)
}

/// Umschalttasten eines Kürzels aus den Qt-Umschaltern (Reihenfolge Strg, Alt, Umschalt, Windows).
pub fn chord_modifiers(modifiers: u32) -> Vec<u32> {
    let mut v = Vec::new();
    if modifiers & crate::keys::CTRL != 0 {
        v.push(SC_LCTRL);
    }
    if modifiers & crate::keys::ALT != 0 {
        v.push(SC_LALT);
    }
    if modifiers & crate::keys::SHIFT != 0 {
        v.push(SC_LSHIFT);
    }
    if modifiers & crate::keys::META != 0 {
        v.push(SC_LWIN);
    }
    v
}

/// Zustand je Sitzung, solange der Modus an ist.
#[derive(Debug, Clone)]
pub struct AdminKeys {
    pub layout: Layout,
    /// Offene Kürzel: Tastencode der Oberfläche → Umschalttasten, die beim Loslassen wieder gelöst werden.
    open: HashMap<u32, (Vec<u32>, InputEvent)>,
    /// Windows-Taste gedrückt, ohne dass eine andere Taste folgte (allein = Startmenü).
    win_alone: bool,
}

impl AdminKeys {
    pub fn new(layout: Layout) -> Self {
        AdminKeys { layout, open: HashMap::new(), win_alone: false }
    }

    /// Ein Tastenereignis der Oberfläche → Ereignisse an den Worker.
    pub fn translate(&mut self, qt_key: u32, modifiers: u32, text: &str, down: bool) -> Vec<InputEvent> {
        let keysym = crate::keys::keysym(qt_key);
        // Umschalttasten allein: nichts senden; nur die Windows-Taste allein öffnet beim Loslassen das Startmenü.
        if keysym.is_some_and(is_modifier_keysym) || is_qt_modifier(qt_key) {
            let win = matches!(keysym, Some(0xffeb | 0xffec)) || qt_key == 0x0100_0022;
            if win {
                if down {
                    self.win_alone = true;
                } else if std::mem::take(&mut self.win_alone) {
                    return vec![InputEvent::Scancodes(wrap(&[], SC_LWIN))];
                }
            }
            return Vec::new();
        }
        self.win_alone = false;
        let ctrl_alt = modifiers & (crate::keys::CTRL | crate::keys::ALT) == crate::keys::CTRL | crate::keys::ALT;
        let shortcut = modifiers & (crate::keys::CTRL | crate::keys::ALT | crate::keys::META) != 0;
        let printable = !text.is_empty() && text.chars().all(|c| !c.is_control());
        // Text: ohne Kürzel-Umschalter, oder unter Windows AltGr als Strg+Alt mit druckbarem Text.
        if printable && (!shortcut || (ctrl_alt && modifiers & crate::keys::META == 0)) {
            if !down {
                return Vec::new();
            }
            return text
                .chars()
                .map(|c| match self.layout.sequence(c) {
                    Some(seq) => InputEvent::Scancodes(seq),
                    None => InputEvent::Text(c.to_string()),
                })
                .collect();
        }
        // Kürzel oder Sondertaste: Umschalter drücken, Taste drücken; beim Loslassen in umgekehrter Reihenfolge lösen.
        if down {
            let mods = chord_modifiers(modifiers);
            let key = match char::from_u32(qt_key).filter(|c| c.is_ascii_graphic()).and_then(|c| self.layout.plain_key(c)) {
                Some(sc) => InputEvent::Scancodes(vec![(sc, true)]),
                None => match keysym {
                    Some(k) => InputEvent::Key { keysym: k, down: true },
                    None => return Vec::new(),
                },
            };
            let release = match &key {
                InputEvent::Scancodes(v) => InputEvent::Scancodes(vec![(v[0].0, false)]),
                InputEvent::Key { keysym, .. } => InputEvent::Key { keysym: *keysym, down: false },
                _ => unreachable!(),
            };
            let mut out = Vec::new();
            // Wiederholung (Taste gehalten): die Umschalter sind schon gedrückt.
            if !self.open.contains_key(&qt_key) && !mods.is_empty() {
                out.push(InputEvent::Scancodes(mods.iter().map(|m| (*m, true)).collect()));
            }
            out.push(key);
            self.open.insert(qt_key, (mods, release));
            return out;
        }
        match self.open.remove(&qt_key) {
            Some((mods, release)) => {
                let mut out = vec![release];
                if !mods.is_empty() {
                    out.push(InputEvent::Scancodes(mods.iter().rev().map(|m| (*m, false)).collect()));
                }
                out
            }
            None => Vec::new(),
        }
    }

    /// Alles Offene lösen (Modus aus, Fokus weg).
    pub fn release_all(&mut self) -> Vec<InputEvent> {
        let mut out = Vec::new();
        for (_, (mods, release)) in self.open.drain() {
            out.push(release);
            if !mods.is_empty() {
                out.push(InputEvent::Scancodes(mods.iter().rev().map(|m| (*m, false)).collect()));
            }
        }
        self.win_alone = false;
        out
    }
}

/// Qt-Tastencodes der Umschalttasten (Shift, Control, Meta, Alt, AltGr, CapsLock).
fn is_qt_modifier(qt_key: u32) -> bool {
    matches!(qt_key, 0x0100_0020..=0x0100_0024 | 0x0100_1103)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{ALT, CTRL, META, SHIFT};

    /// Kontrolltabelle Deutsch (von Hand, nach der Windows-Belegung 00000407).
    const DE: &str = r#"{"belegungen":[{"klid":"00000407","tasten":[
        {"z":97,"sc":30},{"z":65,"sc":30,"stufe":"shift"},{"z":122,"sc":21},{"z":121,"sc":44},
        {"z":64,"sc":16,"stufe":"altgr"},{"z":8364,"sc":18,"stufe":"altgr"},{"z":124,"sc":86,"stufe":"altgr"},
        {"z":123,"sc":8,"stufe":"altgr"},{"z":91,"sc":9,"stufe":"altgr"},{"z":93,"sc":10,"stufe":"altgr"},
        {"z":125,"sc":11,"stufe":"altgr"},{"z":92,"sc":12,"stufe":"altgr"},{"z":126,"sc":27,"stufe":"altgr"},
        {"z":223,"sc":12},{"z":228,"sc":40},{"z":196,"sc":40,"stufe":"shift"},{"z":49,"sc":2},{"z":33,"sc":2,"stufe":"shift"},
        {"z":47,"sc":53,"e0":true},{"z":99,"sc":46}],
        "tot":[94,96,180]}]}"#;

    fn de() -> AdminKeys {
        AdminKeys::new(parse_layouts(DE).unwrap().remove(0))
    }

    fn scs(ev: &[InputEvent]) -> Vec<(u32, bool)> {
        ev.iter()
            .flat_map(|e| match e {
                InputEvent::Scancodes(v) => v.clone(),
                other => panic!("kein Scancode: {other:?}"),
            })
            .collect()
    }

    #[test]
    fn characters_become_complete_sequences() {
        let mut k = de();
        assert_eq!(scs(&k.translate('A' as u32, 0, "a", true)), vec![(0x1E, true), (0x1E, false)]);
        assert_eq!(scs(&k.translate('A' as u32, SHIFT, "A", true)), vec![(SC_LSHIFT, true), (0x1E, true), (0x1E, false), (SC_LSHIFT, false)]);
        assert_eq!(scs(&k.translate('Z' as u32, 0, "z", true)), vec![(0x15, true), (0x15, false)], "z liegt auf Deutsch auf 0x15");
        // AltGr-Zeichen: rechte Alt-Taste um die Taste, nie ein einzelnes AltGr.
        for (c, sc) in [('@', 0x10), ('€', 0x12), ('|', 0x56), ('{', 0x08), ('[', 0x09), (']', 0x0A), ('}', 0x0B), ('\\', 0x0C), ('~', 0x1B)] {
            assert_eq!(scs(&k.translate(0, 0, &c.to_string(), true)), vec![(SC_RALT, true), (sc, true), (sc, false), (SC_RALT, false)], "{c}");
        }
        assert_eq!(scs(&k.translate(0, 0, "ß", true)), vec![(0x0C, true), (0x0C, false)]);
        assert_eq!(scs(&k.translate(0, 0, "Ä", true)), vec![(SC_LSHIFT, true), (0x28, true), (0x28, false), (SC_LSHIFT, false)]);
        assert_eq!(scs(&k.translate(0, 0, "/", true)), vec![(0x135, true), (0x135, false)], "E0-Taste");
        // Loslassen eines Zeichens sendet nichts mehr.
        assert!(k.translate('A' as u32, 0, "a", false).is_empty());
    }

    #[test]
    fn modifiers_alone_send_nothing() {
        let mut k = de();
        // AltGr (Linux), Umschalt, Strg, Alt allein – gedrückt und losgelassen.
        for key in [0x0100_1103, 0x0100_0020, 0x0100_0021, 0x0100_0023, 0x4000_ffe1, 0x4000_ffe2, 0x4000_ffe3, 0x4000_ffe4, 0x4000_ffe9, 0x4000_ffea] {
            assert!(k.translate(key, 0, "", true).is_empty(), "{key:x}");
            assert!(k.translate(key, 0, "", false).is_empty(), "{key:x}");
        }
        // Windows: AltGr kommt als Strg+Alt – allein ebenfalls nichts.
        assert!(k.translate(0x0100_0021, CTRL | ALT, "", true).is_empty());
        assert!(k.translate(0x0100_0023, CTRL | ALT, "", true).is_empty());
    }

    #[test]
    fn altgr_on_windows_with_text_is_a_character() {
        let mut k = de();
        assert_eq!(scs(&k.translate('Q' as u32, CTRL | ALT, "@", true)), vec![(SC_RALT, true), (0x10, true), (0x10, false), (SC_RALT, false)]);
    }

    #[test]
    fn dead_keys_and_unknown_characters_stay_unicode() {
        let mut k = de();
        assert_eq!(k.translate(0, 0, "^", true), vec![InputEvent::Text("^".into())]);
        assert_eq!(k.translate(0, 0, "´", true), vec![InputEvent::Text("´".into())]);
        assert_eq!(k.translate(0, 0, "é", true), vec![InputEvent::Text("é".into())]);
        assert_eq!(k.translate(0, 0, "😀", true), vec![InputEvent::Text("😀".into())]);
    }

    #[test]
    fn shortcuts_are_complete_and_nothing_stays_pressed() {
        let mut k = de();
        // Strg+Z: Taste der Belegung (0x15), Strg vorher gedrückt, nach dem Loslassen gelöst.
        let down = k.translate('Z' as u32, CTRL, "\u{1a}", true);
        let up = k.translate('Z' as u32, CTRL, "\u{1a}", false);
        assert_eq!(scs(&[down, up].concat()), vec![(SC_LCTRL, true), (0x15, true), (0x15, false), (SC_LCTRL, false)]);
        // Umschalt+Pfeil (Markieren): Umschalt um die Sondertaste.
        let down = k.translate(0x0100_0014, SHIFT, "", true);
        assert_eq!(down, vec![InputEvent::Scancodes(vec![(SC_LSHIFT, true)]), InputEvent::Key { keysym: 0xff53, down: true }]);
        let up = k.translate(0x0100_0014, SHIFT, "", false);
        assert_eq!(up, vec![InputEvent::Key { keysym: 0xff53, down: false }, InputEvent::Scancodes(vec![(SC_LSHIFT, false)])]);
        // Gehaltene Taste (Wiederholung): Umschalter nur einmal gedrückt.
        let a = k.translate('C' as u32, CTRL, "\u{3}", true);
        let b = k.translate('C' as u32, CTRL, "\u{3}", true);
        assert_eq!(a.len(), 2);
        assert_eq!(b.len(), 1);
        // Modus aus mitten im Kürzel: alles lösen.
        let rest = k.release_all();
        assert_eq!(scs(&rest), vec![(0x2E, false), (SC_LCTRL, false)]);
        assert!(k.release_all().is_empty());
    }

    #[test]
    fn plain_keys_without_modifiers_pass_as_before() {
        let mut k = de();
        assert_eq!(k.translate(0x0100_0013, 0, "", true), vec![InputEvent::Key { keysym: 0xff52, down: true }]);
        assert_eq!(k.translate(0x0100_0013, 0, "", false), vec![InputEvent::Key { keysym: 0xff52, down: false }]);
    }

    #[test]
    fn windows_key_alone_opens_the_start_menu_once() {
        let mut k = de();
        assert!(k.translate(0x0100_0053, 0, "", true).is_empty());
        assert_eq!(scs(&k.translate(0x0100_0053, 0, "", false)), vec![(SC_LWIN, true), (SC_LWIN, false)]);
        // Win+E: kein zusätzliches Startmenü.
        assert!(k.translate(0x0100_0053, 0, "", true).is_empty());
        let e = k.translate('E' as u32, META, "e", true);
        assert_eq!(e.len(), 2);
        k.translate('E' as u32, META, "e", false);
        assert!(k.translate(0x0100_0053, 0, "", false).is_empty());
    }

    #[test]
    fn builtin_tables_cover_all_offered_layouts() {
        let all = builtin();
        for (klid, name) in NAMES {
            let l = Layout::find(all, klid).unwrap_or_else(|| panic!("Tabelle fehlt: {name}"));
            assert!(l.keys.len() > 90, "{name}: nur {} Zeichen", l.keys.len());
            // Tottasten-Zeichen gehen nie als Tastenfolge.
            for c in &l.dead {
                assert_eq!(l.sequence(*c), None, "{name}: Tottaste {c:?}");
            }
            // Keine Ziffernblock-Taste (hängt am Server von NumLock ab).
            for (c, (sc, _)) in &l.keys {
                assert!(!matches!(sc & 0xff, 0x37 | 0x47..=0x53) || *sc == 0x35, "{name}: {c:?} auf dem Ziffernblock (0x{sc:x})");
            }
        }
        // Deutsch: mitgelieferte Tabelle stimmt mit der Kontrolltabelle überein.
        let de = Layout::find(all, "00000407").unwrap();
        let control = parse_layouts(DE).unwrap().remove(0);
        for (c, v) in &control.keys {
            if *c != '/' {
                assert_eq!(de.keys.get(c), Some(v), "{c:?}");
            }
        }
        assert_eq!(de.keys.get(&'/'), Some(&(0x08, Level::Shift)), "/ über Umschalt+7, nicht Ziffernblock");
        assert_eq!(de.dead, control.dead);
    }

    #[test]
    fn layout_ids() {
        assert_eq!(klid_of("de").as_deref(), Some("00000407"));
        assert_eq!(klid_of("de-AT").as_deref(), Some("00000407"));
        assert_eq!(klid_of("0x0C07").as_deref(), Some("00000407"));
        assert_eq!(klid_of("de-CH").as_deref(), Some("00000807"));
        assert_eq!(klid_of("00000415").as_deref(), Some("00000415"));
        assert_eq!(klid_of(""), None);
        assert_eq!(klid_of("quatsch"), None);
    }
}
