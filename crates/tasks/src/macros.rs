//! Makro-Engine für textbasierte Sitzungen (SSH, PowerShell).
//!
//! Schritte:
//! - `wait`: auf Text oder regulären Ausdruck warten; Fehlermuster brechen ab.
//! - `send`: Text senden; `{cred:password}`, `{cred:username}` und
//!   `{key:enter}` usw. werden ersetzt. Kennwörter werden nur als Verweis
//!   angegeben und erscheinen nie im Protokoll.
//! - `expect_any`: auf eines von mehreren Mustern warten und den passenden
//!   Zweig ausführen.
//! - `on_error`: Schritte, die nach einem Fehlschlag ausgeführt werden.

use std::io;
use std::time::{Duration, Instant};

use regex::Regex;
use rj_core::Secret;
use serde::{Deserialize, Serialize};

/// Muster für die Ausgabe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pattern {
    Text(String),
    Regex(String),
}

impl Pattern {
    fn compile(&self) -> Result<Matcher, MacroError> {
        Ok(match self {
            Pattern::Text(t) => Matcher::Text(t.clone()),
            Pattern::Regex(r) => Matcher::Regex(Regex::new(r).map_err(|e| MacroError::BadPattern(e.to_string()))?),
        })
    }

    fn describe(&self) -> String {
        match self {
            Pattern::Text(t) => format!("„{t}“"),
            Pattern::Regex(r) => format!("/{r}/"),
        }
    }
}

enum Matcher {
    Text(String),
    Regex(Regex),
}

impl Matcher {
    /// Position hinter dem Treffer.
    fn find(&self, hay: &str) -> Option<usize> {
        match self {
            Matcher::Text(t) => hay.find(t.as_str()).map(|p| p + t.len()),
            Matcher::Regex(r) => r.find(hay).map(|m| m.end()),
        }
    }
}

/// Zweig von `expect_any`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Branch {
    pub pattern: Pattern,
    #[serde(default)]
    pub steps: Vec<Step>,
    /// Treffer dieses Zweigs gilt als Fehlschlag.
    #[serde(default)]
    pub fail: bool,
}

/// Ein Schritt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum Step {
    Wait {
        pattern: Pattern,
        #[serde(default)]
        errors: Vec<Pattern>,
        #[serde(default = "default_timeout")]
        timeout_ms: u64,
    },
    Send {
        text: String,
    },
    ExpectAny {
        branches: Vec<Branch>,
        #[serde(default = "default_timeout")]
        timeout_ms: u64,
    },
}

fn default_timeout() -> u64 {
    15_000
}

/// Ein Makro.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Macro {
    pub steps: Vec<Step>,
    #[serde(default)]
    pub on_error: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MacroError {
    #[error("Zeitüberschreitung beim Warten auf {0}")]
    Timeout(String),
    #[error("Fehlermuster {0} erkannt")]
    ErrorPattern(String),
    #[error("Zweig {0} gilt als Fehlschlag")]
    FailBranch(String),
    #[error("Muster ungültig: {0}")]
    BadPattern(String),
    #[error("Platzhalter ungültig: {0}")]
    BadPlaceholder(String),
    #[error("Zugangsdatenfeld {0} nicht verfügbar")]
    MissingCredential(String),
    #[error("Verbindung: {0}")]
    Io(String),
}

/// Sitzung, auf der ein Makro läuft.
pub trait MacroIo {
    fn send(&mut self, data: &[u8]) -> io::Result<()>;
    /// Liefert neue Ausgabe oder `None` nach Ablauf von `timeout`.
    /// `Err` bei beendeter Verbindung.
    fn recv(&mut self, timeout: Duration) -> io::Result<Option<Vec<u8>>>;
}

/// Quelle für Zugangsdatenfelder (`password`, `username`, `domain`, `otp`).
pub trait CredentialSource {
    fn field(&self, name: &str) -> Option<Secret>;
}

impl<F: Fn(&str) -> Option<Secret>> CredentialSource for F {
    fn field(&self, name: &str) -> Option<Secret> {
        self(name)
    }
}

/// Ergebnis eines Makrolaufs mit Protokoll (ohne Geheimnisse).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroRun {
    pub result: Result<(), MacroError>,
    pub log: Vec<String>,
}

fn key(name: &str) -> Option<&'static [u8]> {
    Some(match name {
        "enter" => b"\r",
        "tab" => b"\t",
        "esc" => b"\x1b",
        "backspace" => b"\x7f",
        "ctrl-c" => b"\x03",
        "ctrl-d" => b"\x04",
        "ctrl-z" => b"\x1a",
        "up" => b"\x1b[A",
        "down" => b"\x1b[B",
        "right" => b"\x1b[C",
        "left" => b"\x1b[D",
        _ => return None,
    })
}

/// Setzt Platzhalter ein. Liefert die Bytes (als [`Secret`], weil
/// Kennwörter enthalten sein können) und die Protokollfassung.
pub fn expand(text: &str, creds: &dyn CredentialSource) -> Result<(Secret, String), MacroError> {
    let mut out = Vec::new();
    let mut log = String::new();
    let mut rest = text;
    while let Some(pos) = rest.find(['{', '}']) {
        out.extend_from_slice(rest[..pos].as_bytes());
        log.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            out.push(tail.as_bytes()[0]);
            log.push(tail.as_bytes()[0] as char);
            rest = &tail[2..];
            continue;
        }
        if tail.starts_with('}') {
            return Err(MacroError::BadPlaceholder(text.chars().take(40).collect()));
        }
        let end = tail.find('}').ok_or_else(|| MacroError::BadPlaceholder(tail.chars().take(40).collect()))?;
        let inner = &tail[1..end];
        if let Some(field) = inner.strip_prefix("cred:") {
            let v = creds.field(field).ok_or_else(|| MacroError::MissingCredential(field.to_string()))?;
            out.extend_from_slice(v.as_bytes());
        } else if let Some(k) = inner.strip_prefix("key:").and_then(key) {
            out.extend_from_slice(k);
        } else {
            return Err(MacroError::BadPlaceholder(inner.to_string()));
        }
        log.push_str(&tail[..=end]);
        rest = &tail[end + 1..];
    }
    out.extend_from_slice(rest.as_bytes());
    log.push_str(rest);
    Ok((Secret::from_bytes(out), log))
}

struct Runner<'a> {
    io: &'a mut dyn MacroIo,
    creds: &'a dyn CredentialSource,
    /// Sichtbarer Text seit dem letzten Treffer.
    buffer: String,
    /// Unvollständige UTF-8-Folge am Ende der letzten Ausgabe.
    pending: Vec<u8>,
    log: Vec<String>,
}

impl Runner<'_> {
    fn feed(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
        let valid = match std::str::from_utf8(&self.pending) {
            Ok(s) => s.len(),
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            // Ungültige Folge mitten im Strom: sichtbar markieren, nicht verschlucken.
            Err(e) => {
                let upto = e.valid_up_to();
                let bad = e.error_len().unwrap_or(1);
                let text = String::from_utf8_lossy(&self.pending[..upto]).into_owned();
                self.buffer.push_str(&crate::ansi::strip(&text));
                for b in &self.pending[upto..upto + bad] {
                    self.buffer.push_str(&format!("⟦0x{b:02X}⟧"));
                }
                self.pending.drain(..upto + bad);
                return self.feed(&[]);
            }
        };
        let text = std::str::from_utf8(&self.pending[..valid]).expect("geprüft").to_string();
        self.pending.drain(..valid);
        self.buffer.push_str(&crate::ansi::strip(&text));
    }

    /// Wartet, bis einer der Matcher passt; liefert dessen Nummer.
    fn wait_any(&mut self, matchers: &[Matcher], timeout: Duration) -> Result<Option<usize>, MacroError> {
        let deadline = Instant::now() + timeout;
        loop {
            let hit = matchers
                .iter()
                .enumerate()
                .filter_map(|(i, m)| m.find(&self.buffer).map(|end| (end, i)))
                .min();
            if let Some((end, i)) = hit {
                self.buffer.drain(..end);
                return Ok(Some(i));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            match self.io.recv(left.min(Duration::from_millis(200))) {
                Ok(Some(b)) => self.feed(&b),
                Ok(None) => {}
                Err(e) => return Err(MacroError::Io(e.to_string())),
            }
        }
    }

    fn run(&mut self, steps: &[Step]) -> Result<(), MacroError> {
        for step in steps {
            match step {
                Step::Send { text } => {
                    let (bytes, logged) = expand(text, self.creds)?;
                    self.log.push(format!("sende {logged:?}"));
                    self.io.send(bytes.as_bytes()).map_err(|e| MacroError::Io(e.to_string()))?;
                }
                Step::Wait { pattern, errors, timeout_ms } => {
                    let mut ms = vec![pattern.compile()?];
                    for e in errors {
                        ms.push(e.compile()?);
                    }
                    self.log.push(format!("warte auf {}", pattern.describe()));
                    match self.wait_any(&ms, Duration::from_millis(*timeout_ms))? {
                        Some(0) => self.log.push(format!("gefunden: {}", pattern.describe())),
                        Some(i) => return Err(MacroError::ErrorPattern(errors[i - 1].describe())),
                        None => return Err(MacroError::Timeout(pattern.describe())),
                    }
                }
                Step::ExpectAny { branches, timeout_ms } => {
                    let ms = branches.iter().map(|b| b.pattern.compile()).collect::<Result<Vec<_>, _>>()?;
                    let names: Vec<_> = branches.iter().map(|b| b.pattern.describe()).collect();
                    self.log.push(format!("erwarte eines von {}", names.join(", ")));
                    match self.wait_any(&ms, Duration::from_millis(*timeout_ms))? {
                        Some(i) => {
                            self.log.push(format!("gefunden: {}", names[i]));
                            if branches[i].fail {
                                self.run(&branches[i].steps)?;
                                return Err(MacroError::FailBranch(names[i].clone()));
                            }
                            self.run(&branches[i].steps)?;
                        }
                        None => return Err(MacroError::Timeout(names.join(", "))),
                    }
                }
            }
        }
        Ok(())
    }
}

impl Macro {
    /// Führt das Makro aus. Bei einem Fehlschlag laufen die `on_error`-Schritte.
    pub fn run(&self, io: &mut dyn MacroIo, creds: &dyn CredentialSource) -> MacroRun {
        let mut r = Runner { io, creds, buffer: String::new(), pending: Vec::new(), log: Vec::new() };
        let result = r.run(&self.steps);
        if let Err(e) = &result {
            r.log.push(format!("Fehler: {e}"));
            if !self.on_error.is_empty() {
                r.log.push("Fehlerbehandlung".into());
                if let Err(e2) = r.run(&self.on_error) {
                    r.log.push(format!("Fehlerbehandlung fehlgeschlagen: {e2}"));
                }
            }
        }
        MacroRun { result, log: r.log }
    }

    /// Eingebautes Makro: `sudo -v` ausführen, Kennwort-Abfrage beantworten,
    /// Erfolg an einer Markierung erkennen. Die Markierungen sind im Befehl
    /// zerteilt (`'RJ-SUDO-''OK'`), damit das Echo der Befehlszeile nicht passt.
    pub fn sudo_validate() -> Macro {
        serde_json::from_value(serde_json::json!({
            "steps": [
                { "step": "send", "text": "sudo -k; LC_ALL=C sudo -v -p 'RJ-SUDO-''PROMPT: ' && echo RJ-SUDO-''OK || echo RJ-SUDO-''FAIL{key:enter}" },
                { "step": "expect_any", "timeout_ms": 15000, "branches": [
                    { "pattern": { "text": "RJ-SUDO-PROMPT: " }, "steps": [
                        { "step": "send", "text": "{cred:password}{key:enter}" },
                        { "step": "expect_any", "timeout_ms": 15000, "branches": [
                            { "pattern": { "regex": "(?m)^RJ-SUDO-OK" } },
                            { "pattern": { "text": "RJ-SUDO-PROMPT: " }, "fail": true,
                              "steps": [ { "step": "send", "text": "{key:ctrl-c}" } ] },
                            { "pattern": { "regex": "(?m)^RJ-SUDO-FAIL" }, "fail": true }
                        ]}
                    ]},
                    { "pattern": { "regex": "(?m)^RJ-SUDO-OK" } },
                    { "pattern": { "regex": "(?m)^RJ-SUDO-FAIL" }, "fail": true }
                ]}
            ],
            "on_error": [ { "step": "send", "text": "{key:ctrl-c}" } ]
        }))
        .expect("eingebautes Makro ist gültig")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Attrappe: antwortet auf gesendete Zeilen nach Drehbuch.
    struct Script {
        sent: Vec<u8>,
        replies: VecDeque<(&'static str, &'static [u8])>,
        out: VecDeque<Vec<u8>>,
    }

    impl MacroIo for Script {
        fn send(&mut self, data: &[u8]) -> io::Result<()> {
            self.sent.extend_from_slice(data);
            while let Some((trigger, reply)) = self.replies.front() {
                if String::from_utf8_lossy(&self.sent).contains(trigger) {
                    self.out.push_back(reply.to_vec());
                    self.replies.pop_front();
                } else {
                    break;
                }
            }
            Ok(())
        }
        fn recv(&mut self, _t: Duration) -> io::Result<Option<Vec<u8>>> {
            Ok(self.out.pop_front())
        }
    }

    fn creds(name: &str) -> Option<Secret> {
        match name {
            "password" => Some(Secret::from_str_exact("Geheim-ä🚀")),
            "username" => Some(Secret::from_str_exact("admin")),
            _ => None,
        }
    }

    #[test]
    fn expand_placeholders_and_redacts() {
        let (bytes, log) = expand("pw={cred:password}{key:enter}{{x}}", &creds).unwrap();
        assert_eq!(bytes.as_bytes(), "pw=Geheim-ä🚀\r{x}".as_bytes());
        assert_eq!(log, "pw={cred:password}{key:enter}{x}");
        assert!(matches!(expand("{cred:otp}", &creds), Err(MacroError::MissingCredential(_))));
        assert!(matches!(expand("{oops}", &creds), Err(MacroError::BadPlaceholder(_))));
    }

    #[test]
    fn sudo_success() {
        let mut io = Script {
            sent: Vec::new(),
            replies: VecDeque::from([
                ("sudo -v", &b"\x1b[0mRJ-SUDO-PROMPT: "[..]),
                ("Geheim-ä🚀\r", &b"\r\nRJ-SUDO-OK\r\n$ "[..]),
            ]),
            out: VecDeque::new(),
        };
        let run = Macro::sudo_validate().run(&mut io, &creds);
        assert_eq!(run.result, Ok(()));
        let log = run.log.join("\n");
        assert!(!log.contains("Geheim"), "Kennwort nie im Protokoll: {log}");
        assert!(log.contains("{cred:password}"));
    }

    #[test]
    fn sudo_wrong_password_runs_on_error() {
        let mut io = Script {
            sent: Vec::new(),
            replies: VecDeque::from([
                ("sudo -v", &b"RJ-SUDO-PROMPT: "[..]),
                ("\r", &b"\r\nSorry, try again.\r\nRJ-SUDO-PROMPT: "[..]),
            ]),
            out: VecDeque::new(),
        };
        // Der erste Auslöser "\r" ist schon im Befehl enthalten; Drehbuch
        // liefert deshalb beide Antworten nacheinander.
        let run = Macro::sudo_validate().run(&mut io, &creds);
        assert!(matches!(run.result, Err(MacroError::FailBranch(_))), "{:?}", run.result);
        assert!(io.sent.ends_with(b"\x03"), "Fehlerbehandlung sendet Strg+C");
    }

    #[test]
    fn wait_timeout_and_error_pattern() {
        let m: Macro = serde_json::from_value(serde_json::json!({
            "steps": [ { "step": "wait", "pattern": { "text": "fertig" }, "errors": [ { "regex": "E: .*" } ], "timeout_ms": 300 } ]
        }))
        .unwrap();
        let mut io = Script { sent: vec![], replies: VecDeque::new(), out: VecDeque::from([b"E: kaputt".to_vec()]) };
        assert!(matches!(m.run(&mut io, &creds).result, Err(MacroError::ErrorPattern(_))));
        let mut io = Script { sent: vec![], replies: VecDeque::new(), out: VecDeque::new() };
        assert!(matches!(m.run(&mut io, &creds).result, Err(MacroError::Timeout(_))));
    }

    #[test]
    fn split_utf8_across_reads() {
        let m: Macro = serde_json::from_value(serde_json::json!({
            "steps": [ { "step": "wait", "pattern": { "text": "Grüße🚀" }, "timeout_ms": 500 } ]
        }))
        .unwrap();
        let bytes = "Grüße🚀".as_bytes();
        let mut io = Script {
            sent: vec![],
            replies: VecDeque::new(),
            out: VecDeque::from([bytes[..3].to_vec(), bytes[3..9].to_vec(), bytes[9..].to_vec()]),
        };
        assert_eq!(m.run(&mut io, &creds).result, Ok(()));
    }
}
