use ratatui::layout::{Position, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct InlineTranscriptCardHitArea {
    pub(super) digest: [u8; 32],
    pub(super) area: Rect,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct InlineTranscriptUiState {
    card_digests: Vec<[u8; 32]>,
    card_hit_areas: Vec<InlineTranscriptCardHitArea>,
}

impl InlineTranscriptUiState {
    pub(super) fn bind_cards(
        &mut self,
        card_digests: Vec<[u8; 32]>,
        card_hit_areas: Vec<InlineTranscriptCardHitArea>,
    ) {
        self.card_digests = card_digests;
        self.card_hit_areas = card_hit_areas;
    }

    pub(super) fn clear_card_hit_areas(&mut self) {
        self.card_digests.clear();
        self.card_hit_areas.clear();
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

    pub(super) fn mouse_capture_requested(&self) -> bool {
        !self.card_hit_areas.is_empty()
    }

    #[cfg(test)]
    pub(super) fn card_hit_areas(&self) -> &[InlineTranscriptCardHitArea] {
        &self.card_hit_areas
    }
}
