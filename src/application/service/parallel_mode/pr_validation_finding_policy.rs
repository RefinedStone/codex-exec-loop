use crate::application::port::outbound::github_pr_validation_port::{
    GithubValidationActivity, GithubValidationActivityKind, GithubValidationReviewState,
};
use crate::domain::github_review::GithubCommitSha;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ActionableFindingIgnoreReason {
    PastRevision,
    MissingOrNonHumanActor,
    SelfAuthored,
    NonActionableReviewState,
    MissingExplicitCommand,
    ResolvedThread,
    UntrustedThreadResolution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ActionableFindingAdmission {
    pub source: &'static str,
    pub summary: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ActionableFindingDecision {
    Ignore(ActionableFindingIgnoreReason),
    Admit(ActionableFindingAdmission),
}

/// Pure semantic admission policy for provider activity. The adapter has already reduced raw
/// bodies to an explicit command marker, so this decision cannot leak or parse provider copy.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct ActionableFindingPolicy {
    self_login: Option<String>,
}

impl ActionableFindingPolicy {
    pub fn new(self_login: Option<&str>) -> Self {
        Self {
            self_login: self_login
                .map(str::trim)
                .filter(|login| !login.is_empty())
                .map(str::to_ascii_lowercase),
        }
    }

    pub fn decide(
        &self,
        activity: &GithubValidationActivity,
        target_sha: &GithubCommitSha,
    ) -> ActionableFindingDecision {
        if activity
            .commit_sha
            .as_ref()
            .is_some_and(|sha| sha != target_sha)
        {
            return ActionableFindingDecision::Ignore(ActionableFindingIgnoreReason::PastRevision);
        }
        let Some(actor) = activity.actor.as_ref().filter(|actor| actor.is_human()) else {
            return ActionableFindingDecision::Ignore(
                ActionableFindingIgnoreReason::MissingOrNonHumanActor,
            );
        };
        if self
            .self_login
            .as_deref()
            .is_some_and(|login| actor.login.eq_ignore_ascii_case(login))
        {
            return ActionableFindingDecision::Ignore(ActionableFindingIgnoreReason::SelfAuthored);
        }

        match activity.kind {
            GithubValidationActivityKind::Review => {
                if activity.review_state == Some(GithubValidationReviewState::ChangesRequested) {
                    Self::admit("review", "pull request review requested changes")
                } else if activity.body_marker.is_explicit_command() {
                    Self::admit(
                        "review_command",
                        "pull request review explicitly requested remediation",
                    )
                } else {
                    ActionableFindingDecision::Ignore(
                        ActionableFindingIgnoreReason::NonActionableReviewState,
                    )
                }
            }
            GithubValidationActivityKind::IssueComment => {
                if activity.body_marker.is_explicit_command() {
                    Self::admit(
                        "issue_comment_command",
                        "pull request command explicitly requested remediation",
                    )
                } else {
                    ActionableFindingDecision::Ignore(
                        ActionableFindingIgnoreReason::MissingExplicitCommand,
                    )
                }
            }
            GithubValidationActivityKind::ReviewThread => match activity.thread_resolved {
                Some(false) => Self::admit(
                    "review_thread",
                    "unresolved pull request review thread requires remediation",
                ),
                Some(true) => {
                    ActionableFindingDecision::Ignore(ActionableFindingIgnoreReason::ResolvedThread)
                }
                None => ActionableFindingDecision::Ignore(
                    ActionableFindingIgnoreReason::UntrustedThreadResolution,
                ),
            },
            GithubValidationActivityKind::ReviewComment => {
                if activity.body_marker.is_explicit_command() {
                    Self::admit(
                        "review_comment_command",
                        "pull request review comment explicitly requested remediation",
                    )
                } else {
                    ActionableFindingDecision::Ignore(
                        ActionableFindingIgnoreReason::MissingExplicitCommand,
                    )
                }
            }
        }
    }

    fn admit(source: &'static str, summary: &'static str) -> ActionableFindingDecision {
        ActionableFindingDecision::Admit(ActionableFindingAdmission { source, summary })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ActionableFindingDecision, ActionableFindingIgnoreReason, ActionableFindingPolicy,
    };
    use crate::application::port::outbound::github_pr_validation_port::{
        GithubValidationActivity, GithubValidationActivityKind, GithubValidationActor,
        GithubValidationActorKind, GithubValidationBodyMarker, GithubValidationReviewState,
    };
    use crate::domain::github_review::{GithubCommitSha, GithubOpaqueId};

    const SHA: &str = "1111111111111111111111111111111111111111";
    const OLD_SHA: &str = "2222222222222222222222222222222222222222";

    fn activity(kind: GithubValidationActivityKind) -> GithubValidationActivity {
        GithubValidationActivity::new(
            GithubOpaqueId::new("activity:1"),
            kind,
            "2026-08-10T00:00:00Z",
        )
        .with_commit_sha(GithubCommitSha::new(SHA))
        .with_actor(Some(GithubValidationActor::new(
            "reviewer",
            GithubValidationActorKind::User,
        )))
    }

    #[test]
    fn approved_lgtm_bot_self_and_resolved_thread_are_not_actionable() {
        let policy = ActionableFindingPolicy::new(Some("akra"));
        let approved = activity(GithubValidationActivityKind::Review)
            .with_review_state(GithubValidationReviewState::Approved);
        assert_eq!(
            policy.decide(&approved, &GithubCommitSha::new(SHA)),
            ActionableFindingDecision::Ignore(
                ActionableFindingIgnoreReason::NonActionableReviewState
            )
        );

        let lgtm = activity(GithubValidationActivityKind::IssueComment);
        assert_eq!(
            policy.decide(&lgtm, &GithubCommitSha::new(SHA)),
            ActionableFindingDecision::Ignore(
                ActionableFindingIgnoreReason::MissingExplicitCommand
            )
        );

        let bot = activity(GithubValidationActivityKind::Review)
            .with_review_state(GithubValidationReviewState::ChangesRequested)
            .with_actor(Some(GithubValidationActor::new(
                "dependabot[bot]",
                GithubValidationActorKind::User,
            )));
        assert_eq!(
            policy.decide(&bot, &GithubCommitSha::new(SHA)),
            ActionableFindingDecision::Ignore(
                ActionableFindingIgnoreReason::MissingOrNonHumanActor
            )
        );

        let authored_by_self = activity(GithubValidationActivityKind::Review)
            .with_review_state(GithubValidationReviewState::ChangesRequested)
            .with_actor(Some(GithubValidationActor::new(
                "AKRA",
                GithubValidationActorKind::User,
            )));
        assert_eq!(
            policy.decide(&authored_by_self, &GithubCommitSha::new(SHA)),
            ActionableFindingDecision::Ignore(ActionableFindingIgnoreReason::SelfAuthored)
        );

        let resolved =
            activity(GithubValidationActivityKind::ReviewThread).with_thread_resolved(true);
        assert_eq!(
            policy.decide(&resolved, &GithubCommitSha::new(SHA)),
            ActionableFindingDecision::Ignore(ActionableFindingIgnoreReason::ResolvedThread)
        );
    }

    #[test]
    fn changes_requested_unresolved_root_and_explicit_human_command_are_actionable() {
        let policy = ActionableFindingPolicy::default();
        let changes = activity(GithubValidationActivityKind::Review)
            .with_review_state(GithubValidationReviewState::ChangesRequested);
        let thread =
            activity(GithubValidationActivityKind::ReviewThread).with_thread_resolved(false);
        let command = activity(GithubValidationActivityKind::IssueComment)
            .with_body_marker(GithubValidationBodyMarker::AkraRemediate);

        for candidate in [&changes, &thread, &command] {
            assert!(matches!(
                policy.decide(candidate, &GithubCommitSha::new(SHA)),
                ActionableFindingDecision::Admit(_)
            ));
        }
    }

    #[test]
    fn past_revision_is_quarantined_before_semantic_admission() {
        let candidate = activity(GithubValidationActivityKind::Review)
            .with_review_state(GithubValidationReviewState::ChangesRequested)
            .with_commit_sha(GithubCommitSha::new(OLD_SHA));
        assert_eq!(
            ActionableFindingPolicy::default().decide(&candidate, &GithubCommitSha::new(SHA)),
            ActionableFindingDecision::Ignore(ActionableFindingIgnoreReason::PastRevision)
        );
    }
}
