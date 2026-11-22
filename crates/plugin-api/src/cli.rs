//! Gemeinsames Vokabular der Befehlszeilen-Sitzungen für Kern und Erweiterungen: Pseudo-Befehle (`#wait`, `#help`),
//! Erkennung der Eingabeaufforderung, Hilfelisten, Ergebnis und Fehler einer Sitzung, kleine Hilfen.

use std::sync::Mutex;

/// Pseudo-Befehl für eine Pause in [`run`]: „#wait 10“ wartet zehn Sekunden, ohne etwas zu senden.
pub const WAIT: &str = "#wait ";

/// Pseudo-Befehl für die CLI-Hilfe in [`run`]: „#help speed“ tippt `speed ?` ohne Zeilenende, liest die Liste bis
/// zur erneut angezeigten Eingabezeile (mit dem angefangenen Befehl), löscht die Zeile mit Rücktasten und schickt
/// erst dann Enter – der angefangene Befehl wird nie ausgeführt, auch wenn er vollständig ist. Danach Warten auf die
/// Eingabeaufforderung. Ausgabe: nur die Hilfezeilen.
pub const HELP: &str = "#help ";

/// Eingabezeile mit angefangenem Befehl: „SW(config-if)# speed“.
pub fn is_partial_prompt(line: &str, typed: &str) -> bool {
    let l = line.trim_end();
    let t = typed.trim();
    l.ends_with(t) && is_prompt(l[..l.len() - t.len()].trim_end())
}

/// Hilfeliste („  wert   Beschreibung“) in Wert und Beschreibung zerlegen; `<cr>` und Leerzeilen fallen weg.
pub fn parse_help(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|l| l.starts_with(' '))
        .filter_map(|l| {
            let t = l.trim();
            let (v, d) = t.split_once("  ").map(|(v, d)| (v.trim(), d.trim())).unwrap_or((t, ""));
            (!v.is_empty() && !v.eq_ignore_ascii_case("<cr>") && !v.contains(char::is_whitespace)).then(|| (v.to_string(), d.to_string()))
        })
        .collect()
}

/// Ergebnis einer Sitzung: Ausgaben je Befehl (bereinigt) und die gesamte Rohantwort (bereinigt).
#[derive(Debug, Default)]
pub struct Session {
    pub outputs: Vec<String>,
    pub text: String,
    /// Sekunden ab Sitzungsbeginn, zu denen die Ausgabe je Befehl vollständig war (gleiche Reihenfolge wie `outputs`).
    pub times: Vec<f32>,
    /// Die Zeitgrenze wurde erreicht, bevor alle Befehle beantwortet waren.
    pub timed_out: bool,
}

/// Fehler einer Sitzung.
#[derive(Debug, PartialEq, Eq)]
pub enum CliError {
    /// Anmeldung abgelehnt (Kennwort, Benutzer).
    Auth,
    Other(String),
}

/// Eingabeaufforderung eines Switches: Name ohne Leerzeichen mit `#` oder `>` am Ende (auch `name(config)#`).
pub fn is_prompt(line: &str) -> bool {
    let l = line.trim();
    (l.ends_with('#') || l.ends_with('>')) && l.len() > 1 && !l.contains(char::is_whitespace)
}

pub fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Rohantwort kürzen (für die Anzeige im Such-Tab; höchstens 4 KB, am Ende abgeschnitten).
pub fn raw_short(s: &str) -> String {
    raw_cut(s, 4096)
}

/// Rohantwort auf höchstens `max` Bytes kürzen (an einer Zeichengrenze).
pub fn raw_cut(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n… (gekürzt)", &s[..cut])
}

