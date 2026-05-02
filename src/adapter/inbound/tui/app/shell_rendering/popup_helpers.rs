// 학습 주석: 이 파일의 세션 popup helper는 현재 렌더링 테스트에서만 컴파일됩니다. production popup frame은 같은 개념을 다른
// 경로에서 그리지만, 테스트는 이 helper를 통해 list/detail 패널 계약을 직접 검증할 수 있습니다.
#[cfg(test)]
// 학습 주석: List/ListItem은 세션 목록 선택 상태를 가진 widget이고, Paragraph/Wrap은 메시지나 상세 본문처럼 선택 상태가 없는
// 텍스트 패널을 그릴 때 사용합니다.
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};

// 학습 주석: OverlayListView는 shell presentation 계층이 만든 세션 목록 view model입니다. rendering helper는 domain/session 구조를
// 모르고 이미 line으로 변환된 view model만 소비합니다.
#[cfg(test)]
use super::super::shell_presentation::OverlayListView;
// 학습 주석: AkraTheme, Frame, Line, NativeTuiApp, Rect는 TUI adapter의 렌더링 표면입니다. helper는 shared theme token과 app의
// list state를 함께 써서 popup family가 같은 chrome을 유지하게 합니다.
#[cfg(test)]
use super::super::{AkraTheme, Frame, Line, NativeTuiApp, Rect};

// 학습 주석: 테스트 빌드에서만 필요한 drawing helper입니다. dead_code 허용은 특정 테스트 시나리오가 detail/list helper 중 하나만
// 참조해도 컴파일 경고가 실패로 번지지 않게 합니다.
#[cfg(test)]
#[allow(dead_code)]
// 학습 주석: draw_session_list_panel은 세션 overlay의 왼쪽 목록 영역을 그립니다. view model이 "목록 대신 메시지를 보여라"라고
// 말하는 상태와 실제 세션 item이 있는 상태를 같은 panel title/chrome 안에서 분기합니다.
pub(super) fn draw_session_list_panel(
    // 학습 주석: frame은 ratatui가 이번 tick에 그릴 surface입니다. helper는 widget을 만들고 이 frame에 바로 render합니다.
    frame: &mut Frame<'_>,
    // 학습 주석: area는 popup frame 안에서 목록 패널에 배정된 사각형입니다. helper는 layout을 다시 계산하지 않고 이 영역만 채웁니다.
    area: Rect,
    // 학습 주석: app은 session_overlay_ui_state를 들고 있습니다. stateful List를 그리려면 선택 index를 app의 list_state와 동기화해야 합니다.
    app: &mut NativeTuiApp,
    // 학습 주석: list_view는 presentation 계층이 만든 세션 목록 표시 모델입니다. message_lines가 있으면 items보다 우선합니다.
    list_view: OverlayListView,
) {
    // 학습 주석: message_lines는 "세션 없음", "불러오는 중", "오류" 같은 목록 대체 상태입니다. 이 경우 List widget을 만들면
    // 선택 상태가 의미 없으므로 Paragraph로 panel 전체를 채웁니다.
    if let Some(message_lines) = list_view.message_lines {
        // 학습 주석: Threads block은 실제 목록 상태와 메시지 상태가 같은 panel chrome을 쓰게 합니다. trim=true는 안내 문구가
        // 좁은 영역에서 줄바꿈될 때 불필요한 leading whitespace를 줄입니다.
        let widget = Paragraph::new(message_lines)
            .block(AkraTheme::panel_block("Threads"))
            .wrap(Wrap { trim: true });
        frame.render_widget(widget, area);
        // 학습 주석: 메시지 패널을 그린 뒤에는 stateful list를 렌더링하지 않습니다. 이 조기 반환이 빈 목록 상태에서 이전 선택
        // highlight가 남는 일을 막습니다.
        return;
    }

    // 학습 주석: 실제 세션 항목이 있을 때는 각 item의 preformatted lines를 ListItem으로 감쌉니다. text wrapping/formatting은
    // presentation layer가 이미 끝냈고, rendering layer는 선택 가능한 list 구조만 부여합니다.
    let list = List::new(
        list_view
            .items
            .into_iter()
            .map(|item| ListItem::new(item.lines)),
    )
    // 학습 주석: panel_block/highlight_style/highlight_symbol은 queue/session/planning overlay가 공유하는 AKRA chrome token입니다.
    .block(AkraTheme::panel_block("Threads"))
    .highlight_style(AkraTheme::selected())
    .highlight_symbol(AkraTheme::list_highlight_symbol());

    // 학습 주석: ratatui List는 선택 상태를 외부 ListState에 저장합니다. view model의 selected_index를 app state에 먼저 반영해야
    // render_stateful_widget이 현재 선택을 올바르게 강조합니다.
    app.session_overlay_ui_state
        .sync_selected_session(list_view.selected_index);
    frame.render_stateful_widget(list, area, &mut app.session_overlay_ui_state.list_state);
}

// 학습 주석: detail panel helper도 테스트 빌드에서만 필요합니다. list panel과 짝을 이루어 오른쪽 선택 세션 세부 정보를 그립니다.
#[cfg(test)]
#[allow(dead_code)]
// 학습 주석: draw_session_detail_panel은 선택된 세션의 metadata/detail line을 Paragraph로 렌더링합니다. 목록과 달리 선택 상태가
// 없으므로 stateless widget으로 충분합니다.
pub(super) fn draw_session_detail_panel(
    // 학습 주석: frame은 popup 내부 detail panel을 실제 terminal buffer에 그리는 대상입니다.
    frame: &mut Frame<'_>,
    // 학습 주석: area는 detail panel에 이미 배정된 영역입니다. list/detail split 비율은 상위 popup frame이 결정합니다.
    area: Rect,
    // 학습 주석: lines는 presentation layer가 선택 세션을 사람이 읽을 수 있는 Line 목록으로 바꾼 결과입니다.
    lines: Vec<Line<'static>>,
) {
    // 학습 주석: detail 본문은 사용자가 복사하거나 비교할 수 있는 세션 경로/상태/시간 정보를 포함할 수 있어 trim=false를 유지합니다.
    // 줄 앞 공백이 의미 있는 경우를 rendering layer가 임의로 제거하지 않게 하기 위한 선택입니다.
    let detail = Paragraph::new(lines)
        .block(AkraTheme::panel_block("Selected Session"))
        .wrap(Wrap { trim: false });
    frame.render_widget(detail, area);
}
