use crate::domain::conversation::ConversationSnapshot;
use crate::domain::parallel_mode::{ParallelModeAgentLeaseIdentity, ParallelModeAgentRosterEntry};

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
struct ParallelPeekSelectionKey {
    agent_id: String,
    slot_id: String,
    // Thread ids arrive after lease startup, so they are not selection identity.
    lease_identity: ParallelModeAgentLeaseIdentity,
}

impl ParallelPeekSelectionKey {
    fn from_entry(entry: &ParallelModeAgentRosterEntry) -> Option<Self> {
        Some(Self {
            agent_id: entry.agent_id.clone(),
            slot_id: entry.slot_id.clone(),
            lease_identity: entry.lease_identity.clone()?,
        })
    }

    fn matches(&self, entry: &ParallelModeAgentRosterEntry) -> bool {
        self.agent_id == entry.agent_id
            && self.slot_id == entry.slot_id
            && entry.lease_identity.as_ref() == Some(&self.lease_identity)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParallelPeekOverlayUiState {
    step: ParallelPeekOverlayStep,
    // This index is only a navigation anchor. Render and Enter require the key below.
    selected_agent_anchor_index: usize,
    selected_agent_key: Option<ParallelPeekSelectionKey>,
    preview: Option<ParallelPeekConversationPreview>,
    conversation_scroll_from_bottom: usize,
}

impl Default for ParallelPeekOverlayUiState {
    fn default() -> Self {
        Self {
            step: ParallelPeekOverlayStep::AgentList,
            selected_agent_anchor_index: 0,
            selected_agent_key: None,
            preview: None,
            conversation_scroll_from_bottom: 0,
        }
    }
}

impl ParallelPeekOverlayUiState {
    pub fn step(&self) -> ParallelPeekOverlayStep {
        self.step
    }

    pub fn selected_agent_index(
        &self,
        active_agents: &[ParallelModeAgentRosterEntry],
    ) -> Option<usize> {
        self.selected_agent_key
            .as_ref()
            .and_then(|key| unique_matching_index(active_agents, key))
    }

    pub fn preview(&self) -> Option<&ParallelPeekConversationPreview> {
        self.preview.as_ref()
    }

    pub fn conversation_scroll_from_bottom(&self) -> usize {
        self.conversation_scroll_from_bottom
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn select_initial_agent(&mut self, active_agents: &[ParallelModeAgentRosterEntry]) {
        self.select(active_agents, 0);
    }

    pub fn move_selection(&mut self, active_agents: &[ParallelModeAgentRosterEntry], delta: isize) {
        if active_agents.is_empty() {
            return;
        }
        let last = active_agents.len() - 1;
        let navigation_index = self
            .selected_agent_index(active_agents)
            .unwrap_or_else(|| self.selected_agent_anchor_index.min(last));
        let selected_agent_index = if delta < 0 {
            navigation_index.saturating_sub(delta.unsigned_abs())
        } else {
            navigation_index.saturating_add(delta as usize)
        }
        .min(last);
        self.select(active_agents, selected_agent_index);
    }

    pub fn sync_selection(&mut self, active_agents: &[ParallelModeAgentRosterEntry]) {
        if let Some(selected_agent_index) = self.selected_agent_index(active_agents) {
            self.selected_agent_anchor_index = selected_agent_index;
        }
    }

    fn select(&mut self, active_agents: &[ParallelModeAgentRosterEntry], index: usize) {
        self.selected_agent_anchor_index = index.min(active_agents.len().saturating_sub(1));
        self.selected_agent_key = active_agents
            .get(self.selected_agent_anchor_index)
            .and_then(ParallelPeekSelectionKey::from_entry);
    }

    pub fn open_preview(&mut self, preview: ParallelPeekConversationPreview) {
        self.step = ParallelPeekOverlayStep::ConversationPreview;
        self.preview = Some(preview);
        self.conversation_scroll_from_bottom = 0;
    }

    pub fn complete_conversation_load(
        &mut self,
        thread_id: &str,
        result: Result<ConversationSnapshot, String>,
    ) -> bool {
        let Some(preview) = self.preview.as_mut() else {
            return false;
        };
        if preview.thread_id.as_deref() != Some(thread_id) {
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
        true
    }

    pub fn back_to_agent_list(&mut self) {
        self.step = ParallelPeekOverlayStep::AgentList;
        self.preview = None;
        self.conversation_scroll_from_bottom = 0;
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

fn unique_matching_index(
    active_agents: &[ParallelModeAgentRosterEntry],
    key: &ParallelPeekSelectionKey,
) -> Option<usize> {
    let mut matches = active_agents
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| key.matches(entry).then_some(index));
    let index = matches.next()?;
    matches.next().is_none().then_some(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(agent_id: &str) -> ParallelModeAgentRosterEntry {
        agent_with_generation(agent_id, agent_id)
    }

    fn agent_with_generation(agent_id: &str, generation: &str) -> ParallelModeAgentRosterEntry {
        ParallelModeAgentRosterEntry::new(
            agent_id,
            format!("Inspect {agent_id}"),
            format!("slot-{agent_id}"),
            format!("branch-{agent_id}"),
            "running",
            "active",
            "working",
        )
        .with_lease_identity(
            format!("task-{agent_id}"),
            format!("session-{generation}"),
            Some(format!("generation-{generation}")),
        )
    }

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
    fn missing_selection_stays_unresolved_until_explicit_navigation() {
        /*
         * A passive roster shrink must not turn the old positional index into
         * authority for a different lease. Up/Down is the explicit action that
         * may adopt a surviving row.
         */
        let mut state = ParallelPeekOverlayUiState::default();
        let three_agents = vec![agent("a"), agent("b"), agent("c")];
        let two_agents = vec![agent("a"), agent("b")];

        state.select_initial_agent(&three_agents);
        state.move_selection(&three_agents, 5);
        assert_eq!(state.selected_agent_index(&three_agents), Some(2));

        state.sync_selection(&two_agents);
        assert_eq!(state.selected_agent_index(&two_agents), None);

        state.move_selection(&two_agents, -5);
        assert_eq!(state.selected_agent_index(&two_agents), Some(0));
    }

    #[test]
    fn transient_empty_roster_preserves_the_selected_lease_identity() {
        let mut state = ParallelPeekOverlayUiState::default();
        let active_agents = vec![agent("a"), agent("b")];
        state.select_initial_agent(&active_agents);
        state.move_selection(&active_agents, 1);

        state.sync_selection(&[]);
        assert_eq!(state.selected_agent_index(&[]), None);

        state.sync_selection(&active_agents);
        assert_eq!(state.selected_agent_index(&active_agents), Some(1));
    }

    #[test]
    fn selection_follows_a_lease_across_reorder_and_thread_capture() {
        let mut state = ParallelPeekOverlayUiState::default();
        let initial = vec![agent("a"), agent("b")];
        state.select_initial_agent(&initial);
        state.move_selection(&initial, 1);

        let reordered = vec![
            agent("b").with_thread_id(Some("thread-b".to_string())),
            agent("a"),
        ];

        assert_eq!(state.selected_agent_index(&reordered), Some(0));
        state.sync_selection(&reordered);
        assert_eq!(state.selected_agent_index(&reordered), Some(0));
    }

    #[test]
    fn replacement_lease_does_not_inherit_the_previous_selection_identity() {
        let mut state = ParallelPeekOverlayUiState::default();
        let initial = vec![agent("a"), agent("b")];
        state.select_initial_agent(&initial);
        state.move_selection(&initial, 1);

        let replacement = vec![agent("a"), agent_with_generation("b", "replacement")];

        state.sync_selection(&replacement);
        assert_eq!(state.selected_agent_index(&replacement), None);
        assert_eq!(state.selected_agent_index(&initial), Some(1));

        state.move_selection(&replacement, 1);
        assert_eq!(state.selected_agent_index(&replacement), Some(1));
    }

    #[test]
    fn duplicate_lease_keys_fail_closed_instead_of_using_position() {
        let mut state = ParallelPeekOverlayUiState::default();
        let duplicate_agents = vec![agent("duplicate"), agent("duplicate")];

        state.select_initial_agent(&duplicate_agents);
        state.move_selection(&duplicate_agents, 1);

        assert_eq!(state.selected_agent_index(&duplicate_agents), None);
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
    fn conversation_load_updates_only_the_matching_current_preview() {
        let mut state = ParallelPeekOverlayUiState::default();
        state.open_preview(preview());

        let stale_snapshot = ConversationSnapshot {
            thread_id: "thread-stale".to_string(),
            title: "Stale".to_string(),
            cwd: "/tmp/stale".to_string(),
            messages: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
            item_lifecycle: Default::default(),
        };
        assert!(!state.complete_conversation_load("thread-stale", Ok(stale_snapshot),));
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
            item_lifecycle: Default::default(),
        };
        assert!(state.complete_conversation_load("thread-peek", Ok(current_snapshot),));
        assert_eq!(
            state
                .preview()
                .and_then(|preview| preview.snapshot.as_ref())
                .map(|snapshot| snapshot.title.as_str()),
            Some("Current")
        );
        assert_eq!(
            state.preview().map(|preview| preview.status_text.as_str()),
            Some("conversation snapshot loaded")
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
