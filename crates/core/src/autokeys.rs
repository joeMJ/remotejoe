//! Tastenfolge nach dem Verbinden (RDP und SSH).
//!
//! Beschreibung am Ziel: eine Schrittfolge aus Tasten, Text und Pausen, ein Auslöser (feste Wartezeit oder
//! „Bild bzw. Ausgabe wird ruhig“) und das Verhalten beim Neuverbinden. Die Schrittfolge steht als Text, eine Zeile
//! je Schritt:
//!
//! ```text
//! # Anmeldung am Ubuntu-Desktop
//! Taste Enter
//! Pause 500
//! Text {cred:password}
//! Taste Enter
//! ```
//!
//! `Taste` kennt Namen (`Enter`, `Tab`, `Esc`, `F5`, `Pfeil` heißt `Up`/`Down`/`Left`/`Right` …) und Kombinationen
//! (`Ctrl+Alt+T`, `Win+R`). `Text` tippt den Rest der Zeile; `{cred:password}`, `{cred:username}` und `{cred:domain}`
//! werden aus den Zugangsdaten des Ziels eingesetzt (nie in Protokollen), `{{` und `}}` sind geschweifte Klammern.

use serde::{Deserialize, Serialize};

use crate::Secret;

/// Höchstzahl der Schritte und Gesamtlänge der Pausen (Schutz vor Fehleingaben).
pub const MAX_STEPS: usize = 200;
pub const MAX_PAUSE_MS: u32 = 120_000;

/// Auslöser: wann nach dem Verbinden gesendet wird.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutoTrigger {
    /// Nach fester Wartezeit seit dem Verbinden.
    Delay,
    /// Sobald Bild (RDP) bzw. Ausgabe (SSH) für `wait_ms` ruhig ist, spätestens nach `max_ms`.
    #[default]
    Quiet,
}

/// Verhalten, wenn die Sitzung neu verbunden wurde.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoReconnect {
    /// Bei jedem Verbinden, auch nach automatischem Neuverbinden.
    #[default]
    Always,
    /// Nur beim ersten Verbinden der Sitzung.
    FirstOnly,
    /// Beim ersten Verbinden und nach manuellem Neuverbinden, nicht nach automatischem.
    ManualOnly,
}

/// Einstellung am Ziel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoKeys {
    pub enabled: bool,
    /// Schrittfolge als Text (siehe Modulbeschreibung).
    pub script: String,
    pub trigger: AutoTrigger,
    /// `Delay`: Wartezeit; `Quiet`: so lange muss Ruhe herrschen (Millisekunden).
    pub wait_ms: u32,
    /// `Quiet`: spätestens nach dieser Zeit seit dem Verbinden wird trotzdem gesendet (Millisekunden).
    pub max_ms: u32,
    pub on_reconnect: AutoReconnect,
}

impl Default for AutoKeys {
    fn default() -> Self {
        AutoKeys { enabled: false, script: String::new(), trigger: AutoTrigger::Quiet, wait_ms: 1000, max_ms: 10_000, on_reconnect: AutoReconnect::Always }
    }
}

/// Ein Schritt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyStep {
    Key(KeySpec),
    /// Text mit Platzhaltern (unaufgelöst).
    Text(String),
    Pause(u32),
}

pub const CTRL: u8 = 1;
pub const ALT: u8 = 2;
pub const SHIFT: u8 = 4;
pub const SUPER: u8 = 8;

/// Eine Taste mit Umschaltern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeySpec {
    pub mods: u8,
    pub key: Key,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Enter,
    Tab,
    Esc,
    Backspace,
    Delete,
    Insert,
    Space,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    /// F1 bis F12.
    F(u8),
    /// Ein einzelnes Zeichen (Buchstabe, Ziffer, Satzzeichen).
    Char(char),
    /// Nur die Windows-/Super-Taste.
    Super,
}

impl KeySpec {
    /// Parst `Enter`, `ctrl+alt+t`, `Win+R`, `F5` …
    pub fn parse(s: &str) -> Result<KeySpec, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("Taste fehlt".into());
        }
        // Ein einzelnes „+“ ist selbst eine Taste.
        let parts: Vec<&str> = if s == "+" { vec!["+"] } else { s.split('+').map(str::trim).collect() };
        let (last, mods_part) = parts.split_last().ok_or("Taste fehlt")?;
        let mut mods = 0u8;
        for m in mods_part {
            mods |= match m.to_lowercase().as_str() {
                "ctrl" | "strg" => CTRL,
                "alt" => ALT,
                "shift" | "umschalt" => SHIFT,
                "win" | "super" | "meta" => SUPER,
                other => return Err(format!("Unbekannter Umschalter „{other}“")),
            };
        }
        let l = last.to_lowercase();
        let key = match l.as_str() {
            "enter" | "return" | "eingabe" => Key::Enter,
            "tab" => Key::Tab,
            "esc" | "escape" => Key::Esc,
            "backspace" | "rueck" | "rück" => Key::Backspace,
            "delete" | "del" | "entf" => Key::Delete,
            "insert" | "einfg" => Key::Insert,
            "space" | "leer" | "leertaste" => Key::Space,
            "up" | "hoch" => Key::Up,
            "down" | "runter" => Key::Down,
            "left" | "links" => Key::Left,
            "right" | "rechts" => Key::Right,
            "home" | "pos1" => Key::Home,
            "end" | "ende" => Key::End,
            "pageup" | "pgup" | "bildhoch" => Key::PageUp,
            "pagedown" | "pgdn" | "bildrunter" => Key::PageDown,
            "win" | "super" | "meta" if mods == 0 => Key::Super,
            f if f.len() >= 2 && f.starts_with('f') && f[1..].chars().all(|c| c.is_ascii_digit()) => {
                let n: u8 = f[1..].parse().map_err(|_| format!("Unbekannte Taste „{last}“"))?;
                if !(1..=12).contains(&n) {
                    return Err(format!("Funktionstaste F{n} nicht unterstützt (F1 bis F12)"));
                }
                Key::F(n)
            }
            _ => {
                let mut cs = last.chars();
                match (cs.next(), cs.next()) {
                    (Some(c), None) if !c.is_control() => Key::Char(c),
                    _ => return Err(format!("Unbekannte Taste „{last}“")),
                }
            }
        };
        Ok(KeySpec { mods, key })
    }

    /// X11-Keysym der Taste (für RDP und VNC wie die übrige Tastaturweitergabe).
    pub fn keysym(&self) -> u32 {
        match self.key {
            Key::Enter => 0xff0d,
            Key::Tab => 0xff09,
            Key::Esc => 0xff1b,
            Key::Backspace => 0xff08,
            Key::Delete => 0xffff,
            Key::Insert => 0xff63,
            Key::Space => 0x20,
            Key::Up => 0xff52,
            Key::Down => 0xff54,
            Key::Left => 0xff51,
            Key::Right => 0xff53,
            Key::Home => 0xff50,
            Key::End => 0xff57,
            Key::PageUp => 0xff55,
            Key::PageDown => 0xff56,
            Key::F(n) => 0xffbd + n as u32,
            Key::Super => 0xffeb,
            Key::Char(c) => {
                // Mit gedrücktem Umschalter zählt der Kleinbuchstabe als Grundtaste.
                let c = if self.mods & (CTRL | ALT | SUPER) != 0 { c.to_ascii_lowercase() } else { c };
                c as u32
            }
        }
    }

    /// Keysyms der Umschalter in fester Reihenfolge (Ctrl, Alt, Shift, Super).
    pub fn modifier_keysyms(&self) -> Vec<u32> {
        let mut v = Vec::new();
        if self.mods & CTRL != 0 {
            v.push(0xffe3);
        }
        if self.mods & ALT != 0 {
            v.push(0xffe9);
        }
        if self.mods & SHIFT != 0 {
            v.push(0xffe1);
        }
        if self.mods & SUPER != 0 {
            v.push(0xffeb);
        }
        v
    }

    /// Bytes für ein Terminal (SSH); `None`, wenn die Taste dort nicht darstellbar ist.
    pub fn terminal_bytes(&self) -> Option<Vec<u8>> {
        let base: Vec<u8> = match self.key {
            Key::Enter => b"\r".to_vec(),
            Key::Tab => if self.mods & SHIFT != 0 { b"\x1b[Z".to_vec() } else { b"\t".to_vec() },
            Key::Esc => b"\x1b".to_vec(),
            Key::Backspace => b"\x7f".to_vec(),
            Key::Delete => b"\x1b[3~".to_vec(),
            Key::Insert => b"\x1b[2~".to_vec(),
            Key::Space => if self.mods & CTRL != 0 { vec![0] } else { b" ".to_vec() },
            Key::Up => b"\x1b[A".to_vec(),
            Key::Down => b"\x1b[B".to_vec(),
            Key::Right => b"\x1b[C".to_vec(),
            Key::Left => b"\x1b[D".to_vec(),
            Key::Home => b"\x1b[H".to_vec(),
            Key::End => b"\x1b[F".to_vec(),
            Key::PageUp => b"\x1b[5~".to_vec(),
            Key::PageDown => b"\x1b[6~".to_vec(),
            Key::F(n) => match n {
                1 => b"\x1bOP".to_vec(),
                2 => b"\x1bOQ".to_vec(),
                3 => b"\x1bOR".to_vec(),
                4 => b"\x1bOS".to_vec(),
                5 => b"\x1b[15~".to_vec(),
                6 => b"\x1b[17~".to_vec(),
                7 => b"\x1b[18~".to_vec(),
                8 => b"\x1b[19~".to_vec(),
                9 => b"\x1b[20~".to_vec(),
                10 => b"\x1b[21~".to_vec(),
                11 => b"\x1b[23~".to_vec(),
                _ => b"\x1b[24~".to_vec(),
            },
            Key::Super => return None,
            Key::Char(c) => {
                if self.mods & CTRL != 0 {
                    let l = c.to_ascii_lowercase();
                    match l {
                        'a'..='z' => vec![l as u8 - b'a' + 1],
                        '[' => vec![0x1b],
                        '\\' => vec![0x1c],
                        ']' => vec![0x1d],
                        _ => return None,
                    }
                } else {
                    c.to_string().into_bytes()
                }
            }
        };
        if self.mods & SUPER != 0 {
            return None;
        }
        // Alt: Escape davor (üblich bei Terminals).
        if self.mods & ALT != 0 {
            let mut v = vec![0x1b];
            v.extend(base);
            Some(v)
        } else {
            Some(base)
        }
    }
}

/// Zerlegt die Schrittfolge. Fehler nennen die Zeilennummer (ab 1).
pub fn parse_script(script: &str) -> Result<Vec<KeyStep>, String> {
    let mut steps = Vec::new();
    for (i, raw) in script.lines().enumerate() {
        let n = i + 1;
        let line = raw.trim_end_matches('\r');
        let t = line.trim_start();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let (word, rest) = match t.split_once(char::is_whitespace) {
            Some((w, r)) => (w, r),
            None => (t, ""),
        };
        let step = match word.to_lowercase().as_str() {
            "taste" | "key" => KeyStep::Key(KeySpec::parse(rest).map_err(|e| format!("Zeile {n}: {e}"))?),
            "text" => {
                // Genau ein Trennzeichen nach dem Wort gehört nicht zum Text.
                check_placeholders(rest).map_err(|e| format!("Zeile {n}: {e}"))?;
                KeyStep::Text(rest.to_string())
            }
            "pause" | "warten" => {
                let ms: u32 = rest.trim().trim_end_matches("ms").trim().parse().map_err(|_| format!("Zeile {n}: Pause braucht Millisekunden, z. B. „Pause 500“"))?;
                if ms > MAX_PAUSE_MS {
                    return Err(format!("Zeile {n}: Pause höchstens {MAX_PAUSE_MS} ms"));
                }
                KeyStep::Pause(ms)
            }
            w if w.starts_with('{') => return Err(format!("Zeile {n}: Es fehlt das Wort „Text“ davor, z. B. „Text {t}“")),
            other => return Err(format!("Zeile {n}: Unbekannter Schritt „{other}“ (erlaubt: Taste, Text, Pause)")),
        };
        steps.push(step);
        if steps.len() > MAX_STEPS {
            return Err(format!("Höchstens {MAX_STEPS} Schritte"));
        }
    }
    Ok(steps)
}

/// Prüft die Platzhalter eines Textes, ohne Werte einzusetzen.
fn check_placeholders(text: &str) -> Result<(), String> {
    expand_with(text, &mut |_| Some(Secret::from_str_exact(""))).map(|_| ())
}

/// Setzt `{cred:feld}` ein. Fehlt ein Feld oder ist ein Platzhalter ungültig, gibt es einen Fehler.
pub fn expand_text(text: &str, creds: &dyn Fn(&str) -> Option<Secret>) -> Result<Secret, String> {
    let mut f = |name: &str| creds(name);
    expand_with(text, &mut f)
}

fn expand_with(text: &str, creds: &mut dyn FnMut(&str) -> Option<Secret>) -> Result<Secret, String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find(['{', '}']) {
        out.extend_from_slice(rest[..pos].as_bytes());
        let tail = &rest[pos..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            out.push(tail.as_bytes()[0]);
            rest = &tail[2..];
            continue;
        }
        if tail.starts_with('}') {
            return Err("Einzelne „}“ (geschweifte Klammern doppelt schreiben: }})".into());
        }
        let end = tail.find('}').ok_or("„{“ ohne schließende Klammer (doppelt schreiben: {{)")?;
        let inner = &tail[1..end];
        let field = inner.strip_prefix("cred:").ok_or_else(|| format!("Platzhalter „{{{inner}}}“ unbekannt (erlaubt: {{cred:password}}, {{cred:username}}, {{cred:domain}})"))?;
        if !matches!(field, "password" | "username" | "domain") {
            return Err(format!("Zugangsdatenfeld „{field}“ unbekannt (erlaubt: password, username, domain)"));
        }
        let v = creds(field).ok_or_else(|| format!("Zugangsdatenfeld „{field}“ ist für dieses Ziel leer"))?;
        out.extend_from_slice(v.as_bytes());
        rest = &tail[end + 1..];
    }
    out.extend_from_slice(rest.as_bytes());
    Ok(Secret::from_bytes(out))
}

impl AutoKeys {
    /// Zerlegt und prüft die Schrittfolge (auch bei ausgeschalteter Einstellung, damit Fehler früh auffallen).
    pub fn steps(&self) -> Result<Vec<KeyStep>, String> {
        parse_script(&self.script)
    }

    /// Prüft die Einstellung; für SSH zusätzlich, dass jede Taste im Terminal darstellbar ist.
    pub fn validate(&self, ssh: bool) -> Result<(), String> {
        let steps = self.steps()?;
        if self.enabled && steps.is_empty() {
            return Err("Die Tastenfolge ist eingeschaltet, enthält aber keine Schritte".into());
        }
        if ssh {
            for (i, s) in steps.iter().enumerate() {
                if let KeyStep::Key(k) = s {
                    if k.terminal_bytes().is_none() {
                        return Err(format!("Schritt {}: diese Taste lässt sich im Terminal nicht senden", i + 1));
                    }
                }
            }
        }
        if self.wait_ms > MAX_PAUSE_MS || self.max_ms > 600_000 {
            return Err("Wartezeiten zu lang (Wartezeit höchstens 120 s, spätestens-Zeit höchstens 600 s)".into());
        }
        Ok(())
    }

    /// Soll beim `n`-ten Verbinden der Sitzung (1 = erstes) gesendet werden?
    /// `automatic`: das Neuverbinden kam vom automatischen Neuverbinden.
    pub fn applies(&self, n: u32, automatic: bool) -> bool {
        if !self.enabled {
            return false;
        }
        match self.on_reconnect {
            AutoReconnect::Always => true,
            AutoReconnect::FirstOnly => n <= 1,
            AutoReconnect::ManualOnly => n <= 1 || !automatic,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_with_comments_and_all_steps() {
        let s = parse_script("# Anmeldung\nTaste Enter\n\nPause 500\nText {cred:password}\nkey ctrl+alt+t\n").unwrap();
        assert_eq!(s.len(), 4);
        assert_eq!(s[0], KeyStep::Key(KeySpec { mods: 0, key: Key::Enter }));
        assert_eq!(s[1], KeyStep::Pause(500));
        assert_eq!(s[2], KeyStep::Text("{cred:password}".into()));
        assert_eq!(s[3], KeyStep::Key(KeySpec { mods: CTRL | ALT, key: Key::Char('t') }));
    }

    #[test]
    fn errors_name_the_line() {
        assert!(parse_script("Taste Enter\nTaste Blubb").unwrap_err().starts_with("Zeile 2:"));
        assert!(parse_script("Pause lang").unwrap_err().contains("Millisekunden"));
        assert!(parse_script("Pause 999999999").unwrap_err().contains("höchstens"));
        assert!(parse_script("Tanzen").unwrap_err().contains("Unbekannter Schritt"));
        assert!(parse_script("Taste Enter\n{cred:password}").unwrap_err().contains("Es fehlt das Wort „Text“"));
        assert!(parse_script("Text {cred:token}").unwrap_err().contains("unbekannt"));
        assert!(parse_script("Text {unsinn}").unwrap_err().contains("unbekannt"));
        assert!(parse_script("Text {offen").unwrap_err().contains("schließende"));
        assert!(parse_script("Taste F13").is_err());
        assert!(parse_script("Taste Hyper+A").is_err());
    }

    #[test]
    fn placeholders_expand_and_braces_escape() {
        let creds = |f: &str| match f {
            "password" => Some(Secret::from_str_exact("pä{ss}")),
            "username" => Some(Secret::from_str_exact("anna")),
            _ => None,
        };
        let r = expand_text("{cred:username}:{cred:password} {{ok}}", &creds).unwrap();
        assert_eq!(r.as_bytes(), "anna:pä{ss} {ok}".as_bytes());
        assert!(expand_text("{cred:domain}", &creds).unwrap_err().contains("leer"));
    }

    #[test]
    fn keysyms_and_terminal_bytes() {
        let k = |s| KeySpec::parse(s).unwrap();
        assert_eq!(k("Enter").keysym(), 0xff0d);
        assert_eq!(k("F5").keysym(), 0xffc2);
        assert_eq!(k("Ctrl+C").keysym(), 'c' as u32);
        assert_eq!(k("Ctrl+Alt+Delete").modifier_keysyms(), vec![0xffe3, 0xffe9]);
        assert_eq!(k("Enter").terminal_bytes().unwrap(), b"\r");
        assert_eq!(k("Ctrl+C").terminal_bytes().unwrap(), vec![3]);
        assert_eq!(k("Alt+x").terminal_bytes().unwrap(), b"\x1bx");
        assert_eq!(k("Up").terminal_bytes().unwrap(), b"\x1b[A");
        assert_eq!(k("F5").terminal_bytes().unwrap(), b"\x1b[15~");
        assert!(k("Win+R").terminal_bytes().is_none());
        assert!(k("Win").terminal_bytes().is_none());
        assert_eq!(KeySpec::parse("+").unwrap().key, Key::Char('+'));
    }

    #[test]
    fn validation_and_reconnect_policy() {
        let mut a = AutoKeys { enabled: true, script: "Taste Enter\nText {cred:password}\nTaste Enter".into(), ..Default::default() };
        assert!(a.validate(false).is_ok() && a.validate(true).is_ok());
        a.script = "Taste Win+R".into();
        assert!(a.validate(false).is_ok());
        assert!(a.validate(true).unwrap_err().contains("Terminal"));
        a.script.clear();
        assert!(a.validate(false).unwrap_err().contains("keine Schritte"));
        // Verhalten beim Neuverbinden.
        let mut a = AutoKeys { enabled: true, script: "Taste Enter".into(), ..Default::default() };
        assert!(a.applies(1, false) && a.applies(2, true));
        a.on_reconnect = AutoReconnect::FirstOnly;
        assert!(a.applies(1, false) && !a.applies(2, false) && !a.applies(2, true));
        a.on_reconnect = AutoReconnect::ManualOnly;
        assert!(a.applies(1, false) && a.applies(2, false) && !a.applies(2, true));
        a.enabled = false;
        assert!(!a.applies(1, false));
    }

    #[test]
    fn options_roundtrip_old_files_and_defaults() {
        let a: AutoKeys = serde_json::from_str(r#"{"enabled":true,"script":"Taste Enter"}"#).unwrap();
        assert_eq!((a.trigger, a.wait_ms, a.max_ms, a.on_reconnect), (AutoTrigger::Quiet, 1000, 10_000, AutoReconnect::Always));
        let j = serde_json::to_string(&AutoKeys { trigger: AutoTrigger::Delay, on_reconnect: AutoReconnect::ManualOnly, ..a }).unwrap();
        assert!(j.contains("\"delay\"") && j.contains("\"manual_only\""), "{j}");
    }
}
