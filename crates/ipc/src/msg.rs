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

/// Angaben zu einer Sicherheitsrückfrage (Host-Schlüssel oder Zertifikat).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityInfo {
    /// `hostkey` (SSH) oder `certificate` (TLS); leer bei sonstigen Fragen.
    pub kind: String,
    pub host: String,
    pub port: u16,
    /// Schlüsseltyp, z. B. `ssh-ed25519` oder `X.509`.
    pub key_type: String,
    /// Vollständiger Fingerabdruck (SHA-256).
    pub fingerprint: String,
    /// Bisher bekannter Fingerabdruck, wenn sich der Schlüssel geändert hat.
    pub previous: Option<String>,
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
    /// Feste Folge von RDP-Scancodes (Set 1, `0x100` = E0-Präfix) mit gedrückt/losgelassen: administrativer
    /// Tastaturmodus (Zeichen als Tastendrücke statt Unicode).
    Scancodes(Vec<(u32, bool)>),
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
    /// Befehl an die Terminal-Zustandsmaschine des Workers (nur SSH).
    Term(TermCmd),
}

/// Zustand einer Sitzung.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    Connecting,
    Authenticating,
    Connected,
    Disconnected { reason: String },
    Failed { reason: String },
    /// Gewollt beendet (z. B. in Windows abgemeldet): kein automatisches Neuverbinden.
    Ended { reason: String },
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
    /// `handle`: Linux die Nummer des über den Socket übergebenen
    /// Deskriptors, Windows die Kennung im Worker (siehe [`crate::fdpass`]).
    SharedFrame { handle: u64, width: u32, height: u32 },
    Frame(FrameUpdate),
    /// Terminalausgabe (Rohbytes) für Terminal-Worker.
    Output(Vec<u8>),
    Clipboard(ClipData),
    /// Rückfrage an den Benutzer (Sicherheitsabfrage). `info` trägt Art,
    /// Schlüsseltyp und vollständige Fingerabdrücke für die Anzeige.
    Question { id: u32, text: String, detail: String, info: SecurityInfo },
    /// Hinweis für die Oberfläche (z. B. Zwischenablage gesperrt).
    Notice(String),
    /// Protokolleintrag; nie Kennwörter, Eingaben oder Zwischenablage-Inhalte.
    Log(String),
    Pong,
    Custom(Vec<u8>),
    /// Schnappschuss oder Ereignis der Terminal-Zustandsmaschine (nur SSH).
    Term(TermMsg),
    /// Mauszeiger der Gegenstelle (RDP, VNC).
    Pointer(PointerShape),
}

/// Form des Mauszeigers der Gegenstelle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointerShape {
    /// 0 = Standardzeiger, 1 = ausgeblendet, 2 = Bild.
    pub kind: u8,
    pub width: u32,
    pub height: u32,
    pub hot_x: u32,
    pub hot_y: u32,
    /// BGRA32, `width × height` (nur bei `kind = 2`).
    pub bgra: Vec<u8>,
}

/// Farbe als (Rot, Grün, Blau).
pub type Rgb3 = [u8; 3];

/// Farben des Terminals: Vordergrund, Hintergrund, Cursor und die 16 ANSI-Farben.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermScheme {
    pub fg: Rgb3,
    pub bg: Rgb3,
    pub cursor: Rgb3,
    pub ansi: [Rgb3; 16],
}

/// Regel für OSC 52 (Zwischenablage durch das Ziel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Osc52Rule {
    pub allow_write: bool,
    pub allow_read_with_prompt: bool,
}

/// Befehle an das Terminal im Worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TermCmd {
    /// Anzeige um n Zeilen zurück (positiv) oder vor (negativ).
    Scroll(i32),
    ScrollPage { back: bool },
    ScrollBottom,
    SetScheme(TermScheme),
    /// Größe des Rücklaufs in Zeilen.
    SetHistory(u32),
    /// Rohbytes der Ausgabe zusätzlich als `FromWorker::Output` senden (Makro-Abgriff).
    Tap(bool),
    /// Text einfügen; der Worker bildet die Bytes (Zeilenenden, Klammerung) und sendet sie ans Ziel.
    Paste(String),
    SetOsc52(Osc52Rule),
    /// Antwort auf eine Leseanfrage (OSC 52); `None`: abgelehnt.
    ClipboardAnswer { id: u32, text: Option<String> },
}

/// Nachrichten des Terminals im Worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TermMsg {
    Snapshot(TermView),
    Event(TermNotice),
}

/// Ereignisse des Terminals für die Oberfläche.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TermNotice {
    /// Ziel setzt die Zwischenablage (OSC 52).
    ClipboardStore(String),
    /// Ziel möchte die Zwischenablage lesen; Antwort mit `TermCmd::ClipboardAnswer`.
    ClipboardLoad { id: u32 },
    Title(String),
    Bell,
}

/// Lauf gleicher Darstellung innerhalb einer Zeile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermRun {
    pub text: String,
    pub fg: Rgb3,
    pub bg: Rgb3,
    /// Bit 0 fett, 1 kursiv, 2 unterstrichen, 3 doppelt breit.
    pub flags: u8,
}

/// Eine Bildschirmzeile für die Textauswahl (eine Spalte je Eintrag).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridRow {
    /// Zeichen samt kombinierenden Zeichen; Platzhalter hinter doppelt breiten
    /// Zeichen sind leer.
    pub cells: Vec<String>,
    /// Belegte Länge (ohne Leerzeichen am Ende; bei umbrochener Zeile die volle Breite).
    pub len: u32,
    /// Die Zeile ist am Ende umbrochen (die nächste setzt sie fort).
    pub wrapped: bool,
    /// Die letzte Spalte ist der Platzhalter eines doppelt breiten Zeichens, das in der nächsten Zeile steht.
    pub lead_spacer: bool,
    /// Für diesen Fall das Zeichen, das die Auswahl am Zeilenende anhängt.
    pub lead_prev: Option<char>,
}

/// Schnappschuss des Terminals: alles, was die Oberfläche zum Zeichnen,
/// Auswählen und für den Makrostart braucht.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermView {
    pub cols: u32,
    pub rows: u32,
    pub cursor: (u32, u32),
    pub cursor_visible: bool,
    pub alt_screen: bool,
    pub scroll_offset: u32,
    pub history: u32,
    pub bg: Rgb3,
    pub fg: Rgb3,
    pub cursor_color: Rgb3,
    /// Je Zeile Läufe gleicher Darstellung.
    pub lines: Vec<Vec<TermRun>>,
    /// Je Zeile die Zellen für die Textauswahl.
    pub grid: Vec<GridRow>,
    /// Die letzten Zeilen bis zur letzten nichtleeren (höchstens 5, ohne Leerraum am Ende), für den Makrostart.
    pub tail: Vec<String>,
}

impl TermView {
    /// Ungefähre Größe in Bytes bei der Übertragung (ohne die Kodierung auszuführen).
    pub fn approx_size(&self) -> usize {
        let runs: usize = self.lines.iter().flatten().map(|r| r.text.len() + 9).sum();
        let grid: usize = self.grid.iter().map(|r| r.cells.iter().map(|c| c.len() + 1).sum::<usize>() + 12).sum();
        let tail: usize = self.tail.iter().map(|l| l.len() + 2).sum();
        96 + runs + grid + tail
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc<T: Serialize>(v: &T) -> Vec<u8> {
        postcard::to_stdvec(v).unwrap()
    }

    fn round<T: Serialize + serde::de::DeserializeOwned + PartialEq + fmt::Debug>(v: T) {
        let b = enc(&v);
        assert_eq!(postcard::from_bytes::<T>(&b).unwrap(), v);
    }

    fn scheme() -> TermScheme {
        let mut ansi = [[0u8; 3]; 16];
        for (i, c) in ansi.iter_mut().enumerate() {
            *c = [i as u8, 255 - i as u8, 7];
        }
        TermScheme { fg: [1, 2, 3], bg: [4, 5, 6], cursor: [7, 8, 9], ansi }
    }

    fn view() -> TermView {
        TermView {
            cols: 3,
            rows: 2,
            cursor: (1, 2),
            cursor_visible: true,
            alt_screen: false,
            scroll_offset: 4,
            history: 5,
            bg: [0, 0, 0],
            fg: [229, 229, 229],
            cursor_color: [1, 1, 1],
            lines: vec![vec![TermRun { text: "日本".into(), fg: [1, 2, 3], bg: [4, 5, 6], flags: 8 }], vec![]],
            grid: vec![
                GridRow { cells: vec!["日".into(), String::new(), "a\u{308}".into()], len: 3, wrapped: true, lead_spacer: false, lead_prev: None },
                GridRow { cells: vec![" ".into(); 3], len: 0, wrapped: false, lead_spacer: true, lead_prev: Some('x') },
            ],
            tail: vec!["rjtest@srv-01:~$".into()],
        }
    }

    #[test]
    fn new_messages_round_trip() {
        for c in [
            TermCmd::Scroll(-3),
            TermCmd::ScrollPage { back: true },
            TermCmd::ScrollBottom,
            TermCmd::SetScheme(scheme()),
            TermCmd::SetHistory(10_000),
            TermCmd::Tap(true),
            TermCmd::Paste("a\r\nb 日本".into()),
            TermCmd::SetOsc52(Osc52Rule { allow_write: true, allow_read_with_prompt: false }),
            TermCmd::ClipboardAnswer { id: 7, text: Some("ä".into()) },
            TermCmd::ClipboardAnswer { id: 8, text: None },
        ] {
            round(ToWorker::Term(c));
        }
        for m in [
            TermMsg::Snapshot(view()),
            TermMsg::Event(TermNotice::ClipboardStore("Grüße".into())),
            TermMsg::Event(TermNotice::ClipboardLoad { id: 3 }),
            TermMsg::Event(TermNotice::Title("Titel".into())),
            TermMsg::Event(TermNotice::Bell),
        ] {
            round(FromWorker::Term(m));
        }
    }

    /// Postcard kodiert Varianten nach Reihenfolge: die alten behalten ihre Nummer,
    /// die neuen hängen hinten an.
    #[test]
    fn old_messages_keep_their_encoding() {
        assert_eq!(enc(&ToWorker::Answer { question: 1, accept: true }), [2, 1, 1]);
        assert_eq!(enc(&ToWorker::Input(InputEvent::Bytes(vec![9]))), [3, 2, 1, 9]);
        assert_eq!(enc(&ToWorker::Input(InputEvent::Scancodes(vec![(0x2A, true)])))[..2], [3, 5]);
        assert_eq!(enc(&ToWorker::Resize { width: 80, height: 24 }), [4, 80, 24]);
        assert_eq!(enc(&ToWorker::Ping), [6]);
        assert_eq!(enc(&ToWorker::Shutdown), [7]);
        assert_eq!(enc(&ToWorker::Custom(vec![1])), [8, 1, 1]);
        assert_eq!(enc(&ToWorker::Term(TermCmd::ScrollBottom))[0], 9);

        assert_eq!(enc(&FromWorker::Heartbeat { seq: 5 }), [1, 5]);
        assert_eq!(enc(&FromWorker::Output(vec![7])), [5, 1, 7]);
        assert_eq!(enc(&FromWorker::Notice(String::new())), [8, 0]);
        assert_eq!(enc(&FromWorker::Log(String::new())), [9, 0]);
        assert_eq!(enc(&FromWorker::Pong), [10]);
        assert_eq!(enc(&FromWorker::Custom(vec![1])), [11, 1, 1]);
        assert_eq!(enc(&FromWorker::Term(TermMsg::Event(TermNotice::Bell)))[0], 12);
    }
}
