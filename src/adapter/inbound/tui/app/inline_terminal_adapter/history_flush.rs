use ratatui::Terminal;
use ratatui::layout::{Position, Rect};
use ratatui::text::Line;

use super::super::MAX_CONVERSATION_HISTORY_LINES;
use super::super::history_insertion::{
    HistoryInsertionAdapter, HistoryInsertionMode, count_rendered_history_rows,
};
use super::backend::{InlineResizeBackend, InlineResizeSnapshot};

/*
 * Inline terminal rendering has two histories to keep in sync. Ratatui owns the live frame buffer,
 * while the host terminal scrollback should receive durable transcript rows as the conversation
 * grows. HistoryFlushState is the small reconciliation cache between those worlds: it remembers
 * the transcript snapshot already written to scrollback, computes the new suffix, and tracks how
 * many rendered rows now occupy the space above the inline viewport.
 */
#[derive(Default)]
pub(crate) struct HistoryFlushState {
    /*
     * Last transcript snapshot used as the scrollback baseline. This is stored as owned Line
     * values because the next draw tick must diff against it after the app borrow has ended.
     */
    pub(crate) rendered_lines: Vec<Line<'static>>,
    /*
     * Staging buffer for the suffix selected during sync. Tests inspect the field directly, but
     * production clears it after terminal mutation so stale rows cannot be replayed on the next
     * draw.
     */
    pub(crate) pending_history_lines: Vec<Line<'static>>,
    /*
     * Rendered row count currently visible above the ratatui viewport. The value is measured in
     * terminal rows, not transcript Lines, because wrapping long conversation rows changes how far
     * the host scrollback pushes the frame.
     */
    pub(crate) visible_history_rows: u16,
    /*
     * A resize that lands after a completed insertion makes row placement uncertain even though
     * the transcript bytes must remain at-most-once. The next physical reconciliation clears this
     * flag after clamping the cached row count to the observed viewport.
     */
    pub(crate) visible_history_rows_dirty: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HistoryFlushResult {
    inserted_rows: u16,
    stable_geometry: bool,
    history_committed: bool,
}

impl HistoryFlushResult {
    // Callers only need to know whether the host scrollback moved so they can invalidate buffers.
    pub(crate) fn inserted(self) -> bool {
        self.inserted_rows > 0
    }

    pub(crate) fn stable_geometry(self) -> bool {
        self.stable_geometry
    }

    pub(crate) fn history_committed(self) -> bool {
        self.history_committed
    }
}

/*
 * Capped transcript windows can shift by a few lines when MAX_CONVERSATION_HISTORY_LINES is hit.
 * A minimum overlap avoids treating repeated prompt/status fragments as proof that two different
 * threads are the same rolling window.
 */
const MIN_SHIFTED_HISTORY_OVERLAP: usize = 8;

impl HistoryFlushState {
    /*
     * A physical terminal resize already moves rows between the visible screen and host
     * scrollback. Reconcile only the accounting cache in that case; appending more blank lines
     * would apply the same shrink a second time and leave old live-tail rows on screen.
     */
    pub(crate) fn reconcile_physical_resize(&mut self, viewport_area: Rect) -> bool {
        let previous_visible_rows = self.visible_history_rows;
        let was_dirty = self.visible_history_rows_dirty;
        self.visible_history_rows = self.visible_history_rows.min(viewport_area.top());
        self.visible_history_rows_dirty = false;
        was_dirty || self.visible_history_rows != previous_visible_rows
    }

    /*
     * Terminal resize and newline-fallback insertion can leave more history rows visible above the
     * frame than the new viewport can contain. Appending blank lines at the bottom advances the host
     * scrollback until the inline frame has clear space again, then clamps the cache to the new top.
     */
    pub(crate) fn fit_visible_rows_to_viewport<B: InlineResizeBackend>(
        &mut self,
        terminal: &mut Terminal<B>,
        expected: InlineResizeSnapshot,
        viewport_area: Rect,
    ) -> Result<Option<bool>, B::Error> {
        if !terminal.backend().matches_resize_snapshot(expected)? {
            return Ok(None);
        }
        let viewport_top = viewport_area.top();
        if self.visible_history_rows <= viewport_top {
            return Ok(Some(false));
        }
        let overflow_rows = self.visible_history_rows - viewport_top;
        terminal.backend_mut().set_cursor_position(Position {
            x: 0,
            y: expected.size.height.saturating_sub(1),
        })?;
        if !terminal.backend().matches_resize_snapshot(expected)? {
            return Ok(None);
        }
        terminal.backend_mut().append_lines(overflow_rows)?;
        if !terminal.backend().matches_resize_snapshot(expected)? {
            return Ok(None);
        }
        self.visible_history_rows = viewport_top;
        Ok(Some(true))
    }

    /*
     * sync is the draw-cycle write barrier for host scrollback. It chooses the pending transcript
     * suffix, counts how many terminal rows that suffix will render at the current width, delegates
     * the escape-sequence strategy to HistoryInsertionAdapter, and then refreshes the baseline
     * snapshot. Keeping all four steps together makes viewport invalidation depend on the same row
     * count that actually moved the terminal.
     */
    pub(crate) fn sync<B: InlineResizeBackend>(
        &mut self,
        terminal: &mut Terminal<B>,
        current_lines: &[Line<'static>],
        expected: InlineResizeSnapshot,
        insert_mode: HistoryInsertionMode,
    ) -> Result<HistoryFlushResult, B::Error> {
        let pending_history_lines = self.pending_lines(current_lines);
        if !terminal.backend().matches_resize_snapshot(expected)? {
            return Ok(HistoryFlushResult::default());
        }
        let width = expected.size.width;
        let inserted_rows = if pending_history_lines.is_empty() {
            0
        } else {
            count_rendered_history_rows(&pending_history_lines, width).min(u16::MAX as usize) as u16
        };
        /*
         * No pending rows means the app transcript and host scrollback are already aligned. Avoid
         * touching the terminal in that case so cursor position and scroll region state remain
         * stable for the ordinary ratatui frame render.
         */
        if inserted_rows > 0 {
            let insertion = HistoryInsertionAdapter::new(insert_mode)
                .insert_with_rendered_rows_at_snapshot(
                    terminal,
                    &pending_history_lines,
                    inserted_rows,
                    expected,
                )?;
            if !insertion.completed() {
                return Ok(HistoryFlushResult::default());
            }
            if !insertion.stable_geometry() {
                let viewport_top_after_insert = terminal.get_frame().area().top();
                self.visible_history_rows = self.visible_rows_after_insert(
                    pending_history_lines.len(),
                    current_lines.len(),
                    inserted_rows,
                    viewport_top_after_insert,
                );
                self.pending_history_lines = pending_history_lines;
                self.remember(current_lines);
                self.pending_history_lines.clear();
                self.visible_history_rows_dirty = true;
                return Ok(HistoryFlushResult {
                    inserted_rows,
                    stable_geometry: false,
                    history_committed: true,
                });
            }
        }
        self.pending_history_lines = pending_history_lines;
        let viewport_top_after_insert = terminal.get_frame().area().top();
        if current_lines.is_empty() {
            self.visible_history_rows = 0;
        } else if inserted_rows > 0 {
            self.visible_history_rows = self.visible_rows_after_insert(
                self.pending_history_lines.len(),
                current_lines.len(),
                inserted_rows,
                viewport_top_after_insert,
            );
        }
        /*
         * The baseline is updated even when no rows were inserted. That covers render modes that
         * skip host writes for a tick, and it prevents a later mode switch from replaying already
         * observed transcript rows.
         */
        self.remember(current_lines);
        self.pending_history_lines.clear();
        self.visible_history_rows_dirty = false;
        Ok(HistoryFlushResult {
            inserted_rows,
            stable_geometry: true,
            history_committed: true,
        })
    }

    /*
     * Some inline render modes keep transcript rows inside the ratatui frame instead of writing
     * host scrollback. They still need to advance the diff baseline; otherwise switching back to a
     * scrollback-writing mode would dump the full already-rendered transcript as new history.
     */
    pub(crate) fn remember_without_flush(&mut self, current_lines: &[Line<'static>]) {
        if current_lines.is_empty() {
            self.visible_history_rows = 0;
        }
        self.visible_history_rows_dirty = false;
        self.pending_history_lines.clear();
        self.remember(current_lines);
    }

    pub(crate) fn has_pending_lines(&self, current_lines: &[Line<'static>]) -> bool {
        !self.pending_lines(current_lines).is_empty()
    }

    pub(crate) fn mark_visible_history_rows_dirty(&mut self) {
        self.visible_history_rows_dirty = true;
    }

    fn remember(&mut self, current_lines: &[Line<'static>]) {
        self.rendered_lines = current_lines.to_vec();
    }

    fn visible_rows_after_insert(
        &self,
        pending_line_count: usize,
        current_line_count: usize,
        inserted_rows: u16,
        viewport_top: u16,
    ) -> u16 {
        if pending_line_count == current_line_count {
            inserted_rows.min(viewport_top)
        } else {
            self.visible_history_rows
                .saturating_add(inserted_rows)
                .min(viewport_top)
        }
    }

    /*
     * pending_lines separates three transcript shapes. Normal append-only turns flush only the new
     * suffix, capped history windows flush only the rows beyond the detected overlap, and session
     * resets replay the full current transcript because the old scrollback baseline no longer
     * describes the active conversation.
     */
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

    /*
     * The shifted-window detector only runs when the current transcript has reached the shared
     * conversation history cap. It searches from the longest possible overlap downward, so the first
     * match treats the maximum safe prefix as already written and minimizes duplicate scrollback
     * when the oldest capped lines fall away.
     */
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
