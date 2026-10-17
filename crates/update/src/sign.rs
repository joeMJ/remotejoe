//! Schlüssel, Signatur und Prüfsummendatei.
//!
//! - Öffentlicher Schlüssel: Textdatei, eine Zeile
//!   `remotejoe-update-ed25519 <Base64 der 32 Bytes>`; Zeilen mit `#` sind Kommentare.
//! - Privater Schlüssel: `remotejoe-update-ed25519-privat <Base64 der 32 Bytes>`;
//!   bleibt beim Herausgeber und gehört nie in ein Repository.
//! - `SHA256SUMS`: Zeilen `<sha256 hex>  <Dateiname>` (Format von `sha256sum`).
//! - `SHA256SUMS.sig`: Ed25519-Signatur über `SHA256SUMS`, Base64 (oder 64 Rohbytes).

use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::{Result, UpdateError};

pub const PUBLIC_TAG: &str = "remotejoe-update-ed25519";
pub const SECRET_TAG: &str = "remotejoe-update-ed25519-privat";
pub const SUMS: &str = "SHA256SUMS";
pub const SIG: &str = "SHA256SUMS.sig";

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn tagged(text: &str, tag: &str) -> Result<Zeroizing<[u8; 32]>> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .ok_or_else(|| UpdateError::BadKey("leer".into()))?;
    let (t, v) = line.split_once(' ').ok_or_else(|| UpdateError::BadKey("Format".into()))?;
    if t != tag {
        return Err(UpdateError::BadKey(format!("Kennung „{t}“ statt „{tag}“")));
    }
    let raw = Zeroizing::new(b64().decode(v.trim()).map_err(|_| UpdateError::BadKey("Base64".into()))?);
    let mut out = Zeroizing::new([0u8; 32]);
    if raw.len() != 32 {
        return Err(UpdateError::BadKey("Länge".into()));
    }
    out.copy_from_slice(&raw);
    Ok(out)
}

pub fn public_key_from_text(text: &str) -> Result<VerifyingKey> {
    let k = tagged(text, PUBLIC_TAG)?;
    VerifyingKey::from_bytes(&k).map_err(|_| UpdateError::BadKey("kein gültiger Punkt".into()))
}

pub fn public_key_text(k: &VerifyingKey) -> String {
    format!("{PUBLIC_TAG} {}\n", b64().encode(k.as_bytes()))
}

pub fn secret_key_from_text(text: &str) -> Result<SigningKey> {
    Ok(SigningKey::from_bytes(&*tagged(text, SECRET_TAG)?))
}

pub fn secret_key_text(k: &SigningKey) -> Zeroizing<String> {
    Zeroizing::new(format!("{SECRET_TAG} {}\n", b64().encode(Zeroizing::new(k.to_bytes()).as_slice())))
}

/// Neues Schlüsselpaar aus dem Zufallsgenerator des Systems.
pub fn generate() -> Result<SigningKey> {
    let mut seed = Zeroizing::new([0u8; 32]);
    getrandom::fill(seed.as_mut_slice()).map_err(|e| UpdateError::BadKey(e.to_string()))?;
    Ok(SigningKey::from_bytes(&seed))
}

pub fn sign(k: &SigningKey, sums: &[u8]) -> String {
    format!("{}\n", b64().encode(k.sign(sums).to_bytes()))
}

/// Prüft die Signatur der Prüfsummendatei (streng, ohne verformbare Signaturen).
pub fn verify(k: &VerifyingKey, sums: &[u8], sig: &[u8]) -> Result<()> {
    let bytes: Vec<u8> = if sig.len() == 64 {
        sig.to_vec()
    } else {
        let t = std::str::from_utf8(sig).map_err(|_| UpdateError::BadSignature)?;
        b64().decode(t.trim()).map_err(|_| UpdateError::BadSignature)?
    };
    let arr: [u8; 64] = bytes.as_slice().try_into().map_err(|_| UpdateError::BadSignature)?;
    k.verify_strict(sums, &Signature::from_bytes(&arr)).map_err(|_| UpdateError::BadSignature)
}

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sums_text(entries: &[(String, String)]) -> String {
    entries.iter().map(|(name, hex)| format!("{hex}  {name}\n")).collect()
}

/// Erwartete Prüfsumme (hex, klein) einer Datei aus `SHA256SUMS`.
pub fn sums_lookup(sums: &[u8], name: &str) -> Result<String> {
    let text = std::str::from_utf8(sums).map_err(|_| UpdateError::Parse("SHA256SUMS ist kein UTF-8".into()))?;
    for line in text.lines() {
        let Some((hex, file)) = line.split_once(char::is_whitespace) else { continue };
        let file = file.trim_start();
        let file = file.strip_prefix('*').unwrap_or(file);
        if file == name && hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(hex.to_ascii_lowercase());
        }
    }
    Err(UpdateError::NotListed(name.into()))
}

/// Signatur prüfen, dann die Prüfsumme des Archivs.
pub fn verify_package(k: &VerifyingKey, sums: &[u8], sig: &[u8], archive_name: &str, archive: &[u8]) -> Result<()> {
    verify(k, sums, sig)?;
    let want = sums_lookup(sums, archive_name)?;
    if sha256_hex(archive) != want {
        return Err(UpdateError::BadChecksum);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_roundtrip_and_signature_checks() {
        let sk = generate().unwrap();
        let vk = public_key_from_text(&format!("# Kommentar\n{}", public_key_text(&sk.verifying_key()))).unwrap();
        assert_eq!(vk, sk.verifying_key());
        assert_eq!(secret_key_from_text(&secret_key_text(&sk)).unwrap().to_bytes(), sk.to_bytes());
        assert!(public_key_from_text(&secret_key_text(&sk)).is_err(), "privater Schlüssel ist kein öffentlicher");

        let archive = b"Inhalt";
        let sums = sums_text(&[("remotejoe-1.0.0-linux-x86_64.tar.zst".into(), sha256_hex(archive))]);
        let sig = sign(&sk, sums.as_bytes());
        verify_package(&vk, sums.as_bytes(), sig.as_bytes(), "remotejoe-1.0.0-linux-x86_64.tar.zst", archive).unwrap();
        assert!(matches!(verify_package(&vk, sums.as_bytes(), sig.as_bytes(), "remotejoe-1.0.0-linux-x86_64.tar.zst", b"Inhalx"), Err(UpdateError::BadChecksum)));
        let bad_sums = sums.replace('a', "b");
        assert!(matches!(verify_package(&vk, bad_sums.as_bytes(), sig.as_bytes(), "x", archive), Err(UpdateError::BadSignature)));
        let other = generate().unwrap();
        assert!(matches!(verify(&other.verifying_key(), sums.as_bytes(), sig.as_bytes()), Err(UpdateError::BadSignature)));
        assert!(matches!(verify(&vk, sums.as_bytes(), b""), Err(UpdateError::BadSignature)));
        assert!(matches!(sums_lookup(sums.as_bytes(), "anderes"), Err(UpdateError::NotListed(_))));
        assert_eq!(sums_lookup(b"0000000000000000000000000000000000000000000000000000000000000000 *x\n", "x").unwrap().len(), 64);
    }
}
