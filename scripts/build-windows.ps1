<#
.SYNOPSIS
    Baut remotejoe unter Windows 10/11 (MSVC, Qt 6, FreeRDP 3 aus vcpkg).

.DESCRIPTION
    Prüft die Werkzeuge, installiert FreeRDP 3 und pkgconf über vcpkg, setzt die
    Umgebungsvariablen für die Rust-Bauskripte (pkg-config, bindgen, cxx-qt),
    baut alle Programme und legt ein lauffähiges Paket mit Qt- und FreeRDP-DLLs an.
    Jeder Fehler wird mit Ursache und nächstem Schritt gemeldet.
    Anleitung: docs/BUILD-WINDOWS.md

.PARAMETER VcpkgRoot
    Ordner von vcpkg (Vorgabe: Umgebungsvariable VCPKG_ROOT, sonst C:\vcpkg).

.PARAMETER QtDir
    Qt-Ordner für MSVC 64 Bit, z. B. C:\Qt\6.5.3\msvc2019_64 (Vorgabe: qmake.exe im PATH).

.PARAMETER Test
    Zusätzlich die Tests der Pakete ausführen, die unter Windows laufen sollen.

.PARAMETER DebugBuild
    Debug- statt Release-Bau.

.PARAMETER CleanPaths
    Keine Pfade des Bau-Rechners in den Programmen (für Veröffentlichungen): Rust mit
    --remap-path-prefix für Benutzerprofil, Cargo-Registry (auch die Kurzform), Projekt- und
    Qt-Ordner; C++ (MSVC) mit /d1trimfile: für dieselben Ordner; der Linker schreibt nur den
    Dateinamen der PDB (/PDBALTPATH:%_PDB%). Ändert die Bau-Einstellungen, daher Neubau aller Pakete.
#>
param(
    [string]$VcpkgRoot = $(if ($env:VCPKG_ROOT) { $env:VCPKG_ROOT } else { 'C:\vcpkg' }),
    [string]$QtDir = '',
    [switch]$Test,
    [switch]$DebugBuild,
    [switch]$CleanPaths
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

function Pruefe-Befehl([string]$Name, [string]$Hilfe) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) { Fehler "$Name wurde nicht gefunden." $Hilfe }
}

function Ausfuehren([string]$Was, [scriptblock]$Block) {
    & $Block
    if ($LASTEXITCODE -ne 0) { Fehler "$Was ist fehlgeschlagen (Rückgabewert $LASTEXITCODE)." 'Meldungen oberhalb prüfen; Stolpersteine stehen in docs/BUILD-WINDOWS.md.' }
}

$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

Schritt 'Rust prüfen'
Pruefe-Befehl cargo 'Rust über https://rustup.rs installieren, danach: rustup default stable-x86_64-pc-windows-msvc'
$hostLine = (& rustc -vV | Select-String '^host:').ToString()
$rustHost = $hostLine.Split(' ')[1]
if ($rustHost -ne 'x86_64-pc-windows-msvc') {
    Fehler "Die Rust-Werkzeugkette ist $rustHost, benötigt wird x86_64-pc-windows-msvc." 'rustup default stable-x86_64-pc-windows-msvc'
}

Schritt 'MSVC prüfen'
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path $vswhere)) {
    Fehler 'Visual Studio Build Tools fehlen.' 'Build Tools für Visual Studio 2022 mit der Arbeitslast „Desktopentwicklung mit C++“ installieren.'
}
$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) {
    Fehler 'Keine MSVC-Werkzeuge für x64 gefunden.' 'Im Visual Studio Installer „MSVC v143 – VS 2022 C++-x64/x86-Buildtools“ und ein Windows-SDK wählen.'
}
if (-not (Get-Command cl.exe -ErrorAction SilentlyContinue)) {
    # Umgebung der x64-Entwickler-Eingabeaufforderung übernehmen.
    $vcvars = Join-Path $vs 'VC\Auxiliary\Build\vcvars64.bat'
    cmd /c "`"$vcvars`" >nul && set" | ForEach-Object {
        if ($_ -match '^([^=]+)=(.*)$') { Set-Item -Path "env:$($Matches[1])" -Value $Matches[2] }
    }
}
Pruefe-Befehl cl.exe 'vcvars64.bat aus den Build Tools manuell ausführen oder das Skript in der „x64 Native Tools“-Eingabeaufforderung starten.'

Schritt 'Qt 6 prüfen'
if ($QtDir) { $env:PATH = (Join-Path $QtDir 'bin') + ';' + $env:PATH }
Pruefe-Befehl qmake.exe 'Qt 6 für MSVC 64 Bit installieren (Qt-Installer oder: pip install aqtinstall; aqt install-qt windows desktop 6.5.3 win64_msvc2019_64) und -QtDir angeben.'
$qtVersion = (& qmake.exe -query QT_VERSION)
if (-not $qtVersion.StartsWith('6.')) { Fehler "qmake meldet Qt $qtVersion; benötigt wird Qt 6." 'Den bin-Ordner von Qt 6 vor anderen Qt-Versionen in den PATH stellen.' }
$env:QMAKE = (Get-Command qmake.exe).Source
$qtBins = (& qmake.exe -query QT_INSTALL_BINS)
$qtLibs = (& qmake.exe -query QT_INSTALL_LIBS)
if (-not (Test-Path (Join-Path $qtLibs 'Qt6Svg.lib'))) { Fehler 'Das Qt-Modul Svg fehlt.' 'Im Qt-Installer „Qt SVG“ bzw. bei aqt das Paket qtsvg mitinstallieren.' }
if (-not (Test-Path (Join-Path $qtLibs 'Qt6Widgets.lib'))) { Fehler 'Qt Widgets fehlt (falsche Qt-Variante?).' 'Die Variante msvc2019_64 bzw. msvc2022_64 verwenden, nicht mingw.' }

Schritt 'LLVM (bindgen) prüfen'
if (-not $env:LIBCLANG_PATH) {
    foreach ($c in @("$env:ProgramFiles\LLVM\bin", "${env:ProgramFiles(x86)}\LLVM\bin")) {
        if (Test-Path (Join-Path $c 'libclang.dll')) { $env:LIBCLANG_PATH = $c }
    }
}
if (-not $env:LIBCLANG_PATH -or -not (Test-Path (Join-Path $env:LIBCLANG_PATH 'libclang.dll'))) {
    Fehler 'libclang.dll nicht gefunden (bindgen für den RDP-Worker).' 'LLVM installieren (winget install LLVM.LLVM) oder LIBCLANG_PATH auf den Ordner mit libclang.dll setzen.'
}

Schritt 'FreeRDP 3 und pkgconf über vcpkg'
$vcpkg = Join-Path $VcpkgRoot 'vcpkg.exe'
if (-not (Test-Path $vcpkg)) {
    Fehler "vcpkg nicht gefunden unter $VcpkgRoot." 'git clone https://github.com/microsoft/vcpkg C:\vcpkg; C:\vcpkg\bootstrap-vcpkg.bat; dann -VcpkgRoot C:\vcpkg angeben.'
}
Ausfuehren 'vcpkg install' { & $vcpkg install 'freerdp:x64-windows' 'pkgconf:x64-windows' }
$inst = Join-Path $VcpkgRoot 'installed\x64-windows'
if (-not (Test-Path (Join-Path $inst 'lib\pkgconfig\freerdp-client3.pc'))) {
    Fehler 'freerdp-client3.pc fehlt nach vcpkg install.' "„$vcpkg list freerdp“ prüfen; benötigt wird FreeRDP 3.x (vcpkg aktualisieren: git pull; bootstrap-vcpkg.bat)."
}
$env:PKG_CONFIG = Join-Path $inst 'tools\pkgconf\pkgconf.exe'
$env:PKG_CONFIG_PATH = Join-Path $inst 'lib\pkgconfig'
$env:PATH = (Join-Path $inst 'bin') + ';' + $env:PATH
if (-not (Test-Path $env:PKG_CONFIG)) { Fehler "pkgconf.exe fehlt unter $($env:PKG_CONFIG)." 'vcpkg install pkgconf:x64-windows wiederholen.' }

$buildProfile = @()
$outName = 'debug'
if (-not $DebugBuild) { $buildProfile = @('--release'); $outName = 'release' }

if ($CleanPaths) {
    Schritt 'Pfade des Bau-Rechners entfernen'
    # Jeder Ordner auch in Kurzform (8.3, z. B. CARGO~1) und in der Schreibweise des aktuellen Ordners.
    $fso = New-Object -ComObject Scripting.FileSystemObject
    function Formen([string]$Pfad) {
        if (-not $Pfad -or -not (Test-Path $Pfad)) { return @() }
        $voll = (Resolve-Path $Pfad).ProviderPath.TrimEnd('\')
        @($voll, $fso.GetFolder($voll).ShortPath.TrimEnd('\'), $Pfad.TrimEnd('\')) | Select-Object -Unique
    }
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
    # Reihenfolge: allgemein vor speziell (bei mehreren Treffern gilt die letzte passende Zuordnung).
    $maps = @()
    foreach ($p in Formen $env:USERPROFILE) { $maps += , @($p, '~') }
    foreach ($p in Formen (Join-Path $cargoHome 'registry\src')) { $maps += , @($p, 'cargo-registry') }
    foreach ($p in (@(Formen $Root) + @([IO.Directory]::GetCurrentDirectory()) | Select-Object -Unique)) { $maps += , @($p, 'remotejoe') }
    if ($QtDir) { foreach ($p in Formen $QtDir) { $maps += , @($p, 'qt') } }
    $flags = @($maps | ForEach-Object { "--remap-path-prefix=$($_[0])=$($_[1])" }) + @('-C', 'link-arg=/PDBALTPATH:%_PDB%')
    # Durch 0x1F getrennt: Pfade mit Leerzeichen bleiben ganz.
    $env:CARGO_ENCODED_RUSTFLAGS = $flags -join [char]0x1f
    $trim = @($maps | Where-Object { $_[1] -ne '~' } | ForEach-Object { "/d1trimfile:$($_[0])\" }) | Select-Object -Unique
    $env:CL = (@($trim) + @($env:CL) | Where-Object { $_ }) -join ' '
    $maps | ForEach-Object { Write-Host "  $($_[0]) -> $($_[1])" }
}

Schritt 'Bauen'
Ausfuehren 'cargo build' { cargo build @buildProfile --workspace }
Ausfuehren 'cargo build (Web-Worker mit WebView2)' { cargo build @buildProfile -p remotejoe-worker-web --features webview }

if ($Test) {
    Schritt 'Tests der Windows-Pakete'
    $pakete = @('remotejoe-core', 'remotejoe-kdbx', 'remotejoe-ipc', 'remotejoe-supervisor', 'remotejoe-tasks', 'remotejoe-importer',
        'remotejoe-snmp', 'remotejoe-tunnel', 'remotejoe-proxmox', 'remotejoe-worker-ssh', 'remotejoe-worker-vnc',
        'remotejoe-worker-psrp', 'remotejoe-worker-web', 'remotejoe-worker-rdp')
    $testArgs = @()
    foreach ($p in $pakete) { $testArgs += @('-p', $p) }
    Ausfuehren 'cargo test' { cargo test @buildProfile @testArgs }
    Ausfuehren 'cargo test (Logik der Oberfläche)' { cargo test @buildProfile -p remotejoe-ui --lib }
}

Schritt 'Paket zusammenstellen'
$out = Join-Path $Root "target\$outName"
$paket = Join-Path $out 'remotejoe-paket'
New-Item -ItemType Directory -Force -Path $paket | Out-Null
foreach ($exe in @('remotejoe', 'rj-worker-ssh', 'rj-worker-rdp', 'rj-worker-vnc', 'rj-worker-web', 'rj-worker-psrp')) {
    $src = Join-Path $out "$exe.exe"
    if (-not (Test-Path $src)) { Fehler "$exe.exe wurde nicht gebaut." 'cargo build ohne Fehler abschließen.' }
    Copy-Item $src $paket -Force
}
Copy-Item (Join-Path $inst 'bin\*.dll') $paket -Force
# Lizenztexte der mitgelieferten vcpkg-Bibliotheken (FreeRDP, OpenSSL, cJSON, zlib, pkgconf …); build-msi.ps1
# hängt sie an THIRD_PARTY_LICENSES.txt an.
$teile = foreach ($c in Get-ChildItem (Join-Path $inst 'share\*\copyright') -ErrorAction SilentlyContinue |
        Where-Object { $_.Directory.Name -notlike 'vcpkg-*' } | Sort-Object { $_.Directory.Name }) {
    ('=' * 78); "$($c.Directory.Name) (vcpkg, $($c.Directory.Name)\copyright)"; ('=' * 78); (Get-Content -Raw $c.FullName).Trim(); ''
}
if (-not $teile) { Fehler 'Keine Lizenztexte unter vcpkg\installed\x64-windows\share gefunden.' 'vcpkg install wiederholen.' }
Set-Content -Path (Join-Path $paket 'THIRD_PARTY_LICENSES-vcpkg.txt') -Value $teile -Encoding utf8
Ausfuehren 'windeployqt' { & (Join-Path $qtBins 'windeployqt.exe') --translations de (Join-Path $paket 'remotejoe.exe') }
Write-Host "Fertig: $paket" -ForegroundColor Green
