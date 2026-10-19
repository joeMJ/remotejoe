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
    // Name aus der Eingabeaufforderung „name# “ (letzte Zeile mit '#').
    if let Some(p) = out.lines().rev().map(|l| l.trim()).find(|l| l.ends_with('#') && !l.contains(' ')) {
        let n = p.trim_end_matches('#');
        if !n.is_empty() {
            f.push(("name", n.to_string()));
        }
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

/// SSH: erst der Linux-Befehl; liefert er nichts Brauchbares, `show version` in einer Shell (Aruba).
pub fn ssh_inventory(spec: &JobSpec, port: u16, ip: Option<&str>, known: KnownHosts) -> Result<(Found, &'static str), String> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    let (ask, mut q) = tokio::sync::mpsc::unbounded_channel::<(rj_ssh::known_hosts::HostKeyQuestion, tokio::sync::oneshot::Sender<bool>)>();
    rt.spawn(async move {
        while let Some((_, r)) = q.recv().await {
            let _ = r.send(false);
        }
    });
    let conn = rt.block_on(SshConnection::connect(&ssh_params(spec, port), known, ask)).map_err(|e| match e {
        rj_ssh::client::SshError::HostKeyRejected => "Host-Schlüssel unbekannt – bitte zuerst einmal direkt verbinden".to_string(),
        e => e.to_string(),
    })?;
    let linux = rt.block_on(async { tokio::time::timeout(Duration::from_secs(15), conn.exec(LINUX_SCRIPT)).await });
    if let Ok(Ok((out, _, _))) = &linux {
        if let Some(f) = parse_linux(&String::from_utf8_lossy(out), ip) {
            rt.block_on(conn.disconnect());
            return Ok((f, "SSH"));
        }
    }
    // Geräte-CLI (z. B. Aruba Instant): Shell öffnen, `show version` senden, bis zur nächsten Eingabeaufforderung lesen.
    let text = rt.block_on(async {
        let mut sh = conn.open_shell("vt100", 200, 50).await.map_err(|e| e.to_string())?;
        let mut buf = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
        let mut sent = false;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                break;
            }
            match tokio::time::timeout(left.min(Duration::from_millis(1500)), sh.events.recv()).await {
                Ok(Some(ShellEvent::Data(d))) => buf.extend_from_slice(&d),
                Ok(Some(_)) | Ok(None) => break,
                Err(_) if !sent => {
                    // Ruhe nach dem Anmeldetext: Befehl senden.
                    sh.write(b"show version\r\n").await.map_err(|e| e.to_string())?;
                    sent = true;
                }
                Err(_) => break,
            }
        }
        sh.close().await;
        Ok::<String, String>(String::from_utf8_lossy(&buf).into_owned())
    })?;
    rt.block_on(conn.disconnect());
    parse_aruba(&text).map(|f| (f, "SSH (show version)")).ok_or_else(|| "Keine verwertbare Antwort über SSH (weder Linux noch Aruba)".into())
}

/// Windows über PowerShell-Remoting mit dem Transport des Ziels.
pub fn ps_inventory(spec: &JobSpec, ip: Option<&str>) -> Result<(Found, &'static str), String> {
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
    lines.iter().find_map(|l| parse_windows(l, ip)).map(|f| (f, "PowerShell")).ok_or_else(|| errors.first().cloned().unwrap_or_else(|| "Keine Antwort des Inventarskripts".into()))
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
    fn aruba_show_version_names_system_model_and_prompt() {
        let out = "\r\nAP-EG-01# show version\r\nAruba Operating System Software.\r\nArubaOS (MODEL: 315), Version 8.10.0.6\r\n\
                   Website: http://www.arubanetworks.com\r\nAP uptime is 3 days 4 hours\r\nAP-EG-01#";
        let f = parse_aruba(out).unwrap();
        assert_eq!(f[0], ("system", "Aruba Instant 8.10.0.6 (Modell 315)".to_string()));
        assert_eq!(f[1], ("name", "AP-EG-01".to_string()));
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
