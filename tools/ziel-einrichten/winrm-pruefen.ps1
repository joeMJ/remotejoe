<#
  winrm-pruefen.ps1
  Prueft auf einem Windows-Ziel, ob PowerShell-Remoting ueber HTTPS einsatzbereit ist.
  Aendert nichts. Ergebnis: Tabelle mit OK, WARNUNG oder FEHLER; Exitcode 0 ohne FEHLER, sonst 1.

  Aufruf: .\winrm-pruefen.ps1 [-Port 5986]
  Nur ASCII-Zeichen, damit Windows PowerShell 5.1 nichts verstuemmelt.
#>
[CmdletBinding()]
param(
    [ValidateRange(1, 65535)][int]$Port = 5986
)

$ErrorActionPreference = 'Stop'
$RegelName = 'remotejoe WinRM HTTPS'
$CertName  = 'remotejoe-winrm'
$ergebnis  = New-Object System.Collections.ArrayList

function Eintrag([string]$Pruefung, [string]$Status, [string]$Grund) {
    [void]$ergebnis.Add([pscustomobject]@{ Pruefung = $Pruefung; Status = $Status; Grund = $Grund })
}

# 1. Dienst
try {
    $dienst = Get-Service -Name WinRM
    if ($dienst.Status -eq 'Running') { Eintrag 'WinRM-Dienst' 'OK' ('laeuft, Starttyp ' + $dienst.StartType) }
    else { Eintrag 'WinRM-Dienst' 'FEHLER' ('Status ' + $dienst.Status + ' - Dienst starten oder winrm-einrichten.ps1 ausfuehren') }
} catch { Eintrag 'WinRM-Dienst' 'FEHLER' $_.Exception.Message }

# 2. Lauschende Ports
foreach ($p in @(5985, $Port) | Select-Object -Unique) {
    try {
        $l = @(Get-NetTCPConnection -State Listen -LocalPort $p -ErrorAction SilentlyContinue)
        if ($l.Count -gt 0) { Eintrag ('Port ' + $p + ' lauscht') 'OK' 'ja' }
        elseif ($p -eq 5985) { Eintrag ('Port ' + $p + ' lauscht') 'WARNUNG' 'HTTP-Listener nicht aktiv (fuer HTTPS nicht noetig)' }
        else { Eintrag ('Port ' + $p + ' lauscht') 'FEHLER' 'kein Listener - bei geklonten Systemen hilft manchmal ein Neustart' }
    } catch { Eintrag ('Port ' + $p + ' lauscht') 'FEHLER' $_.Exception.Message }
}

# 3. HTTPS-Listener und Zertifikat
try {
    $tp = ''
    foreach ($l in (Get-ChildItem 'WSMan:\localhost\Listener')) {
        if ($l.Keys -contains 'Transport=HTTPS') {
            foreach ($e in (Get-ChildItem $l.PSPath)) { if ($e.Name -eq 'CertificateThumbprint') { $tp = [string]$e.Value } }
        }
    }
    if (-not $tp) { Eintrag 'HTTPS-Listener' 'FEHLER' 'nicht vorhanden - winrm-einrichten.ps1 ausfuehren' }
    else {
        Eintrag 'HTTPS-Listener' 'OK' ('vorhanden, Zertifikat ' + $tp)
        $z = Get-ChildItem Cert:\LocalMachine\My | Where-Object { $_.Thumbprint -eq $tp }
        if ($null -eq $z) { Eintrag 'Zertifikat' 'FEHLER' 'nicht im Speicher des Computers gefunden' }
        elseif ($z.NotAfter -lt (Get-Date)) { Eintrag 'Zertifikat' 'FEHLER' ('abgelaufen am ' + $z.NotAfter.ToString('yyyy-MM-dd')) }
        elseif ($z.NotAfter -lt (Get-Date).AddDays(30)) { Eintrag 'Zertifikat' 'WARNUNG' ('laeuft ab am ' + $z.NotAfter.ToString('yyyy-MM-dd')) }
        else { Eintrag 'Zertifikat' 'OK' ('gueltig bis ' + $z.NotAfter.ToString('yyyy-MM-dd') + ', Namen: ' + ((@($z.DnsNameList | ForEach-Object { $_.Unicode })) -join ', ')) }
    }
} catch { Eintrag 'HTTPS-Listener' 'FEHLER' $_.Exception.Message }

# 4. Firewall
try {
    $r = @(Get-NetFirewallPortFilter -Protocol TCP | Where-Object { $_.LocalPort -eq $Port } |
           Get-NetFirewallRule | Where-Object { $_.Enabled -eq 'True' -and $_.Direction -eq 'Inbound' -and $_.Action -eq 'Allow' })
    if ($r.Count -gt 0) { Eintrag ('Firewall Port ' + $Port) 'OK' ('Regel: ' + $r[0].DisplayName) }
    else { Eintrag ('Firewall Port ' + $Port) 'FEHLER' 'keine aktive Regel, die eingehenden Verkehr erlaubt' }
} catch { Eintrag ('Firewall Port ' + $Port) 'FEHLER' $_.Exception.Message }

# 5. Beispielbefehl ueber HTTPS auf diesen Rechner (Schleife)
try {
    $opt = New-PSSessionOption -SkipCACheck -SkipCNCheck
    $name = Invoke-Command -ComputerName $env:COMPUTERNAME -UseSSL -Port $Port -SessionOption $opt -ScriptBlock { $env:COMPUTERNAME }
    Eintrag 'Beispielbefehl ueber HTTPS' 'OK' ('Antwort von ' + $name)
} catch {
    Eintrag 'Beispielbefehl ueber HTTPS' 'WARNUNG' ('nicht bestaetigt (evtl. Anmeldedaten noetig): ' + $_.Exception.Message)
}

# Ausgabe
Write-Host ''
$ergebnis | Format-Table -AutoSize -Wrap | Out-String -Width 200 | Write-Host
Write-Host 'Von einem anderen Rechner pruefen:  Test-NetConnection <Ziel> -Port' $Port
$fehler = @($ergebnis | Where-Object { $_.Status -eq 'FEHLER' }).Count
if ($fehler -gt 0) { Write-Host ('Ergebnis: ' + $fehler + ' Fehler') -ForegroundColor Red; exit 1 }
Write-Host 'Ergebnis: kein Fehler' -ForegroundColor Green
exit 0
