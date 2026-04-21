#[cfg(test)]
use ratatui::style::{Color, Modifier, Style};
#[cfg(test)]
use ratatui::widgets::{Block, Borders};
#[cfg(test)]
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};

#[cfg(test)]
use super::super::shell_presentation::OverlayListView;
#[cfg(test)]
use super::super::{Frame, Line, NativeTuiApp, Rect};

#[cfg(test)]
#[allow(dead_code)]
pub(super) fn draw_session_list_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut NativeTuiApp,
    list_view: OverlayListView,
) {
    if let Some(message_lines) = list_view.message_lines {
        let widget = Paragraph::new(message_lines)
            .block(Block::default().borders(Borders::ALL).title("Threads"))
            .wrap(Wrap { trim: true });
        frame.render_widget(widget, area);
        return;
    }

    let list = List::new(
        list_view
            .items
            .into_iter()
            .map(|item| ListItem::new(item.lines)),
    )
    .block(Block::default().borders(Borders::ALL).title("Threads"))
    .highlight_style(
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol(">> ");

    app.session_overlay_ui_state
        .sync_selected_session(list_view.selected_index);
    frame.render_stateful_widget(list, area, &mut app.session_overlay_ui_state.list_state);
}

#[cfg(test)]
#[allow(dead_code)]
pub(super) fn draw_session_detail_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    lines: Vec<Line<'static>>,
) {
    let detail = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Selected Session"),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(detail, area);
}

#[cfg(test)]
#[allow(dead_code)]
pub(super) fn draw_automation_list_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut NativeTuiApp,
    list_view: OverlayListView,
) {
    if let Some(message_lines) = list_view.message_lines {
        let widget = Paragraph::new(message_lines)
            .block(Block::default().borders(Borders::ALL).title("Automation"))
            .wrap(Wrap { trim: true });
        frame.render_widget(widget, area);
        return;
    }

    let list = List::new(
        list_view
            .items
            .into_iter()
            .map(|item| ListItem::new(item.lines)),
    )
    .block(Block::default().borders(Borders::ALL).title("Automation"))
    .highlight_style(
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol(">> ");

    app.followup_overlay_ui_state
        .list_state
        .select(list_view.selected_index);
    frame.render_stateful_widget(list, area, &mut app.followup_overlay_ui_state.list_state);
}
