//! Linux (und andere Unix-Systeme): Nachbartabelle, Ping und Traceroute mit den Werkzeugen des Systems.
//! Gegenstück unter Windows: `sys_windows.rs` mit denselben Funktionen.

use std::net::IpAddr;
use std::process::Command;

/// MAC-Adresse im selben Netzsegment aus der Nachbartabelle des Kernels (`/proc/net/arp`, nur IPv4).
pub fn neighbor_mac(ip: IpAddr) -> Option<String> {
    let IpAddr::V4(v4) = ip else { return None };
    parse_arp(&std::fs::read_to_string("/proc/net/arp").ok()?, &v4.to_string())
}

/// Zeile `IP HW-Typ Flags MAC Maske Gerät`; nur vollständige Einträge (Flag 0x2).
fn parse_arp(table: &str, ip: &str) -> Option<String> {
    table.lines().skip(1).find_map(|l| {
        let c: Vec<&str> = l.split_whitespace().collect();
        let flags = c.get(2).and_then(|f| u32::from_str_radix(f.trim_start_matches("0x"), 16).ok()).unwrap_or(0);
        (c.first() == Some(&ip) && flags & 0x2 != 0 && c.get(3).is_some_and(|m| *m != "00:00:00:00:00:00")).then(|| c[3].to_ascii_lowercase())
    })
}

pub fn ping_command(host: &str) -> Command {
    let mut c = Command::new("ping");
    c.args(["-c", "4", "-W", "1", "--", host]);
    c
}

/// `tracepath` gehört unter Ubuntu zur Grundausstattung, `traceroute` nicht; sonst `traceroute`.
pub fn trace_command(host: &str) -> Command {
    if find_program("tracepath") || !find_program("traceroute") {
        let mut c = Command::new("tracepath");
        c.args(["-n", "-m", "30", host]);
        c
    } else {
        let mut c = Command::new("traceroute");
        c.args(["-n", "-w", "1", "-q", "1", "-m", "30", host]);
        c
    }
}

fn find_program(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
}

pub fn decode_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arp_table_only_complete_entries() {
        let t = "IP address       HW type     Flags       HW address            Mask     Device\n\
                 192.0.2.10       0x1         0x2         AA:bb:cc:00:11:22     *        eth0\n\
                 192.0.2.11       0x1         0x0         00:00:00:00:00:00     *        eth0\n";
        assert_eq!(parse_arp(t, "192.0.2.10").as_deref(), Some("aa:bb:cc:00:11:22"));
        assert_eq!(parse_arp(t, "192.0.2.11"), None);
        assert_eq!(parse_arp(t, "192.0.2.12"), None);
    }

    #[test]
    fn commands_name_the_host_last() {
        let p = ping_command("192.0.2.1");
        assert_eq!(p.get_args().last().and_then(|a| a.to_str()), Some("192.0.2.1"));
        let t = trace_command("192.0.2.1");
        assert_eq!(t.get_args().last().and_then(|a| a.to_str()), Some("192.0.2.1"));
    }
}
