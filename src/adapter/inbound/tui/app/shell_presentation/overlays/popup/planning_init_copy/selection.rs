use super::super::super::super::super::{
    Line, PlanningInitDetailSelection, PlanningInitModeSelection,
};
use super::super::super::super::option_lines::overlay_option_line;
use super::super::super::PlanningInitOverlayView;
use super::super::copy::planning_setup_title_line;

pub(super) fn build_mode_selection_overlay_view(
    selected_mode: PlanningInitModeSelection,
) -> PlanningInitOverlayView {
    PlanningInitOverlayView {
        header_lines: vec![
            planning_setup_title_line(" / operator inspection"),
            Line::from("Pick the planning entry path before any files are staged."),
        ],
        summary_lines: vec![
            Line::from(
                "Every guided path stages a promotable draft before active planning changes.",
            ),
            Line::from(
                "Simple mode keeps one generic active direction; detail mode prepares richer direction authoring.",
            ),
        ],
        option_lines: vec![
            overlay_option_line(
                "A",
                "simple mode",
                "stage one generic direction and an empty task ledger",
                selected_mode == PlanningInitModeSelection::Simple,
                false,
            ),
            overlay_option_line(
                "B",
                "detail mode",
                "branch into manual or future llm-assisted authoring",
                selected_mode == PlanningInitModeSelection::Detail,
                false,
            ),
        ],
        status_lines: vec![
            Line::from(format!(
                "current selection: {}",
                match selected_mode {
                    PlanningInitModeSelection::Simple => "simple mode",
                    PlanningInitModeSelection::Detail => "detail mode",
                }
            )),
            Line::from("simple mode is the low-ceremony path for planning-aware execution."),
        ],
        key_lines: vec![
            Line::from("A/B or arrows move selection."),
            Line::from("Enter continues. Esc/Ctrl+C cancels."),
        ],
    }
}

pub(super) fn build_detail_selection_overlay_view(
    selected_detail: PlanningInitDetailSelection,
) -> PlanningInitOverlayView {
    PlanningInitOverlayView {
        header_lines: vec![
            planning_setup_title_line(" / operator inspection"),
            Line::from("Current step: choose how detail-mode drafts should be prepared."),
        ],
        summary_lines: vec![
            Line::from("Manual opens the staged draft editor inside the shell."),
            Line::from("LLM-assisted remains visible for the target UX but is still disabled."),
        ],
        option_lines: vec![
            overlay_option_line(
                "A",
                "manual",
                "stage the detail scaffold and keep editing inside the shell",
                selected_detail == PlanningInitDetailSelection::Manual,
                false,
            ),
            overlay_option_line(
                "B",
                "llm-assisted",
                "future guided drafting flow (not supported yet)",
                selected_detail == PlanningInitDetailSelection::LlmAssisted,
                true,
            ),
        ],
        status_lines: vec![
            Line::from(format!(
                "current selection: {}",
                match selected_detail {
                    PlanningInitDetailSelection::Manual => "manual",
                    PlanningInitDetailSelection::LlmAssisted => "llm-assisted (disabled)",
                }
            )),
            Line::from("Enter on manual opens the embedded draft editor."),
        ],
        key_lines: vec![
            Line::from("A/B or arrows move selection."),
            Line::from("Backspace/Left goes back. Enter continues. Esc/Ctrl+C cancels."),
        ],
    }
}
