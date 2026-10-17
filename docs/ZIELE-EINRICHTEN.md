# Windows-Ziele einrichten (RDP und PowerShell-Remoting)

Dieses Dokument beschreibt, wie ein Windows-Ziel für remotejoe vorbereitet wird. Die Skripte liegen
unter `tools/ziel-einrichten/`. Sie enthalten nur ASCII-Zeichen und speichern keine Kennwörter.

## Variante A: Skript als Datei

1. `winrm-einrichten.ps1`, `winrm-pruefen.ps1` und `winrm-einrichten.bat` auf das Ziel kopieren.
2. Als Administrator ausführen: `winrm-einrichten.bat` (Rechtsklick, „Als Administrator ausführen“) oder in einer
   Administrator-PowerShell `.\winrm-einrichten.ps1`.
3. Das Skript schaltet Remoting ein, legt ein selbstsigniertes Zertifikat und den HTTPS-Listener (Port 5986) an,
   öffnet die Firewall und startet danach die Funktionsprüfung. Es ist mehrfach ausführbar.

Parameter: `-NurVon <Adressen>` (Firewallregel nur für diese Quelladressen), `-DnsNamen <Namen>` (zusätzliche
Namen im Zertifikat, z. B. die IP-Adresse), `-Port <Nummer>`, `-Rueckgaengig` (entfernt Listener, Regel und
Zertifikat von remotejoe; der HTTP-Listener auf 5985 bleibt).

## Variante B: Einfügen in eine Administrator-PowerShell

Ohne Dateiübertragung, ein einziger Block: erst einrichten, dann prüfen. Den ganzen Block markieren und in eine
PowerShell mit Administratorrechten einfügen. Parameter (optional) gehören hinter die Klammer `}` des ersten
Teils, z. B. `} -DnsNamen 192.0.2.30`. Die Windows-PowerShell zeigt beim Einfügen von Zeilen kurz `>>` an, das ist normal.

```powershell
& {
<#
  winrm-einrichten.ps1
  Richtet PowerShell-Remoting ueber HTTPS (Standard: Port 5986) auf einem Windows-Ziel ein.

  Aufruf (Administrator-PowerShell):
    .\winrm-einrichten.ps1
    .\winrm-einrichten.ps1 -NurVon 192.0.2.10,192.0.2.11 -DnsNamen ziel.example.test
    .\winrm-einrichten.ps1 -Rueckgaengig

  Das Skript ist mehrfach ausfuehrbar. Es speichert keine Kennwoerter, schaltet weder
  Basic-Authentifizierung noch unverschluesselten Verkehr ein und aendert keine anderen
  Listener ausser einem vorhandenen HTTPS-Listener fuer Adresse "*".
  Nur ASCII-Zeichen, damit Windows PowerShell 5.1 nichts verstuemmelt.
#>
[CmdletBinding()]
param(
    [string[]]$NurVon,
    [string[]]$DnsNamen,
    [ValidateRange(1, 65535)][int]$Port = 5986,
    [switch]$Rueckgaengig
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2

$RegelName = 'remotejoe WinRM HTTPS'
$CertName  = 'remotejoe-winrm'

function Schritt([string]$Text) { Write-Host ('[..] ' + $Text) }
function Fertig([string]$Text)  { Write-Host ('[OK] ' + $Text) -ForegroundColor Green }
function Hinweis([string]$Text) { Write-Host ('[!!] ' + $Text) -ForegroundColor Yellow }

function Hole-HttpsListener {
    # Liefert alle HTTPS-Listener als Objekte mit Pfad und Zertifikats-Fingerabdruck.
    $liste = @()
    if (-not (Test-Path 'WSMan:\localhost\Listener')) { return $liste }
    foreach ($l in (Get-ChildItem 'WSMan:\localhost\Listener')) {
        if ($l.Keys -contains 'Transport=HTTPS') {
            $tp = ''
            foreach ($p in (Get-ChildItem $l.PSPath)) {
                if ($p.Name -eq 'CertificateThumbprint') { $tp = [string]$p.Value }
            }
            $liste += [pscustomobject]@{
                Pfad        = $l.PSPath
                Adresse     = (($l.Keys | Where-Object { $_ -like 'Address=*' }) -replace '^Address=', '')
                Fingerabdruck = $tp
            }
        }
    }
    return $liste
}

# --- Voraussetzungen ---------------------------------------------------------
$identitaet = [Security.Principal.WindowsIdentity]::GetCurrent()
$prinzipal  = New-Object Security.Principal.WindowsPrincipal($identitaet)
if (-not $prinzipal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Bitte in einer PowerShell mit Administratorrechten starten.'
}
if ($PSVersionTable.PSVersion.Major -lt 5) {
    throw 'Windows PowerShell 5.0 oder neuer ist erforderlich.'
}

# --- Rueckgaengig ------------------------------------------------------------
if ($Rueckgaengig) {
    Schritt 'Entferne HTTPS-Listener mit dem Zertifikat von remotejoe'
    $eigene = @(Get-ChildItem Cert:\LocalMachine\My | Where-Object { $_.FriendlyName -eq $CertName } |
                ForEach-Object { $_.Thumbprint })
    foreach ($l in (Hole-HttpsListener)) {
        if ($eigene -contains $l.Fingerabdruck) { Remove-Item -Path $l.Pfad -Recurse -Force }
    }
    Schritt 'Entferne Firewallregel'
    Get-NetFirewallRule -DisplayName $RegelName -ErrorAction SilentlyContinue | Remove-NetFirewallRule
    Schritt 'Entferne Zertifikat'
    Get-ChildItem Cert:\LocalMachine\My | Where-Object { $_.FriendlyName -eq $CertName } | Remove-Item -Force
    Fertig 'Rueckgaengig gemacht. Der WinRM-Dienst und der HTTP-Listener (Port 5985) bleiben unveraendert.'
    return
}

# --- 1. Remoting grundsaetzlich einschalten ---------------------------------
Schritt 'Schalte PowerShell-Remoting ein (Dienst, HTTP-Listener, Firewallausnahme)'
Enable-PSRemoting -Force -SkipNetworkProfileCheck | Out-Null
Set-Service -Name WinRM -StartupType Automatic
if ((Get-Service -Name WinRM).Status -ne 'Running') { Start-Service -Name WinRM }
Fertig 'WinRM-Dienst laeuft (Starttyp automatisch)'

# --- 2. Zertifikat -----------------------------------------------------------
$namen = @($env:COMPUTERNAME)
if ($DnsNamen) { $namen += $DnsNamen }
$namen = @($namen | Where-Object { $_ } | Select-Object -Unique)

Schritt ('Pruefe Zertifikat fuer: ' + ($namen -join ', '))
$zert = $null
foreach ($c in (Get-ChildItem Cert:\LocalMachine\My | Where-Object { $_.FriendlyName -eq $CertName })) {
    $vorhanden = @($c.DnsNameList | ForEach-Object { $_.Unicode })
    $fehlt = @($namen | Where-Object { $vorhanden -notcontains $_ })
    if (($c.NotAfter -gt (Get-Date).AddDays(30)) -and ($fehlt.Count -eq 0)) { $zert = $c; break }
}
if ($null -eq $zert) {
    Get-ChildItem Cert:\LocalMachine\My | Where-Object { $_.FriendlyName -eq $CertName } | Remove-Item -Force
    $zert = New-SelfSignedCertificate -DnsName $namen -CertStoreLocation Cert:\LocalMachine\My `
            -FriendlyName $CertName -NotAfter (Get-Date).AddYears(5)
    Fertig ('Zertifikat erzeugt, Fingerabdruck ' + $zert.Thumbprint)
} else {
    Fertig ('Vorhandenes Zertifikat weiterverwendet, Fingerabdruck ' + $zert.Thumbprint)
}

# --- 3. HTTPS-Listener -------------------------------------------------------
Schritt ('Richte HTTPS-Listener auf Port ' + $Port + ' ein')
foreach ($l in (Hole-HttpsListener)) {
    if ($l.Adresse -eq '*') {
        if ($l.Fingerabdruck -ne $zert.Thumbprint) { Hinweis ('Ersetze vorhandenen HTTPS-Listener (Zertifikat ' + $l.Fingerabdruck + ')') }
        Remove-Item -Path $l.Pfad -Recurse -Force
    }
}
New-Item -Path WSMan:\localhost\Listener -Transport HTTPS -Address '*' `
         -CertificateThumbPrint $zert.Thumbprint -Port $Port -Force | Out-Null
Fertig 'HTTPS-Listener eingerichtet'

# --- 4. Firewall -------------------------------------------------------------
Schritt 'Richte Firewallregel ein'
Get-NetFirewallRule -DisplayName $RegelName -ErrorAction SilentlyContinue | Remove-NetFirewallRule
$regel = @{
    DisplayName = $RegelName
    Direction   = 'Inbound'
    Protocol    = 'TCP'
    LocalPort   = $Port
    Action      = 'Allow'
    Profile     = 'Any'
}
if ($NurVon) { $regel['RemoteAddress'] = $NurVon }
New-NetFirewallRule @regel | Out-Null
if ($NurVon) { Fertig ('Firewallregel fuer Port ' + $Port + ' nur von: ' + ($NurVon -join ', ')) }
else         { Fertig ('Firewallregel fuer Port ' + $Port + ' von allen Adressen') }

# --- 5. Ergebnis und Pruefung -----------------------------------------------
Write-Host ''
Write-Host 'Fingerabdruck (SHA-1) des Zertifikats zum Vergleichen im Client:'
Write-Host ('  ' + $zert.Thumbprint)
Write-Host ''
$pruef = $null
if ($PSScriptRoot) { $pruef = Join-Path $PSScriptRoot 'winrm-pruefen.ps1' }
if ($pruef -and (Test-Path $pruef)) {
    Schritt 'Starte die Funktionspruefung'
    & $pruef -Port $Port
} else {
    Hinweis 'Keine Pruefung neben diesem Skript gefunden (beim Einfuegen normal); Pruefung separat starten.'
}
}
& {
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
if ($fehler -gt 0) { Write-Host ('Ergebnis: ' + $fehler + ' Fehler') -ForegroundColor Red; return }
Write-Host 'Ergebnis: kein Fehler' -ForegroundColor Green
return
}
```

## RDP einschalten

In einer Administrator-PowerShell auf dem Ziel (Windows Pro, Enterprise oder Server; Home kann kein RDP-Ziel sein):

```powershell
Set-ItemProperty 'HKLM:\System\CurrentControlSet\Control\Terminal Server' -Name fDenyTSConnections -Value 0
Get-NetFirewallRule -Group '@FirewallAPI.dll,-28752' | Enable-NetFirewallRule
```

Danach prüfen, ob Port 3389 lauscht: `Get-NetTCPConnection -State Listen | Where-Object LocalPort -eq 3389`.

## Von Ubuntu aus prüfen, ob ein Port erreichbar ist

```bash
timeout 3 bash -c '</dev/tcp/ZIEL/3389' && echo offen || echo geschlossen
```

## Bekannte Fälle

- **Port 3389 lauscht nicht, obwohl alles eingestellt ist:** Der Dienst `TermService` läuft, hat aber keinen
  Listener, und ein Neustart des Dienstes schlägt fehl. Bei einer VM hilft ein Neustart des Computers (so beobachtet
  bei einer geklonten Windows-10-VM).
- **`Enable-PSRemoting` fehlt vor dem Listener:** Ohne laufenden WinRM-Dienst scheitert `New-Item` auf
  `WSMan:\localhost\Listener` mit „Der Client kann keine Verbindung … herstellen“. Das Skript erledigt die
  Reihenfolge selbst.
- **Selbstsigniertes Zertifikat:** remotejoe zeigt Typ und Fingerabdruck; der Fingerabdruck steht am Ende der
  Ausgabe des Einrichtungsskripts. Zertifikatsprüfung nur für Testziele abschalten.
- **Lokales Konto:** Für den Fernzugriff mit einem lokalen Administratorkonto setzt `Enable-PSRemoting`
  `LocalAccountTokenFilterPolicy`. Besser ist ein Domänenkonto mit Kerberos.
