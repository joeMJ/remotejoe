//! Gegenstellen für Tests: werden auf einem freien Hochport an 127.0.0.1
//! gestartet und beim Freigeben beendet. Fehlt ein Programm oder fehlen
//! Rechte, liefern die Startfunktionen `Err(Skip)` mit Begründung; Tests
//! geben dann einen Hinweis aus und gelten nicht als gegen die echte
//! Gegenstelle bestanden.

pub mod sshd;

use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::time::{Duration, Instant};

/// Grund, warum eine Gegenstelle nicht gestartet werden kann.
#[derive(Debug, Clone)]
pub struct Skip(pub String);

impl std::fmt::Display for Skip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Gegenstelle nicht verfügbar: {}", self.0)
    }
}

/// Freier TCP-Port an 127.0.0.1.
pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).expect("freier Port")
}

/// Wartet, bis ein Port Verbindungen annimmt.
pub fn wait_for_port(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

pub fn is_root() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: geteuid hat keine Vorbedingungen.
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

pub fn have_program(path: &str) -> bool {
    Path::new(path).exists()
}

/// Führt ein Programm aus und liefert Fehlertext bei Misserfolg.
pub fn run(cmd: &str, args: &[&str], stdin: Option<&[u8]>) -> Result<String, Skip> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Skip(format!("{cmd}: {e}")))?;
    if let Some(data) = stdin {
        child.stdin.take().expect("stdin").write_all(data).map_err(|e| Skip(e.to_string()))?;
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().map_err(|e| Skip(e.to_string()))?;
    if !out.status.success() {
        return Err(Skip(format!("{cmd} {args:?}: {}", String::from_utf8_lossy(&out.stderr))));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
