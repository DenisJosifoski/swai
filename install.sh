#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="$SCRIPT_DIR/target/release/swai"
USER="${USER:-$(whoami)}"

echo "=== SWAI (SWitch AI) installer ==="

# 1. Build release binary
echo "[1/5] Building release binary..."
if ! cargo build --release --package swai; then
    echo "ERROR: Build failed. Fix errors and re-run." >&2
    exit 1
fi

# Verify the binary exists
if [[ ! -x "$BINARY" ]]; then
    echo "ERROR: Expected binary not found at $BINARY" >&2
    exit 1
fi
echo "      Binary built: $BINARY"

# 2. Install binary to ~/.local/bin/
echo "[2/5] Installing binary..."
mkdir -p "$HOME/.local/bin"
cp -f "$BINARY" "$HOME/.local/bin/swai"
chmod +x "$HOME/.local/bin/swai"
echo "      Installed to: $HOME/.local/bin/swai"

# 3. Install icon
echo "[3/5] Installing icon..."
ICON_PNG_SRC="$SCRIPT_DIR/swai.png"

PNG_DIR="$HOME/.local/share/icons/hicolor/512x512/apps"
PIXMAPS_DIR="$HOME/.local/share/pixmaps"
ICONS_DIR="$HOME/.local/share/icons"
mkdir -p "$PNG_DIR" "$PIXMAPS_DIR" "$ICONS_DIR"

if [[ -f "$ICON_PNG_SRC" ]]; then
    cp -f "$ICON_PNG_SRC" "$PNG_DIR/swai.png"
    cp -f "$ICON_PNG_SRC" "$PIXMAPS_DIR/swai.png"
    cp -f "$ICON_PNG_SRC" "$ICONS_DIR/swai.png"
    echo "      PNG icon installed to: $PNG_DIR/swai.png & $PIXMAPS_DIR/swai.png"
fi

if command -v gtk-update-icon-cache &>/dev/null; then
    gtk-update-icon-cache -f "$HOME/.local/share/icons/hicolor/" 2>/dev/null || true
    echo "      Icon cache updated."
fi

# 4. Install desktop entry
echo "[4/5] Writing desktop entry..."
mkdir -p "$HOME/.local/share/applications"
ICON_TARGET="$HOME/.local/share/icons/hicolor/512x512/apps/swai.png"

cat > "$HOME/.local/share/applications/swai.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=SWAI
Comment=Native GTK4 local AI model switcher & Anthropic Gateway proxy
Exec=$HOME/.local/bin/swai
Icon=$ICON_TARGET
Categories=Development;Utility;ArtificialIntelligence;
Terminal=false
StartupWMClass=com.swai.app
DESKTOP
echo "      Installed to: $HOME/.local/share/applications/swai.desktop"

if [[ -d "$HOME/Desktop" ]]; then
    cat > "$HOME/Desktop/swai.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=SWAI
Comment=Native GTK4 local AI model switcher & Anthropic Gateway proxy
Exec=$HOME/.local/bin/swai
Icon=$ICON_TARGET
Categories=Development;Utility;ArtificialIntelligence;
Terminal=false
StartupWMClass=com.swai.app
DESKTOP
    chmod +x "$HOME/Desktop/swai.desktop"
    gio set "$HOME/Desktop/swai.desktop" metadata::trusted true 2>/dev/null || true
    echo "      Desktop shortcut created: $HOME/Desktop/swai.desktop"
fi

if command -v update-desktop-database &>/dev/null; then
    update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
fi
if command -v kbuildsycoca6 &>/dev/null; then
    kbuildsycoca6 --noincremental &>/dev/null || true
elif command -v kbuildsycoca5 &>/dev/null; then
    kbuildsycoca5 --noincremental &>/dev/null || true
fi
echo "      Desktop menu database refreshed."

# 5. Done
echo ""
echo "=== SWAI v1 installed successfully! ==="
echo ""
echo "Launch options:"
echo "  • From terminal:  swai"
echo "  • From app menu:  search for \"SWAI\" in your desktop launcher"
echo ""
echo "Configuration is at: $HOME/.config/swai/config.toml"
echo "Run 'swai --help' or check the README for usage details."
