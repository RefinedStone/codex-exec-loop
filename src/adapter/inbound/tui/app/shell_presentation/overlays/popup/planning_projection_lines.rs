// 학습 주석: PlanningDraftEditorBufferState는 draft editor가 가진 파일별 text buffer와 dirty/cursor metadata입니다.
// 이 파일은 그 UI state를 직접 변경하지 않고, 좌측 file list에 그릴 presentation line으로만 변환합니다.
use super::super::super::super::super::planning_draft_editor_ui::PlanningDraftEditorBufferState;
// 학습 주석: file list row는 marker, bold filename, dirty suffix를 Span으로 나눠 구성합니다.
// theme/style type을 가져와 selected row와 dirty row의 색상 우선순위를 이 helper 안에서 통일합니다.
use super::super::super::super::{AkraTheme, Line, Modifier, Span, Style};

// 학습 주석: build_planning_draft_editor_file_lines는 planning draft editor의 파일 tab/list 영역을 만듭니다.
// planning_projection.rs가 editor body/cursor projection을 만들 때 이 helper를 호출해 좌측 파일 목록을 함께 얻습니다.
pub(super) fn build_planning_draft_editor_file_lines(
    // 학습 주석: buffers는 draft editor가 열고 있는 파일들의 현재 buffer snapshot입니다.
    // 각 buffer는 label, dirty 여부, active/staged path 같은 표시 데이터를 제공합니다.
    buffers: &[PlanningDraftEditorBufferState],
    // 학습 주석: selected_index는 keyboard focus가 가리키는 파일 위치입니다. renderer는 이 줄에 selected style과 marker를 씁니다.
    selected_index: usize,
) -> Vec<Line<'static>> {
    buffers
        // 학습 주석: buffer slice를 순서대로 순회해 editor의 파일 ordering을 그대로 유지합니다.
        .iter()
        // 학습 주석: enumerate로 row index를 붙여 selected_index와 비교할 수 있게 합니다.
        .enumerate()
        // 학습 주석: 각 buffer를 renderer가 바로 그릴 수 있는 owned Line<'static> row로 변환합니다.
        .map(|(index, buffer)| {
            // 학습 주석: selected는 현재 row가 editor focus 대상인지 나타냅니다. dirty보다 selection을 더 강하게 표시합니다.
            let selected = index == selected_index;
            // 학습 주석: dirty suffix는 저장되지 않은 변경이 있는 파일만 붙는 inline 상태 표시입니다.
            // label 자체를 바꾸지 않고 suffix로 둬 file_label contract를 다른 곳과 공유합니다.
            let dirty_suffix = if buffer.is_dirty() { " *dirty" } else { "" };
            // 학습 주석: style precedence는 selected > dirty > default입니다. 선택된 dirty file은 warning보다
            // keyboard focus를 먼저 보여 주고, suffix text로 dirty 상태를 함께 보존합니다.
            let style = if selected {
                // 학습 주석: selected style은 현재 편집 대상 파일을 list에서 즉시 찾게 합니다.
                AkraTheme::selected()
            } else if buffer.is_dirty() {
                // 학습 주석: dirty지만 선택되지 않은 파일은 warning style로 저장 필요성을 보여 줍니다.
                AkraTheme::warning()
            } else {
                // 학습 주석: 깨끗하고 선택되지 않은 파일은 주변 list와 같은 기본 스타일입니다.
                Style::default()
            };
            // 학습 주석: marker는 color만으로는 focus를 구분하기 어려운 terminal에서도 selected row를 표시합니다.
            // 선택되지 않은 row도 두 칸을 차지해 filename column alignment를 유지합니다.
            let marker = if selected {
                // 학습 주석: selected marker는 AkraTheme의 공통 list cursor glyph를 사용합니다.
                AkraTheme::selected_marker()
            } else {
                // 학습 주석: idle marker는 빈 공간이지만 selected marker와 같은 폭을 차지하는 alignment spacer입니다.
                "  "
            };
            // 학습 주석: row는 marker, bold file label, dirty suffix의 세 Span입니다. 이렇게 나누면 label만
            // 굵게 처리하면서 row 전체 상태 색상은 동일하게 유지할 수 있습니다.
            Line::from(vec![
                // 학습 주석: 첫 span은 focus marker column입니다.
                Span::styled(marker, style),
                // 학습 주석: file label은 active/staged path에서 나온 사용자 친화 이름이며, list scanning을 위해 bold입니다.
                Span::styled(buffer.file_label(), style.add_modifier(Modifier::BOLD)),
                // 학습 주석: suffix는 dirty 상태일 때만 내용이 있고, 같은 style을 써서 selected/dirty 색상과 일관됩니다.
                Span::styled(dirty_suffix, style),
            ])
        })
        // 학습 주석: popup view DTO가 Vec<Line>을 요구하므로 iterator를 materialize합니다.
        .collect()
}
