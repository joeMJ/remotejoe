//! Konfigablage: eine eigene KDBX je Ordner mit Konfigurationsständen von Geräten (von Erweiterungen gefüllt).
//!
//! Je Gerät ein Eintrag (Kennung = Ziel-ID im Index) in der Gruppe „Switches“ bzw. „Controller“; jeder Stand ist ein
//! Anhang `JJJJ-MM-TT_hh-mm-ss.txt`, sein Anlass ein Zusatzfeld `Anlass JJJJ-MM-TT_hh-mm-ss`. Bleibt die
//! Konfiguration gleich, entsteht kein neuer Stand; nur „Zuletzt geprüft“ wird gesetzt. Es bleiben höchstens `keep`
//! Stände, der älteste fällt weg. Die Datei ist auch in KeePassXC lesbar (Kennwort aus dem Eintrag
//! „Konfigsicherung“ der Haupt-KDBX).

use std::fs;
use std::path::{Path, PathBuf};

use keepass::config::DatabaseConfig;
use keepass::db::{fields, EntryId, GroupId, Value};
use keepass::Database;
use rj_core::fsutil::{atomic_write_locked, WriteLock};
use rj_core::Secret;
use uuid::Uuid;

use crate::{KdfChoice, MasterKey, VaultError};

/// Feld mit dem Zeitpunkt der letzten Prüfung (auch ohne neuen Stand).
pub const CHECKED: &str = "Zuletzt geprüft";
const REASON: &str = "Anlass ";
const SUFFIX: &str = ".txt";

/// Ein gespeicherter Stand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stand {
    /// Name des Anhangs, z. B. `2026-11-16_10-42-05.txt`.
    pub name: String,
    pub size: usize,
    pub reason: String,
}

/// Geöffnete (bzw. neue) Ablage-KDBX eines Ordners.
pub struct ConfigStore {
    path: PathBuf,
    key: MasterKey,
    db: Database,
}

/// Zufallskennwort für die Ablage-KDBX (32 Byte, Base64).
pub fn random_password() -> String {
    use base64::Engine;
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).expect("Zufallszahlen");
    base64::engine::general_purpose::STANDARD_NO_PAD.encode(b)
}

/// Text zum Vergleichen: Zeilenenden ohne Leerraum, ohne Leerzeilen am Ende.
fn normalized(text: &str) -> String {
    let mut lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

impl ConfigStore {
    /// Öffnet die Datei; gibt es sie nicht, eine neue (erst mit [`save`](Self::save) geschrieben).
    pub fn open_or_create(path: &Path, password: &Secret, kdf: KdfChoice, title: &str) -> Result<ConfigStore, VaultError> {
        let key = MasterKey { password: Some(password.clone()), key_file: None };
        let db = if path.exists() {
            Database::parse(&fs::read(path)?, key.to_db_key()?)?
        } else {
            let mut config = DatabaseConfig::default();
            config.kdf_config = kdf.config();
            let mut db = Database::with_config(config);
            db.root_mut().name = title.to_string();
            db.meta.generator = Some("remotejoe".into());
            db.meta.database_name = Some(title.to_string());
            db
        };
        Ok(ConfigStore { path: path.to_path_buf(), key, db })
    }

    fn group(&mut self, name: &str) -> GroupId {
        if let Some(g) = self.db.root().group_by_name(name) {
            return g.id();
        }
        let mut root = self.db.root_mut();
        let mut g = root.add_group();
        g.name = name.to_string();
        g.id()
    }

    /// Kennungen aller Geräte mit Eintrag.
    pub fn devices(&self) -> Vec<Uuid> {
        self.db.iter_all_entries().map(|e| e.id().uuid()).collect()
    }

    /// Stände eines Geräts, neueste zuerst.
    pub fn stands(&self, device: Uuid) -> Vec<Stand> {
        let Some(e) = self.db.entry(EntryId::from_uuid(device)) else { return Vec::new() };
        let mut out: Vec<Stand> = e
            .attachments_named()
            .filter(|(n, _)| n.ends_with(SUFFIX))
            .map(|(n, a)| Stand {
                name: n.to_string(),
                size: a.data.get().len(),
                reason: e.get(&format!("{REASON}{}", n.trim_end_matches(SUFFIX))).unwrap_or_default().to_string(),
            })
            .collect();
        out.sort_by(|a, b| b.name.cmp(&a.name));
        out
    }

    /// Text eines Stands.
    pub fn text(&self, device: Uuid, name: &str) -> Option<String> {
        let e = self.db.entry(EntryId::from_uuid(device))?;
        let a = e.attachment_by_name(name)?;
        Some(String::from_utf8_lossy(a.data.get()).into_owned())
    }

    /// Zeitpunkt der letzten Prüfung.
    pub fn checked(&self, device: Uuid) -> Option<String> {
        self.db.entry(EntryId::from_uuid(device))?.get(CHECKED).map(str::to_string)
    }

    /// Neuen Stand ablegen (`stamp` wie `2026-11-16_10-42-05`). Gleicht er dem neuesten, wird nur „Zuletzt geprüft“
    /// gesetzt und `false` zurückgegeben. Danach höchstens `keep` Stände.
    #[allow(clippy::too_many_arguments)]
    pub fn add(&mut self, device: Uuid, group: &str, title: &str, url: &str, series: &str, stamp: &str, reason: &str, text: &str, keep: usize) -> bool {
        let id = EntryId::from_uuid(device);
        let newest = self.stands(device).first().and_then(|s| self.text(device, &s.name));
        let changed = newest.is_none_or(|n| normalized(&n) != normalized(text));
        if self.db.entry(id).is_none() {
            let gid = self.group(group);
            let mut g = self.db.group_mut(gid).expect("Gruppe vorhanden");
            let _ = g.add_entry_with_id(id);
        }
        let old: Vec<String> = self.stands(device).into_iter().map(|s| s.name).collect();
        let mut e = self.db.entry_mut(id).expect("Eintrag vorhanden");
        e.set_unprotected(fields::TITLE, title);
        e.set_unprotected(fields::URL, url);
        e.set_unprotected(fields::NOTES, series);
        e.set_unprotected(CHECKED, stamp.replacen('_', " ", 1));
        if changed {
            let name = format!("{stamp}{SUFFIX}");
            e.add_attachment(name, Value::protected(text.as_bytes().to_vec()));
            e.set_unprotected(format!("{REASON}{stamp}"), reason);
            // Älteste über `keep` hinaus entfernen (`old` ist neueste zuerst, ohne den neuen).
            for n in old.iter().skip(keep.saturating_sub(1)) {
                e.remove_attachment_by_name(n);
                e.fields.remove(&format!("{REASON}{}", n.trim_end_matches(SUFFIX)));
            }
        }
        e.times.last_modification = Some(chrono::Utc::now().naive_utc());
        changed
    }

    /// Schreibt die Datei (atomar, mit Sperre); liefert ihre Größe in Byte.
    pub fn save(&mut self) -> Result<u64, VaultError> {
        if let Some(dir) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(dir)?;
        }
        let _lock = WriteLock::try_acquire(&self.path)?;
        let mut out = Vec::new();
        self.db.save(&mut out, self.key.to_db_key()?)?;
        atomic_write_locked(&self.path, &out)?;
        Ok(out.len() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stands_are_kept_compared_and_reopened() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Ordner Test [12345678].kdbx");
        let pw = Secret::from_str_exact(&random_password());
        let dev = Uuid::from_u128(7);
        let mut s = ConfigStore::open_or_create(&path, &pw, KdfChoice::Weak, "Konfigsicherung Ordner Test").unwrap();
        assert!(s.add(dev, "Switches", "SW-EG", "ssh://192.0.2.240", "Serie-A", "2026-11-16_10-00-00", "manuell", "hostname SW-EG\nvlan 1\n", 5));
        // Gleich bis auf Leerraum am Zeilenende: kein neuer Stand.
        assert!(!s.add(dev, "Switches", "SW-EG", "ssh://192.0.2.240", "Serie-A", "2026-11-16_10-05-00", "manuell", "hostname SW-EG  \nvlan 1\n\n", 5));
        assert_eq!(s.stands(dev).len(), 1);
        assert_eq!(s.checked(dev).as_deref(), Some("2026-11-16 10-05-00"));
        for i in 1..=6 {
            assert!(s.add(dev, "Switches", "SW-EG", "ssh://192.0.2.240", "Serie-A", &format!("2026-11-16_11-0{i}-00"), "vor VLAN-Änderung", &format!("hostname SW-EG\nvlan {}\n", 10 + i), 5));
        }
        let st = s.stands(dev);
        assert_eq!(st.len(), 5);
        assert_eq!(st[0].name, "2026-11-16_11-06-00.txt");
        assert_eq!(st[0].reason, "vor VLAN-Änderung");
        assert_eq!(st[4].name, "2026-11-16_11-02-00.txt");
        s.save().unwrap();
        let again = ConfigStore::open_or_create(&path, &pw, KdfChoice::Weak, "x").unwrap();
        assert_eq!(again.stands(dev), st);
        assert_eq!(again.devices(), vec![dev]);
        assert_eq!(again.text(dev, "2026-11-16_11-06-00.txt").as_deref(), Some("hostname SW-EG\nvlan 16\n"));
        // Falsches Kennwort öffnet nicht.
        assert!(ConfigStore::open_or_create(&path, &Secret::from_str_exact("falsch"), KdfChoice::Weak, "x").is_err());
    }
}
