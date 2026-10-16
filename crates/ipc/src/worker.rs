//! Worker-Seite des Kanals über die Standardkanäle.
//!
//! Die Standardausgabe gehört ausschließlich dem Kanal; Worker schreiben
//! Meldungen nur auf die Standardfehlerausgabe.

use std::io::{Stdin, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::frame::{FrameReader, FrameWriter, IpcError};
use crate::msg::{FromWorker, ToWorker};
use crate::PROTOCOL_VERSION;

/// Sendeseite; kann zwischen Fäden geteilt werden.
#[derive(Clone)]
pub struct Sender {
    inner: Arc<Mutex<FrameWriter<Box<dyn Write + Send>>>>,
}

impl Sender {
    pub fn send(&self, msg: &FromWorker) -> Result<(), IpcError> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).send(0, msg)
    }
}

/// Verbindung eines Worker-Prozesses zum Hauptprozess.
pub struct WorkerLink {
    pub sender: Sender,
    pub receiver: FrameReader<Stdin>,
}

impl WorkerLink {
    /// Öffnet den Kanal, meldet sich an und startet den Herzschlag.
    ///
    /// Unter Unix wird der Kanal auf eine Kopie der Standardausgabe gelegt und
    /// die Standardausgabe selbst auf die Fehlerausgabe umgeleitet. So können
    /// Ausgaben eingebundener Bibliotheken den Kanal nicht stören.
    pub fn stdio(kind: &str, heartbeat: Duration) -> Result<WorkerLink, IpcError> {
        let sender = Sender { inner: Arc::new(Mutex::new(FrameWriter::new(protected_stdout()?))) };
        sender.send(&FromWorker::Hello {
            pid: std::process::id(),
            kind: kind.to_string(),
            protocol_version: PROTOCOL_VERSION,
        })?;
        let hb = sender.clone();
        thread::Builder::new()
            .name("herzschlag".into())
            .spawn(move || {
                let mut seq = 0u64;
                loop {
                    thread::sleep(heartbeat);
                    seq += 1;
                    if hb.send(&FromWorker::Heartbeat { seq }).is_err() {
                        // Hauptprozess weg: Worker beendet sich.
                        std::process::exit(0);
                    }
                }
            })
            .map_err(IpcError::Io)?;
        Ok(WorkerLink { sender, receiver: FrameReader::new(std::io::stdin()) })
    }

    pub fn recv(&mut self) -> Result<ToWorker, IpcError> {
        self.receiver.recv().map(|(_, m)| m)
    }
}

#[cfg(unix)]
fn protected_stdout() -> Result<Box<dyn Write + Send>, IpcError> {
    use std::os::fd::FromRawFd;
    // SAFETY: dup/dup2 auf die Standardkanäle des eigenen Prozesses.
    unsafe {
        let fd = libc::dup(1);
        if fd < 0 {
            return Err(IpcError::Io(std::io::Error::last_os_error()));
        }
        libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        if libc::dup2(2, 1) < 0 {
            return Err(IpcError::Io(std::io::Error::last_os_error()));
        }
        Ok(Box::new(std::fs::File::from_raw_fd(fd)))
    }
}

#[cfg(not(unix))]
fn protected_stdout() -> Result<Box<dyn Write + Send>, IpcError> {
    Ok(Box::new(std::io::stdout()))
}
