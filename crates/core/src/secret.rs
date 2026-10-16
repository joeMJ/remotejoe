//! Behälter für Kennwörter und andere Geheimnisse.
//!
//! Ein [`Secret`] hält die Bytes genau so, wie sie übergeben wurden: keine
//! Unicode-Normalisierung, kein Kürzen von Leerraum, keine Umkodierung.
//! Beim Freigeben wird der Speicher überschrieben.

use std::fmt;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Clone, Default, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct Secret(Vec<u8>);

impl Secret {
    /// Übernimmt die Bytes unverändert.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Secret(bytes)
    }

    /// Übernimmt eine Zeichenkette unverändert (UTF-8-Bytes).
    pub fn from_str_exact(s: &str) -> Self {
        Secret(s.as_bytes().to_vec())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Liefert den Inhalt als Zeichenkette, wenn er gültiges UTF-8 ist.
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// UTF-16-Codeeinheiten für Unicode-Tastenereignisse (z. B. RDP).
    /// Liefert `None`, wenn der Inhalt kein gültiges UTF-8 ist.
    pub fn utf16_units(&self) -> Option<Vec<u16>> {
        self.as_str().map(|s| s.encode_utf16().collect())
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret(<{} Bytes verborgen>)", self.0.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nfc_and_nfd_stay_distinct_and_exact() {
        let nfc = "Kennw\u{00F6}rt \u{00E9}"; // vorkomponiert
        let nfd = "Kennwo\u{0308}rt e\u{0301}"; // zerlegt
        let a = Secret::from_str_exact(nfc);
        let b = Secret::from_str_exact(nfd);
        assert_eq!(a.as_bytes(), nfc.as_bytes());
        assert_eq!(b.as_bytes(), nfd.as_bytes());
        assert_ne!(a, b);
        // Rundlauf über UTF-16 und zurück ändert nichts.
        let back = String::from_utf16(&b.utf16_units().unwrap()).unwrap();
        assert_eq!(back.as_bytes(), nfd.as_bytes());
    }

    #[test]
    fn whitespace_is_kept() {
        let s = Secret::from_str_exact("  geheim\t \n");
        assert_eq!(s.as_bytes(), b"  geheim\t \n");
    }

    #[test]
    fn arbitrary_bytes_kept() {
        let s = Secret::from_bytes(vec![0xFF, 0x00, 0xC3]);
        assert_eq!(s.as_bytes(), &[0xFF, 0x00, 0xC3]);
        assert!(s.as_str().is_none());
    }

    #[test]
    fn debug_hides_content() {
        let s = Secret::from_str_exact("streng-geheim");
        assert!(!format!("{s:?}").contains("geheim"));
    }
}
