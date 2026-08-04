use std::collections::VecDeque;
use std::mem;
use std::rc::Rc;

use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::super::SharedTranscriptCardDigests;
use super::super::shell_presentation::{
    ConversationTranscriptLineInteraction, ConversationTranscriptLineSurface,
    ConversationTranscriptView,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TranscriptProjectedCardRow {
    pub(super) digest: [u8; 32],
    pub(super) start: usize,
    pub(super) end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TranscriptWrappedRowLayout {
    pub(super) logical_line_index: usize,
    pub(super) soft_wrap_separator: String,
    pub(super) selection_range_id: Option<u64>,
    pub(super) selectable_from_column: u16,
    pub(super) surface: ConversationTranscriptLineSurface,
}

/// Width-bound, immutable rendering data for one canonical transcript revision.
///
/// Formatting and Unicode wrapping are intentionally materialized before the
/// terminal draw. A scroll-only frame can then borrow the few logical lines in
/// the viewport instead of rebuilding and cloning the entire conversation.
pub(in crate::adapter::inbound::tui::app) struct FullscreenTranscriptDocument {
    lines: Rc<[Line<'static>]>,
    wrapped_rows: Rc<[TranscriptWrappedRowLayout]>,
    line_row_starts: Rc<[usize]>,
    card_rows: Rc<[TranscriptProjectedCardRow]>,
    card_digests: SharedTranscriptCardDigests,
}

impl FullscreenTranscriptDocument {
    pub(in crate::adapter::inbound::tui::app) fn from_view(
        view: ConversationTranscriptView,
        width: u16,
    ) -> Self {
        let ConversationTranscriptView {
            lines,
            line_interactions,
            card_rows,
        } = view;
        let wrapped_rows = transcript_wrapped_row_layout(&lines, &line_interactions, width);
        let mut line_row_starts = Vec::with_capacity(lines.len().saturating_add(1));
        let mut row_cursor = 0;
        for line_index in 0..lines.len() {
            line_row_starts.push(row_cursor);
            while wrapped_rows
                .get(row_cursor)
                .is_some_and(|row| row.logical_line_index == line_index)
            {
                row_cursor = row_cursor.saturating_add(1);
            }
        }
        line_row_starts.push(row_cursor);
        let card_rows = card_rows
            .into_iter()
            .filter_map(|row| {
                let start = line_row_starts.get(row.line_index).copied()?;
                let end = line_row_starts
                    .get(row.line_index.saturating_add(1))
                    .copied()?;
                (end > start).then_some(TranscriptProjectedCardRow {
                    digest: row.digest,
                    start,
                    end,
                })
            })
            .collect::<Vec<_>>();
        let card_digests = card_rows.iter().map(|row| row.digest).collect::<Vec<_>>();
        Self {
            lines: Rc::from(lines),
            wrapped_rows: Rc::from(wrapped_rows),
            line_row_starts: Rc::from(line_row_starts),
            card_rows: Rc::from(card_rows),
            card_digests: SharedTranscriptCardDigests::from(card_digests),
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn content_rows(&self) -> usize {
        self.wrapped_rows.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub(super) fn card_digests(&self) -> SharedTranscriptCardDigests {
        SharedTranscriptCardDigests::clone(&self.card_digests)
    }

    pub(super) fn card_rows(&self) -> &[TranscriptProjectedCardRow] {
        &self.card_rows
    }

    pub(super) fn wrapped_rows(&self) -> &[TranscriptWrappedRowLayout] {
        &self.wrapped_rows
    }

    pub(super) fn paragraph_window(
        &self,
        top_row: usize,
        viewport_height: u16,
    ) -> (Vec<Line<'_>>, u16) {
        if viewport_height == 0 || top_row >= self.wrapped_rows.len() {
            return (Vec::new(), 0);
        }
        let visible_end = top_row
            .saturating_add(usize::from(viewport_height))
            .min(self.wrapped_rows.len());
        let start_line = self.wrapped_rows[top_row].logical_line_index;
        let end_line = self.wrapped_rows[visible_end.saturating_sub(1)]
            .logical_line_index
            .saturating_add(1)
            .min(self.lines.len());
        let line_start_row = self
            .line_row_starts
            .get(start_line)
            .copied()
            .unwrap_or(top_row);
        let paragraph_scroll =
            u16::try_from(top_row.saturating_sub(line_start_row)).unwrap_or(u16::MAX);
        let lines = self.lines[start_line..end_line]
            .iter()
            .map(|line| Line {
                style: line.style,
                alignment: line.alignment,
                spans: line
                    .spans
                    .iter()
                    .map(|span| Span::styled(span.content.as_ref(), span.style))
                    .collect(),
            })
            .collect();
        (lines, paragraph_scroll)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TranscriptWrapGrapheme<'a> {
    symbol: &'a str,
    source_index: usize,
}

impl TranscriptWrapGrapheme<'_> {
    fn width(&self) -> u16 {
        u16::try_from(self.symbol.width()).unwrap_or(u16::MAX)
    }

    fn is_whitespace(&self) -> bool {
        self.symbol == "\u{200b}"
            || (self.symbol.chars().all(char::is_whitespace) && self.symbol != "\u{00a0}")
    }
}

pub(super) fn transcript_wrapped_row_layout(
    lines: &[Line<'_>],
    line_interactions: &[ConversationTranscriptLineInteraction],
    width: u16,
) -> Vec<TranscriptWrappedRowLayout> {
    let mut rows = Vec::new();
    for (logical_line_index, line) in lines.iter().enumerate() {
        let source = transcript_line_graphemes(line);
        let wrapped = wrap_transcript_graphemes(&source, width);
        let interaction = line_interactions
            .get(logical_line_index)
            .copied()
            .unwrap_or(ConversationTranscriptLineInteraction {
                selection_range_id: None,
                selectable_from_column: 0,
                surface: ConversationTranscriptLineSurface::Plain,
            });
        rows.extend(wrapped.iter().enumerate().map(|(row_index, row)| {
            let soft_wrap_separator = wrapped
                .get(row_index + 1)
                .map_or_else(String::new, |next_row| {
                    wrap_boundary_separator(&source, row, next_row)
                });
            TranscriptWrappedRowLayout {
                logical_line_index,
                soft_wrap_separator,
                selection_range_id: interaction.selection_range_id,
                selectable_from_column: if row_index == 0 {
                    interaction
                        .selectable_from_column
                        .min(width.saturating_sub(1))
                } else {
                    0
                },
                surface: interaction.surface,
            }
        }));
    }
    rows
}

fn transcript_line_graphemes<'a>(line: &'a Line<'_>) -> Vec<TranscriptWrapGrapheme<'a>> {
    line.spans
        .iter()
        .flat_map(|span| UnicodeSegmentation::graphemes(span.content.as_ref(), true))
        .enumerate()
        .map(|(source_index, symbol)| TranscriptWrapGrapheme {
            symbol,
            source_index,
        })
        .collect()
}

/// Mirrors Ratatui's `WordWrapper` with `trim: false`, while retaining source
/// indices for whitespace that transcript selection consumes at a wrap boundary.
fn wrap_transcript_graphemes<'a>(
    source: &[TranscriptWrapGrapheme<'a>],
    max_line_width: u16,
) -> Vec<Vec<TranscriptWrapGrapheme<'a>>> {
    if max_line_width == 0 {
        return Vec::new();
    }

    let mut wrapped_lines = Vec::new();
    let mut pending_line = Vec::new();
    let mut pending_word = Vec::new();
    let mut pending_whitespace: VecDeque<TranscriptWrapGrapheme<'a>> = VecDeque::new();
    let mut line_width = 0_u16;
    let mut word_width = 0_u16;
    let mut whitespace_width = 0_u16;
    let mut non_whitespace_previous = false;

    for grapheme in source.iter().copied() {
        let is_whitespace = grapheme.is_whitespace();
        let symbol_width = grapheme.width();
        if symbol_width > max_line_width {
            continue;
        }

        let word_found = non_whitespace_previous && is_whitespace;
        let untrimmed_overflow = pending_line.is_empty()
            && word_width
                .saturating_add(whitespace_width)
                .saturating_add(symbol_width)
                > max_line_width;
        if word_found || untrimmed_overflow {
            pending_line.extend(pending_whitespace.drain(..));
            line_width = line_width.saturating_add(whitespace_width);
            pending_line.append(&mut pending_word);
            line_width = line_width.saturating_add(word_width);
            whitespace_width = 0;
            word_width = 0;
        }

        let line_full = line_width >= max_line_width;
        let pending_word_overflow = symbol_width > 0
            && line_width
                .saturating_add(whitespace_width)
                .saturating_add(word_width)
                >= max_line_width;
        if line_full || pending_word_overflow {
            let mut remaining_width = max_line_width.saturating_sub(line_width);
            wrapped_lines.push(mem::take(&mut pending_line));
            line_width = 0;

            while let Some(pending) = pending_whitespace.front() {
                let width = pending.width();
                if width > remaining_width {
                    break;
                }
                whitespace_width = whitespace_width.saturating_sub(width);
                remaining_width = remaining_width.saturating_sub(width);
                pending_whitespace.pop_front();
            }

            if is_whitespace && pending_whitespace.is_empty() {
                continue;
            }
        }

        if is_whitespace {
            whitespace_width = whitespace_width.saturating_add(symbol_width);
            pending_whitespace.push_back(grapheme);
        } else {
            word_width = word_width.saturating_add(symbol_width);
            pending_word.push(grapheme);
        }
        non_whitespace_previous = !is_whitespace;
    }

    pending_line.extend(pending_whitespace);
    pending_line.append(&mut pending_word);
    if !pending_line.is_empty() {
        wrapped_lines.push(pending_line);
    }
    if wrapped_lines.is_empty() {
        wrapped_lines.push(Vec::new());
    }
    wrapped_lines
}

fn wrap_boundary_separator(
    source: &[TranscriptWrapGrapheme<'_>],
    row: &[TranscriptWrapGrapheme<'_>],
    next_row: &[TranscriptWrapGrapheme<'_>],
) -> String {
    let Some(start) = row
        .last()
        .map(|grapheme| grapheme.source_index.saturating_add(1))
    else {
        return String::new();
    };
    let Some(end) = next_row.first().map(|grapheme| grapheme.source_index) else {
        return String::new();
    };
    source
        .get(start..end)
        .unwrap_or_default()
        .iter()
        .filter(|grapheme| grapheme.is_whitespace())
        .map(|grapheme| grapheme.symbol)
        .collect()
}
