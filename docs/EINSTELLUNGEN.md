# Einstellungen

**Extras → Einstellungen** öffnet die Einstellungen als Tab. Sie gelten für das ganze Programm; Änderungen wirken
sofort und werden gespeichert. Links stehen die Bereiche in Gruppen, rechts die Seite des gewählten Bereichs.
Einstellungen einzelner Ziele und Ordner (Verbindung, Zugangsdaten, Inventar, Tunnel …) stehen nicht hier, sondern
in den **Eigenschaften** des Ziels bzw. Ordners (F4).

Übersicht:

| Gruppe | Bereich | Worum es geht |
|---|---|---|
| Sicherheit | [Sperre und Zwischenablage](#sperre-und-zwischenablage) | Automatisch sperren, kopierte Kennwörter löschen, Schlüsselfehler |
| | [Tastatur](#tastatur) | Tastenfolgen, Tastenkürzel in Sitzungen, Tastatur in RDP |
| | [Cipher-Suites](#cipher-suites) | Ältere SSH-Verfahren für Altgeräte |
| Darstellung | [Darstellung](#darstellung) | Farbschema, Baum, Zielbereich, Suche im Zielbaum, Symbolfarben |
| | [Terminal](#terminal) | Schrift, Farben und Rücklauf des SSH-Terminals |
| Daten | [Sicherung](#sicherung) | Sicherungen der Schlüsseldatenbank |
| | [Web-Zwischenspeicher](#web-zwischenspeicher) | Platz der Web-Ziele |
| Claude | [Claude-Anbindung (MCP)](#claude-anbindung-mcp) | Abfragen aus Claude Desktop |
| Updates | [Programm-Updates](#programm-updates) | Nach Updates suchen, Proxy, Installation |

In remotejoe advanced kommen weitere Bereiche hinzu (Lizenz, Erweiterungen und die Bereiche der Erweiterungen).

---

## Sicherheit

### Sperre und Zwischenablage

| Einstellung | Vorgabe | Wirkung |
|---|---|---|
| Sperre nach Leerlauf | Aus | Nach der eingestellten Zeit ohne Eingabe wird die Schlüsseldatenbank gesperrt; zum Weiterarbeiten fragt remotejoe das Hauptkennwort ab. Von Hand: **Datei → Sperren** (Strg+L). |
| Sitzungen beim Sperren trennen | Aus | Ohne diese Option bleiben laufende Sitzungen bestehen, ihre Bilder sind verdeckt. |
| Kennwort-Kopie löschen | 30 s | So lange bleibt ein kopiertes Kennwort in der Zwischenablage; 0 = nie löschen. |
| Bei Schlüsselfehlern trotzdem fortfahren | Aus | Unbekannte oder geänderte Host-Schlüssel und Zertifikate werden ohne Rückfrage angenommen – nur für diese eine Verbindung, gespeichert wird nichts; jeder Fall erscheint rot in der Fußzeile. **Vorsicht:** Ein geänderter Schlüssel kann auf einen fremden Server dazwischen hindeuten. |

### Tastatur

| Einstellung | Vorgabe | Wirkung |
|---|---|---|
| Tastenkürzel an die Sitzung geben | An | Windows-Taste, Alt+Tab und Menütaste gehen an das Ziel, solange die Sitzung den Fokus hat. |
| Strg+Umschalt+W | Nur außerhalb von Sitzungen | Schließt den aktiven Tab. „Nur außerhalb von Sitzungen“: In einer Sitzung geht die Tastenfolge an das Ziel. |
| Strg+F | Nur außerhalb von Sitzungen | Springt in die Suche über dem Zielbaum. In Web-Sitzungen sucht Strg+F immer in der Seite. |
| Tab wechseln (Tab nach links / rechts) | Strg+Umschalt+Bild↑ / Bild↓ | Wechselt zum Tab links bzw. rechts vom aktiven (mit Umlauf). Zum Belegen in das Feld klicken und die Tastenkombination drücken; leer = keine. Strg+Bild↑/↓ wechselt unabhängig davon immer. **Extras → Tastenkürzel …** öffnet diese Seite direkt. |
| Belegung im administrativen Tastaturmodus | Deutsch | Gilt für RDP-Sitzungen, deren Ziel keine eigene Tastaturbelegung nennt; muss zur Belegung der Sitzung am Server passen. Hintergrund: [TASTATUR.md](TASTATUR.md). |
| Administrativen Tastaturmodus abschalten nach | 10 Min. ohne Eingabe | Danach gilt wieder die Vorgabe (Zeichen als Unicode). |

### Cipher-Suites

Ältere SSH-Verfahren gelten als unsicher: Verbindungen damit lassen sich leichter mitlesen oder verfälschen. Nur für
Altgeräte einschalten, die nichts anderes können – und vorher prüfen, ob für das Gerät neuere Firmware verfügbar ist.

| Einstellung | Vorgabe | Wirkung |
|---|---|---|
| Ältere SSH-Verfahren zulassen | Aus | SHA-1-Schlüsselaustausch (Diffie-Hellman-Gruppe 14 und Gruppenaustausch), AES-CBC, HMAC-SHA1 und DSA-Schlüssel. Sie werden nur angeboten, wenn ein Gerät die aktuellen nicht kann, und stehen immer ganz hinten. |
| Auch sehr alte Verfahren zulassen | Aus | 3DES und Diffie-Hellman-Gruppe 1; setzt die Stufe darüber voraus. |

Die Option „Ältere Verfahren“ an einem Ziel bzw. Tunnel wirkt nur mit dieser Freigabe. Ohne sie meldet remotejoe bei
Altgeräten „Schlüsselaustausch gescheitert“ und verweist auf diese Seite.

---

## Darstellung

### Darstellung

| Einstellung | Vorgabe | Wirkung |
|---|---|---|
| Modus | Automatisch nach System | Automatisch hell oder dunkel wie das System (je eine Palette für den hellen und den dunklen Modus) oder immer dieselbe Palette. Klick auf eine Kachel wählt die Palette. |
| Blauen Rahmen um die Sitzung mit dem Tastaturfokus zeigen | An | Zeigt, welche Sitzung die Tastatur bekommt (vor allem beim Kacheln). |
| Größe des Baums | Mittel | Zeilenhöhe, Symbole und Schrift des Zielbaums: klein, mittel, groß. |
| Runde Fensterecken | An | Wirkt nach einem Neustart (unter Windows 11 rundet das System selbst). |
| Favoriten oben im Zielbaum anzeigen | An | Ordner „Favoriten“ ganz oben im Baum. |
| Zielbereich aufklappen nach / zuklappen nach | 300 ms / 500 ms | Gilt, wenn der Zielbereich nicht angeheftet ist (Stecknadel in der Kopfzeile „Ziele“): Er öffnet sich beim Verweilen über „Ziele“ und klappt nach dem Verlassen wieder ein. |
| **Suche im Zielbaum:** Suchen ab | 4 Zeichen | Beim Tippen sucht remotejoe erst ab so vielen Zeichen; darunter steht unter dem Suchfeld, wie viele noch fehlen. Enter sucht immer sofort. |
| **Suche im Zielbaum:** Suche starten | beim Tippen | „beim Tippen“ oder „erst mit Enter“. |
| **Suche im Zielbaum:** Beim Tippen warten | 3 s | Pause nach dem letzten Tastendruck, bevor die Suche startet. |
| Symbolfarben | Einheitlich | Einheitlich oder je Typ; mit „Eigene Farben“ je Art eine eigene Farbe. |

Eigene Paletten: **Eigene Palette anlegen …** oder als JSON-Datei im Paletten-Ordner (Felder `name`, `dark` und die
Farben `window`, `base`, `alternate`, `text`, `muted`, `highlight`, `highlight_text`, `button`, `link`, `warning`,
`error`, `ok` als `#rrggbb`).

### Terminal

Schrift und Farben des SSH-Terminals; Änderungen gelten sofort für alle offenen und neuen Terminals. Auch über
**Extras → Terminal …**.

| Einstellung | Wirkung |
|---|---|
| Festbreitenschrift des Systems verwenden, Schriftart, Schriftgröße | Schrift des Terminals; Strg + Mausrad ändert die Größe direkt im Terminal. |
| Farbschema, Vordergrund, Hintergrund, Cursor, Auswahl | Schema (z. B. Catppuccin) und einzelne Farben; „Auf Vorgabe zurücksetzen“. |
| Rücklauf | Zeilen, die das Terminal zum Zurückblättern behält. |
| Markierten Text sofort in die Zwischenablage kopieren | Wie unter Linux üblich. |
| Vor dem Einfügen mehrerer Zeilen nachfragen | Zeilenumbrüche wirken wie die Eingabetaste – die Rückfrage schützt vor versehentlich ausgeführten Befehlen. |

---

## Daten

### Sicherung

| Einstellung | Vorgabe | Wirkung |
|---|---|---|
| KDBX nach jedem Speichern sichern | An | Gesichert wird die verschlüsselte Datei mit Zeitstempel im Namen (dazu der Index `.rjindex`); Kennwort und Schlüsseldatei bleiben dieselben. |
| Ordner | neben der KDBX | Ort der Sicherungen. |
| Neueste behalten | 10 | So viele der letzten Sicherungen bleiben. |
| Dazu je Tag die erste | 7 Tage | Zusätzlich je Tag die erste Sicherung. |
| Vor einem Update | 3 | Vor jedem Update entsteht eine eigene Sicherung „…-vor-update-<Version>-…“. |
| Zusätzlich an einem zweiten Ort sichern | Aus | z. B. ein NAS (`\\nas\freigabe\remotejoe` bzw. `/mnt/nas/remotejoe`); ist er nicht erreichbar, wird trotzdem gespeichert. |

**Jetzt sichern** sichert sofort. Eine Schlüsseldatei wird nicht mitgesichert – bitte getrennt aufbewahren, nicht neben
den Sicherungen. Aufgeräumt werden nur Dateien nach dem Namensmuster der Sicherungen.

### Web-Zwischenspeicher

Web-Ziele speichern Teile der Seiten zwischen, damit sie schneller laden.

| Einstellung | Vorgabe | Wirkung |
|---|---|---|
| Grenze je Ziel | 50 MB | Liegt der Zwischenspeicher eines Ziels darüber, wird er geleert. |
| Aufbewahrung | 7 Tage | Wurde ein Ziel so lange nicht geöffnet, wird sein Zwischenspeicher geleert; Profile gelöschter Ziele werden nach dieser Zeit entfernt. |

Die Seite zeigt den belegten Platz. **Zwischenspeicher jetzt aufräumen** leert sofort alle Zwischenspeicher. Anmeldungen
auf Webseiten und angenommene Zertifikate bleiben immer erhalten. Aufgeräumt wird nach dem Entsperren.

---

## Claude-Anbindung (MCP)

Ab Werk aus. Einrichtung, Werkzeuge und Datenschutz: [MCP.md](MCP.md).

| Einstellung | Vorgabe | Wirkung |
|---|---|---|
| Claude-Anbindung einschalten | Aus | Claude Desktop kann remotejoe abfragen. |
| IP- und MAC-Adressen ausblenden / Seriennummern ausblenden | Aus | Diese Angaben gehen dann nicht an Claude. |
| Freigegebene Ordner | keine | Claude sieht nur Ziele in diesen Ordnern (mit Unterordnern); gilt je Schlüsseldatenbank. |
| Anmeldeobjekte für Aktionen | keine | Aktionen (Verbinden, Inventar abfragen) nur bei Zielen, deren Zugangsdaten aus einem dieser Anmeldeobjekte stammen – z. B. ein Konto mit wenig Rechten eigens für Claude. Jede Aktion erlaubst du zusätzlich in remotejoe. |

Zugangsdaten, Benutzernamen und Notizen gehen nie an Claude.

---

## Updates

### Programm-Updates

| Einstellung | Vorgabe | Wirkung |
|---|---|---|
| Beim Programmstart nach Updates suchen | Aus | Prüft im Hintergrund die Veröffentlichungen auf GitHub. |
| In regelmäßigen Abständen nach Updates suchen | Aus | Alle n Stunden, Tage oder Wochen, solange remotejoe läuft. |
| Proxy | Automatisch (Systemeinstellung) | Automatisch, kein Proxy oder manuell (`Rechner:Port`). |

**Jetzt suchen** prüft sofort; **Herunterladen und installieren** lädt das Paket, prüft Signatur und Prüfsumme und
installiert. Der Neustart ist freiwillig; bis dahin läuft die bisherige Fassung weiter. Das Protokoll der Seite lässt sich
kopieren. Ablauf und Hintergründe: [UPDATE.md](UPDATE.md).

Windows: Läuft gerade eine andere Installation (z. B. Windows Update), wartet remotejoe und installiert danach von selbst
(höchstens 10 Minuten, „Nicht warten“ bricht ab). Jede Installation schreibt ein Protokoll in den Datenordner
(`update\`, die letzten 5); eine Fehlermeldung nennt die Datei.

---

## Weitere Einstellungen außerhalb dieses Tabs

- **Aufklappzustand als Start speichern / Startzustand zurücksetzen** (Rechtsklick in den Zielbaum): legt fest, welche
  Ordner beim Programmstart offen sind (je Schlüsseldatenbank). **Alle zuklappen** (Knopf in der Kopfzeile „Ziele“)
  klappt den ganzen Baum auf einmal zu.
- **Hilfe → Neuigkeiten**: die Releasenotes; nach einem Update öffnen sie sich einmal von selbst.
- Startparameter `--ohne-erweiterungen`: startet remotejoe ohne Erweiterungen (nur remotejoe advanced; Notausgang, falls
  eine Erweiterung den Start verhindert).
