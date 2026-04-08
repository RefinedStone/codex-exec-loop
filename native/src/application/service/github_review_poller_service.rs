use std::sync::Arc;

use anyhow::Result;

use crate::application::port::outbound::github_review_poller_port::GithubReviewPollerPort;
use crate::domain::github_review::{
    GithubPullRequestActivityEvent, GithubPullRequestActivitySnapshot, GithubPullRequestPollResult,
    GithubPullRequestPollState, GithubPullRequestTarget,
};

#[derive(Clone)]
pub struct GithubReviewPollerService {
    github_review_poller_port: Arc<dyn GithubReviewPollerPort>,
}

impl GithubReviewPollerService {
    pub fn new(github_review_poller_port: Arc<dyn GithubReviewPollerPort>) -> Self {
        Self {
            github_review_poller_port,
        }
    }

    pub fn load_snapshot(
        &self,
        target: &GithubPullRequestTarget,
    ) -> Result<GithubPullRequestActivitySnapshot> {
        let mut snapshot = self
            .github_review_poller_port
            .load_pull_request_activity(target)?;
        snapshot.sort_events();
        Ok(snapshot)
    }

    pub fn poll(
        &self,
        target: &GithubPullRequestTarget,
        previous_state: Option<&GithubPullRequestPollState>,
    ) -> Result<GithubPullRequestPollResult> {
        let snapshot = self.load_snapshot(target)?;
        let changes = Self::collect_changes(&snapshot.events, previous_state);
        let next_state = GithubPullRequestPollState::from_snapshot(&snapshot);

        Ok(GithubPullRequestPollResult {
            snapshot,
            changes,
            next_state,
        })
    }

    fn collect_changes(
        events: &[GithubPullRequestActivityEvent],
        previous_state: Option<&GithubPullRequestPollState>,
    ) -> Vec<GithubPullRequestActivityEvent> {
        let Some(previous_state) = previous_state else {
            return Vec::new();
        };

        let Some(latest_submitted_at) = previous_state.latest_submitted_at.as_ref() else {
            return events.to_vec();
        };

        events
            .iter()
            .filter(|event| {
                event.submitted_at > *latest_submitted_at
                    || (event.submitted_at == *latest_submitted_at
                        && !previous_state
                            .seen_events_at_latest_timestamp
                            .contains(&event.identity()))
            })
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::Result;

    use super::GithubReviewPollerService;
    use crate::application::port::outbound::github_review_poller_port::GithubReviewPollerPort;
    use crate::domain::github_review::{
        GithubPullRequestActivityEvent, GithubPullRequestActivityKind,
        GithubPullRequestActivitySnapshot, GithubPullRequestPollState, GithubPullRequestTarget,
    };

    struct FakeGithubReviewPollerPort {
        snapshot: GithubPullRequestActivitySnapshot,
    }

    impl GithubReviewPollerPort for FakeGithubReviewPollerPort {
        fn load_pull_request_activity(
            &self,
            _target: &GithubPullRequestTarget,
        ) -> Result<GithubPullRequestActivitySnapshot> {
            Ok(self.snapshot.clone())
        }
    }

    #[test]
    fn first_poll_establishes_baseline_without_replaying_existing_activity() {
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        let service = GithubReviewPollerService::new(Arc::new(FakeGithubReviewPollerPort {
            snapshot: snapshot(
                target.clone(),
                vec![
                    event(
                        100,
                        GithubPullRequestActivityKind::IssueComment,
                        "2026-04-08T09:00:00Z",
                    ),
                    event(
                        101,
                        GithubPullRequestActivityKind::Review,
                        "2026-04-08T10:00:00Z",
                    ),
                ],
            ),
        }));

        let result = service.poll(&target, None).expect("poll should succeed");

        assert!(result.changes.is_empty());
        assert_eq!(result.next_state, result.snapshot.poll_state());
    }

    #[test]
    fn poll_returns_only_events_after_previous_cursor() {
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        let service = GithubReviewPollerService::new(Arc::new(FakeGithubReviewPollerPort {
            snapshot: snapshot(
                target.clone(),
                vec![
                    event(
                        100,
                        GithubPullRequestActivityKind::IssueComment,
                        "2026-04-08T09:00:00Z",
                    ),
                    event(
                        101,
                        GithubPullRequestActivityKind::Review,
                        "2026-04-08T10:00:00Z",
                    ),
                    event(
                        201,
                        GithubPullRequestActivityKind::ReviewComment,
                        "2026-04-08T10:30:00Z",
                    ),
                    event(
                        202,
                        GithubPullRequestActivityKind::IssueComment,
                        "2026-04-08T11:00:00Z",
                    ),
                ],
            ),
        }));
        let previous_state = GithubPullRequestPollState {
            latest_submitted_at: Some("2026-04-08T10:00:00Z".to_string()),
            seen_events_at_latest_timestamp: vec![
                event(
                    101,
                    GithubPullRequestActivityKind::Review,
                    "2026-04-08T10:00:00Z",
                )
                .identity(),
            ],
        };

        let result = service
            .poll(&target, Some(&previous_state))
            .expect("poll should succeed");

        assert_eq!(result.changes.len(), 2);
        assert_eq!(result.changes[0].id, 201);
        assert_eq!(result.changes[1].id, 202);
    }

    #[test]
    fn poll_sorts_unsorted_port_responses_before_diffing() {
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        let service = GithubReviewPollerService::new(Arc::new(FakeGithubReviewPollerPort {
            snapshot: snapshot(
                target.clone(),
                vec![
                    event(
                        301,
                        GithubPullRequestActivityKind::Review,
                        "2026-04-08T12:00:00Z",
                    ),
                    event(
                        101,
                        GithubPullRequestActivityKind::IssueComment,
                        "2026-04-08T08:00:00Z",
                    ),
                    event(
                        201,
                        GithubPullRequestActivityKind::ReviewComment,
                        "2026-04-08T10:30:00Z",
                    ),
                ],
            ),
        }));
        let previous_state = GithubPullRequestPollState {
            latest_submitted_at: Some("2026-04-08T08:00:00Z".to_string()),
            seen_events_at_latest_timestamp: vec![
                event(
                    101,
                    GithubPullRequestActivityKind::IssueComment,
                    "2026-04-08T08:00:00Z",
                )
                .identity(),
            ],
        };

        let result = service
            .poll(&target, Some(&previous_state))
            .expect("poll should succeed");

        assert_eq!(result.snapshot.events[0].id, 101);
        assert_eq!(result.snapshot.events[1].id, 201);
        assert_eq!(result.snapshot.events[2].id, 301);
        assert_eq!(result.changes.len(), 2);
        assert_eq!(result.changes[0].id, 201);
        assert_eq!(result.changes[1].id, 301);
    }

    #[test]
    fn poll_surfaces_first_activity_after_empty_baseline() {
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        let service = GithubReviewPollerService::new(Arc::new(FakeGithubReviewPollerPort {
            snapshot: snapshot(
                target.clone(),
                vec![event(
                    201,
                    GithubPullRequestActivityKind::ReviewComment,
                    "2026-04-08T10:30:00Z",
                )],
            ),
        }));
        let previous_state = GithubPullRequestPollState::default();

        let result = service
            .poll(&target, Some(&previous_state))
            .expect("poll should succeed");

        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].id, 201);
    }

    #[test]
    fn poll_keeps_new_same_timestamp_events_visible() {
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        let service = GithubReviewPollerService::new(Arc::new(FakeGithubReviewPollerPort {
            snapshot: snapshot(
                target.clone(),
                vec![
                    event(
                        210,
                        GithubPullRequestActivityKind::IssueComment,
                        "2026-04-08T10:30:00Z",
                    ),
                    event(
                        320,
                        GithubPullRequestActivityKind::ReviewComment,
                        "2026-04-08T10:30:00Z",
                    ),
                ],
            ),
        }));
        let previous_state = GithubPullRequestPollState {
            latest_submitted_at: Some("2026-04-08T10:30:00Z".to_string()),
            seen_events_at_latest_timestamp: vec![
                event(
                    210,
                    GithubPullRequestActivityKind::IssueComment,
                    "2026-04-08T10:30:00Z",
                )
                .identity(),
            ],
        };

        let result = service
            .poll(&target, Some(&previous_state))
            .expect("poll should succeed");

        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].id, 320);
    }

    fn snapshot(
        target: GithubPullRequestTarget,
        events: Vec<GithubPullRequestActivityEvent>,
    ) -> GithubPullRequestActivitySnapshot {
        GithubPullRequestActivitySnapshot {
            target,
            title: "Add review polling".to_string(),
            url: "https://github.com/acme/widgets/pull/42".to_string(),
            head_branch: "feature/native-github-poller-port".to_string(),
            base_branch: "prerelease".to_string(),
            events,
        }
    }

    fn event(
        id: u64,
        kind: GithubPullRequestActivityKind,
        submitted_at: &str,
    ) -> GithubPullRequestActivityEvent {
        GithubPullRequestActivityEvent {
            id,
            kind,
            submitted_at: submitted_at.to_string(),
            author_login: "reviewer".to_string(),
            body: "Looks good".to_string(),
            state: None,
            url: format!("https://github.com/acme/widgets/pull/42#{id}"),
            path: None,
        }
    }
}
