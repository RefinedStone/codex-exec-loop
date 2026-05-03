use super::super::super::super::super::planning_draft_editor_ui::PlanningDraftEditorBufferState;
use super::super::super::super::Line;
use super::projection_lines::build_planning_draft_editor_file_lines;

// Draft editor projection은 mutable editor state와 ratatui widget 사이의 read-only boundary다.
// renderer에는 buffer object를 넘기지 않고, 같은 selected buffer에서 계산한 파일 목록, 본문, scroll, cursor 좌표만 넘긴다.
pub(super) struct PlanningDraftEditorProjection {
    // file list는 전체 draft artifact set과 현재 선택을 보여 주는 왼쪽 navigation read model이다.
    pub(super) file_lines: Vec<Line<'static>>,
    // editor_title은 오른쪽 panel이 어떤 active/staged file pair를 표시하는지 식별한다.
    pub(super) editor_title: String,
    // editor_lines는 selected buffer text를 renderer 소유 line DTO로 복사한 snapshot이다.
    pub(super) editor_lines: Vec<Line<'static>>,
    // editor_scroll은 current viewport height 기준으로 clamp된 Paragraph scroll row다.
    pub(super) editor_scroll: u16,
    // editor_cursor_offset은 full-file cursor 좌표를 visible editor area 기준 좌표로 낮춘 값이다.
    pub(super) editor_cursor_offset: Option<(u16, u16)>,
}

// 이 함수가 manual planning editor의 geometry projection을 한곳에 묶는다.
// surface builder는 session/runtime 의미를 조립하고, 여기서는 selected buffer 기준의 visual state만 확정한다.
pub(super) fn build_planning_draft_editor_projection(
    // 전체 buffer set은 왼쪽 file list를 만들기 위한 source이고, selected buffer와 같은 revision에서 온 값이어야 한다.
    buffers: &[PlanningDraftEditorBufferState],
    // selected_index는 file list 강조와 selected_buffer title/body가 같은 artifact를 가리키게 하는 join key다.
    selected_index: usize,
    // selected_buffer는 오른쪽 editor panel의 단일 source이며 title, body, scroll, cursor를 모두 제공한다.
    selected_buffer: &PlanningDraftEditorBufferState,
    // renderer가 계산한 body height를 받아 scroll clamp와 cursor viewport 좌표를 같은 기준으로 맞춘다.
    editor_height: u16,
) -> PlanningDraftEditorProjection {
    // file list styling은 별도 helper에 맡겨 navigation copy와 editor body geometry가 섞이지 않게 한다.
    let file_lines = build_planning_draft_editor_file_lines(buffers, selected_index);

    // buffer text를 owned Line snapshot으로 복사해 renderer lifetime이 mutable editor state에 묶이지 않게 한다.
    let editor_lines = selected_buffer
        .lines()
        .iter()
        .map(|line| Line::from(line.clone()))
        .collect::<Vec<_>>();
    // zero-height layout은 모든 줄을 overflow로 만드는 특이점이므로 projection boundary에서 1줄 viewport로 포화시킨다.
    let editor_height = editor_height.max(1) as usize;
    // max scroll은 마지막 줄이 viewport 안에 남는 최대 row다. 짧은 파일은 0으로 포화하고 ratatui 좌표 폭에 맞춘다.
    let max_editor_scroll = selected_buffer
        .lines()
        .len()
        .saturating_sub(editor_height)
        .min(u16::MAX as usize) as u16;
    // 저장된 scroll은 파일 내용이나 popup height 변화 뒤 stale해질 수 있어 frame projection마다 다시 clamp한다.
    let editor_scroll = selected_buffer.editor_scroll().min(max_editor_scroll);
    // cursor는 full-file 좌표로 저장되므로 scroll을 뺀 visible-row offset으로 변환해 renderer에 넘긴다.
    let editor_cursor_offset = Some((
        selected_buffer.cursor_column().min(u16::MAX as usize) as u16,
        selected_buffer
            .cursor_line_index()
            // cursor가 scroll 위로 밀린 stale 상태여도 widget 좌표 underflow를 만들지 않는다.
            .saturating_sub(editor_scroll as usize)
            // ratatui cursor API의 u16 좌표 폭에 맞춰 과도한 y offset을 포화시킨다.
            .min(u16::MAX as usize) as u16,
    ));

    // 이 경계 뒤 renderer는 widget 배치만 담당하고 buffer 해석, scroll 보정, cursor 보정을 반복하지 않는다.
    PlanningDraftEditorProjection {
        file_lines,
        editor_title: selected_buffer.file_label(),
        editor_lines,
        editor_scroll,
        editor_cursor_offset,
    }
}
