use anyhow::Result;
use ksni::{Tray, TrayMethods, MenuItem, Icon};
use std::process::Command;
use std::sync::{Arc, Mutex};

pub struct AudioSourceTray {
    pub muted: bool,
    pub device_name: String,
}

impl Tray for AudioSourceTray {
    fn id(&self) -> String {
        "audiosource".into()
    }

    fn icon_name(&self) -> String {
        // Icon name from assets, or a standard icon like "audio-input-microphone"
        "audio-input-microphone".into()
    }

    fn title(&self) -> String {
        "AudioSource".into()
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        use ksni::menu::*;
        vec![
            StandardItem {
                label: self.device_name.clone(),
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Open Console (TUI)".into(),
                activate: Box::new(|_| {
                    println!("Opening TUI...");
                    open_tui();
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Restart App".into(),
                activate: Box::new(|_| {
                    println!("Restarting app...");
                    // In a real scenario we might send a signal or spawn a new process.
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Stop Audio".into(),
                activate: Box::new(|_| {
                    println!("Stopping audio...");
                    // Call daemon stop logic
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: if self.muted { "Unmute Mic".into() } else { "Mute Mic".into() },
                activate: Box::new(|this: &mut Self| {
                    this.muted = !this.muted;
                    let vol = if this.muted { "0%" } else { "100%" };
                    let _ = crate::daemon::set_volume(None, vol);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|_| {
                    std::process::exit(0);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

fn open_tui() {
    let tui_path = std::env::current_exe().unwrap_or_else(|_| "audiosource".into());
    let terminals = vec![
        vec!["foot", "-e"],
        vec!["ghostty", "-e"],
        vec!["kitty", "--"],
        vec!["x-terminal-emulator", "-e"],
        vec!["gnome-terminal", "--"],
        vec!["konsole", "-e"],
        vec!["xfce4-terminal", "-x"],
        vec!["alacritty", "-e"],
    ];
    
    for term in terminals {
        let mut cmd = Command::new(term[0]);
        cmd.arg(term[1]).arg(&tui_path).arg("tui");
        if let Ok(mut child) = cmd.spawn() {
            // Detach and let it run
            return;
        }
    }
    eprintln!("Could not find a suitable terminal emulator to open TUI.");
}

pub fn run_tray() -> Result<()> {
    let mut tray = AudioSourceTray {
        muted: false,
        device_name: "No device connected".into(),
    };
    
    // Get initial device name
    if let Ok(output) = Command::new("adb").args(["shell", "getprop", "ro.product.model"]).output() {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !name.is_empty() {
            tray.device_name = name;
        }
    }

    let mut rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let handle = tray.spawn().await.unwrap();
        
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
            if let Ok(output) = Command::new("adb").args(["shell", "getprop", "ro.product.model"]).output() {
                let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !name.is_empty() {
                    handle.update(|tray: &mut AudioSourceTray| {
                        tray.device_name = name;
                    }).await;
                }
            }
        }
    });
    Ok(())
}
