use plaza_api::models::RatingRange;

use crate::app::AppState;
use crate::theme::Theme;
use crate::tui::widgets::vaporwave_block;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, area: ratatui::layout::Rect, state: &AppState) {
    let block = vaporwave_block("Charts");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    // Tab bar
    let tabs = [
        RatingRange::Overtime,
        RatingRange::Weekly,
        RatingRange::Monthly,
    ];
    let tab_spans: Vec<Span> = tabs
        .iter()
        .map(|r| {
            if *r == state.chart_range {
                Span::styled(format!(" [{}] ", r.display()), Theme::selected())
            } else {
                Span::styled(format!("  {}  ", r.display()), Theme::dim())
            }
        })
        .collect();
    let tab_line = Line::from(tab_spans);
    frame.render_widget(Paragraph::new(tab_line), chunks[0]);

    // List
    if state.charts.items.is_empty() {
        let para = Paragraph::new(Span::styled("Loading...", Theme::dim()));
        frame.render_widget(para, chunks[1]);
        return;
    }

    let items: Vec<ListItem> = state
        .charts
        .items
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let style = if i == state.charts.selected {
                Theme::selected()
            } else {
                Theme::text()
            };
            let rank = entry.rank.unwrap_or(i as u32 + 1);
            let text = format!(
                "#{:3}  {} \u{2014} {}   \u{2665} {}",
                rank, entry.song.artist, entry.song.title, entry.likes
            );
            ListItem::new(Line::from(Span::styled(text, style)))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(state.charts.selected));

    let list = List::new(items);
    frame.render_stateful_widget(list, chunks[1], &mut list_state);
}
