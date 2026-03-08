use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};
use ratatui_image::{protocol::StatefulProtocol, StatefulImage};
use crate::app::SongDetailState;
use crate::theme::Theme;

pub fn vaporwave_block(title: &str) -> Block<'_> {
    Block::default()
        .title(Span::styled(format!(" {} ", title), Theme::title()))
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Double)
        .border_style(Theme::border())
        .style(Style::default().bg(Theme::BACKGROUND))
}

pub fn render_song_detail_popup(
    frame: &mut Frame,
    detail: &SongDetailState,
    artwork: &mut Option<StatefulProtocol>,
) {
    let area = centered_rect(65, 55, frame.area());
    let block = vaporwave_block("Song Details");
    let inner = block.inner(area);

    frame.render_widget(Clear, area);
    frame.render_widget(block, area);

    // Split inner: left for text, right for artwork
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(20)])
        .split(inner);

    // Right: artwork
    if let Some(protocol) = artwork.as_mut() {
        frame.render_stateful_widget(StatefulImage::default(), cols[1], protocol);
    } else {
        // Placeholder block
        let art_block = Block::default()
            .style(Style::default().bg(Theme::PURPLE));
        frame.render_widget(art_block, cols[1]);
    }

    // Left: song info rows
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Artist
            Constraint::Length(1), // Album
            Constraint::Length(1), // Title
            Constraint::Length(1), // blank
            Constraint::Length(1), // Duration + reactions
            Constraint::Length(1), // Favorite status
            Constraint::Length(1), // blank
            Constraint::Length(1), // First played
            Constraint::Min(0),    // spacer
            Constraint::Length(1), // Key hints
        ])
        .split(cols[0]);

    let song = &detail.song;

    // Artist
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Artist: ", Theme::dim()),
            Span::styled(&song.artist, Theme::pink_bold()),
        ])),
        rows[0],
    );

    // Album
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Album:  ", Theme::dim()),
            Span::styled(
                song.album.as_deref().unwrap_or("\u{2014}"),
                Theme::lavender(),
            ),
        ])),
        rows[1],
    );

    // Title
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Title:  ", Theme::dim()),
            Span::styled(&song.title, Theme::cyan_bold()),
        ])),
        rows[2],
    );

    // Duration + reactions
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("\u{231a} ", Theme::dim()),
            Span::styled(song.duration_display(), Theme::text()),
            Span::styled("  \u{2665} ", Theme::pink_bold()),
            Span::styled(song.reactions.to_string(), Theme::text()),
        ])),
        rows[4],
    );

    // Favorite status
    let fav_text = if detail.loading_favorite {
        Span::styled("  \u{2605} checking...", Theme::dim())
    } else if detail.is_favorited {
        Span::styled("  \u{2605} In your favorites", Theme::pink_bold())
    } else {
        Span::styled("  \u{2606} Not in favorites", Theme::dim())
    };
    frame.render_widget(Paragraph::new(Line::from(fav_text)), rows[5]);

    // First played
    let played_str = match detail.played_at {
        Some(ts) => {
            use chrono::{TimeZone, Utc};
            let dt = Utc.timestamp_opt(ts, 0).single()
                .map(|d| d.format("First Played: %b %-d, %Y").to_string())
                .unwrap_or_default();
            dt
        }
        None => String::new(),
    };
    if !played_str.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(played_str, Theme::dim())),
            rows[7],
        );
    }

    // Key hints
    let preview_hint = if detail.is_previewing { "[p] Stop Preview" } else { "[p] Preview" };
    let fav_hint = if !detail.loading_favorite && detail.is_favorited {
        "[f] Unfavorite"
    } else {
        "[f] Favorite"
    };
    let hints = format!("{}  {}  [Esc] Close", preview_hint, fav_hint);
    frame.render_widget(
        Paragraph::new(Span::styled(hints, Theme::dim())),
        rows[9],
    );

    // Preview indicator overlay
    if detail.is_previewing {
        let preview_area = rows[6];
        frame.render_widget(
            Paragraph::new(Span::styled(
                "\u{25b6} Playing preview...",
                Style::default().fg(Theme::CYAN),
            )),
            preview_area,
        );
    }
}

pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
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
