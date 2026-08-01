use super::conversation_model::{
    ProgressiveActivityCardKind, ProgressiveActivityDetailKind, ProgressiveActivityExpandState,
};
use ratatui::layout::{Position, Rect};

const MAX_PROGRESSIVE_ACTIVITY_PAGE_HISTORY: usize = 256;

const CARD_FILTER_CYCLE: &[Option<ProgressiveActivityCardKind>] = &[
    None,
    Some(ProgressiveActivityCardKind::Command),
    Some(ProgressiveActivityCardKind::Patch),
    Some(ProgressiveActivityCardKind::Diff),
    Some(ProgressiveActivityCardKind::Mcp),
    Some(ProgressiveActivityCardKind::Plan),
    Some(ProgressiveActivityCardKind::Reason),
    Some(ProgressiveActivityCardKind::Agent),
    Some(ProgressiveActivityCardKind::Terminal),
    Some(ProgressiveActivityCardKind::Token),
    Some(ProgressiveActivityCardKind::Guardian),
    Some(ProgressiveActivityCardKind::Moderation),
    Some(ProgressiveActivityCardKind::Unknown),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProgressiveActivityDiffLineKind {
    Insert,
    Delete,
    Context,
    Metadata,
    Note,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProgressiveActivityDiffContinuation {
    pub(super) kind: ProgressiveActivityDiffLineKind,
    pub(super) line_number: Option<usize>,
    pub(super) line_end: usize,
    pub(super) line_complete: bool,
    pub(super) hidden: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProgressiveActivityDiffCursor {
    pub(super) old_line: usize,
    pub(super) new_line: usize,
    pub(super) old_remaining: usize,
    pub(super) new_remaining: usize,
    pub(super) in_hunk: bool,
    pub(super) seen_hunk: bool,
    pub(super) gutter_width: usize,
    pub(super) continuation: Option<ProgressiveActivityDiffContinuation>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct ProgressiveActivityPageCursor {
    pub(super) byte_offset: usize,
    pub(super) diff: Option<ProgressiveActivityDiffCursor>,
}

impl ProgressiveActivityPageCursor {
    pub(super) const fn at(byte_offset: usize) -> Self {
        Self {
            byte_offset,
            diff: None,
        }
    }
}

pub(super) fn parse_progressive_activity_detail_kind(
    argument: &str,
) -> Option<ProgressiveActivityDetailKind> {
    match argument.trim().to_ascii_lowercase().as_str() {
        "diff" => Some(ProgressiveActivityDetailKind::Diff),
        "output" => Some(ProgressiveActivityDetailKind::Output),
        _ => None,
    }
}

pub(super) fn parse_progressive_activity_card_filter(
    argument: &str,
) -> Option<Option<ProgressiveActivityCardKind>> {
    match argument.trim().to_ascii_lowercase().as_str() {
        "" | "all" | "list" => Some(None),
        "diff" => Some(Some(ProgressiveActivityCardKind::Diff)),
        "output" | "command" | "cmd" => Some(Some(ProgressiveActivityCardKind::Command)),
        "patch" | "file" => Some(Some(ProgressiveActivityCardKind::Patch)),
        "mcp" => Some(Some(ProgressiveActivityCardKind::Mcp)),
        "plan" => Some(Some(ProgressiveActivityCardKind::Plan)),
        "reason" | "reasoning" => Some(Some(ProgressiveActivityCardKind::Reason)),
        "agent" => Some(Some(ProgressiveActivityCardKind::Agent)),
        "terminal" => Some(Some(ProgressiveActivityCardKind::Terminal)),
        "token" | "tokens" => Some(Some(ProgressiveActivityCardKind::Token)),
        "guardian" => Some(Some(ProgressiveActivityCardKind::Guardian)),
        "moderation" => Some(Some(ProgressiveActivityCardKind::Moderation)),
        "unknown" => Some(Some(ProgressiveActivityCardKind::Unknown)),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProgressiveActivityOverlayUiState {
    /// Legacy Diff/Output shortcut used when `:activity diff|output` is requested.
    selected_kind: ProgressiveActivityDetailKind,
    /// Card list filter. `None` means all progressive kinds.
    card_filter: Option<ProgressiveActivityCardKind>,
    selected_card_index: usize,
    list_focus: bool,
    current_lifecycle_epoch: Option<u64>,
    current_document_sequence: Option<u64>,
    viewport: Option<(u16, u16)>,
    current_page_cursor: ProgressiveActivityPageCursor,
    previous_page_stack: Vec<ProgressiveActivityPageCursor>,
    next_page_cursor: Option<ProgressiveActivityPageCursor>,
    expand_state: ProgressiveActivityExpandState,
    card_hit_areas: Vec<ProgressiveActivityCardHitArea>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProgressiveActivityCardHitArea {
    pub(super) card_index: usize,
    pub(super) area: Rect,
}

impl Default for ProgressiveActivityOverlayUiState {
    fn default() -> Self {
        Self {
            selected_kind: ProgressiveActivityDetailKind::Diff,
            card_filter: None,
            selected_card_index: 0,
            list_focus: true,
            current_lifecycle_epoch: None,
            current_document_sequence: None,
            viewport: None,
            current_page_cursor: ProgressiveActivityPageCursor::at(0),
            previous_page_stack: Vec::new(),
            next_page_cursor: None,
            expand_state: ProgressiveActivityExpandState::default(),
            card_hit_areas: Vec::new(),
        }
    }
}

impl ProgressiveActivityOverlayUiState {
    pub(super) const fn selected_kind(&self) -> ProgressiveActivityDetailKind {
        self.selected_kind
    }

    pub(super) const fn card_filter(&self) -> Option<ProgressiveActivityCardKind> {
        self.card_filter
    }

    pub(super) const fn selected_card_index(&self) -> usize {
        self.selected_card_index
    }

    pub(super) const fn list_focus(&self) -> bool {
        self.list_focus
    }

    pub(super) fn expand_state(&self) -> &ProgressiveActivityExpandState {
        &self.expand_state
    }

    pub(super) fn expand_state_mut(&mut self) -> &mut ProgressiveActivityExpandState {
        &mut self.expand_state
    }

    pub(super) fn bind_card_hit_areas(
        &mut self,
        card_hit_areas: Vec<ProgressiveActivityCardHitArea>,
    ) {
        self.card_hit_areas = card_hit_areas;
    }

    pub(super) fn clear_card_hit_areas(&mut self) {
        self.card_hit_areas.clear();
    }

    pub(super) fn card_index_at(&self, column: u16, row: u16) -> Option<usize> {
        self.card_hit_areas
            .iter()
            .find(|hit_area| hit_area.area.contains(Position::new(column, row)))
            .map(|hit_area| hit_area.card_index)
    }

    #[cfg(test)]
    pub(super) fn mouse_capture_requested(&self) -> bool {
        !self.card_hit_areas.is_empty()
    }

    #[cfg(test)]
    pub(super) const fn current_document_sequence(&self) -> Option<u64> {
        self.current_document_sequence
    }

    #[cfg(test)]
    pub(super) const fn current_lifecycle_epoch(&self) -> Option<u64> {
        self.current_lifecycle_epoch
    }

    #[cfg(test)]
    pub(super) const fn current_page_start(&self) -> usize {
        self.current_page_cursor.byte_offset
    }

    pub(super) const fn current_page_cursor(&self) -> ProgressiveActivityPageCursor {
        self.current_page_cursor
    }

    #[cfg(test)]
    pub(super) const fn viewport(&self) -> Option<(u16, u16)> {
        self.viewport
    }

    #[cfg(test)]
    pub(super) fn previous_page_stack(&self) -> Vec<usize> {
        self.previous_page_stack
            .iter()
            .map(|cursor| cursor.byte_offset)
            .collect()
    }

    #[cfg(test)]
    pub(super) fn next_page_start(&self) -> Option<usize> {
        self.next_page_cursor.map(|cursor| cursor.byte_offset)
    }

    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(super) fn reset_for_kind(&mut self, selected_kind: ProgressiveActivityDetailKind) {
        let expand_state = std::mem::take(&mut self.expand_state);
        let card_filter = match selected_kind {
            ProgressiveActivityDetailKind::Diff => Some(ProgressiveActivityCardKind::Diff),
            ProgressiveActivityDetailKind::Output => Some(ProgressiveActivityCardKind::Command),
        };
        *self = Self {
            selected_kind,
            card_filter,
            expand_state,
            list_focus: true,
            ..Self::default()
        };
    }

    pub(super) fn reset_for_card_filter(
        &mut self,
        card_filter: Option<ProgressiveActivityCardKind>,
    ) {
        let expand_state = std::mem::take(&mut self.expand_state);
        let selected_kind = match card_filter {
            Some(ProgressiveActivityCardKind::Diff) => ProgressiveActivityDetailKind::Diff,
            Some(ProgressiveActivityCardKind::Command) => ProgressiveActivityDetailKind::Output,
            _ => ProgressiveActivityDetailKind::Diff,
        };
        *self = Self {
            selected_kind,
            card_filter,
            expand_state,
            list_focus: true,
            ..Self::default()
        };
    }

    pub(super) fn cycle_kind(&mut self) {
        let current = CARD_FILTER_CYCLE
            .iter()
            .position(|entry| *entry == self.card_filter)
            .unwrap_or(0);
        let next = CARD_FILTER_CYCLE[(current + 1) % CARD_FILTER_CYCLE.len()];
        self.card_filter = next;
        self.selected_kind = match next {
            Some(ProgressiveActivityCardKind::Diff) => ProgressiveActivityDetailKind::Diff,
            Some(ProgressiveActivityCardKind::Command) => ProgressiveActivityDetailKind::Output,
            _ => self.selected_kind,
        };
        self.selected_card_index = 0;
        self.list_focus = true;
        self.reset_navigation();
    }

    pub(super) fn clamp_selected_card(&mut self, filtered_len: usize) {
        if filtered_len == 0 {
            self.selected_card_index = 0;
            return;
        }
        if self.selected_card_index >= filtered_len {
            self.selected_card_index = filtered_len - 1;
        }
    }

    pub(super) fn move_card_selection(&mut self, delta: isize, filtered_len: usize) -> bool {
        if filtered_len == 0 {
            self.selected_card_index = 0;
            return false;
        }
        self.clamp_selected_card(filtered_len);
        let next =
            (self.selected_card_index as isize + delta).rem_euclid(filtered_len as isize) as usize;
        if next == self.selected_card_index {
            return false;
        }
        self.selected_card_index = next;
        self.list_focus = true;
        self.reset_page_navigation();
        true
    }

    pub(super) fn select_card(&mut self, index: usize, filtered_len: usize) -> bool {
        if index >= filtered_len {
            return false;
        }
        let changed = self.selected_card_index != index;
        self.selected_card_index = index;
        self.list_focus = true;
        if changed {
            self.reset_page_navigation();
        }
        true
    }

    pub(super) fn focus_detail(&mut self) {
        self.list_focus = false;
    }

    pub(super) fn focus_list(&mut self) {
        self.list_focus = true;
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

    #[cfg(test)]
    pub(super) fn set_page_window(
        &mut self,
        current_page_start: usize,
        next_page_start: Option<usize>,
    ) {
        self.set_page_cursor_window(
            ProgressiveActivityPageCursor::at(current_page_start),
            next_page_start.map(ProgressiveActivityPageCursor::at),
        );
    }

    pub(super) fn set_page_cursor_window(
        &mut self,
        current_page_cursor: ProgressiveActivityPageCursor,
        next_page_cursor: Option<ProgressiveActivityPageCursor>,
    ) {
        self.current_page_cursor = current_page_cursor;
        self.next_page_cursor =
            next_page_cursor.filter(|next| next.byte_offset > current_page_cursor.byte_offset);
    }

    pub(super) fn move_to_next_page(&mut self) -> bool {
        let Some(next_page_cursor) = self.next_page_cursor else {
            return false;
        };
        if self.previous_page_stack.len() == MAX_PROGRESSIVE_ACTIVITY_PAGE_HISTORY {
            self.previous_page_stack.remove(0);
        }
        self.previous_page_stack.push(self.current_page_cursor);
        self.current_page_cursor = next_page_cursor;
        self.next_page_cursor = None;
        self.list_focus = false;
        true
    }

    pub(super) fn move_to_previous_page(&mut self) -> bool {
        let Some(previous_page_cursor) = self.previous_page_stack.pop() else {
            return false;
        };
        self.next_page_cursor = Some(self.current_page_cursor);
        self.current_page_cursor = previous_page_cursor;
        self.list_focus = false;
        true
    }

    pub(super) fn reset_navigation(&mut self) {
        self.current_lifecycle_epoch = None;
        self.current_document_sequence = None;
        self.reset_page_navigation();
    }

    fn reset_page_navigation(&mut self) {
        self.current_page_cursor = ProgressiveActivityPageCursor::at(0);
        self.previous_page_stack.clear();
        self.next_page_cursor = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_PROGRESSIVE_ACTIVITY_PAGE_HISTORY, ProgressiveActivityCardHitArea,
        ProgressiveActivityCardKind, ProgressiveActivityDetailKind,
        ProgressiveActivityDiffContinuation, ProgressiveActivityDiffCursor,
        ProgressiveActivityDiffLineKind, ProgressiveActivityOverlayUiState,
        ProgressiveActivityPageCursor, parse_progressive_activity_card_filter,
        parse_progressive_activity_detail_kind,
    };
    use ratatui::layout::Rect;

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
    fn card_filter_parser_accepts_all_and_kind_aliases() {
        assert_eq!(parse_progressive_activity_card_filter("all"), Some(None));
        assert_eq!(
            parse_progressive_activity_card_filter("command"),
            Some(Some(ProgressiveActivityCardKind::Command))
        );
        assert_eq!(
            parse_progressive_activity_card_filter("output"),
            Some(Some(ProgressiveActivityCardKind::Command))
        );
        assert_eq!(parse_progressive_activity_card_filter("nope"), None);
    }

    #[test]
    fn kind_switch_resets_document_and_page_navigation() {
        let mut state = ProgressiveActivityOverlayUiState::default();
        state.select_document(1, Some(42));
        state.set_page_window(8, Some(16));
        assert!(state.move_to_next_page());

        state.cycle_kind();

        assert_eq!(
            state.card_filter(),
            Some(ProgressiveActivityCardKind::Command)
        );
        assert_eq!(state.current_document_sequence(), None);
        assert_eq!(state.current_page_start(), 0);
        assert!(state.previous_page_stack().is_empty());
        assert_eq!(state.next_page_start(), None);
        assert!(state.list_focus());
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
    fn page_navigation_preserves_diff_parser_and_continuation_cursor() {
        let mut state = ProgressiveActivityOverlayUiState::default();
        let current = ProgressiveActivityPageCursor {
            byte_offset: 40,
            diff: Some(ProgressiveActivityDiffCursor {
                old_line: 12,
                new_line: 14,
                old_remaining: 2,
                new_remaining: 3,
                in_hunk: true,
                seen_hunk: true,
                gutter_width: 3,
                continuation: None,
            }),
        };
        let next = ProgressiveActivityPageCursor {
            byte_offset: 64,
            diff: Some(ProgressiveActivityDiffCursor {
                old_line: 13,
                new_line: 15,
                old_remaining: 1,
                new_remaining: 2,
                in_hunk: true,
                seen_hunk: true,
                gutter_width: 3,
                continuation: Some(ProgressiveActivityDiffContinuation {
                    kind: ProgressiveActivityDiffLineKind::Insert,
                    line_number: Some(14),
                    line_end: 80,
                    line_complete: false,
                    hidden: false,
                }),
            }),
        };
        state.set_page_cursor_window(current, Some(next));

        assert!(state.move_to_next_page());
        assert_eq!(state.current_page_cursor(), next);
        assert!(state.move_to_previous_page());
        assert_eq!(state.current_page_cursor(), current);
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

    #[test]
    fn card_selection_wraps_and_resets_pages() {
        let mut state = ProgressiveActivityOverlayUiState::default();
        state.set_page_window(0, Some(10));
        assert!(state.move_to_next_page());
        assert!(state.move_card_selection(1, 3));
        assert_eq!(state.selected_card_index(), 1);
        assert_eq!(state.current_page_start(), 0);
        assert!(state.list_focus());
        assert!(state.move_card_selection(-1, 3));
        assert_eq!(state.selected_card_index(), 0);
    }

    #[test]
    fn receipt_owned_card_hit_areas_are_exact_and_clearable() {
        let mut state = ProgressiveActivityOverlayUiState::default();
        state.bind_card_hit_areas(vec![
            ProgressiveActivityCardHitArea {
                card_index: 0,
                area: Rect::new(4, 6, 20, 1),
            },
            ProgressiveActivityCardHitArea {
                card_index: 2,
                area: Rect::new(4, 8, 20, 1),
            },
        ]);

        assert!(state.mouse_capture_requested());
        assert_eq!(state.card_index_at(4, 6), Some(0));
        assert_eq!(state.card_index_at(23, 8), Some(2));
        assert_eq!(state.card_index_at(24, 8), None);
        assert_eq!(state.card_index_at(4, 7), None);

        state.clear_card_hit_areas();
        assert!(!state.mouse_capture_requested());
        assert_eq!(state.card_index_at(4, 6), None);
    }
}
