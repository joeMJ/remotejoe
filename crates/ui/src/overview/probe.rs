//! Angebotene Dienste: Namensauflösung und TCP-Prüfung bekannter Ports (ohne Anmeldung).

use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Geprüfte Ports mit Anzeigenamen.
pub const PORTS: [(u16, &str); 11] = [
    (22, "SSH"),
    (23, "Telnet"),
    (53, "DNS"),
    (80, "HTTP"),
    (443, "HTTPS"),
    (3389, "RDP"),
    (4343, "Aruba"),
    (5900, "VNC"),
    (5985, "WinRM"),
    (5986, "WinRM (TLS)"),
    (8006, "Proxmox"),
];

/// Ergebnis für einen Port.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Service {
    pub port: u16,
    pub name: String,
    pub open: bool,
}

/// Host ohne eckige Klammern (IPv6).
pub fn bare_host(host: &str) -> &str {
    host.trim().trim_start_matches('[').trim_end_matches(']')
}

/// Adresse des Ziels; IPv4 bevorzugt, sonst die erste gelieferte.
pub fn resolve(host: &str) -> Result<IpAddr, String> {
    let h = bare_host(host);
    if h.is_empty() {
        return Err("Keine Adresse am Ziel".into());
    }
    if let Ok(ip) = h.parse::<IpAddr>() {
        return Ok(ip);
    }
    let all: Vec<SocketAddr> = (h, 0).to_socket_addrs().map_err(|e| format!("Name „{h}“ nicht auflösbar: {e}"))?.collect();
    all.iter().find(|a| a.is_ipv4()).or(all.first()).map(|a| a.ip()).ok_or_else(|| format!("Name „{h}“ nicht auflösbar"))
}

/// Liste der zu prüfenden Ports: die bekannten und – falls abweichend – der Port des Ziels.
pub fn port_list(extra: Option<(u16, &str)>) -> Vec<(u16, String)> {
    let mut v: Vec<(u16, String)> = PORTS.iter().map(|(p, n)| (*p, n.to_string())).collect();
    if let Some((p, n)) = extra {
        if !v.iter().any(|(q, _)| *q == p) {
            v.push((p, n.to_string()));
        }
    }
    v
}

/// Alle Ports gleichzeitig prüfen (je ein Faden, kurze Zeitgrenze).
pub fn scan(ip: IpAddr, ports: &[(u16, String)], timeout: Duration) -> Vec<Service> {
    let handles: Vec<_> = ports
        .iter()
        .cloned()
        .map(|(port, name)| {
            std::thread::spawn(move || {
                let open = TcpStream::connect_timeout(&SocketAddr::new(ip, port), timeout).is_ok();
                Service { port, name, open }
            })
        })
        .collect();
    handles.into_iter().filter_map(|h| h.join().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_and_closed_ports_are_told_apart() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let open = l.local_addr().unwrap().port();
        // Ein gerade freigegebener Port ist mit hoher Wahrscheinlichkeit zu.
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let ip = resolve("127.0.0.1").unwrap();
        let r = scan(ip, &[(open, "eigen".into()), (closed, "zu".into())], Duration::from_millis(500));
        assert_eq!(r.iter().find(|s| s.port == open).map(|s| s.open), Some(true));
        assert_eq!(r.iter().find(|s| s.port == closed).map(|s| s.open), Some(false));
    }

    #[test]
    fn names_and_lists() {
        assert_eq!(resolve("[::1]").unwrap(), "::1".parse::<IpAddr>().unwrap());
        assert!(resolve("localhost").is_ok());
        assert!(resolve(" ").is_err());
        assert_eq!(port_list(Some((22, "SSH"))).len(), PORTS.len());
        let l = port_list(Some((2222, "SSH")));
        assert_eq!(l.last(), Some(&(2222, "SSH".to_string())));
        assert!(l.iter().any(|(p, _)| *p == 4343));
    }
}
