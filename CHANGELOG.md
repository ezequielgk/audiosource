# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added
- **Terminal User Interface (TUI)** for controlling the application via an interactive console, making it easier to start/stop the mic and adjust volumes.
- **System Tray Icon Daemon** for running in the background and managing audio streams without requiring an open terminal window.
- **Automated Linux Installation Script** (`install.sh`) with an interactive text menu for installation, uninstallation, and automatic resolution of system dependencies via `apt`, `pacman`, or `dnf`.
- **GitHub Actions Workflow** (`.github/workflows/release.yml`) to automatically package and release the Linux Client whenever a `v*` tag is pushed.
- **Custom ASCII Art Support**: The TUI now dynamically loads custom ASCII art from `~/.config/audiosource/config.json` or `~/.config/audiosource/ascii.txt`.
- **TUI Install Hotkey**: Added the `[I] Install` shortcut directly inside the TUI to run the installation process to your system seamlessly.

### Fixed
- Fixed visual TUI glitches when rendering ASCII art with trailing whitespaces or uneven line lengths. 
- Prevented TUI layout crashes by implementing a dynamic bounding-box that safely truncates ASCII art if it exceeds the terminal dimensions, prioritizing the visibility of the logs and controls.
- Corrected the left-border horizontal line alignment mismatch between the Logs and Mic Volume sections in the TUI.
