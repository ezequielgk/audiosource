use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Clear},
    Terminal,
};
use std::io::{stdout, BufRead, BufReader, ErrorKind, Read};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::collections::HashMap;

#[derive(PartialEq, Clone, Copy)]
enum AppMode {
    Idle,
    Web,
    AdbUsb,
    AdbWifi,
}

#[derive(Clone)]
enum PopupState {
    None,
    QrCode(Vec<String>, String), // QR lines, URL
    AdbActionSelection, // "1. Connect, 2. Pair, 3. Pair via QR"
    AdbIpInput(String, bool), // input buffer, is_pairing
    AdbCodeInput(String, String), // ip buffer, code buffer
    AdbQrPairing(Vec<String>, String, String), // qr_lines, service_name, password
}

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
        crate::utils::notify("AudioSource", "Starting Background Daemon...");
        if let Ok(exe) = std::env::current_exe() {
            let _ = Command::new(exe).arg("tray")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }
    }
}

fn tail_logs(logs: Arc<Mutex<Vec<String>>>, status_msg: Arc<Mutex<String>>, running: Arc<Mutex<bool>>) {
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
                // Also update status if tray died
                let pid_file = crate::utils::get_log_file().parent().unwrap().join("tray.pid");
                let mut tray_alive = false;
                if let Ok(pid_str) = std::fs::read_to_string(&pid_file) {
                    if let Ok(pid) = pid_str.parse::<i32>() {
                        if unsafe { libc::kill(pid, 0) } == 0 {
                            tray_alive = true;
                        }
                    }
                }
                if !tray_alive {
                    *status_msg.lock().unwrap() = "DAEMON OFFLINE".to_string();
                }

                thread::sleep(Duration::from_millis(100));
            }
            Ok(_) => {
                let trimmed = line.trim_end().to_string();
                
                // Parse status
                let mut s = status_msg.lock().unwrap();
                if trimmed.contains("Waiting for device") {
                    *s = "Waiting for device...".to_string();
                } else if trimmed.contains("Forwarding audio") {
                    *s = "Streaming".to_string();
                } else if trimmed.contains("Restarting") {
                    *s = "Reconnecting...".to_string();
                } else if trimmed.contains("Stopped") {
                    *s = "Stopped".to_string();
                } else if trimmed.contains("Error") {
                    *s = "Error".to_string();
                }

                let mut l = logs.lock().unwrap();
                l.push(trimmed);
            }
            Err(e) => {
                if e.kind() != ErrorKind::Interrupted {
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }
}

fn poll_system_volume(sys_volume: Arc<Mutex<String>>, running: Arc<Mutex<bool>>) {
    while *running.lock().unwrap() {
        let source_name = "audiosource_web"; // In a real app we'd fetch this dynamically
        if let Ok(output) = Command::new("pactl").args(["get-source-volume", source_name]).output() {
            let out = String::from_utf8_lossy(&output.stdout);
            if let Some(idx) = out.find('%') {
                if let Some(start) = out[..idx].rfind(' ') {
                    let vol = format!("{}%", &out[start+1..=idx]);
                    if *sys_volume.lock().unwrap() != vol {
                        *sys_volume.lock().unwrap() = vol;
                    }
                }
            }
        }
        thread::sleep(Duration::from_secs(1));
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

// Helper for rendering popups centered in the terminal
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

pub fn run_tui() -> Result<()> {
    crate::utils::log_msg("TUI Connected to Daemon.");
    
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;

    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let running_flag = Arc::new(Mutex::new(true));
    let mode = Arc::new(Mutex::new(AppMode::Idle));
    let is_streaming = Arc::new(Mutex::new(false));
    let popup_state = Arc::new(Mutex::new(PopupState::None));
    let logs = Arc::new(Mutex::new(vec!["App running in background...".to_string()]));
    let volume = Arc::new(Mutex::new(0.0));
    let status_msg = Arc::new(Mutex::new("Idle".to_string()));
    let sys_volume = Arc::new(Mutex::new("100%".to_string()));
    let active_mdns: Arc<Mutex<Option<ServiceDaemon>>> = Arc::new(Mutex::new(None));

    check_and_start_tray(logs.clone());

    let logs_clone = logs.clone();
    let running_clone = running_flag.clone();
    let status_clone = status_msg.clone();
    thread::spawn(move || {
        tail_logs(logs_clone, status_clone, running_clone);
    });

    let vol_clone = volume.clone();
    let running_clone2 = running_flag.clone();
    thread::spawn(move || {
        poll_audio_volume(vol_clone, running_clone2);
    });
    
    let sysvol_clone = sys_volume.clone();
    let running_clone3 = running_flag.clone();
    thread::spawn(move || {
        poll_system_volume(sysvol_clone, running_clone3);
    });

    let mut web_server_started = false;

    while *running_flag.lock().unwrap() {
        terminal.draw(|f| {
            let current_mode = *mode.lock().unwrap();
            let _streaming = *is_streaming.lock().unwrap();
            let current_volume = *volume.lock().unwrap();
            let current_logs = logs.lock().unwrap().clone();
            let current_status = status_msg.lock().unwrap().clone();
            let current_sysvol = sys_volume.lock().unwrap().clone();
            let current_popup = popup_state.lock().unwrap().clone();

            let status_color = match current_status.as_str() {
                "DAEMON OFFLINE" | "Error" => Color::Red,
                "Stopped" => Color::Yellow,
                _ => Color::Green,
            };

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
                    Line::from(Span::styled(format!(" Status: {} ", current_status), Style::default().fg(status_color).add_modifier(Modifier::BOLD)))
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

            // 1. Logo
            let lines: Vec<Line> = ASCII_LOGO.lines().map(|l| Line::from(Span::styled(l, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)))).collect();
            f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), chunks[0]);

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
                    Span::styled("║", Style::default().fg(Color::Reset)),
                ]);
                f.render_widget(Paragraph::new(vol_line), vol_inner);
            }

            // 4. Controls
            let controls_str = format!("[{}] Vol   [S] Start  [C] Stop  [M] Mute  [W] Web  [A] ADB  [Z/X] Vol  [T] Hide  [Q] Quit", current_sysvol);
            let controls = Paragraph::new(controls_str)
                .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                .alignment(Alignment::Center);
            f.render_widget(controls, chunks[3]);

            // Draw Popups
            match current_popup {
                PopupState::QrCode(qr_lines, url) => {
                    let popup_area = centered_rect(60, 80, f.area());
                    let block = Block::default().borders(Borders::ALL).title(" Web Interface QR ").style(Style::default().fg(Color::Cyan));
                    let inner = block.inner(popup_area);
                    f.render_widget(Clear, popup_area);
                    f.render_widget(block, popup_area);
                    
                    let mut lines: Vec<Line> = qr_lines.iter().map(|s| Line::from(s.as_str())).collect();
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled("Open in your browser:", Style::default().fg(Color::Yellow))));
                    lines.push(Line::from(Span::styled(url, Style::default().fg(Color::White).add_modifier(Modifier::BOLD))));
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled("Press ESC or Q to close", Style::default().fg(Color::DarkGray))));
                    
                    let p = Paragraph::new(lines).alignment(Alignment::Center);
                    f.render_widget(p, inner);
                }
                PopupState::AdbActionSelection => {
                    let popup_area = centered_rect(50, 30, f.area());
                    let block = Block::default().borders(Borders::ALL).title(" Wireless Debugging ").style(Style::default().fg(Color::Cyan));
                    let inner = block.inner(popup_area);
                    f.render_widget(Clear, popup_area);
                    f.render_widget(block, popup_area);

                    let text = vec![
                        Line::from("1. Connect (Conectar)"),
                        Line::from("2. Pair (Vincular)"),
                        Line::from("3. Pair via QR (Vincular por QR)"),
                        Line::from(""),
                        Line::from(Span::styled("Press 1, 2 or 3 (or ESC to cancel)", Style::default().fg(Color::Yellow))),
                    ];
                    f.render_widget(Paragraph::new(text).alignment(Alignment::Center), inner);
                }
                PopupState::AdbIpInput(input, is_pairing) => {
                    let popup_area = centered_rect(50, 30, f.area());
                    let title = if is_pairing { " Pair " } else { " Connect " };
                    let block = Block::default().borders(Borders::ALL).title(title).style(Style::default().fg(Color::Cyan));
                    let inner = block.inner(popup_area);
                    f.render_widget(Clear, popup_area);
                    f.render_widget(block, popup_area);

                    let text = vec![
                        Line::from("Enter IP:PORT (e.g., 192.168.1.15:43000):"),
                        Line::from(""),
                        Line::from(format!("> {}", input)),
                        Line::from(""),
                        Line::from(Span::styled("Press ENTER to confirm (ESC to cancel)", Style::default().fg(Color::Yellow))),
                    ];
                    f.render_widget(Paragraph::new(text).alignment(Alignment::Center), inner);
                }
                PopupState::AdbCodeInput(ip, code) => {
                    let popup_area = centered_rect(50, 30, f.area());
                    let block = Block::default().borders(Borders::ALL).title(" Pair Code ").style(Style::default().fg(Color::Cyan));
                    let inner = block.inner(popup_area);
                    f.render_widget(Clear, popup_area);
                    f.render_widget(block, popup_area);

                    let text = vec![
                        Line::from(format!("Pairing with {}", ip)),
                        Line::from("Enter 6-digit Pairing Code:"),
                        Line::from(""),
                        Line::from(format!("> {}", code)),
                        Line::from(""),
                        Line::from(Span::styled("Press ENTER to confirm (ESC to cancel)", Style::default().fg(Color::Yellow))),
                    ];
                    f.render_widget(Paragraph::new(text).alignment(Alignment::Center), inner);
                }
                PopupState::AdbQrPairing(qr_lines, service_name, password) => {
                    let popup_area = centered_rect(60, 80, f.area());
                    let block = Block::default().borders(Borders::ALL).title(" ADB QR Pairing ").style(Style::default().fg(Color::Cyan));
                    let inner = block.inner(popup_area);
                    f.render_widget(Clear, popup_area);
                    f.render_widget(block, popup_area);
                    
                    let mut lines: Vec<Line> = qr_lines.iter().map(|s| Line::from(s.as_str())).collect();
                    lines.push(Line::from(""));
                    lines.push(Line::from("Scan this QR Code from Developer Options -> Wireless Debugging"));
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(format!("Service: {}", service_name), Style::default().fg(Color::Yellow))));
                    lines.push(Line::from(Span::styled(format!("Code: {}", password), Style::default().fg(Color::Yellow))));
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled("Press ESC or Q to cancel", Style::default().fg(Color::DarkGray))));
                    
                    let p = Paragraph::new(lines).alignment(Alignment::Center);
                    f.render_widget(p, inner);
                }
                _ => {}
            }
        })?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                let current_popup = popup_state.lock().unwrap().clone();
                let current_mode = *mode.lock().unwrap();

                match current_popup {
                    PopupState::QrCode(_, _) => {
                        if key.code == KeyCode::Esc || key.code == KeyCode::Enter || key.code == KeyCode::Char('q') {
                            *popup_state.lock().unwrap() = PopupState::None;
                            crate::utils::log_msg("QR Popup closed by user.");
                        }
                    }
                    PopupState::AdbActionSelection => {
                        match key.code {
                            KeyCode::Esc => *popup_state.lock().unwrap() = PopupState::None,
                            KeyCode::Char('1') => *popup_state.lock().unwrap() = PopupState::AdbIpInput(String::new(), false),
                            KeyCode::Char('2') => *popup_state.lock().unwrap() = PopupState::AdbIpInput(String::new(), true),
                            KeyCode::Char('3') => {
                                let micros = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_micros();
                                let password = format!("{:06}", micros % 1000000);
                                let service_name = format!("audiosrc-{:04}", micros % 10000);
                                let port = 43210; 
                                
                                let qr_string = format!("WIFI:T:ADB;S:{};P:{};;", service_name, password);
                                if let Ok(qr) = crate::web::generate_qr(&qr_string) {
                                    let mdns = ServiceDaemon::new().expect("Failed to create mDNS daemon");
                                    let service_type = "_adb-tls-pairing._tcp.local.";
                                    let host_name = format!("{}.local.", service_name);
                                    let properties: HashMap<String, String> = HashMap::new();
                                    let my_info = ServiceInfo::new(
                                        service_type,
                                        &service_name,
                                        &host_name,
                                        "",
                                        port,
                                        properties
                                    ).unwrap();
                                    
                                    if let Ok(_) = mdns.register(my_info) {
                                        *active_mdns.lock().unwrap() = Some(mdns);
                                        *popup_state.lock().unwrap() = PopupState::AdbQrPairing(qr.lines().map(|s| s.to_string()).collect(), service_name, password);
                                        crate::utils::log_msg("Started ADB mDNS Pairing Server. Scan the QR.");
                                    } else {
                                        crate::utils::log_msg("Failed to start mDNS Pairing Server.");
                                        *popup_state.lock().unwrap() = PopupState::None;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    PopupState::AdbIpInput(mut input, is_pairing) => {
                        match key.code {
                            KeyCode::Esc => *popup_state.lock().unwrap() = PopupState::None,
                            KeyCode::Backspace => {
                                input.pop();
                                *popup_state.lock().unwrap() = PopupState::AdbIpInput(input, is_pairing);
                            }
                            KeyCode::Enter => {
                                if is_pairing {
                                    *popup_state.lock().unwrap() = PopupState::AdbCodeInput(input, String::new());
                                } else {
                                    let ip = input.clone();
                                    *popup_state.lock().unwrap() = PopupState::None;
                                    crate::utils::log_msg(&format!("[ADB] Connecting to {}...", ip));
                                    std::thread::spawn(move || {
                                        if let Ok(res) = Command::new("adb").args(["connect", &ip]).output() {
                                            crate::utils::log_msg(&String::from_utf8_lossy(&res.stdout));
                                        }
                                    });
                                }
                            }
                            KeyCode::Char(c) => {
                                input.push(c);
                                *popup_state.lock().unwrap() = PopupState::AdbIpInput(input, is_pairing);
                            }
                            _ => {}
                        }
                    }
                    PopupState::AdbCodeInput(ip, mut code) => {
                        match key.code {
                            KeyCode::Esc => *popup_state.lock().unwrap() = PopupState::None,
                            KeyCode::Backspace => {
                                code.pop();
                                *popup_state.lock().unwrap() = PopupState::AdbCodeInput(ip, code);
                            }
                            KeyCode::Enter => {
                                let final_ip = ip.clone();
                                let final_code = code.clone();
                                *popup_state.lock().unwrap() = PopupState::None;
                                crate::utils::log_msg(&format!("[ADB] Pairing with {}...", final_ip));
                                std::thread::spawn(move || {
                                    if let Ok(res) = Command::new("adb").args(["pair", &final_ip, &final_code]).output() {
                                        crate::utils::log_msg(&String::from_utf8_lossy(&res.stdout));
                                    }
                                });
                            }
                            KeyCode::Char(c) => {
                                code.push(c);
                                *popup_state.lock().unwrap() = PopupState::AdbCodeInput(ip, code);
                            }
                            _ => {}
                        }
                    }
                    PopupState::AdbQrPairing(_, _, _) => {
                        if key.code == KeyCode::Esc || key.code == KeyCode::Enter || key.code == KeyCode::Char('q') {
                            *active_mdns.lock().unwrap() = None; // Drop mDNS daemon
                            *popup_state.lock().unwrap() = PopupState::None;
                            crate::utils::log_msg("ADB QR Pairing cancelled.");
                        }
                    }
                    PopupState::None => {
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Char('Q') => {
                                *running_flag.lock().unwrap() = false;
                                let pid_file = crate::utils::get_log_file().parent().unwrap().join("tray.pid");
                                if let Ok(pid_str) = std::fs::read_to_string(&pid_file) {
                                    if let Ok(pid) = pid_str.parse::<i32>() {
                                        unsafe { libc::kill(pid, 15); }
                                    }
                                }
                            }
                            KeyCode::Char('t') | KeyCode::Char('T') => {
                                *running_flag.lock().unwrap() = false;
                            }
                            KeyCode::Char('c') | KeyCode::Char('C') => {
                                if *is_streaming.lock().unwrap() {
                                    *is_streaming.lock().unwrap() = false;
                                    crate::utils::notify("AudioSource", "Audio stopped");
                                }
                            }
                            KeyCode::Char('z') | KeyCode::Char('Z') => {
                                let _ = crate::daemon::set_volume(None, "-10%");
                                // Optimistic UI update
                                let current = sys_volume.lock().unwrap().clone();
                                if let Ok(val) = current.trim_end_matches('%').parse::<i32>() {
                                    *sys_volume.lock().unwrap() = format!("{}%", std::cmp::max(0, val - 10));
                                }
                            }
                            KeyCode::Char('x') | KeyCode::Char('X') => {
                                let _ = crate::daemon::set_volume(None, "+10%");
                                // Optimistic UI update
                                let current = sys_volume.lock().unwrap().clone();
                                if let Ok(val) = current.trim_end_matches('%').parse::<i32>() {
                                    *sys_volume.lock().unwrap() = format!("{}%", std::cmp::min(150, val + 10)); // pactl allows >100%
                                }
                            }
                            KeyCode::Char('w') | KeyCode::Char('W') => {
                                if current_mode == AppMode::Web && web_server_started {
                                    match crate::web::get_qr_string() {
                                        Ok((qr, url)) => {
                                            *popup_state.lock().unwrap() = PopupState::QrCode(qr.lines().map(|s| s.to_string()).collect(), url);
                                        }
                                        Err(e) => crate::utils::log_msg(&format!("Failed to generate QR: {}", e)),
                                    }
                                } else {
                                    *mode.lock().unwrap() = AppMode::Web;
                                    crate::utils::log_msg("Web Mode selected. Press [S] to start server.");
                                }
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
                                        crate::utils::notify("AudioSource", "Streaming started");
                                        match crate::web::get_qr_string() {
                                            Ok((qr, url)) => {
                                                *popup_state.lock().unwrap() = PopupState::QrCode(qr.lines().map(|s| s.to_string()).collect(), url.clone());
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
                                    } else {
                                        crate::utils::log_msg("Web server already running.");
                                    }
                                } else if current_mode == AppMode::AdbWifi {
                                    *popup_state.lock().unwrap() = PopupState::AdbActionSelection;
                                } else if current_mode == AppMode::AdbUsb {
                                    crate::utils::log_msg("ADB USB connection starting... (Not implemented yet)");
                                }
                            }
                            KeyCode::Char('m') | KeyCode::Char('M') => {
                                let current = sys_volume.lock().unwrap().clone();
                                if current == "0%" {
                                    let _ = crate::daemon::set_volume(None, "100%");
                                    *sys_volume.lock().unwrap() = "100%".to_string();
                                } else {
                                    let _ = crate::daemon::set_volume(None, "0%");
                                    *sys_volume.lock().unwrap() = "0%".to_string();
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    
    Ok(())
}
