//! Inventar mit Anmeldung: Linux und Aruba über SSH, Windows über PowerShell (WinRM bzw. SSH).
//! Zugangsdaten kommen aus der KDBX (`JobSpec`); unbekannte Host-Schlüssel werden abgelehnt.

use std::time::Duration;

use rj_core::{PsrpTransport, Secret};
use rj_ssh::client::{Auth, ShellEvent, SshConnection, SshParams};
use rj_ssh::known_hosts::KnownHosts;

use super::inventory::format_bytes;
use crate::patch::JobSpec;

/// Gefundene Werte: (Feld, Wert).
pub type Found = Vec<(&'static str, String)>;

/// Ein Befehl für `sh`, Abschnitte mit Markierungen; ohne root lauffähig.
const LINUX_SCRIPT: &str = "sh -c 'echo @@name; hostname; echo @@os; (. /etc/os-release && echo \"$PRETTY_NAME\") 2>/dev/null || uname -sr; \
echo @@cpu; nproc 2>/dev/null || getconf _NPROCESSORS_ONLN; echo @@mem; grep -m1 MemTotal /proc/meminfo; \
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
        f.push(("system", o));
    }
    if let Some(c) = first("cpu").filter(|c| c.parse::<u32>().is_ok()) {
        f.push(("vcpu", c));
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

/// Auswertung von `show version` einer Aruba-Instant-AP/VC. `prompt`: zuletzt gesehene Eingabeaufforderung.
pub fn parse_aruba(out: &str) -> Option<Found> {
    if !out.contains("ArubaOS") && !out.contains("Aruba Operating System") {
        return None;
    }
    let mut f: Found = Vec::new();
    // „ArubaOS (MODEL: 315), Version 8.10.0.6“
    if let Some(l) = out.lines().find(|l| l.contains("ArubaOS") && l.contains("Version")) {
        let model = l.split("MODEL:").nth(1).and_then(|r| r.split(')').next()).map(str::trim);
        let ver = l.split("Version").nth(1).map(|v| v.trim().trim_start_matches(':').trim());
        let mut sys = "Aruba Instant".to_string();
        if let Some(v) = ver.filter(|v| !v.is_empty()) {
            sys.push_str(&format!(" {v}"));
        }
        if let Some(m) = model.filter(|m| !m.is_empty()) {
            sys.push_str(&format!(" (Modell {m})"));
        }
        f.push(("system", sys));
    } else {
        f.push(("system", "Aruba Instant".into()));
    }
    // „AP uptime is 2 days 12 hours 29 minutes 34 seconds“ → „2 T 12 Std 29 Min“.
    if let Some(l) = out.lines().find(|l| l.contains("uptime is")) {
        if let Some(rest) = l.split("uptime is").nth(1) {
            let words: Vec<&str> = rest.split_whitespace().collect();
            // Wochen zählen als Tage („10 weeks 4 days“ → „74 T“).
            let (mut days, mut hours, mut mins) = (None::<u64>, None::<u64>, None::<u64>);
            for pair in words.chunks(2) {
                if let [n, unit] = pair {
                    let Ok(n) = n.parse::<u64>() else { continue };
                    match unit.trim_end_matches(',') {
                        "week" | "weeks" => days = Some(days.unwrap_or(0) + 7 * n),
                        "day" | "days" => days = Some(days.unwrap_or(0) + n),
                        "hour" | "hours" => hours = Some(n),
                        "minute" | "minutes" => mins = Some(n),
                        _ => {}
                    }
                }
            }
            let mut parts = Vec::new();
            for (v, u) in [(days, "T"), (hours, "Std"), (mins, "Min")] {
                if let Some(v) = v {
                    parts.push(format!("{v} {u}"));
                }
            }
            if !parts.is_empty() {
                f.push(("uptime", parts.join(" ")));
            }
        }
    }
    // Name aus der Eingabeaufforderung „name# “ (letzte Zeile mit '#').
    if let Some(p) = out.lines().rev().map(|l| l.trim()).find(|l| l.ends_with('#') && !l.contains(' ')) {
        let n = p.trim_end_matches('#');
        if !n.is_empty() {
            f.push(("name", n.to_string()));
        }
    }
    Some(f)
}

/// Ein AP aus der Tabelle von `show aps` (Aruba Instant, alle APs des Schwarms).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArubaAp {
    pub name: String,
    /// IP-Adresse ohne den Stern des Conductors (Masters).
    pub ip: String,
    pub conductor: bool,
    pub serial: Option<String>,
}

/// Alle APs aus `show aps`. Die Seriennummer steht in der Spalte „Serial #“; sind die Spalten verschoben, gilt wie in
/// den Aruba-Werkzeugen das erste seriennummerartige Wort nach dem AP-Typ (z. B. `515(indoor)`).
pub fn parse_aruba_aps(out: &str) -> Vec<ArubaAp> {
    let lines: Vec<&str> = out.lines().collect();
    let Some(h) = lines.iter().position(|l| l.contains("Serial #") && l.contains("IP Address")) else { return Vec::new() };
    let col = lines[h].find("Serial #").unwrap_or(usize::MAX);
    let is_serial = |s: &&str| s.len() >= 8 && s.chars().all(|c| c.is_ascii_digit() || c.is_ascii_uppercase()) && s.chars().any(|c| c.is_ascii_uppercase());
    let is_ipv4 = |s: &str| s.split('.').count() == 4 && s.split('.').all(|p| !p.is_empty() && p.len() <= 3 && p.chars().all(|c| c.is_ascii_digit()));
    lines[h + 1..]
        .iter()
        .filter_map(|row| {
            let words: Vec<&str> = row.split_whitespace().collect();
            let ip_raw = *words.get(1)?;
            let ip = ip_raw.trim_end_matches('*');
            if !is_ipv4(ip) {
                return None;
            }
            let serial = row.get(col..).and_then(|r| r.split_whitespace().next()).filter(is_serial).map(str::to_string).or_else(|| {
                let t = words.iter().position(|w| w.ends_with(')') && w.split('(').next().is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())))?;
                words[t + 1..].iter().find(|w| is_serial(w)).map(|s| s.to_string())
            });
            Some(ArubaAp { name: words[0].to_string(), ip: ip.to_string(), conductor: ip_raw.ends_with('*'), serial })
        })
        .collect()
}

/// Seriennummer des abgefragten APs aus `show aps`. Welche Zeile: 1. Name wie in der Eingabeaufforderung (der AP, auf
/// dem die Sitzung läuft), 2. die Adresse, über die verbunden wurde, 3. der Conductor (Stern) – über die virtuelle
/// Controller-IP landet man auf ihm, die Tabelle führt aber seine eigene IP.
pub fn parse_aruba_serial(out: &str, ip: &str) -> Option<String> {
    let aps = parse_aruba_aps(out);
    let prompt = out.lines().filter_map(|l| l.trim().split_once("# show aps").map(|(name, _)| name.trim().to_string())).next_back();
    prompt
        .and_then(|p| aps.iter().find(|a| a.name == p))
        .or_else(|| aps.iter().find(|a| a.ip == ip))
        .or_else(|| aps.iter().find(|a| a.conductor))
        .and_then(|a| a.serial.clone())
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
/// `prefer_windows`; liefert beides nichts, `show version` in einer Shell (Aruba).
pub fn ssh_inventory(spec: &JobSpec, port: u16, ip: Option<&str>, known: KnownHosts, prefer_windows: bool) -> Result<(Found, &'static str, Option<String>), String> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    // Unbekannter Host-Schlüssel: sofort übernehmen und melden. Geänderter: nie automatisch.
    let (ask, mut q) = tokio::sync::mpsc::unbounded_channel::<(rj_ssh::known_hosts::HostKeyQuestion, tokio::sync::oneshot::Sender<bool>)>();
    let seen: std::sync::Arc<std::sync::Mutex<Option<rj_ssh::known_hosts::HostKeyQuestion>>> = Default::default();
    let seen2 = seen.clone();
    rt.spawn(async move {
        while let Some((question, r)) = q.recv().await {
            let accept = question.status == rj_ssh::known_hosts::HostKeyStatus::Unknown;
            *seen2.lock().unwrap_or_else(|e| e.into_inner()) = Some(question);
            let _ = r.send(accept);
        }
    });
    let mut params = ssh_params(spec, port);
    let mut connected = rt.block_on(SshConnection::connect(&params, known.clone(), ask.clone()));
    // Ältere Geräte (z. B. KVM-Konsolen) bieten nur alte Verfahren an: für diese lesende Abfrage einmal mit ihnen versuchen.
    let mut legacy_used = false;
    if !params.legacy && connected.as_ref().err().is_some_and(|e| e.to_string().contains("No common")) {
        params.legacy = true;
        legacy_used = true;
        connected = rt.block_on(SshConnection::connect(&params, known.clone(), ask.clone()));
    }
    let question = seen.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let notes: Vec<String> = [
        question.as_ref().filter(|q| q.status == rj_ssh::known_hosts::HostKeyStatus::Unknown).map(|q| {
            format!("Neuer SSH-Host-Schlüssel von {}:{} übernommen: {} {}", q.host, q.port, q.algorithm, q.fingerprint)
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
    // Geräte-CLI (z. B. Aruba Instant): Shell öffnen, `show version` senden, bis zur nächsten Eingabeaufforderung lesen.
    // Aruba Instant lehnt Einzelbefehle ab und beendet danach den Kanal bzw. die Verbindung („Channel send error“);
    // scheitert die Shell auf der bisherigen Verbindung, darum einmal auf einer frischen versuchen.
    let mut text = rt.block_on(cli_show_version(&conn));
    if text.as_ref().map_or(true, |t| parse_aruba(t).is_none()) {
        rt.block_on(conn.disconnect());
        // Gleiche Regeln wie oben; den Schlüssel kennt die Ablage nach der ersten Verbindung bereits.
        match rt.block_on(SshConnection::connect(&params, known, ask)) {
            Ok(fresh) => {
                text = rt.block_on(cli_show_version(&fresh));
                rt.block_on(fresh.disconnect());
            }
            Err(e) => text = Err(e.to_string()),
        }
    } else {
        rt.block_on(conn.disconnect());
    }
    let text = text?;
    match parse_aruba(&text) {
        Some(mut f) => {
            // Seriennummer aus „show aps“: Zeile mit der Adresse des Ziels.
            // Ohne bekannte IP (Masseninventar, Rechnername als Adresse): Namen auflösen.
            let addr = ip.map(str::to_string).unwrap_or_else(|| {
                use std::net::ToSocketAddrs;
                (spec.host.as_str(), port).to_socket_addrs().ok().and_then(|mut a| a.find(|x| x.is_ipv4())).map(|a| a.ip().to_string()).unwrap_or_else(|| spec.host.clone())
            });
            if let Some(sn) = parse_aruba_serial(&text, &addr) {
                f.push(("serial", sn));
            }
            Ok((f, "SSH (show version)", note))
        }
        // Auch ohne verwertbare Antwort melden, dass ein Schlüssel übernommen wurde.
        None => Err(format!("Keine verwertbare Antwort über SSH (weder Linux, Windows-PowerShell noch Aruba){}", note.map(|n| format!(". {n}")).unwrap_or_default())),
    }
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

/// Interaktive Shell: auf die Eingabeaufforderung warten, `no paging`, `show version` und `show aps` senden, jeweils
/// bis zur nächsten Eingabeaufforderung lesen (wie die Aruba-Instant-Werkzeuge mit `invoke_shell`). Nur lesende
/// Befehle.
async fn cli_show_version(conn: &SshConnection) -> Result<String, String> {
    const COMMANDS: [&[u8]; 3] = [b"no paging\r\n", b"show version\r\n", b"show aps\r\n"];
    // Breites Terminal: Die Tabelle von `show aps` ist mehrere hundert Zeichen breit.
    let mut sh = conn.open_shell("vt100", 1000, 50).await.map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    // Über langsame Verbindungen (WAN) kommen Antworten erst nach Sekunden: je Befehl bis zur Eingabeaufforderung
    // lesen, begrenzt nur durch die Gesamtfrist.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    // Anzahl gesendeter Befehle; `mark`: Pufferstand beim letzten Senden.
    let mut sent = 0;
    let mut mark = 0;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        // Eingabeaufforderung: letzte Zeile wie `name#` (ohne Leerzeichen). „Serial #“ in der Kopfzeile von
        // `show aps` endet ebenfalls auf „#“ und darf das Lesen nicht vorzeitig beenden.
        let ended_with_prompt = |b: &[u8], from: usize| {
            let text = String::from_utf8_lossy(&b[from.min(b.len())..]);
            let last = text.trim_end().rsplit(['\n', '\r']).next().unwrap_or("").trim_start();
            last.ends_with('#') && last.len() > 1 && !last.contains(char::is_whitespace)
        };
        // Ohne gesendeten Befehl kurz warten (Geräte ohne erkennbare Eingabeaufforderung), danach bis zur Frist.
        let wait = if sent == 0 { left.min(Duration::from_secs(3)) } else { left };
        match tokio::time::timeout(wait, sh.events.recv()).await {
            Ok(Some(ShellEvent::Data(d))) => {
                buf.extend_from_slice(&d);
                if ended_with_prompt(&buf, mark) && buf.len() > mark {
                    if sent == COMMANDS.len() {
                        break;
                    }
                    sh.write(COMMANDS[sent]).await.map_err(|e| e.to_string())?;
                    sent += 1;
                    mark = buf.len();
                }
            }
            Ok(Some(_)) | Ok(None) => break,
            // Ruhe ohne erkannte Eingabeaufforderung (anderes Gerät): nur noch „show version“ senden.
            Err(_) if sent == 0 => {
                sh.write(COMMANDS[1]).await.map_err(|e| e.to_string())?;
                sent = COMMANDS.len();
                mark = buf.len();
            }
            Err(_) => break,
        }
    }
    sh.close().await;
    let text = String::from_utf8_lossy(&buf).into_owned();
    // Diagnose: Rohantwort der Aruba-Sitzung anhängen (nur mit RJ_ARUBA_DUMP=<Datei>; enthält keine Kennwörter).
    if let Some(path) = std::env::var_os("RJ_ARUBA_DUMP") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "===== {} gesendet: {sent} Befehle, {} Bytes =====\n{text}", crate::overview::inventory::now(), buf.len());
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn aruba_uptime_and_serial_from_show_version_and_show_aps() {
        let out = "94:64:24:cc:9b:12# show version\r\nArubaOS (MODEL: 515), Version 8.10.0.23 LSR\r\n\
                   AP uptime is 2 days 12 hours 29 minutes 34 seconds\r\n94:64:24:cc:9b:12# show aps\r\n\
                   2 Access Points\r\n---------------\r\n\
                   Name               IP Address     Mode    Clients  Type  Serial #    Uptime\r\n\
                   ----               ----------     ----    -------  ----  --------    ------\r\n\
                   94:64:24:cc:9b:12  192.0.2.1*  access  0        515   CNQXK9X123  2d:12h:29m\r\n\
                   ap-kueche          192.0.2.1   access  3        505   CNQXK9X999  1d:01h:02m\r\n94:64:24:cc:9b:12#";
        let f = parse_aruba(out).unwrap();
        assert!(f.contains(&("uptime", "2 T 12 Std 29 Min".to_string())), "{f:?}");
        let w = parse_aruba("ArubaOS (MODEL: 555), Version 8.10.0.23 LSR\nAP uptime is 10 weeks 4 days 9 hours 45 minutes 14 seconds\nap#").unwrap();
        assert!(w.contains(&("uptime", "74 T 9 Std 45 Min".to_string())), "Wochen als Tage: {w:?}");
        assert_eq!(parse_aruba_serial(out, "192.0.2.1").as_deref(), Some("CNQXK9X123"));
        // Andere APs des Schwarms stehen in der Liste.
        assert_eq!(parse_aruba_aps(out).iter().find(|a| a.ip == "192.0.2.1").and_then(|a| a.serial.as_deref()), Some("CNQXK9X999"));
        assert_eq!(parse_aruba_serial("keine Tabelle", "192.0.2.1"), None);
    }

    #[test]
    fn aruba_serial_from_real_show_aps_table() {
        // Aufbau wie Aruba Instant 8.10 (`show aps`, gekürzt um hintere Spalten); zweite Zeile mit verschobenen Spalten.
        let out = "MASTER# show aps\r\n\r\n2 Access Points\r\n----------------\r\n\
                   Name                  IP Address      Mode    Spectrum  Clients  Type         IPv6 Address  Mesh Role  Zone  Serial #    radio0 Channel\r\n\
                   ----                  ----------      ----    --------  -------  ----         ------------  ---------  ----  --------    --------------\r\n\
                   MASTER                10.80.227.239*  access  enable    3        515(indoor)  --            N/A        -     CNLGKD5DWX  36\r\n\
                   48:2f:6b:ca:25:fe-lang-ap  10.80.227.225   access  enable    0        505(indoor)  --            N/A        -     CNNPKPPH0Y  40\r\nMASTER#";
        assert_eq!(parse_aruba_serial(out, "10.80.227.239").as_deref(), Some("CNLGKD5DWX"));
        assert_eq!(parse_aruba_aps(out).iter().find(|a| a.ip == "10.80.227.225").and_then(|a| a.serial.as_deref()), Some("CNNPKPPH0Y"), "verschobene Spalten");
        // Virtuelle Controller-IP: keine Zeile mit dieser Adresse, der Name aus der Eingabeaufforderung entscheidet.
        let vc = "94:64:24:cc:9b:12# show aps\r\n\r\n1 Access Point\r\n--------------\r\n\
                  Name               IP Address       Mode    Spectrum  Clients  Type         IPv6 Address  Mesh Role  Zone  Serial #    radio0 Channel\r\n\
                  ----               ----------       ----    --------  -------  ----         ------------  ---------  ----  --------    --------------\r\n\
                  94:64:24:cc:9b:12  192.0.2.1*  access  disable   0        515(indoor)  --            N/A        -     CNMQKD58ZG  52E\r\n\
                  94:64:24:cc:9b:12#";
        assert_eq!(parse_aruba_serial(vc, "192.0.2.1").as_deref(), Some("CNMQKD58ZG"));
        let aps = parse_aruba_aps(out);
        assert_eq!(aps.len(), 2);
        assert!(aps[0].conductor && !aps[1].conductor);
    }

    #[test]
    fn aruba_show_version_names_system_model_and_prompt() {
        let out = "\r\nAP-EG-01# show version\r\nAruba Operating System Software.\r\nArubaOS (MODEL: 315), Version 8.10.0.6\r\n\
                   Website: http://www.arubanetworks.com\r\nAP uptime is 3 days 4 hours\r\nAP-EG-01#";
        let f = parse_aruba(out).unwrap();
        assert_eq!(f[0], ("system", "Aruba Instant 8.10.0.6 (Modell 315)".to_string()));
        assert!(f.contains(&("name", "AP-EG-01".to_string())), "{f:?}");
        assert!(parse_aruba("Linux web01 6.8.0\n$ ").is_none());
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
