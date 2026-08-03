use ratatui::layout::{Position, Rect};
use unicode_width::UnicodeWidthStr;

use super::terminal_interaction_ui::TerminalInteractionUiState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TranscriptCardHitArea {
    pub(super) digest: [u8; 32],
    pub(super) area: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TranscriptSelectionPoint {
    absolute_row: usize,
    column: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TranscriptSelection {
    anchor: TranscriptSelectionPoint,
    focus: TranscriptSelectionPoint,
    dragging: bool,
    resume_follow_tail_on_click: bool,
    source_frame: TranscriptViewportFrame,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TranscriptRenderedRow {
    pub(super) absolute_row: usize,
    pub(super) logical_line_index: usize,
    pub(super) soft_wrap_separator: String,
    pub(super) cells: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TranscriptViewportFrame {
    pub(super) area: Rect,
    pub(super) rows: Vec<TranscriptRenderedRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TranscriptSelectionFinish {
    Ignored,
    Click,
    Copy(String),
}

/// App-owned viewport state for the fullscreen conversation transcript.
///
/// `top_row` is an absolute wrapped-row anchor. That is intentionally different
/// from a distance-from-tail counter: when new stream rows arrive while the
/// operator is reading older output, the visible rows stay put. Only explicit
/// follow-tail mode tracks the growing end of the document.
///
/// Mouse selection uses the exact cells committed by the last stable Ratatui
/// frame. Anchors are absolute wrapped rows rather than screen rows, so a stream
/// append cannot move an in-progress selection. A resize invalidates the cell
/// geometry before another pointer event can use it.
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
    frame_snapshot: Option<TranscriptViewportFrame>,
    selection: Option<TranscriptSelection>,
    last_copied_selection: Option<String>,
    terminal_interaction: TerminalInteractionUiState,
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
            frame_snapshot: None,
            selection: None,
            last_copied_selection: None,
            terminal_interaction: TerminalInteractionUiState::from_environment(),
        }
    }
}

impl TranscriptViewportUiState {
    pub(super) fn bind_document(&mut self, identity: Option<String>) {
        if self.document_identity == identity {
            return;
        }
        let terminal_interaction = self.terminal_interaction.clone();
        *self = Self {
            document_identity: identity,
            terminal_interaction,
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

    pub(super) fn bind_frame(
        &mut self,
        card_digests: Vec<[u8; 32]>,
        card_hit_areas: Vec<TranscriptCardHitArea>,
        mut frame_snapshot: Option<TranscriptViewportFrame>,
    ) {
        if let Some(frame_snapshot) = frame_snapshot.as_mut() {
            for row in &mut frame_snapshot.rows {
                normalize_rendered_row_cells(&mut row.cells);
            }
        }
        self.card_digests = card_digests;
        self.card_hit_areas = card_hit_areas;
        self.frame_snapshot = frame_snapshot;
    }

    pub(super) fn clear_frame_geometry(&mut self) {
        self.card_hit_areas.clear();
        self.frame_snapshot = None;
        self.selection = None;
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

    pub(super) fn begin_selection(&mut self, column: u16, row: u16) -> bool {
        let Some(point) = self.selection_point_at(column, row) else {
            return false;
        };
        let Some(source_frame) = self.frame_snapshot.clone() else {
            return false;
        };
        let resume_follow_tail_on_click = self.follow_tail;
        self.follow_tail = false;
        self.selection = Some(TranscriptSelection {
            anchor: point,
            focus: point,
            dragging: true,
            resume_follow_tail_on_click,
            source_frame,
        });
        true
    }

    pub(super) fn update_selection(&mut self, column: u16, row: u16) -> bool {
        let Some(point) = self
            .selection
            .as_ref()
            .filter(|selection| selection.dragging)
            .and_then(|selection| selection_point_in_frame(&selection.source_frame, column, row))
        else {
            return false;
        };
        let Some(selection) = self
            .selection
            .as_mut()
            .filter(|selection| selection.dragging)
        else {
            return false;
        };
        let changed = selection.focus != point;
        selection.focus = point;
        changed
    }

    pub(super) fn finish_selection(&mut self, column: u16, row: u16) -> TranscriptSelectionFinish {
        if self
            .selection
            .as_ref()
            .is_none_or(|selection| !selection.dragging)
        {
            return TranscriptSelectionFinish::Ignored;
        }
        if let Some(point) = self
            .selection
            .as_ref()
            .and_then(|selection| selection_point_in_frame(&selection.source_frame, column, row))
            && let Some(selection) = self.selection.as_mut()
        {
            selection.focus = point;
        }
        let Some(selection) = self.selection.as_ref() else {
            return TranscriptSelectionFinish::Ignored;
        };
        if selection.anchor == selection.focus {
            let resume_follow_tail = selection.resume_follow_tail_on_click;
            self.selection = None;
            if resume_follow_tail {
                self.follow_latest();
            }
            return TranscriptSelectionFinish::Click;
        }

        let Some(text) = self.selected_text_from_snapshot() else {
            self.selection = None;
            return TranscriptSelectionFinish::Ignored;
        };
        let Some(selection) = self.selection.as_mut() else {
            return TranscriptSelectionFinish::Ignored;
        };
        selection.dragging = false;
        self.last_copied_selection = Some(text.clone());
        TranscriptSelectionFinish::Copy(text)
    }

    pub(super) fn copied_selection_text(&self) -> Option<&str> {
        self.last_copied_selection.as_deref()
    }

    pub(super) fn active_selection_text(&self) -> Option<String> {
        let selection = self.selection.as_ref()?;
        if selection.anchor == selection.focus {
            return None;
        }
        self.selected_text_from_snapshot()
    }

    pub(super) fn selection_columns_for_row(
        &self,
        absolute_row: usize,
        width: u16,
    ) -> Option<(u16, u16)> {
        let selection = self.selection.as_ref()?;
        let (start, end) = ordered_points(selection.anchor, selection.focus);
        if absolute_row < start.absolute_row || absolute_row > end.absolute_row || width == 0 {
            return None;
        }
        let last_column = width.saturating_sub(1);
        let start_column = if absolute_row == start.absolute_row {
            start.column.min(last_column)
        } else {
            0
        };
        let end_column = if absolute_row == end.absolute_row {
            end.column.min(last_column)
        } else {
            last_column
        };
        Some((start_column, end_column))
    }

    pub(super) fn has_unseen_output(&self) -> bool {
        !self.follow_tail && self.latest_revision > self.seen_revision
    }

    fn selection_point_at(&self, column: u16, row: u16) -> Option<TranscriptSelectionPoint> {
        let snapshot = self.frame_snapshot.as_ref()?;
        selection_point_in_frame(snapshot, column, row)
    }

    fn selected_text_from_snapshot(&self) -> Option<String> {
        let selection = self.selection.as_ref()?;
        let snapshot = &selection.source_frame;
        let (start, end) = ordered_points(selection.anchor, selection.focus);
        let selected_rows = snapshot
            .rows
            .iter()
            .filter(|row| {
                row.absolute_row >= start.absolute_row && row.absolute_row <= end.absolute_row
            })
            .collect::<Vec<_>>();
        if selected_rows.first()?.absolute_row != start.absolute_row
            || selected_rows.last()?.absolute_row != end.absolute_row
        {
            return None;
        }

        let mut text = String::new();
        for (index, rendered_row) in selected_rows.iter().enumerate() {
            let start_column = if rendered_row.absolute_row == start.absolute_row {
                start.column
            } else {
                0
            };
            let end_column = if rendered_row.absolute_row == end.absolute_row {
                end.column
            } else {
                snapshot.area.width.saturating_sub(1)
            };
            let segment = rendered_row
                .cells
                .iter()
                .skip(usize::from(start_column))
                .take(usize::from(
                    end_column.saturating_sub(start_column).saturating_add(1),
                ))
                .map(String::as_str)
                .collect::<String>();
            let segment = segment.trim_end_matches(' ');
            text.push_str(segment);

            let Some(next_row) = selected_rows.get(index + 1) else {
                continue;
            };
            if next_row.logical_line_index != rendered_row.logical_line_index {
                text.push('\n');
            } else {
                text.push_str(&rendered_row.soft_wrap_separator);
            }
        }
        (!text.is_empty()).then_some(text)
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

    #[cfg(test)]
    pub(super) fn frame_snapshot(&self) -> Option<&TranscriptViewportFrame> {
        self.frame_snapshot.as_ref()
    }

    pub(super) fn terminal_interaction(&self) -> &TerminalInteractionUiState {
        &self.terminal_interaction
    }

    pub(super) fn terminal_interaction_mut(&mut self) -> &mut TerminalInteractionUiState {
        &mut self.terminal_interaction
    }
}

fn ordered_points(
    first: TranscriptSelectionPoint,
    second: TranscriptSelectionPoint,
) -> (TranscriptSelectionPoint, TranscriptSelectionPoint) {
    if first <= second {
        (first, second)
    } else {
        (second, first)
    }
}

fn selection_point_in_frame(
    frame: &TranscriptViewportFrame,
    column: u16,
    row: u16,
) -> Option<TranscriptSelectionPoint> {
    if !frame.area.contains(Position::new(column, row)) {
        return None;
    }
    let visible_row = usize::from(row.saturating_sub(frame.area.y));
    let rendered_row = frame.rows.get(visible_row)?;
    let mut relative_column = column.saturating_sub(frame.area.x);
    relative_column = relative_column.min(frame.area.width.saturating_sub(1));
    let current_index = usize::from(relative_column);
    if rendered_row
        .cells
        .get(current_index)
        .is_some_and(|cell| cell.is_empty() || cell == " ")
    {
        for candidate in (0..current_index).rev() {
            let Some(symbol) = rendered_row.cells.get(candidate) else {
                continue;
            };
            if symbol.is_empty() {
                continue;
            }
            let symbol_width = symbol.as_str().width().max(1);
            if candidate.saturating_add(symbol_width) > current_index {
                relative_column = u16::try_from(candidate).unwrap_or(relative_column);
            }
            break;
        }
    }
    Some(TranscriptSelectionPoint {
        absolute_row: rendered_row.absolute_row,
        column: relative_column,
    })
}

fn normalize_rendered_row_cells(cells: &mut [String]) {
    let mut column = 0;
    while column < cells.len() {
        let width = cells[column].as_str().width().max(1);
        for continuation in 1..width {
            let Some(cell) = cells.get_mut(column.saturating_add(continuation)) else {
                break;
            };
            cell.clear();
        }
        column = column.saturating_add(width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_snapshot(lines: &[(&str, usize, &str)]) -> TranscriptViewportFrame {
        frame_snapshot_with_width(8, lines)
    }

    fn frame_snapshot_with_width(
        width: u16,
        lines: &[(&str, usize, &str)],
    ) -> TranscriptViewportFrame {
        TranscriptViewportFrame {
            area: Rect::new(2, 3, width, lines.len() as u16),
            rows: lines
                .iter()
                .enumerate()
                .map(|(index, (text, logical_line_index, soft_wrap_separator))| {
                    let mut cells = text
                        .chars()
                        .map(|value| value.to_string())
                        .collect::<Vec<_>>();
                    cells.resize(usize::from(width), " ".to_string());
                    TranscriptRenderedRow {
                        absolute_row: 10 + index,
                        logical_line_index: *logical_line_index,
                        soft_wrap_separator: (*soft_wrap_separator).to_string(),
                        cells,
                    }
                })
                .collect(),
        }
    }

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
        state.bind_frame(
            vec![[1; 32]],
            vec![TranscriptCardHitArea {
                digest: [1; 32],
                area: Rect::new(0, 0, 10, 1),
            }],
            Some(frame_snapshot(&[("alpha", 0, "")])),
        );
        state.page_up();

        state.bind_document(Some("thread-b".to_string()));

        assert_eq!(state.top_row(), 0);
        assert!(state.follow_tail());
        assert!(state.card_hit_areas().is_empty());
        assert!(!state.begin_selection(2, 3));
    }

    #[test]
    fn drag_selection_freezes_tail_and_reconstructs_soft_wrapped_text() {
        let mut state = TranscriptViewportUiState::default();
        state.resolve_frame(12, 2, 1);
        state.bind_frame(
            Vec::new(),
            Vec::new(),
            Some(frame_snapshot(&[("hello", 0, " "), ("world", 0, "")])),
        );

        assert!(state.begin_selection(2, 3));
        assert!(!state.follow_tail());
        assert!(state.update_selection(6, 4));
        assert_eq!(
            state.active_selection_text(),
            Some("hello world".to_string())
        );
        assert_eq!(
            state.finish_selection(6, 4),
            TranscriptSelectionFinish::Copy("hello world".to_string())
        );
        assert_eq!(state.copied_selection_text(), Some("hello world"));
        state.clear_frame_geometry();
        assert_eq!(
            state.copied_selection_text(),
            Some("hello world"),
            "resize invalidates highlight geometry but not the explicit copy source"
        );
    }

    #[test]
    fn exact_width_word_wrap_preserves_the_consumed_separator() {
        let mut state = TranscriptViewportUiState::default();
        state.resolve_frame(2, 2, 1);
        state.bind_frame(
            Vec::new(),
            Vec::new(),
            Some(frame_snapshot_with_width(
                5,
                &[("hello", 0, " "), ("world", 0, "")],
            )),
        );

        assert!(state.begin_selection(2, 3));
        assert!(state.update_selection(6, 4));
        assert_eq!(
            state.finish_selection(6, 4),
            TranscriptSelectionFinish::Copy("hello world".to_string())
        );
    }

    #[test]
    fn wide_glyph_continuation_cells_snap_to_the_visible_glyph() {
        let mut state = TranscriptViewportUiState::default();
        state.resolve_frame(1, 1, 1);
        state.bind_frame(
            Vec::new(),
            Vec::new(),
            Some(TranscriptViewportFrame {
                area: Rect::new(2, 3, 4, 1),
                rows: vec![TranscriptRenderedRow {
                    absolute_row: 0,
                    logical_line_index: 0,
                    soft_wrap_separator: String::new(),
                    cells: vec![
                        "한".to_string(),
                        " ".to_string(),
                        "글".to_string(),
                        " ".to_string(),
                    ],
                }],
            }),
        );

        assert!(state.begin_selection(3, 3));
        assert!(state.update_selection(5, 3));
        assert_eq!(
            state.finish_selection(5, 3),
            TranscriptSelectionFinish::Copy("한글".to_string())
        );
    }

    #[test]
    fn click_without_drag_restores_follow_tail() {
        let mut state = TranscriptViewportUiState::default();
        state.resolve_frame(12, 2, 1);
        state.bind_frame(
            Vec::new(),
            Vec::new(),
            Some(frame_snapshot(&[("click", 0, "")])),
        );

        assert!(state.begin_selection(3, 3));
        assert_eq!(
            state.finish_selection(3, 3),
            TranscriptSelectionFinish::Click
        );
        assert!(state.follow_tail());
    }

    #[test]
    fn reverse_drag_normalizes_copy_order() {
        let mut state = TranscriptViewportUiState::default();
        state.resolve_frame(12, 2, 1);
        state.bind_frame(
            Vec::new(),
            Vec::new(),
            Some(frame_snapshot(&[("alpha", 0, ""), ("beta", 1, "")])),
        );

        assert!(state.begin_selection(5, 4));
        assert!(state.update_selection(3, 3));
        assert_eq!(
            state.finish_selection(3, 3),
            TranscriptSelectionFinish::Copy("lpha\nbeta".to_string())
        );
    }

    #[test]
    fn streaming_redraw_cannot_replace_the_selection_source_snapshot() {
        let mut state = TranscriptViewportUiState::default();
        state.resolve_frame(12, 2, 1);
        state.bind_frame(
            Vec::new(),
            Vec::new(),
            Some(frame_snapshot(&[("stable", 0, "")])),
        );
        assert!(state.begin_selection(2, 3));
        assert!(state.update_selection(7, 3));

        let mut replacement_frame = frame_snapshot(&[("mutate", 0, "")]);
        replacement_frame.rows[0].absolute_row = 40;
        state.bind_frame(Vec::new(), Vec::new(), Some(replacement_frame));

        assert_eq!(
            state.finish_selection(7, 3),
            TranscriptSelectionFinish::Copy("stable".to_string())
        );
    }
}
