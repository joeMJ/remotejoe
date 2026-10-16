//! Nachrichtentypen.

use std::fmt;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Geheime Bytes im Kanal; werden beim Freigeben überschrieben und nie ausgegeben.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct SecretBytes(pub Vec<u8>);

impl fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretBytes(<{} Bytes>)", self.0.len())
    }
}

/// Zugangsdaten für die Dauer des Verbindungsaufbaus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialPayload {
    pub username: String,
    pub domain: Option<String>,
    pub password: SecretBytes,
    /// Privater Schlüssel (z. B. OpenSSH-Format aus einem KDBX-Anhang).
    #[serde(default)]
    pub private_key: Option<SecretBytes>,
}

/// Verbindungsauftrag an einen Worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectRequest {
    pub host: String,
    pub port: u16,
    pub width: u32,
    pub height: u32,
    /// Protokollspezifische Optionen als JSON (aus dem Index).
    pub options_json: String,
    pub connect_timeout_secs: u32,
    pub keepalive_secs: u32,
}

/// Inhalt der Zwischenablage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipData {
    Text(String),
    Files(Vec<ClipFile>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipFile {
    pub name: String,
    pub data: Vec<u8>,
}

/// Maustasten.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
}

/// Eingabeereignis aus der Oberfläche.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputEvent {
    /// Unicode-Zeichen (Tastaturbelegung unabhängig).
    Text(String),
    /// Taste nach X11-Keysym; `down` gedrückt/losgelassen.
    Key { keysym: u32, down: bool },
    /// Rohbytes (Terminal).
    Bytes(Vec<u8>),
    MouseMove { x: i32, y: i32 },
    MouseButton { x: i32, y: i32, button: MouseButton, down: bool },
}

/// Nachrichten an den Worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToWorker {
    Connect(ConnectRequest),
    Credentials(CredentialPayload),
    /// Antwort auf eine Rückfrage des Workers (z. B. Host-Schlüssel annehmen).
    Answer { question: u32, accept: bool },
    Input(InputEvent),
    Resize { width: u32, height: u32 },
    Clipboard(ClipData),
    Ping,
    Shutdown,
    /// Worker-spezifische Erweiterung.
    Custom(Vec<u8>),
}

/// Zustand einer Sitzung.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    Connecting,
    Authenticating,
    Connected,
    Disconnected { reason: String },
    Failed { reason: String },
}

/// Beschreibung eines Teilbilds im geteilten Speicher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameUpdate {
    /// Gesamtgröße des Bildpuffers.
    pub width: u32,
    pub height: u32,
    /// Geänderter Bereich.
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Fortlaufende Nummer des Pufferinhalts.
    pub serial: u64,
}

/// Nachrichten vom Worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FromWorker {
    Hello { pid: u32, kind: String, protocol_version: u32 },
    Heartbeat { seq: u64 },
    State(SessionState),
    /// Geteilter Speicher für Bilddaten wurde (neu) angelegt.
    SharedFrame { path: String, width: u32, height: u32 },
    Frame(FrameUpdate),
    /// Terminalausgabe (Rohbytes) für Terminal-Worker.
    Output(Vec<u8>),
    Clipboard(ClipData),
    /// Rückfrage an den Benutzer (Sicherheitsabfrage).
    Question { id: u32, text: String, detail: String },
    /// Hinweis für die Oberfläche (z. B. Zwischenablage gesperrt).
    Notice(String),
    /// Protokolleintrag; nie Kennwörter, Eingaben oder Zwischenablage-Inhalte.
    Log(String),
    Pong,
    Custom(Vec<u8>),
}
