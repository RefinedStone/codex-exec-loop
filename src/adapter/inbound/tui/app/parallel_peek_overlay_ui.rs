use crate::domain::conversation::ConversationSnapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParallelPeekOverlayStep {
    AgentList,
    ConversationPreview,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParallelPeekConversationPreview {
    pub agent_id: String,
    pub slot_id: String,
    pub task_title: String,
    pub thread_id: Option<String>,
    pub snapshot: Option<ConversationSnapshot>,
    pub status_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingParallelPeekConversationLoad {
    request_id: u64,
    thread_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParallelPeekOverlayUiState {
    step: ParallelPeekOverlayStep,
    selected_agent_index: usize,
    preview: Option<ParallelPeekConversationPreview>,
    conversation_scroll_from_bottom: usize,
    next_load_request_id: u64,
    pending_load: Option<PendingParallelPeekConversationLoad>,
}

impl Default for ParallelPeekOverlayUiState {
    fn default() -> Self {
        Self {
            step: ParallelPeekOverlayStep::AgentList,
            selected_agent_index: 0,
            preview: None,
            conversation_scroll_from_bottom: 0,
            next_load_request_id: 1,
            pending_load: None,
        }
    }
}

impl ParallelPeekOverlayUiState {
    pub fn step(&self) -> ParallelPeekOverlayStep {
        self.step
    }

    pub fn selected_agent_index(&self) -> usize {
        self.selected_agent_index
    }

    pub fn preview(&self) -> Option<&ParallelPeekConversationPreview> {
        self.preview.as_ref()
    }

    pub fn conversation_scroll_from_bottom(&self) -> usize {
        self.conversation_scroll_from_bottom
    }

    pub fn reset(&mut self) {
        let next_load_request_id = self.next_load_request_id;
        *self = Self::default();
        self.next_load_request_id = next_load_request_id;
    }

    pub fn move_selection(&mut self, active_agent_count: usize, delta: isize) {
        if active_agent_count == 0 {
            self.selected_agent_index = 0;
            return;
        }
        let last = active_agent_count - 1;
        self.selected_agent_index = if delta < 0 {
            self.selected_agent_index
                .saturating_sub(delta.unsigned_abs())
        } else {
            self.selected_agent_index.saturating_add(delta as usize)
        }
        .min(last);
    }

    pub fn clamp_selection(&mut self, active_agent_count: usize) {
        self.selected_agent_index = self
            .selected_agent_index
            .min(active_agent_count.saturating_sub(1));
    }

    pub fn open_preview(&mut self, preview: ParallelPeekConversationPreview) {
        self.step = ParallelPeekOverlayStep::ConversationPreview;
        self.preview = Some(preview);
        self.conversation_scroll_from_bottom = 0;
        self.pending_load = None;
    }

    pub fn begin_conversation_load(
        &mut self,
        preview: ParallelPeekConversationPreview,
        thread_id: String,
    ) -> u64 {
        let request_id = self.next_load_request_id;
        self.next_load_request_id = self.next_load_request_id.wrapping_add(1).max(1);
        self.open_preview(preview);
        self.pending_load = Some(PendingParallelPeekConversationLoad {
            request_id,
            thread_id,
        });
        request_id
    }

    pub fn complete_conversation_load(
        &mut self,
        request_id: u64,
        thread_id: &str,
        result: Result<ConversationSnapshot, String>,
    ) -> bool {
        let matches_pending = self.pending_load.as_ref().is_some_and(|pending| {
            pending.request_id == request_id && pending.thread_id == thread_id
        });
        if !matches_pending {
            return false;
        }

        let Some(preview) = self.preview.as_mut() else {
            self.pending_load = None;
            return false;
        };
        if preview.thread_id.as_deref() != Some(thread_id) {
            self.pending_load = None;
            return false;
        }

        match result {
            Ok(snapshot) => {
                preview.snapshot = Some(snapshot);
                preview.status_text = "conversation snapshot loaded".to_string();
            }
            Err(error) => {
                preview.snapshot = None;
                preview.status_text = format!("conversation snapshot failed: {error}");
            }
        }
        self.pending_load = None;
        true
    }

    pub fn back_to_agent_list(&mut self) {
        self.step = ParallelPeekOverlayStep::AgentList;
        self.preview = None;
        self.conversation_scroll_from_bottom = 0;
        self.pending_load = None;
    }

    pub fn scroll_conversation_older(&mut self, row_count: usize) {
        self.conversation_scroll_from_bottom = self
            .conversation_scroll_from_bottom
            .saturating_add(row_count);
    }

    pub fn scroll_conversation_newer(&mut self, row_count: usize) {
        self.conversation_scroll_from_bottom = self
            .conversation_scroll_from_bottom
            .saturating_sub(row_count);
    }

    pub fn scroll_conversation_to_latest(&mut self) {
        self.conversation_scroll_from_bottom = 0;
    }

    pub fn scroll_conversation_to_oldest(&mut self) {
        self.conversation_scroll_from_bottom = usize::MAX;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview() -> ParallelPeekConversationPreview {
        ParallelPeekConversationPreview {
            agent_id: "agent-peek".to_string(),
            slot_id: "slot-2".to_string(),
            task_title: "Inspect parallel transcript".to_string(),
            thread_id: Some("thread-peek".to_string()),
            snapshot: None,
            status_text: "conversation snapshot pending".to_string(),
        }
    }

    #[test]
    fn selection_clamps_to_active_agents_and_empty_roster() {
        /*
         * The picker selection is reused by rendering and Enter dispatch. It
         * must never point past the active roster, even when the roster shrinks
         * while the overlay remains open.
         */
        let mut state = ParallelPeekOverlayUiState::default();

        state.move_selection(3, 5);
        assert_eq!(state.selected_agent_index(), 2);

        state.clamp_selection(2);
        assert_eq!(state.selected_agent_index(), 1);

        state.move_selection(2, -5);
        assert_eq!(state.selected_agent_index(), 0);

        state.move_selection(0, 1);
        assert_eq!(state.selected_agent_index(), 0);
        state.clamp_selection(0);
        assert_eq!(state.selected_agent_index(), 0);
    }

    #[test]
    fn preview_navigation_resets_conversation_state_when_returning_to_picker() {
        /*
         * Esc/Left from the preview returns to the agent picker. The preview
         * payload and scroll position must be dropped together so a later agent
         * selection cannot inherit stale transcript state.
         */
        let mut state = ParallelPeekOverlayUiState::default();

        state.open_preview(preview());
        state.scroll_conversation_older(12);

        assert_eq!(state.step(), ParallelPeekOverlayStep::ConversationPreview);
        assert!(state.preview().is_some());
        assert_eq!(state.conversation_scroll_from_bottom(), 12);

        state.back_to_agent_list();

        assert_eq!(state.step(), ParallelPeekOverlayStep::AgentList);
        assert!(state.preview().is_none());
        assert_eq!(state.conversation_scroll_from_bottom(), 0);
    }

    #[test]
    fn async_preview_load_ignores_results_from_closed_or_replaced_requests() {
        let mut state = ParallelPeekOverlayUiState::default();
        let first_request = state.begin_conversation_load(preview(), "thread-peek".to_string());
        state.back_to_agent_list();
        let second_request = state.begin_conversation_load(preview(), "thread-peek".to_string());
        assert_ne!(first_request, second_request);

        let stale_snapshot = ConversationSnapshot {
            thread_id: "thread-peek".to_string(),
            title: "Stale".to_string(),
            cwd: "/tmp/stale".to_string(),
            messages: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
        };
        assert!(!state.complete_conversation_load(
            first_request,
            "thread-peek",
            Ok(stale_snapshot),
        ));
        assert!(
            state
                .preview()
                .is_some_and(|preview| preview.snapshot.is_none())
        );

        let current_snapshot = ConversationSnapshot {
            thread_id: "thread-peek".to_string(),
            title: "Current".to_string(),
            cwd: "/tmp/current".to_string(),
            messages: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
        };
        assert!(state.complete_conversation_load(
            second_request,
            "thread-peek",
            Ok(current_snapshot),
        ));
        assert_eq!(
            state
                .preview()
                .and_then(|preview| preview.snapshot.as_ref())
                .map(|snapshot| snapshot.title.as_str()),
            Some("Current")
        );
    }

    #[test]
    fn conversation_scroll_controls_saturate_and_support_edge_jumps() {
        /*
         * Runtime key handling maps PageUp/PageDown/Home/End to these helpers.
         * Saturating math keeps repeated key presses from underflowing or
         * wrapping the preview scroll state.
         */
        let mut state = ParallelPeekOverlayUiState::default();
        state.open_preview(preview());

        state.scroll_conversation_older(10);
        state.scroll_conversation_newer(3);
        assert_eq!(state.conversation_scroll_from_bottom(), 7);

        state.scroll_conversation_newer(20);
        assert_eq!(state.conversation_scroll_from_bottom(), 0);

        state.scroll_conversation_older(1);
        state.scroll_conversation_to_latest();
        assert_eq!(state.conversation_scroll_from_bottom(), 0);

        state.scroll_conversation_to_oldest();
        assert_eq!(state.conversation_scroll_from_bottom(), usize::MAX);
    }
}
