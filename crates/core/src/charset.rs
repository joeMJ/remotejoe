//! Zeichensatzprüfung.
//!
//! Alle Eingaben werden als UTF-8 erwartet. Ungültige Bytefolgen führen zu
//! einem Fehler mit Fundort, nie zu stillem Ersetzen. Unsichtbare oder
//! richtungsändernde Zeichen werden erkannt und für die Anzeige sichtbar
//! markiert (`⟦U+XXXX⟧`), aber nie entfernt.

use std::borrow::Cow;
use std::fmt;

/// Fehler beim Lesen von Bytes, die kein gültiges UTF-8 sind.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("ungültiges UTF-8 in {source_name}, Zeile {line}, Spalte {column} (Byte {offset}): {bytes_hex}")]
pub struct Utf8Error {
    /// Herkunft der Daten (Dateiname oder Feldbezeichnung).
    pub source_name: String,
    /// Byte-Position des ersten ungültigen Bytes.
    pub offset: usize,
    /// Zeile (ab 1).
    pub line: usize,
    /// Spalte in Zeichen (ab 1).
    pub column: usize,
    /// Die ungültigen Bytes hexadezimal.
    pub bytes_hex: String,
}

/// Prüft `bytes` auf gültiges UTF-8 und liefert die Zeichenkette ohne Kopie.
pub fn validate_utf8<'a>(bytes: &'a [u8], source_name: &str) -> Result<&'a str, Utf8Error> {
    match std::str::from_utf8(bytes) {
        Ok(s) => Ok(s),
        Err(e) => {
            let offset = e.valid_up_to();
            // Der gültige Teil davor ist sicher dekodierbar.
            let valid = std::str::from_utf8(&bytes[..offset]).unwrap_or_default();
            let line = valid.matches('\n').count() + 1;
            let line_start = valid.rfind('\n').map(|p| p + 1).unwrap_or(0);
            let column = valid[line_start..].chars().count() + 1;
            let bad_len = e.error_len().unwrap_or(bytes.len() - offset).max(1);
            let end = (offset + bad_len).min(bytes.len());
            let bytes_hex = bytes[offset..end]
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            Err(Utf8Error {
                source_name: source_name.to_string(),
                offset,
                line,
                column,
                bytes_hex,
            })
        }
    }
}

/// Art eines auffälligen Zeichens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspiciousKind {
    /// Steuerzeichen (C0, DEL, C1).
    Control,
    /// Zeichen ohne Breite (U+200B..U+200D, U+2060, U+FEFF).
    ZeroWidth,
    /// Bidi-Steuerzeichen (U+202A..U+202E, U+2066..U+2069).
    Bidi,
}

impl fmt::Display for SuspiciousKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SuspiciousKind::Control => "Steuerzeichen",
            SuspiciousKind::ZeroWidth => "Zeichen ohne Breite",
            SuspiciousKind::Bidi => "Schreibrichtungs-Steuerzeichen",
        })
    }
}

/// Ein Fund in einer Zeichenkette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Finding {
    /// Byte-Position in der Zeichenkette.
    pub offset: usize,
    /// Das Zeichen selbst.
    pub ch: char,
    /// Art des Zeichens.
    pub kind: SuspiciousKind,
}

/// Einordnung eines Zeichens. `multiline` erlaubt Tab, LF und CR.
pub fn classify(ch: char, multiline: bool) -> Option<SuspiciousKind> {
    match ch {
        '\t' | '\n' | '\r' if multiline => None,
        '\u{0}'..='\u{1F}' | '\u{7F}'..='\u{9F}' => Some(SuspiciousKind::Control),
        '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}' => Some(SuspiciousKind::ZeroWidth),
        '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' => Some(SuspiciousKind::Bidi),
        _ => None,
    }
}

/// Listet alle auffälligen Zeichen in `s`.
pub fn find_suspicious(s: &str, multiline: bool) -> Vec<Finding> {
    s.char_indices()
        .filter_map(|(offset, ch)| classify(ch, multiline).map(|kind| Finding { offset, ch, kind }))
        .collect()
}

/// Ersatzdarstellung eines Zeichens: `⟦U+XXXX⟧`.
pub fn marker(ch: char) -> String {
    format!("⟦U+{:04X}⟧", ch as u32)
}

/// Liefert eine Anzeigefassung, in der jedes auffällige Zeichen durch seine
/// Ersatzdarstellung sichtbar gemacht ist. Der Originaltext bleibt unverändert.
pub fn mark_visible(s: &str, multiline: bool) -> Cow<'_, str> {
    if !s.chars().any(|c| classify(c, multiline).is_some()) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 16);
    for ch in s.chars() {
        if classify(ch, multiline).is_some() {
            out.push_str(&marker(ch));
        } else {
            out.push(ch);
        }
    }
    Cow::Owned(out)
}

/// Ergebnis der Prüfung eines Namens (Ordner, Ziel, Importfeld).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameCheck {
    /// Anzeigefassung mit Markierungen.
    pub display: String,
    /// Gefundene auffällige Zeichen.
    pub findings: Vec<Finding>,
}

impl NameCheck {
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
}

/// Prüft einen einzeiligen Namen.
pub fn check_name(s: &str) -> NameCheck {
    NameCheck {
        display: mark_visible(s, false).into_owned(),
        findings: find_suspicious(s, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_utf8_passes() {
        let s = "Grüße, Straße 🚀";
        assert_eq!(validate_utf8(s.as_bytes(), "t").unwrap(), s);
    }

    #[test]
    fn invalid_utf8_reports_location() {
        let mut b = b"erste\nzw".to_vec();
        b.push(0xC3); // abgeschnittene Folge
        b.push(b'x');
        let e = validate_utf8(&b, "datei.json").unwrap_err();
        assert_eq!(e.source_name, "datei.json");
        assert_eq!(e.offset, 8);
        assert_eq!(e.line, 2);
        assert_eq!(e.column, 3);
        assert_eq!(e.bytes_hex, "C3");
        assert!(e.to_string().contains("Zeile 2"));
    }

    #[test]
    fn invalid_utf8_at_end() {
        let e = validate_utf8(&[b'a', 0xF0, 0x9F], "x").unwrap_err();
        assert_eq!(e.offset, 1);
        assert_eq!(e.bytes_hex, "F0 9F");
    }

    #[test]
    fn marks_zero_width_and_bidi() {
        let s = "ad\u{200B}min\u{202E}gnp.exe\u{FEFF}";
        let c = check_name(s);
        assert_eq!(c.display, "ad⟦U+200B⟧min⟦U+202E⟧gnp.exe⟦U+FEFF⟧");
        assert_eq!(c.findings.len(), 3);
        assert_eq!(c.findings[0].kind, SuspiciousKind::ZeroWidth);
        assert_eq!(c.findings[1].kind, SuspiciousKind::Bidi);
        // Das Original bleibt erhalten.
        assert_eq!(s.chars().count(), 15);
    }

    #[test]
    fn all_listed_codepoints_are_detected() {
        for cp in [
            0x200B, 0x200C, 0x200D, 0x2060, 0xFEFF, 0x202A, 0x202B, 0x202C, 0x202D, 0x202E, 0x2066,
            0x2067, 0x2068, 0x2069,
        ] {
            let ch = char::from_u32(cp).unwrap();
            assert!(classify(ch, true).is_some(), "U+{cp:04X}");
        }
    }

    #[test]
    fn controls_respect_multiline() {
        assert_eq!(mark_visible("a\tb", false), "a⟦U+0009⟧b");
        assert_eq!(mark_visible("a\tb\r\n", true), "a\tb\r\n");
        assert_eq!(mark_visible("a\u{7}b", true), "a⟦U+0007⟧b");
        assert_eq!(mark_visible("x\u{85}", true), "x⟦U+0085⟧");
    }

    #[test]
    fn clean_text_is_borrowed() {
        assert!(matches!(mark_visible("Grüße", false), Cow::Borrowed(_)));
    }
}
