use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::Style,
    text::{Line, Span},
    widgets::{Gauge, Paragraph},
    Frame,
};
use ratatui_image::{protocol::StatefulProtocol, StatefulImage};
use crate::app::AppState;
use crate::theme::Theme;
use crate::tui::widgets::vaporwave_block;

pub fn render(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    state: &AppState,
    artwork: &mut Option<StatefulProtocol>,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(24), Constraint::Min(0)])
        .split(area);

    // Left: artwork area
    render_artwork(frame, chunks[0], artwork);

    // Right: song info
    render_info(frame, chunks[1], state);
}

fn render_artwork(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    artwork: &mut Option<StatefulProtocol>,
) {
    let block = vaporwave_block("Art");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(protocol) = artwork.as_mut() {
        let img = StatefulImage::default();
        frame.render_stateful_widget(img, inner, protocol);
    } else {
        // Placeholder: alternating color unicode blocks
        let art_lines: Vec<Line> = (0..inner.height)
            .map(|i| {
                let color = if i % 2 == 0 { Theme::PURPLE } else { Theme::PINK };
                Line::from(Span::styled(
                    "\u{2588}".repeat(inner.width as usize),
                    Style::default().fg(color),
                ))
            })
            .collect();
        frame.render_widget(
            Paragraph::new(art_lines).style(Style::default().bg(Theme::BACKGROUND)),
            inner,
        );
    }
}

fn render_info(frame: &mut Frame, area: ratatui::layout::Rect, state: &AppState) {
    let block = vaporwave_block("Now Playing");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(np) = &state.now_playing else {
        let para = Paragraph::new(Span::styled("Waiting for song data...", Theme::dim()))
            .style(Style::default().bg(Theme::BACKGROUND));
        frame.render_widget(para, inner);
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // artist
            Constraint::Length(1), // title
            Constraint::Length(1), // album
            Constraint::Length(1), // blank
            Constraint::Length(1), // duration gauge
            Constraint::Length(1), // listeners
            Constraint::Length(1), // reactions
            Constraint::Length(1), // blank
            Constraint::Length(1), // controls
        ])
        .split(inner);

    // Artist
    frame.render_widget(
        Paragraph::new(Span::styled(&np.song.artist, Theme::pink_bold())),
        chunks[0],
    );

    // Title
    frame.render_widget(
        Paragraph::new(Span::styled(&np.song.title, Theme::cyan_bold())),
        chunks[1],
    );

    // Album
    frame.render_widget(
        Paragraph::new(Span::styled(
            np.song.album.as_deref().unwrap_or(""),
            Theme::lavender(),
        )),
        chunks[2],
    );

    // Duration gauge — advance position client-side using elapsed time since last status update
    let elapsed = state.status_received_at
        .map(|t| t.elapsed().as_secs_f64())
        .unwrap_or(0.0);
    let position = (np.song.position.unwrap_or(0.0) + elapsed)
        .min(np.song.length.unwrap_or(0) as f64);
    let length = np.song.length.unwrap_or(1).max(1) as f64;
    let progress = (position / length).clamp(0.0, 1.0);
    let progress = if progress.is_nan() || progress.is_infinite() { 0.0 } else { progress };
    let pos_str = {
        let secs = position as u32;
        format!("{}:{:02}", secs / 60, secs % 60)
    };
    let len_str = np.song.duration_display();

    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(Theme::PINK).bg(Theme::DIM))
        .ratio(progress)
        .label(format!("{} / {}", pos_str, len_str));
    frame.render_widget(gauge, chunks[4]);

    // Listeners
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!("\u{25c9} {} listening", np.listeners),
            Theme::lavender(),
        )),
        chunks[5],
    );

    // Reactions
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!("\u{2665} {} reactions", np.song.reactions),
            Theme::pink_bold(),
        )),
        chunks[6],
    );

    // Controls hint
    frame.render_widget(
        Paragraph::new(Span::styled(
            "[Space] Play/Pause  [+/-] Volume  [f] Favorite  [r] React",
            Theme::dim(),
        )),
        chunks[8],
    );
}
