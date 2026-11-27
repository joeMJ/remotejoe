//! Telnet-Terminal: TCP-Verbindung mit Optionsverhandlung (RFC 854/855), Terminaltyp (RFC 1091) und Fenstergröße
//! (RFC 1073). Die Daten gehen ohne Verhandlungsfolgen ans Terminal. Automatische Anmeldung: Auf eine Aufforderung
//! für den Benutzer („login:“, „Username:“, „User Name:“) geht der Benutzer, auf „Password:“ das Kennwort – je
//! höchstens einmal, danach ist es ein normales Terminal. Telnet ist unverschlüsselt.

use std::time::Duration;

use rj_core::Secret;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::client::{Shell, ShellEvent, StreamCmd};

const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;
const ECHO: u8 = 1;
const SGA: u8 = 3;
const TTYPE: u8 = 24;
const NAWS: u8 = 31;

/// Zustand der Optionsverhandlung; Folgen dürfen über mehrere Lesevorgänge verteilt ankommen.
#[derive(Debug)]
pub struct Negotiator {
    term: String,
    cols: u16,
    rows: u16,
    /// Optionen, die das Gerät bei sich eingeschaltet hat (WILL angenommen).
    remote: [bool; 256],
    /// Optionen, die wir eingeschaltet haben (DO beantwortet mit WILL).
    local: [bool; 256],
    /// Abgelehnte Optionen (damit Wiederholungen keine Schleife auslösen).
    refused_remote: [bool; 256],
    refused_local: [bool; 256],
    /// Unvollständige Folge vom Ende des letzten Lesevorgangs.
    pending: Vec<u8>,
}

impl Negotiator {
    pub fn new(term: &str, cols: u32, rows: u32) -> Self {
        Negotiator {
            term: term.to_ascii_uppercase(),
            cols: cols.min(u16::MAX as u32) as u16,
            rows: rows.min(u16::MAX as u32) as u16,
            remote: [false; 256],
            local: [false; 256],
            refused_remote: [false; 256],
            refused_local: [false; 256],
            pending: Vec::new(),
        }
    }

    /// Fenstergröße als Unterverhandlung (IAC im Wert verdoppelt).
    fn naws(&self) -> Vec<u8> {
        let mut v = vec![IAC, SB, NAWS];
        for b in [self.cols.to_be_bytes(), self.rows.to_be_bytes()].concat() {
            v.push(b);
            if b == IAC {
                v.push(IAC);
            }
        }
        v.extend_from_slice(&[IAC, SE]);
        v
    }

    /// Neue Größe; liefert die Folge an das Gerät, wenn es die Fenstergröße angefordert hat.
    pub fn resize(&mut self, cols: u32, rows: u32) -> Vec<u8> {
        self.cols = cols.min(u16::MAX as u32) as u16;
        self.rows = rows.min(u16::MAX as u32) as u16;
        if self.local[NAWS as usize] {
            self.naws()
        } else {
            Vec::new()
        }
    }

    /// Daten vom Gerät: liefert (Daten fürs Terminal, Antwort ans Gerät).
    pub fn feed(&mut self, input: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let mut data = std::mem::take(&mut self.pending);
        data.extend_from_slice(input);
        let mut out = Vec::with_capacity(data.len());
        let mut reply = Vec::new();
        let mut i = 0;
        while i < data.len() {
            if data[i] != IAC {
                out.push(data[i]);
                i += 1;
                continue;
            }
            let Some(&cmd) = data.get(i + 1) else {
                self.pending = data[i..].to_vec();
                break;
            };
            match cmd {
                IAC => {
                    out.push(IAC);
                    i += 2;
                }
                DO | DONT | WILL | WONT => {
                    let Some(&opt) = data.get(i + 2) else {
                        self.pending = data[i..].to_vec();
                        break;
                    };
                    self.option(cmd, opt, &mut reply);
                    i += 3;
                }
                SB => {
                    // Unterverhandlung bis IAC SE; fehlt das Ende noch, beim nächsten Mal weiter.
                    let mut j = i + 2;
                    let mut end = None;
                    while j + 1 < data.len() {
                        if data[j] == IAC && data[j + 1] == SE {
                            end = Some(j);
                            break;
                        }
                        j += if data[j] == IAC { 2 } else { 1 };
                    }
                    let Some(end) = end else {
                        self.pending = data[i..].to_vec();
                        break;
                    };
                    let body = &data[i + 2..end];
                    // Terminaltyp angefordert (SEND = 1): mit IS (0) und dem Namen antworten.
                    if body.first() == Some(&TTYPE) && body.get(1) == Some(&1) {
                        reply.extend_from_slice(&[IAC, SB, TTYPE, 0]);
                        reply.extend_from_slice(self.term.as_bytes());
                        reply.extend_from_slice(&[IAC, SE]);
                    }
                    i = end + 2;
                }
                _ => i += 2,
            }
        }
        (out, reply)
    }

    fn option(&mut self, cmd: u8, opt: u8, reply: &mut Vec<u8>) {
        let o = opt as usize;
        match cmd {
            // Das Gerät will etwas bei sich einschalten: Echo und „Suppress Go Ahead“ annehmen, sonst ablehnen.
            WILL => {
                if opt == ECHO || opt == SGA {
                    if !self.remote[o] {
                        self.remote[o] = true;
                        reply.extend_from_slice(&[IAC, DO, opt]);
                    }
                } else if !self.refused_remote[o] {
                    self.refused_remote[o] = true;
                    reply.extend_from_slice(&[IAC, DONT, opt]);
                }
            }
            WONT => {
                if self.remote[o] {
                    self.remote[o] = false;
                    reply.extend_from_slice(&[IAC, DONT, opt]);
                }
            }
            // Das Gerät bittet uns um etwas: Terminaltyp, Fenstergröße und „Suppress Go Ahead“ ja, sonst nein.
            DO => {
                if opt == TTYPE || opt == NAWS || opt == SGA {
                    if !self.local[o] {
                        self.local[o] = true;
                        reply.extend_from_slice(&[IAC, WILL, opt]);
                    }
                    if opt == NAWS {
                        reply.extend_from_slice(&self.naws());
                    }
                } else if !self.refused_local[o] {
                    self.refused_local[o] = true;
                    reply.extend_from_slice(&[IAC, WONT, opt]);
                }
            }
            DONT => {
                if self.local[o] {
                    self.local[o] = false;
                    reply.extend_from_slice(&[IAC, WONT, opt]);
                }
            }
            _ => {}
        }
    }
}

/// Daten ans Gerät: das Byte 255 verdoppeln (sonst hielte das Gerät es für eine Verhandlungsfolge).
pub fn escape(data: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(data.len());
    for &b in data {
        v.push(b);
        if b == IAC {
            v.push(IAC);
        }
    }
    v
}

/// Automatische Anmeldung: erkennt die Aufforderungen am Ende der Ausgabe.
#[derive(Debug, Default)]
pub struct AutoLogin {
    tail: String,
    user_sent: bool,
    pass_sent: bool,
}

/// Was die automatische Anmeldung senden will.
#[derive(Debug, PartialEq, Eq)]
pub enum LoginStep {
    User,
    Password,
}

impl AutoLogin {
    /// Fertig: beides gesendet (oder nichts mehr zu tun).
    pub fn done(&self) -> bool {
        self.user_sent && self.pass_sent
    }

    /// Neue Ausgabe; liefert, was jetzt gesendet werden soll (höchstens je einmal Benutzer und Kennwort).
    pub fn feed(&mut self, data: &[u8], have_user: bool, have_password: bool) -> Option<LoginStep> {
        if self.done() {
            return None;
        }
        self.tail.push_str(&String::from_utf8_lossy(data).to_lowercase());
        if self.tail.len() > 400 {
            let cut = self.tail.len() - 200;
            let cut = (cut..self.tail.len()).find(|i| self.tail.is_char_boundary(*i)).unwrap_or(cut);
            self.tail.drain(..cut);
        }
        let end = self.tail.trim_end();
        let user_prompt = ["login:", "username:", "user name:"].iter().any(|p| end.ends_with(p));
        if user_prompt && !self.user_sent {
            self.user_sent = true;
            self.tail.clear();
            return have_user.then_some(LoginStep::User);
        }
        if end.ends_with("password:") && !self.pass_sent {
            self.pass_sent = true;
            // Ohne Benutzeraufforderung (nur Kennwort, z. B. einfache Geräte) gilt der Benutzer als erledigt.
            self.user_sent = true;
            self.tail.clear();
            return have_password.then_some(LoginStep::Password);
        }
        None
    }
}

/// Telnet-Sitzung öffnen. `user`/`password` leer: Anmeldung von Hand.
pub async fn open(host: &str, port: u16, timeout: Duration, user: String, password: Secret, term: &str, cols: u32, rows: u32) -> Result<Shell, String> {
    let stream = tokio::time::timeout(timeout, tokio::net::TcpStream::connect((host, port)))
        .await
        .map_err(|_| "Zeitüberschreitung beim Verbindungsaufbau".to_string())?
        .map_err(|e| format!("Telnet {host}:{port}: {e}"))?;
    let _ = stream.set_nodelay(true);
    let (mut rd, mut wr) = stream.into_split();
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<StreamCmd>();
    let (ev_tx, ev_rx) = mpsc::unbounded_channel::<ShellEvent>();
    // Antworten der Verhandlung laufen über denselben Schreibweg wie die Eingaben (Reihenfolge bleibt erhalten).
    let (raw_tx, mut raw_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let nego = std::sync::Arc::new(std::sync::Mutex::new(Negotiator::new(term, cols, rows)));

    // Schreiben: Eingaben (maskiert), Verhandlungsantworten (roh), Größe.
    let nego_w = nego.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                c = cmd_rx.recv() => match c {
                    Some(StreamCmd::Data(d)) => { if wr.write_all(&escape(&d)).await.is_err() { break; } }
                    Some(StreamCmd::Resize(c, r)) => {
                        let seq = nego_w.lock().unwrap_or_else(|e| e.into_inner()).resize(c, r);
                        if !seq.is_empty() && wr.write_all(&seq).await.is_err() { break; }
                    }
                    Some(StreamCmd::Close) | None => { let _ = wr.shutdown().await; break; }
                },
                r = raw_rx.recv() => match r {
                    Some(seq) => { if wr.write_all(&seq).await.is_err() { break; } }
                    None => break,
                },
            }
        }
    });

    // Lesen: Verhandlung herausfiltern, Anmeldung erkennen, Daten ans Terminal.
    let have_user = !user.is_empty();
    let have_password = !password.as_bytes().is_empty();
    let login_cmd = cmd_tx.clone();
    tokio::spawn(async move {
        let mut login = AutoLogin::default();
        let mut buf = vec![0u8; 8192];
        loop {
            match rd.read(&mut buf).await {
                Ok(0) | Err(_) => {
                    let _ = ev_tx.send(ShellEvent::Closed);
                    break;
                }
                Ok(n) => {
                    let (data, reply) = nego.lock().unwrap_or_else(|e| e.into_inner()).feed(&buf[..n]);
                    if !reply.is_empty() {
                        let _ = raw_tx.send(reply);
                    }
                    if data.is_empty() {
                        continue;
                    }
                    if (have_user || have_password) && !login.done() {
                        match login.feed(&data, have_user, have_password) {
                            Some(LoginStep::User) => {
                                let _ = login_cmd.send(StreamCmd::Data(format!("{user}\r").into_bytes()));
                            }
                            Some(LoginStep::Password) => {
                                let mut p = password.as_bytes().to_vec();
                                p.push(b'\r');
                                let _ = login_cmd.send(StreamCmd::Data(p));
                            }
                            None => {}
                        }
                    }
                    if ev_tx.send(ShellEvent::Data(data)).is_err() {
                        break;
                    }
                }
            }
        }
    });
    Ok(Shell::from_stream(cmd_tx, ev_rx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiation_is_answered_once_and_removed_from_the_data() {
        let mut n = Negotiator::new("xterm", 120, 40);
        let (d, r) = n.feed(&[IAC, WILL, ECHO, IAC, WILL, SGA, IAC, DO, TTYPE, IAC, DO, NAWS, IAC, DO, 39, b'h', b'i']);
        assert_eq!(d, b"hi");
        let mut want = vec![IAC, DO, ECHO, IAC, DO, SGA, IAC, WILL, TTYPE, IAC, WILL, NAWS, IAC, SB, NAWS, 0, 120, 0, 40, IAC, SE, IAC, WONT, 39];
        assert_eq!(r, want);
        // Wiederholung: keine neue Antwort (keine Schleife), außer der Größe auf DO NAWS.
        let (_, r) = n.feed(&[IAC, WILL, ECHO, IAC, DO, 39]);
        assert!(r.is_empty());
        // Terminaltyp angefordert.
        let (_, r) = n.feed(&[IAC, SB, TTYPE, 1, IAC, SE]);
        want = vec![IAC, SB, TTYPE, 0];
        want.extend_from_slice(b"XTERM");
        want.extend_from_slice(&[IAC, SE]);
        assert_eq!(r, want);
        // Größe ändern: neue Unterverhandlung.
        assert_eq!(n.resize(80, 255), vec![IAC, SB, NAWS, 0, 80, 0, 255, 255, IAC, SE]);
    }

    #[test]
    fn split_sequences_and_doubled_iac_survive() {
        let mut n = Negotiator::new("vt100", 80, 24);
        let (d, r) = n.feed(&[b'a', IAC]);
        assert_eq!(d, b"a");
        assert!(r.is_empty());
        let (d, r) = n.feed(&[WILL, ECHO, b'b', IAC, IAC, b'c', IAC, SB, TTYPE]);
        assert_eq!(d, [b'b', IAC, b'c']);
        assert_eq!(r, vec![IAC, DO, ECHO]);
        let (_, r) = n.feed(&[1, IAC, SE]);
        assert!(r.starts_with(&[IAC, SB, TTYPE, 0]));
        assert_eq!(escape(&[1, IAC, 2]), vec![1, IAC, IAC, 2]);
    }

    /// Ganzer Ablauf gegen einen Testdienst auf 127.0.0.1: Verhandlung, automatische Anmeldung, Eingabe mit
    /// maskiertem Byte 255, Fenstergröße.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn session_against_a_test_service() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut got = Vec::new();
            let mut buf = [0u8; 512];
            // Liest, bis `want` in den empfangenen Bytes steht.
            async fn until(s: &mut tokio::net::TcpStream, got: &mut Vec<u8>, buf: &mut [u8], want: &[u8]) {
                while !got.windows(want.len()).any(|w| w == want) {
                    let n = tokio::time::timeout(Duration::from_secs(5), s.read(buf)).await.expect("Zeitgrenze").unwrap();
                    assert!(n > 0, "Verbindung zu früh beendet");
                    got.extend_from_slice(&buf[..n]);
                }
            }
            s.write_all(&[IAC, DO, TTYPE, IAC, DO, NAWS, IAC, WILL, ECHO]).await.unwrap();
            until(&mut s, &mut got, &mut buf, &[IAC, SB, NAWS, 0, 100, 0, 30, IAC, SE]).await;
            s.write_all(b"\r\nTestswitch\r\nUsername: ").await.unwrap();
            until(&mut s, &mut got, &mut buf, b"admin\r").await;
            s.write_all(b"\r\nPassword: ").await.unwrap();
            until(&mut s, &mut got, &mut buf, b"geheim\r").await;
            s.write_all(b"\r\nSW1# ").await.unwrap();
            until(&mut s, &mut got, &mut buf, &[b'x', IAC, IAC, b'y']).await;
            until(&mut s, &mut got, &mut buf, &[IAC, SB, NAWS, 0, 132, 0, 50, IAC, SE]).await;
            got
        });
        let mut shell = open("127.0.0.1", port, Duration::from_secs(5), "admin".into(), Secret::from_bytes(b"geheim".to_vec()), "xterm", 100, 30).await.unwrap();
        let mut seen = Vec::new();
        while !String::from_utf8_lossy(&seen).contains("SW1# ") {
            match tokio::time::timeout(Duration::from_secs(5), shell.events.recv()).await.expect("Zeitgrenze") {
                Some(ShellEvent::Data(d)) => seen.extend_from_slice(&d),
                other => panic!("unerwartet: {other:?}"),
            }
        }
        assert!(!seen.contains(&IAC), "keine Verhandlungsfolgen im Terminal");
        shell.write(&[b'x', IAC, b'y']).await.unwrap();
        shell.resize(132, 50).await.unwrap();
        let got = server.await.unwrap();
        assert!(got.windows(3).any(|w| w == [IAC, WILL, TTYPE]) && got.windows(3).any(|w| w == [IAC, DO, ECHO]));
        // Kennwort genau einmal gesendet.
        assert_eq!(got.windows(6).filter(|w| *w == b"geheim").count(), 1);
        shell.close().await;
    }

    #[test]
    fn auto_login_sends_user_then_password_once() {
        let mut l = AutoLogin::default();
        assert_eq!(l.feed(b"\r\nTestsystem\r\n\r\nUsername: ", true, true), Some(LoginStep::User));
        assert_eq!(l.feed(b"admin\r\nPassword: ", true, true), Some(LoginStep::Password));
        assert!(l.done());
        assert_eq!(l.feed(b"\r\nPassword: ", true, true), None, "kein zweites Mal");
        // Nur Kennwort (ohne Benutzeraufforderung) und ohne hinterlegten Benutzer.
        let mut l = AutoLogin::default();
        assert_eq!(l.feed(b"Password:", false, true), Some(LoginStep::Password));
        assert!(l.done());
        // Ohne Zugangsdaten: nichts senden.
        let mut l = AutoLogin::default();
        assert_eq!(l.feed(b"login: ", false, false), None);
        let mut l = AutoLogin::default();
        assert_eq!(l.feed(b"User Name:", true, true), Some(LoginStep::User));
    }
}
