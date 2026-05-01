use crate::adapter::inbound::tui::conversation_text::{
    approval_review_status_text, approval_review_summary_text, control_support_label,
};
use crate::domain::conversation::ConversationApprovalReview;

use super::ConversationViewModel;

impl ConversationViewModel {
    fn compact_warning_text(warning: &str) -> String {
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
        const TRUNCATION_SUFFIX: &str = "...";

        let compact = Self::compact_warning_text(warning);
        let max_detail_len = max_detail_len.max(TRUNCATION_SUFFIX.len());
        if compact.chars().count() <= max_detail_len {
            return compact;
        }

        let truncated = compact
            .chars()
            .take(max_detail_len - TRUNCATION_SUFFIX.len())
            .collect::<String>();
        format!("{truncated}{TRUNCATION_SUFFIX}")
    }

    fn selected_warning_for_summary(&self) -> Option<&str> {
        self.base_warnings.last().map(String::as_str)
    }

    fn warning_status_label(&self) -> Option<String> {
        let runtime_count = self.base_warnings.len();

        match runtime_count {
            0 => None,
            1 => Some("warning".to_string()),
            warning_count => Some(format!("warnings ({warning_count})")),
        }
    }

    pub(crate) fn warning_summary(&self, max_detail_len: usize) -> String {
        let Some(selected_warning) = self.selected_warning_for_summary() else {
            return "warning: none".to_string();
        };

        let summary = Self::truncate_warning_text(selected_warning, max_detail_len);
        match self.base_warnings.len() {
            0 => "warning: none".to_string(),
            1 => format!("warning: {summary}"),
            warning_count => format!("warnings ({warning_count}): {summary}"),
        }
    }

    pub(crate) fn runtime_notice_summary(&self, max_detail_len: usize) -> Option<String> {
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
        let planning_notices = self
            .runtime_notices
            .iter()
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
        self.approval_review
            .as_ref()
            .map(approval_review_summary_text)
    }

    pub(crate) fn update_approval_review(&mut self, review: ConversationApprovalReview) {
        self.set_status_with_warnings(approval_review_status_text(
            &review,
            self.turn_control_truth.approval,
        ));
        self.approval_review = Some(review);
    }

    pub(crate) fn interrupt_support_label(&self) -> &'static str {
        control_support_label(self.turn_control_truth.interrupt)
    }

    pub(crate) fn set_status_with_warnings(&mut self, base_status: String) {
        self.status_text = match self.warning_status_label() {
            Some(warning_label) => format!("{base_status} / {warning_label}"),
            None => base_status,
        };
    }
}
