#[cfg(test)]
use crate::domain::conversation::ConversationRuntimeControlTruth;
use crate::domain::conversation::{
    ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationControlSupport,
    ConversationMessage, ConversationMessageKind,
};
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

/*
 * conversation_text is the shared copy adapter for conversation-facing TUI
 * surfaces. Domain types preserve protocol facts; transcript rendering, status
 * panels, and runtime notices call this module so speaker names and control
 * capability language do not drift between visible shell regions.
 */
pub(crate) fn conversation_message_label(message: &ConversationMessage) -> &str {
    // App-server items can provide a display_label that is more precise than the
    // local kind enum. Prefer it so transcript replay keeps provider-authored
    // speaker labels while still falling back for synthetic status/tool rows.
    message
        .display_label
        .as_deref()
        .unwrap_or_else(|| conversation_message_kind_label(message.kind, message.phase.as_deref()))
}

pub(crate) fn conversation_message_kind_label(
    kind: ConversationMessageKind,
    phase: Option<&str>,
) -> &'static str {
    // Commentary is still an agent message in the domain model, but operators
    // need it visually separated from final assistant output in transcript tails.
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
    // Footer summaries have room for one compact approval phrase. Include risk
    // only when the provider sent a non-empty value; empty risk fields should not
    // create visual noise or ambiguous trailing punctuation.
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
    // The full status line is diagnostic copy, not just a badge. It includes the
    // target command item and the runtime's handling mode when the TUI cannot
    // complete approval natively.
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
    // Unknown provider statuses are not necessarily errors. Only statuses that
    // normalize to a human-review request need an additional runtime notice that
    // tells the operator where approval must happen.
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
    // Leaving the shell while a turn is running is blocked for different reasons
    // depending on bridge capability. Shape the copy from the capability enum so
    // runtime reducer and intent handling stay aligned.
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

#[cfg(test)]
pub(crate) fn runtime_control_summary_text(truth: ConversationRuntimeControlTruth) -> String {
    // Tests render both control axes together because approval and interrupt can
    // come from different bridge capabilities during reattach or fallback modes.
    format!(
        "approval: {}  |  interrupt: {}",
        control_support_label(truth.approval),
        control_support_label(truth.interrupt)
    )
}

pub(crate) fn attachment_runtime_notice(profile: TerminalBridgeAttachmentProfile) -> String {
    // Attachment notices expose how the shell recovered a provider session. This
    // is runtime detail, not transcript content, so the copy stays terse enough
    // for status panels and startup diagnostics.
    format!(
        "bridge attachment: {} / recovery: {}",
        profile.mode.label(),
        profile.recovery_anchor.label()
    )
}

fn approval_review_summary_label(status: &ConversationApprovalReviewStatus) -> String {
    // The compact label intentionally drops the "approval review" prefix because
    // its callers already render in approval-specific sections.
    match status {
        ConversationApprovalReviewStatus::InProgress => "reviewing".to_string(),
        ConversationApprovalReviewStatus::Approved => "approved".to_string(),
        ConversationApprovalReviewStatus::Denied => "denied".to_string(),
        ConversationApprovalReviewStatus::Aborted => "aborted".to_string(),
        ConversationApprovalReviewStatus::Unknown(value) => humanize_protocol_status(value),
    }
}

fn approval_review_status_prefix(status: &ConversationApprovalReviewStatus) -> String {
    // Status prefixes are standalone sentences for the main conversation status
    // row, so known statuses keep the subject even though summaries omit it.
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
    // Provider strings can arrive in camelCase, snake_case, kebab-case, or words.
    // Normalize before detecting "human review" so runtime notices remain stable
    // across upstream protocol spelling changes.
    matches!(
        status,
        ConversationApprovalReviewStatus::Unknown(value)
            if humanize_protocol_status(value).contains("human review")
    )
}

pub(crate) fn control_support_label(support: ConversationControlSupport) -> &'static str {
    // This label is shared by footer summaries, test-only diagnostics, and
    // approval status copy. Keep the wording capability-shaped rather than
    // action-shaped so it describes the runtime, not a particular command.
    match support {
        ConversationControlSupport::RuntimeNative => "runtime-native",
        ConversationControlSupport::ManualHandoff => "manual handoff",
        ConversationControlSupport::Unsupported => "unsupported",
    }
}

fn humanize_protocol_status(value: &str) -> String {
    /*
     * Preserve unknown provider statuses by making them readable instead of
     * collapsing to "unknown". This keeps forward compatibility visible to
     * operators and lets tests assert the exact bridge status that reached us.
     */
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
        runtime_control_summary_text,
    };
    use crate::domain::conversation::{
        ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationControlSupport,
        ConversationRuntimeControlTruth,
    };
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

    #[test]
    fn unknown_approval_statuses_are_preserved_as_readable_text() {
        // This locks the forward-compatibility contract: provider-specific
        // approval statuses must remain visible, humanized, and capable of
        // triggering manual-action copy when they imply human review.
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
        // Capability labels feed multiple surfaces. The test keeps interrupt,
        // approval, and attachment notices aligned with the same runtime truth.
        assert_eq!(
            interrupt_blocked_status_text(ConversationControlSupport::Unsupported),
            "turn still running; this runtime does not expose interrupt control in the shell"
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
