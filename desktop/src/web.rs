use anyhow::Result;
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use axum_server::tls_rustls::RustlsConfig;
use futures_util::stream::StreamExt;
use local_ip_address::local_ip;
use qrcode::QrCode;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const INDEX_HTML: &str = include_str!("index.html");
const WORKLET_JS: &str = include_str!("worklet.js");

fn get_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("audiosource")
}

fn generate_or_load_certs() -> Result<(PathBuf, PathBuf)> {
    let config_dir = get_config_dir();
    fs::create_dir_all(&config_dir)?;

    let cert_path = config_dir.join("cert.pem");
    let key_path = config_dir.join("key.pem");

    if !cert_path.exists() || !key_path.exists() {
        println!("Generating new self-signed certificate...");
        let subject_alt_names = vec!["localhost".to_string(), local_ip()?.to_string()];
        let cert = rcgen::generate_simple_self_signed(subject_alt_names)?;
        
        fs::write(&cert_path, cert.cert.pem())?;
        fs::write(&key_path, cert.key_pair.serialize_pem())?;
    }

    Ok((cert_path, key_path))
}

pub fn get_qr_string() -> Result<(String, String)> {
    let ip = local_ip()?;
    let url = format!("https://{}:8443", ip);
    
    let code = QrCode::new(url.as_bytes())?;
    let string = code.render::<char>()
        .quiet_zone(false)
        .module_dimensions(2, 1)
        .build();
        
    Ok((string, url))
}

pub async fn run_web_server() -> Result<()> {
    let (cert_path, key_path) = generate_or_load_certs()?;
    let config = RustlsConfig::from_pem_file(&cert_path, &key_path).await?;

    let app = Router::new()
        .route("/", get(|| async { Html(INDEX_HTML) }))
        .route("/worklet.js", get(|| async { 
            ([(axum::http::header::CONTENT_TYPE, "application/javascript")], WORKLET_JS) 
        }))
        .route("/ws", get(ws_handler));
    // Load PulseAudio modules once before accepting connections
    let name = "audiosource_web";
    crate::daemon::unload_module(name);
    Command::new("pactl").args([
        "load-module", "module-null-sink",
        &format!("sink_name={}_sink", name),
        "sink_properties=device.description=AudioSource_Buffer"
    ]).output()?;
    Command::new("pactl").args([
        "load-module", "module-virtual-source",
        &format!("source_name={}", name),
        &format!("master={}_sink.monitor", name),
        "source_properties=device.description=AudioSource_Microphone device.class=sound device.icon_name=audio-input-microphone"
    ]).output()?;

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], 8443));
    axum_server::bind_rustls(addr, config)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(mut socket: WebSocket) {
    println!("Client connected to WebSocket");
    
    let name = "audiosource_web";
    
    let mut pacat_proc = match Command::new("pacat")
        .args([
            "--playback",
            &format!("--device={}_sink", name),
            "--format=s16le",
            "--channels=1",
            "--rate=44100",
            "--latency-msec=300",
        ])
        .stdin(Stdio::piped())
        .spawn() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Failed to spawn pacat: {}", e);
                return;
            }
        };

    let mut stdin = pacat_proc.stdin.take().expect("Failed to open stdin");

    use std::io::Write;
    while let Some(Ok(msg)) = socket.next().await {
        if let Message::Binary(data) = msg {
            if let Err(e) = stdin.write_all(&data) {
                eprintln!("Failed to write to pacat: {}", e);
                break;
            }
        }
    }

    println!("Client disconnected");
    let _ = pacat_proc.kill();
    let _ = pacat_proc.wait();
}
