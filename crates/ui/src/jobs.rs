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
    rx: Receiver<Continuation>,
}

#[derive(Default)]
pub struct Jobs {
    next: u64,
    list: Vec<Job>,
}

impl Jobs {
    /// Startet eine Aufgabe. `work` läuft im Hintergrund und liefert die
    /// Fortsetzung.
    pub fn spawn(&mut self, label: impl Into<String>, work: impl FnOnce(u64) -> Continuation + Send + 'static) -> u64 {
        if self.next < FIRST_JOB {
            self.next = FIRST_JOB;
        }
        let id = self.next;
        self.next += 1;
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name(format!("rj-job-{id}"))
            .spawn(move || {
                let _ = tx.send(work(id));
            })
            .expect("Faden startbar");
        self.list.push(Job { id, label: label.into(), started: Instant::now(), cancelled: false, rx });
        id
    }

    pub fn is_pending(&self, id: u64) -> bool {
        self.list.iter().any(|j| j.id == id)
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
                true
            }
            None => false,
        }
    }

    pub fn cancel_all(&mut self) {
        for j in &mut self.list {
            j.cancelled = true;
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
        json!(self
            .list
            .iter()
            .filter(|j| !j.cancelled)
            .map(|j| json!({ "id": j.id, "label": j.label, "secs": j.started.elapsed().as_secs() }))
            .collect::<Vec<_>>())
    }
}
