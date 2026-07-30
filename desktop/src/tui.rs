use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use std::io::{stdout, BufRead, BufReader, ErrorKind, Read};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(PartialEq, Clone, Copy)]
enum AppMode {
    Idle,
    Web,
    AdbUsb,
    AdbWifi,
}

// Padded with spaces to 56 characters so Alignment::Center won't break the art
const ASCII_LOGO: &str = r#"                   ___                                  
  ____ ___  ______/ (_)___  _________  __  _______________
 / __ `/ / / / __  / / __ \/ ___/ __ \/ / / / ___/ ___/ _ \
/ /_/ / /_/ / /_/ / / /_/ (__  ) /_/ / /_/ / /  / /__/  __/
\__,_/\__,_/\__,_/_/\____/____/\____/\__,_/_/   \___/\___/"#;

fn check_and_start_tray(logs: Arc<Mutex<Vec<String>>>) {
    let pid_file = crate::utils::get_log_file().parent().unwrap().join("tray.pid");
    let mut is_running = false;
    
    if let Ok(pid_str) = std::fs::read_to_string(&pid_file) {
        if let Ok(pid) = pid_str.parse::<i32>() {
            if unsafe { libc::kill(pid, 0) } == 0 {
                is_running = true;
            }
        }
    }

    if !is_running {
        logs.lock().unwrap().push("Starting Tray Icon in background...".to_string());
        if let Ok(exe) = std::env::current_exe() {
            let _ = Command::new(exe).arg("tray")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }
    }
}

fn tail_logs(logs: Arc<Mutex<Vec<String>>>, running: Arc<Mutex<bool>>) {
    let log_file_path = crate::utils::get_log_file();
    
    while *running.lock().unwrap() && !log_file_path.exists() {
        thread::sleep(Duration::from_millis(500));
    }

    if !*running.lock().unwrap() {
        return;
    }

    let mut file = std::fs::File::open(&log_file_path).unwrap();
    use std::io::Seek;
    let _ = file.seek(std::io::SeekFrom::End(0));

    let mut reader = BufReader::new(file);
    let mut line = String::new();

    while *running.lock().unwrap() {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {
                thread::sleep(Duration::from_millis(100));
            }
            Ok(_) => {
                let mut l = logs.lock().unwrap();
                l.push(line.trim_end().to_string());
            }
            Err(e) => {
                if e.kind() != ErrorKind::Interrupted {
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }
}

fn poll_audio_volume(volume: Arc<Mutex<f64>>, running: Arc<Mutex<bool>>) {
    let mut silence_start = std::time::Instant::now();
    let mut is_silent = true;

    while *running.lock().unwrap() {
        let source_name = "audiosource_web";

        let output = Command::new("pactl").args(["list", "short", "sources"]).output();
        if let Ok(out) = output {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if !stdout.contains(source_name) {
                *volume.lock().unwrap() = 0.0;
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        }

        let mut child = match Command::new("parec")
            .args(["-d", source_name, "--format=s16le", "--channels=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => {
                *volume.lock().unwrap() = 0.0;
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };

        let mut stdout = child.stdout.take().unwrap();
        let mut buffer = [0u8; 4096];

        while *running.lock().unwrap() {
            match stdout.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    let mut peak = 0;
                    for chunk in buffer[..n].chunks_exact(2) {
                        let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                        let abs_sample = sample.abs();
                        if abs_sample > peak {
                            peak = abs_sample;
                        }
                    }
                    let mut vol = peak as f64 / 20000.0;
                    if vol > 1.0 { vol = 1.0; }
                    
                    if vol < 0.01 {
                        if !is_silent && silence_start.elapsed().as_secs() > 3 {
                            is_silent = true;
                            crate::utils::log_msg("[TUI] Silence detected");
                        }
                        *volume.lock().unwrap() = 0.0;
                    } else {
                        if is_silent {
                            is_silent = false;
                            crate::utils::log_msg("[TUI] Audio signal restored");
                        }
                        silence_start = std::time::Instant::now();
                        *volume.lock().unwrap() = vol;
                    }
                }
                Err(_) => break,
            }
        }
        
        *volume.lock().unwrap() = 0.0;
        let _ = child.kill();
        let _ = child.wait();
        thread::sleep(Duration::from_secs(1));
    }
}

pub fn run_tui() -> Result<()> {
    crate::utils::log_msg("TUI Connected to Daemon.");
    
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;

    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let running_flag = Arc::new(Mutex::new(true));
    let mode = Arc::new(Mutex::new(AppMode::Idle));
    let is_streaming = Arc::new(Mutex::new(false));
    let qr_lines = Arc::new(Mutex::new(Vec::new()));
    let logs = Arc::new(Mutex::new(vec!["App running in background...".to_string()]));
    let volume = Arc::new(Mutex::new(0.0));

    check_and_start_tray(logs.clone());

    let logs_clone = logs.clone();
    let running_clone = running_flag.clone();
    thread::spawn(move || {
        tail_logs(logs_clone, running_clone);
    });

    let vol_clone = volume.clone();
    let running_clone2 = running_flag.clone();
    thread::spawn(move || {
        poll_audio_volume(vol_clone, running_clone2);
    });

    let mut web_server_started = false;

    while *running_flag.lock().unwrap() {
        terminal.draw(|f| {
            let current_mode = *mode.lock().unwrap();
            let streaming = *is_streaming.lock().unwrap();
            let current_volume = *volume.lock().unwrap();
            let current_logs = logs.lock().unwrap().clone();
            let current_qr = qr_lines.lock().unwrap().clone();

            let outer_block = Block::default()
                .borders(Borders::ALL)
                .title(
                    Line::from(match current_mode {
                        AppMode::Idle => " AudioSource TUI ",
                        AppMode::Web => " AudioSource TUI (Web Mode) ",
                        AppMode::AdbUsb => " AudioSource TUI (ADB: USB) ",
                        AppMode::AdbWifi => " AudioSource TUI (ADB: Wi-Fi) ",
                    })
                    .alignment(Alignment::Center)
                )
                .title(
                    Line::from(if streaming {
                        Span::styled(" Status: Streaming ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
                    } else if current_mode != AppMode::Idle {
                        Span::styled(" Status: Waiting ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                    } else {
                        Span::styled(" Status: Idle ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                    })
                    .alignment(Alignment::Left)
                )
                .title(
                    Line::from(Span::styled(" v0.1.0 ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)))
                        .alignment(Alignment::Right)
                );

            let inner_area = outer_block.inner(f.area());
            f.render_widget(outer_block, f.area());

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(6), 
                    Constraint::Length(8), 
                    Constraint::Length(3), 
                    Constraint::Length(1), 
                ])
                .split(inner_area);

            // 1. Logo or QR
            let top_content = if current_mode == AppMode::Web && streaming && !current_qr.is_empty() {
                let lines: Vec<Line> = current_qr.iter().map(|s: &String| Line::from(s.as_str())).collect();
                Paragraph::new(lines).alignment(Alignment::Center)
            } else {
                let lines: Vec<Line> = ASCII_LOGO.lines().map(|l| Line::from(Span::styled(l, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)))).collect();
                Paragraph::new(lines).alignment(Alignment::Center)
            };
            f.render_widget(top_content, chunks[0]);

            // 2. Logs
            let log_block = Block::default()
                .borders(Borders::TOP)
                .title(Span::styled("── Log ──", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)));
            
            let log_inner = log_block.inner(chunks[1]);
            f.render_widget(log_block, chunks[1]);
            
            let display_logs: Vec<Line> = current_logs.iter().rev().take(6).rev().map(|log| {
                let color = if log.contains("Error") || log.contains("Failed") || log.contains("exited") || log.contains("Stop") {
                    Color::Red
                } else if log.contains("Waiting") || log.contains("background") {
                    Color::Yellow
                } else if log.contains("Start") || log.contains("Connected") {
                    Color::Green
                } else {
                    Color::Reset
                };
                Line::from(vec![
                    Span::styled("│ ", Style::default().fg(Color::Reset)),
                    Span::styled(log.clone(), Style::default().fg(color)),
                ])
            }).collect();
            f.render_widget(Paragraph::new(display_logs), log_inner);

            // 3. Volume
            let vol_block = Block::default()
                .borders(Borders::TOP)
                .title(Span::styled("── Mic Volume ──", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
            
            let vol_inner = vol_block.inner(chunks[2]);
            f.render_widget(vol_block, chunks[2]);

            let bar_width = vol_inner.width.saturating_sub(10);
            if bar_width > 0 {
                let filled = (current_volume * bar_width as f64) as u16;
                let pct = (current_volume * 100.0) as u16;
                
                let mut bar = String::from("Vol: ║");
                for i in 0..bar_width {
                    if i < filled {
                        bar.push('█');
                    } else {
                        bar.push('░');
                    }
                }
                
                let color = if current_volume > 0.8 {
                    Color::Red
                } else if current_volume > 0.5 {
                    Color::Yellow
                } else {
                    Color::Green
                };

                let vol_line = Line::from(vec![
                    Span::styled(bar, Style::default().fg(color)),
                    Span::styled(format!(" {}%", pct), Style::default().fg(Color::DarkGray)),
                ]);
                f.render_widget(Paragraph::new(vol_line), vol_inner);
            }

            // 4. Controls
            let controls = Paragraph::new("[S] Start  [C] Stop  [M] Mute  [W] Web  [A] ADB  [Z/X] Vol  [T] Hide  [Q] Quit")
                .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                .alignment(Alignment::Center);
            f.render_widget(controls, chunks[3]);
        })?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                let current_mode = *mode.lock().unwrap();
                match key.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') => {
                        *running_flag.lock().unwrap() = false;
                        // Kill tray
                        let pid_file = crate::utils::get_log_file().parent().unwrap().join("tray.pid");
                        if let Ok(pid_str) = std::fs::read_to_string(&pid_file) {
                            if let Ok(pid) = pid_str.parse::<i32>() {
                                unsafe { libc::kill(pid, 15); }
                            }
                        }
                    }
                    KeyCode::Char('t') | KeyCode::Char('T') => {
                        *running_flag.lock().unwrap() = false; // Hide TUI only
                    }
                    KeyCode::Char('c') | KeyCode::Char('C') => {
                        *is_streaming.lock().unwrap() = false;
                        crate::utils::log_msg("Stopped streaming.");
                    }
                    KeyCode::Char('z') | KeyCode::Char('Z') => {
                        let _ = crate::daemon::set_volume(None, "-10%");
                        crate::utils::log_msg("Mic Volume: -10%");
                    }
                    KeyCode::Char('x') | KeyCode::Char('X') => {
                        let _ = crate::daemon::set_volume(None, "+10%");
                        crate::utils::log_msg("Mic Volume: +10%");
                    }
                    KeyCode::Char('w') | KeyCode::Char('W') => {
                        *mode.lock().unwrap() = AppMode::Web;
                        crate::utils::log_msg("Web Mode selected. Press [S] to start server.");
                        *is_streaming.lock().unwrap() = web_server_started; 
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') => {
                        if current_mode == AppMode::AdbUsb {
                            *mode.lock().unwrap() = AppMode::AdbWifi;
                            crate::utils::log_msg("ADB Wi-Fi Mode selected. Press [S] to connect.");
                        } else {
                            *mode.lock().unwrap() = AppMode::AdbUsb;
                            crate::utils::log_msg("ADB USB Mode selected. Press [S] to connect.");
                        }
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        *is_streaming.lock().unwrap() = true;
                        if current_mode == AppMode::Web {
                            if !web_server_started {
                                web_server_started = true;
                                crate::utils::log_msg("Starting Web Server...");
                                match crate::web::get_qr_string() {
                                    Ok((qr, url)) => {
                                        *qr_lines.lock().unwrap() = qr.lines().map(|s| s.to_string()).collect();
                                        crate::utils::log_msg(&format!("Web Server running at {}", url));
                                        
                                        std::thread::spawn(|| {
                                            let rt = tokio::runtime::Runtime::new().unwrap();
                                            let _ = rt.block_on(crate::web::run_web_server());
                                        });
                                    }
                                    Err(e) => {
                                        crate::utils::log_msg(&format!("Failed to generate QR: {}", e));
                                    }
                                }
                            }
                        } else if current_mode == AppMode::AdbUsb || current_mode == AppMode::AdbWifi {
                            crate::utils::log_msg("ADB connection starting... (Not implemented yet)");
                        }
                    }
                    KeyCode::Char('m') | KeyCode::Char('M') => {
                        let _ = crate::daemon::set_volume(None, "0%");
                        crate::utils::log_msg("Microphone muted.");
                    }
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    
    Ok(())
}
