//! Rückfallebene: Index als eigene verschlüsselte Datei neben der KDBX
//! (`<name>.rjindex`).
//!
//! Aufbau Version 2 (wird geschrieben):
//! `RJINDEX2` (8 Bytes) ‖ Verfahren (1, `1` = Argon2id) ‖ Argon2-Version (1, `0x13`) ‖
//! Speicher in KiB (u32 LE) ‖ Durchläufe (u32 LE) ‖ Fäden (u32 LE) ‖ Salz (32) ‖
//! Nonce (24) ‖ Chiffrat.
//! Der Schlüssel wird mit Argon2id aus dem KDBX-Hauptschlüssel (Kennwort und
//! Schlüsseldatei) abgeleitet; das Salz gilt je Datei, die Nonce ist bei jedem
//! Schreiben neu. Der ganze Kopf ist als zusätzliche Daten in die
//! Authentisierung einbezogen. Verschlüsselung XChaCha20-Poly1305.
//!
//! Version 1 (`RJINDEX1` ‖ Salz ‖ Nonce ‖ Chiffrat, Schlüssel per HKDF-SHA256)
//! wird weiterhin gelesen und beim nächsten Speichern als Version 2 geschrieben.
//! Geschrieben wird atomar mit Sicherungskopie.

use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use rj_core::schema::{from_json, LoadedIndex, SchemaError};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::MasterKey;

const MAGIC_V1: &[u8; 8] = b"RJINDEX1";
const MAGIC_V2: &[u8; 8] = b"RJINDEX2";
const INFO_V1: &[u8] = b"remotejoe index-file v1";
const KDF_ARGON2ID: u8 = 1;
const ARGON2_VERSION_13: u8 = 0x13;
const HEADER_V2: usize = 8 + 1 + 1 + 12 + 32;
const NONCE: usize = 24;
const TAG: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum IndexFileError {
    #[error("Indexdatei: {0}")]
    Io(#[from] std::io::Error),
    #[error("Indexdatei hat kein gültiges Format")]
    Format,
    #[error("Indexdatei: unzulässige Parameter der Schlüsselableitung")]
    Params,
    #[error("Indexdatei lässt sich mit diesem Schlüssel nicht entschlüsseln oder ist beschädigt")]
    Decrypt,
    #[error("Zufallszahlen nicht verfügbar")]
    Random,
    #[error(transparent)]
    Schema(#[from] SchemaError),
}

/// Parameter der Argon2id-Ableitung.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Argon2Params {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl Argon2Params {
    /// Wie bei neuen KDBX-Dateien: 64 MiB, 10 Durchläufe, 2 Fäden.
    pub const DEFAULT: Argon2Params = Argon2Params { memory_kib: 64 * 1024, iterations: 10, parallelism: 2 };

    /// Grenzen beim Lesen, damit eine manipulierte Datei keinen unbegrenzten
    /// Speicher oder Rechenaufwand verlangt.
    fn check(self) -> Result<Self, IndexFileError> {
        let ok = (1..=64).contains(&self.parallelism)
            && (1..=1000).contains(&self.iterations)
            && self.memory_kib >= 8 * self.parallelism
            && self.memory_kib <= 4 * 1024 * 1024;
        if ok {
            Ok(self)
        } else {
            Err(IndexFileError::Params)
        }
    }
}

/// Abgeleiteter Dateischlüssel samt Salz und Parametern. Wird beim Verwerfen
/// überschrieben.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct FileKey {
    salt: [u8; 32],
    #[zeroize(skip)]
    params: Argon2Params,
    key: [u8; 32],
}

impl FileKey {
    pub fn params(&self) -> Argon2Params {
        self.params
    }
}

impl std::fmt::Debug for FileKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileKey").field("params", &self.params).finish_non_exhaustive()
    }
}

/// Pfad der Indexdatei zu einer KDBX.
pub fn path_for(kdbx: &Path) -> PathBuf {
    kdbx.with_extension("rjindex")
}

fn derive_v1(key: &MasterKey, salt: &[u8]) -> [u8; 32] {
    let ikm = key.derivation_material();
    let hk = Hkdf::<Sha256>::new(Some(salt), ikm.as_bytes());
    let mut out = [0u8; 32];
    hk.expand(INFO_V1, &mut out).expect("32 Bytes sind zulässig");
    out
}

/// Leitet den Dateischlüssel mit Argon2id ab.
pub fn derive_key(key: &MasterKey, params: Argon2Params, salt: [u8; 32]) -> Result<FileKey, IndexFileError> {
    let params = params.check()?;
    let ikm = key.derivation_material();
    let config = argon2::Config {
        variant: argon2::Variant::Argon2id,
        version: argon2::Version::Version13,
        mem_cost: params.memory_kib,
        time_cost: params.iterations,
        lanes: params.parallelism,
        secret: &[],
        ad: &[],
        hash_length: 32,
        ..argon2::Config::default()
    };
    let mut raw = argon2::hash_raw(ikm.as_bytes(), &salt, &config).map_err(|_| IndexFileError::Params)?;
    let mut k = [0u8; 32];
    k.copy_from_slice(&raw);
    raw.zeroize();
    Ok(FileKey { salt, params, key: k })
}

/// Leitet einen Dateischlüssel mit neuem, zufälligem Salz ab.
pub fn new_key(key: &MasterKey, params: Argon2Params) -> Result<FileKey, IndexFileError> {
    let mut salt = [0u8; 32];
    getrandom::fill(&mut salt).map_err(|_| IndexFileError::Random)?;
    derive_key(key, params, salt)
}

fn header_v2(fk: &FileKey) -> Vec<u8> {
    let mut h = Vec::with_capacity(HEADER_V2);
    h.extend_from_slice(MAGIC_V2);
    h.push(KDF_ARGON2ID);
    h.push(ARGON2_VERSION_13);
    h.extend_from_slice(&fk.params.memory_kib.to_le_bytes());
    h.extend_from_slice(&fk.params.iterations.to_le_bytes());
    h.extend_from_slice(&fk.params.parallelism.to_le_bytes());
    h.extend_from_slice(&fk.salt);
    h
}

/// Verschlüsselt JSON-Bytes mit einem abgeleiteten Schlüssel (Version 2).
pub fn encrypt_with(fk: &FileKey, json: &[u8]) -> Result<Vec<u8>, IndexFileError> {
    let mut nonce = [0u8; NONCE];
    getrandom::fill(&mut nonce).map_err(|_| IndexFileError::Random)?;
    let header = header_v2(fk);
    let cipher = XChaCha20Poly1305::new((&fk.key).into());
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: json, aad: &header })
        .map_err(|_| IndexFileError::Decrypt)?;
    let mut out = header;
    out.reserve(NONCE + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Verschlüsselt JSON-Bytes mit neuem Salz (Version 2).
pub fn encrypt(key: &MasterKey, params: Argon2Params, json: &[u8]) -> Result<Vec<u8>, IndexFileError> {
    encrypt_with(&new_key(key, params)?, json)
}

/// Verschlüsselt im alten Format (Version 1); nur für Verträglichkeitstests.
#[doc(hidden)]
pub fn encrypt_v1(key: &MasterKey, json: &[u8]) -> Result<Vec<u8>, IndexFileError> {
    let mut salt = [0u8; 32];
    let mut nonce = [0u8; NONCE];
    getrandom::fill(&mut salt).map_err(|_| IndexFileError::Random)?;
    getrandom::fill(&mut nonce).map_err(|_| IndexFileError::Random)?;
    let mut k = derive_v1(key, &salt);
    let cipher = XChaCha20Poly1305::new((&k).into());
    k.zeroize();
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: json, aad: MAGIC_V1 })
        .map_err(|_| IndexFileError::Decrypt)?;
    let mut out = Vec::with_capacity(8 + 32 + NONCE + ct.len());
    out.extend_from_slice(MAGIC_V1);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Format-Version eines Dateiinhalts (1 oder 2), sonst `None`.
pub fn format_version(data: &[u8]) -> Option<u8> {
    match data.get(..8) {
        Some(m) if m == MAGIC_V1 => Some(1),
        Some(m) if m == MAGIC_V2 => Some(2),
        _ => None,
    }
}

/// Entschlüsselt einen Dateiinhalt zu JSON-Bytes. Bei Version 2 wird der
/// abgeleitete Schlüssel zurückgegeben, damit späteres Schreiben ihn
/// wiederverwenden kann, ohne erneut abzuleiten.
pub fn decrypt(key: &MasterKey, data: &[u8]) -> Result<(Vec<u8>, Option<FileKey>), IndexFileError> {
    match format_version(data) {
        Some(1) => {
            if data.len() < 8 + 32 + NONCE + TAG {
                return Err(IndexFileError::Format);
            }
            let (salt, rest) = data[8..].split_at(32);
            let (nonce, ct) = rest.split_at(NONCE);
            let mut k = derive_v1(key, salt);
            let cipher = XChaCha20Poly1305::new((&k).into());
            k.zeroize();
            let json = cipher
                .decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad: MAGIC_V1 })
                .map_err(|_| IndexFileError::Decrypt)?;
            Ok((json, None))
        }
        Some(2) => {
            if data.len() < HEADER_V2 + NONCE + TAG {
                return Err(IndexFileError::Format);
            }
            if data[8] != KDF_ARGON2ID || data[9] != ARGON2_VERSION_13 {
                return Err(IndexFileError::Params);
            }
            let u32_at = |i: usize| u32::from_le_bytes(data[i..i + 4].try_into().expect("4 Bytes"));
            let params = Argon2Params { memory_kib: u32_at(10), iterations: u32_at(14), parallelism: u32_at(18) };
            let mut salt = [0u8; 32];
            salt.copy_from_slice(&data[22..54]);
            let fk = derive_key(key, params, salt)?;
            let (header, rest) = data.split_at(HEADER_V2);
            let (nonce, ct) = rest.split_at(NONCE);
            let cipher = XChaCha20Poly1305::new((&fk.key).into());
            let json = cipher
                .decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad: header })
                .map_err(|_| IndexFileError::Decrypt)?;
            Ok((json, Some(fk)))
        }
        _ => Err(IndexFileError::Format),
    }
}

/// Gelesene Indexdatei.
pub struct ReadIndex {
    pub loaded: LoadedIndex,
    pub sha: [u8; 32],
    /// Abgeleiteter Schlüssel (nur Version 2).
    pub key: Option<FileKey>,
}

/// Liest die Indexdatei; `None`, wenn sie fehlt.
pub fn read(path: &Path, key: &MasterKey) -> Result<Option<ReadIndex>, IndexFileError> {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let (json, fk) = decrypt(key, &data)?;
    let sha: [u8; 32] = Sha256::digest(&json).into();
    Ok(Some(ReadIndex { loaded: from_json(&json, &path.display().to_string())?, sha, key: fk }))
}

/// Schreibt die Indexdatei atomar mit Sicherungskopie (Version 2). Ein
/// vorhandener Schlüssel mit denselben Parametern wird wiederverwendet (gleiches
/// Salz, neue Nonce); sonst wird mit neuem Salz abgeleitet und `cache` gesetzt.
pub fn write(
    path: &Path,
    key: &MasterKey,
    params: Argon2Params,
    cache: &mut Option<FileKey>,
    json: &[u8],
) -> Result<(), IndexFileError> {
    if cache.as_ref().map(|c| c.params) != Some(params) {
        *cache = Some(new_key(key, params)?);
    }
    let data = encrypt_with(cache.as_ref().expect("eben gesetzt"), json)?;
    rj_core::fsutil::atomic_write_with_backup(path, &data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAST: Argon2Params = Argon2Params { memory_kib: 64, iterations: 1, parallelism: 1 };

    #[test]
    fn roundtrip_and_wrong_key() {
        let k = MasterKey::password("Grüße 🔑").with_key_file_bytes(b"schluessel".to_vec());
        let ct = encrypt(&k, FAST, b"{\"a\":1}").unwrap();
        assert_eq!(format_version(&ct), Some(2));
        assert_eq!(decrypt(&k, &ct).unwrap().0, b"{\"a\":1}");
        // Salz ist zufällig: zweimal verschlüsseln ergibt Unterschiedliches.
        assert_ne!(encrypt(&k, FAST, b"x").unwrap(), encrypt(&k, FAST, b"x").unwrap());
        let other = MasterKey::password("Grüße 🔑");
        assert!(matches!(decrypt(&other, &ct), Err(IndexFileError::Decrypt)));
        let mut bad = ct.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(matches!(decrypt(&k, &bad), Err(IndexFileError::Decrypt)));
        // Kopf ist authentisiert: geänderte Parameter fallen auf.
        let mut bad = ct.clone();
        bad[14] = 2;
        assert!(matches!(decrypt(&k, &bad), Err(IndexFileError::Decrypt)));
    }

    #[test]
    fn default_params_are_argon2id_like_new_kdbx() {
        let k = MasterKey::password("pw");
        let ct = encrypt(&k, Argon2Params::DEFAULT, b"{}").unwrap();
        assert_eq!(&ct[..8], b"RJINDEX2");
        assert_eq!(ct[8], KDF_ARGON2ID);
        assert_eq!(u32::from_le_bytes(ct[10..14].try_into().unwrap()), 64 * 1024);
        assert_eq!(u32::from_le_bytes(ct[14..18].try_into().unwrap()), 10);
        assert_eq!(u32::from_le_bytes(ct[18..22].try_into().unwrap()), 2);
        let (json, fk) = decrypt(&k, &ct).unwrap();
        assert_eq!(json, b"{}");
        assert_eq!(fk.unwrap().params(), Argon2Params::DEFAULT);
    }

    #[test]
    fn old_v1_file_is_still_readable() {
        let k = MasterKey::password("alt");
        let ct = encrypt_v1(&k, b"{\"v\":1}").unwrap();
        assert_eq!(format_version(&ct), Some(1));
        let (json, fk) = decrypt(&k, &ct).unwrap();
        assert_eq!(json, b"{\"v\":1}");
        assert!(fk.is_none());
        assert!(matches!(decrypt(&MasterKey::password("falsch"), &ct), Err(IndexFileError::Decrypt)));
    }

    #[test]
    fn rejects_excessive_params() {
        let k = MasterKey::password("pw");
        let mut ct = encrypt(&k, FAST, b"{}").unwrap();
        ct[10..14].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(decrypt(&k, &ct), Err(IndexFileError::Params)));
        let mut ct = encrypt(&k, FAST, b"{}").unwrap();
        ct[8] = 9;
        assert!(matches!(decrypt(&k, &ct), Err(IndexFileError::Params)));
        assert!(matches!(decrypt(&k, b"RJINDEX2kurz"), Err(IndexFileError::Format)));
    }

    #[test]
    fn file_roundtrip_reuses_salt() {
        let dir = tempfile::tempdir().unwrap();
        let p = path_for(&dir.path().join("t.kdbx"));
        let k = MasterKey::password("pw");
        assert!(read(&p, &k).unwrap().is_none());
        let idx = rj_core::Index::default();
        let mut cache = None;
        write(&p, &k, FAST, &mut cache, &rj_core::schema::to_json(&idx).unwrap()).unwrap();
        let first = std::fs::read(&p).unwrap();
        let r = read(&p, &k).unwrap().unwrap();
        assert_eq!(r.loaded.index, idx);
        write(&p, &k, FAST, &mut cache, &rj_core::schema::to_json(&idx).unwrap()).unwrap();
        let second = std::fs::read(&p).unwrap();
        assert_eq!(first[22..54], second[22..54], "Salz je Datei");
        assert_ne!(first[54..78], second[54..78], "Nonce je Schreiben");
        assert!(!second.windows(6).any(|w| w == b"Wurzel"));
    }
}
