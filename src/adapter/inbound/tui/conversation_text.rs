use crate::domain::conversation::{
    ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationControlSupport,
    ConversationMessage, ConversationMessageKind, ConversationRuntimeControlTruth,
};
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

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

pub(crate) fn approval_review_status_text(
    review: &ConversationApprovalReview,
    approval_support: ConversationControlSupport,
) -> String {
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
    if approval_support != ConversationControlSupport::RuntimeNative {
        segments.push(format!(
            "handling: {}",
            control_support_label(approval_support)
        ));
    }

    segments.join(" / ")
}

pub(crate) fn approval_review_manual_client_action_notice(
    review: &ConversationApprovalReview,
    approval_support: ConversationControlSupport,
) -> Option<String> {
    requires_manual_client_action(&review.status).then(|| match approval_support {
        ConversationControlSupport::RuntimeNative => {
            "approval requires manual review; use the runtime-native approval flow for this bridge"
                .to_string()
        }
        ConversationControlSupport::ManualHandoff => {
            "approval requires manual review; this runtime hands approval back to the operator"
                .to_string()
        }
        ConversationControlSupport::Unsupported => {
            "approval requires manual review, but this runtime does not expose approval handling"
                .to_string()
        }
    })
}

pub(crate) fn interrupt_blocked_status_text(
    interrupt_support: ConversationControlSupport,
) -> String {
    match interrupt_support {
        ConversationControlSupport::RuntimeNative => {
            "turn still running; use the runtime interrupt control before leaving the shell view"
                .to_string()
        }
        ConversationControlSupport::ManualHandoff => {
            "turn still running; interrupt is handed back to the operator outside this shell"
                .to_string()
        }
        ConversationControlSupport::Unsupported => {
            "turn still running; this runtime does not expose interrupt control in the shell"
                .to_string()
        }
    }
}

pub(crate) fn interrupt_requested_status_text(thread_id: Option<&str>) -> String {
    match thread_id
        .map(str::trim)
        .filter(|thread_id| !thread_id.is_empty())
    {
        Some(thread_id) => format!(
            "runtime interrupt requested for {thread_id}; waiting for the active turn to settle"
        ),
        None => "runtime interrupt requested; waiting for the active turn to settle".to_string(),
    }
}

pub(crate) fn interrupt_request_failed_status_text(error: &str) -> String {
    format!("runtime interrupt failed: {error}")
}

pub(crate) fn runtime_control_summary_text(truth: ConversationRuntimeControlTruth) -> String {
    format!(
        "approval: {}  |  interrupt: {}",
        control_support_label(truth.approval),
        control_support_label(truth.interrupt)
    )
}

pub(crate) fn attachment_runtime_notice(profile: TerminalBridgeAttachmentProfile) -> String {
    format!(
        "bridge attachment: {} / recovery: {}",
        profile.mode.label(),
        profile.recovery_anchor.label()
    )
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

pub(crate) fn control_support_label(support: ConversationControlSupport) -> &'static str {
    match support {
        ConversationControlSupport::RuntimeNative => "runtime-native",
        ConversationControlSupport::ManualHandoff => "manual handoff",
        ConversationControlSupport::Unsupported => "unsupported",
    }
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
        approval_review_summary_text, attachment_runtime_notice, interrupt_blocked_status_text,
        interrupt_request_failed_status_text, interrupt_requested_status_text,
        runtime_control_summary_text,
    };
    use crate::domain::conversation::{
        ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationControlSupport,
        ConversationRuntimeControlTruth,
    };
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

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
            approval_review_status_text(&review, ConversationControlSupport::ManualHandoff),
            "approval review needs human review / target: command-1 / risk: high / handling: manual handoff"
        );
        assert_eq!(
            approval_review_manual_client_action_notice(
                &review,
                ConversationControlSupport::ManualHandoff,
            )
            .as_deref(),
            Some(
                "approval requires manual review; this runtime hands approval back to the operator"
            )
        );
    }

    #[test]
    fn control_copy_stays_capability_shaped() {
        assert_eq!(
            interrupt_blocked_status_text(ConversationControlSupport::Unsupported),
            "turn still running; this runtime does not expose interrupt control in the shell"
        );
        assert_eq!(
            interrupt_requested_status_text(Some("%3")),
            "runtime interrupt requested for %3; waiting for the active turn to settle"
        );
        assert_eq!(
            interrupt_request_failed_status_text("transport closed"),
            "runtime interrupt failed: transport closed"
        );
        assert_eq!(
            runtime_control_summary_text(ConversationRuntimeControlTruth::new(
                ConversationControlSupport::ManualHandoff,
                ConversationControlSupport::Unsupported,
            )),
            "approval: manual handoff  |  interrupt: unsupported"
        );
        assert_eq!(
            attachment_runtime_notice(TerminalBridgeAttachmentProfile::codex_app_server_reattach()),
            "bridge attachment: provider-reattach / recovery: provider-thread-id"
        );
    }
}
