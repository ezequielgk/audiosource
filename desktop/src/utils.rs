use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

pub fn get_log_file() -> PathBuf {
    let dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("audiosource");
    std::fs::create_dir_all(&dir).unwrap_or(());
    dir.join("audiosource.log")
}

pub fn log_msg(msg: &str) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(get_log_file()) {
        let _ = writeln!(file, "{}", msg);
    }
}

pub fn notify(title: &str, body: &str) {
    let icon_path = dirs::config_dir().unwrap_or_default().join("audiosource").join("icon.png");
    let mut cmd = std::process::Command::new("notify-send");
    cmd.args(["-a", "AudioSource"]);
    if icon_path.exists() {
        cmd.args(["-i", icon_path.to_str().unwrap()]);
    }
    cmd.args([title, body]);
    let _ = cmd.spawn();
}
