# Datenmodell

remotejoe trennt **Zugangsdaten** und **Struktur**:

- Zugangsdaten sind gewöhnliche Einträge einer KeePass-Datei (KDBX 4). Sie bleiben mit
  KeePassXC und anderen KDBX-Programmen nutzbar.
- Struktur, Ziele und Einstellungen stehen im **Index**, einem JSON-Dokument, das verschlüsselt
  in derselben KDBX liegt. Der Index verweist per UUID auf KDBX-Einträge und enthält selbst
  keine Kennwörter.

## Index

Oberste Ebene:

| Feld | Inhalt |
|---|---|
| `schema_version` | Version des Formats (derzeit 2) |
| `revision` | Zähler, wird bei jedem Speichern erhöht |
| `root` | Wurzelordner mit dem ganzen Baum |
| `tunnels` | Tunneldefinitionen (SSH-Kette, WireGuard, System) |
| `snmp_devices` | SNMP-Geräte |
| `tasks` | Aufgabendefinitionen |
| `macros` | Makros (Folgen von Schritten) |
| `views` | Ansichten: benannte Auswahl von Zielen; ein Ziel darf in mehreren Ansichten stehen |

Knoten im Baum (`children`, Feld `kind`):

- **Ordner** (`folder`): `id`, `name`, `settings`, `children`. Die Reihenfolge der Kinder
  ist die Anzeigereihenfolge.
- **Ziel** (`target`): `id`, `name`, `protocol` (`rdp`, `ssh`, `vnc`, `web`, `proxmox`,
  `hyperv`, `psrp`), `host`, optional `port` (sonst Standardport des Protokolls), `options`
  (protokollabhängig, z. B. Zwischenablage-Richtung, Auflösung, Formular-Anmeldung),
  `credential` (Name des benötigten Zugangsdatensatzes, Standard `default`), `settings`.

Namen werden unverändert gespeichert. Auffällige Zeichen (Steuerzeichen, Zeichen ohne Breite,
Richtungswechsel) werden bei der Anzeige als `⟦U+XXXX⟧` markiert, nicht entfernt.

Beispiel (gekürzt, erfundene Daten):

```json
{
  "schema_version": 2,
  "revision": 7,
  "root": {
    "id": "6f1c0000-0000-4000-8000-000000000000",
    "name": "",
    "settings": {
      "credentials": {
        "default": { "entry": "0b0b0000-0000-4000-8000-000000000001", "last_title": "Admin allgemein" }
      }
    },
    "children": [
      {
        "kind": "folder",
        "id": "6f1c0000-0000-4000-8000-000000000010",
        "name": "Standort Nord",
        "settings": {
          "credentials": {
            "default": { "entry": "0b0b0000-0000-4000-8000-000000000002", "last_title": "Admin Nord" }
          },
          "overrides": { "default": { "domain": "NORD" } },
          "tunnel": "6f1c0000-0000-4000-8000-0000000000a0"
        },
        "children": [
          { "kind": "target", "id": "6f1c0000-0000-4000-8000-000000000011",
            "name": "srv-01", "protocol": "rdp", "host": "srv-01.example.test" },
          { "kind": "target", "id": "6f1c0000-0000-4000-8000-000000000012",
            "name": "gateway", "protocol": "ssh", "host": "192.0.2.1", "credential": "root",
            "settings": { "tunnel": "none" } }
        ]
      }
    ]
  }
}
```

## Verweise auf Zugangsdaten

- Ein Verweis (`CredentialRef`) enthält die UUID des KDBX-Eintrags (`entry`) und den zuletzt
  bekannten Titel (`last_title`), damit eine Meldung den Eintrag nennen kann, wenn er gelöscht
  wurde („Zugangsdaten fehlen: Eintrag „…“ ist in der KDBX nicht mehr vorhanden“).
- Verweise sind **benannt**: `settings.credentials` bildet Namen (z. B. `default`, `root`,
  `domäne`) auf Einträge ab. Ein Ziel fordert über `credential` einen Namen an.
- Beim Verbinden wird der Eintrag aus der KDBX gelesen: Benutzername, Kennwort, Domäne (aus
  `DOMÄNE\benutzer` oder `benutzer@domäne` abgeleitet), bei SSH ein privater Schlüssel aus einem
  Anhang (`id_*`, `*.key`, `*.pem`).
- Kennwörter werden byte-genau übernommen: nie normalisiert (kein NFC/NFD), nie getrimmt, nie
  umkodiert.
- Einträge, auf die kein Ziel verweist, zeigt die Oberfläche unter „Nicht zugeordnet“; dort
  kann ein Eintrag einem Ziel oder Ordner zugeordnet werden.
- Neue Einträge legt remotejoe in der Gruppe `remotejoe` an und ändert nur Einträge in dieser
  Gruppe. Einträge in anderen Gruppen werden nur gelesen.

## Vererbung

Gesucht wird vom Ziel aus nach oben über alle Elternordner bis zur Wurzel; **der erste Treffer
gewinnt**.

| Einstellung | Regel |
|---|---|
| `credentials` | je Name: der nächstgelegene Ordner bzw. das Ziel, das diesen Namen definiert |
| `overrides` | je Name und Feld (`username`, `domain`): der nächstgelegene Wert; das Kennwort ist nicht überschreibbar |
| `tunnel` | nächstgelegene Angabe; `"none"` schaltet einen geerbten Tunnel ab |
| `psrp_transport` | WinRM (HTTPS/HTTP, Port, Kerberos/NTLM/Basic, Zertifikatsprüfung) oder SSH; nächstgelegene Angabe |
| `snmp` | SNMP-Gerät oder `"none"`; nächstgelegene Angabe |

Gibt es zu einem Namen keinen Eintrag, meldet die Oberfläche „Kein Zugangsdatensatz … am Ziel
oder in einem Elternordner“. Legt das Ziel keinen Namen fest, fehlt `default` und kommen
mehrere Namen in Frage, fragt die Oberfläche, welcher genommen werden soll, und kann die Wahl
am Ziel merken.

## Ablage in der KDBX

Standard: Datenbank-`CustomData` der KDBX (Teil des verschlüsselten Inhalts):

| Schlüssel | Inhalt |
|---|---|
| `remotejoe.index` | Index als JSON (UTF-8), wenn höchstens 64 KiB groß |
| `remotejoe.index.0`, `.1`, … und `remotejoe.index.chunks` | größerer Index in Blöcken zu 64 KiB und Anzahl der Blöcke |
| `remotejoe.index.sha256` | Prüfsumme über das ganze JSON; stimmt sie nicht, wird der Index nicht geladen |
| `remotejoe.backup.mine`, `remotejoe.backup.theirs` | Sicherung beider Fassungen nach einem Konflikt mit „beide behalten“ |

KeePassXC lässt diese Daten beim Speichern unverändert (geprüft mit KeePassXC 2.7.6).

Rückfallebene (in `config.toml` wählbar: `index_storage = "index-file"`): eigene Datei
`<name>.rjindex` neben der KDBX. Aufbau `RJINDEX1` ‖ Salz (32 Bytes) ‖ Nonce (24 Bytes) ‖
Chiffrat; XChaCha20-Poly1305, Schlüssel per HKDF-SHA256 aus dem KDBX-Hauptschlüssel
(Kennwort und Schlüsseldatei), neues Salz bei jedem Schreiben.

## Schema-Versionen

- Ältere Versionen werden beim Laden schrittweise migriert (Version 1 → 2: Einstellungen in
  `settings`, Verweise als Objekt statt bloßer UUID).
- Ist die Version neuer als die des Programms, wird der Index nachsichtig gelesen (unbekannte
  Felder werden übergangen) und **schreibgeschützt** geöffnet; Speichern wird verweigert, damit
  ein älteres Programm keine Daten verliert.

## Speichern und gleichzeitige Änderungen

- Geschrieben wird atomar (temporäre Datei, dann Umbenennen) mit Sicherungskopie
  (`<name>.kdbx.bak`) und einer Sperrdatei (`<name>.kdbx.lock`) gegen gleichzeitiges Schreiben.
- Wurde die KDBX seit dem Öffnen von außen geändert (z. B. Synchronisation, KeePassXC), wird
  sie neu eingelesen; eigene Änderungen an Einträgen werden erneut angewandt.
- Hat sich nur auf einer Seite der Index geändert, wird diese Fassung übernommen. Haben sich
  beide Fassungen geändert, fragt die Oberfläche: eigene behalten, fremde laden oder beide
  behalten (eigene wird gespeichert, beide Fassungen zusätzlich gesichert).

## Weitere Ablagen außerhalb der KDBX

| Datei (Linux / Windows) | Inhalt |
|---|---|
| `~/.config/remotejoe/config.toml` / `%APPDATA%\remotejoe\config.toml` | Pfad der KDBX, Schlüsseldatei, Fenster, Zeitlimits, Sperre nach Leerlauf |
| `~/.local/share/remotejoe/` / `%LOCALAPPDATA%\remotejoe\` | `known_hosts` (SSH-Host-Schlüssel, OpenSSH-Format), `vnc_certs` und `proxmox-zertifikate.json` (angenommene Fingerabdrücke), Web-Profile je Ziel (`web-profile/<Ziel-UUID>`, darin `zertifikate.json`) |

In diesen Dateien stehen keine Kennwörter.
