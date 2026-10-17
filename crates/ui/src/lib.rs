//! Qt-Oberfläche. Die prüfbare Logik (Baum, Arbeitsbereich, Tasten,
//! Farbschema, Sitzungen) liegt in Rust; die Widgets sind in C++ (`cpp/`)
//! und sprechen über das QObject `RjBackend` (cxx-qt) mit Rust.

pub mod app;
pub mod bridge;
pub mod keys;
pub mod patch;
pub mod sessions;
pub mod texts;
pub mod theme;
pub mod tree;
pub mod workspace;
