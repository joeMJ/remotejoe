//! Qt-Oberfläche. Die prüfbare Logik (Baum, Arbeitsbereich, Tasten,
//! Farbschema, Sitzungen) liegt in Rust; die Widgets sind in C++ (`cpp/`)
//! und sprechen über das QObject `RjBackend` (cxx-qt) mit Rust.

pub mod alloc;
pub mod app;
pub mod bridge;
pub mod editor;
pub mod hyper;
pub mod idle;
pub mod jobs;
pub mod keys;
pub mod macro_run;
pub mod patch;
pub mod sessions;
pub mod texts;
pub mod theme;
pub mod tree;
pub mod update;
pub mod vaultlock;
pub mod workspace;
