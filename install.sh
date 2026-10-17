#!/usr/bin/env bash
# remotejoe – Installation im eigenen Benutzerkonto (Ubuntu, kein sudo nötig).
#
#   curl -fsSL https://raw.githubusercontent.com/joeMJ/remotejoe/main/install.sh | bash
#   bash install.sh --uninstall              Programm, Starter und Symbole entfernen
#   bash install.sh --aus-ordner <Ordner>    Paket, SHA256SUMS und SHA256SUMS.sig aus einem Ordner
#                                            installieren (offline; Signatur wird trotzdem geprüft)
#
# Ablauf: neueste Veröffentlichung von GitHub laden, die Ed25519-Signatur von SHA256SUMS mit dem
# unten eingebauten Schlüssel prüfen, dann die Prüfsumme des Archivs. Erst danach wird entpackt.
# Installiert wird nach ~/.local/opt/remotejoe (Starter ~/.local/bin/remotejoe, Eintrag im
# Anwendungsmenü). Einstellungen (~/.config/remotejoe) und Schlüsseldatenbank bleiben unberührt.
set -euo pipefail

REPO="joeMJ/remotejoe"
# Öffentlicher Update-Schlüssel; muss zu crates/ui/res/update-key.pub passen (prüft die Veröffentlichung).
UPDATE_KEY='remotejoe-update-ed25519 rX04dvtzNUPhXGPerFMcSxAuy1RFAeenHEup6eNVkaw='

API="${REMOTEJOE_API:-https://api.github.com}"
# Programme liegen getrennt von den Daten des Programms (~/.local/share/remotejoe: bekannte Host-Schlüssel,
# Browserprofile, Zertifikate). Der Installationsordner wird beim Update ersetzt und darf nie der Datenordner sein.
PREFIX="${REMOTEJOE_PREFIX:-$HOME/.local/opt/remotejoe}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/remotejoe"
BINDIR="${HOME}/.local/bin"
APPDIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"

say() { printf '» %s\n' "$*"; }
die() { printf '✖ %s\n' "$*" >&2; exit 1; }

uninstall() {
    [ "$(readlink -m -- "$PREFIX")" != "$(readlink -m -- "$DATA_DIR")" ] || die "Der Installationsordner darf nicht der Datenordner sein."
    if [ -e "$PREFIX" ] && [ ! -x "$PREFIX/remotejoe" ]; then die "$PREFIX ist keine remotejoe-Installation. Nichts gelöscht."; fi
    rm -rf "$PREFIX" "$PREFIX.neu" "$PREFIX.alt"
    rm -f "$BINDIR/remotejoe" "$APPDIR/remotejoe.desktop"
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPDIR" 2>/dev/null || true
    say "remotejoe entfernt. Einstellungen (~/.config/remotejoe) und deine Schlüsseldatenbank bleiben erhalten."
    exit 0
}

FROM_DIR=""
while [ $# -gt 0 ]; do
    case "$1" in
        --uninstall) uninstall ;;
        --aus-ordner) FROM_DIR="${2:?Ordner fehlt}"; shift 2 ;;
        *) die "Unbekannte Option: $1" ;;
    esac
done

# Schutz: nie den Datenordner oder einen fremden Ordner ersetzen.
canon() { readlink -m -- "$1"; }
[ "$(canon "$PREFIX")" != "$(canon "$DATA_DIR")" ] || die "Der Installationsordner darf nicht der Datenordner ($DATA_DIR) sein."
if [ -e "$PREFIX" ] && [ ! -x "$PREFIX/remotejoe" ]; then
    die "$PREFIX existiert und ist keine remotejoe-Installation (kein Programm 'remotejoe' darin). Nichts verändert."
fi

[ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ] || die "Es gibt Pakete nur für Linux x86_64."
for t in tar zstd openssl sha256sum base64; do
    command -v "$t" >/dev/null 2>&1 || die "Das Werkzeug '$t' fehlt (sudo apt install tar zstd openssl coreutils)."
done
[ -n "$FROM_DIR" ] || command -v curl >/dev/null 2>&1 || die "curl fehlt (sudo apt install curl)."

# Ubuntu 24.04 hat Qt 6.4 und eigene Bibliotheken: dafür gibt es ein eigenes Paket.
VARIANT=""
if [ -r "${RJ_OS_RELEASE:-/etc/os-release}" ] && grep -qx 'ID=ubuntu' "${RJ_OS_RELEASE:-/etc/os-release}" && grep -qx 'VERSION_ID="24.04"' "${RJ_OS_RELEASE:-/etc/os-release}"; then VARIANT="-ubuntu2404"; fi

TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT

if [ -n "$FROM_DIR" ]; then
    say "Verwende Dateien aus $FROM_DIR"
    cp "$FROM_DIR"/SHA256SUMS "$FROM_DIR"/SHA256SUMS.sig "$TMP/"
    ARCHIVE_NAME="$(awk '{print $2}' "$TMP/SHA256SUMS" | grep -E "^remotejoe-[0-9][^-]*(-[A-Za-z0-9.]+)?-linux-x86_64${VARIANT}\\.tar\\.zst$" | head -1)"
    [ -n "$ARCHIVE_NAME" ] || die "Kein Linux-Paket in SHA256SUMS."
    cp "$FROM_DIR/$ARCHIVE_NAME" "$TMP/"
else
    say "Suche die neueste Veröffentlichung …"
    JSON="$(curl -fsSL -H 'Accept: application/vnd.github+json' "$API/repos/$REPO/releases/latest")" || die "Die Veröffentlichung ist nicht abrufbar."
    asset_url() { printf '%s' "$JSON" | grep -o "\"browser_download_url\": *\"[^\"]*/$1\"" | head -1 | sed -E 's/.*: *"([^"]*)"/\1/'; }
    ARCHIVE_NAME="$(printf '%s' "$JSON" | grep -oE "\"name\": *\"remotejoe-[0-9][^\"]*-linux-x86_64${VARIANT}\\.tar\\.zst\"" | head -1 | sed -E 's/.*: *"([^"]*)"/\1/')"
    [ -n "$ARCHIVE_NAME" ] || die "In der neuesten Veröffentlichung gibt es kein Linux-Paket."
    for f in "$ARCHIVE_NAME" SHA256SUMS SHA256SUMS.sig; do
        url="$(asset_url "$f")"; [ -n "$url" ] || die "Die Veröffentlichung enthält '$f' nicht."
        say "Lade $f"
        curl -fsSL --proto '=https' -o "$TMP/$f" "$url" || die "Download von '$f' fehlgeschlagen."
    done
fi
VERSION="${ARCHIVE_NAME#remotejoe-}"; VERSION="${VERSION%-linux-x86_64${VARIANT}.tar.zst}"

# 1. Signatur von SHA256SUMS mit dem eingebauten Schlüssel (Ed25519, über OpenSSL ≥ 3).
KEYTEXT="${REMOTEJOE_UPDATE_KEY:-$UPDATE_KEY}"
KEYB64="${KEYTEXT#remotejoe-update-ed25519 }"
{ printf '\x30\x2a\x30\x05\x06\x03\x2b\x65\x70\x03\x21\x00'; printf '%s' "$KEYB64" | base64 -d; } > "$TMP/key.der" 2>/dev/null || die "Eingebauter Schlüssel ungültig."
[ "$(wc -c < "$TMP/key.der")" = 44 ] || die "Eingebauter Schlüssel hat die falsche Länge."
openssl pkey -pubin -inform DER -in "$TMP/key.der" -out "$TMP/key.pem" 2>/dev/null || die "Schlüssel nicht lesbar (OpenSSL 3 nötig)."
if [ "$(wc -c < "$TMP/SHA256SUMS.sig")" = 64 ]; then cp "$TMP/SHA256SUMS.sig" "$TMP/sig.bin"; else base64 -d "$TMP/SHA256SUMS.sig" > "$TMP/sig.bin" 2>/dev/null || die "Signatur nicht lesbar."; fi
openssl pkeyutl -verify -pubin -inkey "$TMP/key.pem" -rawin -in "$TMP/SHA256SUMS" -sigfile "$TMP/sig.bin" >/dev/null 2>&1 || die "Signatur ungültig: SHA256SUMS ist nicht mit dem eingebauten Schlüssel signiert. Nichts installiert."
say "Signatur in Ordnung."

# 2. Prüfsumme des Archivs aus der signierten Datei.
( cd "$TMP" && grep -F "  $ARCHIVE_NAME" SHA256SUMS | sha256sum -c - >/dev/null 2>&1 ) || die "Prüfsumme des Archivs stimmt nicht. Nichts installiert."
say "Prüfsumme in Ordnung ($VERSION)."

# 3. Entpacken in ein Nachbarverzeichnis und Systembibliotheken prüfen.
rm -rf "$PREFIX.neu"; mkdir -p "$PREFIX.neu" "$(dirname "$PREFIX")"
tar --zstd -xf "$TMP/$ARCHIVE_NAME" -C "$PREFIX.neu" --no-same-owner || { rm -rf "$PREFIX.neu"; die "Archiv nicht entpackbar."; }
[ -x "$PREFIX.neu/remotejoe" ] || { rm -rf "$PREFIX.neu"; die "Das Paket enthält kein Hauptprogramm."; }

pkg_for() {
    case "$1" in
        libQt6Core*) echo libqt6core6t64 ;; libQt6Gui*) echo libqt6gui6 ;; libQt6Widgets*) echo libqt6widgets6 ;;
        libQt6Svg*) echo libqt6svg6 ;; libQt6DBus*) echo libqt6dbus6 ;; libQt6Network*) echo libqt6network6 ;;
        libfreerdp-client3*) echo libfreerdp-client3-3 ;; libfreerdp3*) echo libfreerdp3-3 ;; libwinpr3*) echo libwinpr3-3 ;;
        libwebkit2gtk-4.1*) echo libwebkit2gtk-4.1-0 ;; libjavascriptcoregtk-4.1*) echo libjavascriptcoregtk-4.1-0 ;;
        libgtk-3*) echo libgtk-3-0t64 ;; libsoup-3.0*) echo libsoup-3.0-0 ;;
        *) echo "" ;;
    esac
}
MISSING="$(for f in "$PREFIX.neu"/remotejoe "$PREFIX.neu"/rj-worker-*; do ldd "$f" 2>/dev/null | awk '/not found/ {print $1}'; done | sort -u)"
if [ -n "$MISSING" ]; then
    PKGS=""; OTHER=""
    for l in $MISSING; do p="$(pkg_for "$l")"; if [ -n "$p" ]; then PKGS="$PKGS $p"; else OTHER="$OTHER $l"; fi; done
    rm -rf "$PREFIX.neu"
    printf '✖ Es fehlen Systembibliotheken.\n' >&2
    [ -z "$PKGS" ] || printf '  Installieren mit:  sudo apt install%s\n' "$(printf '%s\n' $PKGS | sort -u | tr '\n' ' ')" >&2
    [ -z "$OTHER" ] || printf '  Weitere (kein Paketname bekannt):%s\n' "$OTHER" >&2
    exit 1
fi

# 4. Austausch; die bisherige Fassung bleibt als .alt.
rm -rf "$PREFIX.alt"
[ ! -e "$PREFIX" ] || mv "$PREFIX" "$PREFIX.alt"
mv "$PREFIX.neu" "$PREFIX" || { [ ! -e "$PREFIX.alt" ] || mv "$PREFIX.alt" "$PREFIX"; die "Austausch fehlgeschlagen."; }

# 5. Starter und Eintrag im Anwendungsmenü.
mkdir -p "$BINDIR" "$APPDIR"
ln -sf "$PREFIX/remotejoe" "$BINDIR/remotejoe"
sed -e "s|@EXEC@|$PREFIX/remotejoe|" -e "s|^Icon=.*|Icon=$PREFIX/share/icons/remotejoe-256.png|" "$PREFIX/share/remotejoe.desktop.in" > "$APPDIR/remotejoe.desktop"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPDIR" 2>/dev/null || true

say "remotejoe $VERSION ist installiert: $PREFIX"
say "Start: im Anwendungsmenü „remotejoe“ oder im Terminal: remotejoe"
case ":$PATH:" in *":$BINDIR:"*) ;; *) say "Hinweis: $BINDIR ist nicht im PATH; das Programm startet trotzdem über das Anwendungsmenü." ;; esac
[ ! -e "$PREFIX.alt" ] || say "Die bisherige Fassung liegt noch unter $PREFIX.alt (kann gelöscht werden)."
