use crate::app::AppState;
use crate::theme::Theme;
use crate::tui::widgets::centered_rect;
use ratatui::{
    layout::Alignment,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, area: ratatui::layout::Rect, _state: &AppState) {
    let popup_area = centered_rect(60, 80, area);

    let block = Block::default()
        .title(Span::styled(" Keyboard Shortcuts ", Theme::title()))
        .borders(Borders::ALL)
        .border_style(Theme::border())
        .style(Style::default().bg(Theme::BACKGROUND));

    let inner = block.inner(popup_area);

    frame.render_widget(Clear, popup_area);
    frame.render_widget(block, popup_area);

    let shortcuts = vec![
        ("1-6", "Switch view"),
        ("j / k", "Scroll down / up"),
        ("J / K", "Scroll 5 lines"),
        ("g / G", "Jump to top / bottom"),
        ("Enter", "Select / confirm"),
        ("Space", "Play / pause radio"),
        ("+ / -", "Volume up / down (5%)"),
        ("f", "Toggle favorite"),
        ("r", "Send reaction"),
        ("h / l", "Previous / next tab (Charts)"),
        ("d", "Remove favorite"),
        ("e", "Export favorites (Favorites view)"),
        ("t", "Sleep timer (off / 15 / 30 / 60 min)"),
        ("L", "Logout (Profile view)"),
        ("?", "Toggle this help"),
        ("q", "Quit"),
    ];

    let lines: Vec<Line> = shortcuts
        .iter()
        .map(|(key, action)| {
            Line::from(vec![
                Span::styled(format!("{:12}", key), Theme::cyan_bold()),
                Span::styled("  ", Theme::dim()),
                Span::styled(*action, Theme::text()),
            ])
        })
        .collect();

    let para = Paragraph::new(lines)
        .style(Style::default().bg(Theme::BACKGROUND))
        .alignment(Alignment::Left);
    frame.render_widget(para, inner);
}
