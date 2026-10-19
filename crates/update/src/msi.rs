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
/// Der neue Prozess erhält in `RJ_UPDATE_RESULT` den Rückgabewert von msiexec (0 bzw. 3010: erfolgreich).
pub fn install_after_exit(msi: &Path, exe: &Path, pid: u32) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // Pfade kommen über die Umgebung, nicht in den Befehlstext (keine Anführungszeichen-Fallen).
    const SCRIPT: &str = "$ErrorActionPreference = 'SilentlyContinue'\n\
        Wait-Process -Id ([int]$env:RJ_UPD_PID) -Timeout 120\n\
        $p = Start-Process -FilePath \"$env:SystemRoot\\System32\\msiexec.exe\" -ArgumentList @('/i', ('\"' + $env:RJ_UPD_MSI + '\"'), '/passive', '/norestart') -Wait -PassThru\n\
        $env:RJ_UPDATE_RESULT = [string]$p.ExitCode\n\
        Remove-Item Env:RJ_UPD_PID, Env:RJ_UPD_MSI\n\
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
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|e| UpdateError::Install(format!("Hilfsprozess für msiexec nicht startbar: {e}")))
}

/// Text zum Rückgabewert von msiexec.
pub fn msiexec_result(code: i32) -> std::result::Result<(), String> {
    match code {
        0 | 3010 => Ok(()),
        1602 => Err("Installation abgebrochen".into()),
        1618 => Err("eine andere Installation läuft gerade".into()),
        1638 => Err("eine andere Fassung ist bereits installiert".into()),
        c => Err(format!("msiexec meldet Fehler {c}")),
    }
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

    #[test]
    fn msi_install_dir_is_recognised() {
        assert!(is_msi_dir(Path::new(r"D:\Konten\nutzer\AppData\Local\Programs\remotejoe")));
        assert!(is_msi_dir(Path::new(r"D:\X\PROGRAMS\Remotejoe")));
        assert!(!is_msi_dir(Path::new(r"C:\src\remotejoe\target\release")));
        assert!(!is_msi_dir(Path::new(r"C:\Programs\other")));
    }

    #[test]
    fn msiexec_codes() {
        assert!(msiexec_result(0).is_ok() && msiexec_result(3010).is_ok());
        assert!(msiexec_result(1602).unwrap_err().contains("abgebrochen"));
        assert!(msiexec_result(1603).unwrap_err().contains("1603"));
    }
}
