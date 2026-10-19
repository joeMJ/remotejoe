//! Windows: Authenticode-Signatur einer Datei prüfen (WinVerifyTrust) und den Fingerabdruck (SHA-1) des
//! Signaturzertifikats mit dem erwarteten vergleichen.
//!
//! Das Zertifikat der remotejoe-Pakete ist selbst ausgestellt. Ist es auf dem Rechner nicht als vertrauenswürdig
//! eingetragen, meldet Windows `CERT_E_UNTRUSTEDROOT`, obwohl die Signatur unversehrt ist. Das wird angenommen,
//! weil der Fingerabdruck des Unterzeichners fest geprüft wird (und die MSI vorher schon über die signierte
//! `SHA256SUMS` geprüft ist). Jeder andere Fehler (keine Signatur, Datei verändert, falsches Zertifikat) lehnt ab.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::{CERT_E_UNTRUSTEDROOT, TRUST_E_NOSIGNATURE};
use windows_sys::Win32::Security::Cryptography::{CertGetCertificateContextProperty, CERT_SHA1_HASH_PROP_ID};
use windows_sys::Win32::Security::WinTrust::{
    WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0,
    WINTRUST_FILE_INFO, WTD_CACHE_ONLY_URL_RETRIEVAL, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
};

use crate::{Result, UpdateError};

/// Fingerabdruck (SHA-1, Großbuchstaben) des Zertifikats, mit dem die remotejoe-Pakete signiert sind.
pub const PUBLISHER_THUMBPRINT: &str = "C595450070432A501AADD72366C837AFB268740A";

/// Prüft die Signatur von `path`; der Unterzeichner muss den Fingerabdruck `thumbprint` (SHA-1, hex) haben.
pub fn verify_file(path: &Path, thumbprint: &str) -> Result<()> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: wide.as_ptr(),
        hFile: std::ptr::null_mut(),
        pgKnownSubject: std::ptr::null_mut(),
    };
    // SAFETY: Nullwerte sind für die Struktur gültig; die Zeiger leben über beide Aufrufe.
    let mut data: WINTRUST_DATA = unsafe { std::mem::zeroed() };
    data.cbStruct = std::mem::size_of::<WINTRUST_DATA>() as u32;
    data.dwUIChoice = WTD_UI_NONE;
    data.fdwRevocationChecks = WTD_REVOKE_NONE;
    data.dwUnionChoice = WTD_CHOICE_FILE;
    data.Anonymous = WINTRUST_DATA_0 { pFile: &mut file };
    data.dwStateAction = WTD_STATEACTION_VERIFY;
    data.dwProvFlags = WTD_CACHE_ONLY_URL_RETRIEVAL;
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // SAFETY: gültige Strukturen; ohne Oberfläche (WTD_UI_NONE), kein Fenster nötig.
    let status = unsafe { WinVerifyTrust(std::ptr::null_mut(), &mut action, &mut data as *mut _ as _) };
    let result = match status {
        0 | CERT_E_UNTRUSTEDROOT => signer_thumbprint(&data).and_then(|t| {
            if t.eq_ignore_ascii_case(thumbprint) {
                Ok(())
            } else {
                Err(UpdateError::Authenticode(format!("unterzeichnet mit einem anderen Zertifikat ({t})")))
            }
        }),
        TRUST_E_NOSIGNATURE => Err(UpdateError::Authenticode("die Datei ist nicht signiert".into())),
        other => Err(UpdateError::Authenticode(format!("Signatur ungültig oder Datei verändert (Code {:#010x})", other as u32))),
    };
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    // SAFETY: gibt den Zustand des ersten Aufrufs frei.
    unsafe { WinVerifyTrust(std::ptr::null_mut(), &mut action, &mut data as *mut _ as _) };
    result
}

/// Fingerabdruck (SHA-1, Großbuchstaben) des ersten Unterzeichners aus dem Prüfzustand.
fn signer_thumbprint(data: &WINTRUST_DATA) -> Result<String> {
    let missing = || UpdateError::Authenticode("kein Unterzeichner gefunden".into());
    // SAFETY: Zeiger aus dem gültigen Prüfzustand; jeder wird vor Gebrauch auf null geprüft.
    unsafe {
        let prov = WTHelperProvDataFromStateData(data.hWVTStateData);
        if prov.is_null() {
            return Err(missing());
        }
        let signer = WTHelperGetProvSignerFromChain(prov, 0, 0, 0);
        if signer.is_null() || (*signer).csCertChain == 0 || (*signer).pasCertChain.is_null() {
            return Err(missing());
        }
        let cert = (*(*signer).pasCertChain).pCert;
        if cert.is_null() {
            return Err(missing());
        }
        let mut hash = [0u8; 20];
        let mut len = hash.len() as u32;
        if CertGetCertificateContextProperty(cert, CERT_SHA1_HASH_PROP_ID, hash.as_mut_ptr().cast(), &mut len) == 0 {
            return Err(missing());
        }
        Ok(hash[..len as usize].iter().map(|b| format!("{b:02X}")).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsigned_file_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("probe.msi");
        std::fs::write(&f, b"keine Signatur").unwrap();
        let e = verify_file(&f, PUBLISHER_THUMBPRINT).unwrap_err().to_string();
        assert!(e.contains("Authenticode"), "{e}");
    }

    /// Mit signtool und dem Zertifikat des Herausgebers (nur auf dem Bau-Rechner): richtig signiert wird angenommen,
    /// ein anderer erwarteter Fingerabdruck und eine danach veränderte Datei werden abgelehnt.
    #[test]
    fn signed_file_is_accepted_only_with_the_right_thumbprint() {
        let signtool = std::fs::read_dir(r"C:\Program Files (x86)\Windows Kits\10\bin")
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path().join(r"x64\signtool.exe"))
            .filter(|p| p.is_file())
            .max();
        let Some(signtool) = signtool else {
            eprintln!("ÜBERSPRUNGEN: signtool fehlt");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("probe.exe");
        std::fs::copy(std::env::current_exe().unwrap(), &f).unwrap();
        let ok = std::process::Command::new(&signtool)
            .args(["sign", "/sha1", PUBLISHER_THUMBPRINT, "/fd", "SHA256"])
            .arg(&f)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            eprintln!("ÜBERSPRUNGEN: Zertifikat des Herausgebers (mit privatem Schlüssel) fehlt");
            return;
        }
        verify_file(&f, PUBLISHER_THUMBPRINT).unwrap();
        assert!(verify_file(&f, "0000000000000000000000000000000000000000").is_err());
        let mut bytes = std::fs::read(&f).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xff;
        std::fs::write(&f, &bytes).unwrap();
        assert!(verify_file(&f, PUBLISHER_THUMBPRINT).is_err(), "veränderte Datei");
    }
}
