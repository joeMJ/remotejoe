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
- **Bearbeiten im Hauptfenster:** Ordner und Ziele anlegen, ändern, verschieben, duplizieren
  und löschen im Eigenschaften-Tab, Vorlagen je Zieltyp, Favoriten, Ansichten (z. B.
  „Patchday“), „Auf Unterobjekte anwenden“, Rückgängig.
- **Mehrere Ziele auf einmal anlegen:** ein Host pro Zeile (bis 500), Namensmuster mit
  `{host}` und `{nr}`, Vorlage und Zugangsdaten, Vorschau mit Hinweisen je Zeile; alles als eine
  rückgängig machbare Aktion.
- **Keine Dialogfenster:** Rückfragen, Hinweise und Fehlermeldungen erscheinen als Leiste im
  Tab oder im Hauptfenster; Import-Vorschau und Patchday sind Tabs.
- **Alle im Ordner verbinden**, neu verbinden oder trennen (gestaffelt, höchstens 4
  gleichzeitig).
- **Sperre nach Leerlauf** (Vorgabe aus) und Sperren von Hand; nach dem Aufwachen aus dem
  Standby werden abgebrochene Verbindungen sofort neu aufgebaut, Tunnel zuerst.
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

1. Programm starten: `remotejoe`. Beim ersten Start fragt die Abdeckung des Hauptfensters nach der
   **Schlüsseldatenbank** (eine KeePass-Datei mit Ihren Zugangsdaten): **Vorhandene öffnen** oder **Datei →
   Neue Schlüsseldatenbank anlegen …** (Datei, Hauptkennwort mit Wiederholung, auf Wunsch eine Schlüsseldatei).
   Der Pfad wird gemerkt.
2. Struktur anlegen: im Baum **Hinzufügen** (Kontextmenü, Menü „Bearbeiten“ oder Strg+N):
   Ordner, RDP, SSH, VNC, Web, Proxmox, Hyper-V, PowerShell, Aufgabe, Makro oder aus einer
   Vorlage. Das neue Objekt öffnet seinen **Eigenschaften-Tab** (später über F4, Alt+Eingabe oder das Kontextmenü „Eigenschaften“; der Doppelklick auf ein Ziel verbindet):
   Allgemein, Verbindung, Zugangsdaten, Tunnel, Aufgaben und Makros, Zwischenablage,
   Organisation, Übergeordneter Ordner; **Speichern** im Tab.
   Oder **Datei → Importieren (RDM, Royal TS) …** mit Vorschau.
3. Zugangsdaten je Ordner oder Ziel: keine, vom übergeordneten Ordner, Benutzername und
   Kennwort direkt (neuer Eintrag in der Gruppe `remotejoe` der Schlüsseldatenbank), vorhandener Datensatz oder
   Datensatz nach Name. Der Tab zeigt, woher der wirksame Wert kommt.
4. Verbinden: Eingabetaste auf einem Ziel oder Kontextmenü **Verbinden**; für einen Ordner
   **Alle im Ordner verbinden**. Fehlt ein Eintrag oder kommen mehrere in Frage, fragt das
   Programm nach. Tunnel werden im Hintergrund aufgebaut (Fortschritt und „Abbrechen“ in der
   Statuszeile).
5. **Bearbeiten → Hinzufügen → Mehrere Ziele …** öffnet einen Tab: Hosts einfügen (ein Host
   pro Zeile, `#` für Kommentare), Vorschau prüfen, Strg+Eingabe legt an; Zeilen mit Fehlern
   bleiben im Feld stehen.
   Baum per Tastatur: F2 umbenennen, Entf löschen, Strg+D duplizieren, Strg+Umschalt+M
   verschieben (auch Ziehen und Ablegen, Alt+↑/↓), Strg+B Favorit, Strg+Z rückgängig.
   Über dem Baum lässt sich zwischen allen Zielen, Favoriten und eigenen Ansichten wählen.
6. Rückfragen zu unbekannten Host-Schlüsseln und Zertifikaten erscheinen als Leiste im Tab der
   Sitzung mit Schlüsseltyp und vollständigem Fingerabdruck; Standard ist „Ablehnen“, ohne
   Antwort wird nach 120 s abgelehnt. Ein geänderter Schlüssel wird deutlich gewarnt (alter und
   neuer Fingerabdruck) und lässt sich erst nach ausdrücklicher Bestätigung annehmen. Die
   Liste bekannter SSH-Host-Schlüssel (`known_hosts`) hat das Format von OpenSSH.
7. Kontextmenü **Aufgaben → Aktualisierungen prüfen** startet den Patchday für die gewählten
   Ziele (zuerst als Trockenlauf).
8. **Datei → Speichern** schreibt Schlüsseldatenbank und Index (atomar, mit Sicherungskopie `.bak`); hat
   jemand die Schlüsseldatenbank inzwischen geändert, fragt eine Leiste: eigene, fremde oder beide behalten.
   **Datei → Sperren** (Strg+L) verwirft Schlüssel und Kennwörter im Speicher; laufende
   Sitzungen bleiben bestehen, ihre Bilder sind verdeckt. **Extras → Sperre nach Leerlauf**
   schaltet das automatische Sperren ein; **Extras → SNMP-Geräte** verwaltet SNMP-Zugänge (v3
   mit getrennten Kennwörtern für Authentisierung und Verschlüsselung).

9. **Fußzeile:** zeigt für die vordere Sitzung den Verbindungstyp, das Ziel, seit wann und als wer
   Sie verbunden sind.
10. **Terminal:** **Extras → Terminal …** (Schrift, Größe, Farbschema mit Catppuccin und weiteren
    Schemata, eigene Farben); Strg + Mausrad ändert die Schriftgröße direkt im Terminal.
11. **Makros** (SSH): Im Baum liegt das Fach **Makros**; Makros werden Zielen oder Ordnern
    zugeordnet (vererbt). In der Kopfzeile jeder SSH-Sitzung legt **Makro anlegen** ein Makro für
    dieses Ziel an, **Makro ausführen ▾** startet eines der zugeordneten. Schritte sind „Senden“
    und „Warten auf“ (Text oder regulärer Ausdruck, Zeitlimit, Fehlermuster); das Kennwort kommt
    als Platzhalter aus den Zugangsdaten der Sitzung und steht nie im Protokoll.

Zum Ausprobieren ohne eigene Daten: `remotejoe --demo` (erfundene Ziele, Vorführsitzungen).

Einstellungen stehen in `~/.config/remotejoe/config.toml` (Windows:
`%APPDATA%\remotejoe\config.toml`), z. B.:

```toml
kdbx_path = "/home/benutzer/zugang.kdbx"

[connection]
connect_timeout_secs = 15
keepalive_secs = 30
auto_reconnect = false
connect_all_parallel = 4

[security]
lock_after_idle_mins = 0      # 0 = aus
disconnect_on_lock = false
```

Handprobe gegen echte Gegenstellen: [`docs/HANDPROBE.md`](docs/HANDPROBE.md).

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

Voraussetzungen: Visual Studio Build Tools (MSVC), Rust (`x86_64-pc-windows-msvc`), Qt 6
(Widgets, Svg), LLVM (für bindgen) und FreeRDP 3 über vcpkg. Schritt-für-Schritt-Anleitung mit
Stolpersteinen: [`docs/BUILD-WINDOWS.md`](docs/BUILD-WINDOWS.md). Das Skript
`scripts/build-windows.ps1` prüft die Voraussetzungen, baut alles und legt ein Paket mit den
nötigen DLLs an:

```powershell
.\scripts\build-windows.ps1 -VcpkgRoot C:\vcpkg -QtDir C:\Qt\6.5.3\msvc2019_64
```

## Lizenz

Apache-2.0, siehe [`LICENSE`](LICENSE) und [`NOTICE`](NOTICE). Verwendete Bibliotheken mit
ihren Lizenzen: [`THIRD_PARTY.md`](THIRD_PARTY.md). Aufbau des Programms:
[`docs/ARCHITEKTUR.md`](docs/ARCHITEKTUR.md).
