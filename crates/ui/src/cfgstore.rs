//! Konfigablage: verschlüsselte Ablage von Gerätekonfigurationen je Ordner in einer eigenen KDBX
//! (`daten/konfigsicherung/<Name> [<ID>].kdbx`, siehe [`rj_kdbx::configstore`]). Das Kennwort dieser Dateien liegt
//! als ein einziger Eintrag „Konfigsicherung“ in der Haupt-KDBX; er wird einmalig nach Rückfrage angelegt.
//! Erweiterungen bekommen Griffe auf geöffnete Ablagen, nie das Kennwort.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use rj_core::Secret;
use rj_kdbx::configstore::{random_password, ConfigStore};
use rj_kdbx::{CredentialData, KdfChoice};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::app::App;

/// Titel des Eintrags in der Haupt-KDBX.
pub const ENTRY_TITLE: &str = "Konfigsicherung";

#[derive(Default)]
pub struct CfgStores {
    open: Mutex<HashMap<u64, ConfigStore>>,
    next: AtomicU64,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Dateiname ohne Zeichen, die Windows nicht erlaubt.
fn safe_name(s: &str) -> String {
    let t: String = s.chars().map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect();
    let t = t.trim().trim_end_matches('.').to_string();
    if t.is_empty() { "Ablage".into() } else { t }
}

/// Datei des Ordners `folder` (Name `name`); eine vorhandene Datei mit derselben Kennung, aber altem Namen wird
/// umbenannt.
pub fn store_path(folder: Uuid, name: &str) -> Option<PathBuf> {
    let dir = rj_core::config::data_dir()?.join("konfigsicherung");
    let tag = format!(" [{}].kdbx", &folder.simple().to_string()[..8]);
    let want = dir.join(format!("{}{tag}", safe_name(name)));
    if !want.exists() {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().ends_with(&tag) {
                    let _ = std::fs::rename(e.path(), &want);
                    break;
                }
            }
        }
    }
    Some(want)
}

impl CfgStores {
    /// Geöffnete Ablage verwahren; liefert den Griff.
    pub fn put(&self, store: ConfigStore) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        lock(&self.open).insert(id, store);
        id
    }

    pub fn close(&self, handle: u64) {
        lock(&self.open).remove(&handle);
    }

    pub fn clear(&self) {
        lock(&self.open).clear();
    }

    /// Operation auf einer geöffneten Ablage (aus beliebigen Fäden).
    pub fn call(&self, op: &str, a: &Value) -> Result<Value, String> {
        let handle = a["handle"].as_u64().unwrap_or(0);
        let mut tab = lock(&self.open);
        let store = tab.get_mut(&handle).ok_or("Konfigablage nicht geöffnet")?;
        let device = || a["device"].as_str().and_then(|s| Uuid::parse_str(s).ok()).ok_or_else(|| "Gerät fehlt".to_string());
        match op {
            "cfgstore_add" => {
                let d = device()?;
                let s = |k: &str| a[k].as_str().unwrap_or_default();
                let changed = store.add(d, s("group"), s("title"), s("url"), s("series"), s("stamp"), s("reason"), s("text"), a["keep"].as_u64().unwrap_or(5) as usize);
                Ok(json!({ "changed": changed, "stands": store.stands(d).len() }))
            }
            "cfgstore_save" => store.save().map(|size| json!({ "size": size })).map_err(|e| e.to_string()),
            "cfgstore_devices" => Ok(json!(store.devices())),
            "cfgstore_stands" => {
                let d = device()?;
                Ok(json!(store.stands(d).into_iter().map(|s| json!({ "name": s.name, "size": s.size, "reason": s.reason })).collect::<Vec<_>>()))
            }
            "cfgstore_checked" => Ok(json!(store.checked(device()?))),
            "cfgstore_text" => Ok(json!(store.text(device()?, a["name"].as_str().unwrap_or_default()))),
            _ => Err(format!("Unbekannte Operation „{op}“")),
        }
    }
}

impl App {
    /// Eintrag „Konfigsicherung“ der Haupt-KDBX (in der Gruppe `remotejoe`): sein Kennwort.
    fn cfgstore_password(&self) -> Option<Secret> {
        let v = self.vault.as_ref()?;
        let e = v.entries().into_iter().find(|e| e.title == ENTRY_TITLE && e.group_path.first().map(String::as_str) == Some(rj_kdbx::GROUP_NAME))?;
        Some(v.credential(e.uuid)?.data.password)
    }

    /// Eintrag „Konfigsicherung“ anlegen, falls er fehlt (die Haupt-KDBX wird dabei gespeichert).
    pub(crate) fn cfgstore_ensure_entry(&mut self) -> Result<(), String> {
        if self.cfgstore_password().is_some() {
            return Ok(());
        }
        let pw = Secret::from_str_exact(&random_password());
        let v = self.vault.as_mut().ok_or("Die Schlüsseldatenbank ist nicht geöffnet")?;
        v.add_credential(CredentialData {
            title: ENTRY_TITLE.into(),
            username: "remotejoe".into(),
            password: pw,
            notes: Some("Kennwort der Konfigsicherungen je Ordner (Ordner daten/konfigsicherung). Nicht ändern oder löschen – sonst lassen sich die Sicherungen nicht mehr öffnen.".into()),
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
        self.save().map_err(|e| format!("Haupt-KDBX konnte nicht gespeichert werden: {e}"))?;
        Ok(())
    }

    /// Stand für die Oberfläche: Datei, ob sie schon existiert, ob der Eintrag in der Haupt-KDBX fehlt.
    pub(crate) fn cfgstore_state(&self, folder: Uuid, name: &str) -> Result<Value, String> {
        let path = store_path(folder, name).ok_or("Datenordner unbekannt")?;
        Ok(json!({ "path": path.to_string_lossy(), "exists": path.exists(), "needs_entry": self.cfgstore_password().is_none() }))
    }

    /// Ablage eines Ordners öffnen (`create`: auch anlegen); `None`, wenn es sie nicht gibt und nicht angelegt wird.
    pub(crate) fn cfgstore_open(&self, folder: Uuid, name: &str, create: bool) -> Result<Option<u64>, String> {
        let path = store_path(folder, name).ok_or("Datenordner unbekannt")?;
        if !path.exists() && !create {
            return Ok(None);
        }
        let pw = self.cfgstore_password().ok_or("Eintrag „Konfigsicherung“ fehlt in der KDBX")?;
        let store = ConfigStore::open_or_create(&path, &pw, KdfChoice::RandomKey, &format!("Konfigsicherung {name}")).map_err(|e| e.to_string())?;
        Ok(Some(self.cfgstores.put(store)))
    }
}
