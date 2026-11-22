//! Kennwort für den SSH-Transport von pwsh (nur Unix).
//!
//! pwsh ruft `ssh` auf und reicht kein Kennwort weiter. remotejoe setzt deshalb `SSH_ASKPASS` auf das
//! Hilfsprogramm `rj-askpass` (`SSH_ASKPASS_REQUIRE=force`). Das Hilfsprogramm holt das Kennwort über einen
//! privaten Unix-Socket (Ordner mit Rechten 0700, nur für diese Sitzung) und gibt es nur auf eine echte
//! Kennwortfrage aus – nie auf eine Host-Schlüssel-Bestätigung oder eine Passphrase. Kein Kennwort auf der
//! Befehlszeile oder in Umgebungsvariablen; der Socket schließt, sobald die Sitzung steht.

use std::io::{Read, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use zeroize::Zeroize;

/// Umgebungsvariable mit dem Pfad des Sockets.
pub const SOCKET_ENV: &str = "RJ_ASKPASS_SOCKET";
/// So oft wird das Kennwort höchstens herausgegeben (Kennwort, ggf. tastatur-interaktiv, ein Neuversuch).
const MAX_SERVES: usize = 3;

/// Ist das eine Kennwortfrage von ssh (und keine Host-Schlüssel-Bestätigung oder Passphrase)?
pub fn is_password_prompt(prompt: &str) -> bool {
    let l = prompt.trim().to_lowercase();
    l.ends_with("password:") && !l.contains("passphrase") && !l.contains("yes/no")
}

/// Hilfsprogramm neben dem laufenden Programm (`rj-askpass`).
pub fn helper() -> Option<PathBuf> {
    let p = std::env::current_exe().ok()?.parent()?.join("rj-askpass");
    p.is_file().then_some(p)
}

/// Gibt das Kennwort über den Socket heraus, solange die Sitzung aufgebaut wird.
pub struct Server {
    dir: PathBuf,
    socket: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    pub fn start(password: &[u8]) -> std::io::Result<Server> {
        let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_dir()).unwrap_or_else(std::env::temp_dir);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
        let dir = base.join(format!("rj-askpass-{}-{nanos:08x}", std::process::id()));
        // Neu anlegen (scheitert, wenn es ihn schon gibt), nur für den eigenen Benutzer.
        std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
        let socket = dir.join("s");
        let listener = UnixListener::bind(&socket)?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let (stop2, mut secret) = (stop.clone(), password.to_vec());
        let thread = std::thread::Builder::new().name("rj-askpass".into()).spawn(move || {
            let mut served = 0;
            while served < MAX_SERVES && !stop2.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut s, _)) => {
                        let _ = s.set_nonblocking(false);
                        let _ = s.write_all(&secret);
                        served += 1;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(30)),
                    Err(_) => break,
                }
            }
            secret.zeroize();
        })?;
        Ok(Server { dir, socket, stop, thread: Some(thread) })
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Teil des Hilfsprogramms: Kennwort vom Socket holen (nur auf eine Kennwortfrage).
pub fn fetch(prompt: &str, socket: &Path) -> Option<Vec<u8>> {
    if !is_password_prompt(prompt) {
        return None;
    }
    let mut s = UnixStream::connect(socket).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).ok()?;
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_password_prompts_are_answered() {
        assert!(is_password_prompt("admin@192.0.2.5's password: "));
        assert!(is_password_prompt("Password:"));
        assert!(!is_password_prompt("Are you sure you want to continue connecting (yes/no/[fingerprint])? "));
        assert!(!is_password_prompt("Enter passphrase for key '/home/x/.ssh/id_ed25519': "));
        assert!(!is_password_prompt("Verification code: "));
    }

    #[test]
    fn socket_hands_out_the_password_and_is_removed_afterwards() {
        let s = Server::start(b"geheim \xc3\xa4").unwrap();
        let path = s.socket().to_path_buf();
        let mode = std::fs::metadata(path.parent().unwrap()).unwrap();
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&mode.permissions()) & 0o777, 0o700);
        assert_eq!(fetch("x@y's password:", &path).as_deref(), Some(&b"geheim \xc3\xa4"[..]));
        assert_eq!(fetch("Are you sure (yes/no)?", &path), None, "keine Antwort auf Bestätigungen");
        drop(s);
        assert!(!path.exists() && !path.parent().unwrap().exists());
    }
}
