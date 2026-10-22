#!/usr/bin/env bash
# Install Vibe Term from an extracted release archive.
#
#   ./install.sh                      install for your user, into ~/.local
#   PREFIX=/usr/local sudo ./install.sh   install for everyone
#   ./install.sh --uninstall          remove it again (PREFIX applies too)
#
# Your config (~/.config/vibe_term) and saved workspaces (~/.local/state/vibe_term)
# are never touched.
set -euo pipefail

app_id=io.github.joselevelsup.VibeTerm
here="$(cd "$(dirname "$0")" && pwd)"
prefix="${PREFIX:-$HOME/.local}"
bin="$prefix/bin/vibe_term"
desktop="$prefix/share/applications/$app_id.desktop"
icon="$prefix/share/icons/hicolor/scalable/apps/$app_id.svg"

# No icon/desktop cache rebuilds: launchers watch these folders, and a stray
# icon-theme.cache in ~/.local can hide icons other apps install later.
if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$bin" "$desktop" "$icon"
    echo "Removed Vibe Term from $prefix."
    exit 0
fi

install -Dm755 "$here/vibe_term" "$bin"
install -Dm644 "$here/$app_id.svg" "$icon"
# Point the launcher at the installed binary, so it works even when
# $prefix/bin is not on PATH (desktop sessions often don't include ~/.local/bin).
tmp="$(mktemp)"
sed "s|^Exec=.*|Exec=\"$bin\"|" "$here/$app_id.desktop" > "$tmp"
install -Dm644 "$tmp" "$desktop"
rm -f "$tmp"

echo "Installed Vibe Term to $bin"
echo "It is in your app launcher as \"Vibe Term\"."
command -v hx >/dev/null 2>&1 || echo "Note: Helix (hx) was not found. Terminals work, but editor containers need it."
case ":$PATH:" in
    *":$prefix/bin:"*) ;;
    *) echo "Note: $prefix/bin is not on your PATH; add it to run vibe_term from a shell." ;;
esac
