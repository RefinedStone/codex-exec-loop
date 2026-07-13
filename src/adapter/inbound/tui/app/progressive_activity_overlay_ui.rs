use super::conversation_model::ProgressiveActivityDetailKind;

const MAX_PROGRESSIVE_ACTIVITY_PAGE_HISTORY: usize = 256;

pub(super) fn parse_progressive_activity_detail_kind(
    argument: &str,
) -> Option<ProgressiveActivityDetailKind> {
    match argument.trim().to_ascii_lowercase().as_str() {
        "diff" => Some(ProgressiveActivityDetailKind::Diff),
        "output" => Some(ProgressiveActivityDetailKind::Output),
        _ => None,
    }
}

const fn next_progressive_activity_detail_kind(
    kind: ProgressiveActivityDetailKind,
) -> ProgressiveActivityDetailKind {
    match kind {
        ProgressiveActivityDetailKind::Diff => ProgressiveActivityDetailKind::Output,
        ProgressiveActivityDetailKind::Output => ProgressiveActivityDetailKind::Diff,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProgressiveActivityOverlayUiState {
    selected_kind: ProgressiveActivityDetailKind,
    current_lifecycle_epoch: Option<u64>,
    current_document_sequence: Option<u64>,
    viewport: Option<(u16, u16)>,
    current_page_start: usize,
    previous_page_stack: Vec<usize>,
    next_page_start: Option<usize>,
}

impl Default for ProgressiveActivityOverlayUiState {
    fn default() -> Self {
        Self {
            selected_kind: ProgressiveActivityDetailKind::Diff,
            current_lifecycle_epoch: None,
            current_document_sequence: None,
            viewport: None,
            current_page_start: 0,
            previous_page_stack: Vec::new(),
            next_page_start: None,
        }
    }
}

impl ProgressiveActivityOverlayUiState {
    pub(super) const fn selected_kind(&self) -> ProgressiveActivityDetailKind {
        self.selected_kind
    }

    #[cfg(test)]
    pub(super) const fn current_document_sequence(&self) -> Option<u64> {
        self.current_document_sequence
    }

    #[cfg(test)]
    pub(super) const fn current_lifecycle_epoch(&self) -> Option<u64> {
        self.current_lifecycle_epoch
    }

    pub(super) const fn current_page_start(&self) -> usize {
        self.current_page_start
    }

    #[cfg(test)]
    pub(super) const fn viewport(&self) -> Option<(u16, u16)> {
        self.viewport
    }

    #[cfg(test)]
    pub(super) fn previous_page_stack(&self) -> &[usize] {
        &self.previous_page_stack
    }

    #[cfg(test)]
    pub(super) const fn next_page_start(&self) -> Option<usize> {
        self.next_page_start
    }

    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(super) fn reset_for_kind(&mut self, selected_kind: ProgressiveActivityDetailKind) {
        *self = Self {
            selected_kind,
            ..Self::default()
        };
    }

    pub(super) fn cycle_kind(&mut self) {
        self.selected_kind = next_progressive_activity_detail_kind(self.selected_kind);
        self.reset_navigation();
    }

    pub(super) fn select_document(&mut self, lifecycle_epoch: u64, sequence: Option<u64>) {
        if self.current_lifecycle_epoch == Some(lifecycle_epoch)
            && self.current_document_sequence == sequence
        {
            return;
        }
        self.current_lifecycle_epoch = Some(lifecycle_epoch);
        self.current_document_sequence = sequence;
        self.reset_page_navigation();
    }

    pub(super) fn sync_viewport(&mut self, width: u16, height: u16) {
        let viewport = Some((width, height));
        if self.viewport == viewport {
            return;
        }
        self.viewport = viewport;
        self.reset_page_navigation();
    }

    pub(super) fn set_page_window(
        &mut self,
        current_page_start: usize,
        next_page_start: Option<usize>,
    ) {
        self.current_page_start = current_page_start;
        self.next_page_start = next_page_start.filter(|next| *next > current_page_start);
    }

    pub(super) fn move_to_next_page(&mut self) -> bool {
        let Some(next_page_start) = self.next_page_start else {
            return false;
        };
        if self.previous_page_stack.len() == MAX_PROGRESSIVE_ACTIVITY_PAGE_HISTORY {
            self.previous_page_stack.remove(0);
        }
        self.previous_page_stack.push(self.current_page_start);
        self.current_page_start = next_page_start;
        self.next_page_start = None;
        true
    }

    pub(super) fn move_to_previous_page(&mut self) -> bool {
        let Some(previous_page_start) = self.previous_page_stack.pop() else {
            return false;
        };
        self.next_page_start = Some(self.current_page_start);
        self.current_page_start = previous_page_start;
        true
    }

    pub(super) fn reset_navigation(&mut self) {
        self.current_lifecycle_epoch = None;
        self.current_document_sequence = None;
        self.reset_page_navigation();
    }

    fn reset_page_navigation(&mut self) {
        self.current_page_start = 0;
        self.previous_page_stack.clear();
        self.next_page_start = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_PROGRESSIVE_ACTIVITY_PAGE_HISTORY, ProgressiveActivityDetailKind,
        ProgressiveActivityOverlayUiState, parse_progressive_activity_detail_kind,
    };

    #[test]
    fn kind_parser_is_closed_and_case_insensitive() {
        assert_eq!(
            parse_progressive_activity_detail_kind(" DIFF "),
            Some(ProgressiveActivityDetailKind::Diff)
        );
        assert_eq!(
            parse_progressive_activity_detail_kind("Output"),
            Some(ProgressiveActivityDetailKind::Output)
        );
        assert_eq!(parse_progressive_activity_detail_kind("all"), None);
    }

    #[test]
    fn kind_switch_resets_document_and_page_navigation() {
        let mut state = ProgressiveActivityOverlayUiState::default();
        state.select_document(1, Some(42));
        state.set_page_window(8, Some(16));
        assert!(state.move_to_next_page());

        state.cycle_kind();

        assert_eq!(state.selected_kind(), ProgressiveActivityDetailKind::Output);
        assert_eq!(state.current_document_sequence(), None);
        assert_eq!(state.current_page_start(), 0);
        assert!(state.previous_page_stack().is_empty());
        assert_eq!(state.next_page_start(), None);
    }

    #[test]
    fn selecting_a_replacement_document_resets_page_state_without_payload_storage() {
        let mut state = ProgressiveActivityOverlayUiState::default();
        state.select_document(1, Some(40));
        state.set_page_window(10, Some(20));

        state.select_document(1, Some(39));
        assert_eq!(state.current_document_sequence(), Some(39));
        assert_eq!(state.current_page_start(), 0);
        assert_eq!(state.next_page_start(), None);
        assert!(!format!("{state:?}").contains("raw activity detail"));
    }

    #[test]
    fn viewport_change_resets_page_boundaries_but_keeps_document_identity() {
        let mut state = ProgressiveActivityOverlayUiState::default();
        state.select_document(1, Some(40));
        state.sync_viewport(80, 12);
        state.set_page_window(0, Some(20));
        assert!(state.move_to_next_page());

        state.sync_viewport(48, 3);

        assert_eq!(state.current_document_sequence(), Some(40));
        assert_eq!(state.viewport(), Some((48, 3)));
        assert_eq!(state.current_page_start(), 0);
        assert!(state.previous_page_stack().is_empty());
        assert_eq!(state.next_page_start(), None);
    }

    #[test]
    fn lifecycle_epoch_change_resets_same_sequence_document_navigation() {
        let mut state = ProgressiveActivityOverlayUiState::default();
        state.select_document(1, Some(0));
        state.set_page_window(0, Some(20));
        assert!(state.move_to_next_page());

        state.select_document(2, Some(0));

        assert_eq!(state.current_lifecycle_epoch(), Some(2));
        assert_eq!(state.current_document_sequence(), Some(0));
        assert_eq!(state.current_page_start(), 0);
        assert!(state.previous_page_stack().is_empty());
        assert_eq!(state.next_page_start(), None);
    }

    #[test]
    fn page_navigation_round_trips_through_bounded_history() {
        let mut state = ProgressiveActivityOverlayUiState::default();
        state.set_page_window(0, Some(10));
        assert!(state.move_to_next_page());
        state.set_page_window(10, Some(20));
        assert!(state.move_to_next_page());
        assert_eq!(state.current_page_start(), 20);
        assert_eq!(state.previous_page_stack(), &[0, 10]);

        assert!(state.move_to_previous_page());
        assert_eq!(state.current_page_start(), 10);
        assert_eq!(state.next_page_start(), Some(20));
        assert!(state.move_to_previous_page());
        assert_eq!(state.current_page_start(), 0);
        assert!(!state.move_to_previous_page());

        for page in 1..=MAX_PROGRESSIVE_ACTIVITY_PAGE_HISTORY + 20 {
            state.set_page_window(page - 1, Some(page));
            assert!(state.move_to_next_page());
        }
        assert_eq!(
            state.previous_page_stack().len(),
            MAX_PROGRESSIVE_ACTIVITY_PAGE_HISTORY
        );
    }

    #[test]
    fn home_and_close_resets_are_distinct() {
        let mut state = ProgressiveActivityOverlayUiState::default();
        state.reset_for_kind(ProgressiveActivityDetailKind::Output);
        state.select_document(1, Some(9));
        state.set_page_window(4, Some(8));

        state.reset_navigation();
        assert_eq!(state.selected_kind(), ProgressiveActivityDetailKind::Output);
        assert_eq!(state.current_document_sequence(), None);

        state.reset();
        assert_eq!(state, ProgressiveActivityOverlayUiState::default());
    }
}
