use crate::adapter::inbound::tui::app::planning_draft_editor_ui::{
    PlanningDraftEditorCloseRisk, PlanningDraftEditorUiState,
};
use crate::domain::planning::PlanningValidationReport;

/*
 * PlanningDraftEditorRuntimeState is the presentation-time interpretation of an active draft editor.
 * The editor UI state owns buffers, selection, validation, and close-guard mechanics; status copy only
 * needs the next recommended command and whether closing would discard or strand work.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PlanningDraftEditorRuntimeState {
    // Command guidance for the status panel, already ordered by dirty-buffer and validation freshness.
    pub(super) next_action: &'static str,
    // Loss model for Esc/close; None means the editor can close without special warning copy.
    pub(super) close_risk: Option<PlanningDraftEditorCloseRisk>,
    // Distinguishes passive risk from the second-step confirmation prompt after a close attempt.
    pub(super) confirmation_pending: bool,
}

// Interpretation boundary between editor state and popup status copy.
// Callers pass already-projected dirty labels so this layer can decide command priority without knowing
// file-kind formatting or buffer rendering details.
pub(super) fn interpret_planning_draft_editor_runtime_state(
    ui_state: &PlanningDraftEditorUiState,
    dirty_labels: &[String],
    validation_report: &PlanningValidationReport,
) -> PlanningDraftEditorRuntimeState {
    // Close state is resolved once so risk text and confirmation styling describe the same UI snapshot.
    let (close_risk, confirmation_pending) = resolve_planning_draft_editor_close_state(ui_state);

    PlanningDraftEditorRuntimeState {
        next_action: planning_draft_editor_next_action(dirty_labels, validation_report),
        close_risk,
        confirmation_pending,
    }
}

// Close-state resolver preserves the editor close-guard priority.
// A pending close confirmation is more specific than the general "closing would be risky" state because
// it means the user has already requested closure and copy must ask for an explicit second action.
fn resolve_planning_draft_editor_close_state(
    ui_state: &PlanningDraftEditorUiState,
) -> (Option<PlanningDraftEditorCloseRisk>, bool) {
    let pending_close_risk = ui_state.pending_close_risk();
    (
        // Pending risk wins so the warning text and danger color represent the confirmation step, not just possibility.
        pending_close_risk.or_else(|| ui_state.close_risk()),
        pending_close_risk.is_some(),
    )
}

// Status guidance policy for the draft editor.
// Dirty buffers outrank validation because the validation report describes the last saved draft, not
// necessarily the text currently visible in the editor.
fn planning_draft_editor_next_action(
    dirty_labels: &[String],
    validation_report: &PlanningValidationReport,
) -> &'static str {
    if !dirty_labels.is_empty() {
        "next action: Ctrl+S re-runs validation, or Ctrl+P saves current edits and promotes if valid"
    } else if validation_report.is_valid() {
        "next action: Ctrl+P promotes this draft into accepted planning state"
    } else {
        "next action: fix validation errors before promoting this draft"
    }
}

#[cfg(test)]
// Tests pin the command-guidance priority independently from the full TUI editor surface.
mod tests {
    use super::planning_draft_editor_next_action;
    use crate::domain::planning::{PlanningFileKind, PlanningValidationReport};

    #[test]
    // Dirty files mean the saved validation report is stale, so save/validate guidance must win.
    fn next_action_prefers_save_guidance_when_dirty_files_exist() {
        let report = PlanningValidationReport::default();
        let dirty_labels = vec!["result-output.md".to_string()];

        let action = planning_draft_editor_next_action(&dirty_labels, &report);

        assert_eq!(
            action,
            "next action: Ctrl+S re-runs validation, or Ctrl+P saves current edits and promotes if valid"
        );
    }

    #[test]
    // Clean buffers plus a valid report are the only state where Ctrl+P can be advertised directly.
    fn next_action_promotes_when_validation_is_clean() {
        let report = PlanningValidationReport::default();

        let action = planning_draft_editor_next_action(&[], &report);

        assert_eq!(
            action,
            "next action: Ctrl+P promotes this draft into accepted planning state"
        );
    }

    #[test]
    // A clean but invalid draft should tell the operator to repair validation before promotion.
    fn next_action_requires_fix_when_validation_has_errors() {
        let mut report = PlanningValidationReport::default();
        report.push_error(
            PlanningFileKind::Directions,
            "missing-summary",
            "summary is required",
        );

        let action = planning_draft_editor_next_action(&[], &report);

        assert_eq!(
            action,
            "next action: fix validation errors before promoting this draft"
        );
    }
}
