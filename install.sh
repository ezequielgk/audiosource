#!/bin/bash

set -e

cd "$(dirname "$0")"

PREFIX="${PREFIX:-$HOME/.local}"
INSTALL_DIR="$PREFIX/share/audiosource"
BIN_DIR="$PREFIX/bin"
DESKTOP_DIR="$PREFIX/share/applications"

echo "Installing Audio Source to $INSTALL_DIR..."

mkdir -p "$INSTALL_DIR"
mkdir -p "$BIN_DIR"
mkdir -p "$DESKTOP_DIR"

echo "Copying files..."
cp -r desktop assets "$INSTALL_DIR/"

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
echo ""
echo "You can now launch the app from your application menu as 'Audio Source',"
echo "or by running 'audiosource' in your terminal."
echo ""
echo "Please ensure you have the following system dependencies installed:"
echo " - android-tools (for adb)"
echo " - pulseaudio or pipewire-pulse (for pactl and parec)"
echo " - python3"
echo " - python3-gi, python3-gi-cairo, gir1.2-gtk-3.0, gir1.2-ayatanaappindicator3-0.1 (for the System Tray)"
echo ""
echo "Note: If '$BIN_DIR' is not in your PATH, you may need to add it or restart your session."
