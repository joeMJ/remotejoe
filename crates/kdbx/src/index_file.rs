//! Rückfallebene: Index als eigene verschlüsselte Datei neben der KDBX
//! (`<name>.rjindex`).
//!
//! Aufbau: `RJINDEX1` (8 Bytes) ‖ Salz (32) ‖ Nonce (24) ‖ Chiffrat.
//! Verschlüsselung XChaCha20-Poly1305; Schlüssel per HKDF-SHA256 aus dem
//! KDBX-Hauptschlüssel (Kennwort und Schlüsseldatei) mit zufälligem Salz je
//! Schreibvorgang. Geschrieben wird atomar mit Sicherungskopie.

use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use rj_core::schema::{from_json, LoadedIndex, SchemaError};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use crate::MasterKey;

const MAGIC: &[u8; 8] = b"RJINDEX1";
const INFO: &[u8] = b"remotejoe index-file v1";

#[derive(Debug, thiserror::Error)]
pub enum IndexFileError {
    #[error("Indexdatei: {0}")]
    Io(#[from] std::io::Error),
    #[error("Indexdatei hat kein gültiges Format")]
    Format,
    #[error("Indexdatei lässt sich mit diesem Schlüssel nicht entschlüsseln oder ist beschädigt")]
    Decrypt,
    #[error("Zufallszahlen nicht verfügbar")]
    Random,
    #[error(transparent)]
    Schema(#[from] SchemaError),
}

/// Pfad der Indexdatei zu einer KDBX.
pub fn path_for(kdbx: &Path) -> PathBuf {
    kdbx.with_extension("rjindex")
}

fn derive(key: &MasterKey, salt: &[u8]) -> [u8; 32] {
    let ikm = key.derivation_material();
    let hk = Hkdf::<Sha256>::new(Some(salt), ikm.as_bytes());
    let mut out = [0u8; 32];
    hk.expand(INFO, &mut out).expect("32 Bytes sind zulässig");
    out
}

/// Verschlüsselt JSON-Bytes zum Dateiinhalt.
pub fn encrypt(key: &MasterKey, json: &[u8]) -> Result<Vec<u8>, IndexFileError> {
    let mut salt = [0u8; 32];
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut salt).map_err(|_| IndexFileError::Random)?;
    getrandom::fill(&mut nonce).map_err(|_| IndexFileError::Random)?;
    let mut k = derive(key, &salt);
    let cipher = XChaCha20Poly1305::new((&k).into());
    k.zeroize();
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: json, aad: MAGIC })
        .map_err(|_| IndexFileError::Decrypt)?;
    let mut out = Vec::with_capacity(8 + 32 + 24 + ct.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Entschlüsselt einen Dateiinhalt zu JSON-Bytes.
pub fn decrypt(key: &MasterKey, data: &[u8]) -> Result<Vec<u8>, IndexFileError> {
    if data.len() < 8 + 32 + 24 + 16 || &data[..8] != MAGIC {
        return Err(IndexFileError::Format);
    }
    let (salt, rest) = data[8..].split_at(32);
    let (nonce, ct) = rest.split_at(24);
    let mut k = derive(key, salt);
    let cipher = XChaCha20Poly1305::new((&k).into());
    k.zeroize();
    cipher
        .decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad: MAGIC })
        .map_err(|_| IndexFileError::Decrypt)
}

/// Liest die Indexdatei; `None`, wenn sie fehlt.
pub fn read(path: &Path, key: &MasterKey) -> Result<Option<(LoadedIndex, [u8; 32])>, IndexFileError> {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let json = decrypt(key, &data)?;
    let sha: [u8; 32] = Sha256::digest(&json).into();
    Ok(Some((from_json(&json, &path.display().to_string())?, sha)))
}

/// Schreibt die Indexdatei atomar mit Sicherungskopie.
pub fn write(path: &Path, key: &MasterKey, json: &[u8]) -> Result<(), IndexFileError> {
    let data = encrypt(key, json)?;
    rj_core::fsutil::atomic_write_with_backup(path, &data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_wrong_key() {
        let k = MasterKey::password("Grüße 🔑").with_key_file_bytes(b"schluessel".to_vec());
        let ct = encrypt(&k, b"{\"a\":1}").unwrap();
        assert_eq!(decrypt(&k, &ct).unwrap(), b"{\"a\":1}");
        // Salz ist zufällig: zweimal verschlüsseln ergibt Unterschiedliches.
        assert_ne!(encrypt(&k, b"x").unwrap(), encrypt(&k, b"x").unwrap());
        let other = MasterKey::password("Grüße 🔑");
        assert!(matches!(decrypt(&other, &ct), Err(IndexFileError::Decrypt)));
        let mut bad = ct.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(matches!(decrypt(&k, &bad), Err(IndexFileError::Decrypt)));
    }

    #[test]
    fn file_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = path_for(&dir.path().join("t.kdbx"));
        let k = MasterKey::password("pw");
        assert!(read(&p, &k).unwrap().is_none());
        let idx = rj_core::Index::default();
        write(&p, &k, &rj_core::schema::to_json(&idx).unwrap()).unwrap();
        assert_eq!(read(&p, &k).unwrap().unwrap().0.index, idx);
        let raw = std::fs::read(&p).unwrap();
        assert!(!raw.windows(6).any(|w| w == b"Wurzel"));
    }
}
