# remotejoe

Verbindungsmanager für Ubuntu 24.04 und Windows 10/11. Verbindungen zu Servern und Geräten
über **RDP, SSH, VNC, Web, PowerShell**, dazu **Proxmox VE** und **Hyper-V**. Die
Zugangsdaten liegen in einer KeePass-Datei (KDBX), die man weiter mit KeePassXC öffnen kann.

![Hauptfenster](docs/screenshots/hauptfenster-hell.png)

## Was das Programm kann

- **Baum links, Sitzungen rechts:** Ordner und Ziele in fester Reihenfolge, Suche über Namen
  und Hosts (Strg+F). Sitzungen in Tabs, gekachelt (1×2, 2×2, frei) oder als eigenes Fenster.
- **RDP** mit Zwischenablage (Text in beide Richtungen, Dateien Ziel → Host), NLA.
- **SSH** mit Terminal, Zwischenablage (geschütztes Einfügen; OSC 52: Schreiben erlaubt, Lesen nur nach Nachfrage),
  Host-Schlüssel-Prüfung im OpenSSH-Format, Schlüssel aus KDBX-Anhängen.
- **VNC** (eigene Umsetzung) mit TLS (VeNCrypt), Zertifikatsprüfung per Fingerabdruck und
  UTF-8-Zwischenablage.
- **Web** in eigenem Fenster mit eigenem Browserprofil je Ziel und automatischer
  Formular-Anmeldung.
- **PowerShell-Remoting** (WinRM oder SSH) über ein lokal installiertes PowerShell 7.
- **Proxmox VE und Hyper-V:** Ziele im Baum aufklappen, Gäste starten, herunterfahren, neu
  starten, ausschalten und deren Konsole öffnen.
- **Tunnel:** SSH-Sprungserver (auch mehrstufig), WireGuard ohne Systemadapter, oder ein
  vorhandenes VPN.
- **Aufgaben:** Aktualisierungen prüfen und einspielen (apt über SSH, Windows Update über
  PowerShell), als „Patchday“ für viele Ziele parallel.
- **SNMP-Ampel** am Baum (CPU, Speicher, Schnittstellen) mit Verlauf.
- **Import** aus Remote Desktop Manager (`.rdm`, JSON) und Royal TS (`.rtsz`, `.rtsx`).
- Jede Verbindung läuft in einem eigenen Prozess: Absturz oder Hänger einer Sitzung betrifft
  die anderen nicht.

## Das Hybridmodell: KDBX und Index

remotejoe trennt Zugangsdaten und Struktur:

1. **Zugangsdaten** sind ganz normale Einträge in der KDBX (Titel, Benutzer, Kennwort,
   Anhänge). Man pflegt sie wie gewohnt, auch mit KeePassXC.
2. **Struktur und Einstellungen** (Ordner, Ziele, Protokolloptionen, Tunnel, Aufgaben) stehen
   im **Index**. Der Index liegt verschlüsselt in derselben KDBX-Datei (als interne
   Zusatzdaten, KeePassXC lässt ihn unverändert) und verweist per UUID auf die Einträge.
3. **Vererbung:** Ein Zugangsdatensatz, ein Tunnel oder der PowerShell-Transport, der an einem
   Ordner hinterlegt ist, gilt für alle Ziele darunter, bis ein Unterordner oder das Ziel etwas
   anderes festlegt. Zugangsdatensätze haben Namen (`default`, `root`, …); ein Ziel kann einen
   bestimmten Namen verlangen. Benutzername und Domäne lassen sich je Ordner überschreiben,
   das Kennwort nicht.

Es gibt also nur eine Datei zum Sichern und Synchronisieren. Wird die KDBX gleichzeitig an
anderer Stelle geändert, führt remotejoe beim Speichern zusammen und fragt nur, wenn beide
Seiten den Index geändert haben. Einzelheiten: [`docs/DATENMODELL.md`](docs/DATENMODELL.md).

Kennwörter werden byte-genau verwendet (nie normalisiert, getrimmt oder umkodiert) und nie über
die Zwischenablage in ein Ziel gebracht. Protokolle enthalten nur Fehler und
Verbindungsereignisse, keine Kennwörter, Eingaben oder Inhalte der Zwischenablage.

## Bedienung

1. Programm starten: `remotejoe`. Beim ersten Start **Datei → KDBX öffnen …** wählen,
   Hauptkennwort und gegebenenfalls Schlüsseldatei angeben.
2. Struktur anlegen: **Datei → Importieren (RDM, Royal TS) …** zeigt zuerst eine Vorschau
   (was übernommen, zusammengelegt oder übersprungen wird) und übernimmt erst nach Bestätigung.
   Zugangsdaten aus dem Import werden als KDBX-Einträge in der Gruppe `remotejoe` angelegt.
3. Verbinden: Doppelklick auf ein Ziel oder Kontextmenü **Verbinden**. Fehlt ein Eintrag oder
   kommen mehrere in Frage, fragt das Programm nach.
4. Rückfragen zu unbekannten Host-Schlüsseln und Zertifikaten zeigen den Fingerabdruck; ein
   geänderter Schlüssel wird deutlich gewarnt und nie still angenommen.
5. Kontextmenü **Aufgaben → Aktualisierungen prüfen** startet den Patchday für die gewählten
   Ziele (zuerst als Trockenlauf).
6. **Datei → Speichern** schreibt KDBX und Index (atomar, mit Sicherungskopie `.bak`).
   **Datei → Sperren** schließt die KDBX; laufende Sitzungen bleiben bestehen.

Zum Ausprobieren ohne eigene Daten: `remotejoe --demo` (erfundene Ziele, Vorführsitzungen).

Einstellungen stehen in `~/.config/remotejoe/config.toml` (Windows:
`%APPDATA%\remotejoe\config.toml`), z. B.:

```toml
kdbx_path = "/home/benutzer/zugang.kdbx"

[connection]
connect_timeout_secs = 15
keepalive_secs = 30
auto_reconnect = false
```

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

Tests: `cargo test --workspace`. Tests gegen lokale Server (sshd, xrdp, Xvnc, snmpd) laufen,
wenn diese installiert sind, sonst werden sie übersprungen.

## Bauen unter Windows 10/11

Voraussetzungen: Rust (stable, Ziel `x86_64-pc-windows-msvc`), Visual Studio Build Tools,
Qt 6 (Widgets, Svg; `qmake` im `PATH`), FreeRDP 3 (z. B. über vcpkg, `pkg-config` muss
`freerdp-client3` finden), LLVM/Clang für bindgen. Der Web-Worker nutzt die
WebView2-Laufzeit (in Windows 10/11 meist vorhanden).

```bat
cargo build --release
cargo build --release -p remotejoe-worker-web --features webview
```

Qt- und FreeRDP-DLLs gehören neben `remotejoe.exe` (z. B. mit `windeployqt`).

## Lizenz

Apache-2.0, siehe [`LICENSE`](LICENSE) und [`NOTICE`](NOTICE). Verwendete Bibliotheken mit
ihren Lizenzen: [`THIRD_PARTY.md`](THIRD_PARTY.md). Aufbau des Programms:
[`docs/ARCHITEKTUR.md`](docs/ARCHITEKTUR.md).
