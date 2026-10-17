#!/bin/sh
# Trägt remotejoe im Anwendungsmenü des Benutzers ein (Programmsymbol, Starter).
# Aufruf im Projektordner:  tools/install-desktop.sh [Pfad-zum-Programm]
# Entfernen:                tools/install-desktop.sh --remove
# Schreibt nur unter ~/.local/share (kein Root nötig).
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
APPS="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICONS="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"
SIZES="16 24 32 48 64 128 256 512"

if [ "${1:-}" = "--remove" ]; then
	rm -f "$APPS/remotejoe.desktop"
	for s in $SIZES; do rm -f "$ICONS/${s}x${s}/apps/remotejoe.png"; done
	command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPS" || true
	command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t -f "$ICONS" || true
	echo "remotejoe aus dem Anwendungsmenü entfernt."
	exit 0
fi

EXEC=${1:-$ROOT/target/release/remotejoe}
[ -x "$EXEC" ] || { echo "Programm nicht gefunden oder nicht ausführbar: $EXEC" >&2; exit 1; }
mkdir -p "$APPS"
sed "s|@EXEC@|$EXEC|" "$ROOT/packaging/linux/remotejoe.desktop" > "$APPS/remotejoe.desktop"
for s in $SIZES; do
	mkdir -p "$ICONS/${s}x${s}/apps"
	cp "$ROOT/assets/logo/remotejoe-$s.png" "$ICONS/${s}x${s}/apps/remotejoe.png"
done
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPS" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t -f "$ICONS" || true
echo "remotejoe ist im Anwendungsmenü eingetragen (Starter: $APPS/remotejoe.desktop)."
echo "Falls das Symbol noch fehlt: Programm beenden und neu starten, notfalls kurz ab- und anmelden."
