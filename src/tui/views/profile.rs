use crate::app::AppState;
use crate::theme::Theme;
use crate::tui::widgets::vaporwave_block;
use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

pub fn render(frame: &mut Frame, area: ratatui::layout::Rect, state: &AppState) {
    let block = vaporwave_block("Profile");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if !state.is_authenticated {
        let para = Paragraph::new(Span::styled("Login required to view profile", Theme::dim()));
        frame.render_widget(para, inner);
        return;
    }

    let Some(user) = &state.user else {
        let para = Paragraph::new(Span::styled("Loading profile...", Theme::dim()));
        frame.render_widget(para, inner);
        return;
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("Username:    ", Theme::dim()),
            Span::styled(&user.username, Theme::pink_bold()),
        ]),
        Line::from(vec![
            Span::styled("Email:       ", Theme::dim()),
            Span::styled(user.email.as_deref().unwrap_or("\u{2014}"), Theme::text()),
        ]),
        Line::from(vec![
            Span::styled("Member Since: ", Theme::dim()),
            Span::styled(user.member_since(), Theme::lavender()),
        ]),
        Line::from(""),
        Line::from(Span::styled("Stats", Theme::cyan_bold())),
        Line::from(vec![
            Span::styled("Reactions Sent: ", Theme::dim()),
            Span::styled(
                state
                    .user_stats
                    .as_ref()
                    .map(|s| s.reactions.to_string())
                    .unwrap_or_default(),
                Theme::text(),
            ),
        ]),
        Line::from(vec![
            Span::styled("Favorites:     ", Theme::dim()),
            Span::styled(
                state
                    .user_stats
                    .as_ref()
                    .map(|s| s.favorites.to_string())
                    .unwrap_or_default(),
                Theme::text(),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled("[L] Logout", Theme::dim())),
    ];

    let para = Paragraph::new(lines);
    frame.render_widget(para, inner);
}
