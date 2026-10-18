//! Ausführung der Patchday-Aufgaben aus der Oberfläche heraus.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use rj_core::{Protocol, PsrpTransport, Secret};
use rj_ipc::CredentialPayload;
use rj_ssh::client::{Auth, BlockingShell, SshConnection, SshParams};
use rj_ssh::known_hosts::KnownHosts;
use rj_tasks::patchday::{Executor, JobHandle, JobState};
use rj_tasks::winupdate::{parse_line, WuEvent};
use uuid::Uuid;

/// Alles, was eine Aufgabe für ein Ziel braucht (vorab aus Index und KDBX ermittelt).
pub struct JobSpec {
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub credentials: CredentialPayload,
    pub transport: Option<PsrpTransport>,
    /// Ältere SSH-Verfahren zulassen (Option des Ziels).
    pub legacy: bool,
}

/// Zugangsdaten eines Patchday-Laufs im Hauptprozess. Eine Aufgabe nimmt
/// ihre Angaben beim Start heraus (danach liegt hier keine Kopie mehr).
/// Braucht sie sie erneut (Neuverbinden) oder ist sie noch nicht gestartet,
/// fordert sie sie an; die App liefert sie nur im entsperrten Zustand nach.
/// Sperren löscht alle hier liegenden Kopien.
#[derive(Default)]
pub struct CredStore {
    inner: Mutex<CredInner>,
    cv: Condvar,
}

#[derive(Default)]
struct CredInner {
    specs: HashMap<Uuid, JobSpec>,
    /// Ziele, deren Aufgabe auf Zugangsdaten wartet.
    wanted: BTreeSet<Uuid>,
    /// Ziele ohne ermittelbare Zugangsdaten.
    missing: BTreeSet<Uuid>,
    locked: bool,
}

impl CredStore {
    pub fn new(specs: HashMap<Uuid, JobSpec>) -> Self {
        CredStore { inner: Mutex::new(CredInner { specs, ..Default::default() }), cv: Condvar::new() }
    }

    fn lock_inner(&self) -> std::sync::MutexGuard<'_, CredInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Nimmt die Angaben für `target` heraus. Liegen keine vor, werden sie
    /// angefordert und es wird gewartet (bis geliefert, abgebrochen oder als
    /// nicht ermittelbar gemeldet). `waiting` wird einmal aufgerufen, wenn
    /// gewartet werden muss.
    pub fn take(&self, target: Uuid, cancelled: &dyn Fn() -> bool, waiting: &dyn Fn(bool)) -> Result<JobSpec, String> {
        let mut g = self.lock_inner();
        let mut told = false;
        loop {
            if cancelled() {
                g.wanted.remove(&target);
                return Err("abgebrochen".into());
            }
            if !g.locked {
                if let Some(spec) = g.specs.remove(&target) {
                    g.wanted.remove(&target);
                    return Ok(spec);
                }
                if g.missing.remove(&target) {
                    g.wanted.remove(&target);
                    return Err("Zugangsdaten für dieses Ziel nicht ermittelbar".into());
                }
            }
            g.wanted.insert(target);
            if !told {
                waiting(g.locked);
                told = true;
            }
            g = self.cv.wait_timeout(g, Duration::from_millis(200)).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    /// Sperren: alle Kopien löschen (die Angaben werden beim Freigeben überschrieben).
    pub fn wipe_on_lock(&self) {
        let mut g = self.lock_inner();
        g.locked = true;
        g.specs.clear();
    }

    /// Ziele, für die Angaben angefordert sind (leer, solange gesperrt).
    pub fn wanted(&self) -> Vec<Uuid> {
        let g = self.lock_inner();
        if g.locked {
            return Vec::new();
        }
        g.wanted.iter().copied().filter(|t| !g.specs.contains_key(t) && !g.missing.contains(t)).collect()
    }

    /// Entsperrt: Angaben nachliefern (`None`: nicht ermittelbar).
    pub fn provide(&self, target: Uuid, spec: Option<JobSpec>) {
        let mut g = self.lock_inner();
        match spec {
            Some(s) => {
                g.specs.insert(target, s);
            }
            None => {
                g.missing.insert(target);
            }
        }
        self.cv.notify_all();
    }

    pub fn set_unlocked(&self) {
        self.lock_inner().locked = false;
        self.cv.notify_all();
    }

    /// Zahl der Kopien von Zugangsdaten, die hier noch liegen.
    pub fn held(&self) -> usize {
        self.lock_inner().specs.len()
    }
}

pub struct AppExecutor {
    pub creds: Arc<CredStore>,
    pub execute: bool,
    pub full_upgrade: bool,
    pub known_hosts: KnownHosts,
}

struct JobProgress<'a>(&'a JobHandle);

impl rj_tasks::apt::Progress for JobProgress<'_> {
    fn phase(&mut self, phase: &str) {
        self.0.set_state(match phase {
            "installing" => JobState::Installing,
            _ => JobState::Searching,
        });
    }
    fn log(&mut self, line: &str) {
        self.0.log(line);
    }
}

impl AppExecutor {
    fn apt(&self, spec: &JobSpec, job: &JobHandle) -> Result<(), String> {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
        // Im Stapelbetrieb keine Rückfragen: unbekannte Host-Schlüssel werden abgelehnt.
        let (ask, mut q) = tokio::sync::mpsc::unbounded_channel::<(rj_ssh::known_hosts::HostKeyQuestion, tokio::sync::oneshot::Sender<bool>)>();
        rt.spawn(async move {
            while let Some((_, r)) = q.recv().await {
                let _ = r.send(false);
            }
        });
        let pw = Secret::from_bytes(spec.credentials.password.0.clone());
        let mut auth = Vec::new();
        if let Some(k) = &spec.credentials.private_key {
            auth.push(Auth::Key { pem: Secret::from_bytes(k.0.clone()), passphrase: None });
        }
        auth.push(Auth::Password(pw.clone()));
        auth.push(Auth::KeyboardInteractive(pw.clone()));
        let params = SshParams { host: spec.host.clone(), port: spec.port, user: spec.credentials.username.clone(), auth, connect_timeout: Duration::from_secs(15), keepalive: Duration::from_secs(30), legacy: spec.legacy };
        let conn = rt
            .block_on(SshConnection::connect(&params, self.known_hosts.clone(), ask))
            .map_err(|e| match e {
                rj_ssh::client::SshError::HostKeyRejected => "Host-Schlüssel unbekannt – bitte zuerst einmal direkt verbinden".to_string(),
                e => e.to_string(),
            })?;
        let mut sh = rt.block_on(conn.open_shell("dumb", 200, 50)).map_err(|e| e.to_string())?;
        let creds = move |n: &str| (n == "password").then(|| pw.clone());
        let handle = rt.handle().clone();
        let mut io = BlockingShell { rt: handle, shell: &mut sh };
        let report = rj_tasks::apt::run(&mut io, &creds, self.execute, self.full_upgrade, &mut JobProgress(job)).map_err(|e| e.to_string())?;
        job.set_count(report.updates.len() as u32);
        for u in report.updates.iter().take(200) {
            job.log(&format!("{} {} → {}", u.package, u.old_version.as_deref().unwrap_or("?"), u.new_version));
        }
        if report.reboot_required {
            job.set_state(JobState::RebootRequired);
        }
        Ok(())
    }

    fn windows(&self, spec: &JobSpec, job: &JobHandle) -> Result<(), String> {
        let transport = PsrpTransport::effective(spec.transport.clone(), None);
        let t = rj_psrp::PsTarget { host: spec.host.clone(), username: spec.credentials.username.clone(), domain: spec.credentials.domain.clone(), transport };
        let mut s = rj_psrp::PsSession::start(&t, &Secret::from_bytes(spec.credentials.password.0.clone())).map_err(|e| e.to_string())?;
        let mut found = 0;
        let mut reboot = false;
        let mut fatal = None;
        s.run(&rj_tasks::winupdate::script(self.execute), |r| {
            if job.cancelled() {
                return;
            }
            match (r.stream.as_str(), parse_line(&r.text)) {
                (_, Some(WuEvent::Phase { .. })) => job.set_state(JobState::Searching),
                (_, Some(WuEvent::Found { title, kb, .. })) => {
                    found += 1;
                    job.set_count(found);
                    job.log(&format!("{kb} {title}"));
                }
                (_, Some(WuEvent::Progress { phase, index, total, title })) => {
                    job.set_state(if phase == "downloading" { JobState::Downloading } else { JobState::Installing });
                    job.set_percent((index * 100 / total.max(1)) as u8);
                    job.log(&format!("{phase} {index}/{total}: {title}"));
                }
                (_, Some(WuEvent::Error { title, code })) => job.log(&format!("Fehler {code}: {title}")),
                (_, Some(WuEvent::Result { reboot_required, .. })) => reboot = reboot_required,
                (_, Some(WuEvent::Fatal { message })) => fatal = Some(message),
                ("error", None) => job.log(&format!("Fehler: {}", r.text)),
                _ => {}
            }
        })
        .map_err(|e| e.to_string())?;
        if let Some(m) = fatal {
            return Err(m);
        }
        if reboot {
            job.set_state(JobState::RebootRequired);
        }
        Ok(())
    }
}

impl AppExecutor {
    fn take(&self, target: Uuid, job: &JobHandle) -> Result<JobSpec, String> {
        self.creds.take(target, &|| job.cancelled(), &|locked| {
            if locked {
                job.log("Gesperrt: Die Zugangsdaten werden erst nach dem Entsperren wieder bereitgestellt.");
            }
        })
    }
}

impl Executor for AppExecutor {
    fn run(&self, target: Uuid, job: &JobHandle) -> Result<(), String> {
        let spec = self.take(target, job)?;
        let r = match spec.protocol {
            Protocol::Ssh => self.apt(&spec, job),
            Protocol::Psrp | Protocol::Rdp | Protocol::Hyperv => self.windows(&spec, job),
            _ => Err("Für dieses Protokoll gibt es keine Aktualisierungsaufgabe".into()),
        };
        drop(spec);
        match r {
            // Verbindung nicht zustande gekommen: einmal neu verbinden; die
            // Zugangsdaten werden dafür neu angefordert (nach dem Entsperren).
            Err(e) if spec_is_retryable(&e) && !job.cancelled() => {
                job.log(&format!("{e} – neuer Versuch"));
                let spec = self.take(target, job)?;
                match spec.protocol {
                    Protocol::Ssh => self.apt(&spec, job),
                    _ => self.windows(&spec, job),
                }
            }
            r => r,
        }
    }
}

/// Fehler beim Verbindungsaufbau, nach denen ein zweiter Versuch sinnvoll ist.
fn spec_is_retryable(e: &str) -> bool {
    e.contains("Zeitüberschreitung beim Verbindungsaufbau") || e.starts_with("Verbindung zu ")
}

/// Vorführung ohne echte Ziele (Bildschirmfotos).
pub struct DemoExecutor;

impl Executor for DemoExecutor {
    fn run(&self, target: Uuid, job: &JobHandle) -> Result<(), String> {
        let n = target.as_bytes()[0];
        let steps = 4 + (n % 5) as u32;
        job.set_count(steps);
        for i in 0..steps {
            if job.cancelled() {
                return Ok(());
            }
            job.set_state(if i < steps / 2 { JobState::Downloading } else { JobState::Installing });
            job.set_percent(((i + 1) * 100 / steps) as u8);
            job.log(&format!("Aktualisierung {}/{steps}", i + 1));
            std::thread::sleep(Duration::from_millis(150 + (n as u64 % 7) * 40));
        }
        match n % 6 {
            0 => Err("Zeitüberschreitung beim Verbindungsaufbau".into()),
            1 => {
                job.set_state(JobState::RebootRequired);
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> JobSpec {
        JobSpec {
            protocol: Protocol::Ssh,
            host: "srv.example.test".into(),
            port: 22,
            credentials: CredentialPayload { username: "root".into(), domain: None, password: rj_ipc::SecretBytes(b"geheim".to_vec()), private_key: None },
            transport: None,
            legacy: false,
        }
    }

    #[test]
    fn copies_are_dropped_after_take_and_reconnect_waits_for_unlock() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let store = Arc::new(CredStore::new(HashMap::from([(a, spec()), (b, spec())])));
        let no = || false;
        // Übernommen: keine Kopie mehr im Hauptprozess.
        assert!(store.take(a, &no, &|_| {}).is_ok());
        assert_eq!(store.held(), 1);
        // Sperren löscht die übrigen Kopien.
        store.wipe_on_lock();
        assert_eq!(store.held(), 0);
        // Neuverbinden im gesperrten Zustand: wartet, keine Anforderung an die App.
        let s2 = store.clone();
        let waited_locked = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let w = waited_locked.clone();
        let t = std::thread::spawn(move || s2.take(a, &|| false, &|locked| w.store(locked, std::sync::atomic::Ordering::SeqCst)).map(|s| s.host));
        std::thread::sleep(Duration::from_millis(300));
        assert!(!t.is_finished());
        assert!(waited_locked.load(std::sync::atomic::Ordering::SeqCst));
        assert!(store.wanted().is_empty(), "gesperrt: nichts anfordern");
        // Entsperren: die App liefert nach, die Aufgabe läuft weiter.
        store.set_unlocked();
        assert_eq!(store.wanted(), vec![a]);
        store.provide(a, Some(spec()));
        assert_eq!(t.join().unwrap().unwrap(), "srv.example.test");
        assert_eq!(store.held(), 0);
        // Nicht ermittelbar: Fehler statt Warten.
        let s3 = store.clone();
        let t = std::thread::spawn(move || s3.take(b, &|| false, &|_| {}));
        while store.wanted().is_empty() {
            std::thread::sleep(Duration::from_millis(10));
        }
        store.provide(b, None);
        assert!(t.join().unwrap().is_err());
        // Abbruch beendet das Warten.
        let c = Uuid::new_v4();
        assert!(store.take(c, &|| true, &|_| {}).is_err());
    }
}
