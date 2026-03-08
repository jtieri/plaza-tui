use ratatui::layout::{Constraint, Direction, Layout, Rect};

pub struct AppLayout {
    pub header: Rect,
    pub sidebar: Rect,
    pub content: Rect,
    pub status_bar: Rect,
}

impl AppLayout {
    pub fn new(area: Rect) -> Self {
        // Split vertically: header (3) / content area / status bar (1)
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(area);

        let header = vertical[0];
        let status_bar = vertical[2];

        // Split content area: sidebar (18) / main content
        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(18), Constraint::Min(0)])
            .split(vertical[1]);

        AppLayout {
            header,
            sidebar: horizontal[0],
            content: horizontal[1],
            status_bar,
        }
    }
}
