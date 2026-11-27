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
    #[error("Diese Schlüsseldatenbank braucht remotejoe {needed} oder neuer (dieses Programm: {own}). Bitte remotejoe aktualisieren; die Daten bleiben unverändert.")]
    ProgramTooOld { needed: String, own: String },
}

/// Version dieses Programms (alle Kisten teilen die Version des Arbeitsbereichs).
pub const PROGRAM_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Name der Angabe im gespeicherten Index: älteste Programmversion, die diese Daten lesen kann.
pub const MIN_PROGRAM_KEY: &str = "min_program";

/// Älteste Programmversion, die `index` lesen kann – gesetzt, sobald die Daten einen neuen **Wert** in einem
/// bestehenden Feld enthalten (z. B. eine neue Ordnerrolle). Neue **Felder** brauchen das nicht: Ältere Fassungen
/// überlesen sie. Fassungen ab 0.1.68 prüfen die Angabe vor dem Einlesen und melden klar, welche Version nötig ist;
/// 0.1.67 und älter kennen sie nicht – neue Werte deshalb frühestens in der Fassung nach 0.1.68.
///
/// Ab 0.1.74 können auch Erweiterungen eine Mindestversion für ihre Daten vorschreiben: Angabe
/// [`MIN_PROGRAM_KEY`] („x.y.z“) unter den Angaben der Erweiterung an einem Ordner. Es gilt die höchste Angabe.
pub fn required_program(index: &Index) -> Option<String> {
    fn walk(f: &crate::model::Folder, best: &mut Option<String>) {
        // 0.1.69: Ordnerrolle „Zugangsdaten“ (`credentials`); ältere Fassungen scheitern sonst am unbekannten Wert.
        if f.role == Some(crate::model::FolderRole::Credentials) {
            raise(best, "0.1.69");
        }
        if let Some(v) = f.extra.get(MIN_PROGRAM_KEY).and_then(serde_json::Value::as_str).filter(|v| valid_version(v)) {
            raise(best, v);
        }
        for n in &f.children {
            match n {
                crate::model::Node::Folder(sub) => walk(sub, best),
                // 0.1.75: Zieltyp Telnet (auch als weiterer Zugangsweg); ältere Fassungen kennen den Wert nicht.
                crate::model::Node::Target(t) => {
                    if t.protocol == crate::model::Protocol::Telnet || t.ways.iter().any(|w| w.protocol == crate::model::Protocol::Telnet) {
                        raise(best, "0.1.75");
                    }
                }
            }
        }
    }
    fn raise(best: &mut Option<String>, v: &str) {
        if best.as_deref().is_none_or(|b| older(b, v)) {
            *best = Some(v.to_string());
        }
    }
    let mut best = None;
    walk(&index.root, &mut best);
    best
}

/// „x.y.z“ aus Ziffern (für Angaben von Erweiterungen).
fn valid_version(v: &str) -> bool {
    let p: Vec<&str> = v.split('.').collect();
    p.len() == 3 && p.iter().all(|x| !x.is_empty() && x.len() <= 6 && x.bytes().all(|b| b.is_ascii_digit()))
}

/// `a` älter als `b` (Versionen „x.y.z“, Vorabzusätze wie „-beta.3“ zählen nicht).
fn older(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> { v.split(['-', '+']).next().unwrap_or("").split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    parts(a) < parts(b)
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

/// Serialisiert einen Index (UTF-8, lesbar eingerückt). Ohne zusätzliche Zugangswege mit Schema 3, damit
/// ältere Fassungen die Datei weiter bearbeiten können; mit ihnen Schema 4 (ältere öffnen sie dann schreibgeschützt).
pub fn to_json(index: &Index) -> Result<Vec<u8>, SchemaError> {
    let mut value = serde_json::to_value(index)?;
    value["schema_version"] = Value::from(written_version(index));
    if let Some(min) = required_program(index) {
        value[MIN_PROGRAM_KEY] = Value::from(min);
    }
    Ok(serde_json::to_vec_pretty(&value)?)
}

/// Schema, mit dem `index` geschrieben wird.
pub fn written_version(index: &Index) -> u32 {
    fn any_ways(f: &crate::model::Folder) -> bool {
        f.children.iter().any(|n| match n {
            crate::model::Node::Target(t) => !t.ways.is_empty() || t.open_with.is_some(),
            crate::model::Node::Folder(sub) => any_ways(sub),
        })
    }
    fn any_own(f: &crate::model::Folder) -> bool {
        f.children.iter().any(|n| match n {
            crate::model::Node::Target(t) => t.ways.iter().any(|w| w.own.is_some()),
            crate::model::Node::Folder(sub) => any_own(sub),
        })
    }
    if any_own(&index.root) {
        SCHEMA_VERSION
    } else if any_ways(&index.root) {
        crate::model::SCHEMA_VERSION_WAYS
    } else {
        crate::model::SCHEMA_VERSION_WITHOUT_WAYS
    }
}

/// Liest einen Index aus Bytes, prüft UTF-8 und migriert ältere Versionen.
pub fn from_json(bytes: &[u8], source_name: &str) -> Result<LoadedIndex, SchemaError> {
    let text = validate_utf8(bytes, source_name)?;
    let mut value: Value = serde_json::from_str(text)?;
    // Zuerst: Braucht die Datei ein neueres Programm? Dann nicht einlesen (neue Werte würden das Einlesen scheitern
    // lassen), sondern klar melden.
    if let Some(min) = value.get(MIN_PROGRAM_KEY).and_then(Value::as_str) {
        if older(PROGRAM_VERSION, min) {
            return Err(SchemaError::ProgramTooOld { needed: min.to_string(), own: PROGRAM_VERSION.to_string() });
        }
    }
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
        // 3 → 4 ändert nichts an vorhandenen Daten (Schema 3 wird ohne Zugangswege weiter geschrieben): kein Hinweis.
        notice: (version < crate::model::SCHEMA_VERSION_WITHOUT_WAYS).then(|| {
            format!("Der Index wurde von Schema {version} auf {SCHEMA_VERSION} umgestellt.")
        }),
    })
}

/// Eine Migrationsstufe von `from` nach `from + 1`.
fn migrate_step(from: u32, value: Value) -> Result<Value, SchemaError> {
    match from {
        1 => migrate_v1_to_v2(value),
        // 2 → 3: neue, optionale Felder (Vorlagen, Favoriten, Beschreibungen,
        // Zugangsdaten-Art); vorhandene Daten bleiben unverändert.
        2 => Ok(value),
        // 3 → 4: zusätzliche Zugangswege je Ziel (optional); vorhandene Daten bleiben unverändert.
        3 => Ok(value),
        // 4 → 5: eigene Zugangsdaten an Zugangswegen (optional); vorhandene Daten bleiben unverändert.
        4 => Ok(value),
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

    #[test]
    fn schema_3_stays_until_the_first_access_way() {
        use crate::model::{AccessWay, Folder, Node, Protocol, Target, WayCredential};
        let mut idx = Index::default();
        let mut f = Folder::new("Standort");
        let mut t = Target::new("ap-01", Protocol::Ssh, "192.0.2.4");
        let tid = t.id;
        t.meta.notes = "Notiz".into();
        f.children.push(Node::Target(t));
        idx.root.children.push(Node::Folder(f));
        // Ohne Zugangswege: Schema 3, und ein erneutes Laden/Speichern ändert nichts.
        let v3 = to_json(&idx).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&v3).unwrap()["schema_version"], 3);
        let loaded = from_json(&v3, "test").unwrap();
        assert!(!loaded.read_only && loaded.notice.is_none(), "kein Hinweis bei Schema 3");
        assert_eq!(loaded.to_json_for_save().unwrap(), v3, "unverändert gespeichert");
        assert!(!String::from_utf8_lossy(&v3).contains("\"ways\""));
        // Mit einem Zugangsweg: Schema 4, Weg bleibt beim Laden erhalten.
        let mut idx2 = loaded.index.clone();
        if let Some(Node::Folder(f)) = idx2.root.children.first_mut() {
            if let Some(Node::Target(t)) = f.children.first_mut() {
                let mut w = AccessWay::new(Protocol::Web);
                w.host = "https://192.0.2.4:4343".into();
                w.credential = WayCredential::None;
                t.open_with = Some(w.id);
                t.ways.push(w);
            }
        }
        let v4 = to_json(&idx2).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&v4).unwrap()["schema_version"], 4);
        let back = from_json(&v4, "test").unwrap();
        assert!(!back.read_only);
        let Some(Node::Target(t)) = crate::edit::node(&back.index, tid) else { panic!("Ziel fehlt") };
        assert_eq!(t.ways.len(), 1);
        assert_eq!(t.ways[0].credential, WayCredential::None);
        assert_eq!(t.open_with, Some(t.ways[0].id));
        let via = t.via(t.ways[0].id).unwrap();
        assert_eq!((via.protocol, via.host.as_str(), via.port, via.id), (Protocol::Web, "192.0.2.4", Some(4343), tid));
        assert!(matches!(&via.options, ProtocolOptions::Web(o) if o.url.as_deref() == Some("https://192.0.2.4:4343")));
        assert!(via.ways.is_empty());
        // Ein Programm, das nur Schema 3 kennt, öffnet Schema 4 schreibgeschützt (bestehende Regel für neuere Dateien).
        assert!(SCHEMA_VERSION == 5);
    }

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
                hops: vec![SshHop { host: "jump.example.test".into(), port: Some(2222), credential: None, legacy_algorithms: false }],
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

    #[test]
    fn example_in_documentation_is_valid() {
        let doc = include_str!("../../../docs/DATENMODELL.md");
        let start = doc.find("```json\n").expect("JSON-Beispiel") + 8;
        let end = start + doc[start..].find("```").expect("Ende");
        let loaded = from_json(doc[start..end].as_bytes(), "DATENMODELL.md").unwrap();
        let idx = &loaded.index;
        assert_eq!(idx.revision, 7);
        let mut n = 0;
        idx.for_each_target(|_| n += 1);
        assert_eq!(n, 2);
    }

    /// Mindest-Programmversion: Ein zu altes Programm liest die Daten nicht ein, sondern meldet klar die nötige Version
    /// – auch wenn sie Werte enthalten, die es nicht kennt. Ein ausreichend neues Programm liest normal.
    #[test]
    fn data_needing_a_newer_program_is_refused_with_a_clear_message() {
        let mut v: Value = serde_json::from_slice(&to_json(&sample()).unwrap()).unwrap();
        assert!(v.get(MIN_PROGRAM_KEY).is_none(), "ohne neue Werte keine Angabe");
        v[MIN_PROGRAM_KEY] = Value::from("99.0.0");
        v["root"]["role"] = Value::from("unbekannte_rolle_aus_der_zukunft");
        let e = from_json(v.to_string().as_bytes(), "t").unwrap_err().to_string();
        assert!(e.contains("braucht remotejoe 99.0.0 oder neuer") && e.contains(PROGRAM_VERSION), "{e}");
        let mut ok: Value = serde_json::from_slice(&to_json(&sample()).unwrap()).unwrap();
        ok[MIN_PROGRAM_KEY] = Value::from("0.1.0");
        assert!(from_json(ok.to_string().as_bytes(), "t").is_ok());
        assert!(older("0.1.67", "0.1.68") && !older("0.1.68", "0.1.68") && !older("0.1.69-beta.2", "0.1.68") && older("0.1.9", "0.1.10"));
    }

    /// Ordnerrolle „Zugangsdaten“ (neu in 0.1.69): Wer sie benutzt, schreibt „braucht 0.1.69“ in die Datei.
    #[test]
    fn credentials_folder_requires_0_1_69() {
        let mut idx = sample();
        let mut f = crate::model::Folder::new("Zugangsdaten");
        f.role = Some(crate::model::FolderRole::Credentials);
        idx.root.children.push(crate::model::Node::Folder(f));
        let v: Value = serde_json::from_slice(&to_json(&idx).unwrap()).unwrap();
        assert_eq!(v[MIN_PROGRAM_KEY], "0.1.69");
        let back = from_json(&to_json(&idx).unwrap(), "t").unwrap();
        assert_eq!(required_program(&back.index).as_deref(), Some("0.1.69"));
    }

    /// Zieltyp Telnet (neu in 0.1.75), auch als weiterer Zugangsweg: Die Datei verlangt dann 0.1.75.
    #[test]
    fn telnet_target_requires_0_1_75() {
        let mut idx = sample();
        assert_ne!(required_program(&idx).as_deref(), Some("0.1.75"));
        idx.root.children.push(crate::model::Node::Target(crate::model::Target::new("alt", crate::model::Protocol::Telnet, "192.0.2.30")));
        let v: Value = serde_json::from_slice(&to_json(&idx).unwrap()).unwrap();
        assert_eq!(v[MIN_PROGRAM_KEY], "0.1.75");
        let back = from_json(&to_json(&idx).unwrap(), "t").unwrap();
        assert_eq!(back.index.root.children.iter().filter(|n| matches!(n, crate::model::Node::Target(t) if t.protocol == crate::model::Protocol::Telnet)).count(), 1);
        assert_eq!(crate::model::Protocol::Telnet.default_port(), 23);
        assert!(crate::model::Protocol::Telnet.is_terminal() && crate::model::Protocol::Ssh.is_terminal() && !crate::model::Protocol::Rdp.is_terminal());
    }

    /// Erweiterungsdaten an einem Ordner schreiben ihre Mindestversion vor; die höchste Angabe gilt, ungültige zählen nicht.
    #[test]
    fn extension_data_can_require_a_program_version() {
        let mut idx = sample();
        let mut a = crate::model::Folder::new("A");
        a.extra.insert(MIN_PROGRAM_KEY.into(), Value::from("0.1.74"));
        let mut b = crate::model::Folder::new("B");
        b.extra.insert(MIN_PROGRAM_KEY.into(), Value::from("0.1.70"));
        let mut c = crate::model::Folder::new("C");
        c.extra.insert(MIN_PROGRAM_KEY.into(), Value::from("kaputt"));
        b.children.push(crate::model::Node::Folder(c));
        idx.root.children.push(crate::model::Node::Folder(a));
        idx.root.children.push(crate::model::Node::Folder(b));
        let v: Value = serde_json::from_slice(&to_json(&idx).unwrap()).unwrap();
        assert_eq!(v[MIN_PROGRAM_KEY], "0.1.74");
        let mut f = crate::model::Folder::new("Zugangsdaten");
        f.role = Some(crate::model::FolderRole::Credentials);
        idx.root.children.push(crate::model::Node::Folder(f));
        assert_eq!(required_program(&idx).as_deref(), Some("0.1.74"));
        assert!(!valid_version("0.1") && !valid_version("0.1.x") && valid_version("1.40.0"));
    }

}

