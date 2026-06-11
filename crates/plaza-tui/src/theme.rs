use ratatui::style::{Color, Modifier, Style};

pub struct Theme;

impl Theme {
    pub const BACKGROUND: Color = Color::Rgb(10, 5, 25);
    pub const PINK: Color = Color::Rgb(255, 0, 128);
    pub const CYAN: Color = Color::Rgb(0, 255, 255);
    pub const PURPLE: Color = Color::Rgb(153, 0, 255);
    pub const LAVENDER: Color = Color::Rgb(180, 130, 255);
    pub const TEXT: Color = Color::Rgb(220, 210, 255);
    pub const DIM: Color = Color::Rgb(80, 70, 120);
    pub const GREEN: Color = Color::Rgb(0, 255, 128);
    pub const RED: Color = Color::Rgb(255, 50, 50);
    pub const YELLOW: Color = Color::Rgb(255, 220, 50);

    pub fn title() -> Style {
        Style::default().fg(Self::PINK).add_modifier(Modifier::BOLD)
    }

    pub fn border() -> Style {
        Style::default().fg(Self::CYAN)
    }

    pub fn selected() -> Style {
        Style::default()
            .fg(Self::BACKGROUND)
            .bg(Self::PINK)
            .add_modifier(Modifier::BOLD)
    }

    pub fn dim() -> Style {
        Style::default().fg(Self::DIM)
    }

    pub fn highlight() -> Style {
        Style::default().fg(Self::CYAN).add_modifier(Modifier::BOLD)
    }

    pub fn text() -> Style {
        Style::default().fg(Self::TEXT)
    }

    pub fn pink_bold() -> Style {
        Style::default().fg(Self::PINK).add_modifier(Modifier::BOLD)
    }

    pub fn cyan_bold() -> Style {
        Style::default().fg(Self::CYAN).add_modifier(Modifier::BOLD)
    }

    pub fn lavender() -> Style {
        Style::default().fg(Self::LAVENDER)
    }
}
