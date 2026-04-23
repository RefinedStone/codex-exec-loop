use crate::domain::planning::PlanningValidationReport;

use super::super::super::super::super::planning_draft_editor_ui::PlanningDraftEditorCloseRisk;
use super::super::super::super::{FOOTER_NOTICE_DETAIL_LIMIT, compact_inline_detail};
use super::copy::{PlanningDraftEditorIssueCopy, PlanningDraftEditorStatusCopy};

#[allow(clippy::too_many_arguments)]
pub(super) fn build_planning_draft_editor_status_copy<'a>(
    draft_name: &'a str,
    active_path: &'a str,
    selected_file_position: usize,
    file_count: usize,
    validation_report: &PlanningValidationReport,
    staged_path: &str,
    dirty_labels: &[String],
    next_action: &'static str,
    close_risk: Option<PlanningDraftEditorCloseRisk>,
    confirmation_pending: bool,
) -> PlanningDraftEditorStatusCopy<'a> {
    PlanningDraftEditorStatusCopy {
        draft_name,
        active_path,
        selected_file_position,
        file_count,
        validation_ok: validation_report.is_valid(),
        first_issue: build_first_issue_copy(validation_report),
        staged_path_summary: compact_inline_detail(staged_path, FOOTER_NOTICE_DETAIL_LIMIT),
        dirty_label_summary: summarize_dirty_labels(dirty_labels),
        has_dirty_labels: !dirty_labels.is_empty(),
        next_action,
        close_risk,
        confirmation_pending,
    }
}

fn build_first_issue_copy(
    validation_report: &PlanningValidationReport,
) -> Option<PlanningDraftEditorIssueCopy> {
    validation_report
        .issues
        .first()
        .map(|issue| PlanningDraftEditorIssueCopy {
            severity: issue.severity,
            detail: compact_inline_detail(&issue.message, FOOTER_NOTICE_DETAIL_LIMIT),
        })
}

fn summarize_dirty_labels(dirty_labels: &[String]) -> String {
    if dirty_labels.is_empty() {
        "none".to_string()
    } else {
        compact_inline_detail(&dirty_labels.join(", "), FOOTER_NOTICE_DETAIL_LIMIT)
    }
}

#[cfg(test)]
mod tests {
    use super::{build_planning_draft_editor_status_copy, summarize_dirty_labels};
    use crate::domain::planning::{
        PlanningFileKind, PlanningValidationReport, PlanningValidationSeverity,
    };

    #[test]
    fn dirty_label_summary_reports_none_when_clean() {
        assert_eq!(summarize_dirty_labels(&[]), "none");
    }

    #[test]
    fn status_copy_prefers_first_validation_issue() {
        let mut validation_report = PlanningValidationReport::default();
        validation_report.push_warning(
            PlanningFileKind::Directions,
            "first-warning",
            "first issue should be clearer",
        );
        validation_report.push_error(
            PlanningFileKind::Directions,
            "second-error",
            "second issue is critical",
        );

        let status_copy = build_planning_draft_editor_status_copy(
            "draft-1",
            "planning/directions.toml",
            1,
            1,
            &validation_report,
            ".draft/planning/directions.toml",
            &[],
            "next action: inspect",
            None,
            false,
        );

        let first_issue = status_copy
            .first_issue
            .expect("first issue should exist for warnings");
        assert_eq!(first_issue.severity, PlanningValidationSeverity::Warning);
        assert!(first_issue.detail.contains("first issue should be clearer"));
    }
}
