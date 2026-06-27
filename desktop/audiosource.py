#!/usr/bin/env python3
"""
AudioSource core client logic.

Handles the connection between the Android device and PulseAudio.
Establishes ADB forwarding and pipes the raw PCM audio stream directly
into a virtual PulseAudio microphone.
"""

import argparse
import fcntl
import hashlib
import os
import signal
import socket
import subprocess
import sys
import time

AUDIOSOURCE_PKG = 'fr.dzx.audiosource'
PIPE_SIZE = 4096
BUF_SIZE = 1024

def get_audiosource_name(serial):
    """
    Generate a deterministic PulseAudio source name based on the device serial.
    
    Uses SHA-256 to ensure the generated name is safe for PulseAudio
    even if the serial contains special characters.
    """
    serial_str = serial if serial else ""
    hash_str = hashlib.sha256(serial_str.encode()).hexdigest()[:7]
    return os.environ.get("AUDIOSOURCE_NAME", f"android-{hash_str}")

def unload_module(name):
    """
    Safely remove the virtual PulseAudio microphone.
    
    Assumes PulseAudio might not be running or the module might already
    be unloaded. Fails silently to prevent crash loops during cleanup.
    """
    import time
    for _ in range(3):
        try:
            res = subprocess.run(["pactl", "list", "modules", "short"], capture_output=True, text=True, check=True)
            module_id = None
            for line in res.stdout.splitlines():
                if "module-pipe-source" in line and f"source_name={name}" in line:
                    module_id = line.split()[0]
                    break
            if module_id:
                subprocess.run(["pactl", "unload-module", module_id], check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                time.sleep(0.5)
            else:
                return
        except subprocess.CalledProcessError:
            return
        except Exception as e:
            print(f"Warning: Failed to unload module: {e}")
            break

def get_adb_env(serial):
    """Inject the specific Android serial into the environment for adb commands."""
    env = os.environ.copy()
    if serial:
        env["ANDROID_SERIAL"] = serial
    return env

def wait_for_device(env):
    """Block execution until the target Android device is connected via ADB."""
    print("[+] Waiting for device")
    start_time = time.time()
    while time.time() - start_time < 30:
        elapsed = time.time() - start_time
        print(f"[ADB] Waiting... {elapsed:.0f}s / 30s")
        try:
            res = subprocess.run(["adb", "get-state"], env=env, capture_output=True, text=True)
            if "device" in res.stdout:
                serial = env.get("ANDROID_SERIAL", "unknown")
                print(f"[ADB] Device connected: {serial}")
                return True
        except Exception:
            pass
        time.sleep(1)
    print("[ADB] Timeout reached, no device found after 30s")
    print("Timeout: no ADB device found")
    return False

def check_permissions(env):
    """
    Verify and attempt to grant necessary Android app permissions via ADB.
    
    Audio capture requires RECORD_AUDIO. We attempt to auto-grant it
    to improve user experience, but gracefully fallback to manual instructions.
    """
    print("[+] Checking permissions")
    try:
        dumpsys = subprocess.run(
            ["adb", "exec-out", "dumpsys", "package", AUDIOSOURCE_PKG],
            env=env, capture_output=True, text=True, check=True
        ).stdout
    except subprocess.CalledProcessError:
        print("Error: Failed to get package dumpsys via adb.")
        return False

    missing = 0
    for perm in ["android.permission.POST_NOTIFICATIONS", "android.permission.RECORD_AUDIO"]:
        granted = False
        if f"{perm}: granted=true" in dumpsys:
            granted = True
        else:
            try:
                subprocess.run(
                    ["adb", "exec-out", "pm", "grant", AUDIOSOURCE_PKG, perm],
                    env=env, check=True, capture_output=True
                )
                granted = True
            except subprocess.CalledProcessError:
                pass
        
        # Ensure AppOps is also granted (Fixes "App op 27 missing, silencing record" on MIUI/Android 11+)
        if perm == "android.permission.RECORD_AUDIO":
            try:
                subprocess.run(
                    ["adb", "shell", "appops", "set", AUDIOSOURCE_PKG, "RECORD_AUDIO", "allow"],
                    env=env, check=False, capture_output=True
                )
            except Exception:
                pass
        
        if granted:
            print(f"{perm}: granted=true")
        else:
            print(f"{perm}: granted=false")
            missing += 1

    if missing > 0:
        print("Error: Could not grant permissions. Please grant them manually by following:")
        print("https://guidebooks.google.com/android/changesettingspermissions/changeyourapppermissions")
        return False
    return True

def start_forwarding(name, env):
    """Start the Android app and establish the ADB port forwarding for the audio socket."""
    print("[+] Starting Audio Source")
    try:
        subprocess.run(["adb", "shell", "am", "force-stop", AUDIOSOURCE_PKG], env=env, check=False)
        subprocess.run(["adb", "shell", "am", "start", f"{AUDIOSOURCE_PKG}/.MainActivity"], env=env, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        # Wait for the app to initialize its LocalServerSocket to prevent premature connection leading to n=0 (Broken Pipe)
        time.sleep(2)
    except subprocess.CalledProcessError:
        print("Error: Failed to start MainActivity via adb.")
        raise
    
    try:
        subprocess.run(["adb", "forward", f"localabstract:{name}", "localabstract:audiosource"], env=env, check=True, capture_output=True)
        print(f"[+] Forwarding audio to {name}")
        print("[!] ACTION REQUIRED: Tap the microphone icon on your Android device to start recording.")
    except subprocess.CalledProcessError:
        print("Error: Failed to forward adb port.")
        raise
    time.sleep(1)

def socat(sock_name, pipe_name):
    """
    Bridge the Android Unix socket to the local PulseAudio FIFO pipe.
    
    Reads raw audio chunks from the ADB-forwarded socket and writes them
    non-blockingly to the pipe. This handles the actual audio streaming.
    """
    if not hasattr(fcntl, 'F_SETPIPE_SZ'):
        fcntl.F_SETPIPE_SZ = 1031

    max_retries = 5
    backoff = 0.5
    
    for attempt in range(max_retries):
        print(f"[SOCAT] Attempt {attempt+1}/{max_retries} connecting to {sock_name}")
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout(3.0)
        try:
            sock.connect('\0' + sock_name)
        except (ConnectionRefusedError, FileNotFoundError, OSError) as e:
            print(f"Error connecting to socket (attempt {attempt+1}/{max_retries}): {e}")
            sock.close()
            if attempt < max_retries - 1:
                time.sleep(backoff)
                backoff *= 1.5
                continue
            return
        except Exception as e:
            print(f"Error connecting to socket: {e}")
            sock.close()
            return

        try:
            fd = os.open(pipe_name, os.O_WRONLY | os.O_NONBLOCK)
        except OSError as e:
            print(f"Error opening pipe: {e}")
            sock.close()
            return

        buf = bytearray(BUF_SIZE)
        try:
            # Optimize buffer size for audio streaming throughput
            fcntl.fcntl(fd, fcntl.F_SETPIPE_SZ, PIPE_SIZE)
        except Exception:
            pass

        start_time = time.time()
        last_check = time.time()
        retry_connection = False
        first_chunk = True

        try:
            while True:
                current_time = time.time()
                if current_time - last_check > 5:
                    try:
                        res = subprocess.run(["pactl", "list", "modules", "short"], capture_output=True, text=True)
                        if f"source_name={sock_name}" not in res.stdout:
                            print("[PIPEWIRE] module-pipe-source disappeared, triggering restart")
                            print("PipeWire module lost, restarting...")
                            break
                    except Exception:
                        pass
                    last_check = current_time

                try:
                    n = sock.recv_into(buf, len(buf))
                except socket.timeout:
                    continue
                except Exception as e:
                    print(f"Socket error: {e}")
                    break

                if n == 0:
                    if time.time() - start_time < 1.0:
                        print("[SOCAT] Immediate disconnect, Android socket not ready, retrying...")
                        print(f"Connection closed immediately by Android app (attempt {attempt+1}/{max_retries}). Retrying...")
                        retry_connection = True
                    break

                if n > 0 and first_chunk:
                    print("[SOCAT] Streaming started")
                    first_chunk = False
                    
                if n == len(buf) and len(buf) < 65536:
                    buf = bytearray(len(buf) * 2)
                    print(f"[SOCAT] Adaptive buffer increased to {len(buf)}")

                # Check for pure silence (all zeroes)
                if not any(buf[:n]):
                    if not hasattr(socat, 'silence_warnings'):
                        socat.silence_warnings = 0
                    socat.silence_warnings += 1
                    if socat.silence_warnings % 100 == 1:
                        print("WARNING: Receiving pure silence (0s) from Android! Microphone might be blocked by OS privacy settings.")
                else:
                    if hasattr(socat, 'silence_warnings') and socat.silence_warnings > 0:
                        print("Audio signal detected!")
                        socat.silence_warnings = 0

                try:
                    os.write(fd, buf[:n])
                except BlockingIOError:
                    pass # Drop chunks if PulseAudio isn't reading fast enough to prevent desync
                except Exception as e:
                    print(f"Write error: {e}")
                    break
        finally:
            try:
                os.write(fd, bytes(BUF_SIZE))
            except Exception:
                pass
            os.close(fd)
        sock.close()
        
        if retry_connection and attempt < max_retries - 1:
            time.sleep(backoff)
            backoff *= 1.5
            continue
            
        return

def run_command(args):
    """
    Main loop for starting the audio bridge.
    
    Handles the entire lifecycle: PulseAudio module loading, ADB connection,
    app startup, and streaming. Respects the auto-restart (-r) flag to recover
    from disconnections automatically.
    """
    serial = args.serial
    name = get_audiosource_name(serial)
    env = get_adb_env(serial)
    pipe_name = f"/tmp/{name}"

    for cmd in ["adb", "pactl"]:
        if subprocess.run(["command", "-v", cmd], shell=True, capture_output=True).returncode != 0:
            print(f"Error: {cmd} not found")
            sys.exit(1)

    def cleanup(signum=None, frame=None):
        unload_module(name)
        time.sleep(0.5)
        if os.path.exists(pipe_name):
            try:
                os.remove(pipe_name)
            except OSError:
                pass
        if signum:
            sys.exit(130)

    signal.signal(signal.SIGINT, cleanup)
    signal.signal(signal.SIGTERM, cleanup)

    while True:
        try:
            unload_module(name)
            
            # Clean up stale pipes
            if os.path.exists(pipe_name):
                try:
                    os.remove(pipe_name)
                except OSError:
                    pass
            
            print("[+] Loading PulseAudio module")
            cmd = [
                "pactl", "load-module", "module-pipe-source",
                f"source_name={name}", 
                "source_properties=device.description=AudioSource_Microphone device.class=sound device.icon_name=audio-input-microphone",
                "channels=1", "format=s16le", "rate=44100", f"file={pipe_name}"
            ]
            subprocess.run(cmd, check=True)

            if not wait_for_device(env):
                break

            if not check_permissions(env):
                cleanup()
                sys.exit(1)

            start_forwarding(name, env)

            socat(name, pipe_name)
        except KeyboardInterrupt:
            cleanup()
            sys.exit(130)
        except subprocess.CalledProcessError:
            pass # Suppressed since stderr is already printed by subprocess
        except Exception as e:
            print(f"Run error: {e}")
        
        if not args.r:
            break
        print("Restarting in 1 second...")
        time.sleep(1)

    cleanup()

def volume_command(args):
    """Set the system volume for the generated virtual microphone."""
    serial = args.serial
    name = get_audiosource_name(serial)
    try:
        subprocess.run(["pactl", "set-source-volume", name, args.level], check=True)
    except subprocess.CalledProcessError as e:
        print(f"Error setting volume: {e}")
        sys.exit(1)

def main():
    parser = argparse.ArgumentParser(description="Audio Source Alternative Python Client")
    parser.add_argument("-s", "--serial", help="Use device with given serial (overrides $ANDROID_SERIAL)")
    
    subparsers = parser.add_subparsers(dest="command", required=True)
    
    run_parser = subparsers.add_parser("run", help="Run Audio Source and start forwarding")
    run_parser.add_argument("-r", action="store_true", help="Automatically restart")
    
    vol_parser = subparsers.add_parser("volume", help="Set volume")
    vol_parser.add_argument("level", help="Volume level (e.g. 250%%)")

    args = parser.parse_args()

    if not args.serial and "ANDROID_SERIAL" in os.environ:
        args.serial = os.environ["ANDROID_SERIAL"]
        
    if not args.serial:
        # Fetch the default device serial safely even if multiple devices exist
        try:
            res = subprocess.run(["adb", "devices"], capture_output=True, text=True, check=True)
            for line in res.stdout.splitlines()[1:]:
                if "device" in line and "offline" not in line and "unauthorized" not in line:
                    args.serial = line.split()[0]
                    break
        except Exception:
            pass

    if args.command == "run":
        run_command(args)
    elif args.command == "volume":
        volume_command(args)

if __name__ == "__main__":
    main()
