#!/usr/bin/env bash
# Install Sourizz for the current user (no root needed).
#   ./install.sh                 build and install into ~/.local
#   PREFIX=/usr/local ./install.sh   (run as root) system-wide
#   ./install.sh --uninstall
#   ./install.sh --yes           do not ask before installing Rust (rustup) when it is missing
set -euo pipefail

PREFIX="${PREFIX:-$HOME/.local}"
BIN="$PREFIX/bin"
APPS="$PREFIX/share/applications"
ICONS="$PREFIX/share/icons/hicolor/scalable/apps"
DOCS="$PREFIX/share/licenses/sourizz"
cd "$(dirname "$0")"

UNINSTALL=0
ASSUME_YES=0
for arg in "$@"; do
    case "$arg" in
        --uninstall) UNINSTALL=1 ;;
        -y | --yes) ASSUME_YES=1 ;;
        *) echo "Unknown option: $arg (see the header of this script)" >&2; exit 2 ;;
    esac
done

# Make sure `cargo` is available, installing Rust with rustup (https://rustup.rs) after asking if it is not.
ensure_rust() {
    local cargo_env="${CARGO_HOME:-$HOME/.cargo}/env"
    # rustup puts cargo in ~/.cargo/bin, which a non-login shell may not have in its PATH yet.
    if [[ -f "$cargo_env" ]]; then
        # shellcheck disable=SC1090
        . "$cargo_env"
    fi
    if command -v cargo >/dev/null; then
        return 0
    fi

    echo "Rust (cargo) is not installed; it is needed to build Sourizz."
    if ! command -v cc >/dev/null; then
        echo "Rust also needs a C linker: install your distribution's build tools first (e.g. 'sudo apt install build-essential')." >&2
        exit 1
    fi
    local fetch
    if command -v curl >/dev/null; then
        fetch=(curl --proto '=https' --tlsv1.2 -fsSL)
    elif command -v wget >/dev/null; then
        fetch=(wget -qO-)
    else
        echo "Install curl or wget, or install Rust yourself from https://rustup.rs, then run this script again." >&2
        exit 1
    fi
    if [[ $ASSUME_YES -ne 1 ]]; then
        if [[ ! -t 0 ]]; then
            echo "Run this script again with --yes to install Rust with rustup, or install it from https://rustup.rs." >&2
            exit 1
        fi
        local reply
        read -r -p "Install it now with rustup (https://rustup.rs) into ${CARGO_HOME:-$HOME/.cargo}? [y/N] " reply
        if [[ ! "$reply" =~ ^[Yy]$ ]]; then
            echo "Aborted. Install Rust from https://rustup.rs and run this script again." >&2
            exit 1
        fi
    fi

    # Download first, run second: a download cut short must not be executed.
    local script
    script="$(mktemp)"
    trap "rm -f '$script'" EXIT
    "${fetch[@]}" https://sh.rustup.rs > "$script"
    sh "$script" -y --profile minimal

    if [[ -f "$cargo_env" ]]; then
        # shellcheck disable=SC1090
        . "$cargo_env"
    fi
    command -v cargo >/dev/null || { echo "Rust was installed but cargo is still not in PATH; open a new terminal and run this script again." >&2; exit 1; }
    echo "Rust installed. Open a new terminal (or run '. $cargo_env') to use cargo yourself."
}

if [[ $UNINSTALL -eq 1 ]]; then
    rm -fv "$BIN/sourizz" "$BIN/sourizz-settings" "$BIN/sourizz-restart" "$APPS/sourizz.desktop" "$APPS/sourizz-settings.desktop" "$ICONS/sourizz.svg" "$DOCS/LICENSE"
    rmdir "$DOCS" 2>/dev/null || true
    rm -fv "${XDG_CONFIG_HOME:-$HOME/.config}/autostart/sourizz.desktop"
    echo "Uninstalled. Your settings in ${XDG_CONFIG_HOME:-$HOME/.config}/sourizz were kept."
    exit 0
fi

ensure_rust

cargo build --release
install -Dm755 target/release/sourizz "$BIN/sourizz"
install -Dm755 target/release/sourizz-settings "$BIN/sourizz-settings"
install -Dm755 packaging/sourizz-restart "$BIN/sourizz-restart"
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
