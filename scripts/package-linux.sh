#!/usr/bin/env bash
# Packt die gebauten Programme (target/release) zu einem Veröffentlichungspaket:
#   scripts/package-linux.sh <version> <ausgabeordner>
# Voraussetzung: cargo build --release, cargo build --release -p remotejoe-worker-web --features webview
# und cargo build --release -p remotejoe-update-tool (liefert rj-update). Ergebnis im Ausgabeordner:
#   remotejoe-<version>-linux-x86_64.tar.zst und SHA256SUMS (ohne Signatur; die setzt der Herausgeber
#   lokal, siehe docs/UPDATE.md).
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION="${1:?Version fehlt (z. B. 1.2.3)}"
OUT="${2:?Ausgabeordner fehlt}"
REL=target/release
PLATFORM="linux-$(uname -m)"

PROGRAMS=(remotejoe rj-worker-ssh rj-worker-rdp rj-worker-vnc rj-worker-psrp rj-worker-web)
for p in "${PROGRAMS[@]}"; do
    [ -x "$REL/$p" ] || { echo "Fehlt: $REL/$p – erst bauen (siehe Kopf dieses Skripts)." >&2; exit 1; }
done
[ -x "$REL/rj-update" ] || { echo "Fehlt: $REL/rj-update" >&2; exit 1; }

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
for p in "${PROGRAMS[@]}"; do
    install -m 0755 "$REL/$p" "$STAGE/$p"
    strip --strip-unneeded "$STAGE/$p" 2>/dev/null || true
done
mkdir -p "$STAGE/share/icons"
for f in assets/logo/remotejoe-*.png; do
    install -m 0644 "$f" "$STAGE/share/icons/$(basename "$f")"
done
install -m 0644 packaging/linux/remotejoe.desktop "$STAGE/share/remotejoe.desktop.in"
install -m 0644 LICENSE NOTICE THIRD_PARTY.md "$STAGE/"
printf '%s\n' "$VERSION" > "$STAGE/VERSION"

mkdir -p "$OUT"
"$REL/rj-update" packen-ohne-signatur "$STAGE" "$VERSION" "$PLATFORM" "$OUT"
ls -l "$OUT"
