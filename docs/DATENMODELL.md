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
| `schema_version` | Version des Formats (derzeit 3) |
| `revision` | Zähler, wird bei jedem Speichern erhöht |
| `root` | Wurzelordner mit dem ganzen Baum |
| `tunnels` | Tunneldefinitionen (SSH-Kette, WireGuard, System) |
| `snmp_devices` | SNMP-Geräte |
| `tasks` | Aufgabendefinitionen |
| `macros` | Makros (Folgen von Schritten) |
| `views` | Ansichten: benannte Auswahl von Zielen; ein Ziel darf in mehreren Ansichten stehen |
| `templates` | Vorlagen je Protokoll (Port, Optionen, Tunnel, Art der Zugangsdaten; eine Standardvorlage je Protokoll); **nie** Kennwörter oder Verweise auf KDBX-Einträge |
| `favorites` | Favoriten (Ziele oder Ordner) in Anzeigereihenfolge |

Knoten im Baum (`children`, Feld `kind`):

- **Ordner** (`folder`): `id`, `name`, `settings`, `meta`, `children`. Die Reihenfolge der
  Kinder ist die Anzeigereihenfolge.
- **Ziel** (`target`): `id`, `name`, `protocol` (`rdp`, `ssh`, `vnc`, `web`, `proxmox`,
  `hyperv`, `psrp`), `host`, optional `port` (sonst Standardport des Protokolls), `options`
  (protokollabhängig, z. B. Zwischenablage-Richtung, Auflösung, Formular-Anmeldung),
  `credential` (Name des benötigten Zugangsdatensatzes, Standard `default`), `settings`, `meta`.
- `meta` (beide, optional): `description`, `notes`, `fields` (eigene Felder Name → Wert),
  `tasks` und `macros` (zugeordnete Aufgaben und Makros per ID). Keine Geheimnisse.

Namen werden unverändert gespeichert. Auffällige Zeichen (Steuerzeichen, Zeichen ohne Breite,
Richtungswechsel) werden bei der Anzeige als `⟦U+XXXX⟧` markiert, nicht entfernt.

Beispiel (gekürzt, erfundene Daten):

```json
{
  "schema_version": 3,
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
- „Mehrere Ziele …“ mit der Art „Benutzername und Kennwort direkt“ legt **einen** neuen Eintrag
  an (Titel = Name des ersten Ziels), auf den alle neu angelegten Ziele verweisen.

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

Zugangsdaten am Knoten (Eigenschaften-Tab, fünf Möglichkeiten):

| Art | Ablage |
|---|---|
| keine | `settings.credentials_off = true`; schneidet die Vererbung ab (auch für Unterobjekte ohne eigene Angabe) |
| vom übergeordneten Ordner | nichts am Knoten |
| direkt (Benutzername und Kennwort) | neuer Eintrag in der KDBX-Gruppe `remotejoe`, Verweis unter `settings.credentials` |
| vorhandener Datensatz | Verweis per UUID unter `settings.credentials` |
| Datensatz nach Name | Ziel: `credential`; Ordner: `settings.credential_name` (gilt für Unterobjekte ohne eigenen Namen) |

Der verlangte Name eines Ziels ist `credential`, sonst der nächstgelegene `credential_name`
eines Ordners, sonst `default`.

Weitere Einstellungen: `clipboard` am Ordner ist die Vorgabe der Zwischenablage-Richtung für
neue Ziele. „Auf Unterobjekte anwenden“ entfernt die eigenen Angaben der Unterobjekte
(Zugangsdaten, Tunnel, PowerShell-Transport, SNMP), damit der Wert des Ordners gilt; bei der
Zwischenablage wird die Richtung an jedem RDP-/VNC-Ziel gesetzt.

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
`<name>.rjindex` neben der KDBX. Aufbau `RJINDEX2` ‖ Verfahren (1 Byte, Argon2id) ‖
Argon2-Version (1 Byte) ‖ Speicher in KiB, Durchläufe, Fäden (je u32) ‖ Salz (32 Bytes) ‖
Nonce (24 Bytes) ‖ Chiffrat; XChaCha20-Poly1305 mit dem ganzen Kopf als zusätzlichen
authentisierten Daten. Der Schlüssel wird mit Argon2id aus dem KDBX-Hauptschlüssel (Kennwort
und Schlüsseldatei) abgeleitet, mit den Argon2-Parametern der KDBX (bei AES-KDF: 64 MiB,
10 Durchläufe, 2 Fäden). Das Salz gilt je Datei, die Nonce ist bei jedem Schreiben neu.
Dateien im älteren Format `RJINDEX1` (Schlüssel per HKDF-SHA256) werden gelesen und beim
nächsten Speichern im neuen Format geschrieben.

## Schema-Versionen

- Ältere Versionen werden beim Laden schrittweise migriert (Version 1 → 2: Einstellungen in
  `settings`, Verweise als Objekt statt bloßer UUID; Version 2 → 3: nur neue, optionale Felder).
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
| `~/.config/remotejoe/config.toml` / `%APPDATA%\remotejoe\config.toml` | Pfad der KDBX, Schlüsseldatei, Fenster, Zeitlimits, `connect_all_parallel` (Vorgabe 4), Sperre nach Leerlauf (`lock_after_idle_mins`, Vorgabe 0 = aus; `disconnect_on_lock`), Update-Quelle `[update]` (`source`, `base_url`, `repo`, `token_entry` als Verweis auf einen KDBX-Eintrag, `check_on_start`, Vorgabe aus) |
| `~/.local/share/remotejoe/` / `%LOCALAPPDATA%\remotejoe\` | `known_hosts` (SSH-Host-Schlüssel im Format von OpenSSH: `host` bzw. `[host]:port`, austauschbar mit `~/.ssh/known_hosts`; gelesen werden auch gehashte Einträge, Listen und Platzhalter, Einträge im früheren Format `host:port` werden beim nächsten Schreiben umgestellt), `vnc_certs` und `proxmox-zertifikate.json` (angenommene Fingerabdrücke), Web-Profile je Ziel (`web-profile/<Ziel-UUID>`, darin `zertifikate.json`) |

In diesen Dateien stehen keine Kennwörter.

## SNMP-Geräte

`snmp_devices[]`: `id`, `name`, `host`, `port`, `version` (`v2c`/`v3`), `credential` (Verweis),
bei v3 `auth_proto` (`md5`, `sha1`, `sha256`; Vorgabe `sha1`) und `priv_proto` (`des`, `aes128`,
`none`; Vorgabe `aes128`). Im KDBX-Eintrag: v2c die Community im Kennwortfeld; v3 Benutzername,
Kennwort zur Authentisierung im Kennwortfeld und Kennwort zur Verschlüsselung im geschützten
Feld `SNMP-Verschlüsselung` (fehlt es, gilt das Kennwortfeld für beides).
