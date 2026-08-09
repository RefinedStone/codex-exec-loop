use std::cmp::Ordering;
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

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
            GithubValidationActivityKind::ReviewThread => {
                if activity.body_marker.is_explicit_command() {
                    Self::admit(
                        "review_thread_command",
                        "pull request review thread explicitly requested remediation",
                    )
                } else {
                    match activity.thread_resolved {
                        Some(false) => Self::admit(
                            "review_thread",
                            "unresolved pull request review thread requires remediation",
                        ),
                        Some(true) => ActionableFindingDecision::Ignore(
                            ActionableFindingIgnoreReason::ResolvedThread,
                        ),
                        None => ActionableFindingDecision::Ignore(
                            ActionableFindingIgnoreReason::UntrustedThreadResolution,
                        ),
                    }
                }
            }
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

    /// Reduces review history to the latest effective submission per case-insensitive actor and
    /// target SHA before semantic admission. A paginated review source is deliberately withheld:
    /// an approval on a later page must be able to supersede an earlier change request before any
    /// remediation task is created.
    pub fn effective_activities<'a>(
        &self,
        activities: &'a [GithubValidationActivity],
        reviews_complete: bool,
    ) -> Vec<&'a GithubValidationActivity> {
        let mut effective = Vec::new();
        let mut latest_reviews =
            BTreeMap::<(String, Option<String>), &'a GithubValidationActivity>::new();
        for activity in activities {
            if activity.kind != GithubValidationActivityKind::Review {
                effective.push(activity);
                continue;
            }
            if !reviews_complete {
                continue;
            }
            let Some(actor) = activity.actor.as_ref() else {
                effective.push(activity);
                continue;
            };
            let key = (
                actor.login.to_ascii_lowercase(),
                activity
                    .commit_sha
                    .as_ref()
                    .map(|sha| sha.as_str().to_string()),
            );
            match latest_reviews.get(&key) {
                Some(current) if review_activity_order(activity, current) != Ordering::Greater => {}
                _ => {
                    latest_reviews.insert(key, activity);
                }
            }
        }
        effective.extend(latest_reviews.into_values());
        effective.sort_by(|left, right| {
            left.observed_at
                .cmp(&right.observed_at)
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.id.cmp(&right.id))
        });
        effective
    }
}

fn review_activity_order(
    left: &GithubValidationActivity,
    right: &GithubValidationActivity,
) -> Ordering {
    normalized_timestamp(&left.observed_at)
        .cmp(&normalized_timestamp(&right.observed_at))
        .then_with(|| left.observed_at.cmp(&right.observed_at))
        .then_with(|| left.id.cmp(&right.id))
}

fn normalized_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
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
        let resolved_thread_command = activity(GithubValidationActivityKind::ReviewThread)
            .with_thread_resolved(true)
            .with_body_marker(GithubValidationBodyMarker::AkraFix);

        for candidate in [&changes, &thread, &command, &resolved_thread_command] {
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

    #[test]
    fn latest_review_per_actor_and_sha_supersedes_earlier_change_requests() {
        let policy = ActionableFindingPolicy::default();
        let changes = activity(GithubValidationActivityKind::Review)
            .with_review_state(GithubValidationReviewState::ChangesRequested);
        let approved = GithubValidationActivity::new(
            GithubOpaqueId::new("activity:2"),
            GithubValidationActivityKind::Review,
            "2026-08-10T00:01:00Z",
        )
        .with_commit_sha(GithubCommitSha::new(SHA))
        .with_actor(Some(GithubValidationActor::new(
            "REVIEWER",
            GithubValidationActorKind::User,
        )))
        .with_review_state(GithubValidationReviewState::Approved);

        let activities = [changes, approved];
        let effective = policy.effective_activities(&activities, true);

        assert_eq!(effective.len(), 1);
        assert_eq!(effective[0].id.as_str(), "activity:2");
        assert_eq!(
            policy.decide(effective[0], &GithubCommitSha::new(SHA)),
            ActionableFindingDecision::Ignore(
                ActionableFindingIgnoreReason::NonActionableReviewState
            )
        );
    }

    #[test]
    fn paginated_review_history_is_withheld_until_latest_state_is_known() {
        let policy = ActionableFindingPolicy::default();
        let changes = activity(GithubValidationActivityKind::Review)
            .with_review_state(GithubValidationReviewState::ChangesRequested);

        assert!(policy.effective_activities(&[changes], false).is_empty());
    }
}
