#!/bin/bash

set -e

# Support for one-liner installation via `curl | bash`
# Use an environment variable to prevent infinite bootstrap loop
if [ -z "$AUDIOSOURCE_BOOTSTRAPPED" ] && ([ ! -d "desktop" ] || [ ! -d "assets" ]); then
    echo "=================================================="
    echo " Audio Source Bootstrap Installer "
    echo "=================================================="
    echo "Downloading latest release from GitHub..."
    TMP_DIR=$(mktemp -d)
    trap "rm -rf $TMP_DIR" EXIT
    
    # Fetch the latest release info
    RELEASE_JSON=$(curl -sSL https://api.github.com/repos/ezequielgk/audiosource/releases/latest)
    
    # Debug: check if we got a valid response
    if echo "$RELEASE_JSON" | grep -q "message.*API rate limit"; then
        echo "Error: GitHub API rate limit exceeded. Please try again later or install manually."
        exit 1
    fi
    
    # Try to find the Linux release file
    LATEST_URL=$(echo "$RELEASE_JSON" | grep "browser_download_url.*audiosource-linux.tar.gz" | head -1 | cut -d '"' -f 4)
    
    if [ -z "$LATEST_URL" ]; then
        echo "Error: Could not find audiosource-linux.tar.gz in latest release."
        echo "Available assets:"
        echo "$RELEASE_JSON" | grep "browser_download_url" | cut -d '"' -f 4
        echo ""
        echo "Please check GitHub or install manually from:"
        echo "https://github.com/ezequielgk/audiosource/releases/latest"
        exit 1
    fi
    
    echo "Found release: $LATEST_URL"
    curl -sSL "$LATEST_URL" -o "$TMP_DIR/release.tar.gz"
    tar -xzmf "$TMP_DIR/release.tar.gz" -C "$TMP_DIR"
    
    # Verify the extracted script exists before executing
    if [ ! -f "$TMP_DIR/audiosource-linux/install.sh" ]; then
        echo "Error: install.sh not found in extracted release."
        exit 1
    fi
    
    # Delegate to the actual script inside the extracted release
    export AUDIOSOURCE_BOOTSTRAPPED=1
    exec bash "$TMP_DIR/audiosource-linux/install.sh" "$@" < /dev/tty
fi

# Change to the script's directory (only applies when run from a local folder)
cd "$(dirname "$0")" 2>/dev/null || true

PREFIX="${PREFIX:-$HOME/.local}"
INSTALL_DIR="$PREFIX/share/audiosource"
BIN_DIR="$PREFIX/bin"
DESKTOP_DIR="$PREFIX/share/applications"

install_app() {
    echo "Installing Audio Source to $INSTALL_DIR..."

    mkdir -p "$INSTALL_DIR"
    mkdir -p "$BIN_DIR"
    mkdir -p "$DESKTOP_DIR"

    echo "Copying files..."
    cp -r desktop assets "$INSTALL_DIR/"

    CONFIG_DIR="$HOME/.config/audiosource"
    mkdir -p "$CONFIG_DIR"
    if [ ! -f "$CONFIG_DIR/ascii.txt" ]; then
        echo "Creating default ASCII configuration in $CONFIG_DIR/ascii.txt..."
        cp "$INSTALL_DIR/desktop/ascii.txt" "$CONFIG_DIR/ascii.txt"
    fi

    chmod +x "$INSTALL_DIR/desktop/tui.py"
    chmod +x "$INSTALL_DIR/desktop/tray.py"

    echo "Creating executable wrapper in $BIN_DIR/audiosource..."
    cat << 'EOF' > "$BIN_DIR/audiosource"
#!/bin/bash
# Wrapper to launch the Audio Source TUI
exec python3 "INSTALL_DIR_PLACEHOLDER/desktop/tui.py" "$@"
EOF

    sed -i "s|INSTALL_DIR_PLACEHOLDER|$INSTALL_DIR|g" "$BIN_DIR/audiosource"
    chmod +x "$BIN_DIR/audiosource"

    echo "Creating desktop entry..."
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

    if command -v update-desktop-database > /dev/null; then
        update-desktop-database "$DESKTOP_DIR" || true
    fi

    echo ""
    echo "=================================================="
    echo "Installation complete!"
    echo "=================================================="
    echo "You can now launch the app from your application menu as 'Audio Source',"
    echo "or by running 'audiosource' in your terminal."
    echo ""
}

uninstall_app() {
    echo "Uninstalling Audio Source..."
    
    if [ -d "$INSTALL_DIR" ]; then
        echo "Removing $INSTALL_DIR..."
        rm -rf "$INSTALL_DIR"
    fi
    
    if [ -f "$BIN_DIR/audiosource" ]; then
        echo "Removing executable $BIN_DIR/audiosource..."
        rm -f "$BIN_DIR/audiosource"
    fi
    
    if [ -f "$DESKTOP_DIR/audiosource.desktop" ]; then
        echo "Removing desktop entry $DESKTOP_DIR/audiosource.desktop..."
        rm -f "$DESKTOP_DIR/audiosource.desktop"
        if command -v update-desktop-database > /dev/null; then
            update-desktop-database "$DESKTOP_DIR" || true
        fi
    fi
    
    echo "=================================================="
    echo "Uninstallation complete."
    echo "=================================================="
    echo ""
}

install_deps() {
    echo "Attempting to install dependencies..."
    if command -v apt &> /dev/null; then
        echo "Detected Debian/Ubuntu base. Running apt..."
        sudo apt update
        sudo apt install -y android-tools-adb pulseaudio-utils python3 python3-gi python3-gi-cairo gir1.2-gtk-3.0 gir1.2-ayatanaappindicator3-0.1
        echo "Dependencies installed."
    elif command -v pacman &> /dev/null; then
        echo "Detected Arch Linux base. Running pacman..."
        sudo pacman -Sy --needed android-tools libpulse python python-gobject gtk3 libayatana-appindicator
        echo "Dependencies installed."
    elif command -v dnf &> /dev/null; then
        echo "Detected Fedora/RHEL base. Running dnf..."
        sudo dnf install -y android-tools pulseaudio-utils python3 python3-gobject gtk3 libayatana-appindicator
        echo "Dependencies installed."
    else
        echo "Could not detect package manager."
        echo "Please install these manually:"
        echo " - adb (android-tools)"
        echo " - pactl/parec (pulseaudio-utils)"
        echo " - python3, python3-gi, python3-gi-cairo, gir1.2-gtk-3.0, gir1.2-ayatanaappindicator3-0.1"
    fi
    echo ""
}

show_menu() {
    while true; do
        echo "=================================================="
        echo " Audio Source Installer Menu "
        echo "=================================================="
        echo "1) Install Audio Source"
        echo "2) Uninstall Audio Source"
        echo "3) Install System Dependencies (requires sudo)"
        echo "4) Exit"
        echo "=================================================="
        read -p "Select an option [1-4]: " option
        
        case $option in
            1) install_app; break ;;
            2) uninstall_app; break ;;
            3) install_deps ;;
            4) echo "Exiting."; exit 0 ;;
            *) echo "Invalid option. Please try again." ;;
        esac
    done
}

# If arguments are passed, allow CLI-style execution to skip the menu
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

# Show menu by default
show_menu
