use super::super::super::super::super::planning_draft_editor_ui::PlanningDraftEditorCloseRisk;
use super::super::super::super::{Color, Line, Modifier, PlanningValidationSeverity, Span, Style};

pub(super) struct PlanningExistingWorkspaceCopy {
    pub(super) workspace_directory: String,
    pub(super) plan_state_label: String,
    pub(super) queue_summary: String,
    pub(super) queue_idle_policy: String,
    pub(super) failure_summary: Option<String>,
    pub(super) plan_enabled: bool,
}

pub(super) struct PlanningSimpleReviewCopy {
    pub(super) draft_name: String,
    pub(super) staged_file_count: usize,
    pub(super) validation_ok: bool,
    pub(super) first_error: Option<String>,
    pub(super) max_auto_turns_label: String,
    pub(super) is_turn_budget_editing: bool,
    pub(super) turn_budget_buffer: String,
}

pub(super) struct PlanningDraftEditorIssueCopy {
    pub(super) severity: PlanningValidationSeverity,
    pub(super) detail: String,
}

pub(super) struct PlanningDraftEditorStatusCopy {
    pub(super) draft_name: String,
    pub(super) active_path: String,
    pub(super) selected_file_position: usize,
    pub(super) file_count: usize,
    pub(super) validation_ok: bool,
    pub(super) first_issue: Option<PlanningDraftEditorIssueCopy>,
    pub(super) staged_path_summary: String,
    pub(super) dirty_label_summary: String,
    pub(super) has_dirty_labels: bool,
    pub(super) next_action: String,
    pub(super) close_risk: Option<PlanningDraftEditorCloseRisk>,
    pub(super) confirmation_pending: bool,
}

pub(super) fn planning_setup_title_line(suffix: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            "Planning Setup",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(suffix.to_string()),
    ])
}

pub(super) fn planning_draft_title_line(suffix: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            "Planning Draft",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(suffix.to_string()),
    ])
}
