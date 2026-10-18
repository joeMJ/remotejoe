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
| `worker-ssh` | SSH-Client (russh), Terminal-Zustandsmaschine im Worker (alacritty_terminal, Schnappschüsse), Host-Schlüssel, Makros |
| `worker-rdp` | RDP über FreeRDP 3 (eigene C-Hülle + bindgen), Zwischenablage, Tastaturbelegung |
| `worker-vnc` | VNC/RFB (Raw, CopyRect, Hextile, ZRLE), VeNCrypt mit TLS, erweiterte Zwischenablage |
| `worker-web` | WebKitGTK in einem `GtkOffscreenWindow` (kein sichtbares Fenster), Bild in den geteilten Speicher, Eingaben als GDK-Ereignisse, Formular-Anmeldung, Profil je Ziel, Zertifikatsrückfrage |
| `worker-psrp` | PowerShell-Remoting über lokales `pwsh` (WinRM oder SSH), Hyper-V-Befehle |
| `proxmox` | Proxmox-VE-REST, Aktionen, Konsole über VNC-Websocket mit lokaler Brücke |
| `tunnel` | SSH-Sprungserver-Ketten, WireGuard im Prozess (boringtun + smoltcp), System-Tunnel |
| `tasks` | Makros, apt-Aktualisierung, Windows-Update-Skript, Patchday (parallel) |
| `importer` | Import exportierter Dateien anderer Verbindungsmanager (Planen, dann Anwenden) |
| `update` | Update-Funktion: SemVer, Abruf von Veröffentlichungen (Gitea, GitHub), HTTPS mit Systemzertifikaten, Ed25519-Signatur und SHA-256, sicheres Entpacken, Installation mit `.alt`-Sicherung |
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
  Rückfrage), `Input` (Tasten, Text, Maus), `Resize`, `Clipboard`, `Ping`, `Shutdown`, `Custom`,
  `Term` (Befehle an das Terminal im SSH-Worker).
- Nachrichten vom Worker (`FromWorker`): `Hello`, `Heartbeat`, `State`, `SharedFrame`, `Frame`
  (geänderter Bereich), `Output` (Terminalrohbytes, nur mit Abgriff für Makros), `Clipboard`,
  `Question` (Sicherheitsabfrage mit `SecurityInfo`: Art, Host, Port, Schlüsseltyp,
  vollständiger und bisheriger Fingerabdruck), `Notice`, `Log`, `Pong`, `Custom`, `Term`
  (Schnappschuss und Ereignisse des Terminals). Neue Varianten stehen immer am Ende der
  Aufzählungen, weil `postcard` Varianten nach ihrer Reihenfolge kodiert. Protokollversion 4.
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

## Terminal (SSH)

Die Terminal-Zustandsmaschine (`alacritty_terminal`, `worker-ssh/src/terminal.rs`) läuft im
SSH-Worker, nicht im Hauptprozess. Ein Absturz oder Hänger des Emulators durch bösartige
Terminalausgabe trifft so nur diese eine Sitzung (Zustand „abgestürzt“, Neuverbinden startet
einen neuen Worker); der Hauptprozess verarbeitet keine Terminalbytes des Ziels mehr.

- **Worker** (`host.rs`, `TermHost`): füttert das Terminal mit der Ausgabe des Ziels, beantwortet
  Anfragen des Terminals (z. B. Cursorposition) selbst am Ziel, bildet beim Einfügen die Bytes
  (Zeilenenden, Klammerung `ESC[200~ … ESC[201~` bei gesetztem `?2004h`) und führt die Befehle
  der Oberfläche aus (`TermCmd`: Scrollen, Seite, ans Ende, Farbschema, Rücklaufgröße, Einfügen,
  Antwort auf eine Leseanfrage, Abgriff). Er schickt **zusammengefasste Schnappschüsse**
  (`TermMsg::Snapshot`): höchstens alle 33 ms und nur bei Änderung, sofort nach einem Befehl,
  der die Anzeige ändert (Scrollen, Größe, Farben). Ereignisse (`TermMsg::Event`): Zwischenablage
  setzen (OSC 52), Leseanfrage mit Kennung, Titel, Klingel; Fluten innerhalb eines Pakets werden
  zusammengefasst.
- **Schnappschuss** (`TermView`): je Zeile Läufe gleicher Darstellung (Text, Vordergrund,
  Hintergrund, Kennzeichen), Größe, Cursor, Alt-Bildschirm, Rücklaufstand, Farben des Schemas;
  dazu `grid` (je Zeile eine Spalte je Eintrag, damit die Oberfläche die Auswahl ohne Rückfrage
  bilden kann) und `tail` (die letzten Zeilen für den Start eines Makros). Ein Schnappschuss
  umfasst nur die sichtbaren Zeilen und bleibt auch bei 400 × 120 unter 1 MiB; übersteigt er
  8 MiB, entfallen die Zellentexte.
- **Oberfläche** (`ui/src/sessions.rs`): hält je SSH-Sitzung nur den zuletzt empfangenen
  Schnappschuss (nach Prüfung von Maßen und Zeilenzahlen), baut daraus `TermSnapshot` für
  `terminal_json` (Form und Feldnamen unverändert) und den Auswahltext mit
  `selection_text_from_grid` (dasselbe Ergebnis wie `Terminal::select`). Eingaben, Scrollen,
  Einfügen und Größenänderung gehen als `ToWorker::Term(..)` bzw. `Resize` an den Worker.
- **Makros**: Die Engine liest Rohbytes. Dafür schaltet die Oberfläche beim Start eines Makros
  `Tap(true)` und danach `Tap(false)`; nur dazwischen schickt der Worker zusätzlich
  `FromWorker::Output`. Der Stand des Bildschirms beim Start kommt aus `tail`.
- **Vorführsitzungen** (`open_demo_terminal`, Bildschirmfotos): haben keinen Worker und halten
  ein lokales `Terminal`, das nie Bytes von einem Ziel bekommt.
- **Schutz**: Mindestgröße 2 × 1 und Höchstgrößen für Spalten, Zeilen und Zellen; Begrenzung
  kombinierender Zeichen je Zelle, der Titellänge und der Zeilen des Endes; ein Wächter beendet
  den Worker, wenn eine Verarbeitung des Terminals länger als 10 s dauert (der Herzschlag läuft
  in einem eigenen Faden und würde den Hänger nicht anzeigen). Der Speicher des Terminals zählt
  nun zur Speichergrenze des Workers (1 GiB).
- **Testbau**: Mit dem Merkmal `test-hooks` (nur als Entwicklungsabhängigkeit der Oberfläche,
  nie im Auslieferungsbau) löst bestimmte Ausgabe eine Panik oder einen Hänger im Terminal des
  Workers aus; die Isolationstests prüfen damit den Absturz einer einzelnen Sitzung.

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

## Web-Sitzungen ohne eigenes Fenster

- Der Web-Worker (`crates/worker-web`, Merkmal `webview`) startet GTK und WebKitGTK, aber
  zeigt kein Fenster: Die WebView steckt in einem `GtkOffscreenWindow`, Hardwarebeschleunigung
  ist aus. Das Fenster meldet geänderte Bereiche (`damage-event`); im 16-ms-Takt wird deren
  Vereinigung aus der Oberfläche des Fensters in den versiegelten Bildpuffer kopiert und als
  `Frame` gemeldet – derselbe Weg wie bei RDP und VNC. Die Oberfläche zeigt ihn im `FrameView`
  des Tabs; jede Kachel hat ihren eigenen Worker, ihr eigenes Bild und ihre eigene Größe.
- Größe: `Resize` aus dem `FrameView` (dort schon entprellt) wird nach weiteren 120 ms
  übernommen; ein neuer Bildpuffer passender Größe wird angelegt und übergeben.
- Eingaben (`Input`) werden zu GDK-Ereignissen (Bewegung, Tasten, Rad, Taste mit Keysym,
  Unicode-Text) und an GTK übergeben (`gtk_main_do_event`). WebKit verarbeitet Maus- und
  Tastenereignisse in getrennten Warteschlangen; damit die Reihenfolge stimmt, warten Tasten
  60 ms nach einem Klick und Bearbeiten-Befehle 120 ms nach der letzten Taste.
- Erweiterung des Kanals für Web (`Custom`, JSON): an den Worker `nav` (Zurück, Vor, Neu laden),
  `focus`, `edit` (Kopieren, Ausschneiden, Einfügen, Alles auswählen), `menu` (Auswahl);
  vom Worker `status` (Seitentitel, Laden, Fehler, Zurück/Vor möglich) und `menu`
  (Kontextmenü, Auswahlliste), die die Oberfläche als Popup des Hauptfensters zeigt.
- Zwischenablage: Strg+C/X und „Kopieren“ lesen die Markierung per Skript in einer eigenen
  Skriptwelt und schicken sie als `Clipboard` an das Hauptprogramm (Größenlimit des Ziels);
  Strg+V fügt den vom Hauptprogramm angebotenen Text als Bearbeiten-Befehl ein. Die
  Zwischenablage des Systems berührt der Worker nicht; aus Kennwortfeldern wird nichts kopiert.
- Neue Fenster (`target=_blank`, `window.open`) öffnen im selben Tab; Vollbild und
  Hinweisfenster (Tooltips) sind gesperrt.
- Windows (WebView2): noch nicht umgesetzt; der Worker meldet das und öffnet kein Fenster.

## Update

- Quelle in `config.toml` (`[update]`): `gitea` (Basisadresse) oder `github`, Repository, Verweis
  auf den KDBX-Eintrag mit dem Zugriffsschlüssel, Prüfung beim Start (aus).
- Prüfen und Installieren laufen als Hintergrundaufgaben; Ergebnis als `JobResult`
  (`update_check`, `update_install`) an den Tab „Programm-Update“.
- Ablauf: `releases/latest` → Version nach SemVer vergleichen → `SHA256SUMS.sig` und
  `SHA256SUMS` laden → Signatur mit dem eingebauten Schlüssel (`crates/ui/res/update-key.pub`)
  prüfen → Archiv laden und SHA-256 prüfen → in `<programm>.neu` entpacken → Programmverzeichnis
  nach `<programm>.alt` verschieben und `.neu` an seine Stelle setzen.
- Einzelheiten und Werkzeug für Herausgeber: [`UPDATE.md`](UPDATE.md).

## Tunnel

- Ein Tunnel wird vor dem Start des Workers aufgebaut; der Worker verbindet sich dann mit einem
  lokalen Endpunkt (127.0.0.1, zufälliger Port). Mehrere Sitzungen teilen einen Tunnel
  (Zählung im `TunnelManager`), er schließt mit der letzten Sitzung.
- Arten: SSH-Kette über einen oder mehrere Sprungserver, WireGuard im Prozess (kein
  Systemadapter, keine Administratorrechte), System-Tunnel (nur Erreichbarkeitsprobe, z. B.
  für ein bereits laufendes VPN).
