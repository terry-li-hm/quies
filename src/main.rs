mod audio;
mod ui;

use std::env;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::DefaultTerminal;

use audio::{AudioEngine, PRESETS};

pub struct App {
    engine: AudioEngine,
    selected: usize,
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
            engine,
            selected: 0,
        })
    }

    fn next_layer(&mut self) {
        if !self.engine.layers.is_empty() {
            self.selected = (self.selected + 1) % self.engine.layers.len();
        }
    }

    fn prev_layer(&mut self) {
        if !self.engine.layers.is_empty() {
            self.selected = self
                .selected
                .checked_sub(1)
                .unwrap_or(self.engine.layers.len() - 1);
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

Presets: {}

TUI controls: j/k select, h/l volume, m mute, q quit",
        preset_names()
    );
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
            let _ = write_response(&stream, "stopped\n");
            return false;
        }
        "status" => {
            format!("playing\n{}\n", app.engine.status())
        }
        "vol" => {
            let layer_name = parts.get(1).unwrap_or(&"");
            let vol_str = parts.get(2).unwrap_or(&"");
            match (app.engine.find_layer(layer_name), vol_str.parse::<f32>()) {
                (Some(idx), Ok(vol)) => {
                    app.engine.set_volume(idx, vol);
                    let actual = (app.engine.get_volume(idx) * 100.0).round() as u8;
                    format!("{} → {}%\n", app.engine.layers[idx].name, actual)
                }
                (None, _) => format!("unknown layer: {layer_name}\n"),
                (_, Err(_)) => format!("invalid volume: {vol_str}\n"),
            }
        }
        "mute" => {
            let layer_name = parts.get(1).unwrap_or(&"");
            match app.engine.find_layer(layer_name) {
                Some(idx) => {
                    app.engine.toggle_mute(idx);
                    let state = if app.engine.is_active(idx) {
                        "unmuted"
                    } else {
                        "muted"
                    };
                    format!("{} {state}\n", app.engine.layers[idx].name)
                }
                None => format!("unknown layer: {layer_name}\n"),
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
            match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Char('j') | KeyCode::Down => app.next_layer(),
                KeyCode::Char('k') | KeyCode::Up => app.prev_layer(),
                KeyCode::Char('l') | KeyCode::Right => app.engine.volume_up(app.selected),
                KeyCode::Char('h') | KeyCode::Left => app.engine.volume_down(app.selected),
                KeyCode::Char('m') => app.engine.toggle_mute(app.selected),
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
