//! Inventar eines Ziels: je Feld Wert, Quelle und Zeitpunkt.
//!
//! Ein neuer, abweichender Wert aus einer Abfrage ersetzt den bisherigen nicht, sondern wird
//! vorgemerkt und kann übernommen werden. Von Hand gesetzte Werte überschreibt die Automatik nie.
//! Abgelegt wird je Ziel eine Datei `inventar/<ID>.json` im Datenordner (nie in der KDBX).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Felder in Anzeigereihenfolge.
pub const FIELDS: [&str; 8] = ["name", "ip", "mac", "vcpu", "memory", "system", "uptime", "serial"];

/// Momentwerte: jede Abfrage überschreibt sie (kein Vorschlag „abweichender Wert“), außer von Hand gesetzt.
const VOLATILE: [&str; 1] = ["uptime"];

/// Quelle eines von Hand gesetzten Werts.
pub const MANUAL: &str = "von Hand";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub value: String,
    pub source: String,
    pub at: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub source: String,
    /// Zeitpunkt (Unix-Sekunden), zu dem der Wert zuletzt bestätigt wurde.
    #[serde(default)]
    pub at: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub manual: bool,
    /// Abweichender Wert aus einer späteren Abfrage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<Proposal>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    #[serde(default)]
    pub fields: BTreeMap<String, Field>,
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Inventory {
    /// Wert aus einer Abfrage anbieten: leeres Feld füllen, gleichen Wert bestätigen, abweichenden vormerken.
    pub fn offer(&mut self, key: &str, value: &str, source: &str, at: u64) {
        let value = value.trim();
        if value.is_empty() || !FIELDS.contains(&key) {
            return;
        }
        let f = self.fields.entry(key.to_string()).or_default();
        if f.value.is_empty() || (VOLATILE.contains(&key) && !f.manual) {
            *f = Field { value: value.to_string(), source: source.to_string(), at, manual: false, pending: None };
        } else if f.value == value {
            if !f.manual {
                f.source = source.to_string();
                f.at = at;
            }
            f.pending = None;
        } else {
            f.pending = Some(Proposal { value: value.to_string(), source: source.to_string(), at });
        }
    }

    /// Von Hand setzen; leer entfernt das Feld.
    pub fn set_manual(&mut self, key: &str, value: &str, at: u64) -> Result<(), String> {
        if !FIELDS.contains(&key) {
            return Err(format!("Unbekanntes Inventarfeld „{key}“"));
        }
        let value = value.trim();
        if value.is_empty() {
            self.fields.remove(key);
        } else {
            self.fields.insert(key.to_string(), Field { value: value.to_string(), source: MANUAL.into(), at, manual: true, pending: None });
        }
        Ok(())
    }

    /// Vorgemerkten Wert übernehmen (`true`) oder verwerfen (`false`).
    pub fn decide(&mut self, key: &str, accept: bool) -> bool {
        let Some(f) = self.fields.get_mut(key) else { return false };
        let Some(p) = f.pending.take() else { return false };
        if accept {
            *f = Field { value: p.value, source: p.source, at: p.at, manual: false, pending: None };
        }
        true
    }

    pub fn value(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(|f| f.value.as_str()).filter(|v| !v.is_empty())
    }
}

/// Geräteart aus dem Inventar, für ein eigenes Symbol im Baum: `aruba-ap`, sobald das Inventar ein Aruba-Gerät
/// meldet (System „Aruba Instant …“ aus `show version`).
pub fn device_kind(inv: &Inventory) -> Option<&'static str> {
    inv.value("system").filter(|s| s.starts_with("Aruba")).map(|_| "aruba-ap")
}

/// Ablage der Inventardateien.
#[derive(Debug, Clone)]
pub struct Store {
    pub dir: Option<PathBuf>,
}

impl Store {
    pub fn default_location() -> Self {
        Store { dir: rj_core::config::data_dir().map(|d| d.join("inventar")) }
    }

    /// Datei für einen Schlüssel: Ziel-ID oder `gast-<Hypervisor-ID>-<Gast>` (nur sichere Zeichen).
    fn path(&self, key: &str) -> Option<PathBuf> {
        let safe: String = key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
        self.dir.as_ref().map(|d| d.join(format!("{safe}.json")))
    }

    /// Gerätearten aller Ziele mit Inventardatei (einmal beim Start; Gäste haben keine UUID als Namen).
    pub fn scan_devices(&self) -> std::collections::HashMap<Uuid, &'static str> {
        let mut out = std::collections::HashMap::new();
        let Some(dir) = &self.dir else { return out };
        let Ok(entries) = std::fs::read_dir(dir) else { return out };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".json").and_then(|s| Uuid::parse_str(s).ok()) else { continue };
            if let Some(kind) = device_kind(&self.load(id)) {
                out.insert(id, kind);
            }
        }
        out
    }

    pub fn load(&self, id: Uuid) -> Inventory {
        self.load_key(&id.to_string())
    }

    pub fn save(&self, id: Uuid, inv: &Inventory) -> Result<(), String> {
        self.save_key(&id.to_string(), inv)
    }

    pub fn load_key(&self, key: &str) -> Inventory {
        self.path(key).and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save_key(&self, key: &str, inv: &Inventory) -> Result<(), String> {
        let p = self.path(key).ok_or("kein Datenordner")?;
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        if inv.fields.is_empty() {
            return match std::fs::remove_file(&p) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            };
        }
        let data = serde_json::to_vec_pretty(inv).map_err(|e| e.to_string())?;
        write_atomic(&p, &data).map_err(|e| e.to_string())
    }
}

fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

/// Bytes lesbar (Basis 1024, deutsches Komma), z. B. „7,8 GiB“.
pub fn format_bytes(b: u64) -> String {
    const U: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i + 1 < U.len() {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{:.1} {}", v, U[i]).replace('.', ",")
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn uptime_is_overwritten_other_fields_become_proposals() {
        let mut inv = Inventory::default();
        inv.offer("uptime", "1 T", "SSH", 1);
        inv.offer("uptime", "2 T", "SSH", 2);
        assert_eq!((inv.value("uptime"), inv.fields["uptime"].pending.is_none()), (Some("2 T"), true));
        inv.offer("serial", "CN1", "SSH", 1);
        inv.offer("serial", "CN2", "SSH", 2);
        assert_eq!(inv.value("serial"), Some("CN1"));
        assert!(inv.fields["serial"].pending.is_some(), "abweichende Seriennummer als Vorschlag");
        inv.set_manual("uptime", "von Hand", 3).unwrap();
        inv.offer("uptime", "3 T", "SSH", 4);
        assert_eq!(inv.value("uptime"), Some("von Hand"));
    }

    #[test]
    fn aruba_system_marks_the_device_kind() {
        let mut inv = Inventory::default();
        assert_eq!(device_kind(&inv), None);
        inv.offer("system", "Ubuntu 24.04", "SSH", 1);
        assert_eq!(device_kind(&inv), None);
        let mut ap = Inventory::default();
        ap.offer("system", "Aruba Instant 8.10.0.23 LSR (Modell 515)", "SSH (show version)", 1);
        assert_eq!(device_kind(&ap), Some("aruba-ap"));
        let dir = std::env::temp_dir().join(format!("rj-geraete-{}", Uuid::new_v4()));
        let store = Store { dir: Some(dir.clone()) };
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        store.save(a, &ap).unwrap();
        store.save(b, &inv).unwrap();
        store.save_key("gast-x-pve1/qemu/100", &ap).unwrap();
        let found = store.scan_devices();
        assert_eq!(found.len(), 1, "nur Ziele mit UUID, Gäste nicht");
        assert_eq!(found.get(&a), Some(&"aruba-ap"));
        let _ = std::fs::remove_dir_all(dir);
    }

    use super::*;

    #[test]
    fn offers_fill_confirm_or_wait_and_manual_values_stay() {
        let mut inv = Inventory::default();
        inv.offer("name", "srv01", "RDP", 10);
        assert_eq!(inv.fields["name"].value, "srv01");
        inv.offer("name", "srv01", "SSH", 20);
        assert_eq!((inv.fields["name"].source.as_str(), inv.fields["name"].at), ("SSH", 20), "gleicher Wert wird bestätigt");
        inv.offer("name", "srv02", "SSH", 30);
        assert_eq!(inv.fields["name"].value, "srv01", "abweichender Wert wird nur vorgemerkt");
        assert_eq!(inv.fields["name"].pending.as_ref().unwrap().value, "srv02");
        assert!(inv.decide("name", true));
        assert_eq!((inv.fields["name"].value.as_str(), inv.fields["name"].pending.is_none()), ("srv02", true));

        inv.set_manual("vcpu", "4", 40).unwrap();
        inv.offer("vcpu", "4", "SSH", 50);
        assert_eq!((inv.fields["vcpu"].source.as_str(), inv.fields["vcpu"].manual), (MANUAL, true), "von Hand bleibt von Hand");
        inv.offer("vcpu", "8", "SSH", 60);
        assert_eq!(inv.fields["vcpu"].value, "4");
        assert!(inv.decide("vcpu", false));
        assert_eq!((inv.fields["vcpu"].value.as_str(), inv.fields["vcpu"].pending.is_none()), ("4", true));

        inv.offer("unbekannt", "x", "SSH", 1);
        inv.offer("mac", "  ", "ARP", 1);
        assert!(!inv.fields.contains_key("unbekannt") && !inv.fields.contains_key("mac"));
        assert!(inv.set_manual("unbekannt", "x", 1).is_err());
        inv.set_manual("vcpu", "", 70).unwrap();
        assert!(!inv.fields.contains_key("vcpu"));
    }

    #[test]
    fn store_round_trip_and_empty_inventory_removes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store { dir: Some(dir.path().join("inventar")) };
        let id = Uuid::new_v4();
        assert_eq!(store.load(id), Inventory::default());
        let mut inv = Inventory::default();
        inv.offer("ip", "10.0.0.5", "DNS", 1);
        store.save(id, &inv).unwrap();
        assert_eq!(store.load(id), inv);
        store.save(id, &Inventory::default()).unwrap();
        assert!(!dir.path().join("inventar").join(format!("{id}.json")).exists());
        // Gäste: Schlüssel mit Schrägstrichen werden zu einem sicheren Dateinamen.
        store.save_key("gast-x-pve1/qemu/100", &inv).unwrap();
        assert!(dir.path().join("inventar").join("gast-x-pve1_qemu_100.json").exists());
        assert_eq!(store.load_key("gast-x-pve1/qemu/100"), inv);
    }

    #[test]
    fn bytes_are_shown_in_binary_units() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(8 * 1024 * 1024 * 1024), "8,0 GiB");
        assert_eq!(format_bytes(8_253_000 * 1024), "7,9 GiB");
    }
}
