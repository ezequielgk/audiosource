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
