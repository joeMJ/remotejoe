//! Dienst für Befehlszeilen-Sitzungen, SNMP und externe Programme mit den Zugangsdaten des Kerns. Erweiterungen
//! bekommen Zugangsscheine (Nummern); die Zugangsdaten bleiben hier und werden beim Sperren verworfen.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rj_plugin_api::{CliOptions, CliOutput, CliTicket, DialogStep};
use rj_ssh::known_hosts::KnownHosts;
use serde_json::{json, Value};

use crate::cli::{run_logged, ssh_try, CliError, Cred, Telnet};
use crate::patch::JobSpec;

enum Ticket {
    /// SSH (bei Bedarf Telnet) zu einer Adresse mit aufgelösten Zugangsdaten; `cred`: Eintrag der Zugangsdaten in
    /// der Schlüsseldatenbank (Kennung, kein Geheimnis – zum Erkennen gleicher Zugangsdaten).
    Cli { spec: JobSpec, known: KnownHosts, cred: Option<uuid::Uuid> },
    /// Offene SSH-Verbindung des Kerns (Inventar): Befehle laufen darauf, der Kern trennt danach. Mit den Angaben
    /// für eine frische Verbindung (Geräte, die nach einem Befehl den Kanal schließen).
    Open { open: OpenSsh },
    /// SNMP-Zugangsdaten eines Geräts.
    Snmp { device: rj_snmp::Device },
    /// HTTPS mit den Zugangsdaten eines Ziels bzw. Wegs (siehe [`crate::https_ticket`]).
    Http { spec: Arc<crate::https_ticket::HttpSpec> },
    /// Wird gerade benutzt (aus der Tabelle genommen).
    Busy,
}

/// Offene SSH-Verbindung samt Angaben zum Neuverbinden (Host-Schlüssel sind nach der ersten Verbindung bekannt).
pub struct OpenSsh {
    pub rt: tokio::runtime::Runtime,
    pub conn: rj_ssh::client::SshConnection,
    pub params: rj_ssh::client::SshParams,
    pub known: KnownHosts,
    pub ask: crate::overview::remote::AskSender,
}

#[derive(Default)]
pub struct CliService {
    tickets: Mutex<HashMap<u64, Ticket>>,
    next: AtomicU64,
    /// Gestartete externe Programme (Prozessnummer → Kind), zum Beenden und für den Zustand.
    children: Mutex<HashMap<u32, std::process::Child>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl CliService {
    fn put(&self, t: Ticket) -> CliTicket {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        lock(&self.tickets).insert(id, t);
        CliTicket(id)
    }

    /// Schein für SSH/Telnet mit aufgelösten Zugangsdaten (`cred`: Eintrag in der Schlüsseldatenbank).
    pub fn issue(&self, spec: JobSpec, known: KnownHosts, cred: Option<uuid::Uuid>) -> CliTicket {
        self.put(Ticket::Cli { spec, known, cred })
    }

    /// Schein für eine offene Verbindung (der Aufrufer holt sie mit [`take_open`](Self::take_open) zurück).
    pub fn issue_open(&self, open: OpenSsh) -> CliTicket {
        self.put(Ticket::Open { open })
    }

    pub fn issue_snmp(&self, device: rj_snmp::Device) -> CliTicket {
        self.put(Ticket::Snmp { device })
    }

    /// Schein für HTTPS-Anfragen (Zugangsdaten bleiben im Kern).
    pub fn issue_http(&self, spec: crate::https_ticket::HttpSpec) -> CliTicket {
        self.put(Ticket::Http { spec: Arc::new(spec) })
    }

    /// HTTPS-Anfrage über einen Schein (siehe [`crate::https_ticket::request`]); mehrere Anfragen dürfen gleichzeitig
    /// laufen.
    pub fn http(&self, t: CliTicket, a: &Value) -> Result<Value, String> {
        let spec = match lock(&self.tickets).get(&t.0) {
            Some(Ticket::Http { spec }) => spec.clone(),
            Some(_) => return Err("Kein HTTPS-Ticket".into()),
            None => return Err("Ticket unbekannt oder verfallen (Schlüsseldatenbank gesperrt?)".into()),
        };
        crate::https_ticket::request(&spec, a)
    }

    pub fn take_open(&self, t: CliTicket) -> Option<OpenSsh> {
        match lock(&self.tickets).remove(&t.0) {
            Some(Ticket::Open { open }) => Some(open),
            _ => None,
        }
    }

    pub fn revoke(&self, t: CliTicket) {
        lock(&self.tickets).remove(&t.0);
    }

    /// Beim Sperren: alle Scheine verwerfen.
    pub fn clear(&self) {
        lock(&self.tickets).clear();
    }

    /// Adresse und Port hinter einem Schein (ohne Zugangsdaten).
    pub fn info(&self, t: CliTicket) -> Value {
        match lock(&self.tickets).get(&t.0) {
            Some(Ticket::Cli { spec, cred, .. }) => json!({ "host": spec.host, "port": spec.port, "user": spec.credentials.username, "legacy": spec.legacy, "cred_id": cred }),
            Some(Ticket::Snmp { device }) => json!({ "host": device.host, "port": device.port }),
            Some(Ticket::Http { spec }) => json!({ "https": true, "host": spec.host, "port": spec.port, "user": spec.credentials.username }),
            Some(Ticket::Open { open }) => json!({ "open": true, "host": open.params.host, "port": open.params.port, "user": open.params.user }),
            Some(Ticket::Busy) => json!({ "busy": true }),
            None => Value::Null,
        }
    }

    /// Sitzung: je Aufruf eine Verbindung, die Schritte, dann trennen. Switch-CLI: SSH, bei abgelehntem
    /// Schlüsseltausch ältere Verfahren, bei abgelehnter Verbindung Telnet (wenn erlaubt). Sonst die einfache Shell
    /// mit Eingabeaufforderung (Controller, APs) über die offene bzw. neue Verbindung.
    pub fn run(&self, t: CliTicket, steps: &[DialogStep], o: &CliOptions) -> Result<CliOutput, String> {
        // Ein Ticket in Benutzung bleibt unangetastet (sonst ginge es dem verloren, der es gerade benutzt).
        let taken = {
            let mut tab = lock(&self.tickets);
            if matches!(tab.get(&t.0), Some(Ticket::Busy)) {
                return Err("Ticket ist gerade in Benutzung".into());
            }
            let taken = tab.remove(&t.0).ok_or("Ticket unbekannt oder verfallen (Schlüsseldatenbank gesperrt?)")?;
            // Während der Sitzung steht das Ticket als „in Benutzung“ in der Tabelle, damit ein Sperren es verwirft.
            tab.insert(t.0, Ticket::Busy);
            taken
        };
        let limit = Duration::from_secs(o.limit_secs.max(1));
        let (result, back) = match taken {
            Ticket::Cli { spec, known, cred } => {
                let used = match &o.host {
                    Some(h) if !h.is_empty() => JobSpec { protocol: spec.protocol, host: h.clone(), port: spec.port, credentials: spec.credentials.clone(), transport: spec.transport.clone(), legacy: spec.legacy },
                    _ => JobSpec { protocol: spec.protocol, host: spec.host.clone(), port: spec.port, credentials: spec.credentials.clone(), transport: spec.transport.clone(), legacy: spec.legacy },
                };
                let r = if o.switch_cli { switch_session(&used, &known, steps, o, limit) } else { shell_session(&used, known.clone(), steps, limit) };
                (r, Some(Ticket::Cli { spec, known, cred }))
            }
            Ticket::Open { mut open } => {
                if o.fresh {
                    open.rt.block_on(open.conn.disconnect());
                    match open.rt.block_on(rj_ssh::client::SshConnection::connect(&open.params, open.known.clone(), open.ask.clone())) {
                        Ok(c) => open.conn = c,
                        Err(e) => {
                            lock(&self.tickets).remove(&t.0);
                            return Err(e.to_string());
                        }
                    }
                }
                let r = open_session(&open, steps, o, limit);
                (r, Some(Ticket::Open { open }))
            }
            Ticket::Snmp { device } => (Err("Kein Befehlszeilen-Schein".into()), Some(Ticket::Snmp { device })),
            Ticket::Http { spec } => (Err("Kein Befehlszeilen-Schein".into()), Some(Ticket::Http { spec })),
            Ticket::Busy => (Err("Ticket ist gerade in Benutzung".into()), None),
        };
        let mut tab = lock(&self.tickets);
        match (tab.remove(&t.0), back) {
            // Zwischenzeitlich verworfen (Sperre): nicht wieder eintragen.
            (Some(Ticket::Busy), Some(b)) => {
                tab.insert(t.0, b);
            }
            _ => {}
        }
        result
    }

    /// SNMP-Werte über den Schein eines Geräts.
    pub fn snmp_get(&self, t: CliTicket, host: &str, oids: &[String]) -> Result<Value, String> {
        let device = match lock(&self.tickets).get(&t.0) {
            Some(Ticket::Snmp { device }) => rj_snmp::Device { host: if host.is_empty() { device.host.clone() } else { host.to_string() }, ..device.clone() },
            _ => return Err("Kein SNMP-Schein".into()),
        };
        let mut cl = rj_snmp::Client::open(&device).map_err(|e| e.to_string())?;
        let refs: Vec<&str> = oids.iter().map(String::as_str).collect();
        let mut out = serde_json::Map::new();
        for (oid, v) in cl.get(&refs).map_err(|e| e.to_string())? {
            let j = match v {
                rj_snmp::Val::Int(i) => json!(i),
                rj_snmp::Val::Counter32(c) | rj_snmp::Val::Gauge(c) | rj_snmp::Val::Ticks(c) => json!(c),
                rj_snmp::Val::Counter64(c) => json!(c),
                rj_snmp::Val::Text(t) | rj_snmp::Val::Oid(t) => json!(t),
                rj_snmp::Val::Missing => Value::Null,
            };
            out.insert(oid, j);
        }
        Ok(Value::Object(out))
    }

    /// Externes Programm mit den Zugangsdaten eines Scheins: `{{user}}`/`{{password}}` in Argumenten, Umgebung und
    /// der ersten Eingabezeile (`stdin`, danach geschlossen); Ausgabe und Fehlerausgabe gehen in die Datei `log`.
    /// Unter Windows ohne Konsolenfenster.
    pub fn run_external(&self, t: CliTicket, program: &str, args: &[String], env: &serde_json::Map<String, Value>, cwd: &str, log: &str, stdin: Option<&str>) -> Result<u32, String> {
        let (user, pass) = match lock(&self.tickets).get(&t.0) {
            Some(Ticket::Cli { spec, .. }) => (spec.credentials.username.clone(), String::from_utf8_lossy(&spec.credentials.password.0).into_owned()),
            _ => return Err("Kein Ticket".into()),
        };
        // JSON-gerecht einsetzen (Anführungszeichen und Rückstriche im Kennwort).
        let fill = |s: &str| s.replace("{{user}}", &json_inner(&user)).replace("{{password}}", &json_inner(&pass));
        let mut cmd = std::process::Command::new(program);
        cmd.args(args.iter().map(|a| fill(a)));
        for (k, v) in env {
            cmd.env(k, fill(v.as_str().unwrap_or_default()));
        }
        if !cwd.is_empty() {
            cmd.current_dir(cwd);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let out = std::fs::File::create(log).map_err(|e| e.to_string())?;
        let err = out.try_clone().map_err(|e| e.to_string())?;
        cmd.stdout(out).stderr(err).stdin(if stdin.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() });
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        if let (Some(text), Some(mut sin)) = (stdin, child.stdin.take()) {
            use std::io::Write;
            let mut line = fill(text).into_bytes();
            let _ = sin.write_all(&line);
            zeroize::Zeroize::zeroize(&mut line);
        }
        let pid = child.id();
        lock(&self.children).insert(pid, child);
        Ok(pid)
    }

    /// Externes Programm beenden.
    pub fn external_stop(&self, pid: u32) {
        if let Some(c) = lock(&self.children).get_mut(&pid) {
            let _ = c.kill();
        }
    }

    /// Zustand eines externen Programms: läuft es noch, sonst sein Rückgabewert (danach vergessen).
    pub fn external_state(&self, pid: u32) -> Value {
        let mut tab = lock(&self.children);
        let Some(c) = tab.get_mut(&pid) else { return json!({ "running": false, "code": Value::Null, "known": false }) };
        match c.try_wait() {
            Ok(Some(st)) => {
                let code = st.code();
                tab.remove(&pid);
                json!({ "running": false, "code": code, "known": true })
            }
            Ok(None) => json!({ "running": true, "known": true }),
            Err(e) => {
                tab.remove(&pid);
                json!({ "running": false, "code": Value::Null, "known": true, "error": e.to_string() })
            }
        }
    }
}

/// Text wie in einem JSON-String (ohne die umschließenden Anführungszeichen).
fn json_inner(s: &str) -> String {
    let j = serde_json::to_string(s).unwrap_or_default();
    j[1..j.len().saturating_sub(1)].to_string()
}

/// Schritte auf einer offenen Verbindung: Switch-Engine (Begrüßung, Seitenumbruch) oder einfache Shell.
fn open_session(open: &OpenSsh, steps: &[DialogStep], o: &CliOptions, limit: Duration) -> Result<CliOutput, String> {
    if o.switch_cli {
        let cmds: Vec<&str> = steps.iter().map(|s| s.command.as_str()).collect();
        let shell = open.rt.block_on(open.conn.open_shell("vt100", o.cols.max(80), 50)).map_err(|e| e.to_string())?;
        let mut ch = crate::cli::SshChan { rt: &open.rt, shell };
        let mut log = String::new();
        let r = run_logged(&mut ch, None, &cmds, limit, &mut log);
        open.rt.block_on(ch.shell.close());
        return match r {
            Ok(s) => Ok(CliOutput { outputs: s.outputs, times: s.times, timed_out: s.timed_out, log, note: None, via: "ssh".into() }),
            Err(CliError::Auth) => Err("Anmeldung abgelehnt".into()),
            Err(CliError::Other(e)) => Err(e),
        };
    }
    let st: Vec<crate::overview::remote::CliStep> = steps.iter().map(|s| crate::overview::remote::CliStep { command: s.command.clone(), prompt: s.prompt.clone(), answer: s.answer.clone() }).collect();
    let r = open.rt.block_on(crate::overview::remote::cli_session_steps(&open.conn, &st, limit))?;
    Ok(CliOutput { timed_out: r.outputs.len() < steps.len(), outputs: r.outputs, log: r.raw, note: None, via: "ssh".into(), times: Vec::new() })
}

/// Switch-Befehlszeile: SSH, bei abgelehntem Schlüsseltausch mit älteren Verfahren, bei abgelehnter Verbindung
/// Telnet (abschaltbar). Gemeinsam für die Abfragen der Erweiterungen und das Inventar.
fn switch_session(spec: &JobSpec, known: &KnownHosts, steps: &[DialogStep], o: &CliOptions, limit: Duration) -> Result<CliOutput, String> {
    let ip: std::net::Ipv4Addr = spec.host.trim().parse().map_err(|_| format!("{}: keine IPv4-Adresse", spec.host))?;
    let cred = Cred { name: "", payload: spec.credentials.clone() };
    let cmds: Vec<&str> = steps.iter().map(|s| s.command.as_str()).collect();
    let mut log = String::new();
    let mut via = "ssh";
    if o.telnet_only {
        let user = cred.payload.username.clone();
        let pass = String::from_utf8_lossy(&cred.payload.password.0).into_owned();
        let r = match Telnet::connect(std::net::IpAddr::V4(ip), 23, Duration::from_secs(5)) {
            Ok(mut t) => run_logged(&mut t, Some((&user, &pass)), &cmds, limit, &mut log),
            Err(e) => Err(CliError::Other(e)),
        };
        return match r {
            Ok(s) => Ok(CliOutput { outputs: s.outputs, times: s.times, timed_out: s.timed_out, log, note: None, via: "telnet".into() }),
            Err(CliError::Auth) => Err(format!("Anmeldung abgelehnt\n{log}")),
            Err(CliError::Other(e)) => Err(if log.is_empty() { e } else { format!("{e}\n{log}") }),
        };
    }
    let mut r = ssh_try(ip, &cred, spec.legacy, known, &cmds, limit, &mut log);
    let msg = |r: &Result<_, CliError>| match r {
        Err(CliError::Other(e)) => e.to_lowercase(),
        _ => String::new(),
    };
    let m = msg(&r);
    let kex_failed = m.contains("key exchange") || m.contains("kex") || m.contains("no common");
    // Ältere Verfahren nur, wenn die Einstellungen sie zulassen (Cipher-Suites).
    if o.legacy_retry && !spec.legacy && kex_failed && rj_ssh::client::legacy_allowed() > 0 {
        if o.marks {
            log.push_str("\n----- erneut mit älteren SSH-Verfahren -----\n");
        }
        r = ssh_try(ip, &cred, true, known, &cmds, limit, &mut log);
    }
    // Scheitert der Schlüsselaustausch weiter (bzw. ohne Rückfall), den Weg zur Einstellung nennen.
    if rj_ssh::client::legacy_allowed() < 2 {
        let m = msg(&r);
        if m.contains("key exchange") || m.contains("kex") || m.contains("no common") {
            if let Err(CliError::Other(e)) = &mut r {
                e.push_str(&format!(" – {}", rj_ssh::client::LEGACY_HINT));
            }
        }
    }
    let m = msg(&r);
    if o.telnet && (m.contains("refused") || m.contains("verbind") || m.contains("connect")) {
        if o.marks {
            log.push_str("\n----- Telnet -----\n");
        }
        let user = cred.payload.username.clone();
        let pass = String::from_utf8_lossy(&cred.payload.password.0).into_owned();
        via = "telnet";
        r = match Telnet::connect(std::net::IpAddr::V4(ip), 23, Duration::from_secs(5)) {
            Ok(mut t) => run_logged(&mut t, Some((&user, &pass)), &cmds, limit, &mut log),
            Err(e) => Err(CliError::Other(e)),
        };
    }
    match r {
        Ok(s) => Ok(CliOutput { outputs: s.outputs, times: s.times, timed_out: s.timed_out, log, note: None, via: via.into() }),
        Err(CliError::Auth) => Err(format!("Anmeldung abgelehnt\n{log}")),
        Err(CliError::Other(e)) => Err(if log.is_empty() { e } else { format!("{e}\n{log}") }),
    }
}

/// Einfache Shell mit Eingabeaufforderung (Controller, AP): Verbindung öffnen, Schritte, trennen.
fn shell_session(spec: &JobSpec, known: KnownHosts, steps: &[DialogStep], limit: Duration) -> Result<CliOutput, String> {
    let (rt, conn, note, _, _) = crate::overview::remote::ssh_open(spec, spec.port, known)?;
    let st: Vec<crate::overview::remote::CliStep> = steps.iter().map(|s| crate::overview::remote::CliStep { command: s.command.clone(), prompt: s.prompt.clone(), answer: s.answer.clone() }).collect();
    let r = rt.block_on(crate::overview::remote::cli_session_steps(&conn, &st, limit));
    rt.block_on(conn.disconnect());
    let r = r?;
    Ok(CliOutput { timed_out: r.outputs.len() < steps.len(), outputs: r.outputs, log: r.raw, note, via: "ssh".into(), times: Vec::new() })
}
