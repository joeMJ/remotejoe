//! Sperre je Schlüsseldatenbank: Solange eine remotejoe-Instanz die Datei geöffnet
//! hat, hält sie eine Sperrdatei neben der KDBX. Eine zweite Instanz (auch mit
//! anderem Profil) kann dieselbe KDBX dann nicht öffnen und nicht überschreiben.
//! Die Sperre liegt im Betriebssystem und fällt bei einem Absturz von selbst weg.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct VaultLock {
    _file: File,
    path: PathBuf,
}

impl VaultLock {
    /// Name der Sperrdatei zu einer KDBX: `.<Dateiname>.remotejoe-lock`.
    pub fn path_for(kdbx: &Path) -> PathBuf {
        let name = kdbx.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "kdbx".into());
        kdbx.with_file_name(format!(".{name}.remotejoe-lock"))
    }

    /// Sperrt die KDBX für diese Instanz. Fehler: von einer anderen Instanz belegt
    /// oder Sperrdatei nicht anlegbar.
    pub fn acquire(kdbx: &Path) -> Result<VaultLock, String> {
        let path = Self::path_for(kdbx);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| format!("Sperrdatei „{}“ nicht anlegbar: {e}", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(VaultLock { _file: file, path }),
            Err(TryLockError::WouldBlock) => Err(format!(
                "Die Schlüsseldatenbank „{}“ ist bereits in einer anderen remotejoe-Instanz geöffnet.",
                kdbx.display()
            )),
            Err(TryLockError::Error(e)) => Err(format!("Sperre „{}“: {e}", path.display())),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_lock_is_refused_until_first_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let kdbx = dir.path().join("a.kdbx");
        let first = VaultLock::acquire(&kdbx).unwrap();
        let err = VaultLock::acquire(&kdbx).unwrap_err();
        assert!(err.contains("bereits"), "{err}");
        assert!(VaultLock::acquire(&dir.path().join("b.kdbx")).is_ok(), "andere Datei bleibt frei");
        drop(first);
        assert!(VaultLock::acquire(&kdbx).is_ok());
    }

    #[test]
    fn lock_file_sits_next_to_kdbx() {
        let p = VaultLock::path_for(Path::new("/x/y/db.kdbx"));
        assert_eq!(p, Path::new("/x/y/.db.kdbx.remotejoe-lock"));
    }
}
