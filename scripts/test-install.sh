#!/usr/bin/env bash
# Prüft install.sh offline mit einem Wegwerf-Schlüssel und einem Wegwerf-Benutzerverzeichnis:
# gültiges Paket, Austausch mit .alt, falscher Schlüssel, manipuliertes Archiv, Entfernen.
#   scripts/test-install.sh        (braucht target/release mit remotejoe, Workern und rj-update)
set -euo pipefail
cd "$(dirname "$0")/.."
W="$(mktemp -d)"; trap 'rm -rf "$W"' EXIT
fail() { echo "✖ $*" >&2; exit 1; }

scripts/package-linux.sh 0.0.1 "$W/roh" >/dev/null
target/release/rj-update schluessel "$W/keys" >/dev/null
STAGE="$W/stage"; mkdir -p "$STAGE"; tar --zstd -xf "$W"/roh/remotejoe-0.0.1-linux-x86_64.tar.zst -C "$STAGE"
target/release/rj-update packen "$STAGE" 0.0.1 linux-x86_64 "$W/keys/update-key.priv" "$W/pak" >/dev/null
KEY="$(grep -v '^#' "$W/keys/update-key.pub" | head -1)"
run() { HOME="$W/home" REMOTEJOE_UPDATE_KEY="${KEY_OVERRIDE:-$KEY}" ./install.sh "$@"; }
mkdir -p "$W/home"
# Daten des Programms, die nie angefasst werden dürfen (Datenordner ≠ Installationsordner).
DATA="$W/home/.local/share/remotejoe"; mkdir -p "$DATA/web-profile"; echo "wichtig" > "$DATA/known_hosts"

run --aus-ordner "$W/pak" >/dev/null 2>&1 || fail "gültiges Paket wurde nicht installiert"
[ -x "$W/home/.local/opt/remotejoe/remotejoe" ] || fail "Hauptprogramm fehlt"
[ -L "$W/home/.local/bin/remotejoe" ] || fail "Starter-Verknüpfung fehlt"
grep -q "^Exec=$W/home/.local/opt/remotejoe/remotejoe" "$W/home/.local/share/applications/remotejoe.desktop" || fail "Menüeintrag falsch"
run --aus-ordner "$W/pak" >/dev/null 2>&1 || fail "zweite Installation fehlgeschlagen"
[ -d "$W/home/.local/opt/remotejoe.alt" ] || fail "bisherige Fassung nicht als .alt behalten"

KEY_OVERRIDE="remotejoe-update-ed25519 $(head -c 32 /dev/urandom | base64)" run --aus-ordner "$W/pak" >/dev/null 2>&1 && fail "falscher Schlüssel wurde angenommen"
mkdir -p "$W/schlecht"; cp "$W"/pak/* "$W/schlecht/"; printf x >> "$W/schlecht/remotejoe-0.0.1-linux-x86_64.tar.zst"
run --aus-ordner "$W/schlecht" >/dev/null 2>&1 && fail "manipuliertes Archiv wurde angenommen"

[ "$(cat "$DATA/known_hosts")" = wichtig ] && [ -d "$DATA/web-profile" ] || fail "Daten des Programms wurden verändert"
# Installationsordner = Datenordner wird verweigert, fremde Ordner werden nicht ersetzt.
REMOTEJOE_PREFIX="$DATA" run --aus-ordner "$W/pak" >/dev/null 2>&1 && fail "Installation in den Datenordner wurde nicht verweigert"
mkdir -p "$W/fremd" && echo x > "$W/fremd/datei"
REMOTEJOE_PREFIX="$W/fremd" run --aus-ordner "$W/pak" >/dev/null 2>&1 && fail "Ein fremder Ordner wurde ersetzt"
[ -f "$W/fremd/datei" ] || fail "Fremder Ordner wurde verändert"
[ "$(cat "$DATA/known_hosts")" = wichtig ] || fail "Daten nach den Verweigerungen verändert"

# Ubuntu 24.04 bekommt das eigene Paket (…-ubuntu2404), alle anderen das allgemeine.
printf 'ID=ubuntu\nVERSION_ID="24.04"\n' > "$W/os24"; printf 'ID=ubuntu\nVERSION_ID="26.04"\n' > "$W/os26"
mkdir -p "$W/var24"; cp "$W"/roh/remotejoe-0.0.1-linux-x86_64.tar.zst "$W/stage24.tar.zst"
mkdir -p "$W/st24"; tar --zstd -xf "$W/stage24.tar.zst" -C "$W/st24"
target/release/rj-update packen "$W/st24" 0.0.1 linux-x86_64-ubuntu2404 "$W/keys/update-key.priv" "$W/pak24" >/dev/null
RJ_OS_RELEASE="$W/os24" run --aus-ordner "$W/pak24" >/dev/null 2>&1 || fail "24.04-Paket wurde auf Ubuntu 24.04 nicht installiert"
RJ_OS_RELEASE="$W/os26" run --aus-ordner "$W/pak24" >/dev/null 2>&1 && fail "24.04-Paket wurde auf Ubuntu 26.04 angenommen"
RJ_OS_RELEASE="$W/os24" run --aus-ordner "$W/pak" >/dev/null 2>&1 && fail "allgemeines Paket wurde auf Ubuntu 24.04 angenommen"

run --uninstall >/dev/null 2>&1
[ "$(cat "$DATA/known_hosts")" = wichtig ] || fail "Entfernen hat Daten gelöscht"
[ ! -e "$W/home/.local/opt/remotejoe" ] && [ ! -e "$W/home/.local/bin/remotejoe" ] || fail "Entfernen unvollständig"
echo "✔ install.sh: alle Prüfungen bestanden"
