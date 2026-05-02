// 학습 주석: layout helper가 `Rc<[Rect]>`를 반환해 draw 단계와 inspection 단계이 같은 Rect slice를
// 복사 없이 공유합니다. 렌더링 중 layout은 불변이어야 하므로 Rc shared ownership만 있으면 충분합니다.
use std::rc::Rc;

// 학습 주석: shell_presentation은 app state를 Line 기반 view로 접고, 이 파일은 그 view를 실제 frame에
// 배치합니다. tail view와 live transcript line은 inline rendering의 두 주요 입력입니다.
use super::shell_presentation::{build_inline_live_transcript_lines, build_inline_tail_view};
// 학습 주석: rendering entrypoint는 app module의 공통 ratatui 타입, overlay state, theme helpers를 폭넓게 씁니다.
use super::*;

// 학습 주석: inline inspection은 popup frame이 아니라 main buffer 안에서 overlay 내용을 보여 주는 하위 renderer입니다.
#[path = "shell_rendering/inline_inspection.rs"]
mod inline_inspection;
// 학습 주석: inline layout은 transcript/tail/inspection 영역을 나누는 책임을 분리한 모듈입니다.
#[path = "shell_rendering/inline_layout.rs"]
mod inline_layout;
// 학습 주석: popup_frame은 inline flow 위에 그려지는 modal 성격의 frame renderer를 담습니다.
#[path = "shell_rendering/popup_frame.rs"]
mod popup_frame;
// 학습 주석: popup_helpers는 modal Rect 계산과 공통 drawing helper를 제공하는 보조 모듈입니다.
#[path = "shell_rendering/popup_helpers.rs"]
mod popup_helpers;

#[cfg(test)]
// 학습 주석: contract test가 planning draft editor overlay view를 직접 구성해 렌더링 좌표와 copy를 검증합니다.
use super::shell_presentation::build_planning_draft_editor_overlay_view;
// 학습 주석: inline inspection renderer는 overlay가 열렸을 때 layout[0] 영역에 내용을 그립니다.
use inline_inspection::draw_inline_shell_inspection;
#[cfg(test)]
// 학습 주석: centered_rect는 popup layout contract test에서만 직접 필요합니다.
use inline_layout::centered_rect;
// 학습 주석: inline_layout의 public helpers는 frame 전체를 inline terminal flow로 그리고, cursor를
// prompt tail 안에만 배치하도록 보장합니다.
use inline_layout::{
    build_inline_terminal_flow_layout, inline_body_render_area, render_inline_body,
    set_cursor_if_visible,
};
// 학습 주석: exit confirmation은 inline inspection과 별개의 modal layer로 마지막에 그려집니다.
use popup_frame::draw_exit_confirmation;

pub(super) fn prepare_render_state(app: &mut NativeTuiApp, mode: ShellFrontendMode, area: Rect) {
    // 학습 주석: 현재 prepare 단계는 frontend mode별 분기가 없지만, draw와 같은 signature를 유지해
    // future backend/frontend mode 차이를 같은 entrypoint에서 처리할 수 있게 둡니다.
    let _ = mode;
    // 학습 주석: directions manual editor가 열려 있으면 render 직전 scroll sync가 필요합니다.
    let directions_editor_open = app.shell_overlay == ShellOverlay::DirectionsMaintenance
        && app.directions_maintenance_overlay_ui_state.step()
            == DirectionsMaintenanceOverlayStep::ManualEditor;
    // 학습 주석: planning init manual editor도 같은 editor scroll model을 사용합니다.
    let planning_editor_open = app.shell_overlay == ShellOverlay::PlanningInit
        && app.planning_init_overlay_ui_state.step() == PlanningInitOverlayStep::ManualEditor;
    // 학습 주석: manual editor가 아니면 scroll sync가 필요 없습니다. 일반 render path에서 editor
    // state를 건드리지 않아 커서/스크롤 상태가 불필요하게 변하지 않게 합니다.
    if !directions_editor_open && !planning_editor_open {
        return;
    }

    // 학습 주석: editor가 실제로 들어갈 inspection 영역은 tail 높이에 따라 달라지므로 tail view를 먼저 계산합니다.
    let tail_view = build_inline_tail_view(app, area.width);
    // 학습 주석: layout[0]은 overlay/inspection body 영역입니다. 여기에서 editor viewport 높이를 역산합니다.
    let inspection_area = build_inline_terminal_flow_layout(app, area, &tail_view.lines)[0];
    // 학습 주석: editor 주변 chrome, header, validation/status copy가 차지하는 대략 14줄을 빼고
    // 실제 text area 높이를 구합니다. 작은 viewport에서도 최소 1줄은 보장합니다.
    let editor_content_height = inspection_area
        .height
        .saturating_sub(14)
        .max(6)
        .saturating_sub(1)
        .max(1);
    // 학습 주석: render 직전 scroll sync를 수행해야 cursor가 visible area 밖으로 밀려난 상태로 그려지지 않습니다.
    app.planning_draft_editor_ui_state
        .sync_editor_scroll(editor_content_height);
}

pub(super) fn draw(frame: &mut Frame<'_>, app: &mut NativeTuiApp, mode: ShellFrontendMode) {
    // 학습 주석: 현재 inline renderer만 있으므로 mode는 아직 선택에 쓰지 않습니다. entrypoint signature는
    // frontend abstraction과 맞춰 둡니다.
    let _ = mode;
    // 학습 주석: frame 전체 영역을 기준으로 inline terminal flow를 계산합니다.
    let frame_area = frame.area();
    // 학습 주석: tail view는 prompt, status tail, cursor offset을 포함한 하단 anchored view입니다.
    let tail_view = build_inline_tail_view(app, frame_area.width);
    // 학습 주석: live transcript는 hidden overlay 상태에서 tail 위쪽에 이어 붙일 streaming/history lines입니다.
    let live_transcript_lines = build_inline_live_transcript_lines(app);
    // 학습 주석: layout[0]은 inspection 영역, layout[1]은 tail/inline body가 들어가는 영역입니다.
    let layout = build_inline_terminal_flow_layout(app, frame_area, &tail_view.lines);

    draw_inline_conversation_shell(frame, app, tail_view, live_transcript_lines, &layout);

    // 학습 주석: overlay가 열려 있으면 inline shell을 먼저 그린 뒤 inspection area 위에 overlay 내용을 그립니다.
    if app.shell_overlay != ShellOverlay::Hidden {
        draw_inline_shell_inspection(frame, app, layout[0]);
    }

    // 학습 주석: exit confirmation은 가장 마지막 modal layer라 다른 inline overlay보다 위에 떠야 합니다.
    if app.is_exit_confirmation_visible() {
        draw_exit_confirmation(frame);
    }
}

fn draw_inline_conversation_shell(
    // 학습 주석: ratatui frame입니다.
    frame: &mut Frame<'_>,
    // 학습 주석: overlay visibility와 exit confirmation 여부를 읽기 위한 app state입니다.
    app: &mut NativeTuiApp,
    // 학습 주석: prompt tail과 cursor offset을 포함하는 하단 view입니다.
    tail_view: super::shell_presentation::InlineTailView,
    // 학습 주석: overlay가 숨겨졌을 때 tail 위에 그릴 live transcript lines입니다.
    live_transcript_lines: Vec<Line<'static>>,
    // 학습 주석: draw entrypoint에서 계산한 inline terminal layout입니다.
    layout: &Rc<[Rect]>,
) {
    // 학습 주석: 매 frame 전체를 지워 이전 overlay나 stale tail row가 남지 않게 합니다.
    let frame_area = frame.area();
    frame.render_widget(Clear, frame_area);
    // 학습 주석: overlay도 exit modal도 없으면 가장 단순한 inline conversation mode입니다.
    // 이때는 host scrollback과 prompt anchoring을 위해 tail positioning을 직접 처리합니다.
    if app.shell_overlay == ShellOverlay::Hidden && !app.is_exit_confirmation_visible() {
        // 학습 주석: startup banner나 inspection replacement처럼 top부터 전체 body를 쓰는 view는
        // live transcript split 없이 frame 전체에 tail lines를 그립니다.
        if tail_view.render_from_top {
            render_inline_body(frame, frame_area, tail_view.lines, false);
            set_cursor_if_visible(frame, frame_area, tail_view.prompt_cursor_offset);
            return;
        }

        // 학습 주석: 일반 conversation mode에서는 prompt tail 높이를 먼저 계산하고, 그 위쪽 빈 공간에
        // live transcript를 그려 tail이 항상 하단에 붙도록 합니다.
        let tail_area = inline_body_render_area(frame_area, &tail_view.lines);
        render_inline_live_transcript(frame, frame_area, tail_area, live_transcript_lines);
        render_inline_body(frame, tail_area, tail_view.lines, false);
        set_cursor_if_visible(frame, tail_area, tail_view.prompt_cursor_offset);
        return;
    }

    // 학습 주석: overlay나 exit modal이 있으면 layout[1]에 tail을 그리고, layout[0]은 overlay/inspection이
    // 사용할 수 있게 비워 둡니다. exit modal은 이 함수 밖에서 마지막에 덮어씁니다.
    render_inline_body(
        frame,
        inline_body_render_area(layout[1], &tail_view.lines),
        tail_view.lines,
        false,
    );
}

fn render_inline_live_transcript(
    // 학습 주석: transcript body를 그릴 frame입니다.
    frame: &mut Frame<'_>,
    // 학습 주석: 전체 inline shell 영역입니다.
    frame_area: Rect,
    // 학습 주석: prompt tail이 차지하는 영역입니다. transcript는 이 영역 위쪽에만 그려야 합니다.
    tail_area: Rect,
    // 학습 주석: rendering할 live transcript lines입니다.
    live_transcript_lines: Vec<Line<'static>>,
) {
    // 학습 주석: transcript line이 없거나 tail이 frame 상단까지 올라온 경우, transcript가 들어갈
    // vertical space가 없으므로 그리지 않습니다.
    if live_transcript_lines.is_empty() || tail_area.y <= frame_area.y {
        return;
    }

    // 학습 주석: live_container는 frame top부터 tail 직전까지의 영역입니다. tail이 하단을 점유하므로
    // transcript는 이 container 안에서만 scroll/clip 됩니다.
    let live_container = Rect::new(
        frame_area.x,
        frame_area.y,
        frame_area.width,
        tail_area.y.saturating_sub(frame_area.y),
    );
    // 학습 주석: transcript line 수에 맞춰 container 내부 실제 render area를 하단 정렬로 계산합니다.
    let live_area = inline_body_render_area(live_container, &live_transcript_lines);
    render_inline_body(frame, live_area, live_transcript_lines, false);
}

#[cfg(test)]
// 학습 주석: contract tests는 overlay, inline tail, viewport replay 같은 renderer behavioral 계약을 검증합니다.
#[path = "shell_rendering_contract_tests.rs"]
mod contract_tests;
#[cfg(test)]
// 학습 주석: snapshot-oriented rendering tests는 ready/streaming/queue/planning shell output을 고정합니다.
#[path = "shell_rendering_tests.rs"]
mod tests;
