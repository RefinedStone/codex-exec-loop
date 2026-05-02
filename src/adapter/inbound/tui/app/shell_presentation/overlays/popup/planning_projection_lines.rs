use super::super::super::super::super::planning_draft_editor_ui::PlanningDraftEditorBufferState;
use super::super::super::super::{AkraTheme, Line, Modifier, Span, Style};

// draft editor 좌측 파일 목록은 editor buffer state를 변경하지 않는 순수 projection이다.
// `planning_projection.rs`가 selected buffer의 body/cursor projection을 만들 때 이 helper의
// file rows를 함께 묶어 popup/inline renderer 모두 같은 file-list grammar를 쓰게 한다.
pub(super) fn build_planning_draft_editor_file_lines(
    buffers: &[PlanningDraftEditorBufferState],
    selected_index: usize,
) -> Vec<Line<'static>> {
    buffers
        .iter()
        .enumerate()
        .map(|(index, buffer)| {
            // selected_index는 `PlanningDraftEditorSession`이 보장하는 selected_buffer와
            // 같은 항목을 가리킨다. 여기서는 그 불변식을 시각적 focus marker로만 변환한다.
            let selected = index == selected_index;
            // dirty 상태는 file_label 자체를 바꾸지 않고 suffix로 둔다. 같은 label은 status
            // summary나 close-risk copy에서도 쓰이므로, 목록 전용 표시를 label contract에 섞지 않는다.
            let dirty_suffix = if buffer.is_dirty() { " *dirty" } else { "" };
            // keyboard focus가 저장 경고보다 우선한다. 선택된 dirty file은 selected style을
            // 쓰되 suffix로 dirty 사실을 보존하고, 선택되지 않은 dirty file만 warning으로 보인다.
            let style = if selected {
                AkraTheme::selected()
            } else if buffer.is_dirty() {
                AkraTheme::warning()
            } else {
                Style::default()
            };
            // marker column은 색을 볼 수 없는 terminal에서도 focus를 구분하게 하고, idle row도
            // 같은 폭을 차지해 파일명이 선택 이동 때 좌우로 흔들리지 않게 한다.
            let marker = if selected {
                AkraTheme::selected_marker()
            } else {
                "  "
            };
            // row를 marker, bold label, dirty suffix span으로 나누면 label만 scan anchor로
            // 강조하면서 row 전체는 selected/dirty/default 상태 색상을 공유할 수 있다.
            Line::from(vec![
                Span::styled(marker, style),
                Span::styled(buffer.file_label(), style.add_modifier(Modifier::BOLD)),
                Span::styled(dirty_suffix, style),
            ])
        })
        .collect()
}
