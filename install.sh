#!/bin/sh
# Builds Simple Music Player and installs it for the current user (~/.local).
# Usage: ./install.sh            install / update
#        ./install.sh --uninstall
set -eu
cd "$(dirname "$0")"

PREFIX="${PREFIX:-$HOME/.local}"
BIN="$PREFIX/bin/simplemusicplayer"
APPS="$PREFIX/share/applications/simplemusicplayer.desktop"
ICONS="$PREFIX/share/icons/hicolor"

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$BIN" "$APPS" "$ICONS/scalable/apps/simplemusicplayer.svg"
    for s in 16 24 32 48 64 128 256; do rm -f "$ICONS/${s}x${s}/apps/simplemusicplayer.png"; done
    echo "Simple Music Player entfernt (Einstellungen bleiben in ~/.config/simplemusicplayer)."
    exit 0
fi

cargo build --release

install -Dm755 target/release/simplemusicplayer "$BIN"
# Absolute path: the Plasma session's PATH often lacks ~/.local/bin.
mkdir -p "$(dirname "$APPS")"
sed "s|^Exec=.*|Exec=$BIN|" packaging/simplemusicplayer.desktop > "$APPS"
chmod 644 "$APPS"
install -Dm644 assets/simplemusicplayer.svg "$ICONS/scalable/apps/simplemusicplayer.svg"
if command -v rsvg-convert >/dev/null 2>&1; then
    for s in 16 24 32 48 64 128 256; do
        mkdir -p "$ICONS/${s}x${s}/apps"
        rsvg-convert -w "$s" -h "$s" assets/simplemusicplayer.svg -o "$ICONS/${s}x${s}/apps/simplemusicplayer.png"
    done
fi

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$PREFIX/share/applications" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$ICONS" 2>/dev/null || true
command -v kbuildsycoca6 >/dev/null 2>&1 && kbuildsycoca6 >/dev/null 2>&1 || true

echo "Installiert: $BIN"
echo "Startmenü: Multimedia → Simple Music Player"
