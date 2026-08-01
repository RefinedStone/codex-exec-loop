use ratatui::layout::{Position, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TranscriptCardHitArea {
    pub(super) digest: [u8; 32],
    pub(super) area: Rect,
}

/// App-owned viewport state for the fullscreen conversation transcript.
///
/// `top_row` is an absolute wrapped-row anchor. That is intentionally different
/// from a distance-from-tail counter: when new stream rows arrive while the
/// operator is reading older output, the visible rows stay put. Only explicit
/// follow-tail mode tracks the growing end of the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TranscriptViewportUiState {
    top_row: usize,
    max_scroll: usize,
    page_height: usize,
    follow_tail: bool,
    latest_revision: u64,
    seen_revision: u64,
    document_identity: Option<String>,
    card_digests: Vec<[u8; 32]>,
    card_hit_areas: Vec<TranscriptCardHitArea>,
}

impl Default for TranscriptViewportUiState {
    fn default() -> Self {
        Self {
            top_row: 0,
            max_scroll: 0,
            page_height: 1,
            follow_tail: true,
            latest_revision: 0,
            seen_revision: 0,
            document_identity: None,
            card_digests: Vec::new(),
            card_hit_areas: Vec::new(),
        }
    }
}

impl TranscriptViewportUiState {
    pub(super) fn bind_document(&mut self, identity: Option<String>) {
        if self.document_identity == identity {
            return;
        }
        *self = Self {
            document_identity: identity,
            ..Self::default()
        };
    }

    pub(super) fn resolve_frame(
        &mut self,
        content_rows: usize,
        viewport_height: u16,
        transcript_revision: u64,
    ) -> usize {
        self.page_height = usize::from(viewport_height.max(1));
        self.max_scroll = content_rows.saturating_sub(self.page_height);
        self.latest_revision = transcript_revision;
        if self.follow_tail {
            self.top_row = self.max_scroll;
            self.seen_revision = transcript_revision;
        } else {
            self.top_row = self.top_row.min(self.max_scroll);
        }
        self.top_row
    }

    pub(super) fn bind_cards(
        &mut self,
        card_digests: Vec<[u8; 32]>,
        card_hit_areas: Vec<TranscriptCardHitArea>,
    ) {
        self.card_digests = card_digests;
        self.card_hit_areas = card_hit_areas;
    }

    pub(super) fn clear_card_hit_areas(&mut self) {
        self.card_hit_areas.clear();
    }

    pub(super) fn scroll_up(&mut self, rows: usize) -> bool {
        let next = self.top_row.saturating_sub(rows.max(1));
        let changed = next != self.top_row || self.follow_tail;
        self.top_row = next;
        self.follow_tail = false;
        changed
    }

    pub(super) fn scroll_down(&mut self, rows: usize) -> bool {
        let next = self
            .top_row
            .saturating_add(rows.max(1))
            .min(self.max_scroll);
        let next_follows_tail = next == self.max_scroll;
        let changed = next != self.top_row || self.follow_tail != next_follows_tail;
        self.top_row = next;
        self.follow_tail = next_follows_tail;
        if self.follow_tail {
            self.seen_revision = self.latest_revision;
        }
        changed
    }

    pub(super) fn page_up(&mut self) -> bool {
        self.scroll_up(self.page_height.saturating_sub(2).max(1))
    }

    pub(super) fn page_down(&mut self) -> bool {
        self.scroll_down(self.page_height.saturating_sub(2).max(1))
    }

    pub(super) fn jump_to_top(&mut self) -> bool {
        let changed = self.top_row != 0 || self.follow_tail;
        self.top_row = 0;
        self.follow_tail = false;
        changed
    }

    pub(super) fn follow_latest(&mut self) -> bool {
        let changed = self.top_row != self.max_scroll || !self.follow_tail;
        self.top_row = self.max_scroll;
        self.follow_tail = true;
        self.seen_revision = self.latest_revision;
        changed
    }

    pub(super) fn digest_at(&self, column: u16, row: u16) -> Option<[u8; 32]> {
        self.card_hit_areas
            .iter()
            .find(|hit_area| hit_area.area.contains(Position::new(column, row)))
            .map(|hit_area| hit_area.digest)
    }

    pub(super) fn latest_digest(&self) -> Option<[u8; 32]> {
        self.card_digests.last().copied()
    }

    pub(super) fn has_unseen_output(&self) -> bool {
        !self.follow_tail && self.latest_revision > self.seen_revision
    }

    #[cfg(test)]
    pub(super) fn follow_tail(&self) -> bool {
        self.follow_tail
    }

    #[cfg(test)]
    pub(super) fn top_row(&self) -> usize {
        self.top_row
    }

    #[cfg(test)]
    pub(super) fn card_hit_areas(&self) -> &[TranscriptCardHitArea] {
        &self.card_hit_areas
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appended_rows_do_not_move_a_reader_who_left_follow_tail() {
        let mut state = TranscriptViewportUiState::default();
        assert_eq!(state.resolve_frame(100, 20, 1), 80);
        assert!(state.page_up());
        let reading_row = state.top_row();

        assert_eq!(state.resolve_frame(130, 20, 2), reading_row);
        assert!(state.has_unseen_output());
    }

    #[test]
    fn following_reader_tracks_latest_rows_and_marks_them_seen() {
        let mut state = TranscriptViewportUiState::default();
        assert_eq!(state.resolve_frame(100, 20, 1), 80);
        assert_eq!(state.resolve_frame(130, 20, 2), 110);
        assert!(!state.has_unseen_output());
    }

    #[test]
    fn switching_documents_resets_scroll_and_card_geometry() {
        let mut state = TranscriptViewportUiState::default();
        state.bind_document(Some("thread-a".to_string()));
        state.resolve_frame(100, 20, 1);
        state.bind_cards(
            vec![[1; 32]],
            vec![TranscriptCardHitArea {
                digest: [1; 32],
                area: Rect::new(0, 0, 10, 1),
            }],
        );
        state.page_up();

        state.bind_document(Some("thread-b".to_string()));

        assert_eq!(state.top_row(), 0);
        assert!(state.follow_tail());
        assert!(state.card_hit_areas().is_empty());
    }
}
