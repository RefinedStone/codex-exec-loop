// session popup helper는 test-only renderer다.
// 전체 production popup frame을 모든 rendering assertion에 끌어오지 않고 list/detail panel contract만 고정한다.
#[cfg(test)]
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};

#[cfg(test)]
use super::super::shell_presentation::OverlayListView;
#[cfg(test)]
use super::super::{AkraTheme, Frame, Line, NativeTuiApp, Rect};

#[cfg(test)]
#[allow(dead_code)]
// list panel은 같은 "Threads" chrome 뒤에 두 renderer mode를 둔다.
// empty/loading/error message copy와 stateful selectable session list를 같은 panel boundary에서 비교한다.
pub(super) fn draw_session_list_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut NativeTuiApp,
    // `OverlayListView`는 이미 presentation 소유이므로 이 helper는 session/domain data를 다시 들여다보지 않는다.
    list_view: OverlayListView,
) {
    // message line은 "선택할 row 없음" state를 뜻하므로 List로 render하면 stale selection highlight가 새어 나간다.
    if let Some(message_lines) = list_view.message_lines {
        let widget = Paragraph::new(message_lines)
            .block(AkraTheme::panel_block("Threads"))
            .wrap(Wrap { trim: true });
        frame.render_widget(widget, area);
        return;
    }

    // row text와 wrapping policy는 이미 view model 안에 있으므로 renderer는 row에 list semantics만 부여한다.
    let list = List::new(
        list_view
            .items
            .into_iter()
            .map(|item| ListItem::new(item.lines)),
    )
    .block(AkraTheme::panel_block("Threads"))
    .highlight_style(AkraTheme::selected())
    .highlight_symbol(AkraTheme::list_highlight_symbol());

    // ratatui는 list selection을 widget 밖에 저장한다.
    // draw 직전에 presentation selection을 app 소유 ListState로 맞춰 test가 production과 같은 stateful-widget contract를 검증하게 한다.
    app.session_overlay_ui_state
        .sync_selected_session(list_view.selected_index);
    frame.render_stateful_widget(list, area, &mut app.session_overlay_ui_state.list_state);
}

#[cfg(test)]
#[allow(dead_code)]
// detail panel은 selection이 이미 presentation에서 concrete line으로 resolve된 뒤라 stateless로 남는다.
pub(super) fn draw_session_detail_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    // line에는 path나 정렬된 metadata가 들어갈 수 있으므로 이 panel은 leading whitespace를 보존한다.
    lines: Vec<Line<'static>>,
) {
    let detail = Paragraph::new(lines)
        .block(AkraTheme::panel_block("Selected Session"))
        .wrap(Wrap { trim: false });
    frame.render_widget(detail, area);
}
