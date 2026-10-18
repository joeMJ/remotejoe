//! Sitzungsverwaltung: je Sitzung ein Worker-Prozess über den Supervisor,
//! Terminal-Zustand für SSH, Bildpuffer für RDP/VNC, Wiederverbinden mit
//! Rückstau.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rj_core::Protocol;
use rj_ipc::*;
use rj_ssh::terminal::{selection_text_from_grid, Osc52Policy, Scheme, Terminal};
use rj_supervisor::{ExitReason, RestartPolicy, Supervisor, SupervisorEvent, WorkerId, WorkerSpec};
use serde::Serialize;
use uuid::Uuid;

use crate::workspace::SessionId;

/// Alles, was zum (Wieder-)Verbinden nötig ist.
#[derive(Clone)]
pub struct ConnectSpec {
    pub target: Uuid,
    pub title: String,
    pub protocol: Protocol,
    pub request: ConnectRequest,
    pub credentials: CredentialPayload,
    /// Weitere benannte Zugangsdatensätze, die die Tastenfolge einsetzt (`{cred:Name:password}`).
    pub named_credentials: BTreeMap<String, CredentialPayload>,
}

/// Nachrichten an den Worker für einen Schritt der Tastenfolge. Die Werte der Zugangsdaten werden erst hier
/// eingesetzt und erscheinen nirgends sonst.
fn auto_step_messages(protocol: Protocol, creds: &CredentialPayload, named: &BTreeMap<String, CredentialPayload>, step: &rj_core::autokeys::KeyStep) -> Result<Vec<ToWorker>, String> {
    use rj_core::autokeys::{expand_text, Key, KeyStep};
    let input = |e: InputEvent| ToWorker::Input(e);
    let mut out = Vec::new();
    match step {
        KeyStep::Pause(_) => {}
        KeyStep::Key(k) => {
            if protocol == Protocol::Ssh {
                out.push(input(InputEvent::Bytes(k.terminal_bytes().ok_or("Taste im Terminal nicht darstellbar")?)));
            } else if let (0, Key::Char(c)) = (k.mods, k.key) {
                // Gewöhnliche Zeichen als Text (unabhängig von der Tastaturbelegung).
                out.push(input(InputEvent::Text(c.to_string())));
            } else {
                let mods = k.modifier_keysyms();
                out.extend(mods.iter().map(|m| input(InputEvent::Key { keysym: *m, down: true })));
                out.push(input(InputEvent::Key { keysym: k.keysym(), down: true }));
                out.push(input(InputEvent::Key { keysym: k.keysym(), down: false }));
                out.extend(mods.iter().rev().map(|m| input(InputEvent::Key { keysym: *m, down: false })));
            }
        }
        KeyStep::Text(t) => {
            let field = |set: Option<&str>, name: &str| {
                let c = match set {
                    None => creds,
                    Some(n) => named.get(n)?,
                };
                match name {
                    "password" => Some(rj_core::Secret::from_bytes(c.password.0.clone())),
                    "username" => Some(rj_core::Secret::from_str_exact(&c.username)),
                    "domain" => c.domain.as_deref().filter(|d| !d.is_empty()).map(rj_core::Secret::from_str_exact),
                    _ => None,
                }
            };
            let secret = expand_text(t, &field)?;
            if protocol == Protocol::Ssh {
                out.push(input(InputEvent::Bytes(secret.as_bytes().to_vec())));
            } else {
                out.push(input(InputEvent::Text(String::from_utf8(secret.as_bytes().to_vec()).map_err(|_| "Text ist kein gültiges UTF-8".to_string())?)));
            }
        }
    }
    Ok(out)
}

/// Ab dieser Fläche (Bildpunkte) gilt eine Bildänderung als „Bewegung“ (etwa 100 × 100).
const SIGNIFICANT_UPDATE_AREA: u64 = 10_000;

fn is_significant_update(w: u32, h: u32) -> bool {
    w as u64 * h as u64 >= SIGNIFICANT_UPDATE_AREA
}

/// Qt-Tastencode von Esc.
const QT_KEY_ESCAPE: u32 = 0x0100_0000;

/// Größenlimit der Zwischenablage aus den Optionen (Vorgabe 16 MiB).
fn clipboard_limit(options_json: &str) -> u64 {
    serde_json::from_str::<serde_json::Value>(options_json)
        .ok()
        .and_then(|v| v.get("clipboard_limit").and_then(|l| l.as_u64()))
        .unwrap_or(16 * 1024 * 1024)
}

/// Sind Breite **und** Höhe in den Optionen eingetragen, gilt diese Größe fest;
/// sonst folgt die Bildsitzung der Größe ihres Fensters.
fn fixed_size(options_json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(options_json)
        .ok()
        .is_some_and(|v| v.get("width").is_some_and(|x| x.is_u64()) && v.get("height").is_some_and(|x| x.is_u64()))
}

/// Ereignis für die Oberfläche.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum UiEvent {
    State { s: SessionId, state: String, detail: String },
    /// Neuer Bildpuffer: `fd` ist ein geprüfter, versiegelter Deskriptor
    /// (Windows: Kennung), den die Oberfläche abbildet und dann schließt.
    SharedFrame { s: SessionId, fd: i64, w: u32, h: u32 },
    Frame { s: SessionId, x: u32, y: u32, w: u32, h: u32 },
    Terminal { s: SessionId },
    Question { s: SessionId, q: u32, text: String, detail: String, info: rj_ipc::msg::SecurityInfo, timeout_secs: u64 },
    /// Eine Rückfrage ist erledigt, ohne dass die Oberfläche geantwortet hat
    /// (`timeout`: abgelehnt nach Zeitüberschreitung, `closed`: Sitzung
    /// geschlossen, `ended`: Verbindung beendet).
    QuestionClosed { s: SessionId, q: u32, reason: String },
    Notice { s: SessionId, text: String },
    Clipboard { s: SessionId, text: String },
    ClipboardReadRequest { s: SessionId },
    Title { s: SessionId, title: String },
    /// Zustand einer Web-Sitzung: Seitentitel, Laden, Fehler, Zurück/Vor möglich.
    WebStatus { s: SessionId, title: String, loading: bool, progress: u8, error: Option<String>, back: bool, forward: bool },
    /// Menü des Web-Workers (Kontextmenü oder Auswahlliste) an Bildpunkt `x`, `y`.
    WebMenu { s: SessionId, id: u32, kind: String, x: i32, y: i32, items: serde_json::Value },
    /// Felder und Knöpfe einer Web-Seite (Antwort auf „Felder erkennen“): `users`, `passwords`, `submits`.
    WebFields { s: SessionId, fields: serde_json::Value },
    /// Ergebnis von „Felder erkennen“ für ein Ziel (`error`: Grund, wenn nichts erkannt wurde).
    FieldsDetected { target: String, error: Option<String>, fields: serde_json::Value },
    Bell { s: SessionId },
    Unlocked { targets: usize },
    /// „Alle im Ordner verbinden“ ist abgeschlossen.
    ConnectAllDone,
    /// Ampeln haben sich geändert; Baum neu zeichnen.
    TreeChanged,
    UnlockFailed { reason: String },
    /// Eine Sitzung aus einer Hintergrundaufgabe wurde geöffnet.
    SessionOpened { s: SessionId, job: u64 },
    /// Hintergrundaufgabe fehlgeschlagen (optional mit Rückfrage zum Host-Schlüssel).
    JobFailed { job: u64, error: String, hostkey: Option<serde_json::Value> },
    /// Ergebnis eines Abrufs (z. B. Gäste eines Hypervisors) als JSON.
    JobResult { job: u64, kind: String, target: String, json: String },
    /// Die Liste der laufenden Hintergrundaufgaben hat sich geändert.
    JobsChanged,
    /// Gesperrt (`idle` nach Leerlauf, `manual` von Hand).
    Locked { reason: String },
    /// Aufwachen aus dem Standby erkannt.
    Resumed,
    /// Ein Makro in einer Sitzung ist zu Ende (`ok`) oder fehlgeschlagen.
    MacroDone { s: SessionId, name: String, ok: bool, message: String },
}

/// Ausschnitt des Terminals für die Darstellung.
#[derive(Debug, Clone, Serialize)]
pub struct TermSnapshot {
    pub cols: usize,
    pub rows: usize,
    pub cursor: (usize, usize),
    pub cursor_visible: bool,
    /// Bildschirm-Programm (less, top): kein Rücklauf, das Mausrad sendet Pfeiltasten.
    pub alt_screen: bool,
    /// Zeilen, um die die Anzeige zurückgeblättert ist (0: am Ende).
    pub scroll_offset: usize,
    /// Zeilen im Rücklauf.
    pub history: usize,
    /// Hintergrund-, Vordergrund- und Cursorfarbe des Schemas (`#rrggbb`).
    pub bg: String,
    pub fg: String,
    pub cursor_color: String,
    /// Je Zeile Läufe gleicher Darstellung: (Text, Vordergrund, Hintergrund, Kennzeichen).
    pub lines: Vec<Vec<(String, String, String, u8)>>,
}

/// Größte Zeilen- und Spaltenzahl und Zellenzahl, die ein Schnappschuss des Workers haben darf.
const MAX_TERM_SIDE: u32 = 1000;
const MAX_TERM_CELLS: u32 = 250_000;

/// Prüft einen Schnappschuss des Workers, bevor die Oberfläche ihn benutzt.
fn valid_view(v: &TermView) -> bool {
    let (cols, rows) = (v.cols as usize, v.rows as usize);
    (1..=MAX_TERM_SIDE).contains(&v.cols)
        && (1..=MAX_TERM_SIDE).contains(&v.rows)
        && v.cols * v.rows <= MAX_TERM_CELLS
        && v.lines.len() == rows
        && v.grid.len() == rows
        && v.grid.iter().all(|r| r.cells.len() == cols && r.len as usize <= cols)
        && v.tail.len() <= 5
}

fn hex(c: Rgb3) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Darstellung für die Oberfläche aus dem Schnappschuss des Terminals.
fn snapshot_from_view(v: &TermView) -> TermSnapshot {
    TermSnapshot {
        cols: v.cols as usize,
        rows: v.rows as usize,
        cursor: (v.cursor.0 as usize, v.cursor.1 as usize),
        cursor_visible: v.cursor_visible,
        alt_screen: v.alt_screen,
        scroll_offset: v.scroll_offset as usize,
        history: v.history as usize,
        bg: hex(v.bg),
        fg: hex(v.fg),
        cursor_color: hex(v.cursor_color),
        lines: v.lines.iter().map(|l| l.iter().map(|r| (r.text.clone(), hex(r.fg), hex(r.bg), r.flags)).collect()).collect(),
    }
}

/// Leerer Bildschirm in der Größe des Verbindungsauftrags (bis der erste Schnappschuss des Workers da ist).
fn blank_view(cols: usize, rows: usize, scheme: &Scheme) -> TermView {
    let mut t = Terminal::with_history(cols.max(1), rows.max(1), Osc52Policy::default(), rj_ssh::terminal::HISTORY_MIN);
    t.set_scheme(scheme.clone());
    t.view()
}

struct Session {
    spec: ConnectSpec,
    worker: Option<WorkerId>,
    /// Zuletzt empfangener Schnappschuss des Terminals im Worker (nur SSH).
    view: Option<TermView>,
    /// Nur Vorführung: lokales Terminal ohne Worker, das nie Bytes von einem Ziel bekommt.
    demo_term: Option<Terminal>,
    state: String,
    /// Seit wann die Verbindung steht (Systemzeit); `None`, solange sie nicht steht.
    connected_at: Option<std::time::SystemTime>,
    attempts: u32,
    reconnect_at: Option<Instant>,
    /// Offene Leseanfragen des Ziels (OSC 52) nach Kennung des Workers.
    osc52_pending: Vec<u32>,
    /// Tastenfolge nach dem Verbinden (läuft oder wartet auf ihren Auslöser).
    auto: Option<crate::autokeys::AutoRun>,
    /// Wie oft die Sitzung verbunden war (für das Verhalten beim Neuverbinden).
    connect_count: u32,
    /// Das nächste Verbinden kommt vom automatischen Neuverbinden.
    auto_reconnect: bool,
    /// Zeitpunkt des letzten Bild- bzw. Ausgabeereignisses (Auslöser „ruhig“).
    last_activity: Instant,
    /// Hält den Bildspeicher einer Vorführsitzung am Leben.
    _demo_frame: Option<rj_ipc::shm::FrameBufferWriter>,
}


/// Einstellungen für das Wiederverbinden.
#[derive(Debug, Clone, Copy)]
pub struct Reconnect {
    pub auto: bool,
    pub max: Duration,
}

pub struct Manager {
    /// Bis wann nach dem Aufwachen sofort neu verbunden wird.
    resume_until: Option<Instant>,
    /// Sitzungen, die nach dem Aufwachen neu verbunden werden (Hinweis im Tab).
    resumed: std::collections::BTreeSet<SessionId>,
    /// Sitzungen über einen Tunnel: das Neuverbinden steuert die App (Tunnel zuerst).
    hold: std::collections::BTreeSet<SessionId>,
    /// Gesperrt: Zugangsdaten sind gelöscht; Neuverbinden wartet auf das Entsperren.
    locked: bool,
    deferred: std::collections::BTreeSet<SessionId>,
    /// Neuverbindungen gehaltener Sitzungen, die die App ausführt.
    app_requests: Vec<SessionId>,
    sup: Supervisor,
    sessions: BTreeMap<SessionId, Session>,
    by_worker: HashMap<WorkerId, SessionId>,
    worker_dir: PathBuf,
    next: SessionId,
    reconnect: Reconnect,
    pending: Vec<UiEvent>,
    /// Offene Sicherheitsrückfragen mit Frist; danach gilt „Ablehnen“.
    questions: BTreeMap<(SessionId, u32), Instant>,
    /// Frist für Sicherheitsrückfragen.
    pub question_timeout: Duration,
    /// Farben aller Terminals (auch neuer).
    scheme: Scheme,
    scrollback: usize,
    /// Abzweig der Terminalausgabe für laufende Makros.
    macro_taps: HashMap<SessionId, std::sync::mpsc::Sender<Vec<u8>>>,
}

/// Vorgabe der Frist für Sicherheitsrückfragen.
pub const QUESTION_TIMEOUT: Duration = Duration::from_secs(120);

/// Programmname des Workers je Protokoll.
pub fn worker_program(p: Protocol) -> Option<&'static str> {
    match p {
        Protocol::Rdp | Protocol::Hyperv => Some("rj-worker-rdp"),
        Protocol::Ssh => Some("rj-worker-ssh"),
        Protocol::Vnc => Some("rj-worker-vnc"),
        // Proxmox: die Weboberfläche des Hosts; Gäste haben eigene Konsolen.
        Protocol::Web | Protocol::Proxmox => Some("rj-worker-web"),
        Protocol::Psrp => Some("rj-worker-psrp"),
    }
}

fn exe_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

impl Manager {
    pub fn new(worker_dir: PathBuf, reconnect: Reconnect) -> Self {
        Manager {
            resume_until: None,
            resumed: Default::default(),
            hold: Default::default(),
            locked: false,
            deferred: Default::default(),
            app_requests: Vec::new(),
            sup: Supervisor::new(),
            sessions: BTreeMap::new(),
            by_worker: HashMap::new(),
            worker_dir,
            next: 1,
            reconnect,
            pending: Vec::new(),
            questions: BTreeMap::new(),
            question_timeout: QUESTION_TIMEOUT,
            scheme: Scheme::default(),
            scrollback: rj_ssh::terminal::HISTORY_DEFAULT,
            macro_taps: HashMap::new(),
        }
    }

    /// Setzt das Farbschema aller offenen und künftigen Terminals.
    pub fn set_scheme(&mut self, scheme: Scheme) {
        let cmd = TermCmd::SetScheme(TermScheme::from(&scheme));
        for id in self.terminal_sessions() {
            self.term_cmd(id, cmd.clone());
        }
        for x in self.sessions.values_mut() {
            if let Some(t) = x.demo_term.as_mut() {
                t.set_scheme(scheme.clone());
            }
        }
        self.scheme = scheme;
    }

    /// Größe des Rücklaufs aller Terminals (wirkt sofort und für neue).
    pub fn set_scrollback(&mut self, lines: usize) {
        for id in self.terminal_sessions() {
            self.term_cmd(id, TermCmd::SetHistory(lines.min(u32::MAX as usize) as u32));
        }
        for x in self.sessions.values_mut() {
            if let Some(t) = x.demo_term.as_mut() {
                t.set_history(lines);
            }
        }
        self.scrollback = lines;
    }

    /// Terminal-Anzeige um `lines` zurück (positiv) oder vor (negativ) blättern.
    pub fn scroll_terminal(&mut self, s: SessionId, lines: i32) {
        if let Some(t) = self.sessions.get_mut(&s).and_then(|x| x.demo_term.as_mut()) {
            t.scroll(lines);
        } else {
            self.term_cmd(s, TermCmd::Scroll(lines));
        }
    }

    pub fn scroll_terminal_page(&mut self, s: SessionId, back: bool) {
        if let Some(t) = self.sessions.get_mut(&s).and_then(|x| x.demo_term.as_mut()) {
            t.scroll_page(back);
        } else {
            self.term_cmd(s, TermCmd::ScrollPage { back });
        }
    }

    /// Sitzungen mit Terminal (SSH).
    fn terminal_sessions(&self) -> Vec<SessionId> {
        self.sessions.iter().filter(|(_, x)| x.spec.protocol == Protocol::Ssh).map(|(id, _)| *id).collect()
    }

    /// Befehl an das Terminal im Worker einer SSH-Sitzung.
    fn term_cmd(&mut self, s: SessionId, cmd: TermCmd) {
        if self.sessions.get(&s).is_some_and(|x| x.spec.protocol == Protocol::Ssh) {
            self.send(s, &ToWorker::Term(cmd));
        }
    }

    /// Ordner der Worker: neben dem Hauptprogramm.
    pub fn default_worker_dir() -> PathBuf {
        std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn count(&self) -> usize {
        self.sessions.len()
    }

    pub fn title(&self, s: SessionId) -> Option<&str> {
        self.sessions.get(&s).map(|x| x.spec.title.as_str())
    }

    pub fn protocol(&self, s: SessionId) -> Option<Protocol> {
        self.sessions.get(&s).map(|x| x.spec.protocol)
    }

    /// Angaben für die Fußzeile: Ziel, Benutzer und Beginn der Verbindung. Enthält nie Kennwörter.
    pub fn info(&self, s: SessionId) -> Option<serde_json::Value> {
        let x = self.sessions.get(&s)?;
        let user = match (&x.spec.credentials.domain, x.spec.credentials.username.as_str()) {
            (_, "") => String::new(),
            (Some(d), u) if !d.is_empty() => format!("{d}\\{u}"),
            (_, u) => u.to_string(),
        };
        let since = x.connected_at.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs());
        Some(serde_json::json!({
            "title": x.spec.title,
            "target": x.spec.target,
            "protocol": x.spec.protocol.as_str(),
            "host": x.spec.request.host,
            "port": x.spec.request.port,
            "user": user,
            "state": x.state,
            "since": since,
        }))
    }

    pub(crate) fn spec(&self, s: SessionId) -> Option<&ConnectSpec> {
        self.sessions.get(&s).map(|x| &x.spec)
    }

    /// Alle offenen Sitzungen.
    pub fn ids(&self) -> Vec<SessionId> {
        let mut v: Vec<SessionId> = self.sessions.keys().copied().collect();
        v.sort_unstable();
        v
    }

    pub fn target(&self, s: SessionId) -> Option<Uuid> {
        self.sessions.get(&s).map(|x| x.spec.target)
    }

    pub fn state(&self, s: SessionId) -> Option<&str> {
        self.sessions.get(&s).map(|x| x.state.as_str())
    }

    fn emit_state(&mut self, s: SessionId, state: &str, detail: String) {
        if let Some(x) = self.sessions.get_mut(&s) {
            // Die Zeit beginnt mit dem Übergang auf „connected“ und endet mit jedem anderen Zustand.
            if state == "connected" {
                if x.state != "connected" || x.connected_at.is_none() {
                    x.connected_at = Some(std::time::SystemTime::now());
                }
            } else {
                x.connected_at = None;
                // Ohne Verbindung läuft keine Tastenfolge weiter.
                x.auto = None;
            }
            x.state = state.to_string();
        }
        self.pending.push(UiEvent::State { s, state: state.to_string(), detail });
        if matches!(state, "failed" | "disconnected" | "crashed") {
            self.drop_questions(s, false, "ended");
        }
    }

    /// Beendet die offenen Rückfragen einer Sitzung; `reject` schickt dem
    /// Worker ausdrücklich „Ablehnen“.
    fn drop_questions(&mut self, s: SessionId, reject: bool, reason: &str) {
        let open: Vec<u32> = self.questions.range((s, 0)..=(s, u32::MAX)).map(|((_, q), _)| *q).collect();
        for q in open {
            self.questions.remove(&(s, q));
            if reject {
                self.send(s, &ToWorker::Answer { question: q, accept: false });
            }
            self.pending.push(UiEvent::QuestionClosed { s, q, reason: reason.into() });
        }
    }

    /// Zahl der offenen Sicherheitsrückfragen.
    pub fn open_questions(&self) -> usize {
        self.questions.len()
    }

    /// Neue Sitzung öffnen (startet den Worker sofort).
    pub fn open(&mut self, spec: ConnectSpec) -> SessionId {
        let id = self.next;
        self.next += 1;
        let view = (spec.protocol == Protocol::Ssh).then(|| blank_view(spec.request.width as usize, spec.request.height as usize, &self.scheme));
        self.sessions.insert(id, Session { spec, worker: None, view, demo_term: None, state: String::new(), connected_at: None, attempts: 0, reconnect_at: None, osc52_pending: Vec::new(), auto: None, connect_count: 0, auto_reconnect: false, last_activity: Instant::now(), _demo_frame: None });
        self.start(id);
        id
    }

    fn start(&mut self, id: SessionId) {
        let Some(sess) = self.sessions.get(&id) else { return };
        let Some(prog) = worker_program(sess.spec.protocol) else { return };
        let path = self.worker_dir.join(exe_name(prog));
        if !path.exists() {
            self.emit_state(id, "failed", format!("Worker {prog} nicht gefunden"));
            return;
        }
        let mut ws = WorkerSpec::new(path);
        ws.restart = RestartPolicy { enabled: false, ..Default::default() };
        if matches!(sess.spec.protocol, Protocol::Web | Protocol::Proxmox) {
            // WebKit/WebView2 reservieren sehr viel Adressraum; keine RLIMIT_AS-Grenze.
            ws.memory_limit = None;
        }
        match self.sup.spawn(ws) {
            Ok((wid, _pid)) => {
                let spec = self.sessions[&id].spec.clone();
                self.by_worker.insert(wid, id);
                let x = self.sessions.get_mut(&id).expect("vorhanden");
                x.worker = Some(wid);
                let _ = self.sup.send(wid, &ToWorker::Connect(spec.request));
                let _ = self.sup.send(wid, &ToWorker::Credentials(spec.credentials));
                if spec.protocol == Protocol::Ssh {
                    // Farben und Rücklauf gelten für das neue Terminal im Worker; ein laufendes Makro behält den Abgriff.
                    let _ = self.sup.send(wid, &ToWorker::Term(TermCmd::SetScheme(TermScheme::from(&self.scheme))));
                    let _ = self.sup.send(wid, &ToWorker::Term(TermCmd::SetHistory(self.scrollback.min(u32::MAX as usize) as u32)));
                    if self.macro_taps.contains_key(&id) {
                        let _ = self.sup.send(wid, &ToWorker::Term(TermCmd::Tap(true)));
                    }
                }
                self.emit_state(id, "connecting", String::new());
            }
            Err(e) => self.emit_state(id, "failed", e.to_string()),
        }
    }

    /// Vorführsitzung mit Terminalinhalt (ohne Worker).
    pub fn open_demo_terminal(&mut self, title: &str, content: &str, cols: usize, rows: usize) -> SessionId {
        let id = self.next;
        self.next += 1;
        let mut term = Terminal::new(cols, rows, Osc52Policy::default());
        term.set_scheme(self.scheme.clone());
        term.feed(content.as_bytes());
        let spec = demo_spec(title, Protocol::Ssh);
        self.sessions.insert(id, Session { spec, worker: None, view: None, demo_term: Some(term), state: "connected".into(), connected_at: Some(std::time::SystemTime::now()), attempts: 0, reconnect_at: None, osc52_pending: Vec::new(), auto: None, connect_count: 0, auto_reconnect: false, last_activity: Instant::now(), _demo_frame: None });
        self.pending.push(UiEvent::State { s: id, state: "connected".into(), detail: String::new() });
        self.pending.push(UiEvent::Terminal { s: id });
        id
    }

    /// Vorführsitzung mit erzeugtem Bild (ohne Worker).
    /// Testhilfe: verbundene Vorführsitzung für ein bestimmtes Ziel.
    #[cfg(test)]
    pub(crate) fn open_demo_for_target(&mut self, target: Uuid, state: &str) -> SessionId {
        let id = self.open_demo_terminal("test", "", 80, 24);
        if let Some(x) = self.sessions.get_mut(&id) {
            x.spec.target = target;
            x.state = state.to_string();
        }
        id
    }

    pub fn open_demo_frame(&mut self, title: &str, w: u32, h: u32, hue: u8) -> SessionId {
        let id = self.next;
        self.next += 1;
        let mut fb = rj_ipc::shm::FrameBufferWriter::create(w, h).expect("Bildspeicher");
        let px = fb.pixels_mut();
        for y in 0..h as usize {
            for x in 0..w as usize {
                let i = (y * w as usize + x) * 4;
                let band = (y * 8 / h as usize) as u8;
                px[i] = hue.wrapping_add(band * 20);
                px[i + 1] = (x * 200 / w as usize) as u8 + 30;
                px[i + 2] = 60 + band * 10;
                px[i + 3] = 255;
                if y < 28 {
                    px[i] = 40;
                    px[i + 1] = 40;
                    px[i + 2] = 40;
                }
            }
        }
        let fd = fb.share_handle().map(into_raw).unwrap_or(-1);
        let spec = demo_spec(title, Protocol::Rdp);
        self.sessions.insert(id, Session { spec, worker: None, view: None, demo_term: None, state: "connected".into(), connected_at: Some(std::time::SystemTime::now()), attempts: 0, reconnect_at: None, osc52_pending: Vec::new(), auto: None, connect_count: 0, auto_reconnect: false, last_activity: Instant::now(), _demo_frame: Some(fb) });
        self.pending.push(UiEvent::State { s: id, state: "connected".into(), detail: String::new() });
        self.pending.push(UiEvent::SharedFrame { s: id, fd, w, h });
        self.pending.push(UiEvent::Frame { s: id, x: 0, y: 0, w, h });
        id
    }

    fn worker_of(&self, s: SessionId) -> Option<WorkerId> {
        self.sessions.get(&s).and_then(|x| x.worker)
    }

    fn send(&mut self, s: SessionId, m: &ToWorker) {
        if let Some(w) = self.worker_of(s) {
            let _ = self.sup.send(w, m);
        }
    }

    /// Rohe Eingabe (z. B. von einem Makro) an das Terminal der Sitzung senden.
    pub fn send_bytes(&mut self, s: SessionId, bytes: Vec<u8>) {
        if self.sessions.get(&s).is_some_and(|x| x.spec.protocol == Protocol::Ssh) {
            self.send(s, &ToWorker::Input(InputEvent::Bytes(bytes)));
        }
    }

    /// Ausgabe der Sitzung an ein Makro weitergeben (bis `detach_macro_tap`).
    pub fn attach_macro_tap(&mut self, s: SessionId, tap: std::sync::mpsc::Sender<Vec<u8>>) {
        self.macro_taps.insert(s, tap);
        self.term_cmd(s, TermCmd::Tap(true));
    }

    pub fn detach_macro_tap(&mut self, s: SessionId) {
        if self.macro_taps.remove(&s).is_some() {
            self.term_cmd(s, TermCmd::Tap(false));
        }
    }

    /// Sichtbarer Text des Terminals (für den Start eines Makros: Eingabeaufforderung steht schon da).
    pub fn screen_text(&self, s: SessionId) -> Option<String> {
        let x = self.sessions.get(&s)?;
        let tail = match (&x.demo_term, &x.view) {
            (Some(t), _) => t.tail_text(),
            (None, Some(v)) => v.tail.clone(),
            _ => return None,
        };
        (!tail.is_empty()).then(|| tail.join("\n"))
    }

    /// Zugangsdaten der Sitzung (für Platzhalter in Makros).
    pub fn credentials(&self, s: SessionId) -> Option<CredentialPayload> {
        self.sessions.get(&s).map(|x| x.spec.credentials.clone())
    }

    /// Taste aus der Oberfläche.
    pub fn key(&mut self, s: SessionId, qt_key: u32, modifiers: u32, text: &str, down: bool) {
        // Esc bricht eine laufende oder wartende Tastenfolge ab (und geht nicht ans Ziel).
        if qt_key == QT_KEY_ESCAPE && self.sessions.get(&s).is_some_and(|x| x.auto.is_some()) {
            if down {
                self.cancel_auto_keys(s, "Tastenfolge abgebrochen");
            }
            return;
        }
        let Some(sess) = self.sessions.get(&s) else { return };
        if sess.spec.protocol == Protocol::Ssh {
            if down {
                let bytes = crate::keys::terminal_bytes(qt_key, modifiers, text, false);
                if !bytes.is_empty() {
                    // Eingabe springt ans Ende zurück.
                    self.scroll_bottom(s);
                    self.send(s, &ToWorker::Input(InputEvent::Bytes(bytes)));
                }
            }
            return;
        }
        if let Some(ev) = image_key_input(qt_key, modifiers, text, down) {
            self.send(s, &ToWorker::Input(ev));
        }
    }

    pub fn mouse(&mut self, s: SessionId, x: i32, y: i32, button: Option<MouseButton>, down: bool) {
        let ev = match button {
            None => InputEvent::MouseMove { x, y },
            Some(b) => InputEvent::MouseButton { x, y, button: b, down },
        };
        self.send(s, &ToWorker::Input(ev));
    }

    pub fn resize(&mut self, s: SessionId, w: u32, h: u32) {
        if let Some(sess) = self.sessions.get(&s) {
            // Bild-Sitzungen mit fest eingetragener Breite und Höhe folgen dem Fenster nicht.
            if sess.spec.protocol != Protocol::Ssh && fixed_size(&sess.spec.request.options_json) {
                return;
            }
        }
        if let Some(t) = self.sessions.get_mut(&s).and_then(|x| x.demo_term.as_mut()) {
            t.resize(w as usize, h as usize);
        }
        self.send(s, &ToWorker::Resize { width: w, height: h });
    }

    /// Einfügen ins Terminal (mit Klammerung) bzw. Zwischenablage anbieten (RDP/VNC).
    pub fn paste(&mut self, s: SessionId, text: &str) {
        if self.sessions.get(&s).is_some_and(|x| x.spec.protocol == Protocol::Ssh) {
            // Der Worker bildet die Bytes (Zeilenenden, Klammerung) und springt ans Ende.
            self.term_cmd(s, TermCmd::Paste(text.to_string()));
        } else {
            self.send(s, &ToWorker::Clipboard(ClipData::Text(text.to_string())));
        }
    }

    /// Anzeige ans Ende zurück (Eingabe).
    fn scroll_bottom(&mut self, s: SessionId) {
        if let Some(t) = self.sessions.get_mut(&s).and_then(|x| x.demo_term.as_mut()) {
            t.scroll_bottom();
        }
        self.term_cmd(s, TermCmd::ScrollBottom);
    }

    /// Befehl an eine Web-Sitzung (Zurück, Vor, Neu laden, Fokus, Bearbeiten, Menüauswahl).
    pub fn web_command(&mut self, s: SessionId, cmd: &serde_json::Value) -> Result<(), String> {
        if !self.sessions.get(&s).is_some_and(|x| matches!(x.spec.protocol, Protocol::Web | Protocol::Proxmox)) {
            return Err("Keine Web-Sitzung".into());
        }
        let op = cmd.get("op").and_then(|v| v.as_str()).unwrap_or("");
        if !matches!(op, "nav" | "focus" | "edit" | "menu" | "detect") {
            return Err(format!("Unbekannter Web-Befehl „{op}“"));
        }
        self.send(s, &ToWorker::Custom(cmd.to_string().into_bytes()));
        Ok(())
    }

    /// Lokale Zwischenablage an eine grafische Sitzung anbieten.
    pub fn offer_clipboard(&mut self, s: SessionId, text: &str) {
        if self.sessions.get(&s).is_some_and(|x| x.spec.protocol != Protocol::Ssh) {
            self.send(s, &ToWorker::Clipboard(ClipData::Text(text.to_string())));
        }
    }

    /// Antwort auf eine OSC-52-Leseanfrage.
    pub fn clipboard_read_answer(&mut self, s: SessionId, allow: bool, text: &str) {
        let Some(sess) = self.sessions.get_mut(&s) else { return };
        let pend = std::mem::take(&mut sess.osc52_pending);
        // Der Worker schreibt die Antwort ans Ziel; bei Ablehnung verwirft er die Anfrage.
        for id in pend {
            self.term_cmd(s, TermCmd::ClipboardAnswer { id, text: allow.then(|| text.to_string()) });
        }
    }

    pub fn answer(&mut self, s: SessionId, q: u32, accept: bool) {
        // Nach Ablauf der Frist zählt keine Zustimmung mehr.
        if self.questions.remove(&(s, q)).is_none() {
            return;
        }
        self.send(s, &ToWorker::Answer { question: q, accept });
    }

    /// Text der Auswahl aus den Zellen des letzten Schnappschusses (ohne Rückfrage an den Worker).
    pub fn select(&mut self, s: SessionId, start: (usize, usize), end: (usize, usize)) -> Option<String> {
        let x = self.sessions.get(&s)?;
        match (&x.demo_term, &x.view) {
            (Some(t), _) => selection_text_from_grid(&t.grid_rows(), start, end),
            (None, Some(v)) => selection_text_from_grid(&v.grid, start, end),
            _ => None,
        }
    }

    pub fn terminal(&self, s: SessionId) -> Option<TermSnapshot> {
        let x = self.sessions.get(&s)?;
        match (&x.demo_term, &x.view) {
            (Some(t), _) => Some(snapshot_from_view(&t.view())),
            (None, Some(v)) => Some(snapshot_from_view(v)),
            _ => None,
        }
    }

    pub fn disconnect(&mut self, s: SessionId) {
        if let Some(w) = self.worker_of(s) {
            self.sup.stop(w);
        }
        if let Some(x) = self.sessions.get_mut(&s) {
            x.reconnect_at = None;
        }
    }

    /// Nach dem Aufwachen: sofort neu verbinden, Rückstau beginnt von vorn.
    pub fn reconnect_resumed(&mut self, s: SessionId) {
        if let Some(x) = self.sessions.get_mut(&s) {
            x.attempts = 0;
            self.resumed.insert(s);
        }
        self.reconnect(s);
    }

    /// Zeitfenster nach dem Aufwachen, in dem abgebrochene Sitzungen sofort
    /// neu verbunden werden.
    pub fn set_resume_window(&mut self, until: Option<Instant>) {
        self.resume_until = until;
    }

    pub fn in_resume_window(&self) -> bool {
        self.resume_until.is_some_and(|t| Instant::now() < t)
    }

    /// Neuverbindungen, die die App ausführen soll (Tunnel bzw. Brücke zuerst).
    pub fn take_reconnect_requests(&mut self) -> Vec<SessionId> {
        std::mem::take(&mut self.app_requests)
    }

    pub fn is_held(&self, s: SessionId) -> bool {
        self.hold.contains(&s)
    }

    /// Hinweis im Tab einer Sitzung.
    pub fn push_notice(&mut self, s: SessionId, text: &str) {
        self.pending.push(UiEvent::Notice { s, text: text.to_string() });
    }

    pub fn set_credentials(&mut self, s: SessionId, c: CredentialPayload) {
        if let Some(x) = self.sessions.get_mut(&s) {
            x.spec.credentials = c;
        }
    }

    /// Sitzung über einen Tunnel oder eine Brücke: Neuverbinden steuert die App.
    pub fn set_hold(&mut self, s: SessionId, on: bool) {
        if on {
            self.hold.insert(s);
        } else {
            self.hold.remove(&s);
        }
    }

    pub fn mark_resumed(&mut self, s: SessionId) {
        self.resumed.insert(s);
    }

    /// Neuer Endpunkt (z. B. nach neu aufgebautem Tunnel).
    pub fn set_endpoint(&mut self, s: SessionId, host: &str, port: u16) {
        if let Some(x) = self.sessions.get_mut(&s) {
            x.spec.request.host = host.to_string();
            x.spec.request.port = port;
        }
    }

    /// Sperren: Zugangsdaten aller Sitzungen überschreiben; Neuverbinden
    /// wartet bis zum Entsperren.
    pub fn lock(&mut self) {
        self.locked = true;
        for x in self.sessions.values_mut() {
            x.spec.credentials = CredentialPayload { username: String::new(), domain: None, password: SecretBytes(Vec::new()), private_key: None };
        }
    }

    /// Entsperrt: Zugangsdaten je Sitzung neu setzen (`creds` je Ziel) und
    /// zurückgestellte Verbindungen aufbauen.
    pub fn unlock(&mut self, mut creds: impl FnMut(&ConnectSpec) -> Option<CredentialPayload>) {
        self.locked = false;
        for x in self.sessions.values_mut() {
            if let Some(c) = creds(&x.spec) {
                x.spec.credentials = c;
            }
        }
        for s in std::mem::take(&mut self.deferred) {
            self.reconnect(s);
        }
    }

    pub fn is_locked(&self) -> bool {
        self.locked
    }

    pub fn reconnect(&mut self, s: SessionId) {
        if self.locked {
            if self.sessions.contains_key(&s) && self.deferred.insert(s) {
                self.pending.push(UiEvent::Notice { s, text: crate::texts::text("session.locked_reconnect").to_string() });
            }
            return;
        }
        if let Some(w) = self.worker_of(s) {
            self.by_worker.remove(&w);
            self.sup.forget(w);
        }
        if let Some(x) = self.sessions.get_mut(&s) {
            x.worker = None;
            x.reconnect_at = None;
        }
        self.start(s);
    }

    pub fn close(&mut self, s: SessionId) {
        self.drop_questions(s, true, "closed");
        self.hold.remove(&s);
        self.resumed.remove(&s);
        self.deferred.remove(&s);
        if let Some(w) = self.worker_of(s) {
            self.by_worker.remove(&w);
            self.sup.stop(w);
            self.sup.forget(w);
        }
        self.sessions.remove(&s);
    }

    fn schedule_reconnect(&mut self, s: SessionId) {
        let window = self.in_resume_window();
        if !self.reconnect.auto && !window {
            return;
        }
        // Das nächste Verbinden ist ein automatisches Neuverbinden (für die Tastenfolge).
        if let Some(x) = self.sessions.get_mut(&s) {
            x.auto_reconnect = true;
        }
        if self.hold.contains(&s) {
            // Über einen Tunnel oder eine Konsolenbrücke: die App baut erst
            // Tunnel bzw. Brücke neu auf und verbindet dann.
            if window {
                self.resumed.insert(s);
            }
            self.app_requests.push(s);
            return;
        }
        let policy = RestartPolicy { enabled: true, base: Duration::from_secs(1), max: self.reconnect.max, stable_after: Duration::from_secs(30) };
        if let Some(x) = self.sessions.get_mut(&s) {
            // Erster Versuch nach dem Aufwachen ohne Wartezeit.
            let delay = if window && x.attempts == 0 { Duration::ZERO } else { policy.delay(x.attempts) };
            if window {
                self.resumed.insert(s);
            }
            x.attempts += 1;
            x.reconnect_at = Some(Instant::now() + delay);
            self.pending.push(UiEvent::Notice { s, text: crate::texts::text("session.reconnect_in").replace("%1", &delay.as_secs().to_string()) });
        }
    }

    /// Verarbeitet Ereignisse der Worker; liefert Ereignisse für die Oberfläche.
    pub fn poll(&mut self, wait: Duration) -> Vec<UiEvent> {
        for ev in self.sup.tick(wait) {
            match ev {
                SupervisorEvent::Started { .. } => {}
                SupervisorEvent::Message { id, msg } => {
                    let Some(&s) = self.by_worker.get(&id) else { continue };
                    self.on_message(s, msg);
                }
                SupervisorEvent::Exited { id, reason, .. } => {
                    let Some(s) = self.by_worker.remove(&id) else { continue };
                    if let Some(x) = self.sessions.get_mut(&s) {
                        x.worker = None;
                    }
                    if reason.is_crash() {
                        self.emit_state(s, "crashed", reason.describe());
                        self.schedule_reconnect(s);
                    } else if reason != ExitReason::Stopped {
                        self.emit_state(s, "disconnected", reason.describe());
                    } else {
                        self.emit_state(s, "disconnected", String::new());
                    }
                }
                SupervisorEvent::RestartScheduled { .. } => {}
            }
        }
        let due: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, x)| x.reconnect_at.is_some_and(|t| t <= Instant::now()))
            .map(|(id, _)| *id)
            .collect();
        for s in due {
            self.reconnect(s);
        }
        self.run_auto_keys();
        let now = Instant::now();
        let expired: Vec<(SessionId, u32)> = self.questions.iter().filter(|(_, t)| **t <= now).map(|(k, _)| *k).collect();
        for (s, q) in expired {
            self.questions.remove(&(s, q));
            self.send(s, &ToWorker::Answer { question: q, accept: false });
            self.pending.push(UiEvent::QuestionClosed { s, q, reason: "timeout".into() });
        }
        std::mem::take(&mut self.pending)
    }

    /// Startet die Tastenfolge des Ziels, wenn sie eingeschaltet ist und zum Verbinden passt.
    fn start_auto_keys(&mut self, s: SessionId) {
        let Some(x) = self.sessions.get_mut(&s) else { return };
        x.connect_count += 1;
        let (n, automatic) = (x.connect_count, std::mem::take(&mut x.auto_reconnect));
        x.auto = None;
        if !matches!(x.spec.protocol, Protocol::Rdp | Protocol::Ssh) {
            return;
        }
        let cfg: Option<rj_core::autokeys::AutoKeys> = serde_json::from_str::<serde_json::Value>(&x.spec.request.options_json)
            .ok()
            .and_then(|v| v.get("auto_keys").cloned())
            .and_then(|v| serde_json::from_value(v).ok());
        let Some(cfg) = cfg.filter(|c| c.applies(n, automatic)) else { return };
        match cfg.validate(x.spec.protocol == Protocol::Ssh) {
            Ok(()) => {
                let steps = cfg.steps().unwrap_or_default();
                let now = Instant::now();
                x.last_activity = now;
                eprintln!("[remotejoe] Tastenfolge Sitzung {s}: gestartet ({} Schritte, Auslöser {:?}, Wartezeit {} ms, spätestens {} ms)", steps.len(), cfg.trigger, cfg.wait_ms, cfg.max_ms);
                x.auto = Some(crate::autokeys::AutoRun::new(&cfg, steps, now));
                self.pending.push(UiEvent::Notice { s, text: crate::texts::text("autokeys.waiting").to_string() });
            }
            Err(e) => self.pending.push(UiEvent::Notice { s, text: format!("{}: {e}", crate::texts::text("autokeys.invalid")) }),
        }
    }

    /// Bricht die Tastenfolge der Sitzung ab; `message` erscheint kurz in der Fußzeile (leer: nur löschen).
    pub fn cancel_auto_keys(&mut self, s: SessionId, message: &str) {
        eprintln!("[remotejoe] Tastenfolge Sitzung {s}: beendet ({message})");
        if let Some(x) = self.sessions.get_mut(&s) {
            x.auto = None;
        }
        self.pending.push(UiEvent::Notice { s, text: message.to_string() });
    }

    /// Bringt alle Tastenfolgen voran und sendet fällige Schritte.
    fn run_auto_keys(&mut self) {
        let now = Instant::now();
        let ids: Vec<SessionId> = self.sessions.iter().filter(|(_, x)| x.auto.is_some()).map(|(id, _)| *id).collect();
        for s in ids {
            let Some(x) = self.sessions.get_mut(&s) else { continue };
            let last = x.last_activity;
            let was_running = x.auto.as_ref().is_some_and(|a| a.is_running());
            let Some(run) = x.auto.as_mut() else { continue };
            let (steps, done) = run.poll(now, last);
            let running = run.is_running();
            let (protocol, creds, named) = (x.spec.protocol, x.spec.credentials.clone(), x.spec.named_credentials.clone());
            if running && !was_running {
                let since_move = now.saturating_duration_since(last).as_millis();
                eprintln!("[remotejoe] Tastenfolge Sitzung {s}: Auslöser erreicht nach {} ms (letzte Bildbewegung vor {since_move} ms)", self.sessions.get(&s).and_then(|x| x.auto.as_ref()).map(|a| a.elapsed_ms(now)).unwrap_or(0));
                self.pending.push(UiEvent::Notice { s, text: crate::texts::text("autokeys.running").to_string() });
            }
            let mut failed = None;
            for step in steps {
                // Nur Art und Länge, nie der Inhalt (Kennwort).
                let what = match &step {
                    rj_core::autokeys::KeyStep::Key(k) => format!("Taste (Keysym {:#x}, Umschalter {})", k.keysym(), k.mods),
                    rj_core::autokeys::KeyStep::Text(t) => format!("Text ({} Zeichen)", t.chars().count()),
                    rj_core::autokeys::KeyStep::Pause(ms) => format!("Pause {ms} ms"),
                };
                eprintln!("[remotejoe] Tastenfolge Sitzung {s}: sende {what}");
                if let Err(e) = self.send_auto_step(s, protocol, &creds, &named, &step) {
                    failed = Some(e);
                    break;
                }
            }
            if let Some(e) = failed {
                self.cancel_auto_keys(s, &format!("{}: {e}", crate::texts::text("autokeys.failed")));
            } else if done {
                eprintln!("[remotejoe] Tastenfolge Sitzung {s}: fertig");
                if let Some(x) = self.sessions.get_mut(&s) {
                    x.auto = None;
                }
                // Hinweis löschen; Kennwort oder Text erscheint nirgends.
                self.pending.push(UiEvent::Notice { s, text: String::new() });
            }
        }
    }

    fn send_auto_step(&mut self, s: SessionId, protocol: Protocol, creds: &CredentialPayload, named: &BTreeMap<String, CredentialPayload>, step: &rj_core::autokeys::KeyStep) -> Result<(), String> {
        for m in auto_step_messages(protocol, creds, named, step)? {
            self.send(s, &m);
        }
        Ok(())
    }

    fn on_message(&mut self, s: SessionId, msg: FromWorker) {
        match msg {
            FromWorker::State(st) => {
                let deliberate = matches!(st, SessionState::Ended { .. });
                let (name, detail) = match st {
                    SessionState::Connecting => ("connecting", String::new()),
                    SessionState::Authenticating => ("connecting", String::new()),
                    SessionState::Connected => {
                        if let Some(x) = self.sessions.get_mut(&s) {
                            x.attempts = 0;
                        }
                        if self.resumed.remove(&s) {
                            self.pending.push(UiEvent::Notice { s, text: crate::texts::text("session.reconnected").to_string() });
                        }
                        ("connected", String::new())
                    }
                    SessionState::Disconnected { reason } => ("disconnected", reason),
                    SessionState::Failed { reason } => ("failed", reason),
                    // Gewollt beendet: wie „getrennt“ mit Grund und Knopf „Neu verbinden“, ohne Automatik.
                    SessionState::Ended { reason } => ("disconnected", reason),
                };
                self.emit_state(s, name, detail);
                if name == "connected" {
                    self.start_auto_keys(s);
                }
                if name == "disconnected" && !deliberate {
                    self.schedule_reconnect(s);
                }
            }
            FromWorker::SharedFrame { handle, width, height } => {
                // Nur geprüfte Puffer (versiegelt, groß genug) gehen an die Oberfläche.
                let shared = self.worker_of(s).and_then(|w| self.sup.take_shared(w, handle));
                match shared.map(|h| rj_ipc::shm::FrameBufferReader::validate(&h, width, height).map(|_| h)) {
                    Some(Ok(h)) => self.pending.push(UiEvent::SharedFrame { s, fd: into_raw(h), w: width, h: height }),
                    Some(Err(e)) => self.pending.push(UiEvent::Notice { s, text: format!("Bildpuffer des Workers abgelehnt: {e}") }),
                    None => self.pending.push(UiEvent::Notice { s, text: "Bildpuffer des Workers nicht übergeben".into() }),
                }
            }
            FromWorker::Frame(f) => {
                // Kleine Änderungen (blinkender Textcursor, Uhr) zählen nicht als Bewegung für den Auslöser „ruhig“.
                if let Some(x) = self.sessions.get_mut(&s).filter(|_| is_significant_update(f.w, f.h)) {
                    x.last_activity = Instant::now();
                    if let Some(a) = x.auto.as_mut() {
                        a.note_activity();
                    }
                }
                self.pending.push(UiEvent::Frame { s, x: f.x, y: f.y, w: f.w, h: f.h })
            }
            // Rohbytes kommen nur noch, solange ein Makro mitliest (Tap).
            FromWorker::Output(bytes) => {
                if let Some(x) = self.sessions.get_mut(&s) {
                    x.last_activity = Instant::now();
                    if let Some(a) = x.auto.as_mut() {
                        a.note_activity();
                    }
                }
                if let Some(tap) = self.macro_taps.get(&s) {
                    let _ = tap.send(bytes);
                }
            }
            FromWorker::Term(TermMsg::Snapshot(v)) => {
                if let Some(x) = self.sessions.get_mut(&s).filter(|x| x.spec.protocol == Protocol::Ssh) {
                    if valid_view(&v) {
                        x.view = Some(v);
                        // Für den Auslöser „ruhig“ der Tastenfolge: Das Terminal hat sich geändert.
                        x.last_activity = Instant::now();
                        if let Some(a) = x.auto.as_mut() {
                            a.note_activity();
                        }
                        self.pending.push(UiEvent::Terminal { s });
                    }
                }
            }
            FromWorker::Term(TermMsg::Event(e)) => match e {
                TermNotice::ClipboardStore(text) => {
                    let limit = self.sessions.get(&s).map(|x| clipboard_limit(&x.spec.request.options_json)).unwrap_or(16 * 1024 * 1024);
                    self.pending.push(clipboard_event(s, text, limit));
                }
                TermNotice::ClipboardLoad { id } => {
                    if let Some(x) = self.sessions.get_mut(&s) {
                        x.osc52_pending.push(id);
                        self.pending.push(UiEvent::ClipboardReadRequest { s });
                    }
                }
                TermNotice::Title(title) => self.pending.push(UiEvent::Title { s, title: rj_core::charset::mark_visible(&title, false).into_owned() }),
                TermNotice::Bell => self.pending.push(UiEvent::Bell { s }),
            },
            FromWorker::Clipboard(ClipData::Text(text)) => {
                // Größenlimit des Ziels auch für grafische Sitzungen; protokolliert wird nie der Inhalt.
                let limit = self.sessions.get(&s).map(|x| clipboard_limit(&x.spec.request.options_json)).unwrap_or(16 * 1024 * 1024);
                self.pending.push(clipboard_event(s, text, limit));
            }
            FromWorker::Clipboard(ClipData::Files(files)) => {
                let names: Vec<String> = files.iter().map(|f| f.name.clone()).collect();
                self.pending.push(UiEvent::Notice { s, text: format!("Dateien in der Zwischenablage empfangen: {}", names.join(", ")) });
            }
            FromWorker::Question { id, text, detail, info } => {
                self.questions.insert((s, id), Instant::now() + self.question_timeout);
                self.pending.push(UiEvent::Question { s, q: id, text, detail, info, timeout_secs: self.question_timeout.as_secs() });
            }
            FromWorker::Notice(text) => self.pending.push(UiEvent::Notice { s, text }),
            FromWorker::Custom(bytes) => {
                if self.sessions.get(&s).is_some_and(|x| matches!(x.spec.protocol, Protocol::Web | Protocol::Proxmox)) {
                    if let Some(e) = web_event(s, &bytes) {
                        self.pending.push(e);
                    }
                }
            }
            FromWorker::Log(_) | FromWorker::Pong | FromWorker::Hello { .. } | FromWorker::Heartbeat { .. } => {}
        }
    }

    pub fn shutdown(&mut self) {
        self.sup.shutdown();
    }
}

/// Text aus der Zwischenablage des Ziels, höchstens `limit` Bytes.
fn clipboard_event(s: SessionId, text: String, limit: u64) -> UiEvent {
    if text.len() as u64 <= limit {
        UiEvent::Clipboard { s, text }
    } else {
        UiEvent::Notice { s, text: format!("Zwischenablage vom Ziel verworfen: {} Bytes überschreiten das Limit von {limit} Bytes", text.len()) }
    }
}

/// Meldung des Web-Workers (JSON in `FromWorker::Custom`) in ein Ereignis der Oberfläche.
fn web_event(s: SessionId, bytes: &[u8]) -> Option<UiEvent> {
    let v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let text = |k: &str| v.get(k).and_then(|x| x.as_str()).map(|t| rj_core::charset::mark_visible(t, false).into_owned()).unwrap_or_default();
    let flag = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
    match v.get("ev")?.as_str()? {
        "status" => Some(UiEvent::WebStatus {
            s,
            title: text("title"),
            loading: flag("loading"),
            progress: v.get("progress").and_then(|x| x.as_u64()).unwrap_or(0).min(100) as u8,
            error: v.get("error").and_then(|x| x.as_str()).map(|t| rj_core::charset::mark_visible(t, false).into_owned()),
            back: flag("back"),
            forward: flag("forward"),
        }),
        "menu" => Some(UiEvent::WebMenu {
            s,
            id: v.get("id").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
            kind: text("kind"),
            x: v.get("x").and_then(|x| x.as_i64()).unwrap_or(0) as i32,
            y: v.get("y").and_then(|x| x.as_i64()).unwrap_or(0) as i32,
            items: v.get("items").cloned().unwrap_or_default(),
        }),
        "fields" => {
            // Beschriftungen sichtbar machen (versteckte Zeichen markieren); Selektoren bleiben unverändert.
            let list = |k: &str| -> serde_json::Value {
                let items = v.get(k).and_then(|x| x.as_array()).cloned().unwrap_or_default();
                serde_json::Value::Array(
                    items
                        .into_iter()
                        .map(|i| {
                            let sel = i.get("selector").and_then(|x| x.as_str()).unwrap_or_default().to_string();
                            let label = i.get("label").and_then(|x| x.as_str()).map(|t| rj_core::charset::mark_visible(t, false).into_owned()).unwrap_or_default();
                            serde_json::json!({ "selector": sel, "label": label })
                        })
                        .collect(),
                )
            };
            Some(UiEvent::WebFields { s, fields: serde_json::json!({ "users": list("users"), "passwords": list("passwords"), "submits": list("submits") }) })
        }
        _ => None,
    }
}

/// Gibt die Kennung an die Oberfläche ab (die sie nach dem Abbilden schließt).
fn into_raw(h: rj_ipc::shm::SharedHandle) -> i64 {
    #[cfg(unix)]
    return std::os::fd::IntoRawFd::into_raw_fd(h) as i64;
    #[cfg(windows)]
    return std::os::windows::io::IntoRawHandle::into_raw_handle(h) as usize as i64;
}

fn demo_spec(title: &str, protocol: Protocol) -> ConnectSpec {
    ConnectSpec {
        target: Uuid::nil(),
        title: title.into(),
        protocol,
        request: ConnectRequest { host: String::new(), port: 0, width: 80, height: 24, options_json: "{}".into(), connect_timeout_secs: 15, keepalive_secs: 30 },
        credentials: CredentialPayload { username: String::new(), domain: None, password: SecretBytes(Vec::new()), private_key: None },
        named_credentials: BTreeMap::new(),
    }
}

#[cfg(test)]
mod osc52_tests {
    use super::*;
    use rj_ssh::terminal::osc52_policy;

    #[test]
    fn policy_follows_target_options() {
        let p = osc52_policy("{}");
        assert!(p.allow_write && !p.allow_read_with_prompt);
        let p = osc52_policy(r#"{"osc52_write":false,"osc52_read":true,"clipboard_limit":5}"#);
        assert!(!p.allow_write && p.allow_read_with_prompt);
        assert_eq!(clipboard_limit(r#"{"clipboard_limit":5}"#), 5);
        assert_eq!(clipboard_limit("kaputt"), 16 * 1024 * 1024);
    }
}

/// Tastendruck für RDP/VNC: druckbarer Text als Text, alles mit Strg, Alt oder der Windows-Taste
/// (Kürzel wie Win+E, Strg+C) und Sondertasten als Taste. Text nur beim Drücken.
pub(crate) fn image_key_input(qt_key: u32, modifiers: u32, text: &str, down: bool) -> Option<InputEvent> {
    let with_modifier = modifiers & (crate::keys::CTRL | crate::keys::ALT | crate::keys::META) != 0;
    let printable = !text.is_empty() && text.chars().all(|c| !c.is_control());
    if printable && !with_modifier {
        return down.then(|| InputEvent::Text(text.to_string()));
    }
    crate::keys::keysym(qt_key).map(|k| InputEvent::Key { keysym: k, down })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_keys_windows_alt_and_menu() {
        use crate::keys::{ALT, CTRL, META, NATIVE_KEYSYM, SHIFT};
        let key = |k, d| Some(InputEvent::Key { keysym: k, down: d });
        // Windows-Taste allein, links und rechts (unter Linux als Keysym aus dem System).
        assert_eq!(image_key_input(NATIVE_KEYSYM | 0xffeb, 0, "", true), key(0xffeb, true));
        assert_eq!(image_key_input(NATIVE_KEYSYM | 0xffec, META, "", false), key(0xffec, false));
        // Win+E: das E geht als Taste, nicht als Text.
        assert_eq!(image_key_input('E' as u32, META, "e", true), key('e' as u32, true));
        // Alt+Tab, Alt+F4, Kontextmenü-Taste.
        assert_eq!(image_key_input(0x0100_0001, ALT, "\t", true), key(0xff09, true));
        assert_eq!(image_key_input(0x0100_0033, ALT, "", true), key(0xffc1, true));
        assert_eq!(image_key_input(0x0100_0055, 0, "", true), key(0xff67, true));
        assert_eq!(image_key_input(NATIVE_KEYSYM | 0xff67, 0, "", true), key(0xff67, true));
        // Unverändert: normaler Text (auch mit Umschalt und AltGr-Zeichen), Strg+C, rechte Umschalttaste.
        assert_eq!(image_key_input('A' as u32, SHIFT, "A", true), Some(InputEvent::Text("A".into())));
        assert_eq!(image_key_input('@' as u32, 0, "@", true), Some(InputEvent::Text("@".into())));
        assert_eq!(image_key_input('A' as u32, SHIFT, "A", false), None);
        assert_eq!(image_key_input('C' as u32, CTRL, "\u{3}", true), key('c' as u32, true));
        assert_eq!(image_key_input(NATIVE_KEYSYM | 0xffe2, SHIFT, "", true), key(0xffe2, true));
    }

    #[test]
    fn demo_sessions_and_snapshot() {
        let mut m = Manager::new(PathBuf::from("/nicht/vorhanden"), Reconnect { auto: false, max: Duration::from_secs(60) });
        let t = m.open_demo_terminal("Terminal", "\x1b[32mgrün\x1b[0m Grüße 🚀\r\nZeile 2", 40, 5);
        let f = m.open_demo_frame("Bild", 64, 48, 10);
        let ev = m.poll(Duration::from_millis(1));
        assert!(ev.iter().any(|e| matches!(e, UiEvent::SharedFrame { s, .. } if *s == f)));
        let snap = m.terminal(t).unwrap();
        assert_eq!(snap.rows, 5);
        assert_eq!(snap.lines[0][0].0, "grün");
        assert_eq!(snap.lines[0][0].1, "#00cd00");
        assert!(snap.lines[0].iter().any(|r| r.0.contains('🚀')));
        assert_eq!(m.select(t, (1, 0), (1, 6)).as_deref(), Some("Zeile 2"));
        assert!(m.terminal(f).is_none());
    }

    #[test]
    fn missing_worker_is_reported() {
        let mut m = Manager::new(PathBuf::from("/nicht/vorhanden"), Reconnect { auto: false, max: Duration::from_secs(60) });
        let s = m.open(demo_spec("x", Protocol::Ssh));
        let ev = m.poll(Duration::from_millis(1));
        assert!(ev.iter().any(|e| matches!(e, UiEvent::State { s: id, state, .. } if *id == s && state == "failed")));
    }
}

#[cfg(test)]
mod web_tests {
    use super::*;

    #[test]
    fn web_events_and_clipboard_limit() {
        let st = br#"{"ev":"status","title":"Seite\u202e","uri":"http://example.test/","loading":true,"progress":40,"error":null,"back":true,"forward":false}"#;
        match web_event(7, st) {
            Some(UiEvent::WebStatus { s, title, loading, progress, error, back, forward }) => {
                assert_eq!((s, loading, progress, error, back, forward), (7, true, 40, None, true, false));
                assert_eq!(title, "Seite⟦U+202E⟧");
            }
            other => panic!("{other:?}"),
        }
        let menu = br#"{"ev":"menu","id":3,"kind":"option","x":10,"y":20,"items":[{"action":"","label":"a","enabled":true,"selected":false}]}"#;
        assert!(matches!(web_event(7, menu), Some(UiEvent::WebMenu { id: 3, x: 10, y: 20, .. })));
        assert!(web_event(7, b"kaputt").is_none());
        assert!(matches!(clipboard_event(1, "abc".into(), 3), UiEvent::Clipboard { .. }));
        match clipboard_event(1, "abcd".into(), 3) {
            UiEvent::Notice { text, .. } => assert!(!text.contains("abcd")),
            other => panic!("{other:?}"),
        }
    }
}

#[cfg(test)]
mod fixed_size_tests {
    use super::fixed_size;

    #[test]
    fn only_both_values_make_the_size_fixed() {
        assert!(!fixed_size("{}"));
        assert!(!fixed_size(r#"{"width":1280}"#));
        assert!(!fixed_size(r#"{"height":720}"#));
        assert!(fixed_size(r#"{"width":1280,"height":720}"#));
        assert!(!fixed_size("kein json"));
    }
}

#[cfg(test)]
mod autokeys_tests {
    use super::*;
    use rj_core::autokeys::{parse_script, KeyStep};

    fn creds() -> CredentialPayload {
        CredentialPayload { username: "anna".into(), domain: None, password: SecretBytes(b"p\xc3\xa4ss{1}".to_vec()), private_key: None }
    }

    fn named() -> BTreeMap<String, CredentialPayload> {
        BTreeMap::from([("Sitzung".to_string(), CredentialPayload { username: "sitz".into(), domain: None, password: SecretBytes(b"zweites Kennwort".to_vec()), private_key: None })])
    }

    fn msgs(p: Protocol, line: &str) -> Result<Vec<ToWorker>, String> {
        auto_step_messages(p, &creds(), &named(), &parse_script(line).unwrap()[0])
    }

    fn manager_with(auto_keys: serde_json::Value) -> (Manager, SessionId) {
        let mut m = Manager::new(PathBuf::from("/nicht/vorhanden"), Reconnect { auto: false, max: Duration::from_secs(60) });
        let id = m.open_demo_terminal("Ziel", "", 80, 24);
        m.sessions.get_mut(&id).unwrap().spec.request.options_json = serde_json::json!({ "auto_keys": auto_keys }).to_string();
        m.poll(Duration::from_millis(1));
        (m, id)
    }

    fn notice_texts(m: &mut Manager) -> Vec<String> {
        m.poll(Duration::from_millis(1)).into_iter().filter_map(|e| if let UiEvent::Notice { text, .. } = e { Some(text) } else { None }).collect()
    }

    #[test]
    fn lifecycle_start_cancel_with_esc_and_reconnect_policy() {
        let on = serde_json::json!({ "enabled": true, "script": "Taste Enter", "trigger": "delay", "wait_ms": 60000 });
        let (mut m, id) = manager_with(on.clone());
        m.start_auto_keys(id);
        assert!(m.sessions[&id].auto.is_some(), "Tastenfolge wartet auf den Auslöser");
        assert_eq!(notice_texts(&mut m), vec![crate::texts::text("autokeys.waiting").to_string()]);
        // Esc bricht ab und geht nicht weiter; andere Tasten stören nicht.
        m.key(id, 0x41, 0, "a", true);
        assert!(m.sessions[&id].auto.is_some());
        m.key(id, QT_KEY_ESCAPE, 0, "", true);
        assert!(m.sessions[&id].auto.is_none());
        assert_eq!(notice_texts(&mut m), vec!["Tastenfolge abgebrochen".to_string()]);

        // Neuverbinden: „nur beim ersten Verbinden“ sendet beim zweiten Mal nichts.
        let (mut m, id) = manager_with(serde_json::json!({ "enabled": true, "script": "Taste Enter", "on_reconnect": "first_only" }));
        m.start_auto_keys(id);
        assert!(m.sessions[&id].auto.is_some());
        m.sessions.get_mut(&id).unwrap().auto = None;
        m.start_auto_keys(id);
        assert!(m.sessions[&id].auto.is_none(), "zweites Verbinden");
        // „nur nach manuellem Neuverbinden“: automatisches Neuverbinden sendet nicht, manuelles schon.
        let (mut m, id) = manager_with(serde_json::json!({ "enabled": true, "script": "Taste Enter", "on_reconnect": "manual_only" }));
        m.start_auto_keys(id);
        m.sessions.get_mut(&id).unwrap().auto = None;
        m.sessions.get_mut(&id).unwrap().auto_reconnect = true;
        m.start_auto_keys(id);
        assert!(m.sessions[&id].auto.is_none(), "automatisches Neuverbinden");
        m.start_auto_keys(id);
        assert!(m.sessions[&id].auto.is_some(), "manuelles Neuverbinden");
        // Ausgeschaltet oder ungültig: nichts bzw. nur ein Hinweis.
        let (mut m, id) = manager_with(serde_json::json!({ "enabled": false, "script": "Taste Enter" }));
        m.start_auto_keys(id);
        assert!(m.sessions[&id].auto.is_none());
        let (mut m, id) = manager_with(serde_json::json!({ "enabled": true, "script": "Taste Blubb" }));
        m.start_auto_keys(id);
        assert!(m.sessions[&id].auto.is_none());
        assert!(notice_texts(&mut m).iter().any(|t| t.starts_with("Tastenfolge ungültig")));
    }

    #[test]
    fn sequence_runs_to_the_end_and_clears_the_notice() {
        let (mut m, id) = manager_with(serde_json::json!({ "enabled": true, "script": "Taste Enter\nPause 20\nText {cred:password}\nTaste Enter", "trigger": "delay", "wait_ms": 0 }));
        m.start_auto_keys(id);
        let mut texts = notice_texts(&mut m);
        let t0 = Instant::now();
        while m.sessions[&id].auto.is_some() && t0.elapsed() < Duration::from_secs(5) {
            texts.extend(notice_texts(&mut m));
            std::thread::sleep(Duration::from_millis(5));
        }
        texts.extend(notice_texts(&mut m));
        assert!(m.sessions[&id].auto.is_none(), "Folge zu Ende");
        assert_eq!(texts.first().map(String::as_str), Some(crate::texts::text("autokeys.waiting")));
        assert!(texts.iter().any(|t| t == crate::texts::text("autokeys.running")));
        assert_eq!(texts.last().map(String::as_str), Some(""), "Hinweis wird am Ende gelöscht");
        assert!(!texts.iter().any(|t| t.contains("päss") || t.contains("pass")), "Kennwort nie in Hinweisen");
    }

    #[test]
    fn connection_loss_ends_the_sequence() {
        let (mut m, id) = manager_with(serde_json::json!({ "enabled": true, "script": "Taste Enter", "trigger": "delay", "wait_ms": 60000 }));
        m.start_auto_keys(id);
        assert!(m.sessions[&id].auto.is_some());
        m.emit_state(id, "disconnected", String::new());
        assert!(m.sessions[&id].auto.is_none());
    }

    #[test]
    fn small_updates_do_not_count_as_activity() {
        assert!(!is_significant_update(2, 20), "blinkender Cursor");
        assert!(!is_significant_update(90, 100), "kleines Feld");
        assert!(is_significant_update(100, 100));
        assert!(is_significant_update(1564, 849), "ganzes Bild");
    }

    #[test]
    fn rdp_keys_text_and_modifiers() {
        assert_eq!(msgs(Protocol::Rdp, "Taste Enter").unwrap(), vec![
            ToWorker::Input(InputEvent::Key { keysym: 0xff0d, down: true }),
            ToWorker::Input(InputEvent::Key { keysym: 0xff0d, down: false }),
        ]);
        // Gewöhnliches Zeichen: Text; Kombination: Umschalter drücken, Taste, Umschalter lösen (umgekehrt).
        assert_eq!(msgs(Protocol::Rdp, "Taste x").unwrap(), vec![ToWorker::Input(InputEvent::Text("x".into()))]);
        assert_eq!(msgs(Protocol::Rdp, "Taste Ctrl+Alt+t").unwrap(), vec![
            ToWorker::Input(InputEvent::Key { keysym: 0xffe3, down: true }),
            ToWorker::Input(InputEvent::Key { keysym: 0xffe9, down: true }),
            ToWorker::Input(InputEvent::Key { keysym: 't' as u32, down: true }),
            ToWorker::Input(InputEvent::Key { keysym: 't' as u32, down: false }),
            ToWorker::Input(InputEvent::Key { keysym: 0xffe9, down: false }),
            ToWorker::Input(InputEvent::Key { keysym: 0xffe3, down: false }),
        ]);
        // Kennwort wird byte-genau getippt, Klammern bleiben erhalten.
        assert_eq!(msgs(Protocol::Rdp, "Text {cred:password}").unwrap(), vec![ToWorker::Input(InputEvent::Text("päss{1}".into()))]);
        assert_eq!(msgs(Protocol::Rdp, "Text {cred:username}@x").unwrap(), vec![ToWorker::Input(InputEvent::Text("anna@x".into()))]);
        assert!(msgs(Protocol::Rdp, "Text {cred:domain}").unwrap_err().contains("leer"));
        // Zweites Kennwort aus einem benannten Zugangsdatensatz; ein unbekannter Name bricht ab.
        assert_eq!(msgs(Protocol::Rdp, "Text {cred:Sitzung:password}").unwrap(), vec![ToWorker::Input(InputEvent::Text("zweites Kennwort".into()))]);
        assert!(msgs(Protocol::Rdp, "Text {cred:Fremd:password}").unwrap_err().contains("Fremd"));
        assert!(msgs(Protocol::Rdp, "Pause 10").unwrap().is_empty());
    }

    #[test]
    fn ssh_sends_terminal_bytes() {
        assert_eq!(msgs(Protocol::Ssh, "Taste Enter").unwrap(), vec![ToWorker::Input(InputEvent::Bytes(b"\r".to_vec()))]);
        assert_eq!(msgs(Protocol::Ssh, "Text cd /var/log").unwrap(), vec![ToWorker::Input(InputEvent::Bytes(b"cd /var/log".to_vec()))]);
        assert_eq!(msgs(Protocol::Ssh, "Taste Ctrl+C").unwrap(), vec![ToWorker::Input(InputEvent::Bytes(vec![3]))]);
        assert!(msgs(Protocol::Ssh, "Taste Win+R").is_err());
        // Schrittfolgen des Beispiels aus der Beschreibung: Eingabe, Kennwort, Eingabe.
        let steps = parse_script("Taste Enter\nText {cred:password}\nTaste Enter").unwrap();
        assert!(matches!(steps[1], KeyStep::Text(_)));
    }
}
