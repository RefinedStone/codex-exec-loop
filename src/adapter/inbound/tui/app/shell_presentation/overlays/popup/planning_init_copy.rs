// planning init copy surface는 wizard router가 세부 화면 파일을 직접 알지 않게 하는
// facade다. 각 module은 copy DTO 또는 UI-local selection enum을 받아 공통
// `PlanningInitOverlayView`로 수렴한다.
#[path = "planning_init_copy/existing_workspace.rs"]
mod existing_workspace;
#[path = "planning_init_copy/review.rs"]
mod review;
#[path = "planning_init_copy/selection.rs"]
mod selection;

use super::super::super::super::{PlanningInitDetailSelection, PlanningInitModeSelection};
use super::super::PlanningInitOverlayView;
use super::copy::{PlanningExistingWorkspaceCopy, PlanningSimpleReviewCopy};

// existing workspace path는 runtime projection에서 만든 copy를 받아 warning/summary/options
// layout으로 바꾼다. app-level snapshot 선택은 위 layer에서 이미 끝났으므로 여기서는
// stale data policy를 다시 판단하지 않는다.
pub(super) fn build_existing_workspace_overlay_view(
    copy: PlanningExistingWorkspaceCopy,
) -> PlanningInitOverlayView {
    existing_workspace::build_existing_workspace_overlay_view(copy)
}

// mode selection은 아직 planning artifact를 만들지 않은 순수 wizard 단계다. 선택 enum
// 하나만 넘겨 line copy를 만들면 selection builder가 app/runtime state에 의존하지 않는다.
pub(super) fn build_mode_selection_overlay_view(
    selected_mode: PlanningInitModeSelection,
) -> PlanningInitOverlayView {
    selection::build_mode_selection_overlay_view(selected_mode)
}

// detail selection도 mode selection과 같은 pre-artifact 단계지만, 이후 manual/editor
// 흐름의 세부 route를 결정한다. 이 facade는 그 route choice만 selection module로 넘긴다.
pub(super) fn build_detail_selection_overlay_view(
    selected_detail: PlanningInitDetailSelection,
) -> PlanningInitOverlayView {
    selection::build_detail_selection_overlay_view(selected_detail)
}

// simple review는 staged draft metadata와 validation summary를 이미 copy DTO로 받은 뒤의
// 화면이다. review module은 app을 다시 읽지 않고 promote 가능성, first error,
// auto-follow budget copy만 사용해 최종 overlay section을 만든다.
pub(super) fn build_simple_review_overlay_view(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    review::build_simple_review_overlay_view(copy)
}

// manual editor 안내는 dedicated editor surface와 함께 뜨는 고정 copy다. 입력을 받지
// 않는다는 사실 자체가 이 branch가 app/runtime state를 읽지 않는다는 계약이다.
pub(super) fn build_manual_editor_overlay_view() -> PlanningInitOverlayView {
    review::build_manual_editor_overlay_view()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Line;

    #[test]
    fn mode_and_detail_selection_copy_tracks_focused_route() {
        let simple = build_mode_selection_overlay_view(PlanningInitModeSelection::Simple);
        assert!(section_text(&simple.option_lines).contains("simple mode"));
        assert!(section_text(&simple.status_lines).contains("current selection: simple mode"));

        let detail = build_mode_selection_overlay_view(PlanningInitModeSelection::Detail);
        assert!(section_text(&detail.status_lines).contains("current selection: detail mode"));
        assert!(section_text(&detail.key_lines).contains("Enter continues"));

        let worker_detail =
            build_detail_selection_overlay_view(PlanningInitDetailSelection::WorkerAssisted);
        assert!(
            section_text(&worker_detail.status_lines)
                .contains("current selection: worker-assisted (disabled)")
        );
        assert!(section_text(&worker_detail.option_lines).contains("future guided drafting flow"));
        assert!(section_text(&worker_detail.key_lines).contains("Backspace/Left goes back"));
    }

    #[test]
    fn manual_editor_copy_points_back_to_dedicated_editor_surface() {
        let view = build_manual_editor_overlay_view();

        assert!(section_text(&view.header_lines).contains("operator inspection"));
        assert!(section_text(&view.summary_lines).contains("dedicated planning draft editor view"));
        assert!(section_text(&view.option_lines).contains("Ctrl+S saves"));
        assert!(section_text(&view.status_lines).contains("editing the staged planning draft"));
    }

    fn section_text(lines: &[Line<'_>]) -> String {
        lines.iter().map(line_text).collect::<Vec<_>>().join("\n")
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }
}
