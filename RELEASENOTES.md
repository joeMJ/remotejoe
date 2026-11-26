## 0.1.73 – Fenstergröße unter Windows

- Windows: Nach dem Öffnen einer Web-Seite ließ sich das Fenster nicht mehr über die Ränder in der Größe ändern. Das geht jetzt wieder an allen Kanten und Ecken, auch mit offener Web-Seite. Reicht eine Web-Seite bis an den Fensterrand, bleibt neben ihr ein schmaler Streifen zum Ziehen frei.

## 0.1.72 – Kontextmenü aufgeräumt

- Zielbaum, Kontextmenü: neues Untermenü „Umwandeln“; Umwandeln und „Aufgaben“ stehen jetzt in einem eigenen Abschnitt zwischen Trennlinien.
- „Benutzernamen kopieren“ und „Kennwort kopieren“ sind rot hinterlegt, damit sie im Menü auffallen.
- Hilfe → Neuigkeiten: die Menüpunkte ohne Auslassungspunkte.
- Übersicht: Ziele mit erkannter Geräteart (Switch, Controller, AP, Router) zeigen immer die Geräte-Anordnung – Laufzeit, IP-Adresse, Seriennummer und Version als Kacheln, die MAC in der Kopfzeile.
- Zielbaum: Router haben ein eigenes Symbol.
- SNMP: Geräte, die eine sehr große Nachrichtengröße angeben, lassen sich lesen (angepasste Kopie der Bibliothek snmp2).
- Datenschutzhinweise: neuer Absatz zu Ansprechpartnern an Ordnern; sie sind freiwillig und bleiben in der verschlüsselten Schlüsseldatenbank.

## 0.1.71 – Neuigkeiten schließen

- Hilfe → Neuigkeiten: Der Tab „Neuigkeiten Erweiterungen“ lässt sich wieder über das Kreuz schließen (in 0.1.69 und 0.1.70 blieb er offen).

## 0.1.70 – Einfügen unter Ubuntu 24.04

- Linux (Ubuntu 24.04): remotejoe stürzte beim Einfügen aus der Zwischenablage in ein mehrzeiliges Feld ab (z. B. beim Einfügen der Lizenz, per Rechtsklick → Einfügen oder Strg+V). Ursache war die Qt-Fassung von Ubuntu 24.04 unter Wayland. remotejoe fügt in diese Felder jetzt selbst als reinen Text ein; der Absturz tritt nicht mehr auf.

## 0.1.69 – Suche im Zielbaum

- Suche im Zielbaum: Eine Suche klappt nur noch die Ordner auf dem Weg zu Treffern auf; ein gefundener Ordner bleibt zu (sein Inhalt ist mit dem Pfeil erreichbar). Nach der Suche kommt der Aufklappzustand von vorher zurück. Behebt das bekannte Problem aus 0.1.68.
- Suche einstellbar (Einstellungen → Darstellung → Suche im Zielbaum): Suchen ab 4 Zeichen, Suche beim Tippen oder erst mit Enter, beim Tippen 3 Sekunden Pause (alles änderbar). Enter sucht immer sofort, auch mit weniger Zeichen. Unter dem Suchfeld steht, wie viele Zeichen noch fehlen.
- Enter im Suchfeld wirkt wieder (sucht und springt in den Baum); bisher kam die Taste dort nicht an.
- Schlüsseldatenbank: Enthält sie etwas, das erst eine neuere Fassung lesen kann, steht die nötige Fassung in der Datei; ältere Fassungen ab 0.1.68 melden das klar, statt mit einem Ladefehler abzubrechen.

## 0.1.68 – Windows-Update, Tabs und Zielbaum

**Windows-Update:** Läuft gerade eine andere Installation (z. B. Windows Update), wartet remotejoe jetzt, statt mit einer Fehlermeldung des Windows Installers abzubrechen, und installiert danach von selbst (Issue #1).

- Windows-Update: „Windows installiert gerade etwas anderes – warte, das Update startet danach von selbst …“ mit „Nicht warten“, höchstens 10 Minuten. Fehler des Windows Installers werden verständlich gemeldet, mit Hinweis, was zu tun ist. Jede Installation schreibt ein Protokoll in den Datenordner (`update\`, die letzten 5); die Meldung nennt die Datei.
- Zielbaum: Neuer Knopf „Alle zuklappen“ in der Kopfzeile „Ziele“ (neben der Stecknadel). Er klappt den ganzen Baum auf einmal zu; der Baum bleibt auch nach einer Suche zu.
- Tabs: Das Rechtsklick-Menü eines Tabs hat „Tab schließen“ und „Alle Tabs schließen“ (mit Rückfrage).
- Übersicht: Erweiterungen können festlegen, welche vier Kacheln die Übersicht eines Ziels zeigt.
- Schlüsseldatenbank: Braucht eine Datenbank eine neuere Fassung von remotejoe, meldet remotejoe ab dieser Fassung klar „Diese Schlüsseldatenbank braucht remotejoe … oder neuer“ und lässt die Daten unverändert, statt mit einem unverständlichen Ladefehler abzubrechen.

**Bekannte Probleme:**

- Suche im Zielbaum: Eine Suche klappt alle Ordner auf, auch die gesamten Inhalte gefundener Ordner. Nach der Suche bleibt der Baum teilweise ganz aufgeklappt. Abhilfe bis zur Behebung: „Alle zuklappen“.
- Windows: Der Zielbaum kann nach dem Start ganz aufgeklappt sein, auch wenn ein Startzustand gespeichert ist. Die Ursache wird untersucht. Abhilfe: „Alle zuklappen“.

## 0.1.67 – Speicherplatz der Web-Ziele, robuste Erweiterungen

**Speicherplatz:** In 0.1.66 hat das Aufräumen des Web-Zwischenspeichers die eigentlichen Platzfresser nicht erfasst. Das ist jetzt behoben. Nach dem ersten Entsperren räumt remotejoe den Altbestand im Hintergrund weg.

- Linux: Die Web-Engine leerte den Speicher mancher Seiten nie; eine Controller-Oberfläche brachte es auf über 1 GB pro Stunde. Web-Ziele halten die Daten der Seite jetzt nur noch während der Sitzung, Anmeldungen bleiben erhalten. Was sich eine Seite selbst merkt (z. B. Ansichtseinstellungen), gilt nur bis zum Schließen des Tabs.
- Windows: Jedes Web-Ziel hatte einen eigenen Browser-Ordner mit rund 70 MB nachgeladener Zusatzteile. Alle Web-Ziele teilen sich jetzt einen Ordner, mit eigenem Profil je Ziel; die Zusatzteile werden nicht mehr geladen. Gespeicherte Anmeldungen auf Webseiten gehen dabei einmalig verloren, remotejoe meldet automatisch neu an.
- Erweiterungen laden: über eine einzige Verbindung statt vieler einzelner. Das hilft bei Anschlüssen, die viele Verbindungen kurz hintereinander verwerfen. Zusammengehörige Erweiterungen werden nur gemeinsam aktualisiert. Netzfehler werden als Netzfehler gemeldet, mit Schritt und Server, statt „nicht in der Update-Quelle“.
- Erweiterungen aus Datei (neu, Einstellungen → Updates → Erweiterungen): „Aus Datei einspielen …“ mit Prüfung vor dem Einspielen; nur mit laufender Wartung.
- Kein Absturz mehr durch unpassende Erweiterungen: Unverträgliche Fassungen werden nicht geladen. Endete ein Start unerwartet, startet remotejoe einmal ohne Erweiterungen und zeigt eine Leiste. Notausgang: Startparameter `--ohne-erweiterungen`.

## 0.1.66 – Web-Zwischenspeicher und Suche

*Veröffentlicht am 23.11.2026*

**Platz zurück:** Web-Ziele haben ihren Zwischenspeicher bisher unbegrenzt aufbewahrt; der Datenordner konnte dadurch mehrere GB groß werden. Nach dem ersten Entsperren mit 0.1.66 räumt remotejoe einmal im Hintergrund auf. Anmeldungen und angenommene Zertifikate bleiben erhalten.

- Web-Zwischenspeicher (neu, Einstellungen → Daten): Grenze je Ziel (Vorgabe 50 MB) und Aufbewahrung (Vorgabe 7 Tage), beides einstellbar. Liegt der Zwischenspeicher eines Ziels über der Grenze oder wurde das Ziel so lange nicht geöffnet, wird er geleert; Profile gelöschter Ziele werden nach dieser Zeit entfernt. Die Seite zeigt den belegten Platz; „Zwischenspeicher jetzt aufräumen“ leert sofort alle Zwischenspeicher und meldet, wie viel frei geworden ist. „Felder erkennen“ hinterlässt kein eigenes Profil mehr.
- Suche im Zielbaum: Nach dem Leeren der Suche kommt der vorherige Aufklappzustand zurück, auch wenn sich der Baum während der Suche neu aufgebaut hat. Bisher blieb der Baum nach einer Suche mit vielen Treffern ganz aufgeklappt.

## 0.1.65 – Tab-Wechsel, Cipher-Suites und Web-Anmeldung

*Veröffentlicht am 23.11.2026*

**Wichtig für ältere Geräte:** Ältere, unsichere SSH-Verfahren bietet remotejoe nur noch an, wenn sie unter Einstellungen → Sicherheit → Cipher-Suites zugelassen sind (Vorgabe: aus). Geräte, die nur solche Verfahren können, melden nach dem Update „Schlüsselaustausch gescheitert“ mit Verweis auf diese Einstellung. Vorher bitte prüfen, ob für das Gerät neuere Firmware verfügbar ist.

- Cipher-Suites (neu, Einstellungen → Sicherheit): zwei Stufen. „Ältere SSH-Verfahren“ (SHA-1-Schlüsselaustausch, AES-CBC, HMAC-SHA1, DSA-Schlüssel) und zusätzlich „sehr alte Verfahren“ (3DES, Diffie-Hellman-Gruppe 1). Sie werden nur angeboten, wenn ein Gerät die aktuellen nicht kann. Die Option „Ältere Verfahren“ an einem Ziel bzw. Tunnel wirkt nur mit dieser Freigabe.
- Tabs wechseln: Strg+Umschalt+Bild↑/↓ wechselt zum Tab links bzw. rechts (mit Umlauf). Einstellungen → Tastatur: Tastenfolgen frei belegbar, wahlweise nur außerhalb von Sitzungen. Strg+Bild↑/↓ wechselt weiterhin immer. Extras → „Tastenkürzel …“ öffnet die Seite.
- Web-Anmeldung: Die Feldkennungen der Anmeldeseite (Verbindung) heißen jetzt „Feldkennung Benutzername / Kennwort / Anmeldeknopf (CSS-Selektor)“ und stehen unter „Erweitert: Felder der Anmeldeseite“ (aufgeklappt, sobald etwas eingetragen ist). Wird eine eingetragene Kennung auf der Seite nicht gefunden, nimmt remotejoe die automatische Erkennung und meldet das einmal, statt die Anmeldung abzubrechen.
- Fehler melden: Die Knöpfe heißen „Fehler an GitHub melden“ und „Abbrechen“; der Browser öffnet sich wieder. Ist der Text zu lang für die Adresse oder öffnet sich kein Browser, liegt der vollständige Text in der Zwischenablage.
- Erweiterungen: „ד schließt auch Tabs der Erweiterungen. Erweiterungen können HTTPS-Schnittstellen eines Ziels nutzen, ohne Zugangsdaten zu sehen (über ein Ticket des Programms, Zertifikat per Fingerabdruck bestätigt). Ein Ziel kann dafür ein benanntes Geheimnis tragen (z. B. einen API-Schlüssel), das nie angezeigt wird.
- Parallele Abfragen über Tickets: Ein gerade benutztes Ticket bleibt gültig; gleichzeitige Abfragen brechen nicht mehr ab. Meldungen sagen „Ticket“ statt „Zugangsschein“.

## 0.1.64 – Neuigkeiten und Tabs

*Veröffentlicht am 22.11.2026*

Fehlerbehebung zu 0.1.63.

- Neuigkeiten: Der Tab lässt sich wieder schließen; der Hinweis unter dem Kopf entfällt.
- Tabs: Kurze Titel werden nicht mehr in der Mitte gekürzt. Bei voller Leiste werden lange Titel höchstens auf 160 Pixel schmaler, danach blättert die Leiste mit den Pfeilen.

## 0.1.63 – Neues Aussehen

*Veröffentlicht am 22.11.2026*

Überarbeitete Oberfläche bei gleicher Ladezeit.

- Fenster und Tabs: Tabs ganz oben, der aktive Tab als helle Karte, die übrigen flach mit Trennstrichen; keine Leiste mehr über den Tabs. Runde Fensterecken (Einstellungen → Darstellung, abschaltbar): unter Windows 11 rundet das System, unter Windows 10 und in Remotedesktop-Sitzungen rundet remotejoe selbst; unter Linux sitzt das Fenster bündig am Bildschirmrand. Alle Farben folgen weiter dem gewählten Farbschema.
- Sprechende Symbole in Baum, Menüs, Kontextmenüs und Einstellungen (Font Awesome Free).
- Zielbereich: Kopfzeile „Ziele“ mit Stecknadel. Gelöst klappt der Bereich ein und öffnet sich beim Überfahren der Registerkarte „Ziele“ am linken Rand; nach dem Verbinden klappt er wieder zu. F9 klappt ihn auf und zu, Strg+F öffnet die Suche. Verzögerungen sind einstellbar.
- Favoriten stehen als Ordner oben im Zielbaum (abschaltbar); die Auswahlliste der Ansichten entfällt. Die Suche steht unten im Zielbereich.
- Einstellungen eines Ziels als übersichtlicher Baum: das Ziel, darunter der Hauptweg mit Verbindung, Zugangsdaten, Tunnel und Aufgaben sowie jeder weitere Zugangsweg einzeln; Zugangswege direkt dort hinzufügen und entfernen.
- Extras → Einstellungen als Baum mit Gruppen (Sicherheit, Darstellung, Daten, Lizenz und Erweiterungen, Claude, Updates).
- Neu: Einstellungen → Updates. Programm-Updates mit Suche beim Programmstart und in regelmäßigen Abständen (alle N Stunden, Tage oder Wochen), Proxy und Protokoll; der bisherige Tab „Programm-Update“ entfällt, Hilfe → „Nach Updates suchen“ öffnet die Seite. Mit Lizenz zusätzlich „Erweiterungen“: Bezug, eigenes Kennwort (im Schlüsselbund) oder das der Lizenz, Aktualisieren und Protokoll.
- Neuigkeiten: Beim ersten Start einer neuen Version zeigt remotejoe die Änderungen; später unter Hilfe → Neuigkeiten.
- Erste Schritte: Nach dem Anlegen der Schlüsseldatenbank zeigen Hinweise, wo Menü und erstes Ziel zu finden sind; jederzeit unter Hilfe → Erste Schritte.
- Hinzufügen nur noch über das Kontextmenü des Zielbaums (Rechtsklick → Hinzufügen, Strg+N).
- Lizenzen und Bibliotheken Dritter als übersichtliche Seite mit Inhaltsverzeichnis; Links öffnen im Browser.
- Das Logo bei leerem Tabbereich führt zur Projektseite.

## 0.1.62 – Programm-Updates für alle über GitHub

*Veröffentlicht am 22.11.2026*

Fehlerbehebung zu 0.1.61.

- Programm-Update: „Nach Updates suchen“ findet neue Versionen für alle über GitHub, auch mit Lizenz. Bisher meldete remotejoe mit Lizenz eine ältere Version als aktuell.
- Einstellungen → Lizenz: Die Lizenz betrifft nur noch die Erweiterungen; Beschriftungen entsprechend angepasst.
- Kleinere Textanpassungen bei Zwischenablage-Dateien (Empfangsordner) und Anmeldeobjekten.

## 0.1.61 – Absturz beim Einfügen der Lizenz behoben

*Veröffentlicht am 22.11.2026*

Fehlerbehebung zu 0.1.60.

- Einstellungen → Lizenz: Beim Einfügen einer Lizenz über das Kontextmenü konnte remotejoe abstürzen, wenn sich die Seite gleichzeitig nach einem Download neu aufbaute (unter anderem Ubuntu 24.04 mit Wayland). Das Eingabefeld bleibt jetzt bestehen, und die Seite baut sich erst nach dem Schließen offener Menüs neu auf.

## 0.1.60 – eine Fassung mit ladbaren Erweiterungen

*Veröffentlicht am 22.11.2026*

Neu: RDP-Ordnerfreigaben und Dateien über die Zwischenablage je Ziel; eine Fassung für alle mit signiert geladenen Erweiterungen.

- RDP: Ordnerfreigaben je Ziel (Eigenschaften → Freigaben). Freigegebene Ordner erscheinen in der Sitzung als Laufwerke; fehlende Ordner werden beim Verbinden gemeldet.
- RDP: Dateien über die Zwischenablage je Ziel schaltbar (Richtung, Grenze je Vorgang 16, 64 oder 256 MiB, Empfangsordner für Dateien). Größere Vorgänge werden abgelehnt und gemeldet.
- Einstellungen → Lizenz: Stand je Erweiterung, „Erweiterungen aktualisieren“ und „Lizenz entfernen“. Der Fenstertitel zeigt „remotejoe basic“ bzw. „remotejoe advanced“, sobald eine Erweiterung aktiv ist.
- Über-Dialog: Lizenz und Bibliotheken Dritter im eingebauten Leser mit Suche statt über den Desktop.
- Programm-Update: Bei „aktuell“ zeigt der Reiter die gefundene Version statt „–“.
- Windows: --demo, --screenshots und --fenster-zuruecksetzen wirken auch über die Befehlszeile; das Paket enthält Qt PrintSupport.
- Linux: install.sh prüft zusätzlich libqt6printsupport6 (Ubuntu: sudo apt install libqt6printsupport6).
- Code überarbeitet: Aufbau des Programms vereinfacht, Schnittstellen und Tests vereinheitlicht.
