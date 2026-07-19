use crate::core::app::ReviewCenterLoadCorrelation;
pub(super) use crate::core::app::ReviewCenterSnapshot as ReviewsOverlayAuthoritySnapshot;

use super::{ConversationState, NativeTuiApp, ShellOverlay};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReviewsOverlayThreadContext {
    pub(super) thread_id: String,
    pub(super) review_summary: Option<String>,
    pub(super) manual_handoff_context: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReviewsOverlayContext {
    pub(super) workspace_directory: String,
    pub(super) active_thread: Option<ReviewsOverlayThreadContext>,
}

impl ReviewsOverlayContext {
    fn has_same_authority_identity(&self, other: &Self) -> bool {
        self.workspace_directory == other.workspace_directory
            && self.active_thread.as_ref().map(|thread| &thread.thread_id)
                == other.active_thread.as_ref().map(|thread| &thread.thread_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReviewsOverlayLoadRequest {
    pub(super) correlation: ReviewCenterLoadCorrelation,
    pub(super) context: ReviewsOverlayContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReviewsOverlayLoadCompletion {
    Applied,
    Ignored,
    ReloadRequired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReviewsOverlayProjectionState {
    Idle,
    Loading(ReviewsOverlayLoadRequest),
    Ready {
        request: ReviewsOverlayLoadRequest,
        authority: ReviewsOverlayAuthoritySnapshot,
    },
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ReviewsOverlayScreenModel<'a> {
    Idle,
    Loading(&'a ReviewsOverlayLoadRequest),
    Ready {
        request: &'a ReviewsOverlayLoadRequest,
        authority: &'a ReviewsOverlayAuthoritySnapshot,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReviewsOverlayUiState {
    projection: ReviewsOverlayProjectionState,
}

impl Default for ReviewsOverlayUiState {
    fn default() -> Self {
        Self {
            projection: ReviewsOverlayProjectionState::Idle,
        }
    }
}

impl ReviewsOverlayUiState {
    pub(super) fn begin_load(
        &mut self,
        correlation: ReviewCenterLoadCorrelation,
        context: ReviewsOverlayContext,
    ) -> ReviewsOverlayLoadRequest {
        let request = ReviewsOverlayLoadRequest {
            correlation,
            context,
        };
        self.projection = ReviewsOverlayProjectionState::Loading(request.clone());
        request
    }

    pub(super) fn apply_loaded(
        &mut self,
        correlation: ReviewCenterLoadCorrelation,
        authority: ReviewsOverlayAuthoritySnapshot,
    ) -> bool {
        let ReviewsOverlayProjectionState::Loading(request) = &self.projection else {
            return false;
        };
        if request.correlation != correlation {
            return false;
        }
        let request = request.clone();
        self.projection = ReviewsOverlayProjectionState::Ready { request, authority };
        true
    }

    fn loading_request(
        &self,
        correlation: &ReviewCenterLoadCorrelation,
    ) -> Option<&ReviewsOverlayLoadRequest> {
        match &self.projection {
            ReviewsOverlayProjectionState::Loading(request)
                if request.correlation == *correlation =>
            {
                Some(request)
            }
            ReviewsOverlayProjectionState::Idle
            | ReviewsOverlayProjectionState::Loading(_)
            | ReviewsOverlayProjectionState::Ready { .. } => None,
        }
    }

    fn requires_authority_load_for(&self, context: &ReviewsOverlayContext) -> bool {
        match &self.projection {
            ReviewsOverlayProjectionState::Loading(request)
            | ReviewsOverlayProjectionState::Ready { request, .. } => {
                !request.context.has_same_authority_identity(context)
            }
            ReviewsOverlayProjectionState::Idle => true,
        }
    }

    pub(super) fn screen_model(&self) -> ReviewsOverlayScreenModel<'_> {
        match &self.projection {
            ReviewsOverlayProjectionState::Idle => ReviewsOverlayScreenModel::Idle,
            ReviewsOverlayProjectionState::Loading(request) => {
                ReviewsOverlayScreenModel::Loading(request)
            }
            ReviewsOverlayProjectionState::Ready { request, authority } => {
                ReviewsOverlayScreenModel::Ready { request, authority }
            }
        }
    }

    pub(super) fn reset(&mut self) {
        self.projection = ReviewsOverlayProjectionState::Idle;
    }
}

impl NativeTuiApp {
    pub(super) fn begin_reviews_overlay_load(
        &mut self,
        correlation: ReviewCenterLoadCorrelation,
    ) -> ReviewsOverlayLoadRequest {
        let context = self.current_reviews_overlay_context();
        self.reviews_overlay_ui_state
            .begin_load(correlation, context)
    }

    pub(super) fn apply_reviews_overlay_loaded(
        &mut self,
        correlation: ReviewCenterLoadCorrelation,
        authority: ReviewsOverlayAuthoritySnapshot,
    ) -> ReviewsOverlayLoadCompletion {
        if self.shell_overlay != ShellOverlay::Reviews {
            return ReviewsOverlayLoadCompletion::Ignored;
        }
        let Some(request) = self
            .reviews_overlay_ui_state
            .loading_request(&correlation)
            .cloned()
        else {
            return ReviewsOverlayLoadCompletion::Ignored;
        };
        if !self
            .current_reviews_overlay_context()
            .has_same_authority_identity(&request.context)
        {
            return ReviewsOverlayLoadCompletion::ReloadRequired;
        }
        let applied = self
            .reviews_overlay_ui_state
            .apply_loaded(correlation, authority);
        debug_assert!(applied);
        ReviewsOverlayLoadCompletion::Applied
    }

    pub(super) fn reviews_overlay_authority_load_required(&self) -> bool {
        if self.shell_overlay != ShellOverlay::Reviews {
            return false;
        }
        self.reviews_overlay_ui_state
            .requires_authority_load_for(&self.current_reviews_overlay_context())
    }

    pub(super) fn current_reviews_overlay_context(&self) -> ReviewsOverlayContext {
        let active_thread = match &self.conversation_state {
            ConversationState::Ready(conversation) if conversation.has_active_thread() => {
                Some(ReviewsOverlayThreadContext {
                    thread_id: conversation.thread_id.clone(),
                    review_summary: conversation
                        .resumed_thread_review_summary()
                        .map(str::to_string),
                    manual_handoff_context: conversation
                        .resumed_thread_review_manual_handoff_context()
                        .map(str::to_string),
                })
            }
            ConversationState::Loading
            | ConversationState::Ready(_)
            | ConversationState::Failed(_) => None,
        };
        ReviewsOverlayContext {
            workspace_directory: self.planning_workspace_directory(),
            active_thread,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::inbound::tui::app::test_helpers::test_native_tui_app;
    use crate::core::app::ReviewCenterInboxItemSnapshot;

    fn context(workspace: &str, thread_id: Option<&str>) -> ReviewsOverlayContext {
        ReviewsOverlayContext {
            workspace_directory: workspace.to_string(),
            active_thread: thread_id.map(|thread_id| ReviewsOverlayThreadContext {
                thread_id: thread_id.to_string(),
                review_summary: Some("review summary".to_string()),
                manual_handoff_context: Some("operator handoff".to_string()),
            }),
        }
    }

    fn authority() -> ReviewsOverlayAuthoritySnapshot {
        ReviewsOverlayAuthoritySnapshot {
            current_thread_reviews: Ok(Vec::new()),
            pending_inbox: Ok(Vec::new()),
            recent_history: Ok(Vec::new()),
        }
    }

    fn correlation_for(
        generation: u64,
        context: &ReviewsOverlayContext,
    ) -> ReviewCenterLoadCorrelation {
        ReviewCenterLoadCorrelation::new(
            generation,
            context.workspace_directory.clone(),
            context
                .active_thread
                .as_ref()
                .map(|thread| thread.thread_id.clone()),
        )
    }

    fn begin_app_load(app: &mut NativeTuiApp, generation: u64) -> ReviewsOverlayLoadRequest {
        let context = app.current_reviews_overlay_context();
        app.begin_reviews_overlay_load(correlation_for(generation, &context))
    }

    #[test]
    fn authority_identity_tracks_workspace_and_thread_but_not_display_metadata() {
        let original = context("/tmp/repo", Some("thread-1"));
        let mut updated_metadata = original.clone();
        let active_thread = updated_metadata
            .active_thread
            .as_mut()
            .expect("active thread should exist");
        active_thread.review_summary = Some("new summary".to_string());
        active_thread.manual_handoff_context = None;

        assert!(original.has_same_authority_identity(&updated_metadata));
        assert!(!original.has_same_authority_identity(&context("/tmp/other", Some("thread-1"))));
        assert!(!original.has_same_authority_identity(&context("/tmp/repo", Some("thread-2"))));
    }

    #[test]
    fn app_requests_reload_when_pending_authority_identity_changes() {
        let mut app = test_native_tui_app();
        app.shell_overlay = ShellOverlay::Reviews;
        let request = begin_app_load(&mut app, 1);
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("test app should have a ready conversation");
        };
        conversation.cwd = "/tmp/other".to_string();
        conversation.draft_workspace_directory = "/tmp/other".to_string();

        assert_eq!(
            app.apply_reviews_overlay_loaded(request.correlation.clone(), authority()),
            ReviewsOverlayLoadCompletion::ReloadRequired
        );

        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("test app should have a ready conversation");
        };
        conversation.record_thread_prepared(
            "thread-2".to_string(),
            "Replacement thread".to_string(),
            "/tmp/root".to_string(),
        );
        assert_eq!(
            app.apply_reviews_overlay_loaded(request.correlation, authority()),
            ReviewsOverlayLoadCompletion::ReloadRequired
        );
    }

    #[test]
    fn leaving_and_reopening_reviews_ignores_the_previous_request() {
        let mut app = test_native_tui_app();
        app.shell_overlay = ShellOverlay::Reviews;
        let stale = begin_app_load(&mut app, 1);
        app.dispatch_shell_chrome(super::super::ShellChromeEvent::HelpOverlayShown);
        assert!(matches!(
            app.reviews_overlay_ui_state.screen_model(),
            ReviewsOverlayScreenModel::Idle
        ));

        app.shell_overlay = ShellOverlay::Reviews;
        let current = begin_app_load(&mut app, 2);
        assert_eq!(
            app.apply_reviews_overlay_loaded(stale.correlation, authority()),
            ReviewsOverlayLoadCompletion::Ignored
        );
        assert!(matches!(
            app.reviews_overlay_ui_state.screen_model(),
            ReviewsOverlayScreenModel::Loading(request) if request == &current
        ));
    }

    #[test]
    fn exact_load_completion_becomes_the_immutable_screen_model() {
        let mut state = ReviewsOverlayUiState::default();
        let context = context("/tmp/repo", Some("thread-1"));
        let request = state.begin_load(correlation_for(1, &context), context);

        assert!(matches!(
            state.screen_model(),
            ReviewsOverlayScreenModel::Loading(pending) if pending == &request
        ));
        assert!(state.apply_loaded(request.correlation.clone(), authority()));
        assert!(matches!(
            state.screen_model(),
            ReviewsOverlayScreenModel::Ready { request: ready, .. } if ready == &request
        ));
    }

    #[test]
    fn stale_request_workspace_and_thread_completions_are_ignored() {
        let mut state = ReviewsOverlayUiState::default();
        let pending_context = context("/tmp/repo", Some("thread-2"));
        let pending = state.begin_load(correlation_for(1, &pending_context), pending_context);
        let other_workspace = context("/tmp/other", Some("thread-2"));
        let other_thread = context("/tmp/repo", Some("thread-1"));
        let stale_requests = [
            ReviewsOverlayLoadRequest {
                correlation: correlation_for(2, &pending.context),
                context: pending.context.clone(),
            },
            ReviewsOverlayLoadRequest {
                correlation: correlation_for(1, &other_workspace),
                context: other_workspace,
            },
            ReviewsOverlayLoadRequest {
                correlation: correlation_for(1, &other_thread),
                context: other_thread,
            },
        ];

        for stale in stale_requests {
            assert!(!state.apply_loaded(stale.correlation, authority()));
            assert!(matches!(
                state.screen_model(),
                ReviewsOverlayScreenModel::Loading(request) if request == &pending
            ));
        }
    }

    #[test]
    fn section_failures_remain_data_in_the_ready_screen_model() {
        let mut state = ReviewsOverlayUiState::default();
        let context = context("/tmp/repo", None);
        let request = state.begin_load(correlation_for(1, &context), context);
        let authority = ReviewsOverlayAuthoritySnapshot {
            current_thread_reviews: Ok(Vec::new()),
            pending_inbox: Err("inbox unavailable".to_string()),
            recent_history: Err("history unavailable".to_string()),
        };

        assert!(state.apply_loaded(request.correlation, authority));
        assert!(matches!(
            state.screen_model(),
            ReviewsOverlayScreenModel::Ready { authority, .. }
                if authority.pending_inbox.as_ref().err().map(String::as_str)
                    == Some("inbox unavailable")
                    && authority.recent_history.as_ref().err().map(String::as_str)
                        == Some("history unavailable")
        ));
    }

    #[test]
    fn partial_section_failure_projects_error_without_hiding_loaded_sections() {
        let mut state = ReviewsOverlayUiState::default();
        let context = context("/tmp/repo", None);
        let request = state.begin_load(correlation_for(1, &context), context);
        let authority = ReviewsOverlayAuthoritySnapshot {
            current_thread_reviews: Ok(Vec::new()),
            pending_inbox: Ok(vec![ReviewCenterInboxItemSnapshot {
                review_id: "review-1".to_string(),
                thread_id: "thread-1".to_string(),
                inbox_state: "pending".to_string(),
                summary: "inbox remains visible".to_string(),
                requested_at: "2026-07-16T10:00:00Z".to_string(),
                last_activity_at: "2026-07-16T10:01:00Z".to_string(),
                handoff_target: None,
            }]),
            recent_history: Err("history unavailable".to_string()),
        };
        assert!(state.apply_loaded(request.correlation, authority));

        let view =
            crate::adapter::inbound::tui::app::shell_presentation::build_reviews_overlay_view(
                state.screen_model(),
            );

        assert_eq!(view.inbox_reviews.len(), 1);
        assert!(
            view.inbox_reviews[0]
                .summary_line
                .to_string()
                .contains("inbox remains visible")
        );
        assert_eq!(view.history_reviews.len(), 1);
        assert_eq!(
            view.history_reviews[0].summary_line.to_string(),
            "Review data unavailable"
        );
        assert!(
            view.history_reviews[0].detail_lines[0]
                .to_string()
                .contains("history unavailable")
        );
    }

    #[test]
    fn reset_invalidates_an_in_flight_completion_and_accepts_a_new_core_correlation() {
        let mut state = ReviewsOverlayUiState::default();
        let context = context("/tmp/repo", None);
        let stale = state.begin_load(correlation_for(1, &context), context.clone());
        state.reset();
        assert!(!state.apply_loaded(stale.correlation.clone(), authority()));

        let next = state.begin_load(correlation_for(2, &context), context);
        assert!(next.correlation.generation > stale.correlation.generation);
    }
}
