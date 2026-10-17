//! Programmkonfiguration (`config.toml`). Enthält nie Kennwörter.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::charset::{validate_utf8, Utf8Error};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Konfiguration konnte nicht gelesen werden: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Utf8(#[from] Utf8Error),
    #[error("Konfiguration fehlerhaft: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("Konfiguration nicht speicherbar: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// Ablageort des Index.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IndexStorage {
    /// In der KDBX als Datenbank-`CustomData`.
    #[default]
    KdbxCustomData,
    /// Eigene verschlüsselte Datei neben der KDBX (`<name>.rjindex`).
    IndexFile,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Pfad der KDBX-Datei.
    pub kdbx_path: Option<PathBuf>,
    /// Optionale Schlüsseldatei.
    pub key_file: Option<PathBuf>,
    pub index_storage: IndexStorage,
    pub window: WindowState,
    pub connection: ConnectionDefaults,
    pub security: SecurityDefaults,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            kdbx_path: None,
            key_file: None,
            index_storage: IndexStorage::default(),
            window: WindowState::default(),
            connection: ConnectionDefaults::default(),
            security: SecurityDefaults::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
    /// Breite des Baums in Bildpunkten.
    pub tree_width: u32,
}

impl Default for WindowState {
    fn default() -> Self {
        WindowState { x: 100, y: 100, width: 1280, height: 800, maximized: false, tree_width: 280 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConnectionDefaults {
    /// Zeitlimit für den Verbindungsaufbau in Sekunden.
    pub connect_timeout_secs: u32,
    /// Keepalive bei Leerlauf in Sekunden.
    pub keepalive_secs: u32,
    pub auto_reconnect: bool,
    /// Obergrenze des Rückstaus beim Wiederverbinden in Sekunden.
    pub reconnect_max_secs: u32,
    /// Größenlimit der Zwischenablage in Bytes.
    pub clipboard_limit: u64,
    /// „Alle im Ordner verbinden“: höchstens so viele Verbindungsaufbauten
    /// gleichzeitig.
    pub connect_all_parallel: u32,
}

impl Default for ConnectionDefaults {
    fn default() -> Self {
        ConnectionDefaults {
            connect_timeout_secs: 15,
            keepalive_secs: 30,
            auto_reconnect: false,
            reconnect_max_secs: 60,
            clipboard_limit: 16 * 1024 * 1024,
            connect_all_parallel: 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SecurityDefaults {
    /// Sperren nach Leerlauf in Minuten; 0 = nie.
    pub lock_after_idle_mins: u32,
}

impl Default for SecurityDefaults {
    fn default() -> Self {
        SecurityDefaults { lock_after_idle_mins: 15 }
    }
}

/// Konfigurationsordner: Linux nach XDG, Windows `%APPDATA%`.
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("remotejoe"))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|p| p.join("remotejoe"))
    }
}

/// Ordner für Daten, die nicht synchronisiert werden müssen (Linux:
/// `XDG_DATA_HOME`, Windows `%LOCALAPPDATA%`).
pub fn data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("remotejoe"))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .map(|p| p.join("remotejoe"))
    }
}

pub fn default_config_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("config.toml"))
}

impl Config {
    /// Liest die Konfiguration; fehlt die Datei, gelten die Standardwerte.
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => return Err(e.into()),
        };
        let text = validate_utf8(&bytes, &path.display().to_string())?;
        Ok(toml::from_str(text)?)
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self)?;
        crate::fsutil::atomic_write_with_backup(path, text.as_bytes())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let c = Config::load(&dir.path().join("fehlt.toml")).unwrap();
        assert_eq!(c, Config::default());
        assert_eq!(c.connection.connect_timeout_secs, 15);
        assert_eq!(c.connection.keepalive_secs, 30);
    }

    #[test]
    fn roundtrip_with_umlaut_path() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Ordner mit Ümlaut").join("config.toml");
        let c = Config {
            kdbx_path: Some(dir.path().join("Schlüssel 🔑.kdbx")),
            index_storage: IndexStorage::IndexFile,
            ..Config::default()
        };
        c.save(&p).unwrap();
        assert_eq!(Config::load(&p).unwrap(), c);
    }

    #[test]
    fn partial_file_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.toml");
        std::fs::write(&p, "[connection]\nkeepalive_secs = 10\n").unwrap();
        let c = Config::load(&p).unwrap();
        assert_eq!(c.connection.keepalive_secs, 10);
        assert_eq!(c.connection.connect_timeout_secs, 15);
    }

    #[test]
    fn invalid_utf8_reported() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.toml");
        std::fs::write(&p, b"kdbx_path = \"\xFF\"\n").unwrap();
        assert!(matches!(Config::load(&p), Err(ConfigError::Utf8(_))));
    }
}
