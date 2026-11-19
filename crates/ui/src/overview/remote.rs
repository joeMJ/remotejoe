//! Inventar mit Anmeldung: Linux über SSH, Windows über PowerShell (WinRM bzw. SSH).
//! Zugangsdaten kommen aus der KDBX (`JobSpec`); unbekannte Host-Schlüssel werden abgelehnt.

use std::time::Duration;

use rj_core::{PsrpTransport, Secret};
use rj_ssh::client::{Auth, ShellEvent, SshConnection, SshParams};
use rj_ssh::known_hosts::KnownHosts;

use super::inventory::format_bytes;
use crate::patch::JobSpec;

// Inventar eines Moduls.
#[cfg(feature = "aruba")]
#[path = "../modules/aruba/apinventory.rs"]
mod aruba_inv;
#[cfg(feature = "aruba")]
pub use aruba_inv::{parse_aruba, parse_aruba_aps, parse_aruba_serial, ArubaAp};

/// Gefundene Werte: (Feld, Wert).
pub type Found = Vec<(&'static str, String)>;

/// Ein Befehl für `sh`, Abschnitte mit Markierungen; ohne root lauffähig.
const LINUX_SCRIPT: &str = "sh -c 'echo @@name; hostname; echo @@os; (. /etc/os-release && echo \"$PRETTY_NAME\") 2>/dev/null || uname -sr; \
echo @@osver; cat /etc/debian_version /etc/alpine-release 2>/dev/null; \
echo @@cpu; nproc 2>/dev/null || getconf _NPROCESSORS_ONLN; echo @@mem; grep -m1 MemTotal /proc/meminfo; \
echo @@uptime; cat /proc/uptime 2>/dev/null; \
echo @@ip; ip -o -4 addr show scope global 2>/dev/null; echo @@link; ip -o link 2>/dev/null; echo @@end'";

fn sections(out: &str) -> std::collections::HashMap<String, Vec<String>> {
    let mut m = std::collections::HashMap::new();
    let mut cur: Option<String> = None;
    for l in out.lines() {
        let l = l.trim_end_matches('\r');
        if let Some(k) = l.strip_prefix("@@") {
            cur = Some(k.trim().to_string());
            continue;
        }
        if let Some(k) = &cur {
            if !l.trim().is_empty() {
                m.entry(k.clone()).or_insert_with(Vec::new).push(l.trim().to_string());
            }
        }
    }
    m
}

/// Auswertung der Linux-Ausgabe. `ip`: Adresse, unter der das Ziel erreicht wurde (wählt Schnittstelle und MAC).
/// Genaue Version in die Systemzeile: `PRETTY_NAME` nennt bei Debian nur die Hauptversion („Debian GNU/Linux 13
/// (trixie)“), bei Alpine Haupt- und Nebenversion („Alpine Linux v3.16“); `/etc/debian_version` bzw.
/// `/etc/alpine-release` haben die volle („13.1“, „3.16.9“). Nur ersetzt, wenn die volle Version die kurze fortsetzt
/// (Ubuntu hat in `/etc/debian_version` etwa „trixie/sid“).
fn precise_os(os: &str, full: Option<&str>) -> String {
    let Some(full) = full.map(str::trim).filter(|v| v.contains('.') && v.chars().all(|c| c.is_ascii_digit() || c == '.')) else {
        return os.to_string();
    };
    if !(os.starts_with("Debian") || os.starts_with("Alpine")) {
        return os.to_string();
    }
    let words: Vec<String> = os
        .split(' ')
        .map(|w| {
            let (prefix, num) = w.strip_prefix('v').map_or(("", w), |n| ("v", n));
            if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit() || c == '.') && full.starts_with(&format!("{num}.")) {
                format!("{prefix}{full}")
            } else {
                w.to_string()
            }
        })
        .collect();
    words.join(" ")
}

/// Laufzeit kurz: Tage nur ab einem Tag, Stunden ab einer Stunde („2 T 0 Std 5 Min“, „3 Std 56 Min“, „4 Min“).
fn uptime_text(secs: u64) -> String {
    let (days, hours, mins) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    if days > 0 {
        format!("{days} T {hours} Std {mins} Min")
    } else if hours > 0 {
        format!("{hours} Std {mins} Min")
    } else {
        format!("{mins} Min")
    }
}

pub fn parse_linux(out: &str, ip: Option<&str>) -> Option<Found> {
    let s = sections(out);
    if !s.contains_key("end") && !out.contains("@@end") {
        return None;
    }
    let first = |k: &str| s.get(k).and_then(|v| v.first()).cloned();
    let mut f: Found = Vec::new();
    if let Some(n) = first("name") {
        f.push(("name", n));
    }
    if let Some(o) = first("os") {
        f.push(("system", precise_os(&o, first("osver").as_deref())));
    }
    if let Some(c) = first("cpu").filter(|c| c.parse::<u32>().is_ok()) {
        f.push(("vcpu", c));
    }
    // „/proc/uptime“: Sekunden seit dem Start („123456.78 …“) → „1 T 10 Std 17 Min“.
    if let Some(secs) = first("uptime").and_then(|u| u.split_whitespace().next().and_then(|v| v.parse::<f64>().ok())) {
        f.push(("uptime", uptime_text(secs as u64)));
    }
    if let Some(kb) = first("mem").and_then(|m| m.split_whitespace().nth(1).and_then(|v| v.parse::<u64>().ok())) {
        f.push(("memory", format_bytes(kb * 1024)));
    }
    // „2: eth0    inet 192.0.2.5/24 brd …“
    let addrs: Vec<(String, String)> = s
        .get("ip")
        .into_iter()
        .flatten()
        .filter_map(|l| {
            let c: Vec<&str> = l.split_whitespace().collect();
            let i = c.iter().position(|w| *w == "inet")?;
            Some((c.get(1)?.trim_end_matches(':').to_string(), c.get(i + 1)?.split('/').next()?.to_string()))
        })
        .collect();
    // „2: eth0: <…> … link/ether aa:bb:… brd …“
    let macs: Vec<(String, String)> = s
        .get("link")
        .into_iter()
        .flatten()
        .filter_map(|l| {
            let c: Vec<&str> = l.split_whitespace().collect();
            let i = c.iter().position(|w| *w == "link/ether")?;
            Some((c.get(1)?.trim_end_matches(':').split('@').next()?.to_string(), c.get(i + 1)?.to_ascii_lowercase()))
        })
        .collect();
    let chosen = ip.and_then(|ip| addrs.iter().find(|(_, a)| a == ip)).or(addrs.first());
    if let Some((dev, a)) = chosen {
        f.push(("ip", a.clone()));
        if let Some((_, m)) = macs.iter().find(|(d, _)| d == dev) {
            f.push(("mac", m.clone()));
        }
    } else if let Some((_, m)) = macs.first() {
        f.push(("mac", m.clone()));
    }
    Some(f)
}

/// Auswertung der JSON-Zeile des Windows-Skripts (Felder nach Schlüsseln).
pub fn parse_windows(line: &str, ip: Option<&str>) -> Option<Found> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::trim).filter(|x| !x.is_empty()).map(str::to_string);
    let list = |k: &str| -> Vec<String> {
        match v.get(k) {
            Some(serde_json::Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
            Some(serde_json::Value::String(x)) => vec![x.clone()],
            _ => Vec::new(),
        }
    };
    let mut f: Found = Vec::new();
    if let Some(n) = s("name") {
        f.push(("name", n));
    }
    if let Some(o) = s("os") {
        f.push(("system", o));
    }
    if let Some(c) = v.get("vcpu").and_then(|x| x.as_u64()).filter(|c| *c > 0) {
        f.push(("vcpu", c.to_string()));
    }
    if let Some(m) = v.get("mem_bytes").and_then(|x| x.as_u64()).filter(|m| *m > 0) {
        f.push(("memory", format_bytes(m)));
    }
    let ips = list("ips");
    let v4: Vec<&String> = ips.iter().filter(|a| a.parse::<std::net::Ipv4Addr>().is_ok()).collect();
    if let Some(a) = ip.and_then(|ip| ips.iter().find(|a| *a == ip)).or(v4.first().copied()).or(ips.first()) {
        f.push(("ip", a.clone()));
    }
    let macs = list("macs");
    if macs.len() == 1 {
        f.push(("mac", macs[0].clone()));
    }
    (!f.is_empty()).then_some(f)
}

fn ssh_params(spec: &JobSpec, port: u16) -> SshParams {
    let pw = Secret::from_bytes(spec.credentials.password.0.clone());
    let mut auth = Vec::new();
    if let Some(k) = &spec.credentials.private_key {
        auth.push(Auth::Key { pem: Secret::from_bytes(k.0.clone()), passphrase: None });
    }
    auth.push(Auth::Password(pw.clone()));
    auth.push(Auth::KeyboardInteractive(pw));
    SshParams { host: spec.host.clone(), port, user: spec.credentials.username.clone(), auth, connect_timeout: Duration::from_secs(10), keepalive: Duration::from_secs(30), legacy: spec.legacy }
}

/// Inventarskript der Windows-Ziele als Befehl für OpenSSH unter Windows (Standard-Shell cmd oder PowerShell).
fn windows_command() -> String {
    format!("powershell -NoProfile -NonInteractive -EncodedCommand {}", rj_psrp::encoded(rj_psrp::inventory::windows_inventory_script()))
}

/// SSH mit den Zugangsdaten aus der KDBX: Linux-Befehl und Windows-PowerShell (über OpenSSH), Reihenfolge nach
/// `prefer_windows`; liefert beides nichts, `show version` in einer Shell.
pub fn ssh_inventory(spec: &JobSpec, port: u16, ip: Option<&str>, known: KnownHosts, prefer_windows: bool) -> Result<(Found, &'static str, Option<String>), String> {
    let (rt, conn, note, params, ask) = ssh_open(spec, port, known.clone())?;
    ssh_inventory_on(spec, port, rt, conn, note, params, ask, known, ip, prefer_windows)
}

type AskSender = tokio::sync::mpsc::UnboundedSender<(rj_ssh::known_hosts::HostKeyQuestion, tokio::sync::oneshot::Sender<rj_ssh::client::HostKeyAnswer>)>;

/// SSH-Verbindung für lesende Abfragen mit den Zugangsdaten des Ziels: unbekannter Host-Schlüssel wird übernommen und
/// gemeldet, ein geänderter nie; ältere Verfahren nur, wenn das Gerät nichts anderes anbietet. Liefert die Laufzeit,
/// die Verbindung und einen Hinweis (übernommener Schlüssel, ältere Verfahren).
#[allow(clippy::type_complexity)]
pub fn ssh_open(spec: &JobSpec, port: u16, known: KnownHosts) -> Result<(tokio::runtime::Runtime, SshConnection, Option<String>, rj_ssh::client::SshParams, AskSender), String> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    // Unbekannter Host-Schlüssel: sofort übernehmen und melden. Geänderter: nie automatisch – außer mit „Bei
    // Schlüsselfehlern trotzdem fortfahren“, dann nur für diese Verbindung (nicht gespeichert) und gemeldet.
    let (ask, mut q) = tokio::sync::mpsc::unbounded_channel::<(rj_ssh::known_hosts::HostKeyQuestion, tokio::sync::oneshot::Sender<rj_ssh::client::HostKeyAnswer>)>();
    let seen: std::sync::Arc<std::sync::Mutex<Option<rj_ssh::known_hosts::HostKeyQuestion>>> = Default::default();
    let seen2 = seen.clone();
    rt.spawn(async move {
        while let Some((question, r)) = q.recv().await {
            let answer = if question.status == rj_ssh::known_hosts::HostKeyStatus::Unknown {
                rj_ssh::client::HostKeyAnswer::Accept
            } else if crate::keyerrors::accept() {
                crate::keyerrors::warn(crate::keyerrors::text("hostkey", &question.host, question.port, &question.algorithm, &question.fingerprint, true));
                rj_ssh::client::HostKeyAnswer::AcceptOnce
            } else {
                rj_ssh::client::HostKeyAnswer::Reject
            };
            *seen2.lock().unwrap_or_else(|e| e.into_inner()) = Some(question);
            let _ = r.send(answer);
        }
    });
    let mut params = ssh_params(spec, port);
    let mut connected = rt.block_on(SshConnection::connect(&params, known.clone(), ask.clone()));
    // Ältere Geräte (z. B. KVM-Konsolen) bieten nur alte Verfahren an: für diese lesende Abfrage einmal mit ihnen versuchen.
    let mut legacy_used = false;
    // Manche Switches melden statt „No common …“ oft nur „Key exchange init failed“.
    if !params.legacy && connected.as_ref().err().is_some_and(|e| {
        let e = e.to_string().to_lowercase();
        e.contains("no common") || e.contains("key exchange") || e.contains("kex")
    }) {
        params.legacy = true;
        legacy_used = true;
        connected = rt.block_on(SshConnection::connect(&params, known.clone(), ask.clone()));
    }
    let question = seen.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let notes: Vec<String> = [
        question.as_ref().filter(|q| q.status == rj_ssh::known_hosts::HostKeyStatus::Unknown).map(|q| {
            format!("Neuer SSH-Host-Schlüssel von {}:{} übernommen: {} {}", q.host, q.port, q.algorithm, q.fingerprint)
        }),
        question.as_ref().filter(|q| q.status != rj_ssh::known_hosts::HostKeyStatus::Unknown && connected.is_ok()).map(|q| {
            format!("Geänderter SSH-Host-Schlüssel von {}:{} nur für diese Verbindung angenommen: {} {}", q.host, q.port, q.algorithm, q.fingerprint)
        }),
        legacy_used.then(|| "Ältere SSH-Verfahren verwendet – das Gerät bietet nur diese an".to_string()),
    ]
    .into_iter()
    .flatten()
    .collect();
    let note = (!notes.is_empty()).then(|| notes.join(". "));
    let conn = connected.map_err(|e| {
        let msg = match (e, question.as_ref()) {
            (rj_ssh::client::SshError::HostKeyRejected, Some(q)) => format!(
                "Host-Schlüssel von {}:{} hat sich geändert ({} {}) – nicht übernommen. Bitte beim direkten Verbinden oder über „Schlüssel und Zertifikate prüfen“ klären.",
                q.host, q.port, q.algorithm, q.fingerprint
            ),
            (rj_ssh::client::SshError::HostKeyRejected, None) => "Host-Schlüssel abgelehnt".to_string(),
            (e, _) => e.to_string(),
        };
        // Auch beim Scheitern danach (z. B. Anmeldung) melden, dass ein Schlüssel übernommen wurde.
        match &note {
            Some(n) => format!("{msg}. {n}"),
            None => msg,
        }
    })?;
    Ok((rt, conn, note, params, ask))
}

#[allow(clippy::too_many_arguments)]
fn ssh_inventory_on(
    spec: &JobSpec,
    port: u16,
    rt: tokio::runtime::Runtime,
    conn: SshConnection,
    note: Option<String>,
    params: rj_ssh::client::SshParams,
    ask: AskSender,
    known: KnownHosts,
    ip: Option<&str>,
    prefer_windows: bool,
) -> Result<(Found, &'static str, Option<String>), String> {
    let known_switch = known.clone();
    // Windows mit OpenSSH zuerst, wenn es danach aussieht; sonst Linux zuerst.
    let order: [bool; 2] = if prefer_windows { [true, false] } else { [false, true] };
    for windows in order {
        let cmd = if windows { windows_command() } else { LINUX_SCRIPT.to_string() };
        let r = rt.block_on(async { tokio::time::timeout(Duration::from_secs(20), conn.exec(&cmd)).await });
        if let Ok(Ok((out, _, _))) = &r {
            let text = String::from_utf8_lossy(out);
            let f = if windows { text.lines().find_map(|l| parse_windows(l, ip)) } else { parse_linux(&text, ip) };
            if let Some(f) = f {
                rt.block_on(conn.disconnect());
                return Ok((f, if windows { "SSH (PowerShell)" } else { "SSH" }, note));
            }
        }
    }
    #[cfg(feature = "aruba")]
    return aruba_inv::device_cli_inventory(spec, port, &rt, conn, params, known, ask, ip, note, &known_switch);
    #[cfg(not(feature = "aruba"))]
    {
        let _ = (spec, port, params, known, ask, known_switch);
        rt.block_on(conn.disconnect());
        Err(format!("Keine verwertbare Antwort über SSH (weder Linux noch Windows-PowerShell){}", note.map(|n| format!(". {n}")).unwrap_or_default()))
    }
}

/// Switch über seine CLI abfragen (gemeinsame Funktion `switches::switch_system`); Fehler, wenn es keiner ist.
#[cfg(not(feature = "standorte"))]
pub(crate) fn switch_inventory(_spec: &JobSpec, _port: u16, _known: &KnownHosts) -> Result<Found, String> {
    Err("Keine verwertbare Antwort über SSH".into())
}

#[cfg(feature = "standorte")]
pub(crate) fn switch_inventory(spec: &JobSpec, port: u16, known: &KnownHosts) -> Result<Found, String> {
    use std::net::ToSocketAddrs;
    let ip = match spec.host.trim().parse::<std::net::Ipv4Addr>() {
        Ok(ip) => ip,
        Err(_) => (spec.host.as_str(), port)
            .to_socket_addrs()
            .ok()
            .and_then(|mut a| a.find_map(|a| match a.ip() {
                std::net::IpAddr::V4(v) => Some(v),
                _ => None,
            }))
            .ok_or_else(|| format!("{}: keine IPv4-Adresse", spec.host))?,
    };
    let c = crate::switches::Cred { name: "Ziel", payload: spec.credentials.clone() };
    let f = crate::switches::switch_found(&crate::switches::switch_system(ip, &c, known)?);
    if f.is_empty() { Err("Keine Angaben in „show system“".into()) } else { Ok(f) }
}

/// Windows über PowerShell-Remoting mit dem Transport des Ziels.
pub fn ps_inventory(spec: &JobSpec, ip: Option<&str>) -> Result<(Found, &'static str, Option<String>), String> {
    let transport = PsrpTransport::effective(spec.transport.clone(), None);
    let t = rj_psrp::PsTarget { host: spec.host.clone(), username: spec.credentials.username.clone(), domain: spec.credentials.domain.clone(), transport };
    let mut s = rj_psrp::PsSession::start(&t, &Secret::from_bytes(spec.credentials.password.0.clone())).map_err(|e| rj_psrp::explain(e.to_string()))?;
    let mut lines = Vec::new();
    let mut errors = Vec::new();
    s.run(rj_psrp::inventory::windows_inventory_script(), |r| {
        if r.stream == "error" {
            errors.push(r.text.clone());
        } else {
            lines.push(r.text.clone());
        }
    })
    .map_err(|e| rj_psrp::explain(e.to_string()))?;
    lines.iter().find_map(|l| parse_windows(l, ip)).map(|f| (f, "PowerShell", None)).ok_or_else(|| errors.first().cloned().unwrap_or_else(|| "Keine Antwort des Inventarskripts".into()))
}

/// Ergebnis einer CLI-Sitzung: Rohtext (mit Eingabeaufforderungen und Echos) und je gesendetem Befehl dessen Ausgabe
/// ohne Echo und Eingabeaufforderung.
pub struct CliResult {
    pub raw: String,
    pub outputs: Vec<String>,
}

/// Letzte nicht leere Zeile ist eine Eingabeaufforderung wie `name#` (ohne Leerzeichen; „Serial #“ in der Kopfzeile
/// von `show aps` endet ebenfalls auf „#“, enthält aber ein Leerzeichen).
fn is_prompt(line: &str) -> bool {
    let l = line.trim();
    l.ends_with('#') && l.len() > 1 && !l.contains(char::is_whitespace)
}

fn seg_lines(seg: &str) -> Vec<&str> {
    seg.split('\n').map(|l| l.trim_end_matches('\r')).collect()
}

/// Ausgabe eines Befehls fertig? Erst das Echo des Befehls, danach eine Eingabeaufforderung als letzte Zeile.
/// Eine Eingabeaufforderung vor dem Echo (z. B. eine verirrte leere Zeile) zählt nicht.
fn command_done(seg: &str, cmd: &str) -> Option<String> {
    let lines = seg_lines(seg);
    let echo = lines.iter().position(|l| l.contains(cmd))?;
    let last = lines.iter().rposition(|l| !l.trim().is_empty())?;
    (last > echo && is_prompt(lines[last])).then(|| lines[echo + 1..last].join("\n"))
}

/// Interaktive Shell: auf die Eingabeaufforderung warten, dann
/// je Befehl senden (nur `\n` – `\r\n` wertet die CLI als zwei Eingaben) und lesen, bis nach seinem Echo die nächste
/// Eingabeaufforderung kommt. Breites Terminal, weil die Tabelle von `show aps` mehrere hundert Zeichen breit ist.
/// Nur lesende Befehle; Gesamtfrist 30 s.
pub async fn cli_session(conn: &SshConnection, cmds: &[&str]) -> Result<CliResult, String> {
    let mut sh = conn.open_shell("vt100", 1000, 50).await.map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    let mut outputs = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    // Ohne erkennbare Eingabeaufforderung nach 3 s trotzdem senden (Gerät ohne Prompt-Zeile).
    let mut waiting = true;
    let first_wait = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut idx = 0;
    let mut mark = 0;
    while idx < cmds.len() {
        let now = tokio::time::Instant::now();
        let until = if waiting { first_wait.min(deadline) } else { deadline };
        if now >= deadline {
            break;
        }
        match tokio::time::timeout(until.saturating_duration_since(now), sh.events.recv()).await {
            Ok(Some(ShellEvent::Data(d))) => {
                buf.extend_from_slice(&d);
                let text = String::from_utf8_lossy(&buf[mark..]).into_owned();
                if waiting {
                    if seg_lines(&text).iter().rev().find(|l| !l.trim().is_empty()).is_some_and(|l| is_prompt(l)) {
                        waiting = false;
                        sh.write(format!("{}\n", cmds[0]).as_bytes()).await.map_err(|e| e.to_string())?;
                        mark = buf.len();
                    }
                } else if let Some(out) = command_done(&text, cmds[idx]) {
                    outputs.push(out);
                    idx += 1;
                    if idx < cmds.len() {
                        sh.write(format!("{}\n", cmds[idx]).as_bytes()).await.map_err(|e| e.to_string())?;
                    }
                    mark = buf.len();
                }
            }
            Ok(Some(_)) | Ok(None) => break,
            Err(_) if waiting => {
                waiting = false;
                sh.write(format!("{}\n", cmds[0]).as_bytes()).await.map_err(|e| e.to_string())?;
                mark = buf.len();
            }
            Err(_) => break,
        }
    }
    sh.close().await;
    // Unvollständig gelesene Befehle: was da ist, ohne Echo.
    if idx < cmds.len() && !waiting {
        let text = String::from_utf8_lossy(&buf[mark..]).into_owned();
        let lines = seg_lines(&text);
        let start = lines.iter().position(|l| l.contains(cmds[idx])).map_or(0, |i| i + 1);
        outputs.push(lines[start..].join("\n"));
    }
    let raw = String::from_utf8_lossy(&buf).into_owned();
    // Diagnose: Rohantwort der Sitzung anhängen (nur mit RJ_ARUBA_DUMP=<Datei>; enthält keine Kennwörter).
    if let Some(path) = std::env::var_os("RJ_ARUBA_DUMP") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "===== {} {} von {} Befehlen fertig, {} Bytes =====\n{raw}", crate::overview::inventory::now(), outputs.len().min(idx), cmds.len(), buf.len());
        }
    }
    Ok(CliResult { raw, outputs })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_uptime_from_proc() {
        assert_eq!(uptime_text(240), "4 Min");
        assert_eq!(uptime_text(3 * 3600 + 56 * 60 + 26), "3 Std 56 Min");
        assert_eq!(uptime_text(2 * 86_400 + 5 * 60), "2 T 0 Std 5 Min");
        let out = "@@name\nct\n@@os\nAlpine Linux v3.16\n@@uptime\n123456.78 98765.43\n@@end\n";
        let f = parse_linux(out, None).unwrap();
        assert!(f.contains(&("uptime", "1 T 10 Std 17 Min".to_string())), "{f:?}");
        // Ohne /proc/uptime (leerer Abschnitt): keine Laufzeit.
        let f = parse_linux("@@name\nct\n@@uptime\n@@end\n", None).unwrap();
        assert!(!f.iter().any(|(k, _)| *k == "uptime"));
    }

    #[test]
    fn precise_version_for_debian_and_alpine() {
        assert_eq!(precise_os("Debian GNU/Linux 13 (trixie)", Some("13.1")), "Debian GNU/Linux 13.1 (trixie)");
        assert_eq!(precise_os("Alpine Linux v3.16", Some("3.16.9")), "Alpine Linux v3.16.9");
        assert_eq!(precise_os("Ubuntu 24.04.4 LTS", Some("trixie/sid")), "Ubuntu 24.04.4 LTS");
        assert_eq!(precise_os("Debian GNU/Linux 13 (trixie)", Some("12.9")), "Debian GNU/Linux 13 (trixie)");
        assert_eq!(precise_os("Debian GNU/Linux 13 (trixie)", None), "Debian GNU/Linux 13 (trixie)");
        let out = "@@name\nct\n@@os\nDebian GNU/Linux 13 (trixie)\n@@osver\n13.1\n@@cpu\n2\n@@end\n";
        let f = parse_linux(out, None).unwrap();
        assert!(f.contains(&("system", "Debian GNU/Linux 13.1 (trixie)".to_string())), "{f:?}");
    }

    #[test]
    fn linux_output_is_read_and_the_reached_interface_wins() {
        let out = "@@name\nweb01\n@@os\nUbuntu 24.04.1 LTS\n@@cpu\n4\n@@mem\nMemTotal:        8130000 kB\n@@ip\n\
                   2: eth0    inet 192.0.2.5/24 brd 192.0.2.255 scope global eth0\\       valid_lft forever\n\
                   3: eth1    inet 198.51.100.7/24 brd 198.51.100.255 scope global eth1\n@@link\n\
                   1: lo: <LOOPBACK,UP> mtu 65536 qdisc noqueue state UNKNOWN mode DEFAULT group default qlen 1000\\    link/loopback 00:00:00:00:00:00 brd 00:00:00:00:00:00\n\
                   2: eth0: <BROADCAST,UP> mtu 1500 qdisc fq state UP mode DEFAULT\\    link/ether 52:54:00:AA:00:01 brd ff:ff:ff:ff:ff:ff\n\
                   3: eth1@if7: <BROADCAST,UP> mtu 1500\\    link/ether 52:54:00:aa:00:02 brd ff:ff:ff:ff:ff:ff\n@@end\n";
        let f = parse_linux(out, Some("198.51.100.7")).unwrap();
        let get = |k| f.iter().find(|(n, _)| *n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("name"), Some("web01"));
        assert_eq!(get("system"), Some("Ubuntu 24.04.1 LTS"));
        assert_eq!(get("vcpu"), Some("4"));
        assert_eq!(get("memory"), Some("7,8 GiB"));
        assert_eq!((get("ip"), get("mac")), (Some("198.51.100.7"), Some("52:54:00:aa:00:02")));
        let f = parse_linux(out, None).unwrap();
        assert_eq!(f.iter().find(|(n, _)| *n == "mac").map(|(_, v)| v.as_str()), Some("52:54:00:aa:00:01"));
        assert!(parse_linux("Invalid command\n", None).is_none(), "keine Linux-Shell");
    }

    #[test]
    fn windows_command_fits_on_a_command_line() {
        let c = windows_command();
        assert!(c.starts_with("powershell -NoProfile -NonInteractive -EncodedCommand ") && c.len() < 8000, "{}", c.len());
    }

    #[test]
    fn cli_command_ends_only_with_prompt_after_its_echo() {
        // Ablauf wie am Test-AP: „no paging“ ist dort ein Parse error; danach kam eine leere Eingabeaufforderung.
        let p = "02:00:5e:00:53:12#";
        let seg = format!("no paging\r\n                       ^\r\n% Parse error\r\n{p} ");
        assert_eq!(command_done(&seg, "no paging").as_deref(), Some("                       ^\n% Parse error"));
        // Verirrte leere Eingabeaufforderung vor dem Echo: noch nicht fertig.
        let early = format!("\r\n{p} ");
        assert!(command_done(&early, "show version").is_none());
        let partial = format!("\r\n{p} show version\r\nArubaOS (MODEL: 515), Version 8.10.0.23 LSR\r\n");
        assert!(command_done(&partial, "show version").is_none(), "Ausgabe noch ohne Eingabeaufforderung");
        let full = format!("{partial}AP uptime is 3 hours 56 minutes 26 seconds\r\n{p}");
        let out = command_done(&full, "show version").unwrap();
        assert!(out.contains("MODEL: 515") && out.contains("uptime") && !out.contains('#'), "{out}");
        // „Serial #“ in der Kopfzeile von show aps ist keine Eingabeaufforderung.
        assert!(command_done("show aps\r\nName  IP Address  Serial #\r\n", "show aps").is_none());
        assert!(!is_prompt("Name  IP Address  Serial #") && is_prompt("ap-flur-1#") && !is_prompt("#"));
    }

    #[test]
    fn windows_json_in_any_key_order() {
        let l = r#"{"macs":["08:00:27:12:34:56"],"name":"PC7","vcpu":12,"mem_bytes":34359267328,"ips":["fe80::1","192.0.2.9"],"os":"Microsoft Windows 10 Pro 10.0.19045"}"#;
        let f = parse_windows(l, None).unwrap();
        let get = |k| f.iter().find(|(n, _)| *n == k).map(|(_, v)| v.as_str());
        assert_eq!((get("name"), get("vcpu"), get("memory")), (Some("PC7"), Some("12"), Some("32,0 GiB")));
        assert_eq!((get("ip"), get("mac")), (Some("192.0.2.9"), Some("08:00:27:12:34:56")));
        assert_eq!(get("system"), Some("Microsoft Windows 10 Pro 10.0.19045"));
        // Einzelwerte statt Listen (ConvertTo-Json bei einem Element) und mehrere MACs ohne Zuordnung.
        let f = parse_windows(r#"{"name":"X","ips":"192.0.2.3","macs":["a","b"]}"#, None).unwrap();
        assert!(f.contains(&("ip", "192.0.2.3".into())) && !f.iter().any(|(n, _)| *n == "mac"));
        assert!(parse_windows("kein json", None).is_none());
    }
}
