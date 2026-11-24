//! Befehlszeilen-Sitzungen zu Netzgeräten: Anmeldung per SSH oder Telnet, Befehle in einer interaktiven CLI mit
//! Eingabeaufforderung. Herstellerneutral; die Erweiterungen nutzen sie über den Dienst mit Zugangsscheinen
//! (`cli_service.rs`), ohne Zugangsdaten zu sehen.
//!
//! Die Sitzung kennt die Eigenheiten gängiger Switch-Befehlszeilen: Begrüßung „Press any key to continue“,
//! Steuerzeichen (VT100) in der Ausgabe, Seitenumbruch „-- MORE --“ und Eingabeaufforderungen auf `#` oder `>`.
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

pub use rj_plugin_api::cli::{is_partial_prompt, is_prompt, lock, parse_help, raw_cut, raw_short, CliError, Session, HELP, WAIT};



/// Frist je Hilfeabfrage.
const HELP_LIMIT: Duration = Duration::from_secs(8);

/// Senden eines Befehls. Hilfeabfrage: erst den Text mit Leerzeichen (ohne Zeilenende), auf sein Echo warten
/// (höchstens 2 s), dann `?` einzeln – wie getippt. In einem Stück nehmen manche Switches das `?` als Zeichen in die
/// Zeile statt als Hilfeaufruf.
fn send_cmd(chan: &mut dyn Chan, cmd: &str, buf: &mut Vec<u8>) -> Result<(), CliError> {
    let Some(h) = cmd.strip_prefix(HELP).map(str::trim) else {
        return chan.send(format!("{cmd}\r").as_bytes()).map_err(CliError::Other);
    };
    let from = buf.len();
    chan.send(format!("{h} ").as_bytes()).map_err(CliError::Other)?;
    let until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < until {
        if let Ok(Some(d)) = chan.recv(Duration::from_millis(100)) {
            buf.extend_from_slice(&d);
        }
        // Echo, auch umgebrochen („interface “ / „35 speed-d“ / „uplex “).
        let joined: String = clean(&buf[from..]).lines().collect();
        if joined.contains(h) {
            break;
        }
    }
    chan.send(b"?").map_err(CliError::Other)
}

/// Löschen der Eingabezeile nach einer Hilfeabfrage: Rücktasten (DEL) für den Text, das Leerzeichen und etwas
/// Reserve, dann Enter.
fn erase_line(typed: &str) -> Vec<u8> {
    let mut v = vec![0x7f_u8; typed.chars().count() + 1 + 4];
    v.push(b'\r');
    v
}

/// Eingabezeile mit angefangenem Befehl nach der Hilfeliste, auch über mehrere Zeilen umgebrochen: die letzte Zeile ab
/// `from`, die mit einer Eingabeaufforderung beginnt, zusammen mit den folgenden Zeilen endet auf `typed`.
fn partial_prompt_at(lines: &[&str], from: usize, typed: &str) -> bool {
    let Some(i) = (from..lines.len()).rev().find(|&i| lines[i].split_whitespace().next().is_some_and(|t| is_prompt(t) || is_partial_prompt(lines[i], typed))) else { return false };
    let joined: String = lines[i..].concat();
    is_partial_prompt(&joined, typed)
}

/// Gerätename aus einer Eingabeaufforderung mit Leerzeichen, wie sie direkt nach der Anmeldung steht (manche Switches
/// erlaubt Leerzeichen im Namen: „SW1 Halle#“). Nur kurze Zeilen, die auf `#` oder `>` enden und sonst keine
/// Sonderzeichen tragen; Tabellenköpfe wie „Serial #“ stehen nie allein nach der Anmeldung.
fn learn_prompt(line: &str) -> Option<String> {
    let l = line.trim();
    let base = l.strip_suffix('#').or_else(|| l.strip_suffix('>'))?.trim_end();
    let ok = !base.is_empty() && base.len() <= 64 && base.contains(' ') && !base.contains(['#', '>', ':', '(', ')']);
    ok.then(|| base.to_string())
}

/// Eingabeaufforderung: wie [`is_prompt`] oder der gelernte Gerätename mit Leerzeichen, auch im Untermodus
/// („SW1 Halle(config)#“).
fn prompt_with(line: &str, learned: Option<&str>) -> bool {
    if is_prompt(line) {
        return true;
    }
    let Some(base) = learned else { return false };
    let l = line.trim();
    let Some(rest) = l.strip_prefix(base) else { return false };
    let rest = rest.strip_suffix('#').or_else(|| rest.strip_suffix('>'));
    match rest {
        Some("") => true,
        Some(r) => r.starts_with('(') && r.ends_with(')') && !r[1..r.len() - 1].contains(char::is_whitespace),
        None => false,
    }
}



/// Kanal zu einem Gerät: Daten lesen (mit Frist) und schreiben.
pub trait Chan {
    /// Neue Daten; `Ok(None)` bei Fristablauf, `Err` bei geschlossener Verbindung.
    fn recv(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, String>;
    fn send(&mut self, data: &[u8]) -> Result<(), String>;
}



/// Steuerzeichen entfernen: VT100-Folgen (ESC [ … Buchstabe, ESC einzeln), Rücktaste samt Zeichen, `\r`. Eine
/// Cursor-Positionierung (ESC [ … H bzw. f) wird zum Zeilenumbruch: manche Switches setzen die Eingabeaufforderung
/// damit an den Zeilenanfang, ohne einen Umbruch zu senden.
pub fn clean(raw: &[u8]) -> String {
    let s = String::from_utf8_lossy(raw);
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\u{1b}' => match it.peek() {
                Some('[') => {
                    it.next();
                    // Parameter bis zum abschließenden Buchstaben.
                    while let Some(&n) = it.peek() {
                        it.next();
                        if n.is_ascii_alphabetic() || n == '@' || n == '~' {
                            if (n == 'H' || n == 'f') && !out.is_empty() && !out.ends_with('\n') {
                                out.push('\n');
                            }
                            break;
                        }
                    }
                }
                Some(_) => {
                    it.next();
                }
                None => {}
            },
            '\u{8}' => {
                out.pop();
            }
            '\r' => {}
            c if c.is_control() && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

fn last_line(text: &str) -> &str {
    text.lines().rev().find(|l| !l.trim().is_empty()).map(str::trim).unwrap_or("")
}


fn lower_tail(text: &str, n: usize) -> String {
    let start = text.len().saturating_sub(n);
    let start = (start..text.len()).find(|i| text.is_char_boundary(*i)).unwrap_or(text.len());
    text[start..].to_lowercase()
}

/// Interaktive Sitzung: optional anmelden (`login`: Benutzer, Kennwort – bei Telnet), Begrüßung bestätigen, auf die
/// Eingabeaufforderung warten, dann je Befehl senden und lesen, bis nach dem Echo die nächste Eingabeaufforderung
/// kommt. Gesamtfrist `limit`.
pub fn run(chan: &mut dyn Chan, login: Option<(&str, &str)>, cmds: &[&str], limit: Duration) -> Result<Session, CliError> {
    let mut log = String::new();
    run_logged(chan, login, cmds, limit, &mut log)
}

/// Wie [`run`]; `log` erhält die bereinigte Antwort des Geräts über die ganze Sitzung (auch bei Fehlern; das Kennwort
/// zeigt kein Gerät an), für „Rohantwort kopieren“.
pub fn run_logged(chan: &mut dyn Chan, login: Option<(&str, &str)>, cmds: &[&str], limit: Duration, log: &mut String) -> Result<Session, CliError> {
    let begin = Instant::now();
    let deadline = begin + limit;
    let mut timed_out = false;
    let mut times: Vec<f32> = Vec::new();
    let mut buf: Vec<u8> = Vec::new();
    let mut user_sent = false;
    let mut pass_sent = 0;
    let mut keys = 0;
    // Gelernter Gerätename mit Leerzeichen (siehe `learn_prompt`).
    let mut learned: Option<String> = None;
    let mut idx = 0usize;
    let mut started = false;
    let mut mark = 0usize;
    let mut outputs = Vec::new();
    // Länge der Rohantwort beim letzten Weiterblättern (je Seitenumbruch nur einmal Leertaste).
    let mut more_at = usize::MAX;
    // Ohne erkennbare Eingabeaufforderung nach 4 s trotzdem anfangen.
    let mut quiet_since = Instant::now();
    // Beginn der laufenden Hilfeabfrage (Frist 8 s; danach weiter mit dem, was da ist).
    let mut help_since = Instant::now();
    loop {
        if Instant::now() >= deadline {
            timed_out = true;
            break;
        }
        match chan.recv(Duration::from_millis(300)) {
            Ok(Some(d)) => {
                buf.extend_from_slice(&d);
                quiet_since = Instant::now();
            }
            Ok(None) => {}
            Err(e) => {
                if started || !outputs.is_empty() {
                    break;
                }
                log.push_str(&clean(&buf));
                return Err(CliError::Other(e));
            }
        }
        let text = clean(&buf);
        let tail = lower_tail(&text, 200);
        if !started {
            if let Some((user, pass)) = login {
                if !user_sent && (tail.ends_with("username:") || tail.ends_with("login:") || tail.trim_end().ends_with("username:") || tail.trim_end().ends_with("login:")) {
                    chan.send(format!("{user}\r").as_bytes()).map_err(CliError::Other)?;
                    user_sent = true;
                    log.push_str(&clean(&buf));
                    buf.clear();
                    continue;
                }
                if tail.trim_end().ends_with("password:") {
                    if pass_sent >= 1 {
                        {
                    log.push_str(&clean(&buf));
                    return Err(CliError::Auth);
                }
                    }
                    chan.send(format!("{pass}\r").as_bytes()).map_err(CliError::Other)?;
                    pass_sent += 1;
                    log.push_str(&clean(&buf));
                    buf.clear();
                    continue;
                }
            }
            if tail.contains("invalid password") || tail.contains("login incorrect") || tail.contains("access denied") || tail.contains("authentication failed") {
                {
                    log.push_str(&clean(&buf));
                    return Err(CliError::Auth);
                }
            }
            if tail.contains("press any key") && keys < 3 {
                chan.send(b" ").map_err(CliError::Other)?;
                keys += 1;
                log.push_str(&clean(&buf));
                    buf.clear();
                continue;
            }
            // Eingabeaufforderung mit Leerzeichen im Namen: nach der Anmeldung lernen, danach wie jede andere behandeln.
            if learned.is_none() && !is_prompt(last_line(&text)) {
                learned = learn_prompt(last_line(&text));
            }
            let prompt = prompt_with(last_line(&text), learned.as_deref());
            if prompt || quiet_since.elapsed() > Duration::from_secs(4) {
                if cmds.is_empty() {
                    break;
                }
                started = true;
                log.push_str(&clean(&buf));
                    buf.clear();
                mark = 0;
                send_cmd(chan, cmds[0], &mut buf)?;
                help_since = Instant::now();
            }
            continue;
        }
        // Seitenumbruch trotz „no page“: weiterblättern (auch „-- MORE --, next page: Space, …“ in der letzten Zeile).
        let last = tail.trim_end().rsplit('\n').next().unwrap_or("");
        if last.contains("-- more --") || last.contains("--more--") {
            if more_at != buf.len() {
                more_at = buf.len();
                chan.send(b" ").map_err(CliError::Other)?;
            }
            continue;
        }
        // Ab dem letzten Befehl, nach Bytes der Rohantwort (der bereinigte Text kann durch Rücktasten kürzer werden).
        let seg = clean(&buf[mark.min(buf.len())..]);
        let lines: Vec<&str> = seg.lines().collect();
        let help = cmds[idx].strip_prefix(HELP).map(str::trim);
        let cmd = help.unwrap_or(cmds[idx]);
        // Echo des Befehls: auch über mehrere Zeilen verteilt (manche Switches setzen den Cursor mitten im Echo neu).
        let echo = {
            let mut joined = String::new();
            lines.iter().position(|l| {
                joined.push_str(l);
                joined.contains(cmd)
            })
        };
        // Hilfe nach Fristablauf: auch ohne erkanntes Echo bzw. ohne Zeilen weiter.
        let help_over = help.is_some() && help_since.elapsed() > HELP_LIMIT;
        let echo = echo.or_else(|| help_over.then_some(0));
        if let Some(echo) = echo {
            if let Some(last) = lines.iter().rposition(|l| !l.trim().is_empty()).or_else(|| help_over.then_some(0)) {
                let partial = |h: &str| last > echo && partial_prompt_at(&lines, echo + 1, h);
                let done = match help {
                    Some(h) => partial(h) || help_over,
                    None => last > echo && prompt_with(lines[last], learned.as_deref()),
                };
                if done {
                    // Hilfe nach Fristablauf: alles nach dem Echo, ohne angefangene Eingabezeile.
                    let end = match help {
                        // Bis vor die (ggf. umgebrochene) Eingabezeile.
                        Some(h) if partial(h) => (echo + 1..lines.len()).rev().find(|&i| lines[i].split_whitespace().next().is_some_and(is_prompt) && is_partial_prompt(&lines[i..].concat(), h)).unwrap_or(last),
                        Some(_) => last + 1,
                        None => last,
                    }
                    .min(lines.len());
                    let out: Vec<&str> = lines[(echo + 1).min(end)..end].iter().copied().filter(|l| !l.contains("-- MORE --")).collect();
                    outputs.push(out.join("\n"));
                    times.push(begin.elapsed().as_secs_f32());
                    idx += 1;
                    if let Some(h) = help {
                        // Angefangenen Befehl nie ausführen – manche sind vollständig (etwa
                        // „interface 5 flow-control“ oder „speed auto“): Zeile mit Rücktasten leeren (einige mehr
                        // als getippt), dann Enter auf der leeren Zeile und auf die Eingabeaufforderung warten.
                        chan.send(&erase_line(h)).map_err(CliError::Other)?;
                        let from = buf.len();
                        let until = (Instant::now() + Duration::from_secs(8)).min(deadline);
                        while Instant::now() < until {
                            if let Ok(Some(d)) = chan.recv(Duration::from_millis(300)) {
                                buf.extend_from_slice(&d);
                            }
                            let seg = clean(&buf[from..]);
                            if seg.lines().filter(|l| !l.trim().is_empty()).count() >= 1 && prompt_with(last_line(&seg), learned.as_deref()) {
                                break;
                            }
                        }
                    }
                    mark = buf.len();
                    // Pause zwischen zwei Befehlen („#wait 10“, z. B. Kaltstart): weiter lesen, nichts senden.
                    while idx < cmds.len() {
                        let Some(secs) = cmds[idx].strip_prefix(WAIT).and_then(|x| x.trim().parse::<u64>().ok()) else { break };
                        let until = Instant::now() + Duration::from_secs(secs);
                        while Instant::now() < until {
                            if let Ok(Some(d)) = chan.recv(Duration::from_millis(300)) {
                                buf.extend_from_slice(&d);
                            }
                        }
                        outputs.push(String::new());
                        times.push(begin.elapsed().as_secs_f32());
                        idx += 1;
                        mark = buf.len();
                    }
                    if idx == cmds.len() {
                        break;
                    }
                    send_cmd(chan, cmds[idx], &mut buf)?;
                    help_since = Instant::now();
                }
            }
        }
    }
    let text = clean(&buf);
    log.push_str(&text);
    if started && idx < cmds.len() {
        // Unvollständig: was da ist.
        outputs.push(clean(&buf[mark.min(buf.len())..]));
        times.push(begin.elapsed().as_secs_f32());
    }
    if !started && login.is_some() && !user_sent {
        return Err(CliError::Other("Keine Anmeldeaufforderung erhalten".into()));
    }
    Ok(Session { outputs, text, times, timed_out })
}

/// Telnet über TCP: Optionsverhandlung (IAC) wird beantwortet – Echo und „Suppress Go Ahead“ vom Gerät
/// angenommen, alles andere abgelehnt – und aus den Daten entfernt.
pub struct Telnet {
    sock: TcpStream,
}

impl Telnet {
    pub fn connect(ip: std::net::IpAddr, port: u16, timeout: Duration) -> Result<Telnet, String> {
        let sock = TcpStream::connect_timeout(&SocketAddr::new(ip, port), timeout).map_err(|e| format!("Telnet {ip}:{port}: {e}"))?;
        let _ = sock.set_nodelay(true);
        Ok(Telnet { sock })
    }
}

/// IAC-Folgen aus `data` entfernen und die Antworten dazu liefern.
pub fn telnet_filter(data: &[u8]) -> (Vec<u8>, Vec<u8>) {
    const IAC: u8 = 255;
    const DONT: u8 = 254;
    const DO: u8 = 253;
    const WONT: u8 = 252;
    const WILL: u8 = 251;
    const SB: u8 = 250;
    const SE: u8 = 240;
    let mut out = Vec::with_capacity(data.len());
    let mut reply = Vec::new();
    let mut i = 0;
    while i < data.len() {
        if data[i] != IAC {
            out.push(data[i]);
            i += 1;
            continue;
        }
        match data.get(i + 1) {
            Some(&IAC) => {
                out.push(IAC);
                i += 2;
            }
            Some(&c @ (DO | DONT | WILL | WONT)) => {
                if let Some(&opt) = data.get(i + 2) {
                    match c {
                        // Gerät will echoen bzw. ohne Go-Ahead arbeiten: annehmen, sonst ablehnen.
                        WILL => reply.extend_from_slice(&[IAC, if opt == 1 || opt == 3 { DO } else { DONT }, opt]),
                        DO => reply.extend_from_slice(&[IAC, WONT, opt]),
                        _ => {}
                    }
                }
                i += 3;
            }
            Some(&SB) => {
                // Unterverhandlung bis IAC SE überspringen.
                let mut j = i + 2;
                while j + 1 < data.len() && !(data[j] == IAC && data[j + 1] == SE) {
                    j += 1;
                }
                i = j + 2;
            }
            Some(_) => i += 2,
            None => i += 1,
        }
    }
    (out, reply)
}

impl Chan for Telnet {
    fn recv(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, String> {
        let _ = self.sock.set_read_timeout(Some(timeout.max(Duration::from_millis(10))));
        let mut b = [0u8; 4096];
        match self.sock.read(&mut b) {
            Ok(0) => Err("Verbindung geschlossen".into()),
            Ok(n) => {
                let (data, reply) = telnet_filter(&b[..n]);
                if !reply.is_empty() {
                    let _ = self.sock.write_all(&reply);
                }
                Ok(Some(data))
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn send(&mut self, data: &[u8]) -> Result<(), String> {
        self.sock.write_all(data).map_err(|e| e.to_string())
    }
}

/// SSH-Shell als Kanal (blockierend über die Laufzeit der Verbindung).
pub struct SshChan<'a> {
    pub rt: &'a tokio::runtime::Runtime,
    pub shell: rj_ssh::client::Shell,
}

impl Chan for SshChan<'_> {
    fn recv(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, String> {
        match wait_for(self.rt, timeout, self.shell.events.recv()) {
            Ok(Some(rj_ssh::client::ShellEvent::Data(d))) => Ok(Some(d)),
            Ok(Some(_)) | Ok(None) => Err("Verbindung geschlossen".into()),
            Err(_) => Ok(None),
        }
    }

    fn send(&mut self, data: &[u8]) -> Result<(), String> {
        self.rt.block_on(self.shell.write(data)).map_err(|e| e.to_string())
    }
}

/// Auf `fut` höchstens `timeout` warten. Der Zeitgeber wird erst innerhalb der Laufzeit angelegt (außerhalb fehlt der
/// Tokio-Kontext: Panik „there is no reactor running“).
pub fn wait_for<F: std::future::Future>(rt: &tokio::runtime::Runtime, timeout: Duration, fut: F) -> Result<F::Output, tokio::time::error::Elapsed> {
    rt.block_on(async { tokio::time::timeout(timeout, fut).await })
}

// Gemeinsam für Dienst, Inventar und Suche



/// Zugangsdaten für eine Serie (aufgelöst, mit Kennwort) – für die Anmeldung.
#[derive(Clone)]
pub struct Cred {
    pub name: &'static str,
    pub payload: rj_ipc::CredentialPayload,
}



/// SSH-Sitzung zu einem Switch; mit `legacy` auch ältere Verfahren (SHA-1-Schlüsselaustausch, CBC).
pub fn ssh_try(ip: std::net::Ipv4Addr, c: &Cred, legacy: bool, known: &rj_ssh::known_hosts::KnownHosts, cmds: &[&str], limit: Duration, log: &mut String) -> Result<Session, CliError> {
    let spec = crate::patch::JobSpec { protocol: rj_core::Protocol::Ssh, host: ip.to_string(), port: 22, credentials: c.payload.clone(), transport: None, legacy };
    match crate::overview::remote::ssh_open(&spec, 22, known.clone()) {
        Ok((rt, conn, _, _, _)) => match rt.block_on(conn.open_shell("vt100", 200, 50)) {
            Ok(shell) => {
                let mut ch = SshChan { rt: &rt, shell };
                let r = run_logged(&mut ch, None, cmds, limit, log);
                rt.block_on(ch.shell.close());
                rt.block_on(conn.disconnect());
                r
            }
            Err(e) => Err(CliError::Other(e.to_string())),
        },
        Err(e) if e.to_lowercase().contains("anmeld") || e.to_lowercase().contains("auth") => Err(CliError::Auth),
        Err(e) => Err(CliError::Other(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Nachgestellter Switch mit Leerzeichen im Namen (Rohantwort eines echten Geräts vom 24.11.2026, ohne Adressen):
    /// Hinweistext, „Press any key“, dann „SW1 Halle# “; je Befehl Echo, Ausgabe und Eingabeaufforderung.
    struct Switch {
        out: VecDeque<Vec<u8>>,
        line: String,
        after_key: bool,
    }

    impl Switch {
        fn new() -> Self {
            let banner = "Beispiel X100 Switch\r\nSoftware revision AB.16.11.0027\r\n\r\n (C) Copyright 2025 Beispiel GmbH\r\n\r\nPress any key to continue";
            Switch { out: VecDeque::from([banner.as_bytes().to_vec()]), line: String::new(), after_key: false }
        }
    }

    impl Chan for Switch {
        fn recv(&mut self, _timeout: Duration) -> Result<Option<Vec<u8>>, String> {
            Ok(self.out.pop_front())
        }
        fn send(&mut self, data: &[u8]) -> Result<(), String> {
            if !self.after_key {
                self.after_key = true;
                self.out.push_back(b"\r\nYour previous successful login (as manager) was on 2026-01-01 08:00:00\r\nSW1 Halle# ".to_vec());
                return Ok(());
            }
            self.line.push_str(&String::from_utf8_lossy(data));
            if let Some(cmd) = self.line.strip_suffix('\r').or_else(|| self.line.strip_suffix('\n')).map(|c| c.trim().to_string()) {
                self.line.clear();
                let body = match cmd.as_str() {
                    "show version" => "Image stamp:    /build/beispiel\r\n Software revision  : AB.16.11.0027\r\n",
                    "show system" => " System Name        : SW1 Halle\r\n Serial Number      : CN00000000\r\n",
                    _ => "",
                };
                self.out.push_back(format!("{cmd}\r\n{body}\r\nSW1 Halle# ").into_bytes());
            }
            Ok(())
        }
    }

    #[test]
    fn prompt_with_spaces_in_the_switch_name_is_learned() {
        let s = run(&mut Switch::new(), None, &["no page", "show version", "show system"], Duration::from_secs(20)).unwrap();
        assert!(!s.timed_out, "alle Befehle beantwortet");
        assert_eq!(s.outputs.len(), 3);
        assert!(s.outputs[1].contains("AB.16.11.0027") && s.outputs[2].contains("SW1 Halle"), "{:?}", s.outputs);
    }

    #[test]
    fn learned_prompt_rules() {
        assert_eq!(learn_prompt("SW1 Halle# ").as_deref(), Some("SW1 Halle"));
        assert_eq!(learn_prompt("SW1 Halle>").as_deref(), Some("SW1 Halle"));
        assert_eq!(learn_prompt("sw-1#"), None, "ohne Leerzeichen gilt die normale Regel");
        assert!(prompt_with("SW1 Halle#", Some("SW1 Halle")) && prompt_with("SW1 Halle(config)#", Some("SW1 Halle")));
        assert!(!prompt_with("Name  IP Address  Serial #", Some("SW1 Halle")), "Tabellenkopf bleibt keine Eingabeaufforderung");
        assert!(!prompt_with("SW1 Halle uptime #", Some("SW1 Halle")));
        assert!(!prompt_with("Name  IP Address  Serial #", None));
    }
}
