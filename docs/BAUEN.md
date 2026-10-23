# remotejoe bauen

- [Ubuntu 24.04](#bauen-unter-ubuntu-2404)
- [Windows 10/11](#bauen-unter-windows-1011), mit [Schritt-für-Schritt-Anleitung](#schritt-für-schritt-unter-windows)

## Bauen unter Ubuntu 24.04

```sh
sudo apt install build-essential pkg-config cmake clang libclang-dev \
    qt6-base-dev qt6-base-dev-tools qt6-svg-dev qmake6 \
    freerdp3-dev libwinpr3-dev
# für den Web-Worker zusätzlich:
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev
# Rust (stable) über https://rustup.rs
cargo build --release
cargo build --release -p remotejoe-worker-web --features webview
```

Die Programme liegen danach in `target/release/`: `remotejoe` und die Worker `rj-worker-ssh`,
`rj-worker-rdp`, `rj-worker-vnc`, `rj-worker-psrp`, `rj-worker-web`. Die Worker müssen im
selben Ordner wie `remotejoe` liegen.

Für PowerShell-Remoting und Hyper-V muss PowerShell 7 (`pwsh`) installiert sein.

Werkzeug für Herausgeber von Updates (Schlüssel, Packen, Signieren, Hochladen):
`cargo build --release -p remotejoe-update-tool` (liefert `rj-update`, siehe `UPDATE.md`).

Tests: `cargo test --workspace`. Tests gegen lokale Server (sshd, xrdp, Xvnc, snmpd) laufen,
wenn diese installiert sind, sonst werden sie übersprungen. Web-Tests mit echter WebView:
`xvfb-run -a cargo test -p remotejoe-worker-web --features webview` (braucht `xvfb`).

![Web-Sitzungen gekachelt im Tab](screenshots/web-tab.png)

## Bauen unter Windows 10/11

Voraussetzungen: Visual Studio Build Tools (MSVC), Rust (`x86_64-pc-windows-msvc`), Qt 6
(Widgets, Svg), LLVM (für bindgen) und FreeRDP 3 über vcpkg. Schritt-für-Schritt-Anleitung mit
Stolpersteinen: [unten](#schritt-für-schritt-unter-windows). Das Skript
`scripts/build-windows.ps1` prüft die Voraussetzungen, baut alles und legt ein Paket mit den
nötigen DLLs an:

```powershell
.\scripts\build-windows.ps1 -VcpkgRoot C:\vcpkg -QtDir C:\Qt\6.5.3\msvc2019_64
```

Daraus baut `scripts/build-msi.ps1` mit WiX 3.14 die signierte MSI unter `target\msi`
(Beschreibung: [`packaging/windows/remotejoe.wxs`](../packaging/windows/remotejoe.wxs)):

```powershell
.\scripts\build-msi.ps1 -WixDir "C:\Program Files (x86)\WiX Toolset v3.14\bin"
```

## Schritt für Schritt unter Windows

Anleitung für Windows 10/11 (x64). Das Skript `scripts/build-windows.ps1` führt die Schritte 5
bis 7 aus und meldet fehlende Voraussetzungen mit Abhilfe.

### 1. Visual Studio Build Tools (MSVC)

1. „Build Tools für Visual Studio 2022“ installieren
   (<https://visualstudio.microsoft.com/de/downloads/>, Abschnitt „Tools für Visual Studio“).
2. Arbeitslast **Desktopentwicklung mit C++** wählen; darin mindestens
   „MSVC v143 – VS 2022 C++-x64/x86-Buildtools“ und ein aktuelles Windows-SDK.

Prüfen: In der „x64 Native Tools-Eingabeaufforderung“ meldet `cl` die Compiler-Version.

### 2. Rust

```powershell
winget install Rustlang.Rustup
rustup default stable-x86_64-pc-windows-msvc
```

Die Version legt `rust-toolchain.toml` im Projekt fest; `rustup` lädt sie beim ersten Bau.
Wichtig ist die **MSVC**-Variante (`x86_64-pc-windows-msvc`), nicht `-gnu`.

### 3. Qt 6 (Widgets, Svg)

Variante **MSVC 64 Bit** (z. B. `msvc2019_64`, passt zu VS 2022), Qt 6.5 oder neuer.

- Qt-Online-Installer: Qt 6.5.x → „MSVC 2019 64-bit“ und „Qt SVG“ (bei neueren Versionen
  in den „Additional Libraries“).
- oder ohne Konto mit aqtinstall:

  ```powershell
  pip install aqtinstall
  aqt install-qt windows desktop 6.5.3 win64_msvc2019_64 -O C:\Qt
  ```

Danach liegt `qmake.exe` in `C:\Qt\6.5.3\msvc2019_64\bin`. Das Skript erwartet diesen Ordner
mit `-QtDir C:\Qt\6.5.3\msvc2019_64` oder `qmake.exe` im `PATH`.

### 4. LLVM (für bindgen)

Der RDP-Worker erzeugt die Anbindung an FreeRDP mit bindgen; dafür wird `libclang.dll`
gebraucht:

```powershell
winget install LLVM.LLVM
```

Liegt LLVM nicht unter `C:\Program Files\LLVM`, `LIBCLANG_PATH` auf den Ordner mit
`libclang.dll` setzen.

### 5. FreeRDP 3 über vcpkg

```powershell
git clone https://github.com/microsoft/vcpkg C:\vcpkg
C:\vcpkg\bootstrap-vcpkg.bat
C:\vcpkg\vcpkg.exe install freerdp:x64-windows pkgconf:x64-windows
```

`pkgconf` liefert das `pkg-config`, mit dem das Bauskript des RDP-Workers `freerdp-client3`
findet. Das Skript setzt dafür:

| Variable | Wert |
|---|---|
| `PKG_CONFIG` | `C:\vcpkg\installed\x64-windows\tools\pkgconf\pkgconf.exe` |
| `PKG_CONFIG_PATH` | `C:\vcpkg\installed\x64-windows\lib\pkgconfig` |
| `PATH` | zusätzlich `C:\vcpkg\installed\x64-windows\bin` (DLLs für Tests) |
| `QMAKE` | Pfad von `qmake.exe` (für cxx-qt) |
| `LIBCLANG_PATH` | Ordner mit `libclang.dll` |

### 6. Bauen

```powershell
cd <Projektordner>
.\scripts\build-windows.ps1 -VcpkgRoot C:\vcpkg -QtDir C:\Qt\6.5.3\msvc2019_64
```

Optionen: `-Test` führt zusätzlich die Tests der Pakete aus, die unter Windows laufen sollen;
`-DebugBuild` baut ohne Optimierung. Von Hand entspricht das:

```powershell
cargo build --release --workspace
cargo build --release -p remotejoe-worker-web --features webview
```

### 7. Paket

Das Skript legt `target\release\remotejoe-paket` an: `remotejoe.exe`, die Worker
(`rj-worker-ssh.exe`, `rj-worker-rdp.exe`, `rj-worker-vnc.exe`, `rj-worker-web.exe`,
`rj-worker-psrp.exe`), die DLLs von FreeRDP aus vcpkg und die Qt-DLLs (`windeployqt`). Die
Worker müssen im selben Ordner wie `remotejoe.exe` liegen.

Zur Laufzeit zusätzlich nötig: WebView2-Laufzeit (in Windows 10/11 meist vorhanden) für
Web-Ziele, PowerShell 7 (`pwsh`) für PowerShell-Remoting und Hyper-V.

### Bau in GitHub Actions

`.github/workflows/windows.yml` führt dieselben Schritte auf einem Windows-Rechner von GitHub
aus (Qt über `install-qt-action`, vcpkg mit Zwischenspeicher). Der Lauf startet **nur von
Hand** (Reiter „Actions“ → „Windows (von Hand)“ → „Run workflow“), nie bei einem Push.

### Bekannte Stolpersteine

- **Falsche Rust-Variante:** Mit `x86_64-pc-windows-gnu` lassen sich Qt (MSVC) und die
  vcpkg-Bibliotheken nicht binden. `rustup default stable-x86_64-pc-windows-msvc`.
- **`cl.exe` nicht gefunden:** Das Skript lädt die Umgebung über `vcvars64.bat`. Schlägt das
  fehl, das Skript aus der „x64 Native Tools-Eingabeaufforderung“ (dort `pwsh`) starten.
- **Mehrere Qt-Versionen im `PATH`:** cxx-qt nimmt das `qmake`, das zuerst gefunden wird.
  `-QtDir` stellt den richtigen Ordner nach vorn.
- **Lange Pfade:** vcpkg und Cargo legen tiefe Ordner an. Projekt und vcpkg in kurzen Pfaden
  ablegen (z. B. `C:\src\remotejoe`, `C:\vcpkg`) oder lange Pfade einschalten
  (`HKLM\SYSTEM\CurrentControlSet\Control\FileSystem\LongPathsEnabled = 1`).
- **Sonderzeichen in Pfaden:** remotejoe selbst nutzt für Dateien die UTF-16-Schnittstellen
  von Windows; einige Bauwerkzeuge (pkg-config-Dateien, ältere Skripte) kommen mit Umlauten
  oder Leerzeichen im Pfad nicht zurecht. Für Projekt, vcpkg und Qt Pfade aus ASCII ohne
  Leerzeichen verwenden.
- **FreeRDP-Version:** vcpkg muss FreeRDP 3.x liefern (`vcpkg list freerdp`). Bei 2.x vcpkg
  aktualisieren (`git pull`, `bootstrap-vcpkg.bat`).
- **Virenscanner:** Echtzeitprüfung von `target\` und `C:\vcpkg` verlangsamt den Bau stark;
  gegebenenfalls Ausnahmen setzen.
- **DLLs fehlen beim Start:** `remotejoe.exe` aus dem Paketordner starten (dort liegen Qt- und
  FreeRDP-DLLs), nicht direkt aus `target\release`.
- **Tests gegen lokale Server** (sshd, xrdp, Xvnc, snmpd) gibt es unter Windows nicht; diese
  Tests melden „übersprungen“.

### Stand der Prüfung

Unter Linux geprüft: alle Pakete ohne Qt und FreeRDP übersetzen für `x86_64-pc-windows-gnu`
(auch der Web-Worker mit WebView2), die Workflow-Datei besteht `actionlint`, das Skript wird
von PowerShell 7 fehlerfrei eingelesen. Auf einem Windows-Rechner ausgeführt wurde nichts.
