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
pub(super) struct ParallelPeekOverlayUiState {
    step: ParallelPeekOverlayStep,
    selected_agent_index: usize,
    preview: Option<ParallelPeekConversationPreview>,
}

impl Default for ParallelPeekOverlayUiState {
    fn default() -> Self {
        Self {
            step: ParallelPeekOverlayStep::AgentList,
            selected_agent_index: 0,
            preview: None,
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

    pub fn reset(&mut self) {
        *self = Self::default();
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
    }

    pub fn back_to_agent_list(&mut self) {
        self.step = ParallelPeekOverlayStep::AgentList;
        self.preview = None;
    }
}
