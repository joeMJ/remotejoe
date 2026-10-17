//! Import aus Remote Desktop Manager (`.rdm` XML, JSON-Export) und Royal TS
//! (`.rtsz` ZIP mit JSON, `.rtsx` XML).
//!
//! Ablauf: Lesen in ein Zwischenmodell → Plan mit Vorschau und Bericht →
//! Übernahme in Index und KDBX. Kennwörter wandern nur in die KDBX.

pub mod plan;
pub mod rdm;
pub mod royal;
pub mod xml;

use rj_core::{Protocol, Secret};

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("Datei: {0}")]
    Io(#[from] std::io::Error),
    #[error("Zeichensatz: {0}")]
    Encoding(String),
    #[error("Format: {0}")]
    Format(String),
}

/// Zugangsdaten aus der Quelle.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceCredential {
    pub username: String,
    pub domain: Option<String>,
    pub password: Option<Secret>,
}

/// Eintrag aus der Quelle (Ordner oder Verbindung).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceItem {
    Folder {
        id: String,
        parent: Option<String>,
        name: String,
        order: i64,
    },
    Connection {
        id: String,
        parent: Option<String>,
        name: String,
        order: i64,
        protocol: Protocol,
        host: String,
        port: Option<u16>,
        /// Eigene Zugangsdaten der Verbindung.
        credential: Option<SourceCredential>,
        /// Verweis auf einen Zugangsdatensatz der Quelle.
        credential_ref: Option<String>,
    },
    /// Eigenständiger Zugangsdatensatz der Quelle.
    Credential { id: String, name: String, credential: SourceCredential },
}

/// Ergebnis des Lesens: Einträge in Quellreihenfolge und Beobachtungen.
#[derive(Debug, Clone, Default)]
pub struct SourceDocument {
    pub format: String,
    pub items: Vec<SourceItem>,
    /// Übersprungene Einträge (Name, Grund).
    pub skipped: Vec<(String, String)>,
    /// Unbekannte Felder (Feldname, Anzahl).
    pub unknown_fields: Vec<(String, usize)>,
    pub warnings: Vec<String>,
    /// Einträge, deren Kennwort verschlüsselt exportiert wurde und nicht übernommen werden kann.
    pub encrypted_passwords: usize,
}

impl SourceDocument {
    pub(crate) fn note_unknown(&mut self, field: &str) {
        match self.unknown_fields.iter_mut().find(|(f, _)| f == field) {
            Some(e) => e.1 += 1,
            None => self.unknown_fields.push((field.to_string(), 1)),
        }
    }
}

/// Größte zulässige Quelldatei bzw. entpackte Archivdatei.
pub const MAX_FILE: u64 = 64 * 1024 * 1024;
/// Größte Verschachtelungstiefe von Ordnern beim Import.
pub const MAX_DEPTH: usize = 64;

/// Liest eine Datei nach Endung.
pub fn read_file(path: &std::path::Path) -> Result<SourceDocument, ImportError> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.take(MAX_FILE + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(ImportError::Format(format!("Datei ist größer als {} MiB", MAX_FILE >> 20)));
    }
    let name = path.display().to_string();
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "rtsz" => royal::read_rtsz(&bytes),
        "rtsx" => royal::read_rtsx(text(&bytes, &name)?),
        "json" => rdm::read_json(text(&bytes, &name)?),
        _ => rdm::read_xml(text(&bytes, &name)?),
    }
}

pub(crate) fn text<'a>(bytes: &'a [u8], name: &str) -> Result<&'a str, ImportError> {
    let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    rj_core::charset::validate_utf8(b, name).map_err(|e| ImportError::Encoding(e.to_string()))
}
