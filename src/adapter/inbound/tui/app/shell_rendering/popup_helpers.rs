// Session popup helpers are test-only renderers that pin the list/detail panel contract without
// pulling the full production popup frame into every rendering assertion.
#[cfg(test)]
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};

#[cfg(test)]
use super::super::shell_presentation::OverlayListView;
#[cfg(test)]
use super::super::{AkraTheme, Frame, Line, NativeTuiApp, Rect};

#[cfg(test)]
#[allow(dead_code)]
// The list panel has two renderer modes behind the same "Threads" chrome:
// empty/loading/error message copy, or a stateful selectable session list.
pub(super) fn draw_session_list_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut NativeTuiApp,
    // `OverlayListView` is already presentation-owned; this helper does not inspect session/domain data.
    list_view: OverlayListView,
) {
    // Message lines represent "no rows to select" states, so rendering a List would leak stale selection highlights.
    if let Some(message_lines) = list_view.message_lines {
        let widget = Paragraph::new(message_lines)
            .block(AkraTheme::panel_block("Threads"))
            .wrap(Wrap { trim: true });
        frame.render_widget(widget, area);
        return;
    }

    // Row text and wrapping policy are already in the view model; renderer only gives those rows list semantics.
    let list = List::new(
        list_view
            .items
            .into_iter()
            .map(|item| ListItem::new(item.lines)),
    )
    .block(AkraTheme::panel_block("Threads"))
    .highlight_style(AkraTheme::selected())
    .highlight_symbol(AkraTheme::list_highlight_symbol());

    // Ratatui stores list selection outside the widget. Sync the presentation selection into app-owned
    // ListState just before drawing so tests exercise the same stateful-widget contract as production.
    app.session_overlay_ui_state
        .sync_selected_session(list_view.selected_index);
    frame.render_stateful_widget(list, area, &mut app.session_overlay_ui_state.list_state);
}

#[cfg(test)]
#[allow(dead_code)]
// The detail panel is stateless because selection has already been resolved by presentation into concrete lines.
pub(super) fn draw_session_detail_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    // Lines may contain paths or aligned metadata, so this panel preserves leading whitespace.
    lines: Vec<Line<'static>>,
) {
    let detail = Paragraph::new(lines)
        .block(AkraTheme::panel_block("Selected Session"))
        .wrap(Wrap { trim: false });
    frame.render_widget(detail, area);
}
