//! Proxy für die Update-Funktion (HTTP-Proxy mit `CONNECT`; TLS und Zertifikatsprüfung laufen weiter direkt bis
//! zum Ziel, der Proxy sieht nur Rechnername und Port). Woher der Proxy kommt:
//! - „Automatisch“: unter Windows die Systemeinstellung des Benutzers samt Konfigurationsskript (PAC) und
//!   automatischer Erkennung (WPAD) über WinHTTP; sonst bzw. ohne Ergebnis `HTTPS_PROXY`/`ALL_PROXY` mit `NO_PROXY`.
//! - „Kein Proxy“: immer direkt.
//! - „Manuell“: `Rechner:Port`.
//!
//! Eine Anmeldung am Proxy wird nicht unterstützt.

use std::sync::RwLock;

/// Einstellung des Anwenders.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ProxyMode {
    #[default]
    Auto,
    Off,
    Manual(String),
}

impl ProxyMode {
    /// Aus dem Wert der Einstellungen: leer oder „auto“, „none“, sonst `Rechner:Port`.
    pub fn from_setting(s: &str) -> ProxyMode {
        match s.trim() {
            "" | "auto" => ProxyMode::Auto,
            "none" | "off" => ProxyMode::Off,
            other => ProxyMode::Manual(other.to_string()),
        }
    }
}

static MODE: RwLock<Option<ProxyMode>> = RwLock::new(None);

/// Setzt die Einstellung für alle folgenden Abrufe.
pub fn set_mode(mode: ProxyMode) {
    *MODE.write().unwrap_or_else(|e| e.into_inner()) = Some(mode);
}

pub(crate) fn mode() -> ProxyMode {
    MODE.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

/// Prüft eine manuelle Angabe (für die Einstellungen): `Rechner:Port`, optional mit `http://`.
/// Weg einer Anfrage an `url` für Meldungen: „direkt“ oder „über Proxy <Rechner:Port>“ (nach der eingestellten Art).
pub fn describe(url: &str) -> String {
    let host = url.split("://").nth(1).unwrap_or(url).split(['/', ':']).next().unwrap_or_default();
    match for_target(&mode(), url, host) {
        Some((h, p)) => format!("über Proxy {h}:{p}"),
        None => "direkt".to_string(),
    }
}

pub fn check_manual(s: &str) -> Result<(String, u16), String> {
    parse_host_port(s).ok_or_else(|| format!("„{s}“ ist kein Proxy der Form Rechner:Port (z. B. proxy.example:8080)"))
}

/// Proxy für eine Anfrage an `host:port` (`None`: direkt). `url` nur für das Konfigurationsskript unter Windows.
pub(crate) fn for_target(mode: &ProxyMode, url: &str, host: &str) -> Option<(String, u16)> {
    match mode {
        ProxyMode::Off => None,
        ProxyMode::Manual(s) => parse_host_port(s),
        ProxyMode::Auto => {
            if matches!(host, "127.0.0.1" | "localhost" | "::1") {
                return None;
            }
            #[cfg(windows)]
            if let Some(found) = windows::system_proxy(url, host) {
                return found;
            }
            let _ = url;
            from_env(&|k| std::env::var(k).ok(), host)
        }
    }
}

/// `HTTPS_PROXY`/`https_proxy`, ersatzweise `ALL_PROXY`/`all_proxy`; `NO_PROXY` nimmt Rechner aus.
pub(crate) fn from_env(get: &dyn Fn(&str) -> Option<String>, host: &str) -> Option<(String, u16)> {
    let first = |keys: &[&str]| keys.iter().find_map(|k| get(k).filter(|v| !v.trim().is_empty()));
    let proxy = first(&["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"])?;
    if let Some(no) = first(&["NO_PROXY", "no_proxy"]) {
        if no_proxy_matches(&no, host) {
            return None;
        }
    }
    parse_host_port(&proxy)
}

/// `NO_PROXY`: Liste mit Komma; „*“ für alle, „.example.org“ bzw. „example.org“ auch für Unterrechner.
fn no_proxy_matches(list: &str, host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    list.split([',', ' ']).map(str::trim).filter(|p| !p.is_empty()).any(|p| {
        if p == "*" {
            return true;
        }
        let p = p.to_ascii_lowercase();
        let p = p.split(':').next().unwrap_or("").trim_start_matches('*').trim_start_matches('.');
        !p.is_empty() && (host == p || host.ends_with(&format!(".{p}")))
    })
}

/// `http://proxy:8080/`, `proxy:8080` oder `[::1]:8080` → Rechner und Port (Vorgabe 8080 ohne Port).
pub(crate) fn parse_host_port(s: &str) -> Option<(String, u16)> {
    let s = s.trim();
    let s = s.strip_prefix("http://").or_else(|| s.strip_prefix("HTTP://")).unwrap_or(s);
    if s.contains("://") || s.contains('@') {
        // https-, socks-Proxys und Anmeldedaten werden nicht unterstützt.
        return None;
    }
    let s = s.trim_end_matches('/');
    let (host, port) = if let Some(rest) = s.strip_prefix('[') {
        let (h, after) = rest.split_once(']')?;
        (h.to_string(), after.strip_prefix(':').map(|p| p.parse().ok()).unwrap_or(Some(8080))?)
    } else {
        match s.rsplit_once(':') {
            Some((h, p)) if !h.contains(':') => (h.to_string(), p.parse().ok()?),
            None => (s.to_string(), 8080),
            _ => return None,
        }
    };
    let ok = !host.is_empty() && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_'));
    (ok && port != 0).then_some((host, port))
}

/// Proxyliste wie in der Windows-Systemeinstellung bzw. Antwort eines Konfigurationsskripts:
/// `proxy:8080`, `https=proxy:8443;http=proxy:8080`, `PROXY a:1; DIRECT`. Bevorzugt der Eintrag für https.
#[cfg_attr(not(windows), allow(dead_code))] // nur die Windows-Systemeinstellung und Tests
pub(crate) fn pick_from_list(list: &str) -> Option<(String, u16)> {
    let items: Vec<&str> = list.split([';', ' ']).map(str::trim).filter(|x| !x.is_empty()).collect();
    let by = |scheme: &str| items.iter().find_map(|i| i.split_once('=').filter(|(k, _)| k.eq_ignore_ascii_case(scheme)).map(|(_, v)| v));
    if let Some(v) = by("https").or_else(|| by("http")) {
        return parse_host_port(v);
    }
    items.iter().filter(|i| !i.contains('=') && !i.eq_ignore_ascii_case("PROXY") && !i.eq_ignore_ascii_case("DIRECT")).find_map(|i| parse_host_port(i))
}

/// Ausnahmeliste der Windows-Systemeinstellung: `;`-getrennt, `*` als Platzhalter, `<local>` für Namen ohne Punkt.
#[cfg_attr(not(windows), allow(dead_code))] // nur die Windows-Systemeinstellung und Tests
pub(crate) fn bypass_matches(list: &str, host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    list.split([';', ' ']).map(str::trim).filter(|p| !p.is_empty()).any(|p| {
        let p = p.to_ascii_lowercase();
        if p == "<local>" {
            return !host.contains('.');
        }
        wildcard(&p, &host)
    })
}

#[cfg_attr(not(windows), allow(dead_code))] // nur die Windows-Systemeinstellung und Tests
fn wildcard(pattern: &str, text: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == text,
        Some((head, rest)) => {
            let Some(tail) = text.strip_prefix(head) else { return false };
            (0..=tail.len()).filter(|i| tail.is_char_boundary(*i)).any(|i| wildcard(rest, &tail[i..]))
        }
    }
}

#[cfg(windows)]
mod windows {
    //! Systemeinstellung des Benutzers (wie Edge/Internet-Optionen) über WinHTTP, inklusive PAC und WPAD.
    use super::{bypass_matches, pick_from_list};
    use windows_sys::Win32::Foundation::{GlobalFree, FALSE, TRUE};
    use windows_sys::Win32::Networking::WinHttp::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Liest eine von WinHTTP angelegte Zeichenkette und gibt sie frei.
    unsafe fn take(p: *mut u16) -> Option<String> {
        if p.is_null() {
            return None;
        }
        let mut n = 0;
        while *p.add(n) != 0 {
            n += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
        GlobalFree(p as _);
        Some(s)
    }

    /// `Some(None)`: Systemeinstellung sagt „direkt“; `Some(Some(p))`: Proxy; `None`: nichts eingestellt.
    pub(super) fn system_proxy(url: &str, host: &str) -> Option<Option<(String, u16)>> {
        unsafe {
            let mut ie: WINHTTP_CURRENT_USER_IE_PROXY_CONFIG = std::mem::zeroed();
            if WinHttpGetIEProxyConfigForCurrentUser(&mut ie) == FALSE {
                return None;
            }
            let auto_url = take(ie.lpszAutoConfigUrl);
            let fixed = take(ie.lpszProxy);
            let bypass = take(ie.lpszProxyBypass);
            // Konfigurationsskript oder automatische Erkennung: WinHTTP wertet beides aus.
            if ie.fAutoDetect != FALSE || auto_url.is_some() {
                if let Some(r) = from_script(url, auto_url.as_deref(), ie.fAutoDetect != FALSE) {
                    return Some(r);
                }
            }
            let fixed = fixed?;
            if bypass.as_deref().is_some_and(|b| bypass_matches(b, host)) {
                return Some(None);
            }
            Some(pick_from_list(&fixed))
        }
    }

    unsafe fn from_script(url: &str, script: Option<&str>, detect: bool) -> Option<Option<(String, u16)>> {
        let agent = wide("remotejoe");
        let session = WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_NO_PROXY, std::ptr::null(), std::ptr::null(), 0);
        if session.is_null() {
            return None;
        }
        let script_w = script.map(wide);
        let mut opts: WINHTTP_AUTOPROXY_OPTIONS = std::mem::zeroed();
        if detect {
            opts.dwFlags |= WINHTTP_AUTOPROXY_AUTO_DETECT;
            opts.dwAutoDetectFlags = WINHTTP_AUTO_DETECT_TYPE_DHCP | WINHTTP_AUTO_DETECT_TYPE_DNS_A;
        }
        if let Some(w) = &script_w {
            opts.dwFlags |= WINHTTP_AUTOPROXY_CONFIG_URL;
            opts.lpszAutoConfigUrl = w.as_ptr();
        }
        opts.fAutoLogonIfChallenged = TRUE;
        let url_w = wide(url);
        let mut info: WINHTTP_PROXY_INFO = std::mem::zeroed();
        let ok = WinHttpGetProxyForUrl(session, url_w.as_ptr(), &mut opts, &mut info) != FALSE;
        WinHttpCloseHandle(session);
        if !ok {
            return None;
        }
        let list = take(info.lpszProxy);
        let _ = take(info.lpszProxyBypass);
        if info.dwAccessType == WINHTTP_ACCESS_TYPE_NO_PROXY {
            return Some(None);
        }
        Some(list.as_deref().and_then(pick_from_list))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_and_manual_values() {
        assert_eq!(ProxyMode::from_setting(""), ProxyMode::Auto);
        assert_eq!(ProxyMode::from_setting("none"), ProxyMode::Off);
        assert_eq!(ProxyMode::from_setting("proxy.example:3128"), ProxyMode::Manual("proxy.example:3128".into()));
        assert_eq!(parse_host_port("http://proxy.example:3128/"), Some(("proxy.example".into(), 3128)));
        assert_eq!(parse_host_port("proxy.example"), Some(("proxy.example".into(), 8080)));
        assert_eq!(parse_host_port("[2001:db8::1]:3128"), Some(("2001:db8::1".into(), 3128)));
        assert_eq!(parse_host_port("socks5://p:1080"), None);
        assert_eq!(parse_host_port("http://nutzer:pw@p:8080"), None, "keine Anmeldung");
        assert!(check_manual("p:x").is_err());
    }

    #[test]
    fn environment_and_no_proxy() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |k: &str| pairs.iter().find(|(a, _)| *a == k).map(|(_, v)| v.to_string());
        let e = env(&[("HTTPS_PROXY", "http://proxy.example:3128"), ("NO_PROXY", "localhost,.intern.example,github.local")]);
        assert_eq!(from_env(&e, "api.github.com"), Some(("proxy.example".into(), 3128)));
        assert_eq!(from_env(&e, "git.intern.example"), None);
        assert_eq!(from_env(&e, "github.local"), None);
        assert_eq!(from_env(&env(&[("all_proxy", "p:1")]), "x.example"), Some(("p".into(), 1)));
        assert_eq!(from_env(&env(&[]), "x.example"), None);
        // Lokale Ziele gehen bei „Automatisch“ immer direkt.
        assert_eq!(for_target(&ProxyMode::Auto, "http://127.0.0.1/", "127.0.0.1"), None);
        assert_eq!(for_target(&ProxyMode::Off, "https://api.github.com/", "api.github.com"), None);
    }

    #[test]
    fn windows_lists_and_bypass() {
        assert_eq!(pick_from_list("proxy.example:8080"), Some(("proxy.example".into(), 8080)));
        assert_eq!(pick_from_list("http=a.example:80;https=b.example:443"), Some(("b.example".into(), 443)));
        assert_eq!(pick_from_list("ftp=f:21;http=a.example:3128"), Some(("a.example".into(), 3128)));
        assert_eq!(pick_from_list("PROXY p.example:3128; DIRECT"), Some(("p.example".into(), 3128)));
        assert_eq!(pick_from_list("DIRECT"), None);
        assert!(bypass_matches("*.intern.example;<local>", "srv.intern.example"));
        assert!(bypass_matches("*.intern.example;<local>", "fileserver"));
        assert!(!bypass_matches("*.intern.example;<local>", "api.github.com"));
        assert!(bypass_matches("10.*", "10.1.2.3"));
    }
}
