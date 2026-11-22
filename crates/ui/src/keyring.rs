//! Hauptkennwort der Schlüsseldatenbank im Schlüsselbund des Systems (nur auf ausdrücklichen Wunsch).
//!
//! Linux: Secret Service (GNOME-Schlüsselbund, KWallet) über D-Bus; Windows: Anmeldeinformationsverwaltung.
//! Je Datenbank ein Eintrag unter dem Dienst `remotejoe`; der Wert ist JSON mit Kennwort und Pfad der
//! Schlüsseldatei. Wer das Benutzerkonto entsperrt hat, kommt damit an die Datenbank: darum Standard aus,
//! Eintrag jederzeit entfernbar, und nur beim Programmstart wird damit entsperrt (nie nach dem Sperren).

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub const SERVICE: &str = "remotejoe";

#[derive(Serialize, Deserialize)]
struct Stored {
    password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key_file: Option<String>,
}

/// Gespeicherte Angaben zu einer Datenbank.
pub struct Found {
    pub password: Zeroizing<String>,
    pub key_file: Option<String>,
}

/// Schlüssel des Eintrags: der Pfad der Datenbank, wie das Betriebssystem ihn auflöst.
pub fn account(path: &str) -> String {
    let p = std::path::Path::new(path);
    let p = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    format!("tresor:{}", p.display())
}

#[cfg(any(target_os = "linux", windows))]
mod imp {
    use super::*;

    fn entry(path: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, &account(path)).map_err(|e| e.to_string())
    }

    pub fn store(path: &str, password: &str, key_file: Option<&str>) -> Result<(), String> {
        let value = Zeroizing::new(serde_json::to_string(&Stored { password: password.to_string(), key_file: key_file.map(str::to_string) }).map_err(|e| e.to_string())?);
        entry(path)?.set_password(&value).map_err(|e| e.to_string())
    }

    pub fn load(path: &str) -> Result<Option<Found>, String> {
        match entry(path)?.get_password() {
            Ok(v) => {
                let v = Zeroizing::new(v);
                let s: Stored = serde_json::from_str(&v).map_err(|_| "Der Eintrag im Schlüsselbund ist unlesbar".to_string())?;
                Ok(Some(Found { password: Zeroizing::new(s.password), key_file: s.key_file }))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn forget(path: &str) -> Result<bool, String> {
        match entry(path)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod imp {
    use super::*;
    const NA: &str = "Auf diesem System gibt es keinen unterstützten Schlüsselbund";
    pub fn store(_: &str, _: &str, _: Option<&str>) -> Result<(), String> {
        Err(NA.into())
    }
    pub fn load(_: &str) -> Result<Option<Found>, String> {
        Err(NA.into())
    }
    pub fn forget(_: &str) -> Result<bool, String> {
        Err(NA.into())
    }
}

pub use imp::{forget, load, store};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_names_the_database_path_and_stored_value_is_plain_json() {
        assert_eq!(account("/gibt/es/nicht.kdbx"), "tresor:/gibt/es/nicht.kdbx");
        let j = serde_json::to_string(&Stored { password: "p\"w".into(), key_file: Some("/k".into()) }).unwrap();
        let s: Stored = serde_json::from_str(&j).unwrap();
        assert_eq!((s.password.as_str(), s.key_file.as_deref()), ("p\"w", Some("/k")));
        assert!(!serde_json::to_string(&Stored { password: "x".into(), key_file: None }).unwrap().contains("key_file"));
    }

    /// Echter Schlüsselbund (Sitzungs-Bus nötig); legt einen Wegwerf-Eintrag an und entfernt ihn wieder.
    #[test]
    #[ignore = "braucht einen Schlüsselbund im Sitzungs-D-Bus; mit --ignored ausführbar"]
    fn roundtrip_with_the_real_keyring() {
        let path = format!("/tmp/remotejoe-schluesselbund-probe-{}.kdbx", std::process::id());
        assert!(load(&path).unwrap().is_none());
        store(&path, "Prüf-Kennwort ä\"1", Some("/tmp/probe.key")).unwrap();
        let f = load(&path).unwrap().expect("Eintrag vorhanden");
        assert_eq!((f.password.as_str(), f.key_file.as_deref()), ("Prüf-Kennwort ä\"1", Some("/tmp/probe.key")));
        assert!(forget(&path).unwrap());
        assert!(load(&path).unwrap().is_none());
        assert!(!forget(&path).unwrap(), "zweites Entfernen: nichts mehr da");
    }
}
