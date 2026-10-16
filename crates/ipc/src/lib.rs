//! Nachrichten zwischen Hauptprozess und Worker.
//!
//! Rahmen: Länge (u32, little endian, Länge von Kennung und Nutzlast) ‖
//! Kennung (u32, Kanal bzw. Anfragenummer) ‖ Nutzlast (`postcard`).
//! Transport sind die Standardkanäle des Worker-Prozesses (anonyme Pipes):
//! Der Kanal schließt sich mit dem Prozess, ohne Dateisystemeintrag und ohne
//! Rechtevergabe. Bilddaten laufen über geteilten Speicher ([`shm`]).

pub mod frame;
pub mod msg;
pub mod shm;
pub mod worker;

pub use frame::{FrameReader, FrameWriter, IpcError, MAX_FRAME};
pub use msg::*;

/// Version des Nachrichtenprotokolls; Worker und Hauptprozess müssen übereinstimmen.
pub const PROTOCOL_VERSION: u32 = 1;
