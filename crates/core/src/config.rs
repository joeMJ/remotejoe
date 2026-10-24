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
    /// Schrift und Farben des SSH-Terminals.
    pub terminal: crate::termcolors::TerminalSettings,
    /// Update-Einstellungen (Quelle fest: GitHub).
    pub update: UpdateSettings,
    /// Farbschema und Fokusrahmen.
    pub appearance: AppearanceSettings,
    /// Zuschaltbare Zusatzfunktionen.
    pub features: FeatureSettings,
    /// Sicherungen der KDBX.
    pub backup: BackupSettings,
}

/// Sicherungen der KDBX (siehe [`crate::backup`]): nach jedem Speichern und vor jedem Update, im Ordner
/// `Sicherungen` neben der KDBX und wahlweise an einem zweiten Ort.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupSettings {
    pub enabled: bool,
    /// Neueste Sicherungen, die bleiben.
    pub keep_last: u32,
    /// Dazu die erste Sicherung jedes der letzten so vielen Tage.
    pub keep_days: u32,
    /// Sicherungen vor einem Update, die bleiben.
    pub keep_update: u32,
    /// Zusätzlich an einen zweiten Ort sichern (z. B. NAS).
    pub second_enabled: bool,
    pub second_dir: String,
}

impl Default for BackupSettings {
    fn default() -> Self {
        BackupSettings { enabled: true, keep_last: 10, keep_days: 7, keep_update: 3, second_enabled: false, second_dir: String::new() }
    }
}

impl BackupSettings {
    pub fn retention(&self) -> crate::backup::Retention {
        crate::backup::Retention { keep_last: self.keep_last as usize, keep_days: self.keep_days as usize, keep_update: self.keep_update as usize }
    }
}

/// Zuschaltbare Zusatzfunktionen (Vorgabe: alle aus).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FeatureSettings {
    /// Aruba-Werkzeuge: Cluster-Übersicht am virtuellen Controller, AP-Seiten, ap_check.
    pub aruba: bool,
    /// Anwender je AP, ab denen die Zahl auf der Kachel gelb bzw. rot wird (0 = aus).
    pub aruba_clients_warn: u32,
    pub aruba_clients_crit: u32,
    /// Claude-Anbindung (MCP), Vorgabe aus.
    pub mcp: bool,
    /// Adressen (IP, MAC) bzw. Seriennummern gegenüber Claude ausblenden.
    pub mcp_hide_addresses: bool,
    pub mcp_hide_serials: bool,
    /// Für Claude freigegebene Ordner und Anmeldeobjekte, je Schlüsseldatenbank (Pfad → IDs).
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub mcp_folders: std::collections::BTreeMap<String, Vec<String>>,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub mcp_logins: std::collections::BTreeMap<String, Vec<String>>,
}

impl Default for FeatureSettings {
    fn default() -> Self {
        FeatureSettings {
            aruba: false,
            aruba_clients_warn: 30,
            aruba_clients_crit: 64,
            mcp: false,
            mcp_hide_addresses: false,
            mcp_hide_serials: false,
            mcp_folders: Default::default(),
            mcp_logins: Default::default(),
        }
    }
}

/// Darstellung: Farbschema (`system`, `hell`, `dunkel`, `eingebaut:<Name>` oder `datei:<Name>` für eine eigene Palette)
/// und der Rahmen um die Sitzung mit dem Tastaturfokus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceSettings {
    pub scheme: String,
    pub focus_frame: bool,
    /// Bei `scheme = system`: Palette im hellen bzw. dunklen Modus des Systems
    /// (`system` = Farben des Desktops, sonst eine Kennung wie bei `scheme`).
    pub light_theme: String,
    pub dark_theme: String,
    /// Größe des Eintragbaums: `klein`, `mittel` oder `gross` (Zeilenhöhe, Symbole, Schrift).
    pub tree_size: String,
    /// Farbstil der Symbole: `einheitlich` oder `je_typ`.
    pub icon_style: String,
    /// Eigene Symbolfarben je Art (`rdp`, `ssh`, …, `folder` → `#rrggbb`); leer = dem Theme folgen.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub icon_colors: std::collections::BTreeMap<String, String>,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        AppearanceSettings {
            scheme: "system".into(),
            focus_frame: true,
            light_theme: "system".into(),
            dark_theme: "system".into(),
            tree_size: "mittel".into(),
            icon_style: "einheitlich".into(),
            icon_colors: Default::default(),
        }
    }
}

/// Einstellungen der Update-Funktion. Die Quelle ist fest: die Veröffentlichungen von remotejoe auf
/// GitHub (`joeMJ/remotejoe`); ältere Einträge für Quelle, Adresse und Zugriffsschlüssel werden ignoriert.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateSettings {
    /// Beim Start im Hintergrund prüfen (Voreinstellung aus).
    pub check_on_start: bool,
    /// Proxy: leer = automatisch (Systemeinstellung bzw. Umgebung), „none“ = keiner, sonst `Rechner:Port`.
    pub proxy: String,
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
            terminal: Default::default(),
            update: UpdateSettings::default(),
            appearance: AppearanceSettings::default(),
            features: FeatureSettings::default(),
            backup: BackupSettings::default(),
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
    /// Wurde beim Beenden gespeichert; vorher gelten die Voreinstellungen des Fensters nicht.
    pub saved: bool,
    /// Platzsparmodus: Menü im Hamburger-Knopf, Tabs in der Titelzeile.
    pub compact: bool,
    /// Baum eingeklappt.
    pub tree_hidden: bool,
    /// Aufgeklappte Ordner beim Start, je Schlüsseldatenbank (Pfad → Ordner-IDs); fehlt der Eintrag, gilt die Vorgabe.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub tree_start: std::collections::BTreeMap<String, Vec<String>>,
}

impl Default for WindowState {
    fn default() -> Self {
        WindowState { x: 100, y: 100, width: 1280, height: 800, maximized: false, tree_width: 280, saved: false, compact: false, tree_hidden: false, tree_start: Default::default() }
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
    /// Tastenkürzel des Desktops (Windows-Taste, Alt+Tab, Menütaste) gehen an RDP/VNC, solange die
    /// Sitzung den Tastaturfokus hat.
    pub keyboard_capture: bool,
    /// Strg+Umschalt+W schließt den aktiven Tab: „outside“ (nur wenn der Fokus nicht in einer Sitzung liegt),
    /// „always“ oder „never“.
    pub close_tab_key: String,
    /// Administrativer Tastaturmodus (RDP): Belegung (KLID, z. B. „00000407“), wenn das Ziel keine eigene nennt.
    pub admin_keys_layout: String,
    /// … und Abschalten nach so vielen Minuten ohne Eingabe (0 = nie).
    pub admin_keys_timeout_mins: u32,
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
            keyboard_capture: true,
            close_tab_key: "outside".into(),
            admin_keys_layout: "00000407".into(),
            admin_keys_timeout_mins: 10,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SecurityDefaults {
    /// Sperren nach Leerlauf in Minuten; 0 = aus (Vorgabe).
    pub lock_after_idle_mins: u32,
    /// Beim Sperren alle Sitzungen trennen (Vorgabe: verbunden lassen).
    pub disconnect_on_lock: bool,
    /// Sekunden, nach denen ein kopiertes Kennwort aus der Zwischenablage gelöscht wird; 0 = nie.
    pub clipboard_clear_secs: u32,
    /// Das Hauptkennwort liegt (auf Wunsch) im Schlüsselbund des Systems; der Start entsperrt damit.
    pub keyring_unlock: bool,
}

impl Default for SecurityDefaults {
    fn default() -> Self {
        SecurityDefaults { lock_after_idle_mins: 0, disconnect_on_lock: false, clipboard_clear_secs: 30, keyring_unlock: false }
    }
}

/// Alter Konfigurationsordner: Linux nach XDG, Windows `%APPDATA%`.
fn legacy_config_dir() -> Option<PathBuf> {
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

/// Alter Datenordner (Linux `XDG_DATA_HOME`, Windows `%LOCALAPPDATA%`).
fn legacy_data_dir() -> Option<PathBuf> {
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

/// Der eine Ordner für alles (Einstellungen `config.toml`, Paletten, Daten unter `daten/`):
/// 1. `REMOTEJOE_HOME`, wenn gesetzt;
/// 2. wer `XDG_CONFIG_HOME` oder `XDG_DATA_HOME` selbst setzt, behält die alten Ordner (`None`);
/// 3. `~/remotejoe`, sobald dort eine `config.toml` liegt;
/// 4. eine vorhandene alte Installation (alte `config.toml` oder `known_hosts`) bleibt, wo sie ist (`None`);
/// 5. sonst `~/remotejoe` (neue Installation).
/// `None` heißt: die alten Ordner gelten, es geht nichts verloren und nichts wird verschoben.
pub fn home_dir() -> Option<PathBuf> {
    resolve_home(&|k| std::env::var_os(k).filter(|v| !v.is_empty()), &|p| p.exists())
}

/// Wie `home_dir`, mit austauschbarer Umgebung und Dateiprüfung (für Tests).
fn resolve_home(get: &dyn Fn(&str) -> Option<std::ffi::OsString>, exists: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    if let Some(p) = get("REMOTEJOE_HOME") {
        return Some(PathBuf::from(p));
    }
    #[cfg(not(windows))]
    if get("XDG_CONFIG_HOME").is_some() || get("XDG_DATA_HOME").is_some() {
        return None;
    }
    #[cfg(windows)]
    let (home, old_config, old_data) = (get("USERPROFILE").map(PathBuf::from)?, get("APPDATA").map(|p| PathBuf::from(p).join("remotejoe")), get("LOCALAPPDATA").map(|p| PathBuf::from(p).join("remotejoe")));
    #[cfg(not(windows))]
    let (home, old_config, old_data) = {
        let h = PathBuf::from(get("HOME")?);
        (h.clone(), Some(h.join(".config/remotejoe")), Some(h.join(".local/share/remotejoe")))
    };
    let new = home.join("remotejoe");
    if exists(&new.join("config.toml")) {
        return Some(new);
    }
    let old_in_use = old_config.is_some_and(|d| exists(&d.join("config.toml"))) || old_data.is_some_and(|d| exists(&d.join("known_hosts")));
    if old_in_use {
        return None;
    }
    Some(new)
}

/// Konfigurationsordner (`config.toml`, `paletten/`): der eine Ordner, sonst der alte.
pub fn config_dir() -> Option<PathBuf> {
    home_dir().or_else(legacy_config_dir)
}

/// Datenordner (bekannte Host-Schlüssel, FreeRDP- und Web-Profile): `daten/` im einen Ordner, sonst der alte.
pub fn data_dir() -> Option<PathBuf> {
    home_dir().map(|h| h.join("daten")).or_else(legacy_data_dir)
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
    fn window_state_old_file_and_new_fields() {
        // Alte Konfiguration ohne Platzsparmodus und Baumzustand lädt mit Voreinstellung.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.toml");
        std::fs::write(&p, "[window]\nx = 5\ny = 6\nwidth = 900\nheight = 700\nsaved = true\n").unwrap();
        let c = Config::load(&p).unwrap();
        assert!(c.window.saved && !c.window.compact && !c.window.tree_hidden);
        let mut c = c;
        c.window.compact = true;
        c.window.tree_hidden = true;
        c.save(&p).unwrap();
        let again = Config::load(&p).unwrap();
        assert!(again.window.compact && again.window.tree_hidden);
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

#[cfg(test)]
mod home_tests {
    use super::*;
    use std::collections::HashSet;
    use std::ffi::OsString;

    fn run(env: &[(&str, &str)], files: &[&str]) -> Option<PathBuf> {
        let files: HashSet<PathBuf> = files.iter().map(PathBuf::from).collect();
        let env: Vec<(String, String)> = env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        resolve_home(&|k| env.iter().find(|(n, _)| n == k).map(|(_, v)| OsString::from(v)), &|p| files.contains(p))
    }

    #[cfg(not(windows))]
    #[test]
    fn one_folder_for_new_installations_and_old_ones_stay_put() {
        let h = [("HOME", "/home/u")];
        // Neue Installation: ein Ordner unter dem persönlichen Ordner.
        assert_eq!(run(&h, &[]), Some(PathBuf::from("/home/u/remotejoe")));
        // Vorhandene alte Installation bleibt (nichts wird verschoben oder abgespalten).
        assert_eq!(run(&h, &["/home/u/.config/remotejoe/config.toml"]), None);
        assert_eq!(run(&h, &["/home/u/.local/share/remotejoe/known_hosts"]), None);
        // Sobald der neue Ordner eine config.toml hat, gilt er.
        assert_eq!(run(&h, &["/home/u/remotejoe/config.toml", "/home/u/.config/remotejoe/config.toml"]), Some(PathBuf::from("/home/u/remotejoe")));
        // Eigene XDG-Ordner und REMOTEJOE_HOME haben Vorrang.
        assert_eq!(run(&[("HOME", "/home/u"), ("XDG_DATA_HOME", "/x")], &[]), None);
        assert_eq!(run(&[("HOME", "/home/u"), ("XDG_CONFIG_HOME", "/x")], &[]), None);
        assert_eq!(run(&[("HOME", "/home/u"), ("REMOTEJOE_HOME", "/daten/rj"), ("XDG_DATA_HOME", "/x")], &[]), Some(PathBuf::from("/daten/rj")));
        // Ohne persönlichen Ordner gibt es keinen neuen Ordner.
        assert_eq!(run(&[], &[]), None);
    }
}
