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
