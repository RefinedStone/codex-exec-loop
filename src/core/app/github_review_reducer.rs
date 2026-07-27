use super::{
    GithubReviewPollCorrelation, GithubReviewPollingSetupCorrelation, GithubReviewPollingSetupMode,
    GithubReviewPollingSetupRequest, GithubReviewPollingSetupResult,
    github_review_polling_target_is_valid,
};
use crate::domain::github_review::{
    GithubPullRequestPollResult, GithubPullRequestPollState, GithubPullRequestTarget,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum GithubReviewPollingSetupAdmission {
    Started {
        correlation: GithubReviewPollingSetupCorrelation,
    },
    Coalesced,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GithubReviewPollAdmission {
    pub(super) setup_correlation: GithubReviewPollingSetupCorrelation,
    pub(super) correlation: GithubReviewPollCorrelation,
    pub(super) previous_state: Option<GithubPullRequestPollState>,
}

#[derive(Debug, Clone)]
pub(super) struct GithubReviewFeatureReducer {
    next_setup_generation: u64,
    setup_request: Option<GithubReviewPollingSetupRequest>,
    active_setup: Option<GithubReviewPollingSetupCorrelation>,
    accepted_setup_correlation: Option<GithubReviewPollingSetupCorrelation>,
    target: Option<GithubPullRequestTarget>,
    cursor: Option<(GithubPullRequestTarget, GithubPullRequestPollState)>,
    next_poll_generation: u64,
    active_poll: Option<GithubReviewPollCorrelation>,
}

impl GithubReviewFeatureReducer {
    pub(super) fn new() -> Self {
        Self {
            next_setup_generation: 1,
            setup_request: None,
            active_setup: None,
            accepted_setup_correlation: None,
            target: None,
            cursor: None,
            next_poll_generation: 1,
            active_poll: None,
        }
    }

    pub(super) fn begin_setup(
        &mut self,
        request: GithubReviewPollingSetupRequest,
    ) -> GithubReviewPollingSetupAdmission {
        if self.setup_request.as_ref() == Some(&request) {
            return GithubReviewPollingSetupAdmission::Coalesced;
        }
        let correlation = GithubReviewPollingSetupCorrelation::new(
            take_generation(
                &mut self.next_setup_generation,
                "GitHub review polling setup",
            ),
            request.workspace_directory.clone(),
        );
        self.setup_request = Some(request);
        self.active_setup = Some(correlation.clone());
        self.accepted_setup_correlation = None;
        self.target = None;
        self.cursor = None;
        self.active_poll = None;
        GithubReviewPollingSetupAdmission::Started { correlation }
    }

    pub(super) fn complete_setup(
        &mut self,
        correlation: &GithubReviewPollingSetupCorrelation,
        mut result: Result<GithubReviewPollingSetupResult, String>,
    ) -> Option<Result<GithubReviewPollingSetupResult, String>> {
        if self.active_setup.as_ref() != Some(correlation) {
            return None;
        }
        self.active_setup = None;
        let request = self
            .setup_request
            .as_ref()
            .expect("an active GitHub review setup must retain its request");
        if request.workspace_directory != correlation.workspace_directory {
            return None;
        }
        if result.as_ref().is_ok_and(|result| {
            matches!(
                result,
                GithubReviewPollingSetupResult::Active { target }
                    if !github_review_polling_target_is_valid(target)
            )
        }) {
            result = Err("GitHub review polling setup returned an invalid target".to_string());
        }
        if let (
            GithubReviewPollingSetupMode::Explicit { target: expected },
            Ok(GithubReviewPollingSetupResult::Active { target: actual }),
        ) = (&request.mode, &result)
            && expected != actual
        {
            result = Err("GitHub review polling setup returned a different target".to_string());
        }

        match &result {
            Ok(GithubReviewPollingSetupResult::Active { target }) => {
                self.accepted_setup_correlation = Some(correlation.clone());
                self.target = Some(target.clone());
            }
            Ok(GithubReviewPollingSetupResult::Disabled) | Err(_) => {
                self.accepted_setup_correlation = None;
                self.target = None;
            }
        }
        Some(result)
    }

    pub(super) fn begin_poll(&mut self) -> Option<GithubReviewPollAdmission> {
        let target = self.target.clone()?;
        let setup_correlation = self.accepted_setup_correlation.clone()?;
        if self.active_poll.is_some() {
            return None;
        }
        let correlation = GithubReviewPollCorrelation::new(
            take_generation(&mut self.next_poll_generation, "GitHub review poll"),
            target.clone(),
        );
        let previous_state = self
            .cursor
            .as_ref()
            .filter(|(cursor_target, _)| cursor_target == &target)
            .map(|(_, state)| state.clone());
        self.active_poll = Some(correlation.clone());
        Some(GithubReviewPollAdmission {
            setup_correlation,
            correlation,
            previous_state,
        })
    }

    pub(super) fn complete_poll(
        &mut self,
        correlation: &GithubReviewPollCorrelation,
        mut result: Result<Box<GithubPullRequestPollResult>, String>,
    ) -> Option<Result<Box<GithubPullRequestPollResult>, String>> {
        if self.active_poll.as_ref() != Some(correlation) {
            return None;
        }
        self.active_poll = None;
        if result
            .as_ref()
            .is_ok_and(|poll| poll.snapshot.target != correlation.target)
        {
            result = Err("GitHub review poll provider returned a different target".to_string());
        }
        if let Ok(poll) = &result {
            self.cursor = Some((correlation.target.clone(), poll.next_state.clone()));
        }
        Some(result)
    }

    #[cfg(test)]
    pub(super) fn exhaust_setup_generation(&mut self) {
        self.next_setup_generation = u64::MAX;
    }

    #[cfg(test)]
    pub(super) fn exhaust_poll_generation(&mut self) {
        self.next_poll_generation = u64::MAX;
    }
}

impl Default for GithubReviewFeatureReducer {
    fn default() -> Self {
        Self::new()
    }
}

fn take_generation(next_generation: &mut u64, operation: &str) -> u64 {
    let generation = *next_generation;
    *next_generation = generation
        .checked_add(1)
        .unwrap_or_else(|| panic!("{operation} generation exhausted"));
    generation
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::github_review::{
        GithubPullRequestActivitySnapshot, GithubPullRequestPollResult,
    };

    fn target(repository: &str, number: u64) -> GithubPullRequestTarget {
        GithubPullRequestTarget::new(repository, number)
    }

    fn setup_request(
        workspace_directory: &str,
        target: GithubPullRequestTarget,
    ) -> GithubReviewPollingSetupRequest {
        GithubReviewPollingSetupRequest::new(
            workspace_directory,
            GithubReviewPollingSetupMode::Explicit { target },
        )
    }

    fn started(
        admission: GithubReviewPollingSetupAdmission,
    ) -> GithubReviewPollingSetupCorrelation {
        let GithubReviewPollingSetupAdmission::Started { correlation } = admission else {
            panic!("expected GitHub review setup to start");
        };
        correlation
    }

    fn poll_result(
        target: GithubPullRequestTarget,
        latest_submitted_at: &str,
    ) -> Box<GithubPullRequestPollResult> {
        Box::new(GithubPullRequestPollResult {
            snapshot: GithubPullRequestActivitySnapshot {
                target,
                title: "Review".to_string(),
                url: "https://example.test/pull/42".to_string(),
                head_branch: "feature".to_string(),
                base_branch: "prerelease".to_string(),
                events: Vec::new(),
            },
            changes: Vec::new(),
            next_state: GithubPullRequestPollState {
                latest_submitted_at: Some(latest_submitted_at.to_string()),
                seen_events_at_latest_timestamp: Vec::new(),
            },
        })
    }

    #[test]
    fn setup_coalesces_and_accepts_only_the_exact_completion_once() {
        let mut reducer = GithubReviewFeatureReducer::new();
        let expected = target("owner/repository", 42);
        let request = setup_request("/workspace", expected.clone());
        let correlation = started(reducer.begin_setup(request.clone()));
        assert_eq!(
            reducer.begin_setup(request),
            GithubReviewPollingSetupAdmission::Coalesced
        );

        let stale = GithubReviewPollingSetupCorrelation::new(99, "/workspace");
        assert!(
            reducer
                .complete_setup(
                    &stale,
                    Ok(GithubReviewPollingSetupResult::Active {
                        target: expected.clone(),
                    }),
                )
                .is_none()
        );
        assert_eq!(
            reducer.complete_setup(
                &correlation,
                Ok(GithubReviewPollingSetupResult::Active {
                    target: expected.clone(),
                }),
            ),
            Some(Ok(GithubReviewPollingSetupResult::Active {
                target: expected,
            }))
        );
        assert!(
            reducer
                .complete_setup(&correlation, Ok(GithubReviewPollingSetupResult::Disabled))
                .is_none()
        );
        assert!(reducer.begin_poll().is_some());
    }

    #[test]
    fn setup_same_request_aba_rejects_superseded_generations() {
        let mut reducer = GithubReviewFeatureReducer::new();
        let target_a = target("owner/a", 1);
        let target_b = target("owner/b", 2);
        let a1 = started(reducer.begin_setup(setup_request("/workspace-a", target_a.clone())));
        let b2 = started(reducer.begin_setup(setup_request("/workspace-b", target_b.clone())));
        let a3 = started(reducer.begin_setup(setup_request("/workspace-a", target_a.clone())));
        assert_eq!((a1.generation, b2.generation, a3.generation), (1, 2, 3));

        assert!(
            reducer
                .complete_setup(
                    &a1,
                    Ok(GithubReviewPollingSetupResult::Active {
                        target: target_a.clone(),
                    }),
                )
                .is_none()
        );
        assert!(
            reducer
                .complete_setup(
                    &b2,
                    Ok(GithubReviewPollingSetupResult::Active { target: target_b }),
                )
                .is_none()
        );
        assert_eq!(
            reducer.complete_setup(
                &a3,
                Ok(GithubReviewPollingSetupResult::Active {
                    target: target_a.clone(),
                }),
            ),
            Some(Ok(GithubReviewPollingSetupResult::Active {
                target: target_a.clone(),
            }))
        );
        let poll = reducer.begin_poll().expect("latest A setup should poll");
        assert_eq!(poll.setup_correlation, a3);
        assert_eq!(poll.correlation.target, target_a);
    }

    #[test]
    fn poll_completion_is_exact_once_and_cursor_survives_failure() {
        let mut reducer = GithubReviewFeatureReducer::new();
        let expected = target("owner/repository", 42);
        let setup = started(reducer.begin_setup(setup_request("/workspace", expected.clone())));
        let _ = reducer
            .complete_setup(
                &setup,
                Ok(GithubReviewPollingSetupResult::Active {
                    target: expected.clone(),
                }),
            )
            .expect("setup should complete");

        let first = reducer.begin_poll().expect("first poll should start");
        assert!(reducer.begin_poll().is_none());
        let stale = GithubReviewPollCorrelation::new(99, expected.clone());
        assert!(
            reducer
                .complete_poll(&stale, Ok(poll_result(expected.clone(), "stale")))
                .is_none()
        );
        let first_result = poll_result(expected.clone(), "2026-07-28T00:00:00Z");
        assert_eq!(
            reducer.complete_poll(&first.correlation, Ok(first_result.clone())),
            Some(Ok(first_result))
        );
        assert!(
            reducer
                .complete_poll(
                    &first.correlation,
                    Ok(poll_result(expected.clone(), "duplicate")),
                )
                .is_none()
        );

        let second = reducer.begin_poll().expect("settled poll should reopen");
        assert_eq!(
            second
                .previous_state
                .as_ref()
                .and_then(|state| state.latest_submitted_at.as_deref()),
            Some("2026-07-28T00:00:00Z")
        );
        assert_eq!(
            reducer.complete_poll(&second.correlation, Err("offline".to_string())),
            Some(Err("offline".to_string()))
        );

        let third = reducer.begin_poll().expect("failed poll should reopen");
        assert_eq!(
            third
                .previous_state
                .as_ref()
                .and_then(|state| state.latest_submitted_at.as_deref()),
            Some("2026-07-28T00:00:00Z")
        );
    }

    #[test]
    fn setup_and_poll_results_fail_closed_on_provider_target_mismatch() {
        let mut reducer = GithubReviewFeatureReducer::new();
        let expected = target("owner/repository", 42);
        let setup = started(reducer.begin_setup(setup_request("/workspace", expected.clone())));
        assert_eq!(
            reducer.complete_setup(
                &setup,
                Ok(GithubReviewPollingSetupResult::Active {
                    target: target("other/repository", 42),
                }),
            ),
            Some(Err(
                "GitHub review polling setup returned a different target".to_string()
            ))
        );
        assert!(reducer.begin_poll().is_none());

        let retry = started(reducer.begin_setup(setup_request("/other", expected.clone())));
        let _ = reducer
            .complete_setup(
                &retry,
                Ok(GithubReviewPollingSetupResult::Active {
                    target: expected.clone(),
                }),
            )
            .expect("retry should complete");
        let poll = reducer.begin_poll().expect("poll should start");
        assert_eq!(
            reducer.complete_poll(
                &poll.correlation,
                Ok(poll_result(target("other/repository", 42), "wrong")),
            ),
            Some(Err(
                "GitHub review poll provider returned a different target".to_string()
            ))
        );
    }

    #[test]
    fn newer_setup_invalidates_prior_poll_and_cursor() {
        let mut reducer = GithubReviewFeatureReducer::new();
        let target_a = target("owner/a", 1);
        let setup_a = started(reducer.begin_setup(setup_request("/workspace-a", target_a.clone())));
        let _ = reducer
            .complete_setup(
                &setup_a,
                Ok(GithubReviewPollingSetupResult::Active {
                    target: target_a.clone(),
                }),
            )
            .expect("A setup should complete");
        let old_poll = reducer.begin_poll().expect("A poll should start");

        let target_b = target("owner/b", 2);
        let setup_b = started(reducer.begin_setup(setup_request("/workspace-b", target_b.clone())));
        assert!(
            reducer
                .complete_poll(
                    &old_poll.correlation,
                    Ok(poll_result(target_a, "superseded")),
                )
                .is_none()
        );
        let _ = reducer
            .complete_setup(
                &setup_b,
                Ok(GithubReviewPollingSetupResult::Active {
                    target: target_b.clone(),
                }),
            )
            .expect("B setup should complete");
        let current = reducer.begin_poll().expect("B poll should start");
        assert_eq!(current.correlation.target, target_b);
        assert!(current.previous_state.is_none());
    }

    #[test]
    #[should_panic(expected = "GitHub review polling setup generation exhausted")]
    fn setup_generation_fails_before_wraparound() {
        let mut reducer = GithubReviewFeatureReducer::new();
        reducer.exhaust_setup_generation();
        let _ = reducer.begin_setup(setup_request("/workspace", target("owner/repository", 42)));
    }

    #[test]
    #[should_panic(expected = "GitHub review poll generation exhausted")]
    fn poll_generation_fails_before_wraparound() {
        let mut reducer = GithubReviewFeatureReducer::new();
        let target = target("owner/repository", 42);
        let setup = started(reducer.begin_setup(setup_request("/workspace", target.clone())));
        let _ = reducer
            .complete_setup(
                &setup,
                Ok(GithubReviewPollingSetupResult::Active { target }),
            )
            .expect("setup should complete");
        reducer.exhaust_poll_generation();
        let _ = reducer.begin_poll();
    }
}
