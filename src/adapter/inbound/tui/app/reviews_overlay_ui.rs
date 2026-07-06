/*
 * Reviews overlay UI state stays presentation-local. The domain review-center
 * projection lives elsewhere; this struct only preserves transient widget focus
 * for the future TUI surface.
 */
#[allow(dead_code)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ReviewsOverlayUiState {
    selected_review_index: usize,
}

#[allow(dead_code)]
impl ReviewsOverlayUiState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}
