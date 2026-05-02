// 학습 주석: PlanningDraftEditorBufferState는 TUI planning draft editor가 파일별 text, cursor, scroll 상태를 보관하는 UI state입니다.
// projection 계층은 이 mutable UI state를 renderer가 바로 소비할 수 있는 불변 DTO로 바꿉니다.
use super::super::super::super::super::planning_draft_editor_ui::PlanningDraftEditorBufferState;
// 학습 주석: Line은 popup renderer가 그릴 styled text 단위입니다. projection은 raw String 라인을 Line으로 올려 renderer가 formatting을
// 다시 계산하지 않게 합니다.
use super::super::super::super::Line;
// 학습 주석: 파일 목록 라인은 별도 helper가 맡습니다. 이 파일은 좌측 파일 목록과 우측 editor body/cursor를 하나의 projection으로
// 합치는 책임만 가집니다.
use super::projection_lines::build_planning_draft_editor_file_lines;

// 학습 주석: PlanningDraftEditorProjection은 popup renderer가 manual planning editor를 그리는 데 필요한 모든 표시 데이터를 묶습니다.
// buffer state 자체를 넘기지 않는 이유는 renderer가 편집 상태를 변경하지 않고, 이미 계산된 line/scroll/cursor 값만 사용하게 하기 위해서입니다.
pub(super) struct PlanningDraftEditorProjection {
    // 학습 주석: file_lines는 좌측 파일 목록 패널에 들어갈 라인입니다. selected_index 강조는 build_planning_draft_editor_file_lines가 처리합니다.
    pub(super) file_lines: Vec<Line<'static>>,
    // 학습 주석: editor_title은 우측 editor panel title입니다. selected buffer의 file_label을 사용해 사용자가 현재 편집 파일을 알게 합니다.
    pub(super) editor_title: String,
    // 학습 주석: editor_lines는 선택된 buffer의 본문을 Line으로 바꾼 결과입니다. renderer는 여기서 다시 buffer를 읽지 않습니다.
    pub(super) editor_lines: Vec<Line<'static>>,
    // 학습 주석: editor_scroll은 panel 높이에 맞춰 clamp된 scroll row입니다. 상태에 저장된 scroll이 파일 길이보다 커져도 안전하게 렌더링됩니다.
    pub(super) editor_scroll: u16,
    // 학습 주석: editor_cursor_offset은 editor viewport 왼쪽 위 기준 cursor 위치입니다. scroll 보정이 끝난 값이라 renderer가 바로 사용할 수 있습니다.
    pub(super) editor_cursor_offset: Option<(u16, u16)>,
}

// 학습 주석: build_planning_draft_editor_projection은 편집 UI state를 popup renderer DTO로 투영하는 유일한 경로입니다. 좌측 파일 목록,
// 우측 editor line, scroll clamp, cursor viewport 보정을 같은 selected buffer 기준으로 계산해 패널 간 상태 불일치를 막습니다.
pub(super) fn build_planning_draft_editor_projection(
    // 학습 주석: buffers는 draft editor가 관리하는 모든 파일 buffer입니다. 좌측 파일 목록을 만들 때 전체 목록이 필요합니다.
    buffers: &[PlanningDraftEditorBufferState],
    // 학습 주석: selected_index는 파일 목록에서 어떤 항목을 강조할지 정하는 index입니다. selected_buffer와 같은 항목을 가리켜야 합니다.
    selected_index: usize,
    // 학습 주석: selected_buffer는 우측 editor에 실제로 표시할 파일 상태입니다. 이 값에서 title, lines, scroll, cursor를 모두 가져옵니다.
    selected_buffer: &PlanningDraftEditorBufferState,
    // 학습 주석: editor_height는 renderer가 우측 editor body에 줄 수 있는 높이입니다. scroll clamp 계산의 기준입니다.
    editor_height: u16,
) -> PlanningDraftEditorProjection {
    // 학습 주석: 파일 목록은 전체 buffers와 selected_index를 기준으로 먼저 만듭니다. editor body 계산과 독립된 helper라 목록 표시 규칙이
    // body/cursor projection에 섞이지 않습니다.
    let file_lines = build_planning_draft_editor_file_lines(buffers, selected_index);

    // 학습 주석: 선택 buffer의 raw text lines를 ratatui Line으로 변환합니다. clone은 renderer가 static line vector를 소유하게 하려는
    // 의도이고, 편집 buffer lifetime에 renderer DTO가 묶이지 않게 합니다.
    let editor_lines = selected_buffer
        .lines()
        .iter()
        .map(|line| Line::from(line.clone()))
        .collect::<Vec<_>>();
    // 학습 주석: editor_height 0은 scroll 계산에서 모든 줄이 overflow처럼 보이는 문제를 만들 수 있어 최소 1로 올립니다.
    let editor_height = editor_height.max(1) as usize;
    // 학습 주석: max_editor_scroll은 마지막 줄이 viewport 안에 들어오도록 허용되는 최대 scroll입니다. saturating_sub는 파일이 viewport보다
    // 짧을 때 underflow 없이 0을 만들고, u16 clamp는 ratatui scroll API 좌표 범위에 맞춥니다.
    let max_editor_scroll = selected_buffer
        .lines()
        .len()
        .saturating_sub(editor_height)
        .min(u16::MAX as usize) as u16;
    // 학습 주석: 저장된 scroll이 파일 변경이나 viewport 변경 뒤 max를 넘을 수 있으므로 projection 시점에 다시 clamp합니다.
    let editor_scroll = selected_buffer.editor_scroll().min(max_editor_scroll);
    // 학습 주석: cursor offset은 실제 파일 좌표를 viewport 좌표로 바꾼 값입니다. x는 column clamp, y는 현재 scroll만큼 빼서 화면 안의
    // 상대 줄 번호로 변환합니다.
    let editor_cursor_offset = Some((
        selected_buffer.cursor_column().min(u16::MAX as usize) as u16,
        selected_buffer
            .cursor_line_index()
            // 학습 주석: cursor가 scroll 위쪽에 있는 비정상 상태라도 underflow하지 않고 0 row로 포화시킵니다.
            .saturating_sub(editor_scroll as usize)
            // 학습 주석: renderer cursor API는 u16 좌표를 사용하므로 너무 큰 y좌표는 u16::MAX로 제한합니다.
            .min(u16::MAX as usize) as u16,
    ));

    // 학습 주석: 최종 projection은 모든 계산 결과를 값으로 담습니다. 이 경계 이후 renderer는 layout과 widget rendering만 책임지고,
    // buffer state 해석이나 cursor 보정을 다시 수행하지 않습니다.
    PlanningDraftEditorProjection {
        file_lines,
        editor_title: selected_buffer.file_label(),
        editor_lines,
        editor_scroll,
        editor_cursor_offset,
    }
}
