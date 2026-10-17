//! Installation: Paket in ein Nachbarverzeichnis entpacken, dann das
//! Programmverzeichnis ersetzen; das bisherige bleibt als `<name>.alt`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::{archive, Result, UpdateError};

/// Wie installiert werden kann.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Programmverzeichnis selbst ersetzen.
    Replace { dir: PathBuf },
    /// Von der Paketverwaltung installiert (unter `/usr`): nur Hinweis, keine Selbstveränderung.
    PackageManaged { dir: PathBuf },
    /// Entwicklungsstand (`target/release` oder `target/debug` eines Arbeitsbereichs): wird nie
    /// ersetzt, sonst ginge der Bauordner verloren.
    Development { dir: PathBuf },
}

/// Plan für das Verzeichnis des laufenden Programms.
pub fn plan(program_dir: &Path) -> Plan {
    let parts: Vec<_> = program_dir.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    if parts.windows(2).any(|w| w[0] == "target" && (w[1] == "release" || w[1] == "debug")) {
        return Plan::Development { dir: program_dir.to_path_buf() };
    }
    if cfg!(unix) && program_dir.starts_with("/usr") {
        Plan::PackageManaged { dir: program_dir.to_path_buf() }
    } else {
        Plan::Replace { dir: program_dir.to_path_buf() }
    }
}

/// Name des Hauptprogramms im Paket.
pub fn main_program() -> &'static str {
    if cfg!(windows) {
        "remotejoe.exe"
    } else {
        "remotejoe"
    }
}

fn sibling(dir: &Path, suffix: &str) -> Result<PathBuf> {
    let name = dir.file_name().ok_or_else(|| UpdateError::Install(format!("„{}“ hat keinen Namen", dir.display())))?;
    let mut n = name.to_os_string();
    n.push(suffix);
    Ok(dir.with_file_name(n))
}

/// Installiert ein bereits geprüftes Archiv nach `dir`. Gibt das Verzeichnis
/// der bisherigen Fassung (`.alt`) zurück.
pub fn install(archive_name: &str, data: &[u8], dir: &Path) -> Result<PathBuf> {
    let staging = sibling(dir, ".neu")?;
    let old = sibling(dir, ".alt")?;
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    if let Err(e) = archive::unpack(archive_name, data, &staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }
    if !staging.join(main_program()).is_file() {
        let _ = fs::remove_dir_all(&staging);
        return Err(UpdateError::Install(format!("das Paket enthält kein Hauptprogramm „{}“", main_program())));
    }
    if old.exists() {
        fs::remove_dir_all(&old)?;
    }
    fs::rename(dir, &old).map_err(|e| UpdateError::Install(format!("bisherige Fassung nicht verschiebbar: {e}")))?;
    if let Err(e) = fs::rename(&staging, dir) {
        // Zurückrollen: bisherige Fassung wieder an ihren Platz.
        let _ = fs::rename(&old, dir);
        return Err(UpdateError::Install(format!("neue Fassung nicht einsetzbar: {e}")));
    }
    Ok(old)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_detects_package_manager() {
        if cfg!(unix) {
            assert!(matches!(plan(Path::new("/usr/lib/remotejoe")), Plan::PackageManaged { .. }));
            assert!(matches!(plan(Path::new("/home/nutzer/remotejoe")), Plan::Replace { .. }));
            assert!(matches!(plan(Path::new("/home/nutzer/.local/share/remotejoe")), Plan::Replace { .. }));
        }
        assert!(matches!(plan(Path::new("/home/nutzer/projekt/target/release")), Plan::Development { .. }));
        assert!(matches!(plan(Path::new("/home/nutzer/projekt/target/debug")), Plan::Development { .. }));
        assert!(matches!(plan(Path::new("/home/nutzer/target-ordner/remotejoe")), Plan::Replace { .. }));
    }

    #[test]
    fn install_replaces_and_keeps_old() {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("remotejoe");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join(main_program()), b"alt").unwrap();
        let src = tempfile::tempdir().unwrap();
        fs::write(src.path().join(main_program()), b"neu").unwrap();
        let data = archive::pack_tar_zst(src.path()).unwrap();
        let old = install("p.tar.zst", &data, &dir).unwrap();
        assert_eq!(fs::read(dir.join(main_program())).unwrap(), b"neu");
        assert_eq!(fs::read(old.join(main_program())).unwrap(), b"alt");
        // Paket ohne Hauptprogramm: nichts verändert.
        let empty = tempfile::tempdir().unwrap();
        fs::write(empty.path().join("anderes"), b"x").unwrap();
        let data = archive::pack_tar_zst(empty.path()).unwrap();
        assert!(install("p.tar.zst", &data, &dir).is_err());
        assert_eq!(fs::read(dir.join(main_program())).unwrap(), b"neu");
    }
}
