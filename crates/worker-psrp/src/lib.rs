//! PowerShell-Remoting über einen lokalen `pwsh` (Voraussetzung auf dem
//! Host). Transport WinRM (HTTPS 5986 / HTTP 5985; Kerberos, NTLM, Basic)
//! oder SSH (PowerShell 7).
//!
//! Das Steuerskript wird als `-EncodedCommand` übergeben und enthält keine
//! Geheimnisse. Das Kennwort folgt als erste Zeile auf der Standardeingabe
//! (Base64 der UTF-8-Bytes, damit es byte-genau bleibt); danach je Zeile ein
//! Befehl (Base64). Die Antworten sind JSON-Zeilen mit getrennten Strömen.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use base64::Engine;
use rj_core::{PsrpTransport, Secret, WinrmAuth};
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum PsError {
    #[error("pwsh nicht gefunden oder nicht startbar ({0}); PowerShell 7 muss auf diesem Rechner installiert sein")]
    NoPwsh(String),
    #[error("PowerShell: {0}")]
    Io(#[from] std::io::Error),
    #[error("Sitzung konnte nicht aufgebaut werden: {0}")]
    Session(String),
    #[error("PowerShell beendet")]
    Ended,
}

/// Ziel einer PowerShell-Sitzung.
#[derive(Debug, Clone)]
pub struct PsTarget {
    pub host: String,
    pub username: String,
    pub domain: Option<String>,
    pub transport: PsrpTransport,
}

fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Steuerskript (ohne Geheimnisse).
pub fn driver_script(t: &PsTarget) -> String {
    let user = match &t.domain {
        Some(d) if !d.is_empty() => format!("{d}\\{}", t.username),
        _ => t.username.clone(),
    };
    let session = match &t.transport {
        PsrpTransport::Winrm { https, port, auth, verify_certificate } => {
            let port = port.unwrap_or(if *https { 5986 } else { 5985 });
            let auth = match auth {
                WinrmAuth::Kerberos => "Kerberos",
                WinrmAuth::Ntlm => "Negotiate",
                WinrmAuth::Basic => "Basic",
            };
            let opts = if *verify_certificate { "New-PSSessionOption".to_string() } else { "New-PSSessionOption -SkipCACheck -SkipCNCheck -SkipRevocationCheck".to_string() };
            format!(
                "$cred = New-Object System.Management.Automation.PSCredential({user}, $sec)\n\
                 $s = New-PSSession -ComputerName {host} -Port {port} {ssl} -Authentication {auth} -Credential $cred -SessionOption ({opts})",
                user = ps_quote(&user),
                host = ps_quote(&t.host),
                ssl = if *https { "-UseSSL" } else { "" },
            )
        }
        PsrpTransport::Ssh { port } => format!(
            "$s = New-PSSession -HostName {host} -UserName {user} -Port {port} -SSHTransport",
            host = ps_quote(&t.host),
            user = ps_quote(&user),
            port = port.unwrap_or(22)
        ),
    };
    format!(
        r#"$ErrorActionPreference = 'Stop'
$ProgressPreference = 'Continue'
function Emit($o) {{ [Console]::Out.WriteLine(($o | ConvertTo-Json -Compress -Depth 3)); [Console]::Out.Flush() }}
$pwB64 = [Console]::In.ReadLine()
$bytes = [Convert]::FromBase64String($pwB64)
$sec = New-Object System.Security.SecureString
foreach ($ch in [Text.Encoding]::UTF8.GetString($bytes).ToCharArray()) {{ $sec.AppendChar($ch) }}
[Array]::Clear($bytes, 0, $bytes.Length); $pwB64 = $null
try {{
{session}
  Emit @{{ ready = $true }}
}} catch {{
  Emit @{{ ready = $false; error = $_.Exception.Message }}
  exit 2
}}
while ($true) {{
  $line = [Console]::In.ReadLine()
  if ($null -eq $line) {{ break }}
  $id, $code = $line.Split(' ', 2)
  $script = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($code))
  $ok = $true
  try {{
    Invoke-Command -Session $s -ScriptBlock ([ScriptBlock]::Create($script)) *>&1 | ForEach-Object {{
      $r = $_
      $kind = switch ($r.GetType().Name) {{
        'ErrorRecord' {{ 'error' }} 'WarningRecord' {{ 'warning' }} 'VerboseRecord' {{ 'verbose' }}
        'DebugRecord' {{ 'debug' }} 'InformationRecord' {{ 'information' }} 'ProgressRecord' {{ 'progress' }}
        default {{ 'output' }} }}
      Emit @{{ id = $id; stream = $kind; text = ($r | Out-String).TrimEnd() }}
    }}
  }} catch {{ $ok = $false; Emit @{{ id = $id; stream = 'error'; text = $_.Exception.Message }} }}
  Emit @{{ id = $id; end = $true; ok = $ok }}
}}
Remove-PSSession $s
"#
    )
}

/// Steuerskript als `-EncodedCommand` (Base64 von UTF-16LE).
pub fn encoded(script: &str) -> String {
    let utf16: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    base64::engine::general_purpose::STANDARD.encode(utf16)
}

/// Ein Eintrag der Ausgabe.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Record {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub stream: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub end: bool,
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub ready: Option<bool>,
    #[serde(default)]
    pub error: Option<String>,
}

pub fn parse_record(line: &str) -> Option<Record> {
    let l = line.trim().trim_start_matches('\u{feff}');
    l.starts_with('{').then(|| serde_json::from_str(l).ok()).flatten()
}

/// Laufende PowerShell-Sitzung.
pub struct PsSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

/// Programm `pwsh` (überschreibbar mit `RJ_PWSH`).
pub fn pwsh_program() -> String {
    std::env::var("RJ_PWSH").unwrap_or_else(|_| if cfg!(windows) { "pwsh.exe".into() } else { "pwsh".into() })
}

impl PsSession {
    /// Startet `pwsh`, übergibt das Kennwort über die Standardeingabe und
    /// wartet auf die Meldung „bereit“.
    pub fn start(t: &PsTarget, password: &Secret) -> Result<PsSession, PsError> {
        let prog = pwsh_program();
        let mut child = Command::new(&prog)
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded(&driver_script(t))])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| PsError::NoPwsh(format!("{prog}: {e}")))?;
        let mut stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        let mut line = base64::engine::general_purpose::STANDARD.encode(password.as_bytes());
        line.push('\n');
        stdin.write_all(line.as_bytes())?;
        stdin.flush()?;
        zeroize::Zeroize::zeroize(&mut line);
        let mut s = PsSession { child, stdin, stdout, next: 1 };
        loop {
            let r = s.read_record()?;
            match r.ready {
                Some(true) => return Ok(s),
                Some(false) => return Err(PsError::Session(r.error.unwrap_or_default())),
                None => continue,
            }
        }
    }

    fn read_record(&mut self) -> Result<Record, PsError> {
        loop {
            let mut l = String::new();
            if self.stdout.read_line(&mut l)? == 0 {
                return Err(PsError::Ended);
            }
            if let Some(r) = parse_record(&l) {
                return Ok(r);
            }
        }
    }

    /// Führt ein Skript im Ziel aus; `on_record` erhält jeden Eintrag.
    /// Liefert, ob das Skript ohne Abbruch durchlief.
    pub fn run(&mut self, script: &str, mut on_record: impl FnMut(&Record)) -> Result<bool, PsError> {
        let id = self.next.to_string();
        self.next += 1;
        let line = format!("{id} {}\n", base64::engine::general_purpose::STANDARD.encode(script.as_bytes()));
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.flush()?;
        loop {
            let r = self.read_record()?;
            if r.id != id {
                continue;
            }
            if r.end {
                return Ok(r.ok);
            }
            on_record(&r);
        }
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for PsSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_variants_without_secrets() {
        let t = PsTarget {
            host: "srv.example.test".into(),
            username: "admin".into(),
            domain: Some("EXAMPLE".into()),
            transport: PsrpTransport::Winrm { https: true, port: None, auth: WinrmAuth::Ntlm, verify_certificate: false },
        };
        let s = driver_script(&t);
        assert!(s.contains("-Port 5986 -UseSSL -Authentication Negotiate"));
        assert!(s.contains("'EXAMPLE\\admin'"));
        assert!(s.contains("-SkipCACheck"));
        let t2 = PsTarget { transport: PsrpTransport::Ssh { port: Some(2222) }, domain: None, ..t.clone() };
        assert!(driver_script(&t2).contains("-HostName 'srv.example.test' -UserName 'admin' -Port 2222 -SSHTransport"));
        let t3 = PsTarget { transport: PsrpTransport::Winrm { https: false, port: None, auth: WinrmAuth::Kerberos, verify_certificate: true }, ..t };
        let s3 = driver_script(&t3);
        assert!(s3.contains("-Port 5985  -Authentication Kerberos"));
        assert!(!s3.contains("SkipCACheck"));
        // Anführungszeichen im Namen werden verdoppelt.
        assert_eq!(ps_quote("o'brien"), "'o''brien'");
    }

    #[test]
    fn records() {
        let r = parse_record(r#"{"id":"3","stream":"warning","text":"Achtung äöü"}"#).unwrap();
        assert_eq!(r.stream, "warning");
        assert!(parse_record("kein JSON").is_none());
        assert_eq!(parse_record(r#"{"ready":false,"error":"Zugriff verweigert"}"#).unwrap().ready, Some(false));
    }
}
