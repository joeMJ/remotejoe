//! Host und Port aus der Adresse (URL) eines Web-Ziels.
//!
//! Bei Web-Zielen genügt die Adresse; Host und Port werden daraus abgeleitet
//! (für Tunnel, Anzeige und Prüfung).

/// Liefert Host und wirksamen Port einer `http://`- oder `https://`-Adresse
/// (Standardports 80 und 443, IPv6 in eckigen Klammern). Adressen mit Zugangsdaten
/// (`benutzer:kennwort@`) werden abgelehnt.
pub fn endpoint(url: &str) -> Option<(String, u16)> {
    let (scheme, rest) = url.trim().split_once("://")?;
    let default = match scheme.to_ascii_lowercase().as_str() {
        "https" => 443u16,
        "http" => 80u16,
        _ => return None,
    };
    let auth = rest.split(['/', '?', '#']).next().unwrap_or("");
    if auth.is_empty() || auth.contains('@') {
        return None;
    }
    let (host, port) = if let Some(r) = auth.strip_prefix('[') {
        let (h, p) = r.split_once(']')?;
        (h.to_string(), p.strip_prefix(':'))
    } else {
        match auth.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), Some(p)),
            None => (auth.to_string(), None),
        }
    };
    if host.is_empty() || host.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let port = match port {
        None => default,
        Some(p) => p.parse::<u16>().ok().filter(|p| *p > 0)?,
    };
    Some((host, port))
}

/// Adresse aus Host und Port (für Ziele, die bisher nur einen Host haben).
pub fn default_url(host: &str, port: u16) -> String {
    let h = if host.contains(':') && !host.starts_with('[') { format!("[{host}]") } else { host.to_string() };
    match port {
        443 => format!("https://{h}/"),
        80 => format!("http://{h}/"),
        _ => format!("https://{h}:{port}/"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_and_port_from_url() {
        assert_eq!(endpoint("https://192.0.2.192/"), Some(("192.0.2.192".into(), 443)));
        assert_eq!(endpoint("http://ilo.example.test"), Some(("ilo.example.test".into(), 80)));
        assert_eq!(endpoint("https://host.example.test:8443/pfad?x=1#a"), Some(("host.example.test".into(), 8443)));
        assert_eq!(endpoint("HTTPS://Host.example.test:1"), Some(("Host.example.test".into(), 1)));
        assert_eq!(endpoint("https://[2001:db8::1]:8443/"), Some(("2001:db8::1".into(), 8443)));
        assert_eq!(endpoint("https://[2001:db8::1]/"), Some(("2001:db8::1".into(), 443)));
        assert_eq!(endpoint("  https://x.example.test/  "), Some(("x.example.test".into(), 443)));
    }

    #[test]
    fn invalid_urls_are_rejected() {
        for bad in ["", "ftp://x.example.test", "x.example.test", "https://", "https://:443/", "https://u:p@x.example.test/", "https://x.example.test:0/", "https://x.example.test:99999/", "https://x y.example.test/"] {
            assert_eq!(endpoint(bad), None, "{bad}");
        }
    }

    #[test]
    fn default_url_roundtrip() {
        assert_eq!(default_url("srv.example.test", 443), "https://srv.example.test/");
        assert_eq!(default_url("srv.example.test", 8443), "https://srv.example.test:8443/");
        assert_eq!(default_url("srv.example.test", 80), "http://srv.example.test/");
        assert_eq!(default_url("2001:db8::1", 443), "https://[2001:db8::1]/");
        assert_eq!(endpoint(&default_url("2001:db8::1", 8443)), Some(("2001:db8::1".into(), 8443)));
    }
}
