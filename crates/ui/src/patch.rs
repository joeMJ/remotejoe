//! Ausführung der Patchday-Aufgaben aus der Oberfläche heraus.

use std::collections::HashMap;
use std::time::Duration;

use rj_core::{Protocol, PsrpTransport, Secret, WinrmAuth};
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
}

pub struct AppExecutor {
    pub jobs: HashMap<Uuid, JobSpec>,
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
        let params = SshParams { host: spec.host.clone(), port: spec.port, user: spec.credentials.username.clone(), auth, connect_timeout: Duration::from_secs(15), keepalive: Duration::from_secs(30) };
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
        let transport = spec.transport.clone().unwrap_or(PsrpTransport::Winrm { https: true, port: None, auth: WinrmAuth::Ntlm, verify_certificate: true });
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

impl Executor for AppExecutor {
    fn run(&self, target: Uuid, job: &JobHandle) -> Result<(), String> {
        let spec = self.jobs.get(&target).ok_or("Zugangsdaten für dieses Ziel nicht ermittelbar")?;
        match spec.protocol {
            Protocol::Ssh => self.apt(spec, job),
            Protocol::Psrp | Protocol::Rdp | Protocol::Hyperv => self.windows(spec, job),
            _ => Err("Für dieses Protokoll gibt es keine Aktualisierungsaufgabe".into()),
        }
    }
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
