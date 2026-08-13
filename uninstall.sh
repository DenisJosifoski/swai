#!/usr/bin/env bash
set -euo pipefail

USER="${USER:-$(whoami)}"

echo "=== SWAI uninstaller ==="

# 1. Remove binary
if [[ -f "$HOME/.local/bin/swai" ]]; then
    rm -f "$HOME/.local/bin/swai"
    echo "[1/4] Removed: $HOME/.local/bin/swai"
fi

# 2. Remove desktop entry
if [[ -f "$HOME/.local/share/applications/swai.desktop" ]]; then
    rm -f "$HOME/.local/share/applications/swai.desktop"
    echo "[2/4] Removed: $HOME/.local/share/applications/swai.desktop"
fi
rm -f "$HOME/Desktop/swai.desktop" 2>/dev/null || true

# 3. Remove icon
if [[ -f "$HOME/.local/share/icons/hicolor/512x512/apps/swai.png" ]]; then
    rm -f "$HOME/.local/share/icons/hicolor/512x512/apps/swai.png"
    echo "[3/4] Removed: $HOME/.local/share/icons/hicolor/512x512/apps/swai.png"

    if command -v gtk-update-icon-cache &>/dev/null; then
        gtk-update-icon-cache -f "$HOME/.local/share/icons/hicolor/" 2>/dev/null || true
        echo "      Icon cache updated."
    fi
fi

echo ""
echo "=== SWAI binary & launcher uninstalled successfully ==="
echo ""

if [[ -t 0 ]]; then
    read -r -p "Do you also want to remove user configuration and logs (~/.config/swai & ~/.local/share/swai)? [y/N] " response
    case "$response" in
        [yY][eE][sS]|[yY])
            rm -rf "$HOME/.config/swai" "$HOME/.local/share/swai"
            echo "Removed user configuration and log directories."
            ;;
        *)
            echo "Preserved user configuration and logs."
            ;;
    esac
else
    echo "User configuration and logs preserved at:"
    echo "  • Config:  $HOME/.config/swai/"
    echo "  • Logs:    $HOME/.local/share/swai/logs/"
    echo "Run 'rm -rf ~/.config/swai ~/.local/share/swai' to clean those up."
fi
