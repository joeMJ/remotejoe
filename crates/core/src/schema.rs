//! Lesen, Schreiben und Migration des Index-Dokuments (JSON).

use serde_json::Value;

use crate::charset::{validate_utf8, Utf8Error};
use crate::model::{Index, SCHEMA_VERSION};

#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error(transparent)]
    Utf8(#[from] Utf8Error),
    #[error("Index ist kein gültiges JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Index enthält kein gültiges Feld schema_version")]
    MissingVersion,
    #[error("Migration von Version {from} fehlgeschlagen: {reason}")]
    Migration { from: u32, reason: String },
    #[error("Index ist schreibgeschützt geöffnet (Schema-Version {found} ist neuer als {supported})")]
    ReadOnly { found: u32, supported: u32 },
}

/// Ein geladener Index mit Angaben zur Herkunft.
#[derive(Debug, Clone)]
pub struct LoadedIndex {
    pub index: Index,
    /// Schema-Version, die in der Datei stand.
    pub original_version: u32,
    /// Die Datei stammt aus einer neueren Programmversion und darf nicht
    /// überschrieben werden.
    pub read_only: bool,
    /// Hinweis für die Oberfläche.
    pub notice: Option<String>,
}

impl LoadedIndex {
    /// Serialisiert den Index zum Speichern; verweigert bei Schreibschutz.
    pub fn to_json_for_save(&self) -> Result<Vec<u8>, SchemaError> {
        if self.read_only {
            return Err(SchemaError::ReadOnly {
                found: self.original_version,
                supported: SCHEMA_VERSION,
            });
        }
        to_json(&self.index)
    }
}

/// Serialisiert einen Index (UTF-8, lesbar eingerückt).
pub fn to_json(index: &Index) -> Result<Vec<u8>, SchemaError> {
    Ok(serde_json::to_vec_pretty(index)?)
}

/// Liest einen Index aus Bytes, prüft UTF-8 und migriert ältere Versionen.
pub fn from_json(bytes: &[u8], source_name: &str) -> Result<LoadedIndex, SchemaError> {
    let text = validate_utf8(bytes, source_name)?;
    let mut value: Value = serde_json::from_str(text)?;
    let version = value
        .get("schema_version")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or(SchemaError::MissingVersion)?;

    if version > SCHEMA_VERSION {
        // Unbekannte neuere Fassung: so gut wie möglich lesen, nie schreiben.
        let index: Index = serde_json::from_value(value)?;
        return Ok(LoadedIndex {
            index,
            original_version: version,
            read_only: true,
            notice: Some(format!(
                "Der Index stammt aus einer neueren Programmversion (Schema {version}). \
                 Er wurde schreibgeschützt geöffnet."
            )),
        });
    }

    let mut v = version;
    while v < SCHEMA_VERSION {
        value = migrate_step(v, value)?;
        v += 1;
    }
    value["schema_version"] = Value::from(SCHEMA_VERSION);
    let index: Index = serde_json::from_value(value)?;
    Ok(LoadedIndex {
        index,
        original_version: version,
        read_only: false,
        notice: (version < SCHEMA_VERSION).then(|| {
            format!("Der Index wurde von Schema {version} auf {SCHEMA_VERSION} umgestellt.")
        }),
    })
}

/// Eine Migrationsstufe von `from` nach `from + 1`.
fn migrate_step(from: u32, value: Value) -> Result<Value, SchemaError> {
    match from {
        1 => migrate_v1_to_v2(value),
        _ => Err(SchemaError::Migration {
            from,
            reason: "keine Migration bekannt".into(),
        }),
    }
}

/// Schema 1 → 2:
/// - `credentials` war eine Abbildung Name → UUID-Zeichenkette; jetzt Name →
///   `{ "entry": UUID, "last_title": … }`.
/// - Ordner und Ziele trugen Einstellungen direkt (`credentials`, `tunnel`);
///   jetzt liegen sie unter `settings`.
/// - `views` ist neu.
fn migrate_v1_to_v2(mut value: Value) -> Result<Value, SchemaError> {
    fn convert_node(node: &mut Value) -> Result<(), SchemaError> {
        let obj = node.as_object_mut().ok_or_else(|| SchemaError::Migration {
            from: 1,
            reason: "Knoten ist kein Objekt".into(),
        })?;
        let mut settings = serde_json::Map::new();
        if let Some(Value::Object(creds)) = obj.remove("credentials") {
            let mut new = serde_json::Map::new();
            for (name, uuid) in creds {
                let uuid = uuid.as_str().ok_or_else(|| SchemaError::Migration {
                    from: 1,
                    reason: format!("Verweis {name} ist keine Zeichenkette"),
                })?;
                new.insert(name, serde_json::json!({ "entry": uuid }));
            }
            settings.insert("credentials".into(), Value::Object(new));
        }
        if let Some(t) = obj.remove("tunnel") {
            settings.insert("tunnel".into(), t);
        }
        if !settings.is_empty() {
            obj.insert("settings".into(), Value::Object(settings));
        }
        if let Some(Value::Array(children)) = obj.get_mut("children") {
            for c in children {
                convert_node(c)?;
            }
        }
        Ok(())
    }

    if let Some(root) = value.get_mut("root") {
        convert_node(root)?;
    }
    if value.get("views").is_none() {
        value["views"] = Value::Array(Vec::new());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use uuid::Uuid;

    fn sample() -> Index {
        let mut idx = Index::default();
        let entry = Uuid::new_v4();
        idx.root.settings.credentials.insert(
            "default".into(),
            CredentialRef { entry, last_title: Some("Verwaltung".into()) },
        );
        let mut f = Folder::new("Büro 🏢");
        f.settings.tunnel = Some(TunnelSetting::Use(Uuid::new_v4()));
        let mut t = Target::new("srv-01", Protocol::Rdp, "srv01.example.test");
        t.port = Some(3390);
        t.options = ProtocolOptions::Rdp(RdpOptions {
            clipboard: ClipboardDirection::HostToTarget,
            ..Default::default()
        });
        t.settings.tunnel = Some(TunnelSetting::None);
        t.settings.overrides.insert(
            "default".into(),
            FieldOverrides { username: None, domain: Some("EXAMPLE".into()) },
        );
        let mut s = Target::new("linux", Protocol::Ssh, "192.0.2.10");
        s.credential = Some("admin".into());
        s.settings.psrp_transport = Some(PsrpTransport::Ssh { port: None });
        f.children.push(Node::Target(t.clone()));
        f.children.push(Node::Target(s));
        idx.root.children.push(Node::Folder(f));
        idx.views.push(View { id: Uuid::new_v4(), name: "Patchday".into(), targets: vec![t.id] });
        idx.tunnels.push(TunnelDef {
            id: Uuid::new_v4(),
            name: "Sprung".into(),
            kind: TunnelKind::Ssh {
                hops: vec![SshHop { host: "jump.example.test".into(), port: Some(2222), credential: None }],
            },
        });
        idx
    }

    #[test]
    fn roundtrip() {
        let idx = sample();
        let bytes = to_json(&idx).unwrap();
        let loaded = from_json(&bytes, "test").unwrap();
        assert!(!loaded.read_only);
        assert!(loaded.notice.is_none());
        assert_eq!(loaded.index, idx);
        // Zweiter Durchlauf liefert identische Bytes (stabile Reihenfolge).
        assert_eq!(to_json(&loaded.index).unwrap(), bytes);
    }

    #[test]
    fn tunnel_none_is_string() {
        let bytes = to_json(&sample()).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\"tunnel\": \"none\""));
    }

    #[test]
    fn rejects_invalid_utf8() {
        let mut bytes = br#"{"schema_version":2,"root":{"id":"00000000-0000-0000-0000-000000000000","name":""#.to_vec();
        bytes.extend_from_slice(&[0xFF, b'"', b'}', b'}']);
        assert!(matches!(from_json(&bytes, "idx"), Err(SchemaError::Utf8(_))));
    }

    #[test]
    fn migrates_v1() {
        let entry = Uuid::new_v4();
        let tunnel = Uuid::new_v4();
        let v1 = serde_json::json!({
            "schema_version": 1,
            "root": {
                "id": Uuid::new_v4(),
                "name": "Wurzel",
                "credentials": { "default": entry.to_string() },
                "children": [
                    { "kind": "folder", "id": Uuid::new_v4(), "name": "Netz", "tunnel": tunnel.to_string(),
                      "children": [
                        { "kind": "target", "id": Uuid::new_v4(), "name": "a", "protocol": "ssh",
                          "host": "a.example.test", "tunnel": "none" }
                      ] }
                ]
            }
        });
        let loaded = from_json(&serde_json::to_vec(&v1).unwrap(), "v1").unwrap();
        assert_eq!(loaded.original_version, 1);
        assert!(!loaded.read_only);
        assert!(loaded.notice.is_some());
        let idx = loaded.index;
        assert_eq!(idx.schema_version, SCHEMA_VERSION);
        assert_eq!(idx.root.settings.credentials["default"].entry, entry);
        let Node::Folder(f) = &idx.root.children[0] else { panic!() };
        assert_eq!(f.settings.tunnel, Some(TunnelSetting::Use(tunnel)));
        let Node::Target(t) = &f.children[0] else { panic!() };
        assert_eq!(t.settings.tunnel, Some(TunnelSetting::None));
        assert!(idx.views.is_empty());
    }

    #[test]
    fn newer_version_is_read_only() {
        let mut v = serde_json::to_value(sample()).unwrap();
        v["schema_version"] = (SCHEMA_VERSION + 1).into();
        v["future_field"] = "egal".into();
        let loaded = from_json(&serde_json::to_vec(&v).unwrap(), "neu").unwrap();
        assert!(loaded.read_only);
        assert!(loaded.notice.as_deref().unwrap().contains("schreibgeschützt"));
        assert!(matches!(loaded.to_json_for_save(), Err(SchemaError::ReadOnly { .. })));
    }

    #[test]
    fn missing_version_fails() {
        assert!(matches!(from_json(b"{}", "x"), Err(SchemaError::MissingVersion)));
    }

    #[test]
    fn path_and_references() {
        let idx = sample();
        let Node::Folder(f) = &idx.root.children[0] else { panic!() };
        let tid = f.children[1].id();
        let path = idx.path_to(tid).unwrap();
        assert_eq!(path.len(), 2);
        assert_eq!(path[0].name(), "Büro 🏢");
        assert_eq!(idx.referenced_entries().len(), 1);
    }
}
