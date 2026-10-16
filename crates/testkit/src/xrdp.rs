//! xrdp mit Xvnc-Sitzung für RDP-Tests.
//!
//! In der Sitzung läuft ein `xterm`, das Tastatureingaben in
//! `~/rj-keys.txt` schreibt; die Zwischenablage der Sitzung ist über
//! `xclip` auf der Anzeige der Sitzung prüfbar.

use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use crate::sshd::set_mode;

/// Testbenutzer für xrdp. xrdp 0.9 verarbeitet Kennwörter mit Zeichen
/// außerhalb der Basisebene (z. B. Emoji) nicht; daher ohne Emoji.
pub const USER: &str = "rjrdp";
/// Zweiter Testbenutzer (zweite, unabhängige Sitzung).
pub const USER2: &str = "rjrdp2";
pub const PASSWORD: &str = "Prüf-ß-Kennwort-ä";

fn ensure_user(user: &str) -> Result<(), Skip> {
    if run("id", &[user], None).is_err() {
        run("useradd", &["-m", "-s", "/bin/bash", user], None)?;
    }
    run("chpasswd", &[], Some(format!("{user}:{PASSWORD}\n").as_bytes()))?;
    // Sitzungsskript: xterm schreibt Tastatureingaben in ~/rj-keys.txt.
    let home = PathBuf::from(format!("/home/{user}"));
    let script = "#!/bin/sh\n\
        xterm -geometry 100x30+0+0 -e sh -c 'stty -echo; cat > \"$HOME/rj-keys.txt\"' &\n\
        exec sleep 100000\n";
    let wm = home.join("startwm.sh");
    fs::write(&wm, script).map_err(|e| Skip(e.to_string()))?;
    set_mode(&wm, 0o755);
    let _ = fs::remove_file(home.join("rj-keys.txt"));
    run("chown", &[&format!("{user}:{user}"), wm.to_str().unwrap_or_default()], None)?;
    Ok(())
}
use crate::{free_port, have_program, is_root, run, wait_for_port, Skip};

pub struct Xrdp {
    pub port: u16,
    pub dir: tempfile::TempDir,
    xrdp: Option<Child>,
    sesman: Option<Child>,
}

fn home() -> PathBuf {
    PathBuf::from(format!("/home/{USER}"))
}

impl Xrdp {
    pub fn start() -> Result<Xrdp, Skip> {
        for p in ["/usr/sbin/xrdp", "/usr/sbin/xrdp-sesman", "/usr/bin/Xvnc", "/usr/bin/xterm", "/usr/bin/xclip"] {
            if !have_program(p) {
                return Err(Skip(format!("{p} fehlt")));
            }
        }
        if !is_root() {
            return Err(Skip("Root-Rechte nötig".into()));
        }
        ensure_user(USER)?;
        ensure_user(USER2)?;
        cleanup_sessions();
        let dir = tempfile::Builder::new().prefix("rj-xrdp").tempdir().map_err(|e| Skip(e.to_string()))?;
        let port = free_port();
        let xrdp_ini = fs::read_to_string("/etc/xrdp/xrdp.ini").map_err(|e| Skip(e.to_string()))?;
        let xrdp_ini = xrdp_ini
            .lines()
            .map(|l| {
                if l.starts_with("port=") && !l.contains("-1") && !l.contains("ask") {
                    format!("port={port}")
                } else if l.starts_with("autorun=") {
                    "autorun=Xvnc".to_string()
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let ini = dir.path().join("xrdp.ini");
        fs::write(&ini, xrdp_ini).map_err(|e| Skip(e.to_string()))?;
        let sesman_ini = dir.path().join("sesman.ini");
        fs::copy("/etc/xrdp/sesman.ini", &sesman_ini).map_err(|e| Skip(e.to_string()))?;

        fs::create_dir_all("/var/run/xrdp").ok();
        let sesman = Command::new("/usr/sbin/xrdp-sesman")
            .args(["-n", "-c"])
            .env("LANG", "C.UTF-8")
            .env("LC_ALL", "C.UTF-8")
            .arg(&sesman_ini)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| Skip(format!("sesman: {e}")))?;
        let xrdp = Command::new("/usr/sbin/xrdp")
            .args(["-n", "-p", &port.to_string(), "-c"])
            .env("LANG", "C.UTF-8")
            .env("LC_ALL", "C.UTF-8")
            .arg(&ini)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| Skip(format!("xrdp: {e}")))?;
        let mut x = Xrdp { port, dir, xrdp: Some(xrdp), sesman: Some(sesman) };
        if !wait_for_port(port, Duration::from_secs(10)) || !wait_for_port(3350, Duration::from_secs(10)) {
            x.stop();
            return Err(Skip("xrdp oder sesman startet nicht".into()));
        }
        Ok(x)
    }

    /// Anzeige der laufenden Sitzung des Testbenutzers (z. B. `:10`).
    pub fn display(&self) -> Option<String> {
        self.display_of(USER)
    }

    pub fn display_of(&self, user: &str) -> Option<String> {
        let out = run("pgrep", &["-u", user, "-a", "Xvnc"], None).ok()?;
        out.split_whitespace().find(|w| w.starts_with(':') && w[1..].chars().all(|c| c.is_ascii_digit())).map(str::to_string)
    }

    /// Führt einen Befehl als Testbenutzer auf der Anzeige der Sitzung aus.
    pub fn x_run(&self, cmd: &str, stdin: Option<&[u8]>) -> Result<String, Skip> {
        let d = self.display().ok_or_else(|| Skip("keine Sitzung".into()))?;
        run("runuser", &["-u", USER, "--", "env", &format!("DISPLAY={d}"), &format!("XAUTHORITY={}/.Xauthority", home().display()), "sh", "-c", cmd], stdin)
    }

    /// Inhalt der Datei, in die das xterm der Sitzung Tastatureingaben schreibt.
    pub fn typed_text(&self) -> String {
        fs::read_to_string(home().join("rj-keys.txt")).unwrap_or_default()
    }

    /// Legt eine Datei im Heimatordner des Testbenutzers an.
    pub fn put_file(&self, name: &str, data: &[u8]) -> Result<PathBuf, Skip> {
        let p = home().join(name);
        fs::write(&p, data).map_err(|e| Skip(e.to_string()))?;
        run("chown", &[&format!("{USER}:{USER}"), p.to_str().unwrap_or_default()], None)?;
        Ok(p)
    }

    pub fn stop(&mut self) {
        for c in [self.xrdp.take(), self.sesman.take()].into_iter().flatten() {
            let mut c = c;
            kill_tree(c.id());
            let _ = c.kill();
            let _ = c.wait();
        }
        cleanup_sessions();
    }
}

/// Beendet einen Prozess samt aller Nachfahren.
fn kill_tree(pid: u32) {
    for p in crate::sshd::descendants(pid) {
        // SAFETY: Signal an einen Nachfahren eines selbst gestarteten Prozesses.
        unsafe {
            libc::kill(p as i32, libc::SIGKILL);
        }
    }
}

/// Beendet übrig gebliebene Testinstanzen (erkennbar am Konfigurationspfad
/// im temporären Ordner `rj-xrdp…`).
fn kill_stale_instances() {
    let Ok(rd) = fs::read_dir("/proc") else { return };
    for e in rd.flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<i32>() else { continue };
        let Ok(cmd) = fs::read(e.path().join("cmdline")) else { continue };
        let cmd = String::from_utf8_lossy(&cmd);
        if (cmd.starts_with("/usr/sbin/xrdp-sesman") || cmd.starts_with("/usr/sbin/xrdp\0")) && cmd.contains("rj-xrdp") {
            // SAFETY: übrig gebliebene Testinstanz.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

fn cleanup_sessions() {
    kill_stale_instances();
    let _ = Command::new("pkill").args(["-KILL", "-u", USER2]).status();
    let _ = Command::new("pkill").args(["-KILL", "-u", USER]).status();
    let _ = Command::new("pkill").args(["-KILL", "-f", "xrdp-chansrv"]).status();
    std::thread::sleep(Duration::from_millis(200));
}

impl Drop for Xrdp {
    fn drop(&mut self) {
        self.stop();
    }
}
