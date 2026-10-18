<#
.SYNOPSIS
    Baut aus dem Paketordner (scripts/build-windows.ps1) eine signierte MSI für Windows.

.DESCRIPTION
    Legt einen Bereitstellungsordner an (Paket ohne vc_redist.x64.exe, dazu die Visual-C++-Laufzeit
    app-lokal neben remotejoe.exe), signiert die eigenen Programme, erzeugt die Dateiliste für WiX,
    baut die MSI mit candle/light (WiX 3.14), signiert sie und schreibt die SHA-256-Prüfsumme daneben.
    Installation pro Benutzer ohne Administratorrechte nach %LOCALAPPDATA%\Programs\remotejoe
    (packaging/windows/remotejoe.wxs). Jeder Fehler wird mit Ursache und nächstem Schritt gemeldet.

.PARAMETER PaketDir
    Paketordner aus dem Bau (Vorgabe: target\release\remotejoe-paket).

.PARAMETER WixDir
    Ordner mit candle.exe und light.exe (Vorgabe: Umgebungsvariable WIX bzw. deren bin, sonst C:\Program Files (x86)\WiX Toolset v3.14\bin).

.PARAMETER Thumbprint
    Fingerabdruck des Signaturzertifikats in Cert:\CurrentUser\My (mit privatem Schlüssel).

.PARAMETER TimestampUrl
    Zeitstempeldienst (RFC 3161).

.PARAMETER NoSign
    Nicht signieren (nur für Probeläufe ohne Zertifikat).
#>
param(
    [string]$PaketDir = '',
    [string]$WixDir = '',
    [string]$Thumbprint = 'C595450070432A501AADD72366C837AFB268740A',
    [string]$TimestampUrl = 'http://timestamp.digicert.com',
    [switch]$NoSign
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Fehler([string]$Text, [string]$Hilfe = '') {
    Write-Host "FEHLER: $Text" -ForegroundColor Red
    if ($Hilfe) { Write-Host "  Abhilfe: $Hilfe" -ForegroundColor Yellow }
    exit 1
}

function Schritt([string]$Text) {
    Write-Host "== $Text" -ForegroundColor Cyan
}

function Ausfuehren([string]$Was, [scriptblock]$Block) {
    & $Block
    if ($LASTEXITCODE -ne 0) { Fehler "$Was ist fehlgeschlagen (Rückgabewert $LASTEXITCODE)." 'Meldungen oberhalb prüfen.' }
}

$Root = Split-Path -Parent $PSScriptRoot
if (-not $PaketDir) { $PaketDir = Join-Path $Root 'target\release\remotejoe-paket' }
$Out = Join-Path $Root 'target\msi'
$Stage = Join-Path $Out 'stage'

Schritt 'Version und Werkzeuge'
$versionLine = Select-String -Path (Join-Path $Root 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
if (-not $versionLine) { Fehler 'Version in Cargo.toml nicht gefunden.' }
$Version = $versionLine.Matches[0].Groups[1].Value
if ($Version -notmatch '^\d+\.\d+\.\d+$') { Fehler "Version „$Version“ passt nicht zu einer MSI (nur a.b.c)." 'Für Vorabversionen keine MSI bauen.' }
Write-Host "Version $Version"

if (-not $WixDir) {
    if ($env:WIX -and (Test-Path (Join-Path $env:WIX 'bin\candle.exe'))) { $WixDir = Join-Path $env:WIX 'bin' }
    else { $WixDir = 'C:\Program Files (x86)\WiX Toolset v3.14\bin' }
}
$candle = Join-Path $WixDir 'candle.exe'
$light = Join-Path $WixDir 'light.exe'
if (-not (Test-Path $candle) -or -not (Test-Path $light)) {
    Fehler "WiX nicht gefunden unter $WixDir." 'WiX 3.14 (wix314-binaries.zip von github.com/wixtoolset/wix3/releases) auspacken und -WixDir angeben.'
}

$signtool = $null
if (-not $NoSign) {
    $signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" -ErrorAction SilentlyContinue |
        Sort-Object { [version]$_.Directory.Parent.Name } -Descending | Select-Object -First 1 -ExpandProperty FullName
    if (-not $signtool) { Fehler 'signtool.exe nicht gefunden.' 'Windows SDK (kommt mit den Visual Studio Build Tools) installieren oder -NoSign.' }
    $cert = Get-ChildItem Cert:\CurrentUser\My | Where-Object Thumbprint -eq $Thumbprint
    if (-not $cert) { Fehler "Zertifikat $Thumbprint fehlt in Cert:\CurrentUser\My." 'Zertifikat mit privatem Schlüssel einspielen oder -Thumbprint angeben.' }
    if (-not $cert.HasPrivateKey) { Fehler "Zum Zertifikat $Thumbprint fehlt der private Schlüssel." }
}

Schritt 'Bereitstellungsordner'
if (-not (Test-Path (Join-Path $PaketDir 'remotejoe.exe'))) {
    Fehler "Kein Paket unter $PaketDir." 'Zuerst scripts\build-windows.ps1 ausführen.'
}
if (Test-Path $Stage) { Remove-Item $Stage -Recurse -Force }
New-Item -ItemType Directory -Force $Stage | Out-Null
# vc_redist.x64.exe braucht Administratorrechte; die Laufzeit liegt stattdessen neben remotejoe.exe.
Get-ChildItem $PaketDir | Where-Object Name -ne 'vc_redist.x64.exe' | Copy-Item -Destination $Stage -Recurse
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$vs = if (Test-Path $vswhere) { & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath } else { $null }
$crt = if ($vs) {
    Get-ChildItem (Join-Path $vs 'VC\Redist\MSVC\*\x64\Microsoft.VC14*.CRT') -Directory -ErrorAction SilentlyContinue |
        Sort-Object { [version]$_.Parent.Parent.Name } -Descending | Select-Object -First 1
} else { $null }
if (-not $crt) { Fehler 'Visual-C++-Laufzeit (Microsoft.VC143.CRT) nicht gefunden.' 'Visual Studio Build Tools mit „Desktopentwicklung mit C++“ installieren.' }
Copy-Item (Join-Path $crt.FullName '*.dll') $Stage
Write-Host "Laufzeit: $($crt.FullName)"

if (-not $NoSign) {
    Schritt 'Eigene Programme signieren'
    $eigene = @(Get-ChildItem $Stage -Filter 'remotejoe.exe') + @(Get-ChildItem $Stage -Filter 'rj-worker-*.exe')
    Ausfuehren 'signtool sign (Programme)' { & $signtool sign /sha1 $Thumbprint /fd SHA256 /tr $TimestampUrl /td SHA256 /d 'remotejoe' $eigene.FullName }
}

Schritt 'Dateiliste für WiX'
# Je Ordner eine Komponente mit Schlüssel in HKCU (pro Benutzer im Benutzerprofil vorgeschrieben) und
# RemoveFolder; feste GUIDs aus dem relativen Pfad, damit Updates dieselben Komponenten erkennen.
$md5 = [Security.Cryptography.MD5]::Create()
function Guid-Aus([string]$Text) {
    $b = $md5.ComputeHash([Text.Encoding]::UTF8.GetBytes("remotejoe-msi:$Text"))
    $b[6] = ($b[6] -band 0x0f) -bor 0x30; $b[8] = ($b[8] -band 0x3f) -bor 0x80
    return ([guid]::new([byte[]]$b)).ToString().ToUpper()
}
function Id-Aus([string]$Prefix, [string]$Text) {
    $h = -join ($md5.ComputeHash([Text.Encoding]::UTF8.GetBytes($Text))[0..7] | ForEach-Object { $_.ToString('x2') })
    return "${Prefix}_$h"
}
function Xml([string]$s) { [Security.SecurityElement]::Escape($s) }
$sb = New-Object Text.StringBuilder
[void]$sb.AppendLine('<?xml version="1.0" encoding="utf-8"?>')
[void]$sb.AppendLine('<!-- Erzeugt von scripts/build-msi.ps1, nicht von Hand ändern. -->')
[void]$sb.AppendLine('<Wix xmlns="http://schemas.microsoft.com/wix/2006/wi"><Fragment>')
$comps = New-Object Collections.Generic.List[string]
function Ordner([string]$Pfad, [string]$Rel, [string]$DirId) {
    $files = @(Get-ChildItem $Pfad -File | Sort-Object Name)
    $compId = Id-Aus 'c' "dir:$Rel"
    $comps.Add($compId)
    [void]$sb.AppendLine("<Component Id=`"$compId`" Guid=`"$(Guid-Aus "dir:$Rel")`" Win64=`"yes`">")
    [void]$sb.AppendLine("<RegistryValue Root=`"HKCU`" Key=`"Software\remotejoe\Installer\Ordner`" Name=`"$(Xml ($(if ($Rel) { $Rel } else { '.' })))`" Type=`"integer`" Value=`"1`" KeyPath=`"yes`" />")
    [void]$sb.AppendLine("<RemoveFolder Id=`"$(Id-Aus 'r' "dir:$Rel")`" On=`"uninstall`" />")
    foreach ($f in $files) {
        $relFile = if ($Rel) { "$Rel\$($f.Name)" } else { $f.Name }
        [void]$sb.AppendLine("<File Id=`"$(Id-Aus 'f' "file:$relFile")`" Name=`"$(Xml $f.Name)`" Source=`"$(Xml $f.FullName)`" KeyPath=`"no`" />")
    }
    [void]$sb.AppendLine('</Component>')
    foreach ($d in (Get-ChildItem $Pfad -Directory | Sort-Object Name)) {
        $relDir = if ($Rel) { "$Rel\$($d.Name)" } else { $d.Name }
        $id = Id-Aus 'd' "dir:$relDir"
        [void]$sb.AppendLine("<Directory Id=`"$id`" Name=`"$(Xml $d.Name)`">")
        Ordner $d.FullName $relDir $id
        [void]$sb.AppendLine('</Directory>')
    }
}
[void]$sb.AppendLine('<DirectoryRef Id="INSTALLDIR">')
Ordner $Stage '' 'INSTALLDIR'
[void]$sb.AppendLine('</DirectoryRef>')
[void]$sb.AppendLine('<ComponentGroup Id="AppFiles">')
foreach ($c in $comps) { [void]$sb.AppendLine("<ComponentRef Id=`"$c`" />") }
[void]$sb.AppendLine('</ComponentGroup>')
[void]$sb.AppendLine('</Fragment></Wix>')
$filesWxs = Join-Path $Out 'files.wxs'
[IO.File]::WriteAllText($filesWxs, $sb.ToString(), (New-Object Text.UTF8Encoding $false))
Write-Host "$((Get-ChildItem $Stage -Recurse -File).Count) Dateien in $($comps.Count) Ordnern"

Schritt 'MSI bauen (candle, light)'
$wxs = Join-Path $Root 'packaging\windows\remotejoe.wxs'
$ico = Join-Path $Root 'packaging\windows\remotejoe.ico'
$obj = Join-Path $Out 'obj'
New-Item -ItemType Directory -Force $obj | Out-Null
$msiName = "remotejoe-$Version-windows-x86_64.msi"
$msi = Join-Path $Out $msiName
Ausfuehren 'candle' { & $candle -nologo -arch x64 "-dVersion=$Version" "-dIconFile=$ico" -out "$obj\" $wxs $filesWxs }
# ICE91: Dateien im Benutzerprofil bei einer Installation pro Benutzer (gewollt).
# ICE61: Dieselbe Version ersetzt sich selbst (AllowSameVersionUpgrades, neu gebaute MSI; gewollt).
Ausfuehren 'light' { & $light -nologo -spdb -sice:ICE91 -sice:ICE61 -out $msi "$obj\remotejoe.wixobj" "$obj\files.wixobj" }

if (-not $NoSign) {
    Schritt 'MSI signieren'
    Ausfuehren 'signtool sign (MSI)' { & $signtool sign /sha1 $Thumbprint /fd SHA256 /tr $TimestampUrl /td SHA256 /d "remotejoe $Version" $msi }
}

Schritt 'Prüfsumme'
$hash = (Get-FileHash $msi -Algorithm SHA256).Hash.ToLower()
[IO.File]::WriteAllText("$msi.sha256", "$hash  $msiName`n", (New-Object Text.UTF8Encoding $false))
Write-Host ("Fertig: {0} ({1:N1} MB)" -f $msi, ((Get-Item $msi).Length / 1MB))
Write-Host "SHA-256: $hash"
