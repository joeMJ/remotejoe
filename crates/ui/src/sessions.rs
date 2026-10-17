//! Sitzungsverwaltung: je Sitzung ein Worker-Prozess über den Supervisor,
//! Terminal-Zustand für SSH, Bildpuffer für RDP/VNC, Wiederverbinden mit
//! Rückstau.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rj_core::Protocol;
use rj_ipc::*;
use rj_ssh::terminal::{Osc52Policy, TermEvent, Terminal};
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
}

/// Ereignis für die Oberfläche.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum UiEvent {
    State { s: SessionId, state: String, detail: String },
    SharedFrame { s: SessionId, path: String, w: u32, h: u32 },
    Frame { s: SessionId, x: u32, y: u32, w: u32, h: u32 },
    Terminal { s: SessionId },
    Question { s: SessionId, q: u32, text: String, detail: String },
    Notice { s: SessionId, text: String },
    Clipboard { s: SessionId, text: String },
    ClipboardReadRequest { s: SessionId },
    Title { s: SessionId, title: String },
    Bell { s: SessionId },
    Unlocked { targets: usize },
    /// Ampeln haben sich geändert; Baum neu zeichnen.
    TreeChanged,
    UnlockFailed { reason: String },
}

/// Ausschnitt des Terminals für die Darstellung.
#[derive(Debug, Clone, Serialize)]
pub struct TermSnapshot {
    pub cols: usize,
    pub rows: usize,
    pub cursor: (usize, usize),
    pub cursor_visible: bool,
    /// Je Zeile Läufe gleicher Darstellung: (Text, Vordergrund, Hintergrund, Kennzeichen).
    pub lines: Vec<Vec<(String, String, String, u8)>>,
}

struct Session {
    spec: ConnectSpec,
    worker: Option<WorkerId>,
    term: Option<Terminal>,
    state: String,
    attempts: u32,
    reconnect_at: Option<Instant>,
    osc52_pending: Vec<std::sync::Arc<dyn Fn(&str) -> String + Send + Sync>>,
    /// Hält den Bildspeicher einer Vorführsitzung am Leben.
    _demo_frame: Option<rj_ipc::shm::FrameBufferWriter>,
    pid: Option<u32>,
}

fn remove_frame_files(pid: u32) {
    let dir = if cfg!(target_os = "linux") { PathBuf::from("/dev/shm") } else { std::env::temp_dir() };
    let prefix = format!("remotejoe-{pid}-");
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// Einstellungen für das Wiederverbinden.
#[derive(Debug, Clone, Copy)]
pub struct Reconnect {
    pub auto: bool,
    pub max: Duration,
}

pub struct Manager {
    sup: Supervisor,
    sessions: BTreeMap<SessionId, Session>,
    by_worker: HashMap<WorkerId, SessionId>,
    worker_dir: PathBuf,
    next: SessionId,
    reconnect: Reconnect,
    pending: Vec<UiEvent>,
}

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
        Manager { sup: Supervisor::new(), sessions: BTreeMap::new(), by_worker: HashMap::new(), worker_dir, next: 1, reconnect, pending: Vec::new() }
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

    pub fn target(&self, s: SessionId) -> Option<Uuid> {
        self.sessions.get(&s).map(|x| x.spec.target)
    }

    pub fn state(&self, s: SessionId) -> Option<&str> {
        self.sessions.get(&s).map(|x| x.state.as_str())
    }

    fn emit_state(&mut self, s: SessionId, state: &str, detail: String) {
        if let Some(x) = self.sessions.get_mut(&s) {
            x.state = state.to_string();
        }
        self.pending.push(UiEvent::State { s, state: state.to_string(), detail });
    }

    /// Neue Sitzung öffnen (startet den Worker sofort).
    pub fn open(&mut self, spec: ConnectSpec) -> SessionId {
        let id = self.next;
        self.next += 1;
        let term = (spec.protocol == Protocol::Ssh)
            .then(|| Terminal::new(spec.request.width as usize, spec.request.height as usize, Osc52Policy::default()));
        self.sessions.insert(id, Session { spec, worker: None, term, state: String::new(), attempts: 0, reconnect_at: None, osc52_pending: Vec::new(), _demo_frame: None, pid: None });
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
            Ok((wid, pid)) => {
                let spec = self.sessions[&id].spec.clone();
                self.by_worker.insert(wid, id);
                let x = self.sessions.get_mut(&id).expect("vorhanden");
                x.worker = Some(wid);
                x.pid = Some(pid);
                let _ = self.sup.send(wid, &ToWorker::Connect(spec.request));
                let _ = self.sup.send(wid, &ToWorker::Credentials(spec.credentials));
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
        term.feed(content.as_bytes());
        let spec = demo_spec(title, Protocol::Ssh);
        self.sessions.insert(id, Session { spec, worker: None, term: Some(term), state: "connected".into(), attempts: 0, reconnect_at: None, osc52_pending: Vec::new(), _demo_frame: None, pid: None });
        self.pending.push(UiEvent::State { s: id, state: "connected".into(), detail: String::new() });
        self.pending.push(UiEvent::Terminal { s: id });
        id
    }

    /// Vorführsitzung mit erzeugtem Bild (ohne Worker).
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
        let path = fb.path().display().to_string();
        let spec = demo_spec(title, Protocol::Rdp);
        self.sessions.insert(id, Session { spec, worker: None, term: None, state: "connected".into(), attempts: 0, reconnect_at: None, osc52_pending: Vec::new(), _demo_frame: Some(fb), pid: None });
        self.pending.push(UiEvent::State { s: id, state: "connected".into(), detail: String::new() });
        self.pending.push(UiEvent::SharedFrame { s: id, path, w, h });
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

    /// Taste aus der Oberfläche.
    pub fn key(&mut self, s: SessionId, qt_key: u32, modifiers: u32, text: &str, down: bool) {
        let Some(sess) = self.sessions.get(&s) else { return };
        if sess.spec.protocol == Protocol::Ssh {
            if down {
                let bytes = crate::keys::terminal_bytes(qt_key, modifiers, text, false);
                if !bytes.is_empty() {
                    self.send(s, &ToWorker::Input(InputEvent::Bytes(bytes)));
                }
            }
            return;
        }
        let ctrl_or_alt = modifiers & (crate::keys::CTRL | crate::keys::ALT) != 0;
        let printable = !text.is_empty() && text.chars().all(|c| !c.is_control());
        if printable && !ctrl_or_alt {
            if down {
                self.send(s, &ToWorker::Input(InputEvent::Text(text.to_string())));
            }
        } else if let Some(k) = crate::keys::keysym(qt_key) {
            self.send(s, &ToWorker::Input(InputEvent::Key { keysym: k, down }));
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
        if let Some(t) = self.sessions.get_mut(&s).and_then(|x| x.term.as_mut()) {
            t.resize(w as usize, h as usize);
        }
        self.send(s, &ToWorker::Resize { width: w, height: h });
    }

    /// Einfügen ins Terminal (mit Klammerung) bzw. Zwischenablage anbieten (RDP/VNC).
    pub fn paste(&mut self, s: SessionId, text: &str) {
        let bytes = self.sessions.get(&s).and_then(|x| x.term.as_ref()).map(|t| t.paste_bytes(text));
        match bytes {
            Some(b) => self.send(s, &ToWorker::Input(InputEvent::Bytes(b))),
            None => self.send(s, &ToWorker::Clipboard(ClipData::Text(text.to_string()))),
        }
    }

    /// Lokale Zwischenablage an eine grafische Sitzung anbieten.
    pub fn offer_clipboard(&mut self, s: SessionId, text: &str) {
        if self.sessions.get(&s).is_some_and(|x| x.term.is_none()) {
            self.send(s, &ToWorker::Clipboard(ClipData::Text(text.to_string())));
        }
    }

    /// Antwort auf eine OSC-52-Leseanfrage.
    pub fn clipboard_read_answer(&mut self, s: SessionId, allow: bool, text: &str) {
        let Some(sess) = self.sessions.get_mut(&s) else { return };
        let pend = std::mem::take(&mut sess.osc52_pending);
        if allow {
            for f in pend {
                let reply = f(text);
                self.send(s, &ToWorker::Input(InputEvent::Bytes(reply.into_bytes())));
            }
        }
    }

    pub fn answer(&mut self, s: SessionId, q: u32, accept: bool) {
        self.send(s, &ToWorker::Answer { question: q, accept });
    }

    pub fn select(&mut self, s: SessionId, start: (usize, usize), end: (usize, usize)) -> Option<String> {
        let t = self.sessions.get_mut(&s)?.term.as_mut()?;
        t.select(start, end);
        t.selection_text()
    }

    pub fn terminal(&self, s: SessionId) -> Option<TermSnapshot> {
        let t = self.sessions.get(&s)?.term.as_ref()?;
        let hex = |c: alacritty_terminal::vte::ansi::Rgb| format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b);
        let mut lines = Vec::with_capacity(t.rows());
        for r in 0..t.rows() {
            let mut runs: Vec<(String, String, String, u8)> = Vec::new();
            for c in 0..t.columns() {
                let cell = t.cell(r, c);
                if cell.spacer {
                    continue;
                }
                let (mut fg, mut bg) = (hex(cell.fg), hex(cell.bg));
                if cell.inverse {
                    std::mem::swap(&mut fg, &mut bg);
                }
                let flags = cell.bold as u8 | (cell.italic as u8) << 1 | (cell.underline as u8) << 2 | (cell.wide as u8) << 3;
                match runs.last_mut() {
                    Some(last) if last.1 == fg && last.2 == bg && last.3 == flags && !cell.wide => last.0.push(cell.ch),
                    _ => runs.push((cell.ch.to_string(), fg, bg, flags)),
                }
            }
            lines.push(runs);
        }
        Some(TermSnapshot { cols: t.columns(), rows: t.rows(), cursor: t.cursor(), cursor_visible: t.cursor_visible(), lines })
    }

    pub fn disconnect(&mut self, s: SessionId) {
        if let Some(w) = self.worker_of(s) {
            self.sup.stop(w);
        }
        if let Some(x) = self.sessions.get_mut(&s) {
            x.reconnect_at = None;
        }
    }

    pub fn reconnect(&mut self, s: SessionId) {
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
        if let Some(w) = self.worker_of(s) {
            self.by_worker.remove(&w);
            self.sup.stop(w);
            self.sup.forget(w);
        }
        self.sessions.remove(&s);
    }

    fn schedule_reconnect(&mut self, s: SessionId) {
        if !self.reconnect.auto {
            return;
        }
        let policy = RestartPolicy { enabled: true, base: Duration::from_secs(1), max: self.reconnect.max, stable_after: Duration::from_secs(30) };
        if let Some(x) = self.sessions.get_mut(&s) {
            let delay = policy.delay(x.attempts);
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
                        // Bildspeicher eines abgestürzten Workers entfernen.
                        if let Some(pid) = x.pid.take() {
                            remove_frame_files(pid);
                        }
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
        std::mem::take(&mut self.pending)
    }

    fn on_message(&mut self, s: SessionId, msg: FromWorker) {
        match msg {
            FromWorker::State(st) => {
                let (name, detail) = match st {
                    SessionState::Connecting => ("connecting", String::new()),
                    SessionState::Authenticating => ("connecting", String::new()),
                    SessionState::Connected => {
                        if let Some(x) = self.sessions.get_mut(&s) {
                            x.attempts = 0;
                        }
                        ("connected", String::new())
                    }
                    SessionState::Disconnected { reason } => ("disconnected", reason),
                    SessionState::Failed { reason } => ("failed", reason),
                };
                self.emit_state(s, name, detail);
                if name == "disconnected" {
                    self.schedule_reconnect(s);
                }
            }
            FromWorker::SharedFrame { path, width, height } => self.pending.push(UiEvent::SharedFrame { s, path, w: width, h: height }),
            FromWorker::Frame(f) => self.pending.push(UiEvent::Frame { s, x: f.x, y: f.y, w: f.w, h: f.h }),
            FromWorker::Output(bytes) => {
                let mut replies = Vec::new();
                if let Some(x) = self.sessions.get_mut(&s) {
                    if let Some(t) = x.term.as_mut() {
                        t.feed(&bytes);
                        for e in t.take_events() {
                            match e {
                                TermEvent::ClipboardStore(text) => self.pending.push(UiEvent::Clipboard { s, text }),
                                TermEvent::ClipboardLoad(f) => {
                                    x.osc52_pending.push(f);
                                    self.pending.push(UiEvent::ClipboardReadRequest { s });
                                }
                                TermEvent::Reply(r) => replies.push(r),
                                TermEvent::Title(title) => self.pending.push(UiEvent::Title { s, title: rj_core::charset::mark_visible(&title, false).into_owned() }),
                                TermEvent::Bell => self.pending.push(UiEvent::Bell { s }),
                            }
                        }
                    }
                }
                for r in replies {
                    self.send(s, &ToWorker::Input(InputEvent::Bytes(r.into_bytes())));
                }
                self.pending.push(UiEvent::Terminal { s });
            }
            FromWorker::Clipboard(ClipData::Text(text)) => self.pending.push(UiEvent::Clipboard { s, text }),
            FromWorker::Clipboard(ClipData::Files(files)) => {
                let names: Vec<String> = files.iter().map(|f| f.name.clone()).collect();
                self.pending.push(UiEvent::Notice { s, text: format!("Dateien in der Zwischenablage empfangen: {}", names.join(", ")) });
            }
            FromWorker::Question { id, text, detail } => self.pending.push(UiEvent::Question { s, q: id, text, detail }),
            FromWorker::Notice(text) => self.pending.push(UiEvent::Notice { s, text }),
            FromWorker::Log(_) | FromWorker::Pong | FromWorker::Hello { .. } | FromWorker::Heartbeat { .. } | FromWorker::Custom(_) => {}
        }
    }

    pub fn shutdown(&mut self) {
        self.sup.shutdown();
    }
}

fn demo_spec(title: &str, protocol: Protocol) -> ConnectSpec {
    ConnectSpec {
        target: Uuid::nil(),
        title: title.into(),
        protocol,
        request: ConnectRequest { host: String::new(), port: 0, width: 80, height: 24, options_json: "{}".into(), connect_timeout_secs: 15, keepalive_secs: 30 },
        credentials: CredentialPayload { username: String::new(), domain: None, password: SecretBytes(Vec::new()), private_key: None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
