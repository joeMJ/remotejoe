# Architektur

remotejoe ist ein Hauptprogramm mit Oberfläche und je Verbindung einem eigenen
Worker-Prozess. Stürzt eine Verbindung ab oder hängt sie, bleiben Oberfläche und alle
anderen Sitzungen unberührt.

```
┌──────────────────────── remotejoe (Hauptprozess) ────────────────────────┐
│ Qt-Widgets (C++, crates/ui/cpp)  ⇄  RjBackend (cxx-qt)  ⇄  App (Rust)      │
│                                                     │                    │
│   KDBX + Index (rj_kdbx) · Vererbung (rj_core) · Tunnel (rj_tunnel)       │
│   Supervisor (rj_supervisor) · Aufgaben/Patchday (rj_tasks) · SNMP        │
└──────────┬──────────────┬──────────────┬──────────────┬──────────────────┘
   stdin/stdout     stdin/stdout    stdin/stdout    stdin/stdout  (+ Socket für Bildpuffer)
           │              │              │              │
   rj-worker-ssh   rj-worker-rdp   rj-worker-vnc   rj-worker-web  rj-worker-psrp
   (russh,         (FreeRDP 3,     (eigene RFB-    (WebKitGTK     (pwsh)
    Terminal)       C-Hülle)        Umsetzung)      ohne Fenster)
```

## Bausteine

| Paket (`crates/…`) | Aufgabe |
|---|---|
| `core` | Datenmodell des Index, Schema und Migration, Vererbung, Bearbeiten (`edit`: Einfügen, Verschieben, Duplizieren, Löschen, Ansichten, Vorlagen, Auf Unterobjekte anwenden, Verlauf), Zeichensatzprüfung, Kennwort-Typ `Secret`, Konfiguration, atomares Schreiben |
| `kdbx` | KDBX lesen/schreiben (KDBX 4), Zugangsdaten auflösen, Index-Ablage in der KDBX oder als `.rjindex`, Zusammenführen bei fremder Änderung |
| `ipc` | Nachrichten Hauptprozess ⇄ Worker, Rahmung, geteilter Bildspeicher |
| `supervisor` | Worker starten, Herzschlag, Absturzerkennung, Neustart mit Rückstau, Speichergrenze |
| `worker-ssh` | SSH-Client (russh), Terminal (alacritty_terminal), Host-Schlüssel, Makros |
| `worker-rdp` | RDP über FreeRDP 3 (eigene C-Hülle + bindgen), Zwischenablage, Tastaturbelegung |
| `worker-vnc` | VNC/RFB (Raw, CopyRect, Hextile, ZRLE), VeNCrypt mit TLS, erweiterte Zwischenablage |
| `worker-web` | WebKitGTK in einem `GtkOffscreenWindow` (kein sichtbares Fenster), Bild in den geteilten Speicher, Eingaben als GDK-Ereignisse, Formular-Anmeldung, Profil je Ziel, Zertifikatsrückfrage |
| `worker-psrp` | PowerShell-Remoting über lokales `pwsh` (WinRM oder SSH), Hyper-V-Befehle |
| `proxmox` | Proxmox-VE-REST, Aktionen, Konsole über VNC-Websocket mit lokaler Brücke |
| `tunnel` | SSH-Sprungserver-Ketten, WireGuard im Prozess (boringtun + smoltcp), System-Tunnel |
| `tasks` | Makros, apt-Aktualisierung, Windows-Update-Skript, Patchday (parallel) |
| `importer` | Import aus Remote Desktop Manager und Royal TS (Planen, dann Anwenden) |
| `snmp` | SNMP v2c/v3, Zähler mit Überlauf, Verlauf, Ampel |
| `ui` | Hauptprogramm `remotejoe`: Qt-Oberfläche, Sitzungsverwaltung, Arbeitsbereich, Bearbeitungsschicht (`editor`), Hintergrundaufgaben (`jobs`), Leerlauf und Standby (`idle`), überschreibende Speicherverwaltung (`alloc`) |
| `testkit` | Hilfen für Tests gegen lokale Server (sshd, xrdp, Xvnc, snmpd) |

## Prozessmodell

- Der Hauptprozess startet Worker als Kindprozesse über den Supervisor
  (`rj_supervisor::Supervisor::spawn`). Die Worker liegen neben dem Hauptprogramm.
- Zuordnung Protokoll → Worker: RDP und Hyper-V-Host → `rj-worker-rdp`, SSH →
  `rj-worker-ssh`, VNC → `rj-worker-vnc`, Web und Proxmox-Weboberfläche → `rj-worker-web`,
  PowerShell → `rj-worker-psrp`. Konsolen von Proxmox-Gästen laufen im VNC-Worker über eine
  lokale Brücke, Konsolen von Hyper-V-Gästen im RDP-Worker.
- Ablauf einer Sitzung: Worker starten → `Hello` (Prozessnummer, Art, Protokollversion) →
  `Connect` (Host, Port, Größe, Optionen als JSON, Zeitlimits) → `Credentials` →
  Zustandsmeldungen (`Connecting`, `Authenticating`, `Connected`, `Disconnected`, `Failed`).
- Herzschlag: Jeder Worker meldet sich regelmäßig. Bleibt der Herzschlag 10 s aus, gilt er als
  hängend und wird beendet.
- Absturzarten: Panik (Rückgabewert 101), Abbruch, Speicherzugriffsfehler, Hänger,
  Speichergrenze (Linux: belegter Speicher aus `/proc` und `RLIMIT_AS`; der Web-Worker ohne
  `RLIMIT_AS`, weil WebKit sehr viel Adressraum reserviert). Neustart, wenn eingeschaltet, mit
  Rückstau 1, 2, 4 … bis 60 s.
- Zeitlimits: Verbindungsaufbau 15 s, Keepalive bei Leerlauf 30 s (einstellbar in
  `config.toml`).

## Interprozesskommunikation (IPC)

- Transport: die Standardkanäle des Workers (anonyme Pipes). Kein Dateisystemeintrag, keine
  Rechtevergabe; der Kanal schließt sich mit dem Prozess. Unter Linux erhält jeder Worker
  zusätzlich ein Ende eines `SOCK_SEQPACKET`-Socket-Paars (Nummer in `RJ_FRAME_FD`), über das
  er nur Deskriptoren von Bildpuffern übergibt (`SCM_RIGHTS`).
  Unter Linux wird im Worker die ursprüngliche Standardausgabe für den Kanal dupliziert und
  Kanal 1 auf die Fehlerausgabe umgelenkt, damit Bibliotheken, die auf die Standardausgabe
  schreiben (z. B. FreeRDP), den Kanal nicht stören.
- Rahmen: Länge (u32, little endian) ‖ Kennung (u32) ‖ Nutzlast im Format `postcard`.
  Höchstens 64 MiB je Rahmen.
- Nachrichten an den Worker (`ToWorker`): `Connect`, `Credentials`, `Answer` (Antwort auf eine
  Rückfrage), `Input` (Tasten, Text, Maus), `Resize`, `Clipboard`, `Ping`, `Shutdown`, `Custom`.
- Nachrichten vom Worker (`FromWorker`): `Hello`, `Heartbeat`, `State`, `SharedFrame`, `Frame`
  (geänderter Bereich), `Output` (Terminalbytes), `Clipboard`, `Question` (Sicherheitsabfrage
  mit `SecurityInfo`: Art, Host, Port, Schlüsseltyp, vollständiger und bisheriger
  Fingerabdruck), `Notice`, `Log`, `Pong`, `Custom`. Protokollversion 3.
- Bilddaten (RDP, VNC, Web): anonymer geteilter Speicher ohne Dateinamen, BGRA mit 32 Bit je
  Bildpunkt; über den Kanal laufen nur die geänderten Rechtecke und eine laufende Nummer.
  Linux: `memfd` mit Versiegelung gegen Verkleinern und Vergrößern; der Hauptprozess prüft
  Versiegelung (`F_GET_SEALS`) und Größe und lehnt andere Deskriptoren ab, sodass ein Worker
  den Puffer nicht nachträglich verkürzen kann. Windows: anonymes Dateimapping, dessen Kennung
  der Hauptprozess aus dem Worker dupliziert.
- Sicherheitsrückfragen haben eine Frist (Vorgabe 120 s); danach, beim Schließen der Sitzung
  und bei einer späten Antwort gilt „Ablehnen“. Beim SSH-Aufbau zählt die Bedenkzeit nicht zum
  Zeitlimit des Verbindungsaufbaus.
- Kennwörter laufen nur in `Credentials` als Bytes, werden im Worker nach Gebrauch
  überschrieben und nie protokolliert. Kennwörter werden nie über die Zwischenablage in ein Ziel
  gebracht, sondern als Protokollereignisse (z. B. Tastenfolge, Anmeldedaten im Protokoll,
  eingespeistes Skript im Web-Worker).
- Protokolle (`Log`) enthalten nur Fehler und Verbindungsereignisse, nie Kennwörter,
  Tastatureingaben oder Inhalte der Zwischenablage.

## Oberfläche

- Widgets in C++ (Qt 6 Widgets), Logik in Rust. Die Brücke ist ein QObject `RjBackend`
  (cxx-qt), dessen Methoden JSON zurückgeben (Baum, Eigenschaften, Arbeitsbereich, Ereignisse).
- Die Oberfläche fragt Ereignisse der Sitzungen im Takt ab (`poll`) und zeichnet Bilder aus dem
  geteilten Speicher (nur lesend abgebildet).
- Texte der Oberfläche stehen in `crates/ui/res/texte.de.json`.
- Auffällige Zeichen in Namen (Steuerzeichen, Zeichen ohne Breite, Richtungswechsel) werden als
  `⟦U+XXXX⟧` angezeigt, nicht entfernt; Eingabefelder melden sie beim Tippen.
- Keine zusätzlichen Fenster der obersten Ebene (außer der Dateiauswahl des Betriebssystems und
  einem auf Wunsch abgerissenen Tab): Eigenschaften-Tab neben den Sitzungs-Tabs
  (`cpp/editor.cpp`), Rückfragen, Hinweise und Fehlermeldungen in einer Leiste im Fenster
  (`ActionBar`), Host-Schlüssel und Zertifikate in einer `SecurityBar` im Tab der Sitzung
  (Standard Ablehnen, voller Fingerabdruck, bei Änderung alter und neuer Fingerabdruck),
  Rückfragen vor dem Bestehen einer Sitzung (Zugangsdatensatz, Sprungserver, Proxmox-Zertifikat)
  im Tab „Verbinden mit …“, Entsperren als Abdeckung des Hauptfensters, SNMP-Geräte,
  Mehrere Ziele, Import-Vorschau und Patchday als Tabs. Alle Änderungen laufen über
  eine Brückenmethode `edit` (JSON-Befehle, `src/editor.rs`) und von dort über
  `Vault::set_index`/`Vault::save` (Sicherungskopie, atomares Schreiben, Sperre,
  Konfliktabfrage).

## Hintergrundaufgaben

- Was auf das Netz wartet, läuft nicht im Oberflächenfaden: Tunnelaufbau beim Verbinden,
  Gäste von Proxmox/Hyper-V abrufen, Gastaktionen, Proxmox-Konsole öffnen. Eine Aufgabe läuft
  in einem eigenen Faden und liefert eine Fortsetzung, die im Takt der Oberfläche (`poll`)
  angewandt wird (Sitzung öffnen, Ergebnis melden). Abbrechen: Die Oberfläche wartet nicht
  mehr, ein noch aufgebauter Tunnel wird freigegeben; das Warten auf das Netz endet über eine
  Abbruchmarke sofort, sodass die Tunnelverwaltung gleich wieder frei ist.
- „Alle im Ordner verbinden“ startet höchstens `connect_all_parallel` Aufbauten gleichzeitig;
  Fehler einzelner Ziele halten die übrigen nicht auf.

## Sperren, Leerlauf und Standby

- Sperren (von Hand oder nach Leerlauf, Vorgabe aus): Tresor, Import-Plan und Verlauf werden
  verworfen, die Zugangsdaten der Sitzungen überschrieben; Sitzungen bleiben verbunden
  (Option: trennen), Neuverbinden wartet auf das Entsperren. Das Hauptprogramm verwendet eine
  Speicherverwaltung, die jeden freigegebenen Block mit Nullen überschreibt.
- Patchday: Jede Aufgabe übernimmt ihre Zugangsdaten beim Start; im Speicher des Laufs bleibt
  keine Kopie. Sperren löscht noch nicht übernommene Kopien; braucht eine Aufgabe die
  Zugangsdaten erneut (zweiter Versuch) oder startet sie später, werden sie erst nach dem
  Entsperren nachgeliefert.
- Leerlauf zählt mit der monotonen Uhr; Standby zählt nicht. Aufwachen wird über logind
  (D-Bus `PrepareForSleep`), unter Windows über `WM_POWERBROADCAST` und als Rückfall über einen
  Sprung der Systemuhr erkannt. Danach wird nie gesperrt; zwei Minuten lang werden abgebrochene
  Sitzungen sofort neu verbunden, Sitzungen über einen Tunnel erst nach dem Neuaufbau des
  Tunnels. Proxmox-Konsolen werden beim Neuverbinden neu geöffnet (neues Ticket, neue Brücke;
  die alte Brücke schließt Websocket und Client-Verbindung).

## Tunnel

- Ein Tunnel wird vor dem Start des Workers aufgebaut; der Worker verbindet sich dann mit einem
  lokalen Endpunkt (127.0.0.1, zufälliger Port). Mehrere Sitzungen teilen einen Tunnel
  (Zählung im `TunnelManager`), er schließt mit der letzten Sitzung.
- Arten: SSH-Kette über einen oder mehrere Sprungserver, WireGuard im Prozess (kein
  Systemadapter, keine Administratorrechte), System-Tunnel (nur Erreichbarkeitsprobe, z. B.
  für ein bereits laufendes VPN).
