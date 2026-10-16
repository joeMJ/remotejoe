//! Ablage des Index als Datenbank-`CustomData` in der KDBX.
//!
//! Schlüssel:
//! - `remotejoe.index` – JSON als UTF-8, wenn höchstens 64 KiB groß;
//! - sonst `remotejoe.index.0`, `remotejoe.index.1`, … und
//!   `remotejoe.index.chunks` (Anzahl der Blöcke);
//! - `remotejoe.index.sha256` – Prüfsumme über das gesamte JSON (hex).

use keepass::db::{CustomDataItem, CustomDataValue};
use keepass::Database;
use rj_core::schema::{from_json, LoadedIndex, SchemaError};
use sha2::{Digest, Sha256};

pub const KEY: &str = "remotejoe.index";
pub const KEY_SHA: &str = "remotejoe.index.sha256";
pub const KEY_CHUNKS: &str = "remotejoe.index.chunks";
pub const CHUNK_SIZE: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum IndexStoreError {
    #[error("Index in der KDBX ist unvollständig: Block {0} fehlt")]
    MissingChunk(usize),
    #[error("Prüfsumme des Index stimmt nicht (Datei beschädigt oder von fremdem Programm verändert)")]
    Checksum,
    #[error("Index in der KDBX ist kein Text")]
    NotText,
    #[error(transparent)]
    Schema(#[from] SchemaError),
}

/// Liest einen Textwert. Die KDBX speichert `CustomData` als Text; die
/// Bibliothek liefert Werte, die zufällig gültiges Base64 sind, dekodiert
/// zurück. Diese werden wieder kodiert und ergeben so genau den gespeicherten
/// Text (das Dekodieren lehnt nicht-kanonisches Base64 ab).
fn get_str(db: &Database, key: &str) -> Result<Option<String>, IndexStoreError> {
    use base64::Engine;
    match db.meta.custom_data.get(key).and_then(|i| i.value.as_ref()) {
        None => Ok(None),
        Some(CustomDataValue::String(s)) => Ok(Some(s.clone())),
        Some(CustomDataValue::Binary(b)) => Ok(Some(base64::engine::general_purpose::STANDARD.encode(b))),
    }
}

fn set_str(db: &mut Database, key: &str, value: String) {
    db.meta.custom_data.insert(
        key.to_string(),
        CustomDataItem {
            value: Some(CustomDataValue::String(value)),
            last_modification_time: Some(chrono::Utc::now().naive_utc()),
        },
    );
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Liest den rohen JSON-Text des Index; `None`, wenn keiner vorhanden ist.
pub fn read_raw(db: &Database) -> Result<Option<String>, IndexStoreError> {
    let json = if let Some(s) = get_str(db, KEY)? {
        s
    } else if let Some(n) = get_str(db, KEY_CHUNKS)? {
        let n: usize = n.parse().map_err(|_| IndexStoreError::MissingChunk(0))?;
        let mut out = String::new();
        for i in 0..n {
            out.push_str(&get_str(db, &format!("{KEY}.{i}"))?.ok_or(IndexStoreError::MissingChunk(i))?);
        }
        out
    } else {
        return Ok(None);
    };
    match get_str(db, KEY_SHA)? {
        Some(sum) if sum == hex(&Sha256::digest(json.as_bytes())) => Ok(Some(json)),
        _ => Err(IndexStoreError::Checksum),
    }
}

/// Liest den Index samt Prüfsumme.
pub fn read(db: &Database) -> Result<Option<(LoadedIndex, [u8; 32])>, IndexStoreError> {
    let Some(json) = read_raw(db)? else { return Ok(None) };
    let sha: [u8; 32] = Sha256::digest(json.as_bytes()).into();
    Ok(Some((from_json(json.as_bytes(), "KDBX CustomData remotejoe.index")?, sha)))
}

/// Zerlegt Text in Blöcke von höchstens `max` Bytes an Zeichengrenzen.
pub fn split_chunks(s: &str, max: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = s;
    while !rest.is_empty() {
        let mut end = rest.len().min(max);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        out.push(&rest[..end]);
        rest = &rest[end..];
    }
    out
}

fn remove_index_keys(db: &mut Database) {
    db.meta
        .custom_data
        .retain(|k, _| !(k == KEY || k == KEY_SHA || k == KEY_CHUNKS || k.strip_prefix(KEY).and_then(|r| r.strip_prefix('.')).is_some_and(|r| r.parse::<usize>().is_ok())));
}

/// Schreibt den Index (JSON-Bytes, gültiges UTF-8) in die Datenbank.
pub fn write(db: &mut Database, json: &[u8]) -> Result<(), IndexStoreError> {
    let text = std::str::from_utf8(json).map_err(|_| IndexStoreError::NotText)?;
    remove_index_keys(db);
    if text.len() <= CHUNK_SIZE {
        set_str(db, KEY, text.to_string());
    } else {
        let chunks = split_chunks(text, CHUNK_SIZE);
        for (i, c) in chunks.iter().enumerate() {
            set_str(db, &format!("{KEY}.{i}"), c.to_string());
        }
        set_str(db, KEY_CHUNKS, chunks.len().to_string());
    }
    set_str(db, KEY_SHA, hex(&Sha256::digest(json)));
    Ok(())
}

/// Legt eine Sicherung des Index unter `remotejoe.backup.<name>` ab.
pub fn write_backup(db: &mut Database, name: &str, json: &[u8]) -> Result<(), IndexStoreError> {
    let text = std::str::from_utf8(json).map_err(|_| IndexStoreError::NotText)?;
    set_str(db, &format!("remotejoe.backup.{name}"), text.to_string());
    Ok(())
}

/// Liest eine Sicherung des Index.
pub fn read_backup(db: &Database, name: &str) -> Option<String> {
    get_str(db, &format!("remotejoe.backup.{name}")).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_respect_char_boundaries() {
        let s = "aä🚀".repeat(1000);
        let parts = split_chunks(&s, 7);
        assert!(parts.iter().all(|p| p.len() <= 7 && !p.is_empty()));
        assert_eq!(parts.concat(), s);
    }

    #[test]
    fn small_and_large_roundtrip() {
        for size in [10, 200_000] {
            let mut db = Database::new();
            let mut idx = rj_core::Index::default();
            idx.root.name = "x".repeat(size);
            let json = rj_core::schema::to_json(&idx).unwrap();
            write(&mut db, &json).unwrap();
            assert_eq!(db.meta.custom_data.contains_key(KEY), size < CHUNK_SIZE);
            let (back, _) = read(&db).unwrap().unwrap();
            assert_eq!(back.index, idx);
        }
    }

    #[test]
    fn rewrite_removes_stale_chunks() {
        let mut db = Database::new();
        write(&mut db, "x".repeat(CHUNK_SIZE * 3).as_bytes()).unwrap();
        write(&mut db, b"{}").unwrap();
        assert_eq!(db.meta.custom_data.len(), 2);
    }

    #[test]
    fn checksum_detects_tampering() {
        let mut db = Database::new();
        write(&mut db, br#"{"schema_version":2}"#).unwrap();
        set_str(&mut db, KEY, r#"{"schema_version":3}"#.into());
        assert!(matches!(read_raw(&db), Err(IndexStoreError::Checksum)));
    }
}

