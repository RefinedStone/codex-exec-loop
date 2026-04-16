use crate::domain::conversation::{
    ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationMessage,
    ConversationMessageKind,
};

const MANUAL_APPROVAL_REVIEW_NOTICE: &str = "approval requires manual review, but the app-server protocol does not yet expose a client approve/deny action";

pub(crate) fn conversation_message_label(message: &ConversationMessage) -> &str {
    message
        .display_label
        .as_deref()
        .unwrap_or_else(|| conversation_message_kind_label(message.kind, message.phase.as_deref()))
}

pub(crate) fn conversation_message_kind_label(
    kind: ConversationMessageKind,
    phase: Option<&str>,
) -> &'static str {
    match kind {
        ConversationMessageKind::User => "You",
        ConversationMessageKind::Agent => match phase {
            Some("commentary") => "Codex Commentary",
            _ => "Codex",
        },
        ConversationMessageKind::Tool => "Tool",
        ConversationMessageKind::Status => "Status",
    }
}

pub(crate) fn approval_review_summary_text(review: &ConversationApprovalReview) -> String {
    match review
        .risk_level
        .as_deref()
        .filter(|risk| !risk.trim().is_empty())
    {
        Some(risk_level) => format!(
            "{} {risk_level}",
            approval_review_summary_label(&review.status)
        ),
        None => approval_review_summary_label(&review.status),
    }
}

pub(crate) fn approval_review_status_text(review: &ConversationApprovalReview) -> String {
    let mut segments = vec![approval_review_status_prefix(&review.status)];

    if !review.target_item_id.trim().is_empty() {
        segments.push(format!("target: {}", review.target_item_id));
    }
    if let Some(risk_level) = review
        .risk_level
        .as_deref()
        .filter(|risk| !risk.trim().is_empty())
    {
        segments.push(format!("risk: {risk_level}"));
    }

    segments.join(" / ")
}

pub(crate) fn approval_review_manual_client_action_notice(
    review: &ConversationApprovalReview,
) -> Option<String> {
    requires_manual_client_action(&review.status).then(|| MANUAL_APPROVAL_REVIEW_NOTICE.to_string())
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

fn approval_review_status_prefix(status: &ConversationApprovalReviewStatus) -> String {
    match status {
        ConversationApprovalReviewStatus::InProgress => "approval review in progress".to_string(),
        ConversationApprovalReviewStatus::Approved => "approval review approved".to_string(),
        ConversationApprovalReviewStatus::Denied => "approval review denied".to_string(),
        ConversationApprovalReviewStatus::Aborted => "approval review aborted".to_string(),
        ConversationApprovalReviewStatus::Unknown(value) => {
            format!("approval review {}", humanize_protocol_status(value))
        }
    }
}

fn requires_manual_client_action(status: &ConversationApprovalReviewStatus) -> bool {
    matches!(
        status,
        ConversationApprovalReviewStatus::Unknown(value)
            if humanize_protocol_status(value).contains("human review")
    )
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

#[cfg(test)]
mod tests {
    use super::{
        approval_review_manual_client_action_notice, approval_review_status_text,
        approval_review_summary_text,
    };
    use crate::domain::conversation::{
        ConversationApprovalReview, ConversationApprovalReviewStatus,
    };

    #[test]
    fn unknown_approval_statuses_are_preserved_as_readable_text() {
        let review = ConversationApprovalReview {
            target_item_id: "command-1".to_string(),
            status: ConversationApprovalReviewStatus::Unknown("needsHumanReview".to_string()),
            risk_level: Some("high".to_string()),
            rationale: None,
        };

        assert_eq!(
            approval_review_summary_text(&review),
            "needs human review high"
        );
        assert_eq!(
            approval_review_status_text(&review),
            "approval review needs human review / target: command-1 / risk: high"
        );
        assert_eq!(
            approval_review_manual_client_action_notice(&review).as_deref(),
            Some(
                "approval requires manual review, but the app-server protocol does not yet expose a client approve/deny action"
            )
        );
    }
}
