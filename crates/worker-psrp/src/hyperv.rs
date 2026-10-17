//! Hyper-V über PowerShell auf dem Hyper-V-Host: VM-Liste, Zustand,
//! Start/Stopp/Neustart. Die Konsole läuft per RDP auf den Gast (erste
//! IPv4-Adresse aus den Integrationsdiensten).

use serde::Deserialize;

use crate::{ps_quote, PsError, PsSession};

/// Eine VM, wie sie das Listenskript liefert.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HvVm {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "State")]
    pub state: String,
    #[serde(rename = "CpuUsage", default)]
    pub cpu_percent: u32,
    #[serde(rename = "MemoryAssigned", default)]
    pub memory: u64,
    #[serde(rename = "UptimeSeconds", default)]
    pub uptime_secs: u64,
    #[serde(rename = "Addresses", default)]
    pub addresses: Vec<String>,
}

impl HvVm {
    pub fn running(&self) -> bool {
        self.state.eq_ignore_ascii_case("Running")
    }

    /// Ziel für die RDP-Konsole: erste IPv4-Adresse, sonst erste Adresse.
    pub fn console_host(&self) -> Option<&str> {
        let ok = |a: &&String| !a.is_empty() && !a.starts_with("fe80:") && !a.starts_with("169.254.");
        self.addresses
            .iter()
            .filter(ok)
            .find(|a| a.parse::<std::net::Ipv4Addr>().is_ok())
            .or_else(|| self.addresses.iter().find(ok))
            .map(String::as_str)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HvAction {
    Start,
    /// Herunterfahren über die Integrationsdienste.
    Shutdown,
    /// Ausschalten ohne Herunterfahren.
    TurnOff,
    Restart,
}

/// Listenskript: ein JSON-Feld mit allen VMs (Zustand als Text).
pub fn list_script() -> String {
    "$vms = @(Get-VM | ForEach-Object {\n\
       [pscustomobject]@{\n\
         Id = $_.Id.ToString(); Name = $_.Name; State = $_.State.ToString();\n\
         CpuUsage = [int]$_.CPUUsage; MemoryAssigned = [int64]$_.MemoryAssigned;\n\
         UptimeSeconds = [int64]$_.Uptime.TotalSeconds;\n\
         Addresses = @($_.NetworkAdapters | ForEach-Object { $_.IPAddresses } | Where-Object { $_ })\n\
       } })\n\
     ConvertTo-Json -InputObject $vms -Compress -Depth 3"
        .to_string()
}

/// Skript für eine Aktion auf der VM mit der Kennung `id` (GUID).
pub fn action_script(id: &str, a: HvAction) -> String {
    let cmd = match a {
        HvAction::Start => "Start-VM",
        HvAction::Shutdown => "Stop-VM -Force",
        HvAction::TurnOff => "Stop-VM -TurnOff -Force",
        HvAction::Restart => "Restart-VM -Force",
    };
    format!("Get-VM -Id {} | {cmd} -ErrorAction Stop", ps_quote(id))
}

/// Liest die Ausgabe des Listenskripts.
pub fn parse_list(json: &str) -> Result<Vec<HvVm>, String> {
    let t = json.trim();
    if t.is_empty() {
        return Ok(Vec::new());
    }
    let mut v: Vec<HvVm> = serde_json::from_str(t).map_err(|e| format!("VM-Liste unverständlich: {e}"))?;
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(v)
}

fn run_collect(s: &mut PsSession, script: &str) -> Result<(String, Vec<String>), PsError> {
    let mut out = String::new();
    let mut errors = Vec::new();
    let ok = s.run(script, |r| match r.stream.as_str() {
        "output" => out.push_str(&r.text),
        "error" => errors.push(r.text.clone()),
        _ => {}
    })?;
    if !ok && errors.is_empty() {
        errors.push("Befehl fehlgeschlagen".into());
    }
    Ok((out, errors))
}

/// VMs des Hosts abrufen.
pub fn list(s: &mut PsSession) -> Result<Vec<HvVm>, PsError> {
    let (out, errors) = run_collect(s, &list_script())?;
    if out.trim().is_empty() && !errors.is_empty() {
        return Err(PsError::Session(errors.join("; ")));
    }
    parse_list(&out).map_err(PsError::Session)
}

/// Aktion ausführen; Fehlermeldungen der Gegenstelle werden zurückgegeben.
pub fn action(s: &mut PsSession, id: &str, a: HvAction) -> Result<(), PsError> {
    let (_, errors) = run_collect(s, &action_script(id, a))?;
    if errors.is_empty() {
        Ok(())
    } else {
        Err(PsError::Session(errors.join("; ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_scripts_quote_the_id() {
        assert_eq!(action_script("a'b", HvAction::Start), "Get-VM -Id 'a''b' | Start-VM -ErrorAction Stop");
        assert!(action_script("x", HvAction::TurnOff).contains("-TurnOff"));
    }

    #[test]
    fn console_prefers_ipv4() {
        let vm = HvVm {
            id: "1".into(),
            name: "a".into(),
            state: "Running".into(),
            cpu_percent: 0,
            memory: 0,
            uptime_secs: 0,
            addresses: vec!["fe80::1".into(), "169.254.3.4".into(), "2001:db8::5".into(), "192.0.2.10".into()],
        };
        assert_eq!(vm.console_host(), Some("192.0.2.10"));
    }
}
