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

run --aus-ordner "$W/pak" >/dev/null 2>&1 || fail "gültiges Paket wurde nicht installiert"
[ -x "$W/home/.local/share/remotejoe/remotejoe" ] || fail "Hauptprogramm fehlt"
[ -L "$W/home/.local/bin/remotejoe" ] || fail "Starter-Verknüpfung fehlt"
grep -q "^Exec=$W/home/.local/share/remotejoe/remotejoe" "$W/home/.local/share/applications/remotejoe.desktop" || fail "Menüeintrag falsch"
run --aus-ordner "$W/pak" >/dev/null 2>&1 || fail "zweite Installation fehlgeschlagen"
[ -d "$W/home/.local/share/remotejoe.alt" ] || fail "bisherige Fassung nicht als .alt behalten"

KEY_OVERRIDE="remotejoe-update-ed25519 $(head -c 32 /dev/urandom | base64)" run --aus-ordner "$W/pak" >/dev/null 2>&1 && fail "falscher Schlüssel wurde angenommen"
mkdir -p "$W/schlecht"; cp "$W"/pak/* "$W/schlecht/"; printf x >> "$W/schlecht/remotejoe-0.0.1-linux-x86_64.tar.zst"
run --aus-ordner "$W/schlecht" >/dev/null 2>&1 && fail "manipuliertes Archiv wurde angenommen"

run --uninstall >/dev/null 2>&1
[ ! -e "$W/home/.local/share/remotejoe" ] && [ ! -e "$W/home/.local/bin/remotejoe" ] || fail "Entfernen unvollständig"
echo "✔ install.sh: alle Prüfungen bestanden"
