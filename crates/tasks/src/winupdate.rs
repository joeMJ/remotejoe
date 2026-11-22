//! Windows-Aktualisierungen über die COM-Schnittstelle
//! `Microsoft.Update.Session` in einem vom Programm gesendeten Skript (kein
//! Modul auf dem Ziel nötig). Das Skript meldet Fortschritt als JSON-Zeilen.

use serde::{Deserialize, Serialize};

/// Skript für das Ziel. `install = false` ist der Trockenlauf.
pub fn script(install: bool) -> String {
    format!(
        r#"$ErrorActionPreference = 'Stop'
function Emit($o) {{ [Console]::Out.WriteLine(($o | ConvertTo-Json -Compress -Depth 4)) }}
try {{
  $session = New-Object -ComObject Microsoft.Update.Session
  $session.ClientApplicationID = 'remotejoe'
  Emit @{{ type = 'phase'; phase = 'searching' }}
  $result = $session.CreateUpdateSearcher().Search("IsInstalled=0 and IsHidden=0 and Type='Software'")
  $total = $result.Updates.Count
  for ($i = 0; $i -lt $total; $i++) {{
    $u = $result.Updates.Item($i)
    $kb = ($u.KBArticleIDs | ForEach-Object {{ "KB$_" }}) -join ','
    Emit @{{ type = 'found'; title = $u.Title; kb = $kb; size = [int64]$u.MaxDownloadSize; reboot = [bool]($u.InstallationBehavior.RebootBehavior -ne 0) }}
  }}
  $installed = 0; $failed = 0
  if ({install} -and $total -gt 0) {{
    for ($i = 0; $i -lt $total; $i++) {{
      $u = $result.Updates.Item($i)
      if (-not $u.EulaAccepted) {{ $u.AcceptEula() }}
      $coll = New-Object -ComObject Microsoft.Update.UpdateColl
      [void]$coll.Add($u)
      Emit @{{ type = 'progress'; phase = 'downloading'; index = $i + 1; total = $total; title = $u.Title }}
      $d = $session.CreateUpdateDownloader(); $d.Updates = $coll; [void]$d.Download()
      Emit @{{ type = 'progress'; phase = 'installing'; index = $i + 1; total = $total; title = $u.Title }}
      $inst = $session.CreateUpdateInstaller(); $inst.Updates = $coll
      $r = $inst.Install()
      if ($r.ResultCode -eq 2 -or $r.ResultCode -eq 3) {{ $installed++ }} else {{ $failed++; Emit @{{ type = 'error'; title = $u.Title; code = [int]$r.ResultCode }} }}
    }}
  }}
  $reboot = (New-Object -ComObject Microsoft.Update.SystemInfo).RebootRequired
  Emit @{{ type = 'result'; found = $total; installed = $installed; failed = $failed; reboot_required = [bool]$reboot }}
}} catch {{
  Emit @{{ type = 'fatal'; message = $_.Exception.Message }}
}}
"#,
        install = if install { "$true" } else { "$false" }
    )
}

/// Ereignis aus der Ausgabe des Skripts.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WuEvent {
    Phase { phase: String },
    Found { title: String, #[serde(default)] kb: String, #[serde(default)] size: i64, #[serde(default)] reboot: bool },
    Progress { phase: String, index: u32, total: u32, #[serde(default)] title: String },
    Error { title: String, code: i32 },
    Result { found: u32, installed: u32, failed: u32, reboot_required: bool },
    Fatal { message: String },
}

/// Liest eine Zeile; andere Ausgaben (z. B. Warnungen) liefern `None`.
pub fn parse_line(line: &str) -> Option<WuEvent> {
    let l = line.trim().trim_start_matches('\u{feff}');
    if !l.starts_with('{') {
        return None;
    }
    serde_json::from_str(l).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Aufgezeichnete (erfundene) Ausgabe des Skripts.
    const RECORDED: &str = r#"{"type":"phase","phase":"searching"}
{"type":"found","title":"2026-10 Kumulatives Update für Windows Server 2022 (KB5099999)","kb":"KB5099999","size":412345678,"reboot":true}
{"type":"found","title":"Update für Microsoft Defender Antivirus – Ä/Ö/Ü-Test","kb":"KB2267602","size":123456,"reboot":false}
WARNUNG: irgendein Text
{"type":"progress","phase":"downloading","index":1,"total":2,"title":"2026-10 Kumulatives Update"}
{"type":"progress","phase":"installing","index":1,"total":2,"title":"2026-10 Kumulatives Update"}
{"type":"error","title":"Update für Microsoft Defender Antivirus","code":4}
{"type":"result","found":2,"installed":1,"failed":1,"reboot_required":true}
"#;

    #[test]
    fn parses_recorded_output() {
        let ev: Vec<WuEvent> = RECORDED.lines().filter_map(parse_line).collect();
        assert_eq!(ev.len(), 7);
        assert!(matches!(&ev[1], WuEvent::Found { kb, reboot: true, .. } if kb == "KB5099999"));
        assert!(matches!(&ev[2], WuEvent::Found { title, .. } if title.contains("Ä/Ö/Ü")));
        assert_eq!(ev[6], WuEvent::Result { found: 2, installed: 1, failed: 1, reboot_required: true });
        assert_eq!(parse_line(r#"{"type":"fatal","message":"Zugriff verweigert"}"#), Some(WuEvent::Fatal { message: "Zugriff verweigert".into() }));
    }

    #[test]
    fn script_dry_run_has_no_install() {
        let s = script(false);
        assert!(s.contains("Microsoft.Update.Session"));
        assert!(s.contains("if ($false -and"));
        assert!(script(true).contains("if ($true -and"));
    }
}
