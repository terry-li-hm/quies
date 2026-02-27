mod audio;
mod ui;

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

fn main() -> anyhow::Result<()> {
    let preset = std::env::args().nth(1).unwrap_or_else(|| "default".into());

    if preset == "--help" || preset == "-h" {
        let names: Vec<&str> = PRESETS.iter().map(|(name, _)| *name).collect();
        eprintln!(
            "Usage: quies [preset]\n\nPresets: {}\n\nControls: j/k select, h/l volume, m mute, q quit",
            names.join(", ")
        );
        std::process::exit(0);
    }

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &preset);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, preset: &str) -> anyhow::Result<()> {
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
