// 학습 주석: 이 surface builder는 NativeTuiApp 전체를 읽지만 mutation은 하지 않습니다.
// app 안의 planning_draft_editor_ui_state를 최종 popup view DTO로 낮추는 presentation boundary입니다.
use super::super::super::super::NativeTuiApp;
// 학습 주석: PlanningDraftEditorOverlayView는 renderer가 header/file list/editor/status/keys 영역을 그릴 때 쓰는 최종 DTO입니다.
use super::super::PlanningDraftEditorOverlayView;
// 학습 주석: editor_copy helpers는 이미 해석된 상태를 실제 Line 묶음으로 바꿉니다.
// surface는 copy 문구를 직접 만들지 않고 header/status/key 영역별 builder에 위임합니다.
use super::editor_copy::{
    build_planning_draft_editor_header_lines, build_planning_draft_editor_key_lines,
    build_planning_draft_editor_status_lines,
};
// 학습 주석: status input builder는 session/runtime/validation 값을 status copy DTO로 모읍니다.
// 그 뒤 editor_copy가 copy DTO를 styled line으로 변환합니다.
use super::editor_inputs::build_planning_draft_editor_status_copy;
// 학습 주석: projection builder는 file list, editor text, scroll, cursor offset처럼 rendering geometry에 가까운 값을 계산합니다.
use super::projection::build_planning_draft_editor_projection;
// 학습 주석: runtime interpreter는 dirty/validation/confirmation state를 next action과 close risk로 해석합니다.
use super::runtime::interpret_planning_draft_editor_runtime_state;
// 학습 주석: session collector는 ui_state에서 draft 이름, buffers, selected buffer, validation report를 한 view snapshot으로 묶습니다.
use super::session::collect_planning_draft_editor_session_view;

// 학습 주석: build_planning_draft_editor_overlay_view_for_app는 app state를 읽어 manual planning draft editor popup을 구성합니다.
// session이 아직 열리지 않은 상태면 None을 반환해 caller가 popup을 그리지 않게 합니다.
pub(super) fn build_planning_draft_editor_overlay_view_for_app(
    // 학습 주석: app은 shell 전체 state container입니다. 여기서는 planning draft editor substate만 읽습니다.
    app: &NativeTuiApp,
    // 학습 주석: editor_height는 renderer layout이 계산한 editor panel 높이입니다. cursor/scroll projection clamp에 필요합니다.
    editor_height: u16,
) -> Option<PlanningDraftEditorOverlayView> {
    // 학습 주석: session view가 없다는 것은 draft editor가 열리지 않았거나 아직 session data가 준비되지 않았다는 뜻입니다.
    // `?`로 None을 즉시 반환해 빈/깨진 editor view를 만들지 않습니다.
    let session = collect_planning_draft_editor_session_view(&app.planning_draft_editor_ui_state)?;
    // 학습 주석: runtime_state는 raw UI flags를 사용자에게 보여 줄 next action, close risk, confirmation 상태로 해석한 값입니다.
    // dirty labels와 validation report를 함께 보아 저장/닫기 안내가 실제 risk와 맞게 표시됩니다.
    let runtime_state = interpret_planning_draft_editor_runtime_state(
        &app.planning_draft_editor_ui_state,
        &session.dirty_labels,
        session.validation_report,
    );
    // 학습 주석: projection은 visual geometry와 text buffer projection을 담당합니다.
    // surface builder는 이 결과를 최종 overlay field로 옮기기만 합니다.
    let projection = build_planning_draft_editor_projection(
        session.buffers,
        session.selected_index,
        session.selected_buffer,
        editor_height,
    );
    // 학습 주석: status copy는 draft/session metadata와 runtime decision을 한데 모읍니다.
    // selected_index는 zero-based지만 사용자에게는 1-based position으로 보여 주기 위해 +1 합니다.
    let status_copy = build_planning_draft_editor_status_copy(
        session.draft_name,
        session.selected_buffer.active_path(),
        session.selected_index + 1,
        session.buffers.len(),
        session.validation_report,
        session.selected_buffer.staged_path(),
        &session.dirty_labels,
        runtime_state.next_action,
        runtime_state.close_risk,
        runtime_state.confirmation_pending,
    );
    // 학습 주석: status_lines는 copy DTO를 TUI renderer가 그릴 Line들로 변환한 결과입니다.
    let status_lines = build_planning_draft_editor_status_lines(status_copy);

    // 학습 주석: 최종 DTO를 만들 때 각 하위 builder의 결과를 field별로 배치합니다.
    // 이 함수가 app state와 renderer contract 사이의 마지막 조립 지점입니다.
    Some(PlanningDraftEditorOverlayView {
        // 학습 주석: header는 draft directory를 포함해 사용자가 어떤 draft session을 편집 중인지 알려 줍니다.
        header_lines: build_planning_draft_editor_header_lines(session.draft_directory),
        // 학습 주석: file_lines는 좌측 file list panel에 들어가는 styled rows입니다.
        file_lines: projection.file_lines,
        // 학습 주석: editor_title은 현재 선택된 file label이며 editor panel title로 쓰입니다.
        editor_title: projection.editor_title,
        // 학습 주석: editor_lines는 selected buffer의 본문을 Line으로 낮춘 값입니다.
        editor_lines: projection.editor_lines,
        // 학습 주석: editor_scroll은 renderer Paragraph scroll에 전달되어 cursor 주변 viewport를 유지합니다.
        editor_scroll: projection.editor_scroll,
        // 학습 주석: editor_cursor_offset은 visible editor area 안에서 cursor를 어디에 둘지 알려 줍니다.
        editor_cursor_offset: projection.editor_cursor_offset,
        // 학습 주석: status_lines는 validation, dirty files, staged path, next action을 하단 status panel에 보여 줍니다.
        status_lines,
        // 학습 주석: key_lines는 close risk와 confirmation 상태에 따라 Ctrl+S/Esc 같은 조작 안내를 바꿉니다.
        key_lines: build_planning_draft_editor_key_lines(
            runtime_state.close_risk,
            runtime_state.confirmation_pending,
        ),
    })
}
