//! „Schlüssel und Zertifikate prüfen“: Für viele Ziele den Server-Schlüssel bzw. das Zertifikat
//! abholen, ohne sich anzumelden, und als Liste zeigen (neu, bekannt, geändert, nicht erreichbar).
//! Nach Bestätigung werden nur die **neuen** als vertrauenswürdig gespeichert; geänderte nie
//! automatisch – die bleiben beim nächsten Verbinden einzeln zu prüfen.

use std::time::Duration;

use rj_ssh::known_hosts::{fingerprint, HostKeyStatus, KnownHosts};
use serde_json::{json, Value};
use uuid::Uuid;

/// Zeitlimit je Ziel für Verbindungsaufbau und Handshake.
const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
/// Höchstens so viele Ziele gleichzeitig.
const PARALLEL: usize = 8;

/// Was für ein Ziel geprüft wird.
#[derive(Debug, Clone)]
pub struct ProbeSpec {
    pub target: Uuid,
    pub name: String,
    pub protocol: rj_core::Protocol,
    pub host: String,
    pub port: u16,
    pub legacy: bool,
    /// Ziel wird nicht geprüft (Grund für die Liste), z. B. Verbindung über einen Tunnel.
    pub skip: Option<String>,
}

/// Zustand einer Zeile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckState {
    New,
    Known,
    Changed { previous: String },
    Unreachable(String),
    Skipped(String),
    /// Für Erledigt-Anzeige nach dem Übernehmen.
    Saved,
}

/// Was zum Speichern gebraucht wird.
#[derive(Debug, Clone)]
pub enum SaveData {
    None,
    /// SSH-Host-Schlüssel im OpenSSH-Format (`alg base64`).
    Ssh(String),
}

#[derive(Debug, Clone)]
pub struct CheckRow {
    pub spec: ProbeSpec,
    pub kind: &'static str,
    pub fingerprint: String,
    pub state: CheckState,
    pub save: SaveData,
}

impl CheckRow {
    pub fn json(&self) -> Value {
        let (state, detail) = match &self.state {
            CheckState::New => ("new", String::new()),
            CheckState::Known => ("known", String::new()),
            CheckState::Changed { previous } => ("changed", previous.clone()),
            CheckState::Unreachable(e) => ("unreachable", e.clone()),
            CheckState::Skipped(e) => ("skipped", e.clone()),
            CheckState::Saved => ("saved", String::new()),
        };
        json!({
            "target": self.spec.target,
            "name": self.spec.name,
            "protocol": self.spec.protocol.as_str(),
            "address": format!("{}:{}", self.spec.host, self.spec.port),
            "kind": self.kind,
            "fingerprint": self.fingerprint,
            "state": state,
            "detail": detail,
        })
    }
}

/// Protokolle, die dieser Prüflauf kennt.
pub fn supported(p: rj_core::Protocol) -> bool {
    matches!(p, rj_core::Protocol::Ssh)
}

/// Prüft alle Ziele (läuft im Hintergrundfaden, blockiert bis zum Ende oder Abbruch).
pub fn run(rt: &tokio::runtime::Handle, specs: Vec<ProbeSpec>, cancel: std::sync::Arc<std::sync::atomic::AtomicBool>, known: Option<KnownHosts>) -> Vec<CheckRow> {
    rt.block_on(async move {
        let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(PARALLEL));
        let mut set = tokio::task::JoinSet::new();
        for (i, spec) in specs.into_iter().enumerate() {
            let sem = sem.clone();
            let known = known.clone();
            let cancel = cancel.clone();
            set.spawn(async move {
                let _permit = sem.acquire_owned().await;
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return (i, row(spec, "", String::new(), CheckState::Skipped("abgebrochen".into()), SaveData::None));
                }
                (i, probe(spec, known.as_ref()).await)
            });
        }
        let mut rows: Vec<(usize, CheckRow)> = Vec::new();
        while let Some(r) = set.join_next().await {
            if let Ok(r) = r {
                rows.push(r);
            }
        }
        rows.sort_by_key(|(i, _)| *i);
        rows.into_iter().map(|(_, r)| r).collect()
    })
}

fn row(spec: ProbeSpec, kind: &'static str, fingerprint: String, state: CheckState, save: SaveData) -> CheckRow {
    CheckRow { spec, kind, fingerprint, state, save }
}

async fn probe(spec: ProbeSpec, known: Option<&KnownHosts>) -> CheckRow {
    if let Some(why) = spec.skip.clone() {
        let kind = kind_of(spec.protocol);
        return row(spec, kind, String::new(), CheckState::Skipped(why), SaveData::None);
    }
    match spec.protocol {
        rj_core::Protocol::Ssh => probe_ssh(spec, known).await,
        _ => {
            let kind = kind_of(spec.protocol);
            row(spec, kind, String::new(), CheckState::Skipped("Protokoll wird noch nicht geprüft".into()), SaveData::None)
        }
    }
}

fn kind_of(p: rj_core::Protocol) -> &'static str {
    match p {
        rj_core::Protocol::Ssh => "SSH-Host-Schlüssel",
        _ => "Zertifikat",
    }
}

async fn probe_ssh(spec: ProbeSpec, known: Option<&KnownHosts>) -> CheckRow {
    let kind = kind_of(spec.protocol);
    let key = match rj_ssh::client::probe_host_key(&spec.host, spec.port, PROBE_TIMEOUT, spec.legacy).await {
        Ok(k) => k,
        Err(e) => return row(spec, kind, String::new(), CheckState::Unreachable(e.to_string()), SaveData::None),
    };
    let fp = fingerprint(&key);
    let state = match known.map(|k| k.check(&spec.host, spec.port, &key)) {
        Some(HostKeyStatus::Known) => CheckState::Known,
        Some(HostKeyStatus::Changed { previous_fingerprint }) => CheckState::Changed { previous: previous_fingerprint },
        Some(HostKeyStatus::Unknown) | None => CheckState::New,
    };
    let openssh = key.to_openssh().unwrap_or_default();
    row(spec, kind, fp, state, SaveData::Ssh(openssh))
}

/// Speichert alle neuen Zeilen. Gibt (gespeichert, Fehlermeldungen) zurück; gespeicherte Zeilen
/// stehen danach auf `Saved`. Geänderte, bekannte und nicht erreichbare bleiben unberührt.
pub fn accept_new(rows: &mut [CheckRow], known: Option<KnownHosts>) -> (usize, Vec<String>) {
    let mut saved = 0;
    let mut errors = Vec::new();
    for r in rows.iter_mut().filter(|r| r.state == CheckState::New) {
        let res = match &r.save {
            SaveData::Ssh(line) => match &known {
                Some(k) => k.remember_openssh(&r.spec.host, r.spec.port, line).map_err(|e| e.to_string()),
                None => Err("kein Datenordner".to_string()),
            },
            SaveData::None => continue,
        };
        match res {
            Ok(()) => {
                r.state = CheckState::Saved;
                saved += 1;
            }
            Err(e) => errors.push(format!("{}: {e}", r.spec.name)),
        }
    }
    (saved, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(host: &str, port: u16) -> ProbeSpec {
        ProbeSpec { target: Uuid::new_v4(), name: "t".into(), protocol: rj_core::Protocol::Ssh, host: host.into(), port, legacy: false, skip: None }
    }

    #[test]
    fn closed_port_is_unreachable_and_skips_are_reported() {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut tunnel = spec("10.0.0.1", 22);
        tunnel.skip = Some("über Tunnel".into());
        let rows = run(rt.handle(), vec![spec("127.0.0.1", closed), tunnel], Default::default(), None);
        assert_eq!(rows.len(), 2);
        assert!(matches!(rows[0].state, CheckState::Unreachable(_)), "{:?}", rows[0].state);
        assert_eq!(rows[1].state, CheckState::Skipped("über Tunnel".into()));
        assert_eq!(rows[1].json()["state"], "skipped");
    }

    #[test]
    fn only_new_rows_are_saved() {
        let mut rows = vec![
            row(spec("a", 22), "SSH-Host-Schlüssel", "fp".into(), CheckState::Changed { previous: "alt".into() }, SaveData::Ssh("x".into())),
            row(spec("b", 22), "SSH-Host-Schlüssel", "fp".into(), CheckState::Known, SaveData::Ssh("x".into())),
        ];
        let (n, errors) = accept_new(&mut rows, None);
        assert_eq!((n, errors.len()), (0, 0));
        assert!(matches!(rows[0].state, CheckState::Changed { .. }));
    }

    /// Echter `sshd` als normaler Benutzer (nur Schlüsselaustausch, keine Anmeldung nötig).
    struct UserSshd {
        child: std::process::Child,
        port: u16,
        fingerprint: String,
        _dir: tempfile::TempDir,
    }

    impl Drop for UserSshd {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn user_sshd() -> Option<UserSshd> {
        if !std::path::Path::new("/usr/sbin/sshd").exists() {
            eprintln!("ÜBERSPRUNGEN: /usr/sbin/sshd fehlt");
            return None;
        }
        let dir = tempfile::tempdir().ok()?;
        let key = dir.path().join("hostkey");
        let ok = std::process::Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(&key).status().ok()?.success();
        if !ok {
            return None;
        }
        let port = std::net::TcpListener::bind("127.0.0.1:0").ok()?.local_addr().ok()?.port();
        let cfg = dir.path().join("cfg");
        std::fs::write(&cfg, format!("Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nPidFile none\nUsePAM no\n", key.display())).ok()?;
        let child = std::process::Command::new("/usr/sbin/sshd").args(["-D", "-e", "-f"]).arg(&cfg).stderr(std::process::Stdio::null()).spawn().ok()?;
        // Fingerabdruck wie OpenSSH ihn zeigt („256 SHA256:… Kommentar“).
        let out = std::process::Command::new("ssh-keygen").arg("-lf").arg(dir.path().join("hostkey.pub")).output().ok()?;
        let fp = String::from_utf8_lossy(&out.stdout).split_whitespace().nth(1)?.to_string();
        let s = UserSshd { child, port, fingerprint: fp, _dir: dir };
        for _ in 0..50 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return Some(s);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!("ÜBERSPRUNGEN: sshd als Benutzer startet nicht");
        None
    }

    #[test]
    fn ssh_key_is_new_then_saved_then_known_and_a_different_key_is_changed() {
        let Some(sshd) = user_sshd() else { return };
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let known = KnownHosts::new(dir.path().join("known_hosts"));
        let specs = || vec![spec("127.0.0.1", sshd.port)];

        let mut rows = run(rt.handle(), specs(), Default::default(), Some(known.clone()));
        assert_eq!(rows[0].state, CheckState::New, "{:?}", rows[0].state);
        assert_eq!(rows[0].fingerprint, sshd.fingerprint);
        let (n, errors) = accept_new(&mut rows, Some(known.clone()));
        assert_eq!((n, errors.len()), (1, 0));
        assert_eq!(rows[0].state, CheckState::Saved);

        let rows = run(rt.handle(), specs(), Default::default(), Some(known.clone()));
        assert_eq!(rows[0].state, CheckState::Known);

        // Anderer Schlüssel unter derselben Adresse gespeichert: „geändert“, und Übernehmen lässt ihn stehen.
        let other = dir.path().join("other_known_hosts");
        let k2 = KnownHosts::new(&other);
        k2.remember_openssh("127.0.0.1", sshd.port, "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGq6lxXqVOFtYi0gm3VYw6c1BZrfF1t6wEXeL1qD5xQ9").unwrap();
        let before = std::fs::read(&other).unwrap();
        let mut rows = run(rt.handle(), specs(), Default::default(), Some(k2.clone()));
        assert!(matches!(rows[0].state, CheckState::Changed { .. }), "{:?}", rows[0].state);
        let (n, _) = accept_new(&mut rows, Some(k2));
        assert_eq!(n, 0);
        assert_eq!(std::fs::read(&other).unwrap(), before);
    }
}
