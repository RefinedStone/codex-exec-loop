#[cfg(test)]
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};

#[cfg(test)]
use super::super::shell_presentation::OverlayListView;
#[cfg(test)]
use super::super::{AkraTheme, Frame, Line, NativeTuiApp, Rect};

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
            .block(AkraTheme::panel_block("Threads"))
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
    .block(AkraTheme::panel_block("Threads"))
    .highlight_style(AkraTheme::selected())
    .highlight_symbol(AkraTheme::list_highlight_symbol());

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
        .block(AkraTheme::panel_block("Selected Session"))
        .wrap(Wrap { trim: false });
    frame.render_widget(detail, area);
}
