use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::Style,
    text::{Span, Text},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use crate::app::AppState;
use crate::theme::Theme;
use crate::tui::widgets::centered_rect;

const LOGO: &str = r#"
 NIGHTWAVE
  PLAZA"#;

pub fn render(frame: &mut Frame, area: ratatui::layout::Rect, state: &AppState) {
    // Background
    frame.render_widget(
        Block::default().style(Style::default().bg(Theme::BACKGROUND)),
        area,
    );

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(30),
            Constraint::Length(7),
            Constraint::Min(0),
        ])
        .split(area);

    // Logo
    let logo_para = Paragraph::new(Text::from(LOGO))
        .style(Style::default().fg(Theme::PINK))
        .alignment(Alignment::Center);
    frame.render_widget(logo_para, chunks[0]);

    // Form area
    let form_area = centered_rect(40, 100, chunks[1]);

    let form_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(form_area);

    // Username field
    let username_block = Block::default()
        .title(Span::styled(
            " Username ",
            if !state.login_focus_password {
                Theme::pink_bold()
            } else {
                Theme::dim()
            },
        ))
        .borders(Borders::ALL)
        .border_style(if !state.login_focus_password {
            Theme::border()
        } else {
            Theme::dim()
        })
        .style(Style::default().bg(Theme::BACKGROUND));
    let username_para =
        Paragraph::new(Span::styled(&state.login_username, Theme::text())).block(username_block);
    frame.render_widget(username_para, form_chunks[0]);

    // Password field
    let masked: String = "*".repeat(state.login_password.len());
    let password_block = Block::default()
        .title(Span::styled(
            " Password ",
            if state.login_focus_password {
                Theme::pink_bold()
            } else {
                Theme::dim()
            },
        ))
        .borders(Borders::ALL)
        .border_style(if state.login_focus_password {
            Theme::border()
        } else {
            Theme::dim()
        })
        .style(Style::default().bg(Theme::BACKGROUND));
    let password_para =
        Paragraph::new(Span::styled(&masked, Theme::text())).block(password_block);
    frame.render_widget(password_para, form_chunks[1]);

    // Hint / error
    let hint = if let Some(err) = &state.login_error {
        Span::styled(err.as_str(), Style::default().fg(Theme::RED))
    } else {
        Span::styled("[Tab] Switch field  [Enter] Login  [g] Guest Mode", Theme::dim())
    };
    let hint_para = Paragraph::new(hint).alignment(Alignment::Center);
    frame.render_widget(hint_para, form_chunks[2]);
}
