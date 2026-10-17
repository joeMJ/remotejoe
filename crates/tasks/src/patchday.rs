//! Patchday: Aufgaben auf vielen Zielen, parallel begrenzt, mit Zustand,
//! Fortschritt, Protokoll und Abbruch je Ziel.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use serde::Serialize;
use uuid::Uuid;

/// Zustand eines Ziels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "detail")]
pub enum JobState {
    Waiting,
    Searching,
    Downloading,
    Installing,
    RebootRequired,
    Error(String),
    Cancelled,
    Done,
}

impl JobState {
    pub fn finished(&self) -> bool {
        matches!(self, JobState::RebootRequired | JobState::Error(_) | JobState::Cancelled | JobState::Done)
    }

    pub fn label(&self) -> &'static str {
        match self {
            JobState::Waiting => "wartet",
            JobState::Searching => "sucht",
            JobState::Downloading => "lädt",
            JobState::Installing => "installiert",
            JobState::RebootRequired => "Neustart nötig",
            JobState::Error(_) => "Fehler",
            JobState::Cancelled => "abgebrochen",
            JobState::Done => "fertig",
        }
    }
}

/// Zeile der Patchday-Tabelle.
#[derive(Debug, Clone, Serialize)]
pub struct JobRow {
    pub target: Uuid,
    pub name: String,
    #[serde(flatten)]
    pub state: JobState,
    pub label: &'static str,
    /// Anzahl gefundener Aktualisierungen.
    pub count: Option<u32>,
    /// Fortschritt 0–100.
    pub percent: u8,
    pub log: Vec<String>,
}

/// Rückmeldungen einer laufenden Aufgabe.
#[derive(Clone)]
pub struct JobHandle {
    rows: Arc<Mutex<Vec<JobRow>>>,
    index: usize,
    cancel: Arc<AtomicBool>,
}

impl JobHandle {
    fn with<R>(&self, f: impl FnOnce(&mut JobRow) -> R) -> R {
        let mut rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut rows[self.index])
    }

    pub fn set_state(&self, s: JobState) {
        self.with(|r| {
            r.label = s.label();
            r.state = s;
        });
    }

    pub fn set_count(&self, n: u32) {
        self.with(|r| r.count = Some(n));
    }

    pub fn set_percent(&self, p: u8) {
        self.with(|r| r.percent = p.min(100));
    }

    /// Protokollzeile; Inhalte von Kennwörtern gehören nie hierher.
    pub fn log(&self, line: &str) {
        self.with(|r| r.log.push(line.to_string()));
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Ausführung einer Aufgabe auf einem Ziel (z. B. apt über SSH,
/// Windows-Update über PowerShell). Endzustand setzt der Ablauf, wenn die
/// Aufgabe ihn nicht selbst setzt.
pub trait Executor: Send + Sync {
    fn run(&self, target: Uuid, job: &JobHandle) -> Result<(), String>;
}

struct Queue {
    pending: VecDeque<usize>,
}

/// Ein Patchday-Lauf.
pub struct Patchday {
    rows: Arc<Mutex<Vec<JobRow>>>,
    cancels: Vec<Arc<AtomicBool>>,
    running: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
    threads: Vec<std::thread::JoinHandle<()>>,
    done: Arc<(Mutex<usize>, Condvar)>,
}

impl Patchday {
    /// Startet den Lauf mit höchstens `parallel` gleichzeitigen Zielen.
    pub fn start(targets: Vec<(Uuid, String)>, parallel: usize, exec: Arc<dyn Executor>) -> Patchday {
        let n = targets.len();
        let rows = Arc::new(Mutex::new(
            targets
                .iter()
                .map(|(id, name)| JobRow { target: *id, name: name.clone(), state: JobState::Waiting, label: JobState::Waiting.label(), count: None, percent: 0, log: Vec::new() })
                .collect::<Vec<_>>(),
        ));
        let cancels: Vec<Arc<AtomicBool>> = (0..n).map(|_| Arc::new(AtomicBool::new(false))).collect();
        let queue = Arc::new(Mutex::new(Queue { pending: (0..n).collect() }));
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let done = Arc::new((Mutex::new(0usize), Condvar::new()));
        let mut threads = Vec::new();
        for _ in 0..parallel.clamp(1, 64).min(n.max(1)) {
            let (queue, rows, cancels, exec, running, peak, done) = (queue.clone(), rows.clone(), cancels.clone(), exec.clone(), running.clone(), peak.clone(), done.clone());
            let ids: Vec<Uuid> = targets.iter().map(|t| t.0).collect();
            threads.push(std::thread::spawn(move || loop {
                let Some(i) = queue.lock().unwrap_or_else(|e| e.into_inner()).pending.pop_front() else { break };
                let job = JobHandle { rows: rows.clone(), index: i, cancel: cancels[i].clone() };
                if job.cancelled() {
                    job.set_state(JobState::Cancelled);
                } else {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    job.set_state(JobState::Searching);
                    let r = exec.run(ids[i], &job);
                    running.fetch_sub(1, Ordering::SeqCst);
                    let state = job.with(|r| r.state.clone());
                    match r {
                        _ if job.cancelled() => job.set_state(JobState::Cancelled),
                        Err(e) => {
                            job.log(&format!("Fehler: {e}"));
                            job.set_state(JobState::Error(e));
                        }
                        Ok(()) if !state.finished() => {
                            job.set_percent(100);
                            job.set_state(JobState::Done);
                        }
                        Ok(()) => job.set_percent(100),
                    }
                }
                let (m, cv) = &*done;
                *m.lock().unwrap_or_else(|e| e.into_inner()) += 1;
                cv.notify_all();
            }));
        }
        Patchday { rows, cancels, running, peak, threads, done }
    }

    pub fn rows(&self) -> Vec<JobRow> {
        self.rows.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Abbruch je Ziel: wartende Ziele starten nicht, laufende Aufgaben
    /// prüfen den Abbruch selbst.
    pub fn cancel(&self, target: Uuid) {
        let rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = rows.iter().position(|r| r.target == target) {
            self.cancels[i].store(true, Ordering::Relaxed);
        }
    }

    /// Höchste gleichzeitige Anzahl laufender Ziele.
    pub fn peak_parallel(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }

    pub fn running(&self) -> usize {
        self.running.load(Ordering::SeqCst)
    }

    pub fn finished(&self) -> bool {
        *self.done.0.lock().unwrap_or_else(|e| e.into_inner()) >= self.cancels.len()
    }

    /// Wartet auf das Ende aller Ziele.
    pub fn wait(mut self) -> Vec<JobRow> {
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        self.rows()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Attrappe: spielt aufgezeichnete Ereignisse des Windows-Skripts bzw.
    /// von apt ab.
    struct Fake {
        fail: Uuid,
        reboot: Uuid,
    }

    impl Executor for Fake {
        fn run(&self, target: Uuid, job: &JobHandle) -> Result<(), String> {
            let lines = crate::winupdate::script(false); // nur zur Sicherheit erzeugbar
            assert!(!lines.is_empty());
            let recorded = [
                r#"{"type":"found","title":"Kumulatives Update (KB5099999)","kb":"KB5099999","size":1,"reboot":true}"#,
                r#"{"type":"progress","phase":"downloading","index":1,"total":1}"#,
                r#"{"type":"progress","phase":"installing","index":1,"total":1}"#,
            ];
            let mut found = 0;
            for l in recorded {
                if job.cancelled() {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(20));
                match crate::winupdate::parse_line(l) {
                    Some(crate::winupdate::WuEvent::Found { title, .. }) => {
                        found += 1;
                        job.set_count(found);
                        job.log(&format!("gefunden: {title}"));
                    }
                    Some(crate::winupdate::WuEvent::Progress { phase, index, total, .. }) => {
                        job.set_state(if phase == "downloading" { JobState::Downloading } else { JobState::Installing });
                        job.set_percent((index * 100 / total.max(1)) as u8);
                    }
                    _ => {}
                }
            }
            if target == self.fail {
                return Err("Zugriff verweigert (0x80070005)".into());
            }
            if target == self.reboot {
                job.set_state(JobState::RebootRequired);
            }
            Ok(())
        }
    }

    #[test]
    fn twenty_five_targets_limited_parallel_with_cancel() {
        let targets: Vec<(Uuid, String)> = (0..25).map(|i| (Uuid::new_v4(), format!("srv-{i:02}"))).collect();
        let (fail, reboot, cancel) = (targets[3].0, targets[7].0, targets[24].0);
        let p = Patchday::start(targets.clone(), 4, Arc::new(Fake { fail, reboot }));
        p.cancel(cancel);
        // Während des Laufs: höchstens 4 gleichzeitig.
        let mut seen_running = false;
        for _ in 0..20 {
            assert!(p.running() <= 4);
            seen_running |= p.running() > 0;
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(seen_running);
        let peak = p.peak_parallel();
        let rows = p.wait();
        assert!(peak <= 4 && peak >= 2, "Spitze {peak}");
        assert_eq!(rows.len(), 25);
        let by = |id: Uuid| rows.iter().find(|r| r.target == id).unwrap().clone();
        assert!(matches!(by(fail).state, JobState::Error(ref e) if e.contains("0x80070005")));
        assert_eq!(by(reboot).state, JobState::RebootRequired);
        assert_eq!(by(cancel).state, JobState::Cancelled);
        let done = rows.iter().filter(|r| r.state == JobState::Done).count();
        assert_eq!(done, 22);
        assert!(rows.iter().filter(|r| r.state == JobState::Done).all(|r| r.count == Some(1) && r.percent == 100));
        let json = serde_json::to_string(&rows).unwrap();
        assert!(json.contains("\"label\":\"Neustart nötig\""));
    }
}
