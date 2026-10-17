//! Start, Überwachung und Neustart von Worker-Prozessen.
//!
//! Jede Sitzung läuft in einem eigenen Prozess. Der Supervisor liest je
//! Worker in einem eigenen Faden, erkennt Abstürze (Ende des Kanals,
//! Beendigungsgrund), Hänger (Herzschlag bleibt aus) und zu hohen
//! Speicherverbrauch, räumt beendete Prozesse ab (keine Zombies) und startet
//! auf Wunsch mit wachsendem Rückstau neu.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use rj_ipc::shm::SharedHandle;
use rj_ipc::{FrameReader, FrameWriter, FromWorker, IpcError, ToWorker};

/// Vom Worker übergebene Bildpuffer, bis die Sitzungsverwaltung sie abholt.
type SharedMap = std::sync::Arc<std::sync::Mutex<HashMap<(WorkerId, u64), SharedHandle>>>;

pub type WorkerId = u64;

/// Neustart-Regel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestartPolicy {
    pub enabled: bool,
    /// Erste Wartezeit; verdoppelt sich je Fehlschlag.
    pub base: Duration,
    /// Obergrenze der Wartezeit.
    pub max: Duration,
    /// Nach dieser Laufzeit gilt ein Worker als stabil, der Zähler beginnt neu.
    pub stable_after: Duration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        RestartPolicy {
            enabled: false,
            base: Duration::from_secs(1),
            max: Duration::from_secs(60),
            stable_after: Duration::from_secs(30),
        }
    }
}

impl RestartPolicy {
    /// Wartezeit vor dem `attempt`-ten Neustart (ab 0): 1, 2, 4, … bis `max`.
    pub fn delay(&self, attempt: u32) -> Duration {
        let factor = 1u32.checked_shl(attempt.min(31)).unwrap_or(u32::MAX);
        self.base.saturating_mul(factor).min(self.max)
    }
}

/// Beschreibung eines Worker-Prozesses.
#[derive(Debug, Clone)]
pub struct WorkerSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Bleibt der Herzschlag so lange aus, gilt der Worker als hängend.
    pub heartbeat_timeout: Duration,
    /// Obergrenze für den belegten Arbeitsspeicher (Bytes).
    pub memory_limit: Option<u64>,
    pub restart: RestartPolicy,
}

impl WorkerSpec {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        WorkerSpec {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            heartbeat_timeout: Duration::from_secs(10),
            memory_limit: Some(1024 * 1024 * 1024),
            restart: RestartPolicy::default(),
        }
    }
}

/// Grund für das Ende eines Workers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitReason {
    /// Ordentlich beendet (Code 0).
    Normal,
    /// Rust-Panik (Code 101).
    Panic,
    /// Abbruch (SIGABRT bzw. Windows-Abbruchcode).
    Abort,
    /// Speicherzugriffsfehler (SIGSEGV bzw. Zugriffsverletzung).
    Segfault,
    /// Herzschlag blieb aus; vom Supervisor beendet.
    Hung,
    /// Speichergrenze überschritten; vom Supervisor beendet.
    MemoryLimit { rss: u64 },
    /// Auf Anforderung beendet.
    Stopped,
    /// Anderer Beendigungscode.
    ExitCode(i32),
    /// Anderes Signal.
    Signal(i32),
    /// Protokollfehler im Kanal.
    Protocol(String),
}

impl ExitReason {
    pub fn is_crash(&self) -> bool {
        !matches!(self, ExitReason::Normal | ExitReason::Stopped)
    }

    /// Beschreibung für die Oberfläche.
    pub fn describe(&self) -> String {
        match self {
            ExitReason::Normal => "ordentlich beendet".into(),
            ExitReason::Panic => "Programmfehler (Panik)".into(),
            ExitReason::Abort => "abgebrochen".into(),
            ExitReason::Segfault => "Speicherzugriffsfehler".into(),
            ExitReason::Hung => "reagiert nicht mehr (Herzschlag ausgeblieben)".into(),
            ExitReason::MemoryLimit { rss } => format!("Speichergrenze überschritten ({} MiB)", rss / (1024 * 1024)),
            ExitReason::Stopped => "beendet".into(),
            ExitReason::ExitCode(c) => format!("beendet mit Code {c}"),
            ExitReason::Signal(s) => format!("durch Signal {s} beendet"),
            ExitReason::Protocol(e) => format!("Kanalfehler: {e}"),
        }
    }
}

/// Ereignisse für die Oberfläche.
#[derive(Debug, Clone, PartialEq)]
pub enum SupervisorEvent {
    Started { id: WorkerId, pid: u32 },
    Message { id: WorkerId, msg: FromWorker },
    Exited { id: WorkerId, reason: ExitReason, at: SystemTime },
    RestartScheduled { id: WorkerId, delay: Duration, attempt: u32 },
}

enum Internal {
    Msg(WorkerId, u64, FromWorker),
    Closed(WorkerId, u64, Option<String>),
}

struct Slot {
    spec: WorkerSpec,
    generation: u64,
    child: Option<Child>,
    stdin: Option<FrameWriter<ChildStdin>>,
    started: Instant,
    last_beat: Instant,
    attempts: u32,
    restart_at: Option<Instant>,
    /// Vom Supervisor gesetzter Grund (Hänger, Speicher, Stopp).
    kill_reason: Option<ExitReason>,
}

/// Verwaltet alle Worker.
pub struct Supervisor {
    next_id: WorkerId,
    slots: HashMap<WorkerId, Slot>,
    tx: Sender<Internal>,
    rx: Receiver<Internal>,
    shared: SharedMap,
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl Supervisor {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Supervisor { next_id: 1, slots: HashMap::new(), tx, rx, shared: Default::default() }
    }

    /// Startet einen neuen Worker.
    pub fn spawn(&mut self, spec: WorkerSpec) -> std::io::Result<(WorkerId, u32)> {
        let id = self.next_id;
        self.next_id += 1;
        let now = Instant::now();
        let mut slot = Slot {
            spec,
            generation: 0,
            child: None,
            stdin: None,
            started: now,
            last_beat: now,
            attempts: 0,
            restart_at: None,
            kill_reason: None,
        };
        let pid = self.launch(id, &mut slot)?;
        self.slots.insert(id, slot);
        Ok((id, pid))
    }

    fn launch(&self, id: WorkerId, slot: &mut Slot) -> std::io::Result<u32> {
        let mut cmd = Command::new(&slot.spec.program);
        cmd.args(&slot.spec.args)
            .envs(slot.spec.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        #[cfg(unix)]
        if let Some(limit) = slot.spec.memory_limit {
            use std::os::unix::process::CommandExt;
            // Harte Grenze für den Adressraum als zweite Sicherung; die
            // eigentliche Prüfung erfolgt über den belegten Speicher.
            let hard = limit.saturating_mul(4).saturating_add(512 * 1024 * 1024);
            // SAFETY: setrlimit ist async-signal-sicher und wird vor exec aufgerufen.
            unsafe {
                cmd.pre_exec(move || {
                    let rl = libc::rlimit { rlim_cur: hard as libc::rlim_t, rlim_max: hard as libc::rlim_t };
                    libc::setrlimit(libc::RLIMIT_AS, &rl);
                    Ok(())
                });
            }
        }
        // Zweiter Weg nur für Bildpuffer (Deskriptoren per SCM_RIGHTS).
        #[cfg(unix)]
        let frame_sock = {
            use std::os::fd::AsRawFd;
            use std::os::unix::process::CommandExt;
            let (parent, child_end) = rj_ipc::fdpass::pair()?;
            let raw = child_end.as_raw_fd();
            cmd.env(rj_ipc::fdpass::FRAME_FD_ENV, raw.to_string());
            // SAFETY: fcntl ist async-signal-sicher; nur dieses Ende wird vererbt.
            unsafe {
                cmd.pre_exec(move || {
                    libc::fcntl(raw, libc::F_SETFD, 0);
                    Ok(())
                });
            }
            (parent, child_end)
        };
        let mut child = cmd.spawn()?;
        #[cfg(unix)]
        let frame_sock = {
            drop(frame_sock.1);
            frame_sock.0
        };
        let shared = self.shared.clone();
        let pid = child.id();
        let stdout = child.stdout.take().expect("stdout angefordert");
        let stdin = child.stdin.take().expect("stdin angefordert");
        slot.generation += 1;
        let generation = slot.generation;
        let tx = self.tx.clone();
        thread::Builder::new().name(format!("worker-{id}")).spawn(move || {
            let mut r = FrameReader::new(stdout);
            loop {
                match r.recv::<FromWorker>() {
                    Ok((_, msg)) => {
                        // Der Deskriptor wird vor der Nachricht gesendet.
                        #[cfg(unix)]
                        if let FromWorker::SharedFrame { handle, .. } = &msg {
                            let _ = frame_sock.set_read_timeout(Some(Duration::from_secs(2)));
                            while let Ok((token, fd)) = rj_ipc::fdpass::recv_fd(&frame_sock) {
                                if token == *handle {
                                    let mut m = shared.lock().unwrap_or_else(|e| e.into_inner());
                                    // Höchstens wenige nicht abgeholte Puffer je Worker.
                                    let mut old: Vec<u64> = m.keys().filter(|k| k.0 == id).map(|k| k.1).collect();
                                    old.sort_unstable();
                                    for t in old.iter().rev().skip(3) {
                                        m.remove(&(id, *t));
                                    }
                                    m.insert((id, token), fd);
                                    break;
                                }
                            }
                        }
                        #[cfg(not(unix))]
                        let _ = &shared;
                        if tx.send(Internal::Msg(id, generation, msg)).is_err() {
                            return;
                        }
                    }
                    Err(IpcError::Closed) => {
                        let _ = tx.send(Internal::Closed(id, generation, None));
                        return;
                    }
                    Err(e) => {
                        let _ = tx.send(Internal::Closed(id, generation, Some(e.to_string())));
                        return;
                    }
                }
            }
        })?;
        let now = Instant::now();
        slot.child = Some(child);
        slot.stdin = Some(FrameWriter::new(stdin));
        slot.started = now;
        slot.last_beat = now;
        slot.kill_reason = None;
        slot.restart_at = None;
        Ok(pid)
    }

    /// Sendet eine Nachricht an einen laufenden Worker.
    pub fn send(&mut self, id: WorkerId, msg: &ToWorker) -> Result<(), IpcError> {
        let slot = self.slots.get_mut(&id).ok_or(IpcError::Closed)?;
        slot.stdin.as_mut().ok_or(IpcError::Closed)?.send(0, msg)
    }

    /// Holt den Bildpuffer zu `SharedFrame { handle }` eines Workers ab.
    /// Unix: der über den Socket übergebene Deskriptor; Windows: die aus dem
    /// Worker duplizierte Kennung.
    pub fn take_shared(&mut self, id: WorkerId, handle: u64) -> Option<SharedHandle> {
        #[cfg(unix)]
        {
            self.shared.lock().unwrap_or_else(|e| e.into_inner()).remove(&(id, handle))
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            let child = self.slots.get(&id)?.child.as_ref()?;
            rj_ipc::shm::duplicate_from(child.as_raw_handle() as _, handle).ok()
        }
    }

    pub fn is_running(&self, id: WorkerId) -> bool {
        self.slots.get(&id).is_some_and(|s| s.child.is_some())
    }

    pub fn pid(&self, id: WorkerId) -> Option<u32> {
        self.slots.get(&id)?.child.as_ref().map(Child::id)
    }

    pub fn running_count(&self) -> usize {
        self.slots.values().filter(|s| s.child.is_some()).count()
    }

    /// Beendet einen Worker ohne Neustart.
    pub fn stop(&mut self, id: WorkerId) {
        if let Some(slot) = self.slots.get_mut(&id) {
            slot.spec.restart.enabled = false;
            if let Some(w) = slot.stdin.as_mut() {
                let _ = w.send(0, &ToWorker::Shutdown);
            }
            slot.kill_reason = Some(ExitReason::Stopped);
            slot.stdin = None;
            if let Some(c) = slot.child.as_mut() {
                // Kurze Schonfrist für ein ordentliches Ende.
                let deadline = Instant::now() + Duration::from_millis(500);
                while Instant::now() < deadline {
                    if let Ok(Some(_)) = c.try_wait() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                let _ = c.kill();
            }
        }
    }

    /// Entfernt einen beendeten Worker aus der Verwaltung.
    pub fn forget(&mut self, id: WorkerId) {
        self.shared.lock().unwrap_or_else(|e| e.into_inner()).retain(|k, _| k.0 != id);
        if let Some(mut s) = self.slots.remove(&id) {
            if let Some(mut c) = s.child.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }

    /// Verarbeitet eingegangene Nachrichten und prüft alle Worker. Wartet
    /// höchstens `wait` auf die erste Nachricht.
    pub fn tick(&mut self, wait: Duration) -> Vec<SupervisorEvent> {
        let mut events = Vec::new();
        let mut first = true;
        loop {
            let item = if first {
                first = false;
                self.rx.recv_timeout(wait).ok()
            } else {
                self.rx.try_recv().ok()
            };
            let Some(item) = item else { break };
            match item {
                Internal::Msg(id, generation, msg) => {
                    let Some(slot) = self.slots.get_mut(&id) else { continue };
                    if slot.generation != generation {
                        continue;
                    }
                    slot.last_beat = Instant::now();
                    match msg {
                        FromWorker::Heartbeat { .. } => {}
                        FromWorker::Hello { pid, .. } => events.push(SupervisorEvent::Started { id, pid }),
                        other => events.push(SupervisorEvent::Message { id, msg: other }),
                    }
                }
                Internal::Closed(id, generation, err) => {
                    if self.slots.get(&id).is_some_and(|s| s.generation == generation) {
                        self.reap(id, err, &mut events);
                    }
                }
            }
        }
        self.check_health(&mut events);
        self.restart_due(&mut events);
        events
    }

    fn reap(&mut self, id: WorkerId, protocol_error: Option<String>, events: &mut Vec<SupervisorEvent>) {
        let Some(slot) = self.slots.get_mut(&id) else { return };
        let Some(mut child) = slot.child.take() else { return };
        slot.stdin = None;
        // Der Kanal ist zu; der Prozess endet gleich oder wird beendet.
        let status = match wait_timeout(&mut child, Duration::from_secs(2)) {
            Some(s) => s,
            None => {
                let _ = child.kill();
                child.wait().unwrap_or_else(exit_status_unknown)
            }
        };
        let reason = slot
            .kill_reason
            .take()
            .or_else(|| protocol_error.map(ExitReason::Protocol).filter(|_| status.success()))
            .unwrap_or_else(|| classify(&status));
        let ran = slot.started.elapsed();
        events.push(SupervisorEvent::Exited { id, reason: reason.clone(), at: SystemTime::now() });
        if slot.spec.restart.enabled && reason.is_crash() {
            if ran >= slot.spec.restart.stable_after {
                slot.attempts = 0;
            }
            let delay = slot.spec.restart.delay(slot.attempts);
            slot.attempts += 1;
            slot.restart_at = Some(Instant::now() + delay);
            events.push(SupervisorEvent::RestartScheduled { id, delay, attempt: slot.attempts });
        }
    }

    fn check_health(&mut self, events: &mut Vec<SupervisorEvent>) {
        let mut to_reap = Vec::new();
        for (id, slot) in &mut self.slots {
            let Some(child) = slot.child.as_mut() else { continue };
            if slot.kill_reason.is_some() {
                continue;
            }
            // Ein Prozess, der gerade abstürzt und sein Speicherabbild schreibt (z. B. über apport),
            // braucht einige Sekunden bis zum Ende; er ist nicht hängend und wird nach seinem Signal eingeordnet.
            let late = slot.last_beat.elapsed();
            if late > slot.spec.heartbeat_timeout && late < slot.spec.heartbeat_timeout + CORE_DUMP_GRACE && is_dumping_core(child.id()) {
                continue;
            }
            if late > slot.spec.heartbeat_timeout {
                slot.kill_reason = Some(ExitReason::Hung);
                let _ = child.kill();
                to_reap.push(*id);
                continue;
            }
            if let (Some(limit), Some(rss)) = (slot.spec.memory_limit, resident_bytes(child.id())) {
                if rss > limit {
                    slot.kill_reason = Some(ExitReason::MemoryLimit { rss });
                    let _ = child.kill();
                    to_reap.push(*id);
                }
            }
        }
        for id in to_reap {
            self.reap(id, None, events);
        }
    }

    fn restart_due(&mut self, events: &mut Vec<SupervisorEvent>) {
        let due: Vec<WorkerId> = self
            .slots
            .iter()
            .filter(|(_, s)| s.child.is_none() && s.restart_at.is_some_and(|t| t <= Instant::now()))
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            let mut slot = self.slots.remove(&id).expect("vorhanden");
            match self.launch(id, &mut slot) {
                Ok(_) => {}
                Err(e) => {
                    events.push(SupervisorEvent::Exited {
                        id,
                        reason: ExitReason::Protocol(format!("Neustart fehlgeschlagen: {e}")),
                        at: SystemTime::now(),
                    });
                    let delay = slot.spec.restart.delay(slot.attempts);
                    slot.attempts += 1;
                    slot.restart_at = Some(Instant::now() + delay);
                }
            }
            self.slots.insert(id, slot);
        }
    }

    /// Beendet alle Worker und räumt sie ab.
    pub fn shutdown(&mut self) {
        let ids: Vec<_> = self.slots.keys().copied().collect();
        for id in &ids {
            if let Some(slot) = self.slots.get_mut(id) {
                slot.spec.restart.enabled = false;
                if let Some(w) = slot.stdin.as_mut() {
                    let _ = w.send(0, &ToWorker::Shutdown);
                }
                slot.stdin = None;
            }
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        for id in ids {
            if let Some(mut slot) = self.slots.remove(&id) {
                if let Some(mut c) = slot.child.take() {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if wait_timeout(&mut c, left).is_none() {
                        let _ = c.kill();
                        let _ = c.wait();
                    }
                }
            }
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        for slot in self.slots.values_mut() {
            if let Some(mut c) = slot.child.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }
}

fn wait_timeout(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return Some(s),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            _ => return None,
        }
    }
}

#[cfg(unix)]
fn exit_status_unknown(_: std::io::Error) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus::from_raw(0x7f00)
}

#[cfg(windows)]
fn exit_status_unknown(_: std::io::Error) -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    ExitStatus::from_raw(1)
}

/// Ordnet einen Beendigungsstatus einem Grund zu.
pub fn classify(status: &ExitStatus) -> ExitReason {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return match sig {
                libc::SIGABRT => ExitReason::Abort,
                libc::SIGSEGV | libc::SIGBUS => ExitReason::Segfault,
                s => ExitReason::Signal(s),
            };
        }
    }
    match status.code() {
        Some(0) => ExitReason::Normal,
        Some(101) => ExitReason::Panic,
        // Windows: STATUS_ACCESS_VIOLATION, Abbruch über abort() bzw. Fail-Fast.
        Some(c) if c as u32 == 0xC000_0005 => ExitReason::Segfault,
        Some(3) if cfg!(windows) => ExitReason::Abort,
        Some(c) if c as u32 == 0xC000_0409 => ExitReason::Abort,
        Some(c) => ExitReason::ExitCode(c),
        None => ExitReason::Signal(0),
    }
}

/// Zusätzliche Frist für einen Prozess, der beim Absturz sein Speicherabbild schreibt.
const CORE_DUMP_GRACE: Duration = Duration::from_secs(15);

/// Schreibt der Prozess gerade ein Speicherabbild (Absturz)? Linux: Prozessmarke `PF_DUMPCORE` in
/// `/proc/<pid>/task/<tid>/stat`; sie steht an dem Faden, der abgestürzt ist, nicht unbedingt am Hauptfaden.
#[cfg(target_os = "linux")]
pub fn is_dumping_core(pid: u32) -> bool {
    const PF_DUMPCORE: u64 = 0x200;
    let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) else { return false };
    tasks.flatten().take(512).any(|t| {
        let Ok(stat) = std::fs::read_to_string(t.path().join("stat")) else { return false };
        // Nach dem Namen in Klammern: Zustand, Elternprozess, Gruppe, Sitzung, Terminal, Vordergrundgruppe, Marken.
        let Some((_, rest)) = stat.rsplit_once(')') else { return false };
        rest.split_whitespace().nth(6).and_then(|f| f.parse::<u64>().ok()).is_some_and(|f| f & PF_DUMPCORE != 0)
    })
}

#[cfg(not(target_os = "linux"))]
pub fn is_dumping_core(_pid: u32) -> bool {
    false
}

/// Belegter Arbeitsspeicher eines Prozesses in Bytes.
#[cfg(target_os = "linux")]
pub fn resident_bytes(pid: u32) -> Option<u64> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
    let pages: u64 = s.split_whitespace().nth(1)?.parse().ok()?;
    // SAFETY: sysconf ohne Nebenwirkungen.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    Some(pages * page.max(4096) as u64)
}

#[cfg(not(target_os = "linux"))]
pub fn resident_bytes(_pid: u32) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn running_process_is_not_dumping_core() {
        assert!(!is_dumping_core(std::process::id()));
        assert!(!is_dumping_core(u32::MAX), "unbekannter Prozess");
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let p = RestartPolicy::default();
        let secs: Vec<u64> = (0..9).map(|a| p.delay(a).as_secs()).collect();
        assert_eq!(secs, [1, 2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(p.delay(200), Duration::from_secs(60));
    }
}
