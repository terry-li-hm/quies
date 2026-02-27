mod audio;
mod ui;

use std::collections::hash_map::DefaultHasher;
use std::env;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::DefaultTerminal;

use audio::{AudioEngine, LayerStatus, PRESETS};

pub struct App {
    engine: Arc<Mutex<AudioEngine>>,
    selected: usize,
    download_count: Arc<Mutex<u8>>,
}

impl App {
    fn new(preset_name: &str) -> anyhow::Result<Self> {
        let preset = PRESETS
            .iter()
            .find(|(name, _)| *name == preset_name)
            .ok_or_else(|| anyhow::anyhow!("unknown preset: {preset_name}"))?;

        let mut engine = AudioEngine::new()?;
        for layer in preset.1 {
            engine.add_layer(layer.name, layer.noise_type, layer.volume);
        }

        Ok(Self {
            engine: Arc::new(Mutex::new(engine)),
            selected: 0,
            download_count: Arc::new(Mutex::new(0)),
        })
    }

    fn layer_count(&self) -> usize {
        self.engine.lock().unwrap().layers.len()
    }

    fn next_layer(&mut self) {
        let count = self.layer_count();
        if count > 0 {
            self.selected = (self.selected + 1) % count;
        }
    }

    fn prev_layer(&mut self) {
        let count = self.layer_count();
        if count > 0 {
            self.selected = self.selected.checked_sub(1).unwrap_or(count - 1);
        }
    }
}

fn socket_path() -> PathBuf {
    env::var("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| "/tmp".into())
        .join("quies.sock")
}

fn preset_names() -> String {
    PRESETS
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

fn print_help() {
    eprintln!(
        "Usage: quies [command]

Commands:
  (no args)          TUI mode with interactive controls
  start [preset]     Start daemon (background playback)
  stop               Stop daemon
  status             Show what's playing
  vol <layer> <0-1>  Set layer volume (e.g. vol brown 0.6)
  mute <layer>       Toggle mute on a layer
  add <name> <url>   Add audio layer from URL (YouTube or direct)

Presets: {}

TUI controls: j/k select, h/l volume, m mute, q quit",
        preset_names()
    );
}

fn url_hash(url: &str) -> String {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn cache_dir() -> PathBuf {
    env::var("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| "/tmp".into())
}

fn is_youtube_url(url: &str) -> bool {
    url.contains("youtube.com") || url.contains("youtu.be")
}

fn check_command(name: &str) -> anyhow::Result<()> {
    match Command::new(name).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status() {
        Ok(s) if s.success() => Ok(()),
        _ => anyhow::bail!("{name} not found — install with: brew install {name}"),
    }
}

/// Spawn a background download thread. Updates layer status on completion/failure.
fn spawn_download(app: &App, idx: usize, url: String, _name: String) {
    let engine = Arc::clone(&app.engine);
    let download_count = Arc::clone(&app.download_count);

    // Concurrent download cap: max 3
    {
        let mut count = download_count.lock().unwrap();
        if *count >= 3 {
            let eng = engine.lock().unwrap();
            *eng.layers[idx].status.lock().unwrap() = LayerStatus::Error("too many downloads".to_string());
            return;
        }
        *count += 1;
    }

    std::thread::spawn(move || {
        let result = run_download(&url);
        let mut eng = engine.lock().unwrap();

        match result {
            Ok(path) => {
                if let Err(e) = eng.activate_audio_layer(idx, path) {
                    *eng.layers[idx].status.lock().unwrap() = LayerStatus::Error(format!("decode: {e}"));
                }
            }
            Err(e) => {
                *eng.layers[idx].status.lock().unwrap() = LayerStatus::Error(e.to_string());
            }
        }

        *download_count.lock().unwrap() -= 1;
    });
}

fn run_download(url: &str) -> anyhow::Result<PathBuf> {
    let hash = url_hash(url);

    if is_youtube_url(url) {
        check_command("yt-dlp")?;
        let final_path = cache_dir().join(format!("quies-{hash}.m4a"));
        // Cache hit
        if final_path.exists() {
            return Ok(final_path);
        }
        let tmp_path = cache_dir().join(format!("quies-{hash}.tmp"));
        let output = Command::new("yt-dlp")
            .args([
                "-f", "bestaudio[ext=m4a]/bestaudio",
                "--max-filesize", "200m",
                "--no-playlist",
                "--no-progress",
                "-o", tmp_path.to_str().unwrap(),
                url,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("{}", stderr.lines().last().unwrap_or("yt-dlp failed"));
        }
        std::fs::rename(&tmp_path, &final_path)?;
        Ok(final_path)
    } else {
        check_command("curl")?;
        // Guess extension from URL, default to mp3
        let ext = url.rsplit('.').next()
            .filter(|e| ["mp3", "m4a", "flac", "wav", "ogg", "aac"].contains(e))
            .unwrap_or("mp3");
        let final_path = cache_dir().join(format!("quies-{hash}.{ext}"));
        // Cache hit
        if final_path.exists() {
            return Ok(final_path);
        }
        let tmp_path = cache_dir().join(format!("quies-{hash}.tmp"));
        let output = Command::new("curl")
            .args([
                "-fSL",
                "--max-filesize", "209715200",
                "--max-time", "600",
                "-o", tmp_path.to_str().unwrap(),
                url,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()?;

        if !output.status.success() {
            let _ = std::fs::remove_file(&tmp_path);
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("{}", stderr.lines().last().unwrap_or("download failed"));
        }
        std::fs::rename(&tmp_path, &final_path)?;
        Ok(final_path)
    }
}

// --- Client: send command to running daemon ---

fn send_command(cmd: &str) -> anyhow::Result<String> {
    let mut stream =
        UnixStream::connect(socket_path()).map_err(|_| anyhow::anyhow!("daemon not running"))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    writeln!(stream, "{cmd}")?;
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => response.push_str(&line),
            Err(_) => break,
        }
    }
    Ok(response)
}

// --- Daemon: background audio with socket control ---

fn run_daemon(preset: &str) -> anyhow::Result<()> {
    let app = App::new(preset)?;
    let sock = socket_path();

    // Clean up stale socket
    let _ = std::fs::remove_file(&sock);

    let listener = UnixListener::bind(&sock)?;
    listener.set_nonblocking(true)?;

    // Write PID for reference
    let pid_path = sock.with_extension("pid");
    std::fs::write(&pid_path, std::process::id().to_string())?;

    eprintln!("quies daemon started (preset: {preset}, pid: {})", std::process::id());

    loop {
        // Check for incoming connections
        match listener.accept() {
            Ok((stream, _)) => {
                if !handle_client(stream, &app) {
                    break; // "stop" command received
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => eprintln!("accept error: {e}"),
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    // Cleanup
    let _ = std::fs::remove_file(&sock);
    let _ = std::fs::remove_file(&pid_path);
    Ok(())
}

/// Handle a client connection. Returns false if daemon should stop.
fn handle_client(stream: UnixStream, app: &App) -> bool {
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return true;
    }
    let parts: Vec<&str> = line.trim().splitn(3, ' ').collect();
    let cmd = parts.first().copied().unwrap_or("");

    let response = match cmd {
        "stop" => {
            // Clean up cached audio files
            let eng = app.engine.lock().unwrap();
            for layer in &eng.layers {
                if let Some(path) = &layer.path {
                    let _ = std::fs::remove_file(path);
                }
            }
            drop(eng);
            let _ = write_response(&stream, "stopped\n");
            return false;
        }
        "status" => {
            let eng = app.engine.lock().unwrap();
            format!("playing\n{}\n", eng.status())
        }
        "vol" => {
            let layer_name = parts.get(1).unwrap_or(&"");
            let vol_str = parts.get(2).unwrap_or(&"");
            let eng = app.engine.lock().unwrap();
            match (eng.find_layer(layer_name), vol_str.parse::<f32>()) {
                (Some(idx), Ok(vol)) => {
                    eng.set_volume(idx, vol);
                    let actual = (eng.get_volume(idx) * 100.0).round() as u8;
                    format!("{} → {}%\n", eng.layers[idx].name, actual)
                }
                (None, _) => format!("unknown layer: {layer_name}\n"),
                (_, Err(_)) => format!("invalid volume: {vol_str}\n"),
            }
        }
        "mute" => {
            let layer_name = parts.get(1).unwrap_or(&"");
            let eng = app.engine.lock().unwrap();
            match eng.find_layer(layer_name) {
                Some(idx) => {
                    eng.toggle_mute(idx);
                    let state = if eng.is_active(idx) { "unmuted" } else { "muted" };
                    format!("{} {state}\n", eng.layers[idx].name)
                }
                None => format!("unknown layer: {layer_name}\n"),
            }
        }
        "add" => {
            let name = parts.get(1).unwrap_or(&"");
            let url = parts.get(2).unwrap_or(&"");
            if name.is_empty() || url.is_empty() {
                "usage: add <name> <url>\n".to_string()
            } else {
                // Check cache first
                let hash = url_hash(url);
                let cached = if is_youtube_url(url) {
                    let p = cache_dir().join(format!("quies-{hash}.m4a"));
                    if p.exists() { Some(p) } else { None }
                } else {
                    let ext = url.rsplit('.').next()
                        .filter(|e| ["mp3", "m4a", "flac", "wav", "ogg", "aac"].contains(e))
                        .unwrap_or("mp3");
                    let p = cache_dir().join(format!("quies-{hash}.{ext}"));
                    if p.exists() { Some(p) } else { None }
                };

                if let Some(path) = cached {
                    let mut eng = app.engine.lock().unwrap();
                    match eng.add_audio_layer(name, path, url, 0.5) {
                        Ok(()) => format!("♪ {name} added (cached)\n"),
                        Err(e) => format!("error: {e}\n"),
                    }
                } else {
                    let mut eng = app.engine.lock().unwrap();
                    let (idx, _, _, _) = eng.add_pending_layer(name, url, 0.5);
                    drop(eng);
                    spawn_download(app, idx, url.to_string(), name.to_string());
                    format!("♪ {name} downloading...\n")
                }
            }
        }
        _ => format!("unknown command: {cmd}\n"),
    };

    let _ = write_response(&stream, &response);
    true
}

fn write_response(mut stream: &UnixStream, msg: &str) -> std::io::Result<()> {
    stream.write_all(msg.as_bytes())?;
    stream.flush()
}

// --- TUI mode ---

fn run_tui(terminal: &mut DefaultTerminal, preset: &str) -> anyhow::Result<()> {
    let mut app = App::new(preset)?;

    loop {
        terminal.draw(|f| ui::render(f, &app))?;

        if event::poll(Duration::from_millis(50))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            let sel = app.selected;
            match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Char('j') | KeyCode::Down => app.next_layer(),
                KeyCode::Char('k') | KeyCode::Up => app.prev_layer(),
                KeyCode::Char('l') | KeyCode::Right => app.engine.lock().unwrap().volume_up(sel),
                KeyCode::Char('h') | KeyCode::Left => app.engine.lock().unwrap().volume_down(sel),
                KeyCode::Char('m') => app.engine.lock().unwrap().toggle_mute(sel),
                _ => {}
            }
        }
    }

    Ok(())
}

// --- Entry point ---

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("");

    match cmd {
        "--help" | "-h" => {
            print_help();
            Ok(())
        }
        "start" => {
            let preset = args.get(1).map(|s| s.as_str()).unwrap_or("default");
            // Check if daemon is already running
            if UnixStream::connect(socket_path()).is_ok() {
                anyhow::bail!("daemon already running ({})", socket_path().display());
            }
            // Spawn daemon as child process and exit
            let exe = env::current_exe()?;
            let log_path = socket_path().with_extension("log");
            let log_file = std::fs::File::create(&log_path)?;
            let child = std::process::Command::new(exe)
                .arg("--daemon")
                .arg(preset)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(log_file)
                .spawn()?;
            eprintln!("log: {}", log_path.display());
            println!("quies daemon started (pid: {})", child.id());
            Ok(())
        }
        "--daemon" => {
            // Hidden: called by "start" to run as background daemon
            let preset = args.get(1).map(|s| s.as_str()).unwrap_or("default");
            run_daemon(preset)
        }
        "stop" => {
            let resp = send_command("stop")?;
            print!("{resp}");
            Ok(())
        }
        "status" => {
            let resp = send_command("status")?;
            print!("{resp}");
            Ok(())
        }
        "vol" => {
            if args.len() < 3 {
                anyhow::bail!("usage: quies vol <layer> <0.0-1.0>");
            }
            let resp = send_command(&format!("vol {} {}", args[1], args[2]))?;
            print!("{resp}");
            Ok(())
        }
        "mute" => {
            if args.len() < 2 {
                anyhow::bail!("usage: quies mute <layer>");
            }
            let resp = send_command(&format!("mute {}", args[1]))?;
            print!("{resp}");
            Ok(())
        }
        "add" => {
            if args.len() < 3 {
                anyhow::bail!("usage: quies add <name> <url>");
            }
            let resp = send_command(&format!("add {} {}", args[1], args[2]))?;
            print!("{resp}");
            Ok(())
        }
        _ => {
            // TUI mode: treat arg as preset name
            let preset = if cmd.is_empty() { "default" } else { cmd };
            let mut terminal = ratatui::init();
            let result = run_tui(&mut terminal, preset);
            ratatui::restore();
            result
        }
    }
}
