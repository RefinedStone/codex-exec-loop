use crate::adapter::inbound::tui::conversation_text::{
    approval_review_status_text, approval_review_summary_text, control_support_label,
};
use crate::domain::conversation::ConversationApprovalReview;

use super::ConversationViewModel;

/*
 * Status projection keeps the conversation read model narrow enough for footer
 * and status-panel rendering. The full warning/notice/review records stay on
 * ConversationViewModel; this impl chooses representative copy, count labels,
 * and truncation rules for compact TUI surfaces.
 */
impl ConversationViewModel {
    fn compact_warning_text(warning: &str) -> String {
        // Runtime diagnostics can include indentation, tabs, and line breaks.
        // Collapse them before measuring so footer summaries spend their width
        // budget on content rather than formatting artifacts.
        let mut compact = String::with_capacity(warning.len());
        for segment in warning.split_whitespace() {
            if !compact.is_empty() {
                compact.push(' ');
            }
            compact.push_str(segment);
        }
        compact
    }

    fn truncate_warning_text(warning: &str, max_detail_len: usize) -> String {
        // Use an ASCII suffix so terminal cell accounting and snapshot tests do
        // not depend on locale-specific ellipsis width.
        const TRUNCATION_SUFFIX: &str = "...";
        let compact = Self::compact_warning_text(warning);
        // Callers may pass very small budgets from narrow layouts; preserve at
        // least the suffix width to avoid underflow in the take count.
        let max_detail_len = max_detail_len.max(TRUNCATION_SUFFIX.len());
        if compact.chars().count() <= max_detail_len {
            return compact;
        }
        // Truncate by char, not byte, because warnings can contain Korean text,
        // paths, and copied command output in the same string.
        let truncated = compact
            .chars()
            .take(max_detail_len - TRUNCATION_SUFFIX.len())
            .collect::<String>();
        format!("{truncated}{TRUNCATION_SUFFIX}")
    }

    fn selected_warning_for_summary(&self) -> Option<&str> {
        // The latest warning is the one most likely to explain the current
        // degraded state. Older warnings are still represented by the count.
        self.base_warnings.last().map(String::as_str)
    }

    fn warning_status_label(&self) -> Option<String> {
        // The primary status line receives only a count badge. Detailed warning
        // copy is exposed through warning_summary so the status line stays
        // scannable during active turns.
        let runtime_count = self.base_warnings.len();
        match runtime_count {
            0 => None,
            1 => Some("warning".to_string()),
            warning_count => Some(format!("warnings ({warning_count})")),
        }
    }

    pub(crate) fn warning_summary(&self, max_detail_len: usize) -> String {
        // Return explicit none-copy rather than an empty string so downstream
        // panels can render a stable row without guessing whether data is absent.
        let Some(selected_warning) = self.selected_warning_for_summary() else {
            return "warning: none".to_string();
        };
        let summary = Self::truncate_warning_text(selected_warning, max_detail_len);
        // Keep count grammar aligned with warning_status_label while adding the
        // representative latest warning detail.
        match self.base_warnings.len() {
            0 => "warning: none".to_string(),
            1 => format!("warning: {summary}"),
            warning_count => format!("warnings ({warning_count}): {summary}"),
        }
    }

    pub(crate) fn runtime_notice_summary(&self, max_detail_len: usize) -> Option<String> {
        // Runtime notices are optional status details. None lets callers hide
        // the row entirely when no notice has been recorded.
        let selected_notice = self.runtime_notices.last()?;
        let summary = Self::truncate_warning_text(selected_notice, max_detail_len);
        Some(if self.runtime_notices.len() == 1 {
            format!("runtime: {summary}")
        } else {
            format!(
                "runtime notices ({}): {summary}",
                self.runtime_notices.len()
            )
        })
    }

    pub(crate) fn planning_notice_summary(&self, max_detail_len: usize) -> Option<String> {
        // Planning notices share runtime_notices storage, but the footer exposes
        // them separately because planning state often needs operator action.
        let planning_notices = self
            .runtime_notices
            .iter()
            // The producer-side prefix is the lightweight boundary here; adding
            // a separate enum would make the view model heavier than this
            // presentation-only split needs to be.
            .filter(|notice| notice.starts_with("planning "))
            .collect::<Vec<_>>();
        let selected_notice = planning_notices.last()?;
        let summary = Self::truncate_warning_text(selected_notice, max_detail_len);

        Some(if planning_notices.len() == 1 {
            format!("planning: {summary}")
        } else {
            format!("planning notices ({}): {summary}", planning_notices.len())
        })
    }

    pub(crate) fn approval_summary(&self) -> Option<String> {
        // Approval copy is delegated to conversation_text so status and summary
        // surfaces use the same control/approval terminology.
        self.approval_review
            .as_ref()
            .map(approval_review_summary_text)
    }

    pub(crate) fn update_approval_review(&mut self, review: ConversationApprovalReview) {
        // Approval review changes affect both the stored review detail and the
        // main status line, because approval availability can change while the
        // same turn remains selected.
        self.set_status_with_warnings(approval_review_status_text(
            &review,
            self.turn_control_truth.approval,
        ));
        self.approval_review = Some(review);
    }

    pub(crate) fn interrupt_support_label(&self) -> &'static str {
        // Interrupt labeling follows the same control-support helper as other
        // footer copy, keeping "supported/blocked" language consistent.
        control_support_label(self.turn_control_truth.interrupt)
    }

    pub(crate) fn set_status_with_warnings(&mut self, base_status: String) {
        // Compose the base conversation status with the warning badge once here
        // so renderers do not need to repeat warning counting logic.
        self.status_text = match self.warning_status_label() {
            Some(warning_label) => format!("{base_status} / {warning_label}"),
            None => base_status,
        };
    }
}
