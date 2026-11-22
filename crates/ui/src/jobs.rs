//! Hintergrundaufgaben (Tunnelaufbau, Abrufe bei Hypervisoren), damit die
//! Oberfläche nie auf das Netz wartet.
//!
//! Eine Aufgabe läuft in einem eigenen Faden und liefert eine Fortsetzung,
//! die im Faden der Oberfläche auf die App angewandt wird. Wurde die Aufgabe
//! abgebrochen, erfährt die Fortsetzung das und räumt auf (z. B. Tunnel
//! freigeben), statt ein Ergebnis zu zeigen.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;

use serde_json::json;

use crate::app::App;
use crate::sessions::UiEvent;

/// Fortsetzung im Faden der Oberfläche; `true` = abgebrochen.
pub type Continuation = Box<dyn FnOnce(&mut App, bool) -> Vec<UiEvent> + Send>;

/// Kennungen von Aufgaben liegen getrennt von Sitzungsnummern.
pub const FIRST_JOB: u64 = 1 << 40;

struct Job {
    id: u64,
    label: String,
    started: Instant,
    cancelled: bool,
    /// Abbruchmarke für die Arbeit im Hintergrund.
    flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    rx: Receiver<Continuation>,
}

/// Auftrag einer Erweiterung: läuft in deren Faden; hier nur Eintrag, Abbruchmarke und Anzeige.
struct ExtJob {
    id: u64,
    label: String,
    started: Instant,
    cancelled: bool,
}

/// Aufträge der Erweiterungen (nebenläufig anmeldbar).
#[derive(Default)]
pub struct ExternalJobs {
    next: u64,
    list: Vec<ExtJob>,
}

impl ExternalJobs {
    pub fn begin(&mut self, label: &str) -> u64 {
        if self.next < FIRST_JOB + (1 << 32) {
            self.next = FIRST_JOB + (1 << 32);
        }
        let id = self.next;
        self.next += 1;
        self.list.push(ExtJob { id, label: label.to_string(), started: Instant::now(), cancelled: false });
        id
    }

    pub fn end(&mut self, id: u64) {
        self.list.retain(|j| j.id != id);
    }

    pub fn cancelled(&self, id: u64) -> bool {
        self.list.iter().find(|j| j.id == id).is_none_or(|j| j.cancelled)
    }

    fn cancel(&mut self, id: u64) -> bool {
        match self.list.iter_mut().find(|j| j.id == id) {
            Some(j) => {
                j.cancelled = true;
                true
            }
            None => false,
        }
    }
}

#[derive(Default)]
pub struct Jobs {
    next: u64,
    list: Vec<Job>,
    external: std::sync::Arc<std::sync::Mutex<ExternalJobs>>,
}

impl Jobs {
    /// Startet eine Aufgabe. `work` läuft im Hintergrund und liefert die
    /// Fortsetzung.
    pub fn spawn(&mut self, label: impl Into<String>, work: impl FnOnce(u64) -> Continuation + Send + 'static) -> u64 {
        self.spawn_cancellable(label, move |id, _| work(id))
    }

    /// Wie [`spawn`](Self::spawn); die Arbeit erhält die Abbruchmarke, die
    /// beim Abbrechen gesetzt wird, und kann ihr Warten sofort beenden.
    pub fn spawn_cancellable(
        &mut self,
        label: impl Into<String>,
        work: impl FnOnce(u64, std::sync::Arc<std::sync::atomic::AtomicBool>) -> Continuation + Send + 'static,
    ) -> u64 {
        if self.next < FIRST_JOB {
            self.next = FIRST_JOB;
        }
        let id = self.next;
        self.next += 1;
        let (tx, rx) = mpsc::channel();
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let f = flag.clone();
        std::thread::Builder::new()
            .name(format!("rj-job-{id}"))
            .spawn(move || {
                let _ = tx.send(work(id, f));
            })
            .expect("Faden startbar");
        self.list.push(Job { id, label: label.into(), started: Instant::now(), cancelled: false, flag, rx });
        id
    }

    /// Aufträge der Erweiterungen (geteilt mit deren Fäden).
    pub fn external(&self) -> std::sync::Arc<std::sync::Mutex<ExternalJobs>> {
        self.external.clone()
    }

    fn ext(&self) -> std::sync::MutexGuard<'_, ExternalJobs> {
        self.external.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_pending(&self, id: u64) -> bool {
        self.list.iter().any(|j| j.id == id) || self.ext().list.iter().any(|j| j.id == id)
    }

    pub fn pending_ids(&self) -> Vec<u64> {
        self.list.iter().map(|j| j.id).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Abbrechen: Die Oberfläche wartet nicht mehr; die Fortsetzung räumt
    /// auf, sobald der Hintergrund fertig ist.
    pub fn cancel(&mut self, id: u64) -> bool {
        match self.list.iter_mut().find(|j| j.id == id) {
            Some(j) => {
                j.cancelled = true;
                j.flag.store(true, std::sync::atomic::Ordering::Relaxed);
                true
            }
            None => self.ext().cancel(id),
        }
    }

    pub fn cancel_all(&mut self) {
        for j in &mut self.ext().list {
            j.cancelled = true;
        }
        for j in &mut self.list {
            j.cancelled = true;
            j.flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Fertige Aufgaben: (Kennung, abgebrochen, Fortsetzung).
    pub fn take_finished(&mut self) -> Vec<(u64, bool, Option<Continuation>)> {
        let mut done = Vec::new();
        self.list.retain(|j| match j.rx.try_recv() {
            Ok(c) => {
                done.push((j.id, j.cancelled, Some(c)));
                false
            }
            Err(TryRecvError::Disconnected) => {
                done.push((j.id, j.cancelled, None));
                false
            }
            Err(TryRecvError::Empty) => true,
        });
        done
    }

    /// Laufende, nicht abgebrochene Aufgaben für die Anzeige.
    pub fn json(&self) -> serde_json::Value {
        let mut v: Vec<serde_json::Value> = self.list.iter().filter(|j| !j.cancelled).map(|j| json!({ "id": j.id, "label": j.label, "secs": j.started.elapsed().as_secs() })).collect();
        v.extend(self.ext().list.iter().filter(|j| !j.cancelled).map(|j| json!({ "id": j.id, "label": j.label, "secs": j.started.elapsed().as_secs() })));
        json!(v)
    }
}
