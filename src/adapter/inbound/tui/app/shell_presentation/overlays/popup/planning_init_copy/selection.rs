use super::super::super::super::super::{
    AkraTheme, Line, PlanningInitDetailSelection, PlanningInitModeSelection,
};
use super::super::super::super::option_lines::overlay_option_line;
use super::super::super::PlanningInitOverlayView;
use super::super::copy::planning_setup_title_line;

// First planning-init inspection screen: choose the authoring route before any planning files are staged.
pub(super) fn build_mode_selection_overlay_view(
    selected_mode: PlanningInitModeSelection,
) -> PlanningInitOverlayView {
    PlanningInitOverlayView {
        header_lines: vec![
            planning_setup_title_line(" / operator inspection"),
            Line::from("Pick the planning entry path before any files are staged."),
        ],
        // Both paths produce a promotable draft first; accepted planning state is never mutated from this selector.
        summary_lines: vec![
            Line::from(
                "Every guided path stages a promotable draft before active planning changes.",
            ),
            Line::from(
                "Simple mode keeps one generic active direction; detail mode prepares richer direction authoring.",
            ),
        ],
        // Option rows are keyed to the controller enum; neither route is disabled at the mode-selection step.
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
        // Repeat the selected route in status copy so narrow popup layouts still expose the current cursor.
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
        // Key copy mirrors the mode-selection handler: move cursor, continue, or cancel without side effects.
        key_lines: vec![
            AkraTheme::key_line("A/B or arrows move selection."),
            AkraTheme::key_line("Enter continues. Esc/Ctrl+C cancels."),
        ],
    }
}

// Second inspection screen for detail mode. Manual is live; LLM-assisted remains visible as the target UX but disabled.
pub(super) fn build_detail_selection_overlay_view(
    selected_detail: PlanningInitDetailSelection,
) -> PlanningInitOverlayView {
    PlanningInitOverlayView {
        header_lines: vec![
            planning_setup_title_line(" / operator inspection"),
            Line::from("Current step: choose how detail-mode drafts should be prepared."),
        ],
        // Keep disabled future UX visible, but clearly route real work into the embedded manual editor.
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
        // Status text makes the disabled cursor state explicit even if the option row is clipped.
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
        // Detail-selection keys include a back edge to mode selection plus the same continue/cancel contract.
        key_lines: vec![
            AkraTheme::key_line("A/B or arrows move selection."),
            AkraTheme::key_line("Backspace/Left goes back. Enter continues. Esc/Ctrl+C cancels."),
        ],
    }
}
