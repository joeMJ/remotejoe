# Claude-Anbindung (MCP)

remotejoe kann sich als Werkzeug für **Claude Desktop** anbieten (Model Context Protocol, MCP). Claude kann dann
Fragen wie „Welche APs in Ordner X laufen noch nicht mit 8.10.0.24?“ oder „Welche Adressen aus dieser Liste fehlen
in remotejoe?“ selbst beantworten – mit den Daten aus remotejoe, aber ohne je Zugangsdaten zu sehen.

> [!IMPORTANT]
> **Datenschutz:** Alles, was Claude über diese Anbindung liest, geht als Gesprächsinhalt an den Claude-Dienst von
> Anthropic – Zielnamen, Ordnernamen (z. B. Schulen und Orte), Adressen, Seriennummern und Inventar. Gib nur Ordner
> frei, deren Inhalt du so weitergeben darfst, und blende Adressen und Seriennummern bei Bedarf aus. Zugangsdaten,
> Benutzernamen, Notizen und andere Inhalte der Schlüsseldatenbank gehen nie hinaus.

## Wie es funktioniert

- Claude Desktop startet das kleine Programm `rj-mcp`, das neben `remotejoe` installiert ist. Es spricht MCP über
  die Standardein- und -ausgabe und reicht jede Anfrage an das **laufende, entsperrte** remotejoe weiter (lokaler
  Kanal, nur für das eigene Benutzerkonto). `rj-mcp` selbst kennt keine Zugangsdaten.
- Ab Werk ist die Anbindung **aus**. Einschalten unter **Einstellungen → Claude-Anbindung (MCP)**.
- Claude sieht nur Ziele in den dort **freigegebenen Ordnern** (je Schlüsseldatenbank). Adressen und Seriennummern
  lassen sich ausblenden.
- **Aktionen** (Verbinden, Inventar oder Cluster abfragen, Nutzer anzeigen) gehen nur bei Zielen, deren Zugangsdaten
  aus einem **für Claude freigegebenen Anmeldeobjekt** stammen – zum Beispiel ein Konto mit wenig Rechten eigens für
  Claude. Zusätzlich fragt remotejoe bei **jeder** Aktion nach („Erlauben“ / „Ablehnen“; ohne Antwort nach 2 Minuten
  abgelehnt).
- **Extras → Claude-Protokoll** zeigt alle Anfragen dieser Sitzung, auch abgelehnte.

## Werkzeuge

| Werkzeug | Was |
| :--- | :--- |
| `ordner_liste` | freigegebene Ordner |
| `ziele_suchen` | Ziele nach Name, Adresse oder Ordner |
| `ziel_uebersicht` | Typ, Ordner, Zugangswege, gespeichertes Inventar eines Ziels |
| `ordner_inventar` | gespeichertes Inventar aller Ziele eines Ordners (z. B. Versionen) |
| `ziele_abgleichen` | Abgleich mit einer externen Liste (Adressen, MACs, Seriennummern oder Namen) |
| `aruba_cluster` | zuletzt abgefragter Stand eines Aruba-Clusters |
| `sitzungen` | offene Sitzungen |
| `verbinden`, `inventar_abfragen`, `cluster_abfragen`, `nutzer_anzeigen` | Aktionen mit Freigabe |

Abgefragt wird bei den lesenden Werkzeugen nichts neu; sie zeigen den gespeicherten Stand.

## Einrichten in Claude Desktop

1. In remotejoe unter **Einstellungen → Claude-Anbindung (MCP)** die Anbindung einschalten, Ordner freigeben und bei
   Bedarf Anmeldeobjekte für Aktionen wählen.
2. Dort **Eintrag kopieren**. Der Eintrag sieht so aus (Pfad je nach Installation):

   Windows:

   ```json
   {
     "mcpServers": {
       "remotejoe": { "command": "C:\\Users\\<Name>\\AppData\\Local\\Programs\\remotejoe\\rj-mcp.exe" }
     }
   }
   ```

   Linux:

   ```json
   {
     "mcpServers": {
       "remotejoe": { "command": "/home/<name>/.local/opt/remotejoe/rj-mcp" }
     }
   }
   ```

3. In Claude Desktop **Einstellungen → Entwickler → Konfiguration bearbeiten** öffnen
   (`claude_desktop_config.json`), den Eintrag unter `mcpServers` ergänzen und Claude Desktop neu starten.
4. remotejoe muss laufen und entsperrt sein, wenn Claude es fragt.

Mit einem eigenen Datenordner (`REMOTEJOE_HOME`) kommt im Eintrag `"env": { "REMOTEJOE_HOME": "…" }` dazu; „Eintrag
kopieren“ fügt das selbst ein.
