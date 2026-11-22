//! Zwischenablage über `cliprdr` (MS-RDPECLIP): Kodierung von Text und
//! Dateilisten, Zeilenenden, Richtung und Größenlimit.
//!
//! Text wird als `CF_UNICODETEXT` (UTF-16LE mit abschließender Null)
//! übertragen. Beim Senden werden Zeilenenden ausdrücklich zu CRLF, beim
//! Empfangen CRLF zu LF umgesetzt. Ungültiges UTF-16 führt zu einer Meldung,
//! nie zu stillem Ersetzen.

use rj_core::ClipboardDirection;
use sha2::{Digest, Sha256};

pub const CF_UNICODETEXT: u32 = 13;
/// Formatkennung, unter der eigene Dateilisten angeboten werden.
pub const LOCAL_FILEGROUP_ID: u32 = 0xC0A1;
pub const FILEGROUP_NAME: &str = "FileGroupDescriptorW";
pub const FILECONTENTS_SIZE: u32 = 1;
pub const FILECONTENTS_RANGE: u32 = 2;

const FD_ATTRIBUTES: u32 = 0x0000_0004;
const FD_FILESIZE: u32 = 0x0000_0040;
const FD_UNICODE: u32 = 0x8000_0000;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const DESCRIPTOR_SIZE: usize = 592;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClipError {
    #[error("Text der Gegenstelle ist kein gültiges UTF-16 (Codeeinheit {index}: {unit:#06X})")]
    InvalidUtf16 { index: usize, unit: u16 },
    #[error("Text der Gegenstelle hat ungerade Länge ({0} Bytes)")]
    OddLength(usize),
    #[error("Dateiliste der Gegenstelle ist beschädigt")]
    BadFileGroup,
    #[error("Inhalt überschreitet das Größenlimit ({size} > {limit} Bytes)")]
    TooLarge { size: u64, limit: u64 },
    #[error("Dateiname ungültig: {0}")]
    BadFileName(String),
}

/// Text zum Senden kodieren: Zeilenenden → CRLF, UTF-16LE, Null am Ende.
pub fn encode_text(text: &str) -> Vec<u8> {
    let lf = text.replace("\r\n", "\n");
    let crlf = lf.replace('\n', "\r\n");
    let mut out = Vec::with_capacity(crlf.len() * 2 + 2);
    for u in crlf.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out.extend_from_slice(&[0, 0]);
    out
}

/// Empfangenen Text dekodieren: bis zur ersten Null, CRLF → LF.
pub fn decode_text(bytes: &[u8]) -> Result<String, ClipError> {
    let mut units: Vec<u16> = Vec::with_capacity(bytes.len() / 2);
    let mut chunks = bytes.chunks_exact(2);
    let mut terminated = false;
    for c in &mut chunks {
        let u = u16::from_le_bytes([c[0], c[1]]);
        if u == 0 {
            terminated = true;
            break;
        }
        units.push(u);
    }
    if !terminated && !chunks.remainder().is_empty() {
        return Err(ClipError::OddLength(bytes.len()));
    }
    let mut s = String::with_capacity(units.len());
    for (i, r) in char::decode_utf16(units.iter().copied()).enumerate() {
        match r {
            Ok(c) => s.push(c),
            Err(e) => {
                // Fundort als Index der Codeeinheit.
                let index = units.iter().position(|&u| u == e.unpaired_surrogate()).unwrap_or(i);
                return Err(ClipError::InvalidUtf16 { index, unit: e.unpaired_surrogate() });
            }
        }
    }
    Ok(s.replace("\r\n", "\n"))
}

/// Eintrag einer Dateiliste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDesc {
    /// Relativer Name; Unterordner mit `\`.
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
}

/// Dateiliste (`FileGroupDescriptorW`) kodieren.
pub fn encode_file_group(files: &[FileDesc]) -> Result<Vec<u8>, ClipError> {
    let mut out = Vec::with_capacity(4 + files.len() * DESCRIPTOR_SIZE);
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for f in files {
        let name: Vec<u16> = f.name.encode_utf16().collect();
        if name.is_empty() || name.len() >= 260 || f.name.contains('/') || f.name.split('\\').any(|p| p == ".." || p.is_empty()) {
            return Err(ClipError::BadFileName(f.name.clone()));
        }
        let mut d = vec![0u8; DESCRIPTOR_SIZE];
        d[0..4].copy_from_slice(&(FD_ATTRIBUTES | FD_FILESIZE | FD_UNICODE).to_le_bytes());
        let attrs = if f.is_dir { FILE_ATTRIBUTE_DIRECTORY } else { FILE_ATTRIBUTE_NORMAL };
        d[36..40].copy_from_slice(&attrs.to_le_bytes());
        d[64..68].copy_from_slice(&((f.size >> 32) as u32).to_le_bytes());
        d[68..72].copy_from_slice(&(f.size as u32).to_le_bytes());
        for (i, u) in name.iter().enumerate() {
            d[72 + i * 2..74 + i * 2].copy_from_slice(&u.to_le_bytes());
        }
        out.extend_from_slice(&d);
    }
    Ok(out)
}

/// Dateiliste der Gegenstelle lesen. Namen mit `..` oder absolute Pfade
/// werden abgelehnt.
pub fn decode_file_group(bytes: &[u8]) -> Result<Vec<FileDesc>, ClipError> {
    if bytes.len() < 4 {
        return Err(ClipError::BadFileGroup);
    }
    let n = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    if bytes.len() < 4 + n * DESCRIPTOR_SIZE {
        return Err(ClipError::BadFileGroup);
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let d = &bytes[4 + i * DESCRIPTOR_SIZE..4 + (i + 1) * DESCRIPTOR_SIZE];
        let attrs = u32::from_le_bytes(d[36..40].try_into().unwrap());
        let size = ((u32::from_le_bytes(d[64..68].try_into().unwrap()) as u64) << 32)
            | u32::from_le_bytes(d[68..72].try_into().unwrap()) as u64;
        let units: Vec<u16> = d[72..72 + 520]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        let name = String::from_utf16(&units).map_err(|_| ClipError::BadFileGroup)?;
        if name.is_empty() || name.starts_with('\\') || name.contains(':') || name.split(['\\', '/']).any(|p| p == "..") {
            return Err(ClipError::BadFileName(name));
        }
        out.push(FileDesc { name, size, is_dir: attrs & FILE_ATTRIBUTE_DIRECTORY != 0 });
    }
    Ok(out)
}

/// Richtung prüfen.
pub fn allows_host_to_target(d: ClipboardDirection) -> bool {
    matches!(d, ClipboardDirection::Both | ClipboardDirection::HostToTarget)
}

pub fn allows_target_to_host(d: ClipboardDirection) -> bool {
    matches!(d, ClipboardDirection::Both | ClipboardDirection::TargetToHost)
}

/// Protokollzeile ohne Inhalt: nur Länge und Prüfsumme.
pub fn audit_line(direction: &str, kind: &str, data: &[u8]) -> String {
    let sum = Sha256::digest(data);
    let hex: String = sum.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("Zwischenablage {direction} {kind}: {} Bytes, SHA-256 {hex}…", data.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_roundtrip_with_line_endings() {
        let cases = [
            ("Grüße ß 🚀👍🏽\ta", "Grüße ß 🚀👍🏽\ta"),
            ("a\nb", "a\nb"),
            ("a\r\nb\n", "a\nb\n"),
            ("einsamer\rWagenrücklauf", "einsamer\rWagenrücklauf"),
        ];
        for (input, expected) in cases {
            let enc = encode_text(input);
            assert_eq!(&enc[enc.len() - 2..], &[0, 0]);
            assert_eq!(decode_text(&enc).unwrap(), expected);
        }
        // Gesendet wird CRLF.
        let enc = encode_text("x\ny");
        let units: Vec<u16> = enc.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(units, [b'x' as u16, 13, 10, b'y' as u16, 0]);
    }

    #[test]
    fn invalid_utf16_reported() {
        let bytes = [0x41, 0x00, 0x00, 0xD8, 0x42, 0x00];
        assert!(matches!(decode_text(&bytes), Err(ClipError::InvalidUtf16 { index: 1, unit: 0xD800 })));
        assert!(matches!(decode_text(&[0x41, 0x00, 0x42]), Err(ClipError::OddLength(3))));
    }

    #[test]
    fn large_text() {
        let big = "Zeile mit Ümlaut 🚀\n".repeat(5000);
        assert_eq!(decode_text(&encode_text(&big)).unwrap(), big);
    }

    #[test]
    fn file_group_roundtrip_and_safety() {
        let files = vec![
            FileDesc { name: "Bericht ä.txt".into(), size: 5_000_000_000, is_dir: false },
            FileDesc { name: "Ordner".into(), size: 0, is_dir: true },
            FileDesc { name: "Ordner\\🚀.bin".into(), size: 3, is_dir: false },
        ];
        let enc = encode_file_group(&files).unwrap();
        assert_eq!(enc.len(), 4 + 3 * 592);
        assert_eq!(decode_file_group(&enc).unwrap(), files);
        assert!(encode_file_group(&[FileDesc { name: "..\\x".into(), size: 0, is_dir: false }]).is_err());
        let mut evil = encode_file_group(&[FileDesc { name: "a".into(), size: 1, is_dir: false }]).unwrap();
        for (i, u) in "..\\..\\etc".encode_utf16().enumerate() {
            evil[4 + 72 + i * 2..4 + 74 + i * 2].copy_from_slice(&u.to_le_bytes());
        }
        assert!(matches!(decode_file_group(&evil), Err(ClipError::BadFileName(_))));
        assert!(decode_file_group(&[1, 0, 0, 0]).is_err());
    }

    #[test]
    fn audit_has_no_content() {
        let l = audit_line("Host→Ziel", "Text", "geheimer Inhalt".as_bytes());
        assert!(!l.contains("geheim"));
        assert!(l.contains("15 Bytes"));
    }
}
