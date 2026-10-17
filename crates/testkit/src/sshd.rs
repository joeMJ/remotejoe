//! OpenSSH-Server für Tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use crate::{free_port, have_program, is_root, run, wait_for_port, Skip};

/// Testbenutzer auf dem Testsystem.
pub const USER: &str = "rjtest";
/// Kennwort des Testbenutzers (mit Umlaut und Emoji, byte-genau zu prüfen).
pub const PASSWORD: &str = "Prüf-Kennwort-ß-🚀";

/// Laufender `sshd`.
pub struct Sshd {
    pub port: u16,
    pub dir: tempfile::TempDir,
    child: Option<Child>,
    config: PathBuf,
}

impl Sshd {
    /// Startet einen `sshd` auf einem freien Hochport. Benötigt Root-Rechte
    /// (Anmeldung eines anderen Benutzers) und `/usr/sbin/sshd`.
    pub fn start() -> Result<Sshd, Skip> {
        Self::start_with(&[])
    }

    /// Wie [`Sshd::start`] mit zusätzlichen Konfigurationszeilen.
    pub fn start_with(extra: &[&str]) -> Result<Sshd, Skip> {
        if !have_program("/usr/sbin/sshd") {
            return Err(Skip("/usr/sbin/sshd fehlt".into()));
        }
        if !is_root() {
            return Err(Skip("Root-Rechte nötig, um einen Testbenutzer anzumelden".into()));
        }
        ensure_user()?;
        fs::create_dir_all("/run/sshd").map_err(|e| Skip(e.to_string()))?;
        let dir = tempfile::Builder::new().prefix("rj-sshd").tempdir().map_err(|e| Skip(e.to_string()))?;
        let d = dir.path();
        // Rechte, damit sshd (StrictModes aus) und der Testbenutzer lesen können.
        set_mode(d, 0o755);
        let host_key = d.join("host_ed25519");
        keygen(&host_key, "ed25519")?;
        for (name, kind) in [("client_ed25519", "ed25519"), ("client_rsa", "rsa")] {
            keygen(&d.join(name), kind)?;
        }
        let mut auth = String::new();
        for name in ["client_ed25519.pub", "client_rsa.pub"] {
            auth.push_str(&fs::read_to_string(d.join(name)).map_err(|e| Skip(e.to_string()))?);
        }
        fs::write(d.join("authorized_keys"), auth).map_err(|e| Skip(e.to_string()))?;
        set_mode(&d.join("authorized_keys"), 0o644);
        let port = free_port();
        let config = d.join("sshd_config");
        let mut cfg = format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {hk}\nPidFile {pid}\nAuthorizedKeysFile {ak}\n\
             StrictModes no\nUsePAM yes\nPasswordAuthentication yes\nKbdInteractiveAuthentication yes\n\
             PubkeyAuthentication yes\nPermitRootLogin no\nAllowUsers {USER}\nMaxStartups 50\nMaxSessions 50\n\
             ClientAliveInterval 0\nLogLevel ERROR\nSubsystem sftp internal-sftp\n",
            hk = host_key.display(),
            pid = d.join("sshd.pid").display(),
            ak = d.join("authorized_keys").display(),
        );
        // Netz-Proxy der Testumgebung an die Sitzungen weiterreichen (z. B. für apt).
        let proxies: Vec<String> = ["http_proxy", "https_proxy", "no_proxy", "HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY"]
            .iter()
            .filter_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty() && !v.contains('"')).map(|v| format!("{k}=\"{v}\"")))
            .collect();
        if !proxies.is_empty() {
            cfg.push_str(&format!("SetEnv {}\n", proxies.join(" ")));
        }
        for l in extra {
            cfg.push_str(l);
            cfg.push('\n');
        }
        fs::write(&config, cfg).map_err(|e| Skip(e.to_string()))?;
        let child = Command::new("/usr/sbin/sshd")
            .args(["-D", "-e", "-f"])
            .arg(&config)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| Skip(format!("sshd: {e}")))?;
        let mut s = Sshd { port, dir, child: Some(child), config };
        if !wait_for_port(port, Duration::from_secs(10)) {
            s.stop();
            return Err(Skip("sshd nimmt keine Verbindungen an".into()));
        }
        Ok(s)
    }

    pub fn client_key(&self, kind: &str) -> PathBuf {
        self.dir.path().join(format!("client_{kind}"))
    }

    pub fn host_key_pub(&self) -> String {
        fs::read_to_string(self.dir.path().join("host_ed25519.pub")).unwrap_or_default()
    }

    pub fn config_path(&self) -> &Path {
        &self.config
    }

    /// Trennt alle bestehenden Sitzungen (Kindprozesse des sshd), der
    /// Server selbst läuft weiter.
    pub fn drop_sessions(&self) {
        if let Some(c) = &self.child {
            for pid in descendants(c.id()) {
                // SAFETY: Signal an einen Nachfahren des selbst gestarteten sshd.
                #[cfg(unix)]
                unsafe {
                    libc::kill(pid as i32, libc::SIGKILL);
                }
            }
        }
    }

    pub fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            self.drop_sessions();
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for Sshd {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Alle Nachfahren eines Prozesses (Linux, über /proc).
pub(crate) fn descendants(root: u32) -> Vec<u32> {
    let mut parents: Vec<(u32, u32)> = Vec::new();
    if let Ok(rd) = fs::read_dir("/proc") {
        for e in rd.flatten() {
            let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
            let Ok(stat) = fs::read_to_string(e.path().join("stat")) else { continue };
            let rest = stat.rsplit_once(')').map(|(_, r)| r).unwrap_or("");
            if let Some(ppid) = rest.split_whitespace().nth(1).and_then(|p| p.parse().ok()) {
                parents.push((pid, ppid));
            }
        }
    }
    let mut out = Vec::new();
    let mut frontier = vec![root];
    while let Some(p) = frontier.pop() {
        for (pid, ppid) in &parents {
            if *ppid == p {
                out.push(*pid);
                frontier.push(*pid);
            }
        }
    }
    out
}

pub(crate) fn set_mode(p: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(p, fs::Permissions::from_mode(mode));
    }
}

fn keygen(path: &Path, kind: &str) -> Result<(), Skip> {
    let p = path.to_str().ok_or_else(|| Skip("Pfad".into()))?;
    let mut args = vec!["-q", "-t", kind, "-N", "", "-f", p, "-C", "rj-test"];
    if kind == "rsa" {
        args.extend(["-b", "2048"]);
    }
    run("ssh-keygen", &args, None)?;
    set_mode(path, 0o600);
    Ok(())
}

/// Legt den Testbenutzer an (einmalig) und setzt Kennwort und sudo-Recht.
pub fn ensure_user() -> Result<(), Skip> {
    static ONCE: std::sync::Mutex<bool> = std::sync::Mutex::new(false);
    let mut done = ONCE.lock().unwrap_or_else(|e| e.into_inner());
    if *done {
        return Ok(());
    }
    if run("id", &[USER], None).is_err() {
        run("useradd", &["-m", "-s", "/bin/bash", USER], None)?;
    }
    run("chpasswd", &[], Some(format!("{USER}:{PASSWORD}\n").as_bytes()))?;
    let sudoers = format!(
        "{USER} ALL=(ALL) ALL\nDefaults:{USER} timestamp_timeout=0\n\
         Defaults:{USER} env_keep += \"http_proxy https_proxy no_proxy HTTP_PROXY HTTPS_PROXY NO_PROXY\"\n"
    );
    fs::write("/etc/sudoers.d/rjtest-remotejoe", sudoers).map_err(|e| Skip(e.to_string()))?;
    set_mode(Path::new("/etc/sudoers.d/rjtest-remotejoe"), 0o440);
    *done = true;
    Ok(())
}
