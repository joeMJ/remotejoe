//! Inventar für die Objektübersicht: PowerShell-Skripte, die je genau eine JSON-Zeile ausgeben.
//! Beide kommen ohne Adminrechte auf dem Ziel aus (Hyper-V: Gruppe „Hyper-V-Administratoren“
//! auf dem Host genügt) und nutzen nur Cmdlets, Eigenschaften, Kerntypen und Hashtables
//! (kein `[pscustomobject]`), damit sie auch im ConstrainedLanguage-Modus laufen.
//! Die Reihenfolge der JSON-Felder ist daher nicht festgelegt.

use crate::ps_quote;

/// Windows-Ziel über WinRM:
/// `{"name":"…","os":"…","ips":["…"],"macs":["…"],"vcpu":n,"mem_bytes":n}`.
/// Quellen: `Win32_ComputerSystem` (Name, logische Prozessoren, Speicher),
/// `Win32_OperatingSystem` (Bezeichnung und Version), `Win32_NetworkAdapterConfiguration`
/// (nur Adapter mit `IPEnabled`). MACs in Kleinbuchstaben mit Doppelpunkten.
pub fn windows_inventory_script() -> &'static str {
    "$ErrorActionPreference = 'Stop'\n\
     $cs = Get-CimInstance -ClassName Win32_ComputerSystem\n\
     $os = Get-CimInstance -ClassName Win32_OperatingSystem\n\
     $na = @(Get-CimInstance -ClassName Win32_NetworkAdapterConfiguration -Filter 'IPEnabled = TRUE')\n\
     $ips = @($na | ForEach-Object { $_.IPAddress } | Where-Object { $_ })\n\
     $macs = @($na | ForEach-Object { $_.MACAddress } | Where-Object { $_ } | ForEach-Object { $_.ToLower().Replace('-', ':') })\n\
     ConvertTo-Json -Compress -InputObject @{\n\
       name = [string]$cs.Name; os = ([string]$os.Caption).Trim() + ' ' + [string]$os.Version;\n\
       ips = $ips; macs = $macs;\n\
       vcpu = [int]$cs.NumberOfLogicalProcessors; mem_bytes = [int64]$cs.TotalPhysicalMemory\n\
     }"
}

/// Hyper-V-Gast über den Host: `{"vcpu":n,"mem_bytes":n,"macs":["…"],"ips":["…"]}`.
/// Speicher: zugewiesener Speicher der laufenden VM, sonst der Startspeicher.
/// IP-Adressen meldet Hyper-V nur bei laufender VM mit Integrationsdiensten.
pub fn hyperv_guest_inventory_script(vm_id: &str) -> String {
    format!(
        "$ErrorActionPreference = 'Stop'\n\
         $vm = Get-VM -Id {id}\n\
         $na = @(Get-VMNetworkAdapter -VM $vm)\n\
         $macs = @($na | ForEach-Object {{ $_.MacAddress }} | Where-Object {{ $_ }} | ForEach-Object {{ ($_ -replace '(..)(?!$)', '$1:').ToLower() }})\n\
         $ips = @($na | ForEach-Object {{ $_.IPAddresses }} | Where-Object {{ $_ }})\n\
         $mem = [int64]$vm.MemoryAssigned\n\
         if ($mem -le 0) {{ $mem = [int64]$vm.MemoryStartup }}\n\
         ConvertTo-Json -Compress -InputObject @{{ vcpu = [int]$vm.ProcessorCount; mem_bytes = $mem; macs = $macs; ips = $ips }}",
        id = ps_quote(vm_id)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_script_reads_the_three_classes_and_prints_one_compact_json_line() {
        let s = windows_inventory_script();
        for class in ["Win32_ComputerSystem", "Win32_OperatingSystem", "Win32_NetworkAdapterConfiguration"] {
            assert!(s.contains(class), "{class}");
        }
        assert!(s.contains("IPEnabled = TRUE"));
        assert!(s.contains("ConvertTo-Json -Compress -InputObject @{"));
        for key in ["name =", "os =", "ips =", "macs =", "vcpu =", "mem_bytes ="] {
            assert!(s.contains(key), "{key}");
        }
        // Nichts, was ConstrainedLanguage verbietet (Add-Type, .NET-Aufrufe, New-Object).
        for bad in ["Add-Type", "New-Object", "[System.", "[Net.", "[pscustomobject]"] {
            assert!(!s.contains(bad), "{bad}");
        }
    }

    #[test]
    fn hyperv_script_quotes_the_vm_id() {
        assert!(!hyperv_guest_inventory_script("x").contains("[pscustomobject]"));
        let s = hyperv_guest_inventory_script("6f1c0000-0000-4000-8000-000000000001");
        assert!(s.contains("Get-VM -Id '6f1c0000-0000-4000-8000-000000000001'\n"), "{s}");
        assert!(s.contains("Get-VMNetworkAdapter -VM $vm"));
        assert!(s.contains("ConvertTo-Json -Compress -InputObject @{"));
        // Einfache Anführungszeichen werden verdoppelt, die Kennung bleibt ein Literal.
        let s = hyperv_guest_inventory_script("x'; Stop-Computer; '");
        assert!(s.contains("Get-VM -Id 'x''; Stop-Computer; '''\n"), "{s}");
        assert_eq!(s.matches("Stop-Computer").count(), 1);
    }

    /// Echter Lauf des Windows-Skripts auf diesem Rechner (ohne Adminrechte), einmal normal und
    /// einmal im ConstrainedLanguage-Modus.
    #[cfg(windows)]
    #[test]
    fn windows_script_runs_locally_and_prints_the_fields() {
        for prefix in ["", "$ExecutionContext.SessionState.LanguageMode = 'ConstrainedLanguage'
"] {
            let script = format!("{prefix}{}", windows_inventory_script());
            let out = std::process::Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                .output()
                .expect("powershell.exe startet");
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(out.status.success() && out.stderr.is_empty(), "{prefix}{text}
{}", String::from_utf8_lossy(&out.stderr));
            let lines: Vec<_> = text.lines().filter(|l| !l.trim().is_empty()).collect();
            assert_eq!(lines.len(), 1, "genau eine Zeile: {text}");
            let v: serde_json::Value = serde_json::from_str(lines[0]).expect("JSON");
            assert!(v["name"].as_str().is_some_and(|s| !s.is_empty()));
            assert!(v["os"].as_str().is_some_and(|s| s.contains("Windows")));
            assert!(v["ips"].is_array() && v["macs"].is_array());
            assert!(v["vcpu"].as_i64().is_some_and(|n| n >= 1));
            assert!(v["mem_bytes"].as_i64().is_some_and(|n| n > 0));
            for m in v["macs"].as_array().unwrap() {
                let m = m.as_str().unwrap();
                assert!(m.len() == 17 && m == m.to_lowercase() && m.matches(':').count() == 5, "{m}");
            }
        }
    }
}
