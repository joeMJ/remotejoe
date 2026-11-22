//! Makros in einer offenen SSH-Sitzung ausführen.
//!
//! Die Makro-Engine (`rj_tasks::macros`) arbeitet blockierend. Sie läuft deshalb in einem
//! eigenen Faden: Die Ausgabe der Sitzung kommt über einen Abzweig (Tap) des Hauptprozesses
//! herein, Eingaben gehen über einen Kanal zurück an die Sitzung. Kennwörter aus den
//! Zugangsdaten der Sitzung stehen nur für `{cred:password}` bereit und nie im Protokoll.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rj_core::Secret;
use rj_tasks::macros::{CredentialSource, Macro, MacroIo};

use crate::workspace::SessionId;

/// Ergebnis eines Makrolaufs für die Anzeige.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub ok: bool,
    pub message: String,
}

/// Ein laufendes Makro.
pub struct RunningMacro {
    pub session: SessionId,
    pub name: String,
    pub cancel: Arc<AtomicBool>,
    pub done: Receiver<Outcome>,
}

/// Zugangsdaten der Sitzung für die Platzhalter `{cred:…}`.
#[derive(Clone)]
pub struct Creds {
    pub password: Option<Secret>,
    pub username: String,
    pub domain: Option<String>,
}

impl CredentialSource for Creds {
    fn field(&self, name: &str) -> Option<Secret> {
        match name {
            "password" => self.password.clone().filter(|p| !p.as_bytes().is_empty()),
            "username" => (!self.username.is_empty()).then(|| Secret::from_str_exact(&self.username)),
            "domain" => self.domain.as_deref().filter(|d| !d.is_empty()).map(Secret::from_str_exact),
            _ => None,
        }
    }
}

struct TapIo {
    session: SessionId,
    rx: Receiver<Vec<u8>>,
    out: Sender<(SessionId, Vec<u8>)>,
    cancel: Arc<AtomicBool>,
}

impl MacroIo for TapIo {
    fn send(&mut self, data: &[u8]) -> io::Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "abgebrochen"));
        }
        self.out.send((self.session, data.to_vec())).map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Sitzung beendet"))
    }

    fn recv(&mut self, timeout: Duration) -> io::Result<Option<Vec<u8>>> {
        let end = Instant::now() + timeout;
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "abgebrochen"));
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            match self.rx.recv_timeout(left.min(Duration::from_millis(100))) {
                Ok(b) => return Ok(Some(b)),
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return Err(io::Error::new(io::ErrorKind::BrokenPipe, "Verbindung beendet")),
            }
        }
    }
}

/// Startet das Makro in einem eigenen Faden.
pub fn spawn(session: SessionId, name: String, mac: Macro, creds: Creds, tap: Receiver<Vec<u8>>, input: Sender<(SessionId, Vec<u8>)>) -> RunningMacro {
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, done) = std::sync::mpsc::channel();
    let c = cancel.clone();
    let n = name.clone();
    std::thread::spawn(move || {
        let mut io = TapIo { session, rx: tap, out: input, cancel: c.clone() };
        let run = mac.run(&mut io, &creds);
        let outcome = match run.result {
            Ok(()) => Outcome { ok: true, message: format!("Makro „{n}“ abgeschlossen.") },
            Err(_) if c.load(Ordering::Relaxed) => Outcome { ok: false, message: format!("Makro „{n}“ abgebrochen.") },
            Err(e) => Outcome { ok: false, message: format!("Makro „{n}“ fehlgeschlagen: {e}") },
        };
        let _ = tx.send(outcome);
    });
    RunningMacro { session, name, cancel, done }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn macro_of(json: &str) -> Macro {
        serde_json::from_str(json).unwrap()
    }

    /// Simulierte Sitzung: antwortet auf eine Eingabe mit Ausgabe.
    fn fake_shell(rx_in: Receiver<(SessionId, Vec<u8>)>, tap_tx: Sender<Vec<u8>>, replies: Vec<(&'static str, &'static str)>) -> std::thread::JoinHandle<Vec<String>> {
        std::thread::spawn(move || {
            let mut seen = Vec::new();
            while let Ok((_, bytes)) = rx_in.recv_timeout(Duration::from_secs(3)) {
                let text = String::from_utf8_lossy(&bytes).to_string();
                seen.push(text.clone());
                for (when, out) in &replies {
                    if text.contains(when) {
                        let _ = tap_tx.send(out.as_bytes().to_vec());
                    }
                }
            }
            seen
        })
    }

    #[test]
    fn runs_wait_send_with_password_placeholder() {
        let (tap_tx, tap_rx) = mpsc::channel();
        let (in_tx, in_rx) = mpsc::channel();
        let shell = fake_shell(in_rx, tap_tx.clone(), vec![("sudo -v", "[sudo] Kennwort für root: "), ("geheim", "\r\nroot@host:~# ")]);
        let m = macro_of(r#"{"steps":[
            {"step":"send","text":"sudo -v{key:enter}"},
            {"step":"wait","pattern":{"text":"Kennwort für"},"timeout_ms":3000},
            {"step":"send","text":"{cred:password}{key:enter}"},
            {"step":"wait","pattern":{"text":"root@host:~#"},"timeout_ms":3000}]}"#);
        let creds = Creds { password: Some(Secret::from_str_exact("geheim")), username: "root".into(), domain: None };
        let run = spawn(7, "sudo".into(), m, creds, tap_rx, in_tx);
        let o = run.done.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(o.ok, "{}", o.message);
        assert!(!o.message.contains("geheim"));
        drop(tap_tx);
        let seen = shell.join().unwrap();
        assert_eq!(seen, vec!["sudo -v\r".to_string(), "geheim\r".to_string()]);
    }

    #[test]
    fn timeout_and_error_pattern_fail_with_reason() {
        let (tap_tx, tap_rx) = mpsc::channel::<Vec<u8>>();
        let (in_tx, _in_rx) = mpsc::channel();
        let m = macro_of(r#"{"steps":[{"step":"wait","pattern":{"text":"nie"},"timeout_ms":300}]}"#);
        let run = spawn(1, "warten".into(), m, Creds { password: None, username: String::new(), domain: None }, tap_rx, in_tx);
        let o = run.done.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!o.ok);
        assert!(o.message.contains("Zeitüberschreitung"), "{}", o.message);
        drop(tap_tx);
    }

    #[test]
    fn cancel_stops_a_waiting_macro_quickly() {
        let (_tap_tx, tap_rx) = mpsc::channel::<Vec<u8>>();
        let (in_tx, _in_rx) = mpsc::channel();
        let m = macro_of(r#"{"steps":[{"step":"wait","pattern":{"text":"nie"},"timeout_ms":60000}]}"#);
        let run = spawn(1, "lang".into(), m, Creds { password: None, username: String::new(), domain: None }, tap_rx, in_tx);
        std::thread::sleep(Duration::from_millis(200));
        let t = Instant::now();
        run.cancel.store(true, Ordering::Relaxed);
        let o = run.done.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(t.elapsed() < Duration::from_secs(2));
        assert!(!o.ok);
        assert!(o.message.contains("abgebrochen"), "{}", o.message);
    }
}
