# Updates

remotejoe kann neue Versionen selbst finden, laden und installieren. Quelle ist ein
Gitea-Server oder GitHub. Jedes Paket ist mit einem Ed25519-Schlüssel signiert; das Programm
installiert nur, was mit dem eingebauten öffentlichen Schlüssel signiert ist und dessen
Prüfsumme stimmt.

![Update-Tab](screenshots/update-tab.png)

## Für Anwender

### Quelle einstellen

**Hilfe → Nach Updates suchen …** öffnet den Tab „Programm-Update“.

| Feld | Bedeutung |
|---|---|
| Quelle | Keine (Voreinstellung), Gitea oder GitHub |
| Gitea-Adresse | Basisadresse des Servers, z. B. `https://git.example.test` (bei GitHub leer lassen) |
| Repository | `besitzer/name` |
| Zugriffsschlüssel | Eintrag der Schlüsseldatenbank, dessen **Kennwort** der Zugriffsschlüssel (Token) ist; leer für öffentliche Repositorys |
| Beim Start im Hintergrund prüfen | Voreinstellung aus; prüft einmal je Start, ohne den Start zu verzögern (mit Zugriffsschlüssel erst nach dem Entsperren) |

„Einstellungen speichern“ schreibt die Werte in `config.toml`, Abschnitt `[update]`. Der
Zugriffsschlüssel steht nie in der Konfiguration, sondern nur als Verweis auf den Eintrag
(`token_entry`, UUID).

```toml
[update]
source = "gitea"
base_url = "https://git.example.test"
repo = "team/remotejoe"
token_entry = "0b5c…"      # Eintrag in der KDBX
check_on_start = false
```

### Zugriffsschlüssel anlegen

- **Gitea:** Einstellungen → Anwendungen → „Zugriffstoken generieren“, Berechtigung
  `read:repository` genügt. Das Token als Kennwort eines neuen KDBX-Eintrags speichern (z. B.
  „Gitea Updates“) und im Tab auswählen.
- **GitHub (privates Repository):** Fein granulares Token mit Leserecht auf „Contents“ des
  Repositorys; ebenso als Kennwort eines KDBX-Eintrags.

Der Schlüssel wird byte-genau übertragen (keine Kürzung, keine Umkodierung) und erscheint nie in
Protokollen, Fehlermeldungen oder im Tab.

### Prüfen und installieren

1. „Jetzt prüfen“ fragt `…/releases/latest` ab (Gitea: `/api/v1/repos/{repo}/releases/latest`,
   GitHub: `/repos/{repo}/releases/latest`) und vergleicht nach SemVer mit der eingebauten
   Version. Ältere oder gleiche Versionen werden nie angeboten.
2. Der Tab zeigt installierte und gefundene Version und den Änderungstext (als reiner Text).
3. „Herunterladen und installieren“ lädt `SHA256SUMS.sig`, `SHA256SUMS` und das Archiv, prüft
   **zuerst die Signatur** und **dann die Prüfsumme**. Schlägt etwas fehl, bleibt alles, wie es
   war, und der Tab nennt den Grund.
4. Installation (Linux): Das Archiv wird in ein Nachbarverzeichnis `<programm>.neu` entpackt,
   dann wird das Programmverzeichnis ersetzt; die bisherige Fassung bleibt als
   `<programm>.alt`. „Jetzt neu starten“ ist freiwillig, bis dahin läuft die bisherige Fassung.
5. Sind Sitzungen offen, fragt der Tab vorher nach (Leiste im Tab).
6. Liegt das Programm unter `/usr` (Paketverwaltung), verändert es sich nicht selbst; der Tab
   nennt nur den Download.

**Zurück zur alten Fassung:** Programm beenden, `<programm>` löschen oder umbenennen und
`<programm>.alt` zurückbenennen.

**Sicherheit:** nur `https` (Ausnahme `127.0.0.1`/`localhost` für Tests), Zertifikatsprüfung
gegen die Zertifizierungsstellen des Systems (nicht abschaltbar), höchstens 300 MB je Download,
Zeitlimit 10 Minuten, „Abbrechen“ jederzeit. Weiterleitungen nur auf `https`; beim Wechsel des
Hosts (z. B. Speicherdienst für Downloads) wird der Zugriffsschlüssel nicht mitgeschickt.

## Für Herausgeber

Das Werkzeug `rj-update` (`tools/update/`) erzeugt Schlüssel, packt, signiert und lädt hoch.

```sh
cargo build --release -p remotejoe-update-tool    # liefert target/release/rj-update
```

### 1. Schlüsselpaar erzeugen (einmalig)

```sh
rj-update schluessel ~/remotejoe-signatur
```

- `update-key.priv` – **privat**, Rechte 0600. Nur lokal aufbewahren (am besten auf einem
  Datenträger, der nicht ständig eingebunden ist, und als Sicherung in der KDBX als Anhang). Nie
  in ein Repository: `.gitignore` schließt `*.priv` aus.
- `update-key.pub` – öffentlich; nach `crates/ui/res/update-key.pub` kopieren und das Programm
  neu bauen. Die mitgelieferte Datei ist ein **Platzhalter**, zu dem kein privater Schlüssel
  existiert.

### 2. Paket erzeugen und signieren

Programmordner vorbereiten (alle Dateien, die neben dem Programm liegen: `remotejoe`, die Worker
`rj-worker-*`, …), dann:

```sh
rj-update packen target/release/paket 0.2.0 linux-x86_64 ~/remotejoe-signatur/update-key.priv ausgabe/
rj-update pruefen ausgabe/ crates/ui/res/update-key.pub
```

Ergebnis in `ausgabe/`: `remotejoe-0.2.0-linux-x86_64.tar.zst` (Windows:
`remotejoe-0.2.0-windows-x86_64.zip`), `SHA256SUMS`, `SHA256SUMS.sig`. Die Version muss der
Version in `Cargo.toml` des neuen Programms entsprechen.

### 3. Veröffentlichen

**Gitea:**

```sh
export RJ_GITEA_TOKEN=…        # Token mit write:repository; nur in der Umgebung, nie in Dateien
rj-update hochladen https://git.example.test team/remotejoe v0.2.0 ausgabe/ aenderungen.txt
unset RJ_GITEA_TOKEN
```

Das legt die Veröffentlichung mit dem Tag `v0.2.0` an und lädt alle Dateien aus `ausgabe/` hoch.

**GitHub:** Veröffentlichung von Hand oder mit `gh release create v0.2.0 ausgabe/*` anlegen; die
drei Dateien müssen als Anhänge dabei sein.

### 4. Schlüsselwechsel

1. Neues Schlüsselpaar erzeugen.
2. Eine Übergangsversion mit dem **alten** Schlüssel signieren, die bereits den **neuen**
   öffentlichen Schlüssel eingebaut hat, und veröffentlichen.
3. Ab der nächsten Version mit dem neuen Schlüssel signieren.

Ist der alte private Schlüssel verloren oder kompromittiert, können bestehende Installationen
nicht mehr automatisch aktualisiert werden: neue Fassung von Hand installieren und den
Anwendern mitteilen.
