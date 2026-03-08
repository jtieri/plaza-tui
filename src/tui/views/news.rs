use ratatui::{
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph},
    Frame,
};
use crate::app::AppState;
use crate::theme::Theme;
use crate::tui::widgets::vaporwave_block;

/// Strip HTML tags and decode common HTML entities for display in TUI.
fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    // Decode common HTML entities
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("\r\n", "\n")
        .replace("\r", "\n")
        .trim()
        .to_string()
}

pub fn render(frame: &mut Frame, area: ratatui::layout::Rect, state: &AppState) {
    let block = vaporwave_block("News");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.news.items.is_empty() {
        let para = Paragraph::new(Span::styled("No news", Theme::dim()));
        frame.render_widget(para, inner);
        return;
    }

    let items: Vec<ListItem> = state
        .news
        .items
        .iter()
        .enumerate()
        .flat_map(|(i, item)| {
            let is_selected = i == state.news.selected;
            let header_style = if is_selected {
                Theme::highlight()
            } else {
                Theme::dim()
            };
            let text_style = if is_selected { Theme::text() } else { Theme::dim() };

            let header = format!(
                "{} by {}",
                item.created_at_display(),
                item.author.as_deref().unwrap_or("Plaza Staff")
            );

            vec![
                ListItem::new(Line::from(Span::styled(header, header_style))),
                ListItem::new(Line::from(Span::styled(strip_html(&item.text), text_style))),
                ListItem::new(Line::from("")),
            ]
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(state.news.selected * 3));

    let list = List::new(items);
    frame.render_stateful_widget(list, inner, &mut list_state);
}
