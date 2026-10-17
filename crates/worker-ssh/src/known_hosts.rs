//! Eigene Liste bekannter Host-Schlüssel.
//!
//! Format wie OpenSSH (`known_hosts`): `<host> <algorithmus> <schlüssel>`,
//! bei anderem Port als 22 `[host]:port`.
//! Neue oder geänderte Schlüssel werden nie still angenommen; die
//! Entscheidung trifft der Benutzer.

use std::path::{Path, PathBuf};

use russh::keys::ssh_key::{HashAlg, PublicKey};

/// Ergebnis der Prüfung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyStatus {
    Known,
    Unknown,
    Changed { previous_fingerprint: String },
}

/// Rückfrage an den Benutzer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeyQuestion {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
    pub status: HostKeyStatus,
    /// Schlüssel im OpenSSH-Format (zum Merken nach Zustimmung).
    pub key: String,
}

impl HostKeyQuestion {
    pub fn text(&self) -> String {
        match &self.status {
            HostKeyStatus::Unknown => format!(
                "Der Host-Schlüssel von {}:{} ist unbekannt. Prüfen Sie den Fingerabdruck, bevor Sie ihn annehmen.",
                self.host, self.port
            ),
            HostKeyStatus::Changed { .. } => format!(
                "WARNUNG: Der Host-Schlüssel von {}:{} hat sich geändert. Das kann auf einen Angriff hinweisen.",
                self.host, self.port
            ),
            HostKeyStatus::Known => String::new(),
        }
    }

    /// Angaben für die Rückfrage-Leiste.
    pub fn info(&self) -> rj_ipc::msg::SecurityInfo {
        rj_ipc::msg::SecurityInfo {
            kind: "hostkey".into(),
            host: self.host.clone(),
            port: self.port,
            key_type: self.algorithm.clone(),
            fingerprint: self.fingerprint.clone(),
            previous: match &self.status {
                HostKeyStatus::Changed { previous_fingerprint } => Some(previous_fingerprint.clone()),
                _ => None,
            },
        }
    }

    pub fn detail(&self) -> String {
        match &self.status {
            HostKeyStatus::Changed { previous_fingerprint } => format!(
                "{} {}\nbisher: {}",
                self.algorithm, self.fingerprint, previous_fingerprint
            ),
            _ => format!("{} {}", self.algorithm, self.fingerprint),
        }
    }
}

pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

/// Eine Zeile der Datei.
#[derive(Debug, Clone)]
enum Line {
    /// Eintrag: Hostmuster, Algorithmus, Schlüssel (Base64).
    Entry { hosts: String, alg: String, key: String },
    /// Kommentar, Leerzeile, Zeile mit Markierung (`@cert-authority`,
    /// `@revoked`) oder unlesbare Zeile: bleibt beim Schreiben unverändert.
    Other(String),
}

/// Hostkennung wie bei OpenSSH: Port 22 ohne Klammern, sonst `[host]:port`.
pub fn host_id(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    }
}

/// Erkennt das frühere eigene Format `host:port` (ohne Klammern) und liefert
/// die OpenSSH-Schreibweise. Eine vollständige IPv6-Adresse gilt als
/// OpenSSH-Eintrag für Port 22.
fn legacy_id(field: &str) -> Option<String> {
    if field.contains(',') || field.starts_with('[') || field.starts_with('|') || field.parse::<std::net::Ipv6Addr>().is_ok() {
        return None;
    }
    let (h, p) = field.rsplit_once(':')?;
    let port: u16 = p.parse().ok()?;
    (!h.is_empty()).then(|| host_id(h, port))
}

/// Einfacher Mustervergleich wie bei OpenSSH (`*` und `?`).
fn glob(pattern: &[u8], text: &[u8]) -> bool {
    match (pattern.first(), text.first()) {
        (None, None) => true,
        (Some(b'*'), _) => glob(&pattern[1..], text) || (!text.is_empty() && glob(pattern, &text[1..])),
        (Some(b'?'), Some(_)) => glob(&pattern[1..], &text[1..]),
        (Some(a), Some(b)) if a.eq_ignore_ascii_case(b) => glob(&pattern[1..], &text[1..]),
        _ => false,
    }
}

/// Gehashter Eintrag `|1|salz|hmac` (HMAC-SHA1 über die Hostkennung).
fn hashed_matches(field: &str, id: &str) -> bool {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    let mut parts = field.strip_prefix("|1|").unwrap_or_default().split('|');
    let (Some(salt), Some(hash)) = (parts.next(), parts.next()) else { return false };
    let b64 = base64::engine::general_purpose::STANDARD;
    let (Ok(salt), Ok(hash)) = (b64.decode(salt), b64.decode(hash)) else { return false };
    let Ok(mut mac) = Hmac::<sha1::Sha1>::new_from_slice(&salt) else { return false };
    mac.update(id.as_bytes());
    mac.verify_slice(&hash).is_ok()
}

/// Passt das Hostfeld einer Zeile (Liste mit Kommas, gehasht, mit
/// Platzhaltern, Verneinung mit `!`) auf die Hostkennung?
fn hosts_match(field: &str, id: &str) -> bool {
    if field.starts_with("|1|") {
        return hashed_matches(field, id);
    }
    let mut hit = false;
    for pat in field.split(',') {
        if let Some(neg) = pat.strip_prefix('!') {
            if glob(neg.as_bytes(), id.as_bytes()) {
                return false;
            }
        } else if glob(pat.as_bytes(), id.as_bytes()) {
            hit = true;
        }
    }
    hit
}

/// Liste bekannter Host-Schlüssel in einer Datei im Format von OpenSSH
/// (`known_hosts`). Gelesen werden auch gehashte Einträge, Listen und
/// Platzhalter sowie das frühere eigene Format `host:port`, das beim
/// nächsten Schreiben umgestellt wird.
#[derive(Debug, Clone)]
pub struct KnownHosts {
    path: PathBuf,
}

impl KnownHosts {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        KnownHosts { path: path.into() }
    }

    /// Standardort: Datenordner des Programms.
    pub fn default_location() -> Option<Self> {
        rj_core::config::data_dir().map(|d| KnownHosts::new(d.join("known_hosts")))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn lines(&self) -> Vec<Line> {
        let Ok(bytes) = std::fs::read(&self.path) else { return Vec::new() };
        let Ok(text) = rj_core::charset::validate_utf8(&bytes, &self.path.display().to_string()) else {
            return Vec::new();
        };
        text.lines()
            .map(|l| {
                let t = l.trim_start();
                if t.is_empty() || t.starts_with('#') || t.starts_with('@') {
                    return Line::Other(l.to_string());
                }
                let mut it = t.split_whitespace();
                match (it.next(), it.next(), it.next()) {
                    (Some(h), Some(a), Some(k)) => {
                        let hosts = legacy_id(h).unwrap_or_else(|| h.to_string());
                        Line::Entry { hosts, alg: a.to_string(), key: k.to_string() }
                    }
                    _ => Line::Other(l.to_string()),
                }
            })
            .collect()
    }

    pub fn check(&self, host: &str, port: u16, key: &PublicKey) -> HostKeyStatus {
        let id = host_id(host, port);
        let alg = key.algorithm().to_string();
        let mine = key.to_openssh().unwrap_or_default();
        let b64 = mine.split_whitespace().nth(1).unwrap_or_default().to_string();
        let mut changed = None;
        for line in self.lines() {
            let Line::Entry { hosts, alg: a, key: k, .. } = line else { continue };
            if !hosts_match(&hosts, &id) {
                continue;
            }
            if a == alg && k == b64 {
                return HostKeyStatus::Known;
            }
            // Auch ein bekannter Schlüssel eines anderen Algorithmus gilt als
            // Änderung: Ein Angreifer könnte sonst mit einem anderen
            // Schlüsseltyp eine harmlose Erstabfrage auslösen.
            if a == alg || changed.is_none() {
                let prev = PublicKey::from_openssh(&format!("{a} {k}"))
                    .map(|p| fingerprint(&p))
                    .unwrap_or_else(|_| "unlesbar".into());
                changed = Some(prev);
            }
        }
        match changed {
            Some(previous_fingerprint) => HostKeyStatus::Changed { previous_fingerprint },
            None => HostKeyStatus::Unknown,
        }
    }

    /// Speichert einen Schlüssel im OpenSSH-Format (nach Zustimmung des Benutzers).
    pub fn remember_openssh(&self, host: &str, port: u16, key: &str) -> std::io::Result<()> {
        let k = PublicKey::from_openssh(key).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        self.remember(host, port, &k)
    }

    /// Speichert den Schlüssel; ein bisheriger Schlüssel desselben
    /// Algorithmus für denselben Host (Zeile nur mit diesem Host) wird
    /// ersetzt. Alle übrigen Zeilen bleiben erhalten, Zeilen im früheren
    /// Format werden in die OpenSSH-Schreibweise umgestellt.
    pub fn remember(&self, host: &str, port: u16, key: &PublicKey) -> std::io::Result<()> {
        let id = host_id(host, port);
        let alg = key.algorithm().to_string();
        let openssh = key.to_openssh().map_err(|e| std::io::Error::other(e.to_string()))?;
        let b64 = openssh.split_whitespace().nth(1).unwrap_or_default();
        if let Some(d) = self.path.parent() {
            std::fs::create_dir_all(d)?;
        }
        // Lesen und Schreiben unter einer Sperre, damit gleichzeitig angenommene
        // Schlüssel anderer Sitzungen nicht verloren gehen.
        let _lock = rj_core::fsutil::WriteLock::acquire_wait(&self.path, std::time::Duration::from_secs(5))?;
        let mut out = String::new();
        for line in self.lines() {
            match line {
                Line::Entry { hosts, alg: a, .. } if hosts == id && a == alg => {}
                Line::Entry { hosts, alg, key, .. } => out.push_str(&format!("{hosts} {alg} {key}\n")),
                Line::Other(l) => {
                    out.push_str(&l);
                    out.push('\n');
                }
            }
        }
        out.push_str(&format!("{id} {alg} {b64}\n"));
        rj_core::fsutil::atomic_write_locked(&self.path, out.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ED: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIKyAnCq2Gk7BPfqYFQiNcGYgWfm4W8KibIapJ/jZJJGt";
    const EC: &str = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBL1MlyS6HseQE+xcGL7FDKQgEp1uooPqOjQP6+WcjwleyCFZ0R/VYlRyAPxTEjsJiPKtURy/j5Wvy0iBdrCEP4Q=";

    #[test]
    fn other_key_type_counts_as_changed() {
        let dir = tempfile::tempdir().unwrap();
        let kh = KnownHosts::new(dir.path().join("known_hosts"));
        let ed = PublicKey::from_openssh(ED).unwrap();
        let ec = PublicKey::from_openssh(EC).unwrap();
        assert_eq!(kh.check("srv.example.test", 22, &ed), HostKeyStatus::Unknown);
        kh.remember("srv.example.test", 22, &ed).unwrap();
        assert_eq!(kh.check("srv.example.test", 22, &ed), HostKeyStatus::Known);
        match kh.check("srv.example.test", 22, &ec) {
            HostKeyStatus::Changed { previous_fingerprint } => assert_eq!(previous_fingerprint, fingerprint(&ed)),
            s => panic!("erwartet Änderung, erhalten {s:?}"),
        }
        // Anderer Port ist ein anderer Host.
        assert_eq!(kh.check("srv.example.test", 2222, &ec), HostKeyStatus::Unknown);
    }

    #[test]
    fn openssh_notation_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let kh = KnownHosts::new(dir.path().join("known_hosts"));
        let ed = PublicKey::from_openssh(ED).unwrap();
        let ec = PublicKey::from_openssh(EC).unwrap();
        kh.remember("a.example.test", 22, &ed).unwrap();
        kh.remember("a.example.test", 2222, &ec).unwrap();
        kh.remember("2001:db8::7", 22, &ed).unwrap();
        kh.remember("2001:db8::7", 2200, &ed).unwrap();
        let text = std::fs::read_to_string(kh.path()).unwrap();
        let ids: Vec<&str> = text.lines().map(|l| l.split(' ').next().unwrap()).collect();
        assert_eq!(ids, ["a.example.test", "[a.example.test]:2222", "2001:db8::7", "[2001:db8::7]:2200"]);
        assert_eq!(kh.check("a.example.test", 22, &ed), HostKeyStatus::Known);
        assert_eq!(kh.check("a.example.test", 2222, &ec), HostKeyStatus::Known);
        assert_eq!(kh.check("2001:db8::7", 22, &ed), HostKeyStatus::Known);
        assert_eq!(kh.check("2001:db8::7", 2200, &ed), HostKeyStatus::Known);
        assert_eq!(kh.check("2001:db8::7", 2201, &ed), HostKeyStatus::Unknown);
    }

    #[test]
    fn legacy_entries_are_read_and_migrated() {
        let dir = tempfile::tempdir().unwrap();
        let kh = KnownHosts::new(dir.path().join("known_hosts"));
        std::fs::write(kh.path(), format!("# eigene Notiz\nold.example.test:22 {ED}\nold.example.test:2222 {ED}\n@cert-authority *.example.test {EC}\n")).unwrap();
        let ed = PublicKey::from_openssh(ED).unwrap();
        let ec = PublicKey::from_openssh(EC).unwrap();
        assert_eq!(kh.check("old.example.test", 22, &ed), HostKeyStatus::Known);
        assert_eq!(kh.check("old.example.test", 2222, &ed), HostKeyStatus::Known);
        kh.remember("new.example.test", 22, &ec).unwrap();
        let text = std::fs::read_to_string(kh.path()).unwrap();
        assert_eq!(
            text,
            format!("# eigene Notiz\nold.example.test {ED}\n[old.example.test]:2222 {ED}\n@cert-authority *.example.test {EC}\nnew.example.test {EC}\n")
        );
    }

    #[test]
    fn hashed_lists_and_patterns() {
        use base64::Engine;
        use hmac::{Hmac, Mac};
        let b64 = base64::engine::general_purpose::STANDARD;
        let salt = [7u8; 20];
        let mut mac = Hmac::<sha1::Sha1>::new_from_slice(&salt).unwrap();
        mac.update(b"[h.example.test]:2222");
        let hashed = format!("|1|{}|{}", b64.encode(salt), b64.encode(mac.finalize().into_bytes()));
        let dir = tempfile::tempdir().unwrap();
        let kh = KnownHosts::new(dir.path().join("known_hosts"));
        std::fs::write(kh.path(), format!("{hashed} {ED}\nx.example.test,192.0.2.5 {ED}\n*.wild.example.test,!bad.wild.example.test {ED}\n")).unwrap();
        let ed = PublicKey::from_openssh(ED).unwrap();
        assert_eq!(kh.check("h.example.test", 2222, &ed), HostKeyStatus::Known);
        assert_eq!(kh.check("h.example.test", 22, &ed), HostKeyStatus::Unknown);
        assert_eq!(kh.check("192.0.2.5", 22, &ed), HostKeyStatus::Known);
        assert_eq!(kh.check("a.wild.example.test", 22, &ed), HostKeyStatus::Known);
        assert_eq!(kh.check("bad.wild.example.test", 22, &ed), HostKeyStatus::Unknown);
    }
}
