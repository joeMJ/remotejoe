# remotejoe

Verbindungsmanager für Ubuntu 24.04 und Windows 10/11 (RDP, SSH, VNC, Web, Proxmox,
Hyper-V, PowerShell) mit Zugangsdaten aus einer KeePass-Datei (KDBX).

- Verzeichnisbaum links, Verbindungen in Tabs und gekachelt.
- Zugangsdaten liegen als gewöhnliche Einträge in einer KDBX-Datei.
- Struktur und Einstellungen liegen in einem eigenen Index, verschlüsselt in derselben
  KDBX; der Index verweist per UUID auf die Einträge. Einstellungen werden über den
  Ordnerbaum vererbt.
- Jede Verbindung läuft in einem eigenen Prozess.

Lizenz: Apache-2.0 (siehe `LICENSE`, `NOTICE`, `THIRD_PARTY.md`).

## Bauen

Voraussetzung: Rust (stable).

```
cargo build --release
cargo test
```
