//! Update-Funktion: neueste Veröffentlichung aus Gitea oder GitHub abrufen,
//! Version vergleichen, Paket laden, Signatur und Prüfsumme prüfen und die
//! Programmdateien ersetzen.
//!
//! Ablauf der Prüfung: zuerst die Ed25519-Signatur der Datei `SHA256SUMS`
//! mit dem eingebauten öffentlichen Schlüssel, dann die SHA-256-Prüfsumme des
//! Archivs aus dieser Datei. Schlägt eine Prüfung fehl, wird nichts entpackt.

pub mod archive;
pub mod github;
pub mod http;
pub mod install;
pub mod release;
pub mod sign;
pub mod version;

pub use release::{Found, Release, Source, SourceKind};
pub use version::Version;

/// Fehler der Update-Funktion. Zugriffsschlüssel erscheinen nie im Text.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("Keine Update-Quelle eingestellt")]
    NoSource,
    #[error("Adresse nicht erlaubt: {0} (nur https, außer 127.0.0.1/localhost)")]
    Insecure(String),
    #[error("Adresse ungültig: {0}")]
    Url(String),
    #[error("Netzwerk: {0}")]
    Net(String),
    #[error("Server meldet {status} für {what}")]
    Status { status: u16, what: String },
    #[error("Zeitlimit überschritten")]
    Timeout,
    #[error("Abgebrochen")]
    Cancelled,
    #[error("Download zu groß (mehr als {0} Bytes)")]
    TooLarge(u64),
    #[error("Antwort unverständlich: {0}")]
    Parse(String),
    #[error("Version „{0}“ ist keine gültige SemVer-Version")]
    Version(String),
    #[error("Die Veröffentlichung enthält kein Paket für diese Plattform ({0})")]
    NoAsset(String),
    #[error("Die Veröffentlichung enthält keine Datei „{0}“")]
    MissingFile(String),
    #[error("Signatur ungültig: SHA256SUMS ist nicht mit dem eingebauten Schlüssel signiert")]
    BadSignature,
    #[error("Öffentlicher Schlüssel ungültig: {0}")]
    BadKey(String),
    #[error("Prüfsumme des Archivs stimmt nicht")]
    BadChecksum,
    #[error("Archiv „{0}“ fehlt in SHA256SUMS")]
    NotListed(String),
    #[error("Archiv fehlerhaft: {0}")]
    Archive(String),
    #[error("Installation: {0}")]
    Install(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, UpdateError>;

/// Eingebaute Version (aus `Cargo.toml` des Arbeitsbereichs).
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Plattformkennung im Dateinamen der Pakete, z. B. `linux-x86_64`.
pub fn platform() -> String {
    let base = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    if std::env::consts::OS == "linux" {
        if let Ok(text) = std::fs::read_to_string("/etc/os-release") {
            return format!("{base}{}", linux_variant(&text));
        }
    }
    base
}

/// Zusatz zur Plattformkennung: Pakete für Ubuntu 24.04 (Qt 6.4, ältere Bibliotheken) sind eigene
/// Dateien `…-linux-x86_64-ubuntu2404.tar.zst`; alle anderen Linux-Systeme nehmen `…-linux-x86_64`.
pub fn linux_variant(os_release: &str) -> &'static str {
    let get = |key: &str| os_release.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')).map(|v| v.trim().trim_matches('"').to_string());
    match (get("ID").as_deref(), get("VERSION_ID").as_deref()) {
        (Some("ubuntu"), Some("24.04")) => "-ubuntu2404",
        _ => "",
    }
}

#[cfg(test)]
mod platform_tests {
    use super::*;

    #[test]
    fn ubuntu_2404_gets_its_own_package_variant() {
        assert_eq!(linux_variant("NAME=\"Ubuntu\"\nVERSION_ID=\"24.04\"\nID=ubuntu\n"), "-ubuntu2404");
        assert_eq!(linux_variant("ID=ubuntu\nVERSION_ID=\"26.04\"\n"), "");
        assert_eq!(linux_variant("ID=debian\nVERSION_ID=\"24.04\"\n"), "");
        assert_eq!(linux_variant(""), "");
        assert_eq!(asset_name("0.1.3", "linux-x86_64-ubuntu2404"), "remotejoe-0.1.3-linux-x86_64-ubuntu2404.tar.zst");
    }
}

/// Dateiname des Pakets für eine Version und Plattform.
pub fn asset_name(version: &str, platform: &str) -> String {
    let ext = if platform.starts_with("windows") { "zip" } else { "tar.zst" };
    format!("remotejoe-{version}-{platform}.{ext}")
}

/// Grenzen für den Download.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_bytes: u64,
    pub timeout: std::time::Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_bytes: 300 * 1024 * 1024, timeout: std::time::Duration::from_secs(600) }
    }
}
