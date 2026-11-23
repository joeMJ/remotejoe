//! Qt-Oberfläche. Die prüfbare Logik (Baum, Arbeitsbereich, Tasten,
//! Farbschema, Sitzungen) liegt in Rust; die Widgets sind in C++ (`cpp/`)
//! und sprechen über das QObject `RjBackend` (cxx-qt) mit Rust.

pub mod adminkeys;
pub mod alloc;
pub mod autokeys;
pub mod appearance;
pub mod keyerrors;
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
pub mod cfgstore;
pub mod redact;
pub mod cli;
pub mod cli_service;
pub mod https_ticket;
pub mod host;
pub mod kdbxbackup;
pub mod keys;
pub mod macro_run;
pub mod mcp;
pub mod modules;
pub mod overview;
pub mod patch;
pub mod report;
pub mod sessions;
pub mod sitesetup;
pub mod testing;
pub mod texts;
pub mod legal;
pub mod license;
pub mod plugins;
pub mod plugin_fetch;
pub mod theme;
pub mod tree;
pub mod update;
pub mod vaultlock;
pub mod webcache;
pub mod workspace;
