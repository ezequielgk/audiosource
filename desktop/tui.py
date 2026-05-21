#!/usr/bin/env python3
"""
Terminal User Interface (TUI) for AudioSource.

Provides a rich, curses-based graphical interface in the terminal.
This script acts strictly as a frontend, controlling the background tray
daemon via Unix signals (IPC) and reading logs/audio visualization data
from shared resources, ensuring the UI remains highly responsive.
"""
import curses
import subprocess
import threading
import queue
import time
import struct
import os
import fcntl
import hashlib
import select
import signal
import sys

__version__ = "1.0.1"

import json

ASCII_LOGO = " AudioSource TUI "
USER_CONFIG_DIR = os.path.expanduser("~/.config/audiosource")
USER_ASCII_TXT = os.path.join(USER_CONFIG_DIR, "ascii.txt")
USER_CONFIG_JSON = os.path.join(USER_CONFIG_DIR, "config.json")
BUNDLE_ASCII_TXT = os.path.join(os.path.dirname(__file__), "ascii.txt")

try:
    if os.path.exists(USER_CONFIG_JSON):
        with open(USER_CONFIG_JSON, "r", encoding="utf-8") as f:
            config = json.load(f)
            if "ascii_art" in config:
                ASCII_LOGO = config["ascii_art"]
except Exception:
    pass

if ASCII_LOGO == " AudioSource TUI ":
    try:
        with open(USER_ASCII_TXT, "r", encoding="utf-8") as f:
            ASCII_LOGO = f.read()
    except Exception:
        pass

if ASCII_LOGO == " AudioSource TUI ":
    try:
        with open(BUNDLE_ASCII_TXT, "r", encoding="utf-8") as f:
            ASCII_LOGO = f.read()
    except Exception:
        pass

def get_audiosource_name(serial=None):
    """
    Generate the expected PulseAudio source name.
    
    Must perfectly match the hashing algorithm used in audiosource.py
    so the visualizer connects to the correct virtual microphone.
    """
    serial_str = serial if serial else ""
    if not serial_str and "ANDROID_SERIAL" in os.environ:
        serial_str = os.environ["ANDROID_SERIAL"]
    hash_str = hashlib.sha256(serial_str.encode()).hexdigest()[:7]
    return os.environ.get("AUDIOSOURCE_NAME", f"android-{hash_str}")

PID_FILE = "/tmp/audiosource_tray.pid"
LOG_FILE = "/tmp/audiosource.log"

class AudioSourceTUI:
    """
    Manages the curses application lifecycle and UI rendering.
    
    Features non-blocking keyboard input and dedicated background threads
    for log tailing and live audio visualization processing.
    """
    def __init__(self, stdscr):
        self.stdscr = stdscr
        self.log_queue = queue.Queue()
        self.logs = []
        self.running_tui = True
        self.parec_process = None
        self.volume_level = 0.0
        self.tray_pid = self._get_tray_pid()
        
        curses.start_color()
        curses.use_default_colors()
        curses.init_pair(1, curses.COLOR_GREEN, -1)
        curses.init_pair(2, curses.COLOR_RED, -1)
        curses.init_pair(3, curses.COLOR_CYAN, -1)
        curses.init_pair(4, curses.COLOR_YELLOW, -1)
        curses.init_pair(5, curses.COLOR_MAGENTA, -1)
        
        curses.curs_set(0)
        self.stdscr.timeout(50)
        self.max_y, self.max_x = self.stdscr.getmaxyx()
        
        # Ensure the background daemon is running so the TUI has a backend to talk to
        if not self.tray_pid:
            self.log_queue.put("Starting Tray Icon in background...")
            subprocess.Popen([os.path.join(os.path.dirname(__file__), "tray.py")], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            time.sleep(1)
            self.tray_pid = self._get_tray_pid()

    def _get_tray_pid(self):
        """Check if the tray daemon is alive by testing signal 0 against its PID."""
        if os.path.exists(PID_FILE):
            try:
                with open(PID_FILE, "r") as f:
                    pid = int(f.read().strip())
                os.kill(pid, 0)
                return pid
            except Exception:
                pass
        return None

    def _send_signal(self, sig):
        """Send IPC signal to the background daemon (e.g., SIGUSR1 to start audio)."""
        self.tray_pid = self._get_tray_pid()
        if self.tray_pid:
            try:
                os.kill(self.tray_pid, sig)
            except OSError:
                pass

    def run(self):
        """
        Main TUI loop.
        
        Handles keyboard input, orchestrates rendering, and checks for
        daemon state changes. Automatically exits if the tray daemon dies.
        """
        self.log_queue.put("TUI Connected to Daemon.")
        
        threading.Thread(target=self._read_logs, daemon=True).start()
        threading.Thread(target=self._read_audio, daemon=True).start()
        
        while self.running_tui:
            if not self._get_tray_pid():
                # Clean graceful exit if the user quit the app from the Tray menu
                self.running_tui = False
                break
                
            self.max_y, self.max_x = self.stdscr.getmaxyx()
            self.draw()
            
            while not self.log_queue.empty():
                msg = self.log_queue.get()
                self.logs.append(msg)
                if len(self.logs) > self.max_y - 10:
                    self.logs.pop(0)

            try:
                c = self.stdscr.getch()
                if c == ord('q') or c == ord('Q'):
                    self._send_signal(signal.SIGTERM)
                    self.running_tui = False
                elif c == ord('t') or c == ord('T'):
                    self.running_tui = False
                elif c == ord('s') or c == ord('S'):
                    if not self._get_tray_pid():
                        self.log_queue.put("Starting Tray Icon in background...")
                        subprocess.Popen([os.path.join(os.path.dirname(__file__), "tray.py")], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                        time.sleep(1)
                        self.tray_pid = self._get_tray_pid()
                    self._send_signal(signal.SIGUSR1)
                elif c == ord('r') or c == ord('R'):
                    self.log_queue.put("Restarting TUI and Tray...")
                    self._send_signal(signal.SIGTERM)
                    time.sleep(0.5)
                    # Hard restart via execv to completely reset app state
                    os.execv(sys.executable, ['python3', os.path.abspath(__file__)])
                elif c == ord('c') or c == ord('C'):
                    self._send_signal(signal.SIGUSR2)
                elif c == ord('m') or c == ord('M'):
                    source_name = get_audiosource_name()
                    try:
                        res = subprocess.run(["pactl", "get-source-volume", source_name], capture_output=True, text=True)
                        # We use 0% volume instead of pactl mute because virtual pipes ignore standard mute flags
                        if " 0%" in res.stdout:
                            subprocess.run(["pactl", "set-source-volume", source_name, "100%"])
                            self.log_queue.put("Mic Unmuted (100%).")
                        else:
                            subprocess.run(["pactl", "set-source-volume", source_name, "0%"])
                            self.log_queue.put("Mic Muted (0%).")
                    except Exception:
                        pass
            except curses.error:
                pass
                
        if self.parec_process:
            self.parec_process.terminate()

    def draw(self):
        """Render all visual elements onto the curses standard screen."""
        self.stdscr.erase()
        
        try:
            self.stdscr.border()
        except curses.error:
            pass
        
        title = " AudioSource TUI "
        try:
            self.stdscr.addstr(0, max(0, (self.max_x - len(title)) // 2), title, curses.A_BOLD | curses.color_pair(3) | curses.A_REVERSE)
        except curses.error:
            pass

        logo_lines = [line.rstrip() for line in ASCII_LOGO.strip('\n').split('\n')]
        
        max_allowed_logo_height = self.max_y - 13
        
        if max_allowed_logo_height > 0:
            logo_lines = logo_lines[:max_allowed_logo_height]
            logo_height = len(logo_lines)
            max_logo_width = max((len(line) for line in logo_lines), default=0)
            
            logo_start_x = max(2, (self.max_x - max_logo_width) // 2)
            logo_start_y = 2
            
            for i, line in enumerate(logo_lines):
                try:
                    # Truncate horizontal to avoid wrapping errors
                    display_line = line[:max(0, self.max_x - logo_start_x - 1)]
                    self.stdscr.addstr(logo_start_y + i, logo_start_x, display_line, curses.color_pair(3) | curses.A_BOLD)
                except curses.error:
                    pass
        else:
            logo_height = 0
            logo_start_y = 1
        
        log_start_y = logo_start_y + logo_height + 1
        log_end_y = self.max_y - 8
        try:
            self.stdscr.hline(log_start_y - 1, 1, curses.ACS_HLINE, self.max_x - 2)
            self.stdscr.addstr(log_start_y - 1, 2, " Logs ", curses.A_BOLD | curses.color_pair(5))
        except curses.error:
            pass
        
        for i, log in enumerate(self.logs[- (log_end_y - log_start_y + 1):]):
            if log_start_y + i <= log_end_y:
                try:
                    color = curses.color_pair(0)
                    if "Error" in log or "Failed" in log or "exited" in log or "Stop" in log:
                        color = curses.color_pair(2)
                    elif "Waiting" in log:
                        color = curses.color_pair(4)
                    elif "Start" in log:
                        color = curses.color_pair(1)
                    
                    self.stdscr.addstr(log_start_y + i, 3, "│ " + log[:self.max_x - 7], color)
                except curses.error:
                    pass

        vis_y = self.max_y - 5
        try:
            self.stdscr.hline(vis_y - 1, 1, curses.ACS_HLINE, self.max_x - 2)
            self.stdscr.addstr(vis_y - 1, 2, " Mic Volume ", curses.A_BOLD | curses.color_pair(3))
        except curses.error:
            pass
            
        bar_width = self.max_x - 18
        if bar_width > 0:
            filled = int(self.volume_level * bar_width)
            
            try:
                self.stdscr.addstr(vis_y, 3, "Vol: ", curses.A_BOLD)
                self.stdscr.addstr(vis_y, 8, "║")
                for i in range(bar_width):
                    if i < filled:
                        pct = i / bar_width
                        if pct < 0.6: color = curses.color_pair(1) | curses.A_BOLD
                        elif pct < 0.85: color = curses.color_pair(4) | curses.A_BOLD
                        else: color = curses.color_pair(2) | curses.A_BOLD
                        self.stdscr.addstr(vis_y, 9 + i, "█", color)
                    else:
                        self.stdscr.addstr(vis_y, 9 + i, "░", curses.A_DIM)
                self.stdscr.addstr(vis_y, 9 + bar_width, "║")
            except curses.error:
                pass

        ctrl_y = self.max_y - 2
        try:
            self.stdscr.hline(ctrl_y - 1, 1, curses.ACS_HLINE, self.max_x - 2)
        except curses.error:
            pass
            
        status = " DAEMON ACTIVE " if self._get_tray_pid() else " DAEMON OFFLINE "
        color = curses.color_pair(1) | curses.A_REVERSE | curses.A_BOLD if self._get_tray_pid() else curses.color_pair(2) | curses.A_REVERSE | curses.A_BOLD
        try:
            self.stdscr.addstr(ctrl_y, 2, status, color)
            
            controls = "[S] Start  [R] Restart  [C] Stop  [M] Mute  [T] Hide  [Q] Quit All"
            self.stdscr.addstr(ctrl_y, self.max_x - len(controls) - 2, controls, curses.A_BOLD)
        except curses.error:
            pass
        
        self.stdscr.refresh()

    def _read_logs(self):
        """
        Background thread for tailing the log file.
        
        Uses a continuous polling loop with seek(0, 2) on startup
        to ensure we only show fresh logs, preventing old logs
        from flooding the screen on restart.
        """
        while not os.path.exists(LOG_FILE) and self.running_tui:
            time.sleep(0.5)
            
        if not self.running_tui:
            return
            
        with open(LOG_FILE, "r") as f:
            f.seek(0, 2)
            
            while self.running_tui:
                line = f.readline()
                if not line:
                    time.sleep(0.1)
                    continue
                self.log_queue.put(line.rstrip())

    def _read_audio(self):
        """
        Background thread for live audio visualization.
        
        Spawns a 'parec' process to capture raw PCM audio from the virtual mic.
        Dynamically handles PulseAudio source removal to prevent 'parec' from
        fallback-migrating to the user's laptop microphone.
        """
        source_name = get_audiosource_name()
        
        while self.running_tui:
            try:
                # Security check: Ensure our specific android mic exists before reading.
                # If we omit this, parec will silently fallback to the laptop microphone.
                res = subprocess.run(["pactl", "list", "short", "sources"], capture_output=True, text=True)
                if source_name not in res.stdout:
                    self.volume_level = 0.0
                    time.sleep(1)
                    continue
            except Exception:
                pass
                
            cmd = ["parec", "-d", source_name, "--format=s16le", "--channels=1"]
            try:
                self.parec_process = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
                fd = self.parec_process.stdout.fileno()
                fl = fcntl.fcntl(fd, fcntl.F_GETFL)
                fcntl.fcntl(fd, fcntl.F_SETFL, fl | os.O_NONBLOCK)

                last_check = time.time()
                while self.running_tui and self.parec_process.poll() is None:
                    # Periodically check if PulseAudio deleted our source underneath us
                    # (e.g. if the user pressed Stop).
                    if time.time() - last_check > 2.0:
                        try:
                            res = subprocess.run(["pactl", "list", "short", "sources"], capture_output=True, text=True)
                            if source_name not in res.stdout:
                                break
                        except Exception:
                            pass
                        last_check = time.time()

                    ready, _, _ = select.select([self.parec_process.stdout], [], [], 0.1)
                    if ready:
                        try:
                            data = self.parec_process.stdout.read(4096)
                            if not data:
                                continue
                            
                            samples = len(data) // 2
                            if samples > 0:
                                unpacked = struct.unpack(f"<{samples}h", data)
                                peak = max(abs(s) for s in unpacked)
                                self.volume_level = min(1.0, peak / 20000.0)
                        except IOError:
                            pass
                    else:
                        self.volume_level = max(0.0, self.volume_level - 0.1)
                        
            except Exception:
                pass
            finally:
                self.volume_level = 0.0
                if self.parec_process:
                    self.parec_process.terminate()
                    self.parec_process.wait()
            
            time.sleep(1)

def do_update():
    import urllib.request
    import json
    import tempfile
    import tarfile
    import shutil

    print("Checking for updates from GitHub...")
    try:
        url = "https://api.github.com/repos/ezequielgk/audiosource/releases/latest"
        req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
        with urllib.request.urlopen(req) as response:
            data = json.loads(response.read().decode())
        
        assets = data.get("assets", [])
        download_url = None
        for asset in assets:
            if asset["name"] == "audiosource-linux.tar.gz":
                download_url = asset["browser_download_url"]
                break
        
        if not download_url:
            print("Could not find the release asset 'audiosource-linux.tar.gz'.")
            return

        print(f"Downloading latest release ({data.get('tag_name')})...")
        with tempfile.TemporaryDirectory() as tmpdir:
            tar_path = os.path.join(tmpdir, "release.tar.gz")
            urllib.request.urlretrieve(download_url, tar_path)
            
            print("Extracting...")
            with tarfile.open(tar_path, "r:gz") as tar:
                if hasattr(tarfile, 'data_filter'):
                    tar.extractall(path=tmpdir, filter="data")
                else:
                    tar.extractall(path=tmpdir)
            
            extracted_dir = os.path.join(tmpdir, "audiosource-linux")
            if not os.path.exists(extracted_dir):
                extracted_dir = tmpdir
                
            install_script = os.path.join(extracted_dir, "install.sh")
            if os.path.exists(install_script):
                print("Running installation script...")
                subprocess.run(["bash", install_script, "--install"], cwd=extracted_dir)
                print("Update complete!")
            else:
                print("Error: install.sh not found in the downloaded release.")
                
    except Exception as e:
        print(f"Update failed: {e}")

def main():
    if len(sys.argv) > 1:
        arg = sys.argv[1]
        if arg == "update":
            do_update()
            sys.exit(0)
        elif arg in ("-v", "--version", "version"):
            print(f"Audio Source Linux Client - v{__version__}")
            sys.exit(0)
        elif arg in ("-h", "--help", "help"):
            print("Audio Source Linux Client")
            print("Usage: audiosource [OPTIONS]")
            print("")
            print("Options:")
            print("  -h, --help       Show this help message and exit")
            print("  -v, --version    Show version information and exit")
            print("  update           Download and install the latest release from GitHub")
            print("")
            print("Running 'audiosource' without arguments will launch the Terminal User Interface.")
            sys.exit(0)
        
    try:
        curses.wrapper(lambda stdscr: AudioSourceTUI(stdscr).run())
    except KeyboardInterrupt:
        pass

if __name__ == "__main__":
    main()
