//! Eigene Liste bekannter Host-Schlüssel.
//!
//! Format je Zeile: `<host>:<port> <algorithmus> <schlüssel-base64>`.
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

/// Liste bekannter Host-Schlüssel in einer Datei.
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

    fn entries(&self) -> Vec<(String, String, String)> {
        let Ok(bytes) = std::fs::read(&self.path) else { return Vec::new() };
        let Ok(text) = rj_core::charset::validate_utf8(&bytes, &self.path.display().to_string()) else {
            return Vec::new();
        };
        text.lines()
            .filter_map(|l| {
                let mut it = l.split_whitespace();
                Some((it.next()?.to_string(), it.next()?.to_string(), it.next()?.to_string()))
            })
            .collect()
    }

    fn host_id(host: &str, port: u16) -> String {
        format!("{host}:{port}")
    }

    pub fn check(&self, host: &str, port: u16, key: &PublicKey) -> HostKeyStatus {
        let id = Self::host_id(host, port);
        let alg = key.algorithm().to_string();
        let mine = key.to_openssh().unwrap_or_default();
        let b64 = mine.split_whitespace().nth(1).unwrap_or_default().to_string();
        let mut changed = None;
        for (h, a, k) in self.entries() {
            if h != id {
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
    /// Algorithmus für denselben Host wird ersetzt.
    pub fn remember(&self, host: &str, port: u16, key: &PublicKey) -> std::io::Result<()> {
        let id = Self::host_id(host, port);
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
        for (h, a, k) in self.entries() {
            if !(h == id && a == alg) {
                out.push_str(&format!("{h} {a} {k}\n"));
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
}
