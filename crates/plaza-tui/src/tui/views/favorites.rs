use crate::app::AppState;
use crate::theme::Theme;
use crate::tui::widgets::vaporwave_block;
use ratatui::{
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, area: ratatui::layout::Rect, state: &AppState) {
    let block = vaporwave_block("Favorites");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if !state.is_authenticated {
        let para = Paragraph::new(Span::styled(
            "Login required to view favorites",
            Theme::dim(),
        ));
        frame.render_widget(para, inner);
        return;
    }

    if state.favorites.items.is_empty() {
        let para = Paragraph::new(Span::styled("No favorites yet", Theme::dim()));
        frame.render_widget(para, inner);
        return;
    }

    let items: Vec<ListItem> = state
        .favorites
        .items
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let style = if i == state.favorites.selected {
                Theme::selected()
            } else {
                Theme::text()
            };
            let text = format!(
                "{} \u{2014} {}  ({})",
                entry.song.artist,
                entry.song.title,
                entry.song.duration_display()
            );
            ListItem::new(Line::from(Span::styled(text, style)))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(state.favorites.selected));

    let list = List::new(items);
    frame.render_stateful_widget(list, inner, &mut list_state);
}
