//! Windows: Update über die MSI der Veröffentlichung. Die MSI wird wie unter Linux das Paket über die signierte
//! `SHA256SUMS` geprüft, dazu die Authenticode-Signatur ([`crate::authenticode`]). Installiert wird erst, wenn
//! remotejoe beendet ist: Ein unsichtbarer Hilfsprozess wartet auf das Ende, ruft `msiexec /i … /passive` auf
//! (pro Benutzer, ohne Administratorrechte) und startet remotejoe danach neu.

use std::path::{Path, PathBuf};

use crate::release::{Found, Release};
use crate::sign::{SIG, SUMS};
use crate::{Result, UpdateError, Version};

/// Dateiname der MSI einer Version.
pub fn msi_name(version: &str) -> String {
    format!("remotejoe-{version}-windows-x86_64.msi")
}

/// Wählt MSI, Prüfsummen und Signatur aus einer Veröffentlichung.
pub fn select_msi(release: &Release) -> Result<Found> {
    let version = Version::parse(&release.tag_name)?;
    let want = msi_name(&version.to_string());
    let find = |n: &str| release.assets.iter().find(|a| a.name == n).cloned();
    let archive = find(&want).ok_or_else(|| UpdateError::NoAsset("windows-x86_64 (MSI)".into()))?;
    let sums = find(SUMS).ok_or_else(|| UpdateError::MissingFile(SUMS.into()))?;
    let sig = find(SIG).ok_or_else(|| UpdateError::MissingFile(SIG.into()))?;
    Ok(Found { release: release.clone(), version, archive, sums, sig })
}

/// Ist das Programmverzeichnis die Installation der MSI (`…\Programs\remotejoe`, pro Benutzer)?
pub fn is_msi_dir(dir: &Path) -> bool {
    let mut c = dir.components().rev().map(|c| c.as_os_str().to_string_lossy().to_lowercase());
    c.next().as_deref() == Some("remotejoe") && c.next().as_deref() == Some("programs")
}

/// Ablage für die geladene MSI (wird bei jedem Update überschrieben).
pub fn download_dir() -> PathBuf {
    std::env::temp_dir().join("remotejoe-update")
}

/// Startet den Hilfsprozess, der nach dem Ende von Prozess `pid` die MSI installiert und `exe` neu startet.
/// Der neue Prozess erhält in `RJ_UPDATE_RESULT` den Rückgabewert von msiexec (0 bzw. 3010: erfolgreich) und in
/// `RJ_UPDATE_LOG` den Pfad des msiexec-Protokolls `log`. Ist Windows Installer gerade beschäftigt (eine andere
/// Installation läuft), wartet der Hilfsprozess und versucht es selbst erneut – vor dem Start und bei Rückgabe 1618 –,
/// höchstens [`WAIT_MINUTES`] Minuten lang.
#[cfg(windows)]
pub fn install_after_exit(msi: &Path, exe: &Path, pid: u32, log: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // Pfade kommen über die Umgebung, nicht in den Befehlstext (keine Anführungszeichen-Fallen).
    const SCRIPT: &str = "$ErrorActionPreference = 'SilentlyContinue'\n\
        function Busy { $m = $null; try { if ([System.Threading.Mutex]::TryOpenExisting('Global\\_MSIExecute', [ref]$m)) { $m.Dispose(); return $true } } catch [System.UnauthorizedAccessException] { return $true } catch { }; return $false }\n\
        Wait-Process -Id ([int]$env:RJ_UPD_PID) -Timeout 120\n\
        $deadline = (Get-Date).AddMinutes([int]$env:RJ_UPD_WAIT)\n\
        while ((Busy) -and ((Get-Date) -lt $deadline)) { Start-Sleep -Seconds 5 }\n\
        do {\n\
          $p = Start-Process -FilePath \"$env:SystemRoot\\System32\\msiexec.exe\" -ArgumentList @('/i', ('\"' + $env:RJ_UPD_MSI + '\"'), '/passive', '/norestart', '/l*v+', ('\"' + $env:RJ_UPD_LOG + '\"')) -Wait -PassThru\n\
          $code = $p.ExitCode\n\
          if ($code -ne 1618) { break }\n\
          Start-Sleep -Seconds 10\n\
        } while ((Get-Date) -lt $deadline)\n\
        $env:RJ_UPDATE_RESULT = [string]$code\n\
        $env:RJ_UPDATE_LOG = $env:RJ_UPD_LOG\n\
        Remove-Item Env:RJ_UPD_PID, Env:RJ_UPD_MSI, Env:RJ_UPD_LOG, Env:RJ_UPD_WAIT\n\
        $exe = $env:RJ_UPD_EXE; Remove-Item Env:RJ_UPD_EXE\n\
        Start-Process -FilePath $exe\n";
    use base64::Engine;
    let utf16: Vec<u8> = SCRIPT.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    let ps = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows")).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    std::process::Command::new(ps)
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-EncodedCommand", &encoded])
        .env("RJ_UPD_PID", pid.to_string())
        .env("RJ_UPD_MSI", msi)
        .env("RJ_UPD_EXE", exe)
        .env("RJ_UPD_LOG", log)
        .env("RJ_UPD_WAIT", WAIT_MINUTES.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|e| UpdateError::Install(format!("Hilfsprozess für msiexec nicht startbar: {e}")))
}

/// Wie lange auf eine andere Installation gewartet wird (Minuten), vor dem Update und im Hilfsprozess.
pub const WAIT_MINUTES: u32 = 10;

/// Läuft gerade eine andere Installation über Windows Installer (benannter Mutex `Global\_MSIExecute`)? Dann würde
/// msiexec mit 1618 abbrechen. Ein Mutex ohne Zugriffsrecht zählt als beschäftigt.
#[cfg(windows)]
pub fn installer_busy() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED};
    use windows_sys::Win32::System::Threading::OpenMutexW;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    let name: Vec<u16> = "Global\\_MSIExecute\0".encode_utf16().collect();
    // SAFETY: nullterminierter Name; ein geöffneter Griff wird sofort geschlossen.
    unsafe {
        let h = OpenMutexW(SYNCHRONIZE, 0, name.as_ptr());
        if h.is_null() {
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        CloseHandle(h);
        true
    }
}

#[cfg(not(windows))]
pub fn installer_busy() -> bool {
    false
}

/// Ablage der msiexec-Protokolle (`<Datenordner>/update`): Pfad für ein neues Protokoll der Version `version`
/// (Zeitstempel im Namen); ältere werden bis auf die letzten `keep` entfernt.
pub fn new_log_path(dir: &Path, version: &str, keep: usize) -> PathBuf {
    let _ = std::fs::create_dir_all(dir);
    let mut old: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("msiexec-") && n.ends_with(".log"))).collect())
        .unwrap_or_default();
    // Namen enthalten den Zeitstempel (Sekunden seit 1970, gleich lang): alphabetisch = zeitlich.
    old.sort();
    let drop = old.len().saturating_sub(keep.saturating_sub(1));
    for p in old.iter().take(drop) {
        let _ = std::fs::remove_file(p);
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    dir.join(format!("msiexec-{secs:012}-{version}.log"))
}

/// Verständlicher Text zu einem Rückgabewert von msiexec, mit Handlungshinweis; `None` bei Erfolg (0, 3010).
/// `log`: Protokoll des Laufs, wird bei Fehlern genannt.
pub fn msiexec_text(code: i32, log: Option<&Path>) -> Option<String> {
    let text = match code {
        0 | 3010 => return None,
        1601 => "Der Dienst Windows Installer ist nicht erreichbar (1601). Windows neu starten und das Update erneut ausführen.".to_string(),
        1602 => "Die Installation wurde abgebrochen (1602).".to_string(),
        1603 => "Bei der Installation ist ein schwerer Fehler aufgetreten (1603). Ursache steht im Protokoll; oft hilft es, Windows neu zu starten und das Update erneut auszuführen.".to_string(),
        1605 => "Windows Installer kennt die installierte Fassung nicht (1605). remotejoe mit der MSI von der Projektseite neu installieren.".to_string(),
        1612 | 1619 | 1620 => format!("Das Installationspaket ist nicht lesbar oder fehlt ({code}). Das Update erneut herunterladen und installieren."),
        1615 => "Die Installationsdatenbank des Pakets ist fehlerhaft (1615). Das Update erneut herunterladen; tritt der Fehler wieder auf, bitte mit dem Protokoll melden.".to_string(),
        1618 => format!("Eine andere Installation lief noch nach {WAIT_MINUTES} Minuten Wartezeit (1618). Warten, bis sie fertig ist, dann das Update erneut ausführen."),
        1625 | 1643 => format!("Die Installation ist durch eine Richtlinie von Windows verboten ({code}). Bitte die IT bzw. den Administrator fragen."),
        1638 => "Eine andere Fassung von remotejoe ist bereits installiert (1638). Über „Apps“ in den Windows-Einstellungen entfernen und die MSI neu installieren.".to_string(),
        1642 => "Das Update passt nicht zur installierten Fassung (1642). remotejoe mit der MSI von der Projektseite neu installieren.".to_string(),
        c => format!("Windows-Installer-Fehler {c}."),
    };
    Some(match log {
        Some(l) => format!("{text}\nProtokoll: {}", l.display()),
        None => text,
    })
}

/// Rückgabewert von msiexec als Ergebnis (Text wie [`msiexec_text`], ohne Protokoll).
pub fn msiexec_result(code: i32) -> std::result::Result<(), String> {
    msiexec_text(code, None).map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REL: &str = r#"{"tag_name":"v0.2.0","assets":[
{"name":"remotejoe-0.2.0-windows-x86_64.msi","size":5,"browser_download_url":"https://github.com/a/b/releases/download/v0.2.0/remotejoe-0.2.0-windows-x86_64.msi"},
{"name":"remotejoe-0.2.0-linux-x86_64.tar.zst","size":5,"browser_download_url":"https://github.com/a/b/x"},
{"name":"SHA256SUMS","size":1,"browser_download_url":"https://github.com/s"},{"name":"SHA256SUMS.sig","size":1,"browser_download_url":"https://github.com/t"}]}"#;

    #[test]
    fn msi_is_selected_with_sums_and_signature() {
        let r = Release::parse(REL.as_bytes()).unwrap();
        let f = select_msi(&r).unwrap();
        assert_eq!(f.archive.name, "remotejoe-0.2.0-windows-x86_64.msi");
        assert_eq!(f.version.to_string(), "0.2.0");
        let mut r2 = r.clone();
        r2.assets.retain(|a| !a.name.ends_with(".msi"));
        assert!(matches!(select_msi(&r2), Err(UpdateError::NoAsset(_))));
        r2 = r.clone();
        r2.assets.retain(|a| a.name != "SHA256SUMS.sig");
        assert!(matches!(select_msi(&r2), Err(UpdateError::MissingFile(_))));
    }

    /// Windows-Pfade (nur unter Windows zerlegbar).
    #[cfg(windows)]
    #[test]
    fn msi_install_dir_is_recognised() {
        assert!(is_msi_dir(Path::new(r"D:\Konten\nutzer\AppData\Local\Programs\remotejoe")));
        assert!(is_msi_dir(Path::new(r"D:\X\PROGRAMS\Remotejoe")));
        assert!(!is_msi_dir(Path::new(r"C:\src\remotejoe\target\release")));
        assert!(!is_msi_dir(Path::new(r"C:\Programs\other")));
    }

    /// Jeder häufige Code hat einen eigenen Text mit Code und Hinweis; unbekannte Codes als Windows-Installer-Fehler;
    /// das Protokoll wird genannt.
    #[test]
    fn msiexec_codes() {
        assert!(msiexec_result(0).is_ok() && msiexec_result(3010).is_ok());
        assert!(msiexec_result(1602).unwrap_err().contains("abgebrochen"));
        let cases = [
            (1601, "Dienst Windows Installer"),
            (1603, "schwerer Fehler"),
            (1605, "kennt die installierte Fassung nicht"),
            (1612, "nicht lesbar oder fehlt"),
            (1615, "Installationsdatenbank"),
            (1618, "andere Installation"),
            (1619, "nicht lesbar oder fehlt"),
            (1620, "nicht lesbar oder fehlt"),
            (1625, "Richtlinie"),
            (1638, "andere Fassung"),
            (1642, "passt nicht zur installierten Fassung"),
            (1643, "Richtlinie"),
        ];
        for (code, text) in cases {
            let t = msiexec_result(code).unwrap_err();
            assert!(t.contains(text) && t.contains(&format!("({code})")), "{code}: {t}");
        }
        assert_eq!(msiexec_result(2203).unwrap_err(), "Windows-Installer-Fehler 2203.");
        let t = msiexec_text(1603, Some(Path::new(r"C:\Daten\update\msiexec-1.log"))).unwrap();
        assert!(t.ends_with(r"Protokoll: C:\Daten\update\msiexec-1.log"), "{t}");
        assert!(!installer_busy() || cfg!(windows));
    }

    /// Protokolle: neuer Name mit Zeitstempel und Version; nur die letzten bleiben (zusammen mit dem neuen).
    #[test]
    fn msiexec_logs_are_rotated() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..7 {
            std::fs::write(d.path().join(format!("msiexec-{i:012}-0.1.66.log")), b"x").unwrap();
        }
        std::fs::write(d.path().join("anderes.txt"), b"x").unwrap();
        let p = new_log_path(d.path(), "0.1.67", 5);
        assert!(p.file_name().unwrap().to_str().unwrap().ends_with("-0.1.67.log"));
        let left: Vec<String> = std::fs::read_dir(d.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left.iter().filter(|n| n.starts_with("msiexec-")).count(), 4, "{left:?}");
        assert!(left.contains(&"msiexec-000000000006-0.1.66.log".to_string()) && left.contains(&"anderes.txt".to_string()));
    }
}
