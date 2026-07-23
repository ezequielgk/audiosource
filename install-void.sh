#!/bin/bash

set -e

REPO="ezequielgk/audiosource"

CYAN='\033[0;36m'
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
NC='\033[0m'

info()    { echo -e "${CYAN}  →${NC} $1"; }
success() { echo -e "${GREEN}  ✔${NC} $1"; }
error()   { echo -e "${RED}  ✘${NC} $1"; }
title()   { echo -e "\n${BOLD}$1${NC}"; }

command_exists() {
    command -v "$1" >/dev/null 2>&1
}

run_privileged() {
    if [ "$(id -u)" -eq 0 ]; then
        "$@"
    elif command_exists sudo; then
        sudo "$@"
    else
        error "Root privileges are required to install dependencies (sudo not found)."
        return 1
    fi
}

# Change to the script's directory
cd "$(dirname "$0")" 2>/dev/null || true

PREFIX="${PREFIX:-$HOME/.local}"
INSTALL_DIR="$PREFIX/share/audiosource"
BIN_DIR="$PREFIX/bin"
DESKTOP_DIR="$PREFIX/share/applications"

install_deps() {
    title "Checking system dependencies for Void Linux (musl/glibc)..."
    local missing=()
    
    # Check commands
    command_exists adb || missing+=("android-tools")
    command_exists pactl || missing+=("pulseaudio-utils")
    command_exists python3 || missing+=("python3")
    
    if [ "${#missing[@]}" -gt 0 ] || ! python3 -c 'import gi' 2>/dev/null; then
        info "Attempting to install missing dependencies via xbps..."
        if command_exists xbps-install; then
            run_privileged xbps-install -Sy python3 python3-gobject gtk+3 libayatana-appindicator android-tools pulseaudio-utils
        else
            error "xbps-install not found. This script is intended specifically for Void Linux."
            return 1
        fi
        success "Dependencies installed."
    else
        success "All required dependencies are already installed."
    fi
}

setup_path() {
    # If BIN_DIR is not in PATH, add it
    if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
        title "Configuring PATH..."
        local path_line="export PATH=\"\$PATH:$BIN_DIR\""
        local fish_path_line="fish_add_path $BIN_DIR"
        local updated=false

        local shell_files=("$HOME/.bashrc" "$HOME/.zshrc" "$HOME/.bash_profile" "$HOME/.profile")
        
        for file in "${shell_files[@]}"; do
            if [ -f "$file" ]; then
                if ! grep -q "$BIN_DIR" "$file"; then
                    echo -e "\n# Audio Source\n$path_line" >> "$file"
                    updated=true
                fi
            fi
        done

        if [ -d "$HOME/.config/fish" ]; then
            local fish_conf="$HOME/.config/fish/config.fish"
            mkdir -p "$(dirname "$fish_conf")"
            if ! grep -q "$BIN_DIR" "$fish_conf" 2>/dev/null; then
                echo -e "\n# Audio Source\n$fish_path_line" >> "$fish_conf"
                updated=true
            fi
        fi

        if [ "$updated" = true ]; then
            success "PATH configured successfully."
            echo -e "${YELLOW}${BOLD}⚠ IMPORTANT:${NC} Restart your terminal or run: ${CYAN}source ~/.bashrc${NC}"
        else
            info "PATH is already configured."
        fi
    fi
}

install_app() {
    title "Installing Audio Source (Void Linux Edition)..."
    install_deps || exit 1

    info "Copying files to $INSTALL_DIR..."
    mkdir -p "$INSTALL_DIR" "$BIN_DIR" "$DESKTOP_DIR"
    cp -r desktop assets "$INSTALL_DIR/"

    local CONFIG_DIR="$HOME/.config/audiosource"
    mkdir -p "$CONFIG_DIR"
    if [ ! -f "$CONFIG_DIR/ascii.txt" ]; then
        info "Creating default ASCII configuration..."
        cp "$INSTALL_DIR/desktop/ascii.txt" "$CONFIG_DIR/ascii.txt"
    fi

    chmod +x "$INSTALL_DIR/desktop/tui.py" "$INSTALL_DIR/desktop/tray.py" "$INSTALL_DIR/desktop/launcher.sh"

    info "Setting up executable wrapper..."
    cp "$INSTALL_DIR/desktop/launcher.sh" "$BIN_DIR/audiosource"
    chmod +x "$BIN_DIR/audiosource"

    info "Creating desktop entry..."
    cat << EOF > "$DESKTOP_DIR/audiosource.desktop"
[Desktop Entry]
Name=Audio Source
Comment=Use your Android device as a USB microphone
Exec=$BIN_DIR/audiosource
Icon=$INSTALL_DIR/assets/icon.svg
Terminal=true
Type=Application
Categories=AudioVideo;Audio;
EOF

    if command_exists update-desktop-database; then
        update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true
    fi

    success "Installation complete!"
    setup_path
}

uninstall_app() {
    title "Uninstalling Audio Source..."
    
    local removed=false
    if [ -d "$INSTALL_DIR" ]; then
        rm -rf "$INSTALL_DIR"
        removed=true
    fi
    if [ -f "$BIN_DIR/audiosource" ]; then
        rm -f "$BIN_DIR/audiosource"
        removed=true
    fi
    if [ -f "$DESKTOP_DIR/audiosource.desktop" ]; then
        rm -f "$DESKTOP_DIR/audiosource.desktop"
        if command_exists update-desktop-database; then
            update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true
        fi
        removed=true
    fi
    
    if [ "$removed" = true ]; then
        success "Binaries and desktop entries removed."
    else
        error "Audio Source installation not found."
    fi
}

main_menu() {
    clear
    echo -e "${CYAN}${BOLD}AUDIO SOURCE INSTALLER (VOID LINUX)${NC}"
    echo ""
    echo "  Select an option:"
    echo ""
    echo -e "  ${CYAN}1)${NC} Install Audio Source"
    echo -e "  ${CYAN}2)${NC} Uninstall Audio Source"
    echo -e "  ${CYAN}3)${NC} Install System Dependencies"
    echo -e "  ${CYAN}4)${NC} Exit"
    echo ""
    
    read -rp "  Option [1-4]: " opcion < /dev/tty

    case "$opcion" in
        1) install_app ;;
        2) uninstall_app ;;
        3) install_deps ;;
        4) exit 0 ;;
        *) sleep 1; main_menu ;;
    esac

    echo ""
    read -rp "  Press Enter to continue..." _ < /dev/tty
    main_menu
}

if [ "$1" == "--install" ]; then
    install_app
    exit 0
elif [ "$1" == "--uninstall" ]; then
    uninstall_app
    exit 0
elif [ "$1" == "--deps" ]; then
    install_deps
    exit 0
fi

main_menu
