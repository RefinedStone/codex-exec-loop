use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::{Position, Rect, Size};
use ratatui::text::Line;

use super::super::MAX_CONVERSATION_HISTORY_LINES;
use super::super::history_insertion::{
    HistoryInsertionAdapter, HistoryInsertionMode, count_rendered_history_rows,
};

#[derive(Default)]
pub(crate) struct HistoryFlushState {
    pub(crate) rendered_lines: Vec<Line<'static>>,
    pub(crate) pending_history_lines: Vec<Line<'static>>,
    pub(crate) visible_history_rows: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HistoryFlushResult {
    inserted_rows: u16,
}

impl HistoryFlushResult {
    pub(crate) fn inserted(self) -> bool {
        self.inserted_rows > 0
    }
}

const MIN_SHIFTED_HISTORY_OVERLAP: usize = 8;

impl HistoryFlushState {
    pub(crate) fn fit_visible_rows_to_viewport<B: Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        terminal_size: Size,
        viewport_area: Rect,
    ) -> Result<bool, B::Error> {
        let viewport_top = viewport_area.top();
        if self.visible_history_rows <= viewport_top {
            return Ok(false);
        }

        let overflow_rows = self.visible_history_rows - viewport_top;
        terminal.backend_mut().set_cursor_position(Position {
            x: 0,
            y: terminal_size.height.saturating_sub(1),
        })?;
        terminal.backend_mut().append_lines(overflow_rows)?;
        self.visible_history_rows = viewport_top;
        Ok(true)
    }

    pub(crate) fn sync<B: Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        current_lines: &[Line<'static>],
        insert_mode: HistoryInsertionMode,
    ) -> Result<HistoryFlushResult, B::Error> {
        self.pending_history_lines = self.pending_lines(current_lines);
        let terminal_size = terminal.size()?;
        let width = terminal_size.width;
        let inserted_rows = if self.pending_history_lines.is_empty() {
            0
        } else {
            count_rendered_history_rows(&self.pending_history_lines, width).min(u16::MAX as usize)
                as u16
        };
        if inserted_rows > 0 {
            HistoryInsertionAdapter::new(insert_mode).insert_with_rendered_rows(
                terminal,
                &self.pending_history_lines,
                inserted_rows,
            )?;
        }
        let viewport_top_after_insert = terminal.get_frame().area().top();
        if current_lines.is_empty() {
            self.visible_history_rows = 0;
        } else if inserted_rows > 0 {
            self.visible_history_rows = if self.pending_history_lines.len() == current_lines.len() {
                inserted_rows.min(viewport_top_after_insert)
            } else {
                self.visible_history_rows
                    .saturating_add(inserted_rows)
                    .min(viewport_top_after_insert)
            };
        }
        self.remember(current_lines);
        self.pending_history_lines.clear();
        Ok(HistoryFlushResult { inserted_rows })
    }

    pub(crate) fn remember_without_flush(&mut self, current_lines: &[Line<'static>]) {
        if current_lines.is_empty() {
            self.visible_history_rows = 0;
        }
        self.pending_history_lines.clear();
        self.remember(current_lines);
    }

    fn remember(&mut self, current_lines: &[Line<'static>]) {
        self.rendered_lines = current_lines.to_vec();
    }

    pub(crate) fn pending_lines(&self, current_lines: &[Line<'static>]) -> Vec<Line<'static>> {
        if current_lines.is_empty() {
            return Vec::new();
        }

        if current_lines.starts_with(self.rendered_lines.as_slice()) {
            return current_lines[self.rendered_lines.len()..].to_vec();
        }

        if let Some(overlap_len) = self.shifted_window_overlap_len(current_lines) {
            return current_lines[overlap_len..].to_vec();
        }

        current_lines.to_vec()
    }

    fn shifted_window_overlap_len(&self, current_lines: &[Line<'static>]) -> Option<usize> {
        if current_lines.len() != MAX_CONVERSATION_HISTORY_LINES {
            return None;
        }

        let max_overlap = self.rendered_lines.len().min(current_lines.len());
        if max_overlap < MIN_SHIFTED_HISTORY_OVERLAP {
            return None;
        }

        (MIN_SHIFTED_HISTORY_OVERLAP..=max_overlap)
            .rev()
            .find(|overlap_len| {
                self.rendered_lines[self.rendered_lines.len() - overlap_len..]
                    == current_lines[..*overlap_len]
            })
    }
}
