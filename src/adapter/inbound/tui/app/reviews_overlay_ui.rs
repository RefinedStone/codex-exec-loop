use crate::application::port::outbound::review_center_repository_port::{
    ReviewCenterHistoryEntry, ReviewCenterInboxItem, ReviewCenterThreadProjection,
};

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
    pub(super) request_id: u64,
    pub(super) context: ReviewsOverlayContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReviewsOverlayAuthoritySnapshot {
    pub(super) current_thread_reviews: Result<Vec<ReviewCenterThreadProjection>, String>,
    pub(super) pending_inbox: Result<Vec<ReviewCenterInboxItem>, String>,
    pub(super) recent_history: Result<Vec<ReviewCenterHistoryEntry>, String>,
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
    next_request_id: u64,
    projection: ReviewsOverlayProjectionState,
}

impl Default for ReviewsOverlayUiState {
    fn default() -> Self {
        Self {
            next_request_id: 0,
            projection: ReviewsOverlayProjectionState::Idle,
        }
    }
}

impl ReviewsOverlayUiState {
    pub(super) fn begin_load(
        &mut self,
        context: ReviewsOverlayContext,
    ) -> ReviewsOverlayLoadRequest {
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        let request = ReviewsOverlayLoadRequest {
            request_id: self.next_request_id,
            context,
        };
        self.projection = ReviewsOverlayProjectionState::Loading(request.clone());
        request
    }

    pub(super) fn apply_loaded(
        &mut self,
        request: ReviewsOverlayLoadRequest,
        authority: ReviewsOverlayAuthoritySnapshot,
    ) -> bool {
        let request_matches = matches!(
            &self.projection,
            ReviewsOverlayProjectionState::Loading(pending) if pending == &request
        );
        if !request_matches {
            return false;
        }
        self.projection = ReviewsOverlayProjectionState::Ready { request, authority };
        true
    }

    fn is_loading_request(&self, request: &ReviewsOverlayLoadRequest) -> bool {
        matches!(
            &self.projection,
            ReviewsOverlayProjectionState::Loading(pending) if pending == request
        )
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
    pub(super) fn begin_reviews_overlay_load(&mut self) -> ReviewsOverlayLoadRequest {
        let context = self.current_reviews_overlay_context();
        self.reviews_overlay_ui_state.begin_load(context)
    }

    pub(super) fn apply_reviews_overlay_loaded(
        &mut self,
        request: ReviewsOverlayLoadRequest,
        authority: ReviewsOverlayAuthoritySnapshot,
    ) -> ReviewsOverlayLoadCompletion {
        if self.shell_overlay != ShellOverlay::Reviews
            || !self.reviews_overlay_ui_state.is_loading_request(&request)
        {
            return ReviewsOverlayLoadCompletion::Ignored;
        }
        if !self
            .current_reviews_overlay_context()
            .has_same_authority_identity(&request.context)
        {
            return ReviewsOverlayLoadCompletion::ReloadRequired;
        }
        let applied = self
            .reviews_overlay_ui_state
            .apply_loaded(request, authority);
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

    fn current_reviews_overlay_context(&self) -> ReviewsOverlayContext {
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
        let request = app.begin_reviews_overlay_load();
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("test app should have a ready conversation");
        };
        conversation.cwd = "/tmp/other".to_string();
        conversation.draft_workspace_directory = "/tmp/other".to_string();

        assert_eq!(
            app.apply_reviews_overlay_loaded(request.clone(), authority()),
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
            app.apply_reviews_overlay_loaded(request, authority()),
            ReviewsOverlayLoadCompletion::ReloadRequired
        );
    }

    #[test]
    fn leaving_and_reopening_reviews_ignores_the_previous_request() {
        let mut app = test_native_tui_app();
        app.shell_overlay = ShellOverlay::Reviews;
        let stale = app.begin_reviews_overlay_load();
        app.dispatch_shell_chrome(super::super::ShellChromeEvent::HelpOverlayShown);
        assert!(matches!(
            app.reviews_overlay_ui_state.screen_model(),
            ReviewsOverlayScreenModel::Idle
        ));

        app.shell_overlay = ShellOverlay::Reviews;
        let current = app.begin_reviews_overlay_load();
        assert_eq!(
            app.apply_reviews_overlay_loaded(stale, authority()),
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
        let request = state.begin_load(context("/tmp/repo", Some("thread-1")));

        assert!(matches!(
            state.screen_model(),
            ReviewsOverlayScreenModel::Loading(pending) if pending == &request
        ));
        assert!(state.apply_loaded(request.clone(), authority()));
        assert!(matches!(
            state.screen_model(),
            ReviewsOverlayScreenModel::Ready { request: ready, .. } if ready == &request
        ));
    }

    #[test]
    fn stale_request_workspace_and_thread_completions_are_ignored() {
        let mut state = ReviewsOverlayUiState::default();
        let pending = state.begin_load(context("/tmp/repo", Some("thread-2")));
        let stale_requests = [
            ReviewsOverlayLoadRequest {
                request_id: pending.request_id.wrapping_add(1),
                context: pending.context.clone(),
            },
            ReviewsOverlayLoadRequest {
                request_id: pending.request_id,
                context: context("/tmp/other", Some("thread-2")),
            },
            ReviewsOverlayLoadRequest {
                request_id: pending.request_id,
                context: context("/tmp/repo", Some("thread-1")),
            },
        ];

        for stale in stale_requests {
            assert!(!state.apply_loaded(stale, authority()));
            assert!(matches!(
                state.screen_model(),
                ReviewsOverlayScreenModel::Loading(request) if request == &pending
            ));
        }
    }

    #[test]
    fn section_failures_remain_data_in_the_ready_screen_model() {
        let mut state = ReviewsOverlayUiState::default();
        let request = state.begin_load(context("/tmp/repo", None));
        let authority = ReviewsOverlayAuthoritySnapshot {
            current_thread_reviews: Ok(Vec::new()),
            pending_inbox: Err("inbox unavailable".to_string()),
            recent_history: Err("history unavailable".to_string()),
        };

        assert!(state.apply_loaded(request, authority));
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
        let request = state.begin_load(context("/tmp/repo", None));
        let authority = ReviewsOverlayAuthoritySnapshot {
            current_thread_reviews: Ok(Vec::new()),
            pending_inbox: Ok(vec![ReviewCenterInboxItem::new(
                "review-1",
                "thread-1",
                "pending",
                "inbox remains visible",
                "2026-07-16T10:00:00Z",
                "2026-07-16T10:01:00Z",
            )]),
            recent_history: Err("history unavailable".to_string()),
        };
        assert!(state.apply_loaded(request, authority));

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
    fn reset_invalidates_an_in_flight_completion_without_reusing_request_ids() {
        let mut state = ReviewsOverlayUiState::default();
        let stale = state.begin_load(context("/tmp/repo", None));
        state.reset();
        assert!(!state.apply_loaded(stale.clone(), authority()));

        let next = state.begin_load(context("/tmp/repo", None));
        assert!(next.request_id > stale.request_id);
    }
}
