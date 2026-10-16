//! Kern von remotejoe: Datenmodell des Index, Vererbung, Zeichensatzprüfung,
//! Konfiguration und sicheres Schreiben von Dateien.

pub mod charset;
pub mod config;
pub mod fsutil;
pub mod inherit;
pub mod model;
pub mod schema;
pub mod secret;

pub use model::*;
pub use secret::Secret;
