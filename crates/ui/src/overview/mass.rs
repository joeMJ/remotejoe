//! Masseninventar: „Inventar abfragen“ für alle Ziele eines Ordners bzw. Zweigs, mehrere gleichzeitig.
//! Ziele mit abgeschaltetem Inventar werden übersprungen; Ergebnisse landen wie bei der Einzelabfrage in den
//! Inventardateien (abweichende Werte als Vorschlag, von Hand gesetzte bleiben).

use std::collections::VecDeque;
use std::net::ToSocketAddrs;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rj_core::inherit::{resolve_credential, resolve_inventory, CredentialResolution};
use rj_core::model::Node;
use rj_core::{edit, Protocol};
use serde_json::{json, Value};
use uuid::Uuid;

use super::inventory::{device_kind, now, Store};
use super::remote;
use crate::app::App;
use crate::patch::JobSpec;

/// Höchstens so viele Abfragen gleichzeitig (Netz an den Standorten nicht belasten).
pub const PARALLEL: usize = 8;

#[derive(Debug, Clone)]
pub struct MassRow {
    pub target: Uuid,
    pub name: String,
    pub path: String,
    pub host: String,
    /// wartet, läuft, erkannt, übersprungen, fehler, abgebrochen
    pub state: &'static str,
    /// Erkanntes System bzw. Fehlermeldung bzw. Grund fürs Überspringen.
    pub detail: String,
    pub aruba: bool,
}

impl MassRow {
    fn json(&self) -> Value {
        json!({ "id": self.target, "name": self.name, "path": self.path, "host": self.host, "state": self.state, "detail": self.detail, "aruba": self.aruba })
    }
}

#[derive(Default)]
pub struct MassState {
    pub rows: Arc<Mutex<Vec<MassRow>>>,
    pub done: Arc<AtomicUsize>,
    pub job: Option<u64>,
    pub title: String,
}

/// Eine vorbereitete Abfrage (Zugangsdaten schon aus der KDBX geholt).
struct Work {
    row: usize,
    target: Uuid,
    spec: JobSpec,
    ssh_port: u16,
    /// Windows-Ziel (RDP/PowerShell): SSH mit PowerShell bevorzugen; ohne offenen SSH-Port unter Windows WinRM.
    windows: bool,
}

impl App {
    /// Startet das Masseninventar für die gewählten Ordner und Ziele (`ids`); nur ein Lauf zur Zeit.
    pub fn mass_start(&mut self, c: &Value) -> Result<Value, String> {
        if self.mass.job.is_some_and(|j| self.jobs.is_pending(j)) {
            return Err("Ein Masseninventar läuft bereits".into());
        }
        if self.demo {
            return Err("In der Vorführung nicht verfügbar".into());
        }
        let ids: Vec<Uuid> = c.get("ids").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().and_then(|s| Uuid::parse_str(s).ok())).collect()).unwrap_or_default();
        let mut targets = Vec::new();
        for id in ids {
            match edit::node(&self.index, id) {
                Some(Node::Target(_)) => targets.push(id),
                Some(Node::Folder(_)) => targets.extend(edit::targets_below(&self.index, id)),
                None => {}
            }
        }
        let mut seen = std::collections::HashSet::new();
        targets.retain(|t| seen.insert(*t));
        if targets.is_empty() {
            return Err("Keine Ziele in der Auswahl".into());
        }
        let mut rows = Vec::new();
        let mut queue = VecDeque::new();
        for id in targets {
            let Some(t) = self.target_ref(id).cloned() else { continue };
            let path = self.index.path_to(id).unwrap_or_default().iter().rev().skip(1).rev().map(|n| rj_core::charset::mark_visible(n.name(), false).into_owned()).collect::<Vec<_>>().join(" › ");
            let mut row = MassRow { target: id, name: rj_core::charset::mark_visible(&t.name, false).into_owned(), path, host: t.host.clone(), state: "wartet", detail: String::new(), aruba: false };
            let skip = |row: &mut MassRow, why: &str| {
                row.state = "übersprungen";
                row.detail = why.to_string();
            };
            match resolve_inventory(&self.index, id) {
                Ok((false, _)) => skip(&mut row, "Inventar ist für dieses Ziel abgeschaltet"),
                Err(_) => skip(&mut row, "Ziel nicht gefunden"),
                Ok((true, _)) if matches!(t.protocol, Protocol::Proxmox | Protocol::Hyperv) => skip(&mut row, "Hypervisor: Inventar kommt über die Gäste"),
                Ok((true, _)) if t.host.trim().is_empty() => skip(&mut row, "Keine Adresse"),
                Ok((true, _)) => match resolve_credential(&self.index, id) {
                    Ok(CredentialResolution::Found(r)) => {
                        let windows = matches!(t.protocol, Protocol::Rdp | Protocol::Psrp);
                        match self.credential_payload(&r, if windows { t.protocol } else { Protocol::Ssh }) {
                            Ok(creds) => {
                                let spec = JobSpec {
                                    protocol: t.protocol,
                                    host: t.host.clone(),
                                    port: t.effective_port(),
                                    credentials: creds,
                                    transport: rj_core::inherit::resolve_psrp_transport(&self.index, id).ok().flatten(),
                                    legacy: self.target_legacy_of(id),
                                };
                                let ssh_port = if t.protocol == Protocol::Ssh { t.effective_port() } else { 22 };
                                queue.push_back(Work { row: rows.len(), target: id, spec, ssh_port, windows });
                            }
                            Err(e) => skip(&mut row, &e),
                        }
                    }
                    Ok(CredentialResolution::Ambiguous(_)) => skip(&mut row, "Mehrere Zugangsdaten kommen in Frage"),
                    _ => skip(&mut row, "Keine Zugangsdaten hinterlegt"),
                },
            }
            rows.push(row);
        }
        let total = rows.len();
        let pending = queue.len();
        let shared = Arc::new(Mutex::new(rows));
        let done = Arc::new(AtomicUsize::new(total - pending));
        self.mass.rows = shared.clone();
        self.mass.done = done.clone();
        self.mass.title = c.get("title").and_then(Value::as_str).unwrap_or_default().to_string();
        let known = self.known_hosts_ref();
        let store = self.overview.store.clone();
        let job = self.jobs.spawn_cancellable(format!("Masseninventar ({pending} Ziele)"), move |job, cancel| {
            run(queue, shared.clone(), done, known, store, cancel.clone());
            Box::new(move |app: &mut App, _cancelled: bool| {
                // Gerätearten nachführen (Symbole im Baum).
                for r in shared.lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|r| r.state == "erkannt") {
                    let inv = app.overview.store.load(r.target);
                    match device_kind(&inv) {
                        Some(k) => app.overview.devices.insert(r.target, k),
                        None => app.overview.devices.remove(&r.target),
                    };
                }
                vec![crate::sessions::UiEvent::JobResult { job, kind: "mass_inventory".into(), target: String::new(), json: "{}".into() }]
            })
        });
        self.mass.job = Some(job);
        Ok(json!({ "job": job, "total": total, "pending": pending }))
    }

    /// Stand für den Reiter: Zeilen, Fortschritt, ob noch etwas läuft.
    pub fn mass_rows(&self) -> Value {
        let rows = self.mass.rows.lock().unwrap_or_else(|e| e.into_inner());
        let running = self.mass.job.is_some_and(|j| self.jobs.is_pending(j));
        json!({
            "rows": rows.iter().map(MassRow::json).collect::<Vec<_>>(),
            "done": self.mass.done.load(Ordering::Relaxed),
            "total": rows.len(),
            "running": running,
            "title": self.mass.title,
        })
    }

    pub fn mass_cancel(&mut self) -> Value {
        if let Some(j) = self.mass.job {
            self.jobs.cancel(j);
        }
        json!({ "ok": true })
    }
}

/// Arbeitet die Warteschlange mit [`PARALLEL`] Fäden ab.
fn run(queue: VecDeque<Work>, rows: Arc<Mutex<Vec<MassRow>>>, done: Arc<AtomicUsize>, known: rj_ssh::known_hosts::KnownHosts, store: Store, cancel: Arc<AtomicBool>) {
    let queue = Arc::new(Mutex::new(queue));
    let threads: Vec<_> = (0..PARALLEL)
        .map(|_| {
            let (queue, rows, done, known, store, cancel) = (queue.clone(), rows.clone(), done.clone(), known.clone(), store.clone(), cancel.clone());
            std::thread::spawn(move || loop {
                let Some(w) = queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front() else { break };
                let set = |state: &'static str, detail: String, aruba: bool| {
                    let mut r = rows.lock().unwrap_or_else(|e| e.into_inner());
                    r[w.row].state = state;
                    r[w.row].detail = detail;
                    r[w.row].aruba = aruba;
                };
                if cancel.load(Ordering::Relaxed) {
                    set("abgebrochen", String::new(), false);
                    done.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                set("läuft", String::new(), false);
                let ssh_open = w.spec.protocol == Protocol::Ssh || port_open(&w.spec.host, w.ssh_port);
                let res = if ssh_open {
                    remote::ssh_inventory(&w.spec, w.ssh_port, None, known.clone(), w.windows)
                } else if w.windows && cfg!(windows) {
                    remote::ps_inventory(&w.spec, None)
                } else {
                    Err(format!("SSH (Port {}) nicht erreichbar", w.ssh_port))
                };
                match res {
                    Ok((found, src, note)) => {
                        let mut inv = store.load(w.target);
                        let at = now();
                        for (k, v) in found {
                            inv.offer(k, &v, src, at);
                        }
                        let aruba = device_kind(&inv) == Some("aruba-ap");
                        let system = inv.value("system").unwrap_or("erkannt").to_string();
                        match store.save(w.target, &inv) {
                            Ok(()) => set("erkannt", note.map(|n| format!("{system} – {n}")).unwrap_or(system), aruba),
                            Err(e) => set("fehler", format!("Inventar nicht gespeichert: {e}"), aruba),
                        }
                    }
                    Err(e) => set("fehler", e, false),
                }
                done.fetch_add(1, Ordering::Relaxed);
            })
        })
        .collect();
    for t in threads {
        let _ = t.join();
    }
}

/// Kurzer TCP-Test, ob ein Port offen ist (3 s).
fn port_open(host: &str, port: u16) -> bool {
    let Ok(addrs) = (host.trim_matches(|c| c == '[' || c == ']'), port).to_socket_addrs() else { return false };
    addrs.into_iter().take(2).any(|a| std::net::TcpStream::connect_timeout(&a, Duration::from_secs(3)).is_ok())
}
