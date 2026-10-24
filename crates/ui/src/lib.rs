//! Qt-Oberfläche. Die prüfbare Logik (Baum, Arbeitsbereich, Tasten,
//! Farbschema, Sitzungen) liegt in Rust; die Widgets sind in C++ (`cpp/`)
//! und sprechen über das QObject `RjBackend` (cxx-qt) mit Rust.

pub mod adminkeys;
pub mod alloc;
pub mod autokeys;
pub mod appearance;
pub mod aptools;
pub mod aruba;
pub mod keyring;
pub mod app;
pub mod bridge;
pub mod certcheck;
pub mod credcheck;
pub mod editor;
pub mod hyper;
pub mod icons;
pub mod idle;
pub mod jobs;
pub mod kdbxbackup;
pub mod keys;
pub mod macro_run;
pub mod mcp;
pub mod overview;
pub mod patch;
pub mod report;
pub mod sessions;
pub mod texts;
pub mod theme;
pub mod tree;
pub mod update;
pub mod vaultlock;
pub mod workspace;
