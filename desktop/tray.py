#!/usr/bin/env python3
"""
System Tray Daemon for AudioSource.

Acts as the background daemon running the PulseAudio bridge.
Maintains the GTK tray icon and listens to Unix signals from the TUI
to control the audio stream without requiring a persistent terminal window.
"""
import os
import signal
import subprocess
import sys
import gi

gi.require_version('Gtk', '3.0')
gi.require_version('AyatanaAppIndicator3', '0.1')
from gi.repository import Gtk, AyatanaAppIndicator3, GLib

PID_FILE = "/tmp/audiosource_tray.pid"
LOG_FILE = "/tmp/audiosource.log"

class AudioSourceTray:
    """
    Manages the GTK tray icon and the audiosource background process.
    
    This class orchestrates the lifecycle of the actual audio forwarding script.
    It writes its PID to a file so the frontend TUI can send it signals.
    """
    def __init__(self):
        self.script_dir = os.path.dirname(os.path.abspath(__file__))
        self.icon_path = os.path.join(os.path.dirname(self.script_dir), "assets", "icon.svg")
        
        # Expose PID to allow the TUI frontend to send signals (SIGUSR1, SIGTERM, etc.)
        with open(PID_FILE, "w") as f:
            f.write(str(os.getpid()))
            
        # Initialize log file. Truncating on startup ensures we don't leak space over time.
        with open(LOG_FILE, "w") as f:
            f.write("Tray Daemon Started.\n")
        self.log_file = open(LOG_FILE, "a")

        self.indicator = AyatanaAppIndicator3.Indicator.new(
            "audiosource-tray",
            self.icon_path,
            AyatanaAppIndicator3.IndicatorCategory.APPLICATION_STATUS
        )
        self.indicator.set_status(AyatanaAppIndicator3.IndicatorStatus.ACTIVE)
        self.indicator.set_menu(self.create_menu())
        
        self.process = None
        self.muted = False

    def log(self, msg):
        """Append a message to the shared log file for the TUI to read."""
        try:
            self.log_file.write(msg + "\n")
            self.log_file.flush()
        except Exception:
            pass

    def create_menu(self):
        """Build the GTK menu for the tray icon."""
        menu = Gtk.Menu()
        
        item_open = Gtk.MenuItem(label="Open Console (TUI)")
        item_open.connect("activate", lambda w: self.on_open_tui())
        menu.append(item_open)
        
        menu.append(Gtk.SeparatorMenuItem())
        
        item_restart = Gtk.MenuItem(label="Restart App")
        item_restart.connect("activate", lambda w: self.on_restart_app())
        menu.append(item_restart)
        
        item_stop = Gtk.MenuItem(label="Stop Audio")
        item_stop.connect("activate", lambda w: self.on_stop())
        menu.append(item_stop)
        
        self.item_mute = Gtk.MenuItem(label="Mute Mic")
        self.item_mute.connect("activate", self.on_mute_toggle)
        menu.append(self.item_mute)
        
        menu.append(Gtk.SeparatorMenuItem())
        item_quit = Gtk.MenuItem(label="Quit")
        item_quit.connect("activate", lambda w: self.on_quit())
        menu.append(item_quit)
        
        menu.show_all()
        return menu

    def _get_source_name(self):
        """Retrieve the deterministic source name for pactl commands."""
        import hashlib
        serial_str = os.environ.get("ANDROID_SERIAL", "")
        hash_str = hashlib.sha256(serial_str.encode()).hexdigest()[:7]
        return os.environ.get("AUDIOSOURCE_NAME", f"android-{hash_str}")

    def on_restart_app(self):
        """
        Hard restart the entire daemon.
        
        Uses os.execv to replace the current process. This ensures a completely
        clean state and is useful if the GTK event loop or PulseAudio gets stuck.
        """
        self.log("Restarting Tray...")
        self._stop_process()
        if os.path.exists(PID_FILE):
            try:
                os.remove(PID_FILE)
            except OSError:
                pass
        self.log_file.close()
        os.execv(sys.executable, ['python3', os.path.abspath(__file__)])

    def on_start_audio(self):
        """Spawn the audiosource.py child process to begin audio forwarding."""
        self._stop_process()
        self.log("Starting audiosource in background...")
        # '-u' prevents Python from buffering stdout so logs appear in TUI instantly
        cmd = ["python3", "-u", os.path.join(self.script_dir, "audiosource.py"), "run", "-r"]
        self.process = subprocess.Popen(cmd, stdout=self.log_file, stderr=subprocess.STDOUT)

    def on_stop(self):
        """Halt the audio stream gracefully."""
        self._stop_process()
        self.log("Stopped audiosource.")

    def _stop_process(self):
        """
        Terminate the audiosource child process and clean up PulseAudio modules.
        
        Ensures we don't leave orphaned module-pipe-source instances in PulseAudio
        which would otherwise block future connections with 'Invalid Argument'.
        """
        if self.process:
            self.log("Stopping audiosource process...")
            self.process.terminate()
            self.process.wait()
            self.process = None
            
        source_name = self._get_source_name()
        try:
            res = subprocess.run(["pactl", "list", "modules", "short"], capture_output=True, text=True)
            for line in res.stdout.splitlines():
                if "module-pipe-source" in line and f"source_name={source_name}" in line:
                    module_id = line.split()[0]
                    subprocess.run(["pactl", "unload-module", module_id], check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        except Exception:
            pass

    def on_open_tui(self):
        """
        Attempt to spawn a new terminal window running the TUI.
        
        Since we don't know the user's desktop environment, we iterate through
        common terminal emulators and use the first one that successfully launches.
        """
        tui_path = os.path.join(self.script_dir, "tui.py")
        terminals = [
            ["x-terminal-emulator", "-e"],
            ["gnome-terminal", "--"],
            ["konsole", "-e"],
            ["xfce4-terminal", "-x"],
            ["alacritty", "-e"]
        ]
        
        for term in terminals:
            try:
                cmd = term + [tui_path]
                subprocess.Popen(cmd, start_new_session=True)
                return
            except FileNotFoundError:
                continue
        self.log("No se pudo encontrar un emulador de terminal compatible para abrir TUI.")

    def on_mute_toggle(self, widget):
        """
        Toggle the microphone volume between 0% and 100%.
        
        We use set-source-volume rather than set-source-mute because PulseAudio's
        mute implementation can be buggy with pipe-sources and may not actually
        silence the audio for all clients (e.g. parec visualizers).
        """
        self.muted = not self.muted
        widget.set_label("Unmute Mic" if self.muted else "Mute Mic")
        source_name = self._get_source_name()
        vol = "0%" if self.muted else "100%"
        try:
            subprocess.run(["pactl", "set-source-volume", source_name, vol])
            self.log(f"Mic volume set to: {vol}")
        except Exception as e:
            self.log(f"Failed to mute/unmute: {e}")

    def on_quit(self):
        """Clean up the PID file and terminate the GTK application."""
        self.log("Quitting tray...")
        self._stop_process()
        if os.path.exists(PID_FILE):
            try:
                os.remove(PID_FILE)
            except OSError:
                pass
        self.log_file.close()
        Gtk.main_quit()

def main():
    app = AudioSourceTray()
    
    # Use GLib to handle signals instead of Python's signal module
    # because the GTK main loop blocks standard signal delivery.
    GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGUSR1, lambda: app.on_start_audio() or True)
    GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGUSR2, lambda: app.on_stop() or True)
    GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGTERM, lambda: app.on_quit() or False)
    
    # Ignore SIGHUP so closing terminal doesn't kill the tray
    if hasattr(signal, 'SIGHUP'):
        signal.signal(signal.SIGHUP, signal.SIG_IGN)
    
    app.log("Tray icon running in background (Audio stopped).")
    
    Gtk.main()

if __name__ == "__main__":
    main()
