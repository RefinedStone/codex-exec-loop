// 학습 주석: manual editor overlay도 일반 planning init overlay shape를 사용하므로 themed key line과 plain
// styled line helper만 가져와 각 영역을 채웁니다.
use super::super::super::super::super::super::{AkraTheme, Line};
// 학습 주석: manual editor 상태는 별도 DTO가 아니라 `PlanningInitOverlayView`의 header/summary/options/status/key
// slots로 표현됩니다. 실제 draft editing surface는 dedicated editor view가 담당합니다.
use super::super::super::PlanningInitOverlayView;
// 학습 주석: planning draft title helper는 manual editor variant가 planning draft 흐름 안에 있음을 같은 스타일로 표시합니다.
use super::super::super::copy::planning_draft_title_line;

// 학습 주석: 이 builder는 "manual editor로 넘어간 상태"를 설명하는 planning init overlay를 만듭니다. draft
// 본문 자체는 editor overlay가 렌더링하고, 이 view는 사용자가 왜 editor surface를 보고 있는지 안내합니다.
pub(super) fn build_manual_editor_overlay_view() -> PlanningInitOverlayView {
    PlanningInitOverlayView {
        // 학습 주석: header는 planning draft title과 짧은 instruction을 함께 보여 줍니다. operator inspection
        // suffix는 simple review와 같은 검토 단계 안에서 수동 편집으로 들어왔다는 맥락입니다.
        header_lines: vec![
            planning_draft_title_line(" / operator inspection"),
            // 학습 주석: save 후 validation이 다시 돈다는 문구는 편집 action과 planning validation loop를 연결합니다.
            Line::from("Edit the staged planning draft and save to re-run validation."),
        ],
        // 학습 주석: summary는 실제 editing UI가 별도 draft editor view라는 사실을 알려 이 overlay가 본문
        // editor가 아니라 안내/상태 layer임을 분명히 합니다.
        summary_lines: vec![Line::from(
            "This state renders through the dedicated planning draft editor view.",
        )],
        // 학습 주석: option line은 editor surface에서 가장 중요한 next actions를 설명합니다.
        option_lines: vec![Line::from(
            "next action: Tab switches files. Ctrl+S saves and re-runs validation.",
        )],
        // 학습 주석: status line은 현재 planning init state가 staged draft editing임을 고정 문구로 표시합니다.
        status_lines: vec![Line::from(
            "current state: editing the staged planning draft",
        )],
        // 학습 주석: key line은 이 안내 surface를 닫는 escape path만 제공합니다. save/tab 같은 editor-local
        // 조작은 option line에 남기고, themed key area는 close action에 집중합니다.
        key_lines: vec![AkraTheme::key_line("Esc/Ctrl+C closes this surface.")],
    }
}
