#!/usr/bin/env bash
# Install Sourizz for the current user (no root needed).
#   ./install.sh                 build and install into ~/.local
#   PREFIX=/usr/local ./install.sh   (run as root) system-wide
#   ./install.sh --uninstall
set -euo pipefail

PREFIX="${PREFIX:-$HOME/.local}"
BIN="$PREFIX/bin"
APPS="$PREFIX/share/applications"
ICONS="$PREFIX/share/icons/hicolor/scalable/apps"
DOCS="$PREFIX/share/licenses/sourizz"
cd "$(dirname "$0")"

if [[ "${1:-}" == "--uninstall" ]]; then
    rm -fv "$BIN/sourizz" "$BIN/sourizz-settings" "$APPS/sourizz.desktop" "$APPS/sourizz-settings.desktop" "$ICONS/sourizz.svg" "$DOCS/LICENSE"
    rmdir "$DOCS" 2>/dev/null || true
    rm -fv "${XDG_CONFIG_HOME:-$HOME/.config}/autostart/sourizz.desktop"
    echo "Uninstalled. Your settings in ${XDG_CONFIG_HOME:-$HOME/.config}/sourizz were kept."
    exit 0
fi

command -v cargo >/dev/null || { echo "Rust is required: https://rustup.rs" >&2; exit 1; }

cargo build --release
install -Dm755 target/release/sourizz "$BIN/sourizz"
install -Dm755 target/release/sourizz-settings "$BIN/sourizz-settings"
install -Dm644 packaging/sourizz.svg "$ICONS/sourizz.svg"
install -Dm644 LICENSE "$DOCS/LICENSE"
# Use absolute paths so the launchers work even when ~/.local/bin is not on the desktop's PATH.
install -d "$APPS"
sed "s|^Exec=sourizz\$|Exec=$BIN/sourizz|" packaging/sourizz.desktop > "$APPS/sourizz.desktop"
sed "s|^Exec=sourizz-settings\$|Exec=$BIN/sourizz-settings|" packaging/sourizz-settings.desktop > "$APPS/sourizz-settings.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" 2>/dev/null || true

echo
echo "Installed to $BIN"
case ":$PATH:" in *":$BIN:"*) ;; *) echo "Note: $BIN is not in your PATH." ;; esac
echo "Start it with:   sourizz"
echo "Launch at login: sourizz autostart enable"
echo "Check your setup: sourizz doctor"
