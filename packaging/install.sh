#!/usr/bin/env bash
# Install Vibe Term, from either an extracted release archive or a checkout of
# the repo (then it builds the release binary first; needs Rust).
#
#   ./install.sh                          install or update, for your user (~/.local)
#   PREFIX=/usr/local sudo ./install.sh   install for everyone
#   ./install.sh --uninstall              remove it again (PREFIX applies too)
#
# Your config (~/.config/vibe_term) and saved workspaces (~/.local/state/vibe_term)
# are never touched.
set -euo pipefail

app_id=io.github.joselevelsup.VibeTerm
# readlink -f: also works through the install.sh link at the repo root.
here="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
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

if [ -f "$here/vibe_term" ]; then
    binary="$here/vibe_term" # extracted release archive
elif [ -f "$here/../Cargo.toml" ]; then
    if ! command -v cargo >/dev/null 2>&1; then
        echo "error: building from source needs Rust (cargo): https://rustup.rs" >&2
        exit 1
    fi
    echo "Building Vibe Term (the first build takes a few minutes)..."
    (cd "$here/.." && cargo build --release --locked --quiet)
    binary="$here/../target/release/vibe_term"
else
    echo "error: run this from an extracted release archive or from the vibe_term repo" >&2
    exit 1
fi

install -Dm755 "$binary" "$bin"
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
# A running copy keeps the old code until it is restarted.
if pgrep -x vibe_term >/dev/null 2>&1; then
    echo "Vibe Term is running: close and reopen it to use the new version."
fi
