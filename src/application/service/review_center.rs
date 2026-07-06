use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;

use crate::application::port::outbound::review_center_repository_port::{
    ReviewCenterHistoryEntry, ReviewCenterInboxItem, ReviewCenterRepositoryPort,
    ReviewCenterThreadProjection,
};
use crate::domain::conversation::{ConversationApprovalReview, ConversationApprovalReviewStatus};

#[derive(Clone)]
pub struct ReviewCenterReadService {
    workspace_dir: String,
    review_center_repository: Arc<dyn ReviewCenterRepositoryPort>,
}

#[derive(Clone)]
pub struct ReviewCenterWriteService {
    workspace_dir: String,
    review_center_repository: Arc<dyn ReviewCenterRepositoryPort>,
}

impl ReviewCenterReadService {
    pub fn new(
        workspace_dir: impl Into<String>,
        review_center_repository: Arc<dyn ReviewCenterRepositoryPort>,
    ) -> Self {
        Self {
            workspace_dir: workspace_dir.into(),
            review_center_repository,
        }
    }

    pub fn load_thread_reviews(
        &self,
        thread_id: &str,
    ) -> Result<Vec<ReviewCenterThreadProjection>> {
        self.review_center_repository
            .load_thread_reviews(&self.workspace_dir, thread_id)
    }

    pub fn load_pending_inbox(&self) -> Result<Vec<ReviewCenterInboxItem>> {
        self.review_center_repository
            .load_pending_inbox(&self.workspace_dir)
    }

    pub fn load_recent_history(&self) -> Result<Vec<ReviewCenterHistoryEntry>> {
        self.review_center_repository
            .load_recent_history(&self.workspace_dir)
    }

    pub fn workspace_dir(&self) -> &str {
        self.workspace_dir.as_str()
    }

    pub fn write_service(&self) -> ReviewCenterWriteService {
        ReviewCenterWriteService::new(
            self.workspace_dir.clone(),
            self.review_center_repository.clone(),
        )
    }

    pub fn load_thread_reviews_for_workspace(
        &self,
        workspace_dir: &str,
        thread_id: &str,
    ) -> Result<Vec<ReviewCenterThreadProjection>> {
        self.review_center_repository
            .load_thread_reviews(workspace_dir, thread_id)
    }

    pub fn load_pending_inbox_for_workspace(
        &self,
        workspace_dir: &str,
    ) -> Result<Vec<ReviewCenterInboxItem>> {
        self.review_center_repository
            .load_pending_inbox(workspace_dir)
    }

    pub fn load_recent_history_for_workspace(
        &self,
        workspace_dir: &str,
    ) -> Result<Vec<ReviewCenterHistoryEntry>> {
        self.review_center_repository
            .load_recent_history(workspace_dir)
    }
}

impl ReviewCenterWriteService {
    pub fn new(
        workspace_dir: impl Into<String>,
        review_center_repository: Arc<dyn ReviewCenterRepositoryPort>,
    ) -> Self {
        Self {
            workspace_dir: workspace_dir.into(),
            review_center_repository,
        }
    }

    pub fn persist_approval_review(
        &self,
        thread_id: &str,
        review: &ConversationApprovalReview,
    ) -> Result<()> {
        self.persist_approval_review_for_workspace(&self.workspace_dir, thread_id, review)
    }

    pub fn persist_approval_review_for_workspace(
        &self,
        workspace_dir: &str,
        thread_id: &str,
        review: &ConversationApprovalReview,
    ) -> Result<()> {
        let review_id = review_identifier(thread_id, review);
        let now = Utc::now().to_rfc3339();
        let existing = self
            .review_center_repository
            .load_thread_reviews(workspace_dir, thread_id)?
            .into_iter()
            .find(|stored| stored.review_id == review_id);
        let requested_at = existing
            .as_ref()
            .map(|stored| stored.requested_at.clone())
            .unwrap_or_else(|| now.clone());
        let projection =
            build_thread_review_projection(thread_id, &review_id, review, &requested_at, &now);
        let history_changed = existing
            .as_ref()
            .map(|stored| !same_review_projection(stored, &projection))
            .unwrap_or(true);

        self.review_center_repository
            .upsert_thread_review(workspace_dir, &projection)?;

        let mut inbox = self
            .review_center_repository
            .load_pending_inbox(workspace_dir)?
            .into_iter()
            .filter(|item| !(item.review_id == review_id && item.thread_id == thread_id))
            .collect::<Vec<_>>();
        if let Some(item) = build_pending_inbox_item(&projection) {
            inbox.push(item);
        }
        self.review_center_repository
            .replace_pending_inbox(workspace_dir, &inbox)?;

        if history_changed {
            self.review_center_repository.append_history_entry(
                workspace_dir,
                &build_history_entry(
                    thread_id,
                    &review_id,
                    review,
                    &projection.review_summary,
                    &now,
                ),
            )?;
        }

        Ok(())
    }
}

fn same_review_projection(
    left: &ReviewCenterThreadProjection,
    right: &ReviewCenterThreadProjection,
) -> bool {
    left.review_id == right.review_id
        && left.review_label == right.review_label
        && left.review_state == right.review_state
        && left.review_summary == right.review_summary
        && left.handoff_target == right.handoff_target
        && left.handoff_note == right.handoff_note
}

fn build_thread_review_projection(
    thread_id: &str,
    review_id: &str,
    review: &ConversationApprovalReview,
    requested_at: &str,
    updated_at: &str,
) -> ReviewCenterThreadProjection {
    let mut projection = ReviewCenterThreadProjection::new(
        thread_id,
        review_id,
        review_label(review),
        review_state(review),
        review_summary(review),
        requested_at,
        updated_at,
    );
    if requires_manual_handoff(&review.status) {
        projection.handoff_target = Some("operator".to_string());
        projection.handoff_note = Some("open review center inbox".to_string());
    }
    projection
}

fn build_pending_inbox_item(
    review: &ReviewCenterThreadProjection,
) -> Option<ReviewCenterInboxItem> {
    matches!(review.review_state.as_str(), "pending" | "waiting").then(|| {
        let mut item = ReviewCenterInboxItem::new(
            review.review_id.clone(),
            review.thread_id.clone(),
            review.review_state.clone(),
            review.review_summary.clone(),
            review.requested_at.clone(),
            review.updated_at.clone(),
        );
        item.handoff_target = review.handoff_target.clone();
        item
    })
}

fn build_history_entry(
    thread_id: &str,
    review_id: &str,
    review: &ConversationApprovalReview,
    summary: &str,
    recorded_at: &str,
) -> ReviewCenterHistoryEntry {
    ReviewCenterHistoryEntry::new(
        review_id,
        thread_id,
        history_event_kind(review),
        summary,
        recorded_at,
    )
}

fn review_identifier(thread_id: &str, review: &ConversationApprovalReview) -> String {
    let target_item_id = review.target_item_id.trim();
    if target_item_id.is_empty() {
        format!("{thread_id}:approval-review")
    } else {
        target_item_id.to_string()
    }
}

fn review_label(review: &ConversationApprovalReview) -> &'static str {
    if requires_manual_handoff(&review.status) {
        "manual handoff"
    } else {
        "approval review"
    }
}

fn review_state(review: &ConversationApprovalReview) -> String {
    match &review.status {
        ConversationApprovalReviewStatus::InProgress => "pending".to_string(),
        ConversationApprovalReviewStatus::Approved => "approved".to_string(),
        ConversationApprovalReviewStatus::Denied => "denied".to_string(),
        ConversationApprovalReviewStatus::Aborted => "aborted".to_string(),
        ConversationApprovalReviewStatus::Unknown(value)
            if requires_manual_handoff(&review.status) =>
        {
            if humanize_protocol_status(value).contains("waiting") {
                humanize_protocol_status(value)
            } else {
                "waiting".to_string()
            }
        }
        ConversationApprovalReviewStatus::Unknown(value) => humanize_protocol_status(value),
    }
}

fn review_summary(review: &ConversationApprovalReview) -> String {
    review
        .rationale
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let mut summary = approval_review_summary_label(&review.status);
            if let Some(risk_level) = review
                .risk_level
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                summary.push(' ');
                summary.push_str(risk_level);
            }
            summary
        })
}

fn history_event_kind(review: &ConversationApprovalReview) -> String {
    match &review.status {
        ConversationApprovalReviewStatus::InProgress => "review_requested".to_string(),
        ConversationApprovalReviewStatus::Approved => "review_approved".to_string(),
        ConversationApprovalReviewStatus::Denied => "review_denied".to_string(),
        ConversationApprovalReviewStatus::Aborted => "review_aborted".to_string(),
        ConversationApprovalReviewStatus::Unknown(value)
            if requires_manual_handoff(&review.status) =>
        {
            format!("manual_handoff_{}", normalize_status_fragment(value))
        }
        ConversationApprovalReviewStatus::Unknown(value) => {
            format!("review_{}", normalize_status_fragment(value))
        }
    }
}

fn approval_review_summary_label(status: &ConversationApprovalReviewStatus) -> String {
    match status {
        ConversationApprovalReviewStatus::InProgress => "reviewing".to_string(),
        ConversationApprovalReviewStatus::Approved => "approved".to_string(),
        ConversationApprovalReviewStatus::Denied => "denied".to_string(),
        ConversationApprovalReviewStatus::Aborted => "aborted".to_string(),
        ConversationApprovalReviewStatus::Unknown(value) => humanize_protocol_status(value),
    }
}

fn requires_manual_handoff(status: &ConversationApprovalReviewStatus) -> bool {
    matches!(
        status,
        ConversationApprovalReviewStatus::Unknown(value)
            if humanize_protocol_status(value).contains("human review")
    )
}

fn normalize_status_fragment(value: &str) -> String {
    humanize_protocol_status(value).replace(' ', "_")
}

fn humanize_protocol_status(value: &str) -> String {
    let mut normalized = String::new();
    let mut previous_was_separator = false;
    let mut previous_was_lower_or_digit = false;
    for ch in value.chars() {
        if ch == '-' || ch == '_' || ch.is_whitespace() {
            if !normalized.is_empty() && !previous_was_separator {
                normalized.push(' ');
            }
            previous_was_separator = true;
            previous_was_lower_or_digit = false;
            continue;
        }
        if ch.is_uppercase() && previous_was_lower_or_digit && !normalized.ends_with(' ') {
            normalized.push(' ');
        }
        normalized.extend(ch.to_lowercase());
        previous_was_separator = false;
        previous_was_lower_or_digit = ch.is_lowercase() || ch.is_ascii_digit();
    }
    normalized.trim().to_string()
}
