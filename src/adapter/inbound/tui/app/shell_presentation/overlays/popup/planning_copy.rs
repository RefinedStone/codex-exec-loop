use super::super::super::super::super::planning_draft_editor_ui::PlanningDraftEditorCloseRisk;
use super::super::super::super::{AkraTheme, Line, PlanningValidationSeverity};

// Existing-workspace copy is the guard screen schema for "planning already exists here".
// Input builders collapse runtime projections into strings so line builders do not need planning service enums.
pub(super) struct PlanningExistingWorkspaceCopy {
    // Workspace root anchors the warning to the directory that would otherwise receive a new bootstrap.
    pub(super) workspace_directory: String,
    // Uses the same plan substate vocabulary as footer/status panels.
    pub(super) plan_state_label: String,
    // Compact queue state tells the operator what existing runtime work would be affected.
    pub(super) queue_summary: String,
    // Idle policy is pre-rendered here so view code stays branch-free.
    pub(super) queue_idle_policy: String,
    // Optional failure detail keeps degraded snapshot loading visible without replacing the whole guard screen.
    pub(super) failure_summary: Option<String>,
}

// Simple review copy is deliberately small because a deep view pipeline fans it out through many section builders.
// It joins staged draft validation with the adjacent auto-follow turn-budget editor in one immutable input.
pub(super) struct PlanningSimpleReviewCopy {
    // Identifies the staged draft that Enter/Ctrl+P would promote.
    pub(super) draft_name: String,
    // Shows the scale of generated bootstrap artifacts without exposing their full paths here.
    pub(super) staged_file_count: usize,
    // Coarse promotion gate shared by status tone and key guidance.
    pub(super) validation_ok: bool,
    // First blocking error is enough for the compact review surface; full reports stay in validation state.
    pub(super) first_error: Option<String>,
    // Committed auto-follow budget label shown when the numeric editor is not active.
    pub(super) max_auto_turns_label: String,
    // Switches review copy from committed budget label to raw input buffer display.
    pub(super) is_turn_budget_editing: bool,
    // Raw unparsed budget input preserves partially typed values for the text-control mode.
    pub(super) turn_budget_buffer: String,
}

// Draft editor status only needs the representative validation issue, not the full report object.
pub(super) struct PlanningDraftEditorIssueCopy {
    // Severity lets status copy preserve error vs warning weight after report projection.
    pub(super) severity: PlanningValidationSeverity,
    // Detail is already compacted for the footer/status area.
    pub(super) detail: String,
}

// Manual editor status copy joins session metadata, validation summary, dirty-buffer summary, and close-risk state.
// Borrowed fields point back into the session snapshot; derived summaries are owned because they are formatted here.
pub(super) struct PlanningDraftEditorStatusCopy<'a> {
    // Staged draft session currently open in the manual editor.
    pub(super) draft_name: &'a str,
    // Promote target for the selected staged file.
    pub(super) active_path: &'a str,
    // One-based display index such as 2 in "2/5".
    pub(super) selected_file_position: usize,
    // Total staged files in the editor session.
    pub(super) file_count: usize,
    // Shared promote/readiness gate for status and next-action copy.
    pub(super) validation_ok: bool,
    // Representative validation issue shown in the compact status panel.
    pub(super) first_issue: Option<PlanningDraftEditorIssueCopy>,
    // Compact relation between staged file path and active destination path.
    pub(super) staged_path_summary: String,
    // Human-readable dirty file labels for unsaved-change warnings.
    pub(super) dirty_label_summary: String,
    // Boolean companion avoids parsing an empty/non-empty summary string downstream.
    pub(super) has_dirty_labels: bool,
    // Runtime interpreter owns this guidance so status and key copy stay aligned.
    pub(super) next_action: &'static str,
    // Close-risk state comes from UI control flow and drives confirmation wording.
    pub(super) close_risk: Option<PlanningDraftEditorCloseRisk>,
    // True once the close confirmation prompt is active.
    pub(super) confirmation_pending: bool,
}

// Setup title keeps mode selection, review, and existing-workspace guard under the same overlay identity.
pub(super) fn planning_setup_title_line(suffix: &'static str) -> Line<'static> {
    AkraTheme::title_line("Planning Setup", suffix)
}

// Draft title separates staged-file editing from the earlier setup decision surface.
pub(super) fn planning_draft_title_line(suffix: &'static str) -> Line<'static> {
    AkraTheme::title_line("Planning Draft", suffix)
}
