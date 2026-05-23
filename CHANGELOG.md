# Changelog

All notable changes to this project will be documented in this file.

## [1.1.0] - 2026-05-22

### Added
- **Persistent Audio State**: The application now saves the microphone's mute state and specific volume percentage (`~/.config/audiosource/config.json`) and automatically restores them across application restarts and system reboots.
- **Dynamic Connection Indicator**: The volume indicator gracefully switches to `[Waiting for connection...]` when the Android app is not actively transmitting.
- **Version Capsule**: Added a clear `v1.1.0` version indicator to the top right of the TUI.
- **New Volume Keybinds**: Replaced `+`/`-` with more accessible `Z` (Volume Down) and `X` (Volume Up) shortcuts.

### Changed
- **TUI Redesign**: Moved the `MICROPHONE ACTIVE / MUTED` status capsules from the bottom to the top left of the interface alongside the title.
- **Log Panel Cleanup**: Removed redundant volume adjustment messages from the event logs. Volume feedback is now exclusively reflected in real-time on the dedicated visual indicator.
- **Streamlined Controls**: Renamed `[Q] Quit All` to `[Q] Quit` for better readability.

### Fixed
- **PipeWire Source Name**: Renamed the virtual microphone source to `AudioSource_Microphone` (removed spaces) to prevent name truncation bugs in modern PipeWire/PulseAudio environments.
- **Robust ADB Detection**: The daemon now reliably targets the first available ADB device instead of failing when multiple emulators or devices are connected.
- **Startup Race Condition**: Added a safety delay during Android app launch to prevent `socat` "Broken pipe" crashes caused by the socket not being ready.
- **TUI Corruption Bug**: Hid raw `pactl` error outputs (`stderr`) that were bleeding into the terminal and breaking the `curses` layout when adjusting volume before a full connection.

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
