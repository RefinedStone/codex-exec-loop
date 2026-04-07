use super::DEFAULT_TRANSCRIPT_PAGE_STEP;

#[derive(Debug, Clone)]
pub(super) struct TranscriptViewportState {
    manual_scroll_offset: Option<u16>,
    page_step: u16,
    last_max_scroll_offset: u16,
}

impl Default for TranscriptViewportState {
    fn default() -> Self {
        Self {
            manual_scroll_offset: None,
            page_step: DEFAULT_TRANSCRIPT_PAGE_STEP,
            last_max_scroll_offset: 0,
        }
    }
}

impl TranscriptViewportState {
    pub(super) fn sync_metrics(&mut self, max_scroll_offset: u16, visible_height: u16) {
        self.last_max_scroll_offset = max_scroll_offset;
        self.page_step = visible_height.saturating_sub(1).max(1);

        if let Some(offset) = self.manual_scroll_offset {
            if max_scroll_offset == 0 || offset >= max_scroll_offset {
                self.manual_scroll_offset = None;
            }
        }
    }

    pub(super) fn current_scroll_offset(&self) -> u16 {
        self.manual_scroll_offset
            .unwrap_or(self.last_max_scroll_offset)
    }

    pub(super) fn status_label(&self) -> String {
        match self.manual_scroll_offset {
            Some(offset) => format!("manual {offset}/{}", self.last_max_scroll_offset),
            None => "tail".to_string(),
        }
    }

    pub(super) fn scroll_page_up(&mut self) {
        self.scroll_by(-(self.page_step as i32));
    }

    pub(super) fn scroll_page_down(&mut self) {
        self.scroll_by(self.page_step as i32);
    }

    pub(super) fn scroll_to_top(&mut self) {
        if self.last_max_scroll_offset == 0 {
            self.manual_scroll_offset = None;
        } else {
            self.manual_scroll_offset = Some(0);
        }
    }

    pub(super) fn scroll_to_tail(&mut self) {
        self.manual_scroll_offset = None;
    }

    fn scroll_by(&mut self, delta: i32) {
        if self.last_max_scroll_offset == 0 {
            self.manual_scroll_offset = None;
            return;
        }

        let amount = delta.unsigned_abs().min(u16::MAX as u32) as u16;
        let next_offset = if delta.is_negative() {
            self.current_scroll_offset().saturating_sub(amount)
        } else {
            self.current_scroll_offset()
                .saturating_add(amount)
                .min(self.last_max_scroll_offset)
        };

        if next_offset >= self.last_max_scroll_offset {
            self.manual_scroll_offset = None;
        } else {
            self.manual_scroll_offset = Some(next_offset);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TranscriptViewportState;

    #[test]
    fn page_navigation_switches_between_tail_and_manual() {
        let mut state = TranscriptViewportState::default();
        state.sync_metrics(24, 6);

        state.scroll_page_up();
        assert_eq!(state.manual_scroll_offset, Some(19));
        assert_eq!(state.status_label(), "manual 19/24");

        state.scroll_page_down();
        assert_eq!(state.manual_scroll_offset, None);
        assert_eq!(state.status_label(), "tail");
    }

    #[test]
    fn home_and_end_jump_between_top_and_tail() {
        let mut state = TranscriptViewportState::default();
        state.sync_metrics(30, 8);

        state.scroll_to_top();
        assert_eq!(state.manual_scroll_offset, Some(0));
        assert_eq!(state.status_label(), "manual 0/30");

        state.scroll_to_tail();
        assert_eq!(state.manual_scroll_offset, None);
        assert_eq!(state.status_label(), "tail");
    }
}
