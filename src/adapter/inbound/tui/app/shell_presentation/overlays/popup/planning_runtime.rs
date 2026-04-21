use crate::domain::planning::PlanningValidationReport;

use super::super::super::super::super::planning_draft_editor_ui::PlanningDraftEditorCloseRisk;
use super::super::super::super::NativeTuiApp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PlanningDraftEditorRuntimeState {
    pub(super) next_action: &'static str,
    pub(super) close_risk: Option<PlanningDraftEditorCloseRisk>,
    pub(super) confirmation_pending: bool,
}

pub(super) fn interpret_planning_draft_editor_runtime_state(
    app: &NativeTuiApp,
    dirty_labels: &[String],
    validation_report: &PlanningValidationReport,
) -> PlanningDraftEditorRuntimeState {
    let (close_risk, confirmation_pending) = resolve_planning_draft_editor_close_state(app);

    PlanningDraftEditorRuntimeState {
        next_action: planning_draft_editor_next_action(dirty_labels, validation_report),
        close_risk,
        confirmation_pending,
    }
}

fn resolve_planning_draft_editor_close_state(
    app: &NativeTuiApp,
) -> (Option<PlanningDraftEditorCloseRisk>, bool) {
    let pending_close_risk = app.planning_draft_editor_ui_state.pending_close_risk();
    (
        pending_close_risk.or_else(|| app.planning_draft_editor_ui_state.close_risk()),
        pending_close_risk.is_some(),
    )
}

fn planning_draft_editor_next_action(
    dirty_labels: &[String],
    validation_report: &PlanningValidationReport,
) -> &'static str {
    if !dirty_labels.is_empty() {
        "next action: Ctrl+S re-runs validation, or Ctrl+P saves current edits and promotes if valid"
    } else if validation_report.is_valid() {
        "next action: Ctrl+P promotes this draft into active planning files"
    } else {
        "next action: fix validation errors before promoting this draft"
    }
}

#[cfg(test)]
mod tests {
    use super::planning_draft_editor_next_action;
    use crate::domain::planning::{PlanningFileKind, PlanningValidationReport};

    #[test]
    fn next_action_prefers_save_guidance_when_dirty_files_exist() {
        let report = PlanningValidationReport::default();
        let dirty_labels = vec!["directions.toml".to_string()];

        let action = planning_draft_editor_next_action(&dirty_labels, &report);

        assert_eq!(
            action,
            "next action: Ctrl+S re-runs validation, or Ctrl+P saves current edits and promotes if valid"
        );
    }

    #[test]
    fn next_action_promotes_when_validation_is_clean() {
        let report = PlanningValidationReport::default();

        let action = planning_draft_editor_next_action(&[], &report);

        assert_eq!(
            action,
            "next action: Ctrl+P promotes this draft into active planning files"
        );
    }

    #[test]
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
