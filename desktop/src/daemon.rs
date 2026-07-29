use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const AUDIOSOURCE_PKG: &str = "fr.dzx.audiosource";
const BUF_SIZE: usize = 8192;

pub fn get_audiosource_name(serial: Option<&str>) -> String {
    if let Ok(name) = env::var("AUDIOSOURCE_NAME") {
        return name;
    }
    
    let serial_str = serial.unwrap_or("");
    let mut hasher = Sha256::new();
    hasher.update(serial_str.as_bytes());
    let hash = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>();
    format!("android-{}", &hash[..7])
}

pub fn get_adb_env(serial: Option<&str>) -> Vec<(String, String)> {
    let mut envs = vec![];
    if let Some(s) = serial {
        envs.push(("ANDROID_SERIAL".to_string(), s.to_string()));
    }
    envs
}

fn command_with_env(cmd: &str, args: &[&str], envs: &[(String, String)]) -> Command {
    let mut command = Command::new(cmd);
    command.args(args);
    for (k, v) in envs {
        command.env(k, v);
    }
    command
}

pub fn unload_module(name: &str) {
    for _ in 0..3 {
        let output = match Command::new("pactl").args(["list", "modules", "short"]).output() {
            Ok(o) => o,
            Err(_) => return,
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut modules_to_unload = vec![];
        
        for line in stdout.lines() {
            if (line.contains("module-virtual-source") && line.contains(&format!("source_name={}", name)))
                || (line.contains("module-null-sink") && line.contains(&format!("sink_name={}_sink", name)))
                || (line.contains("module-pipe-source") && line.contains(&format!("source_name={}", name))) {
                if let Some(mod_id) = line.split_whitespace().next() {
                    modules_to_unload.push(mod_id.to_string());
                }
            }
        }
        
        if modules_to_unload.is_empty() {
            return;
        }
        
        for mod_id in modules_to_unload {
            let _ = Command::new("pactl").args(["unload-module", &mod_id]).output();
        }
        thread::sleep(Duration::from_millis(500));
    }
}

pub fn wait_for_device(envs: &[(String, String)]) -> bool {
    println!("[+] Waiting for device");
    let start_time = Instant::now();
    while start_time.elapsed().as_secs() < 30 {
        let elapsed = start_time.elapsed().as_secs();
        println!("[ADB] Waiting... {}s / 30s", elapsed);
        
        if let Ok(output) = command_with_env("adb", &["get-state"], envs).output() {
            if String::from_utf8_lossy(&output.stdout).contains("device") {
                let serial = envs.iter().find(|(k, _)| k == "ANDROID_SERIAL").map(|(_, v)| v.as_str()).unwrap_or("unknown");
                println!("[ADB] Device connected: {}", serial);
                return true;
            }
        }
        thread::sleep(Duration::from_secs(1));
    }
    eprintln!("[ADB] Timeout reached, no device found after 30s");
    false
}

pub fn check_permissions(envs: &[(String, String)]) -> bool {
    println!("[+] Checking permissions");
    
    let output = match command_with_env("adb", &["exec-out", "dumpsys", "package", AUDIOSOURCE_PKG], envs).output() {
        Ok(o) => o,
        Err(_) => {
            eprintln!("Error: Failed to get package dumpsys via adb.");
            return false;
        }
    };
    
    let dumpsys = String::from_utf8_lossy(&output.stdout);
    let mut missing = 0;
    
    for perm in ["android.permission.POST_NOTIFICATIONS", "android.permission.RECORD_AUDIO"] {
        let mut granted = dumpsys.contains(&format!("{}: granted=true", perm));
        
        if !granted {
            let _ = command_with_env("adb", &["exec-out", "pm", "grant", AUDIOSOURCE_PKG, perm], envs).output();
            granted = true; // Attempt auto-grant
        }
        
        if perm == "android.permission.RECORD_AUDIO" {
            let _ = command_with_env("adb", &["shell", "appops", "set", AUDIOSOURCE_PKG, "RECORD_AUDIO", "allow"], envs).output();
        }
        
        if granted {
            println!("{}: granted=true", perm);
        } else {
            eprintln!("{}: granted=false", perm);
            missing += 1;
        }
    }
    
    if missing > 0 {
        eprintln!("Error: Could not grant permissions. Please grant manually.");
        return false;
    }
    true
}

pub fn start_forwarding(name: &str, envs: &[(String, String)]) -> Result<()> {
    println!("[+] Starting Audio Source");
    
    let _ = command_with_env("adb", &["shell", "am", "force-stop", AUDIOSOURCE_PKG], envs).output();
    command_with_env("adb", &["shell", "am", "start", &format!("{}/.MainActivity", AUDIOSOURCE_PKG)], envs).output().context("Failed to start MainActivity")?;
    
    thread::sleep(Duration::from_secs(2));
    
    command_with_env("adb", &["forward", &format!("localabstract:{}", name), "localabstract:audiosource"], envs).output().context("Failed to forward adb port")?;
    
    println!("[+] Forwarding audio to {}", name);
    println!("[!] ACTION REQUIRED: Tap the microphone icon on your Android device to start recording.");
    
    thread::sleep(Duration::from_secs(1));
    Ok(())
}

pub fn socat(sock_name: &str, name: &str) -> Result<()> {
    let max_retries = 5;
    let mut backoff = 0.5;
    
    for attempt in 0..max_retries {
        println!("[SOCAT] Attempt {}/{} connecting to {}", attempt + 1, max_retries, sock_name);
        
        let abstract_sock_name = format!("\0{}", sock_name);
        let mut sock = match UnixStream::connect(&abstract_sock_name) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error connecting to socket (attempt {}/{}): {}", attempt + 1, max_retries, e);
                if attempt < max_retries - 1 {
                    thread::sleep(Duration::from_secs_f64(backoff));
                    backoff *= 1.5;
                    continue;
                }
                return Ok(());
            }
        };
        let _ = sock.set_read_timeout(Some(Duration::from_secs(3)));
        
        let mut pacat_proc = Command::new("pacat")
            .args([
                "--playback",
                "--device", &format!("{}_sink", name),
                "--format=s16le",
                "--channels=1",
                "--rate=44100",
                "--latency-msec=300",
            ])
            .stdin(Stdio::piped())
            .spawn()
            .context("Error starting pacat")?;
            
        let mut pacat_stdin = pacat_proc.stdin.take().expect("Failed to open pacat stdin");
        
        let mut buf = vec![0u8; BUF_SIZE];
        let start_time = Instant::now();
        let mut last_check = Instant::now();
        let mut retry_connection = false;
        let mut first_chunk = true;
        let mut silence_warnings = 0;
        
        loop {
            if last_check.elapsed().as_secs() > 5 {
                if let Ok(output) = Command::new("pactl").args(["list", "modules", "short"]).output() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    if !stdout.contains(&format!("sink_name={}_sink", name)) {
                        println!("[PIPEWIRE] Sink disappeared, triggering restart");
                        break;
                    }
                }
                last_check = Instant::now();
            }
            
            let n = match sock.read(&mut buf) {
                Ok(0) => {
                    if start_time.elapsed().as_secs_f32() < 1.0 {
                        println!("[SOCAT] Immediate disconnect, Android socket not ready, retrying...");
                        retry_connection = true;
                    }
                    break;
                }
                Ok(n) => n,
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                    continue;
                }
                Err(e) => {
                    eprintln!("Socket error: {}", e);
                    break;
                }
            };
            
            if first_chunk {
                println!("[SOCAT] Streaming started (Wi-Fi Optimized)");
                first_chunk = false;
            }
            
            if n == buf.len() && buf.len() < 16384 {
                buf.resize(buf.len() * 2, 0);
            }
            
            let is_silence = buf[..n].iter().all(|&x| x == 0);
            if is_silence {
                silence_warnings += 1;
                if silence_warnings % 100 == 1 {
                    println!("WARNING: Receiving pure silence (0s) from Android!");
                }
            } else {
                if silence_warnings > 0 {
                    println!("Audio signal detected!");
                    silence_warnings = 0;
                }
            }
            
            if let Err(e) = pacat_stdin.write_all(&buf[..n]) {
                eprintln!("Write error to pacat: {}", e);
                break;
            }
            let _ = pacat_stdin.flush();
        }
        
        let _ = pacat_proc.kill();
        let _ = pacat_proc.wait();
        
        if retry_connection && attempt < max_retries - 1 {
            thread::sleep(Duration::from_secs_f64(backoff));
            backoff *= 1.5;
            continue;
        }
        return Ok(());
    }
    
    Ok(())
}

pub fn run_bridge(serial: Option<String>, auto_restart: bool) -> Result<()> {
    let name = get_audiosource_name(serial.as_deref());
    let envs = get_adb_env(serial.as_deref());
    
    for cmd in ["adb", "pactl"] {
        if Command::new("which").arg(cmd).output().is_err() {
            anyhow::bail!("Error: {} not found", cmd);
        }
    }
    
    loop {
        unload_module(&name);
        
        println!("[+] Loading PulseAudio modules (Wi-Fi Stable Mode)");
        Command::new("pactl").args([
            "load-module", "module-null-sink",
            &format!("sink_name={}_sink", name),
            "sink_properties=device.description=AudioSource_Buffer"
        ]).output().context("Failed to load null sink")?;
        
        Command::new("pactl").args([
            "load-module", "module-virtual-source",
            &format!("source_name={}", name),
            &format!("master={}_sink.monitor", name),
            "source_properties=device.description=AudioSource_Microphone device.class=sound device.icon_name=audio-input-microphone"
        ]).output().context("Failed to load virtual source")?;
        
        if !wait_for_device(&envs) {
            break;
        }
        
        if !check_permissions(&envs) {
            break;
        }
        
        if let Err(e) = start_forwarding(&name, &envs) {
            eprintln!("Failed to start forwarding: {}", e);
        } else {
            if let Err(e) = socat(&name, &name) {
                eprintln!("Socat error: {}", e);
            }
        }
        
        if !auto_restart {
            break;
        }
        println!("Restarting in 1 second...");
        thread::sleep(Duration::from_secs(1));
    }
    
    unload_module(&name);
    Ok(())
}

pub fn set_volume(serial: Option<String>, level: &str) -> Result<()> {
    let name = get_audiosource_name(serial.as_deref());
    Command::new("pactl").args(["set-source-volume", &name, level]).output().context("Failed to set volume")?;
    Ok(())
}
