use super::*;
use crate::domain::issue_masking::plan_issue_masking;
use crate::domain::issue_secret_spans::secret_spans;
use crate::domain::issue_work::IssueSharedCopy;

pub(super) enum AiCopyPurpose {
    ReadinessPreview,
    ReviewSubmission,
}

pub(super) fn masked(
    conventions: &Conventions,
    text: &IssueDraftText,
    purpose: AiCopyPurpose,
) -> Result<IssueSharedCopy, IssueDraftReadinessBlocker> {
    let mut spans: Vec<_> = text
        .spans
        .iter()
        .map(|span| sensitive_span(text, span).ok_or(IssueDraftReadinessBlocker::InvalidSpan))
        .collect::<Result<_, _>>()?;
    // A publish-specific span decision is not a separate AI handoff authorization.
    match purpose {
        AiCopyPurpose::ReadinessPreview => {}
        AiCopyPurpose::ReviewSubmission => {
            for span in &mut spans {
                span.user_decision = None;
            }
        }
    }
    spans.extend(secret_spans(IssueTextField::Title, &text.title));
    spans.extend(secret_spans(IssueTextField::Body, &text.body));
    plan_issue_masking(
        &text.title,
        &text.body,
        &spans,
        conventions
            .policy
            .disclosure_for(IssueDisclosureTarget::AiHandoff),
    )
    .map(|plan| plan.shared_copy)
    .map_err(|error| match error {
        IssueMaskingBlocked::NeedsUserConfirmation { .. } => {
            IssueDraftReadinessBlocker::DisclosureConfirmationRequired
        }
        IssueMaskingBlocked::InvalidSpan { .. } => IssueDraftReadinessBlocker::InvalidSpan,
    })
}
