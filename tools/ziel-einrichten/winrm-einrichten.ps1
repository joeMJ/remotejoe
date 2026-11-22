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
