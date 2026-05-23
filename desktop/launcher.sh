#!/bin/bash

# Resolves the path dynamically so it works from both the tarball and the installed location
if [ -f "$(dirname "$0")/tui.py" ]; then
    # When run directly from the desktop/ folder
    exec python3 "$(dirname "$0")/tui.py" "$@"
elif [ -f "$(dirname "$0")/desktop/tui.py" ]; then
    # When run from the root of the tarball (where it is copied as 'audiosource')
    exec python3 "$(dirname "$0")/desktop/tui.py" "$@"
elif [ -f "${XDG_DATA_HOME:-$HOME/.local/share}/audiosource/desktop/tui.py" ]; then
    # When run from the user's bin directory after installation
    exec python3 "${XDG_DATA_HOME:-$HOME/.local/share}/audiosource/desktop/tui.py" "$@"
else
    echo "Error: Could not find Audio Source TUI."
    exit 1
fi
