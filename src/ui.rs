use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, Paragraph};
use ratatui::Frame;

use crate::App;
use crate::audio::LayerStatus;

pub fn render(frame: &mut Frame, app: &App) {
    let engine = app.engine.lock().unwrap();
    let layer_count = engine.layers.len();

    let mut constraints: Vec<Constraint> = Vec::new();
    // Title
    constraints.push(Constraint::Length(1));
    // One line per layer
    for _ in 0..layer_count {
        constraints.push(Constraint::Length(1));
    }
    // Spacer + help bar
    constraints.push(Constraint::Fill(1));
    constraints.push(Constraint::Length(1));

    let areas = Layout::vertical(constraints).split(frame.area());

    // Title
    let title = Paragraph::new(Line::from(vec![Span::raw(" quies").bold()]));
    frame.render_widget(title, areas[0]);

    // Layers
    for (i, layer) in engine.layers.iter().enumerate() {
        let vol = engine.get_volume(i);
        let active = engine.is_active(i);
        let selected = i == app.selected;
        let status = layer.status.lock().unwrap();

        let kind = if layer.url.is_some() { "♪" } else { "~" };
        let prefix = if selected { format!(" \u{25b8} {kind} ") } else { format!("   {kind} ") };

        // For non-playing states, show status instead of volume
        let label_text = match &*status {
            LayerStatus::Downloading => format!("{prefix}{:<16} [downloading...]", layer.name),
            LayerStatus::Error(e) => format!("{prefix}{:<16} [error: {e}]", layer.name),
            LayerStatus::Playing => {
                let pct = (vol * 100.0).round() as u8;
                let suffix = if !active { " [off]" } else { "" };
                format!("{prefix}{:<16} {pct:>3}%{suffix}", layer.name)
            }
        };

        let gauge_style = if !active {
            Style::new().dim()
        } else if selected {
            Style::new().bold()
        } else {
            Style::new()
        };

        let ratio = if active && matches!(*status, LayerStatus::Playing) { vol as f64 } else { 0.0 };
        drop(status);

        // Render label and gauge bar side by side
        let area = areas[1 + i];
        let horiz = Layout::horizontal([Constraint::Length(28), Constraint::Fill(1)]).split(area);

        let label_style = if !active {
            Style::new().dim()
        } else if selected {
            Style::new().bold()
        } else {
            Style::new()
        };

        let label = Paragraph::new(label_text).style(label_style);
        frame.render_widget(label, horiz[0]);

        let gauge = Gauge::default()
            .gauge_style(gauge_style)
            .ratio(ratio.clamp(0.0, 1.0));
        frame.render_widget(gauge, horiz[1]);
    }

    // Help bar
    let help = Paragraph::new(Line::from(vec![
        Span::raw(" j/k").bold(),
        Span::raw(" select  "),
        Span::raw("h/l").bold(),
        Span::raw(" volume  "),
        Span::raw("m").bold(),
        Span::raw(" mute  "),
        Span::raw("q").bold(),
        Span::raw(" quit"),
    ]));

    frame.render_widget(help, areas[areas.len() - 1]);
}
