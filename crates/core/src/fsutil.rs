//! Sicheres Schreiben von Dateien: Sicherungskopie, atomares Ersetzen, Sperre.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Pfad der Sicherungskopie (`<datei>.bak`).
pub fn backup_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".bak");
    PathBuf::from(s)
}

/// Pfad der Sperrdatei (`<datei>.lock`).
pub fn lock_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".lock");
    PathBuf::from(s)
}

/// Windows: Datei kurz von einem anderen Prozess geöffnet (Virenscanner, Indexdienst): Zugriff verweigert (5),
/// Sharing-Verletzung (32) oder Sperr-Verletzung (33). Solche Fehler gehen meist nach Millisekunden vorbei.
fn is_sharing_violation(e: &io::Error) -> bool {
    cfg!(windows) && matches!(e.raw_os_error(), Some(5) | Some(32) | Some(33))
}

/// Wiederholt `f`, solange `transient` den Fehler als vorübergehend einstuft, höchstens `total` lang.
fn retry_while<T>(mut f: impl FnMut() -> io::Result<T>, transient: impl Fn(&io::Error) -> bool, total: std::time::Duration) -> io::Result<T> {
    let deadline = std::time::Instant::now() + total;
    loop {
        match f() {
            Err(e) if transient(&e) && std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(25)),
            r => return r,
        }
    }
}

/// Exklusive Sperre gegen gleichzeitiges Schreiben. Wird beim Freigeben gelöst.
#[derive(Debug)]
pub struct WriteLock {
    file: File,
    path: PathBuf,
}

impl WriteLock {
    /// Versucht die Sperre zu bekommen, ohne zu warten.
    pub fn try_acquire(target: &Path) -> io::Result<WriteLock> {
        let path = lock_path(target);
        let file = OpenOptions::new().create(true).truncate(false).write(true).open(&path)?;
        match file.try_lock() {
            Ok(()) if Self::still_current(&file, &path) => Ok(WriteLock { file, path }),
            Ok(()) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                format!("Datei wird gerade von einem anderen Vorgang geschrieben: {}", target.display()),
            )),
            Err(fs::TryLockError::WouldBlock) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                format!("Datei wird gerade von einem anderen Vorgang geschrieben: {}", target.display()),
            )),
            Err(fs::TryLockError::Error(e)) => Err(e),
        }
    }
}

impl WriteLock {
    /// Wartet bis zu `timeout` auf die Sperre (für kurze Schreibvorgänge
    /// mehrerer Prozesse, z. B. gleichzeitig angenommene Host-Schlüssel).
    pub fn acquire_wait(target: &Path, timeout: std::time::Duration) -> io::Result<WriteLock> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match Self::try_acquire(target) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                r => return r,
            }
        }
    }

    /// Die gesperrte Datei ist noch die unter dem Pfad (nicht inzwischen vom
    /// vorigen Halter entfernt und neu angelegt).
    fn still_current(file: &File, path: &Path) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            match (file.metadata(), fs::metadata(path)) {
                (Ok(a), Ok(b)) => a.ino() == b.ino() && a.dev() == b.dev(),
                _ => false,
            }
        }
        #[cfg(not(unix))]
        {
            // Unter Windows lässt sich eine geöffnete Datei nicht entfernen.
            let _ = file;
            path.exists()
        }
    }
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
        // Unter Windows bleibt die Sperrdatei liegen: Eine gelöschte, aber noch von einem Wartenden geöffnete
        // Datei lässt zwei Schreiber gleichzeitig die Sperre bekommen (verschiedene „Generationen“ derselben Datei).
        #[cfg(not(windows))]
        let _ = fs::remove_file(&self.path);
    }
}

/// Schreibt `data` atomar nach `path`: vorher Sicherungskopie (eine
/// Generation), dann temporäre Datei im selben Ordner, `fsync`, Umbenennen.
pub fn atomic_write_with_backup(path: &Path, data: &[u8]) -> io::Result<()> {
    let _lock = WriteLock::try_acquire(path)?;
    atomic_write_locked(path, data)
}

/// Wie [`atomic_write_with_backup`], aber der Aufrufer hält die Sperre bereits.
pub fn atomic_write_locked(path: &Path, data: &[u8]) -> io::Result<()> {
    let patience = std::time::Duration::from_secs(3);
    if path.exists() {
        retry_while(|| fs::copy(path, backup_path(path)), is_sharing_violation, patience)?;
    }
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let file_name = path.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "kein Dateiname"))?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(file_name);
    tmp_name.push(format!(".tmp{}", std::process::id()));
    let tmp = dir.join(tmp_name);
    {
        let mut f = retry_while(|| File::create(&tmp), is_sharing_violation, patience)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    // Ein Virenscanner oder Indexdienst hält die frisch geschriebene Datei oft kurz offen (Windows, Fehler 32).
    if let Err(e) = retry_while(|| fs::rename(&tmp, path), is_sharing_violation, patience) {
        let _ = retry_while(|| fs::remove_file(&tmp), is_sharing_violation, patience);
        return Err(e);
    }
    #[cfg(unix)]
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_with_backup() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("daten.bin");
        atomic_write_with_backup(&p, b"eins").unwrap();
        assert!(!backup_path(&p).exists());
        atomic_write_with_backup(&p, b"zwei").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"zwei");
        assert_eq!(fs::read(backup_path(&p)).unwrap(), b"eins");
        atomic_write_with_backup(&p, b"drei").unwrap();
        assert_eq!(fs::read(backup_path(&p)).unwrap(), b"zwei");
        // Keine temporären Reste.
        // Unter Windows bleibt die Sperrdatei liegen.
        let files = fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok()).filter(|e| !e.file_name().to_string_lossy().ends_with(".lock")).count();
        assert_eq!(files, 2);
    }

    #[test]
    fn transient_errors_are_retried_until_they_pass_but_others_fail_at_once() {
        let mut n = 0;
        let r = retry_while(
            || {
                n += 1;
                if n < 3 { Err(io::Error::from_raw_os_error(32)) } else { Ok(n) }
            },
            |e| e.raw_os_error() == Some(32),
            std::time::Duration::from_secs(2),
        );
        assert_eq!(r.unwrap(), 3);
        // Anderer Fehler: sofort, ohne Wiederholung.
        let mut calls = 0;
        let r: io::Result<()> = retry_while(|| { calls += 1; Err(io::Error::from(io::ErrorKind::NotFound)) }, |e| e.raw_os_error() == Some(32), std::time::Duration::from_secs(2));
        assert!(r.is_err());
        assert_eq!(calls, 1);
        // Dauerhafter vorübergehender Fehler: gibt nach der Frist auf.
        let t0 = std::time::Instant::now();
        let r: io::Result<()> = retry_while(|| Err(io::Error::from_raw_os_error(32)), |e| e.raw_os_error() == Some(32), std::time::Duration::from_millis(200));
        assert!(r.is_err() && t0.elapsed() >= std::time::Duration::from_millis(200));
    }

    #[test]
    fn lock_is_exclusive() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x");
        let l = WriteLock::try_acquire(&p).unwrap();
        let e = WriteLock::try_acquire(&p).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::WouldBlock);
        drop(l);
        WriteLock::try_acquire(&p).unwrap();
    }

    #[test]
    fn waiting_lock_keeps_all_concurrent_updates() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("liste");
        let threads: Vec<_> = (0..20)
            .map(|i| {
                let p = p.clone();
                std::thread::spawn(move || {
                    let _l = WriteLock::acquire_wait(&p, std::time::Duration::from_secs(10)).unwrap();
                    let mut t = fs::read_to_string(&p).unwrap_or_default();
                    t.push_str(&format!("eintrag-{i}\n"));
                    atomic_write_locked(&p, t.as_bytes()).unwrap();
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(fs::read_to_string(&p).unwrap().lines().count(), 20);
    }
}
