# Changelog

All notable changes to this project will be documented in this file.

## [1.0.1] - 2024-05-21

### Added
- **One-liner Curl Installation**: Added a bootstrap system in `install.sh` allowing users to install the application instantly by piping via `curl`.
- **CLI Updates**: Added `audiosource update` command to automatically fetch, extract, and install the latest release directly from GitHub.
- **CLI Help & Version**: Added standard `-h`, `--help`, `-v`, and `--version` arguments to the `audiosource` global command.

## [1.0.0] - 2024-05-21

### Added
- **Terminal User Interface (TUI)** for controlling the application via an interactive console, making it easier to start/stop the mic and adjust volumes.
- **System Tray Icon Daemon** for running in the background and managing audio streams without requiring an open terminal window.
- **Automated Linux Installation Script** (`install.sh`) with an interactive text menu for installation, uninstallation, and automatic resolution of system dependencies via `apt`, `pacman`, or `dnf`.
- **GitHub Actions Workflow** (`.github/workflows/release.yml`) to automatically package and release the Linux Client whenever a `v*` tag is pushed.
- **Custom ASCII Art Support**: The TUI now dynamically loads custom ASCII art from `~/.config/audiosource/config.json` or `~/.config/audiosource/ascii.txt`.

### Fixed
- Fixed visual TUI glitches when rendering ASCII art with trailing whitespaces or uneven line lengths. 
- Prevented TUI layout crashes by implementing a dynamic bounding-box that safely truncates ASCII art if it exceeds the terminal dimensions, prioritizing the visibility of the logs and controls.
- Corrected the left-border horizontal line alignment mismatch between the Logs and Mic Volume sections in the TUI.
