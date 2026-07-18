use ratatui::text::{Line, Span};

use super::super::super::{
    AkraTheme, ProgressiveActivityCard, ProgressiveActivityCardKind, ProgressiveActivityDetailKind,
    ProgressiveActivityPageCursor,
};
use super::activity_diff::build_bounded_diff_page;

const PAGE_SCAN_BYTES_PER_CELL: usize = 8;
const PAGE_OUTPUT_BYTES_PER_CELL: usize = 8;
const MAX_LIST_ROWS: usize = 8;

#[derive(Clone, Copy)]
pub(crate) struct ActivityOverlayDocument<'a> {
    pub(crate) text: &'a str,
    pub(crate) source_bytes: u64,
    pub(crate) retained_bytes: u64,
    pub(crate) truncated_bytes: u64,
    pub(crate) history_incomplete: bool,
}

pub(crate) struct ActivityOverlayView {
    pub(crate) header_lines: Vec<Line<'static>>,
    pub(crate) detail_title: Line<'static>,
    pub(crate) detail_lines: Vec<Line<'static>>,
    pub(crate) key_lines: Vec<Line<'static>>,
    pub(crate) current_page_cursor: ProgressiveActivityPageCursor,
    pub(crate) next_page_cursor: Option<ProgressiveActivityPageCursor>,
}

#[cfg(test)]
pub(crate) fn build_activity_overlay_view(
    selected_kind: ProgressiveActivityDetailKind,
    diff_available: bool,
    output_available: bool,
    document: Option<ActivityOverlayDocument<'_>>,
    requested_page_cursor: ProgressiveActivityPageCursor,
    viewport_width: u16,
    viewport_height: u16,
) -> ActivityOverlayView {
    build_activity_overlay_list_view(
        None,
        &[],
        0,
        true,
        selected_kind,
        diff_available,
        output_available,
        document,
        requested_page_cursor,
        viewport_width,
        viewport_height,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_activity_overlay_list_view(
    card_filter: Option<ProgressiveActivityCardKind>,
    cards: &[ProgressiveActivityCard],
    selected_card_index: usize,
    list_focus: bool,
    selected_kind: ProgressiveActivityDetailKind,
    diff_available: bool,
    output_available: bool,
    document: Option<ActivityOverlayDocument<'_>>,
    requested_page_cursor: ProgressiveActivityPageCursor,
    viewport_width: u16,
    viewport_height: u16,
) -> ActivityOverlayView {
    let mut header_lines = vec![build_filter_line(
        card_filter,
        selected_kind,
        diff_available,
        output_available,
    )];
    header_lines.extend(build_card_list_lines(
        cards,
        selected_card_index,
        list_focus,
    ));

    let Some(document) = document else {
        if cards.is_empty() {
            header_lines.push(Line::styled(
                "no retained progressive activity cards",
                AkraTheme::muted(),
            ));
        }
        return ActivityOverlayView {
            header_lines,
            detail_title: Line::from(selected_card_detail_title(cards, selected_card_index)),
            detail_lines: vec![Line::from(
                "No retained detail is available for the selected activity card.".to_string(),
            )],
            key_lines: build_activity_overlay_key_lines(viewport_width),
            current_page_cursor: ProgressiveActivityPageCursor::at(0),
            next_page_cursor: None,
        };
    };

    header_lines.extend(build_document_status_lines(&document));
    let detail_kind = activity_document_detail_kind(
        cards.get(selected_card_index).map(|card| card.key.kind),
        selected_kind,
    );
    let page = match detail_kind {
        ProgressiveActivityDetailKind::Diff => {
            let page = build_bounded_diff_page(
                document.text,
                requested_page_cursor,
                viewport_width,
                viewport_height,
            );
            ActivityDocumentPage {
                lines: page.lines,
                start_byte: page.current_cursor.byte_offset,
                end_byte: page.end_byte,
                current_cursor: page.current_cursor,
                next_cursor: page.next_cursor,
            }
        }
        ProgressiveActivityDetailKind::Output => {
            let page = build_bounded_document_page(
                document.text,
                requested_page_cursor.byte_offset,
                viewport_width,
                viewport_height,
            );
            ActivityDocumentPage {
                lines: page.lines,
                start_byte: page.start_byte,
                end_byte: page.end_byte,
                current_cursor: ProgressiveActivityPageCursor::at(page.start_byte),
                next_cursor: page.next_page_start.map(ProgressiveActivityPageCursor::at),
            }
        }
    };
    let detail_title = Line::from(format!(
        "{} | {}-{} / {} B",
        selected_card_detail_title(cards, selected_card_index),
        page.start_byte,
        page.end_byte,
        document.retained_bytes
    ));

    ActivityOverlayView {
        header_lines,
        detail_title,
        detail_lines: page.lines,
        key_lines: build_activity_overlay_key_lines(viewport_width),
        current_page_cursor: page.current_cursor,
        next_page_cursor: page.next_cursor,
    }
}

fn activity_document_detail_kind(
    card_kind: Option<ProgressiveActivityCardKind>,
    fallback: ProgressiveActivityDetailKind,
) -> ProgressiveActivityDetailKind {
    match card_kind {
        Some(ProgressiveActivityCardKind::Diff | ProgressiveActivityCardKind::Patch) => {
            ProgressiveActivityDetailKind::Diff
        }
        Some(_) => ProgressiveActivityDetailKind::Output,
        None => fallback,
    }
}

struct ActivityDocumentPage {
    lines: Vec<Line<'static>>,
    start_byte: usize,
    end_byte: usize,
    current_cursor: ProgressiveActivityPageCursor,
    next_cursor: Option<ProgressiveActivityPageCursor>,
}

fn build_filter_line(
    card_filter: Option<ProgressiveActivityCardKind>,
    selected_kind: ProgressiveActivityDetailKind,
    diff_available: bool,
    output_available: bool,
) -> Line<'static> {
    if card_filter.is_none()
        && matches!(
            selected_kind,
            ProgressiveActivityDetailKind::Diff | ProgressiveActivityDetailKind::Output
        )
        && (diff_available || output_available)
    {
        // Keep the legacy Diff/Output tab chrome when the list is unfiltered and those
        // documents still exist so existing snapshots/operators retain familiar labels.
    }

    let filter_label = match card_filter {
        None => "all",
        Some(kind) => kind.label(),
    };
    Line::from(vec![
        Span::styled("filter: ", AkraTheme::muted()),
        Span::styled(filter_label.to_string(), AkraTheme::selected()),
        Span::raw("    "),
        tab_span(
            "Diff",
            matches!(selected_kind, ProgressiveActivityDetailKind::Diff)
                || card_filter == Some(ProgressiveActivityCardKind::Diff),
            diff_available || card_filter == Some(ProgressiveActivityCardKind::Diff),
        ),
        Span::raw("  "),
        tab_span(
            "Output",
            matches!(selected_kind, ProgressiveActivityDetailKind::Output)
                || card_filter == Some(ProgressiveActivityCardKind::Command),
            output_available || card_filter == Some(ProgressiveActivityCardKind::Command),
        ),
    ])
}

fn build_card_list_lines(
    cards: &[ProgressiveActivityCard],
    selected_card_index: usize,
    list_focus: bool,
) -> Vec<Line<'static>> {
    if cards.is_empty() {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let start = selected_card_index.saturating_sub(MAX_LIST_ROWS / 2);
    let end = (start + MAX_LIST_ROWS).min(cards.len());
    let start = end.saturating_sub(MAX_LIST_ROWS);
    for (offset, card) in cards[start..end].iter().enumerate() {
        let index = start + offset;
        let selected = index == selected_card_index;
        let marker = if selected {
            AkraTheme::selected_marker()
        } else {
            AkraTheme::idle_marker()
        };
        let indicator = if card.expandable {
            AkraTheme::collapsed_indicator()
        } else {
            AkraTheme::non_expandable_indicator()
        };
        let fact = if card.fact.is_empty() {
            String::new()
        } else {
            format!("  {}", card.fact)
        };
        let text = format!(
            "{marker}{indicator}{}{:<9} {}{fact}",
            AkraTheme::tool_card_bullet_glyph(),
            card.key.kind.label(),
            card.title
        );
        let style = if selected && list_focus {
            AkraTheme::selected()
        } else if selected {
            AkraTheme::accent()
        } else {
            AkraTheme::tool_card_header()
        };
        lines.push(Line::styled(text, style));
    }
    if cards.len() > MAX_LIST_ROWS {
        lines.push(Line::styled(
            format!("  showing {}-{} / {}", start + 1, end, cards.len()),
            AkraTheme::muted(),
        ));
    }
    lines
}

fn selected_card_detail_title(
    cards: &[ProgressiveActivityCard],
    selected_card_index: usize,
) -> String {
    cards
        .get(selected_card_index)
        .map(|card| format!("{} · {}", card.key.kind.label(), card.title))
        .unwrap_or_else(|| "Retained Activity Detail".to_string())
}

fn tab_span(label: &'static str, selected: bool, available: bool) -> Span<'static> {
    let marker = if selected { "> " } else { "  " };
    let style = if selected {
        AkraTheme::selected()
    } else if available {
        AkraTheme::muted()
    } else {
        AkraTheme::subtle()
    };
    Span::styled(format!("{marker}{label}"), style)
}

fn build_document_status_lines(document: &ActivityOverlayDocument<'_>) -> Vec<Line<'static>> {
    if document.truncated_bytes == 0 && !document.history_incomplete {
        return Vec::new();
    }

    let mut spans = vec![Span::styled(
        format!(
            "retained:{}/{} B",
            document.retained_bytes, document.source_bytes
        ),
        AkraTheme::warning(),
    )];
    if document.truncated_bytes > 0 {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("truncated:{} B", document.truncated_bytes),
            AkraTheme::warning(),
        ));
    }
    if document.history_incomplete {
        spans.push(Span::raw("  "));
        spans.push(Span::styled("history:incomplete", AkraTheme::warning()));
    }
    vec![Line::from(spans)]
}

fn build_activity_overlay_key_lines(viewport_width: u16) -> Vec<Line<'static>> {
    if viewport_width >= 96 {
        return vec![AkraTheme::key_line(
            "Up/Down: card | Enter/e: detail | PgUp/PgDn: page | Tab: filter | Home: first | Esc: close",
        )];
    }
    if viewport_width >= 64 {
        return vec![
            AkraTheme::key_line("Up/Down: card | Enter/e: detail | PgUp/PgDn: page"),
            AkraTheme::key_line("Tab: filter | Home: first | Esc: close"),
        ];
    }
    vec![
        AkraTheme::key_line("Up/Down: card | Enter/e: detail"),
        AkraTheme::key_line("PgUp/PgDn: page | Tab: filter"),
        AkraTheme::key_line("Home: first | Esc: close"),
    ]
}

struct BoundedDocumentPage {
    lines: Vec<Line<'static>>,
    start_byte: usize,
    end_byte: usize,
    next_page_start: Option<usize>,
    #[cfg(test)]
    scanned_bytes: usize,
    #[cfg(test)]
    rendered_bytes: usize,
}

fn build_bounded_document_page(
    text: &str,
    requested_start: usize,
    width: u16,
    height: u16,
) -> BoundedDocumentPage {
    let width = usize::from(width);
    let height = usize::from(height);
    let start_byte = clamped_char_boundary(text, requested_start);
    if width == 0 || height == 0 || start_byte == text.len() {
        return BoundedDocumentPage {
            lines: Vec::new(),
            start_byte,
            end_byte: start_byte,
            next_page_start: None,
            #[cfg(test)]
            scanned_bytes: 0,
            #[cfg(test)]
            rendered_bytes: 0,
        };
    }

    let cell_budget = width.saturating_mul(height);
    let scan_budget = cell_budget.saturating_mul(PAGE_SCAN_BYTES_PER_CELL).max(4);
    let output_budget = cell_budget
        .saturating_mul(PAGE_OUTPUT_BYTES_PER_CELL)
        .max(8);
    let mut completed_rows = Vec::with_capacity(height);
    let mut current_row = String::with_capacity(width.min(output_budget));
    let mut current_width = 0usize;
    let mut cursor = start_byte;
    let mut scanned_bytes = 0usize;
    let mut rendered_bytes = 0usize;

    while cursor < text.len() && completed_rows.len() < height {
        let character = text[cursor..]
            .chars()
            .next()
            .expect("cursor must remain on a character boundary");
        let source_bytes = character.len_utf8();
        if scanned_bytes.saturating_add(source_bytes) > scan_budget {
            break;
        }

        if character == '\n' {
            completed_rows.push(Line::from(std::mem::take(&mut current_row)));
            current_width = 0;
            cursor += source_bytes;
            scanned_bytes += source_bytes;
            continue;
        }

        let mut utf8 = [0; 4];
        let mut escaped = character.is_control().then(|| escape_control(character));
        let mut token = escaped
            .as_deref()
            .unwrap_or_else(|| character.encode_utf8(&mut utf8));
        let mut token_width = if character.is_ascii() {
            token.len()
        } else {
            Line::from(token).width()
        };
        if token_width == 0 {
            escaped = Some(format!("\\u{{{:x}}}", u32::from(character)));
            token = escaped.as_deref().expect("zero-width escape must exist");
            token_width = token.len();
        }
        if (!character.is_control() && token_width > width)
            || token_width > cell_budget
            || token.len() > output_budget
        {
            token = "?";
            token_width = 1;
        }

        let used_cells = completed_rows
            .len()
            .saturating_mul(width)
            .saturating_add(current_width);
        let remaining_cells = cell_budget.saturating_sub(used_cells);
        if token_width > remaining_cells
            || rendered_bytes.saturating_add(token.len()) > output_budget
        {
            break;
        }

        append_token(
            token,
            width,
            height,
            &mut completed_rows,
            &mut current_row,
            &mut current_width,
        );
        cursor += source_bytes;
        scanned_bytes += source_bytes;
        rendered_bytes += token.len();
    }

    if !current_row.is_empty() && completed_rows.len() < height {
        completed_rows.push(Line::from(current_row));
    }

    BoundedDocumentPage {
        lines: completed_rows,
        start_byte,
        end_byte: cursor,
        next_page_start: (cursor < text.len()).then_some(cursor),
        #[cfg(test)]
        scanned_bytes,
        #[cfg(test)]
        rendered_bytes,
    }
}

fn append_token(
    token: &str,
    width: usize,
    height: usize,
    completed_rows: &mut Vec<Line<'static>>,
    current_row: &mut String,
    current_width: &mut usize,
) {
    for character in token.chars() {
        let character_width = if character.is_ascii() {
            1
        } else {
            Line::from(character.to_string()).width()
        };
        if current_width.saturating_add(character_width) > width {
            completed_rows.push(Line::from(std::mem::take(current_row)));
            *current_width = 0;
        }
        if completed_rows.len() == height {
            return;
        }
        current_row.push(character);
        *current_width += character_width;
        if *current_width == width && completed_rows.len() + 1 < height {
            completed_rows.push(Line::from(std::mem::take(current_row)));
            *current_width = 0;
        }
    }
}

fn escape_control(character: char) -> String {
    match character {
        '\r' => "\\r".to_string(),
        '\t' => "\\t".to_string(),
        '\u{1b}' => "\\x1b".to_string(),
        other => format!("\\u{{{:x}}}", u32::from(other)),
    }
}

fn clamped_char_boundary(text: &str, requested: usize) -> usize {
    let mut boundary = requested.min(text.len());
    while !text.is_char_boundary(boundary) {
        boundary = boundary.saturating_sub(1);
    }
    boundary
}

#[cfg(test)]
mod tests {
    use super::{
        ActivityOverlayDocument, BoundedDocumentPage, PAGE_OUTPUT_BYTES_PER_CELL,
        PAGE_SCAN_BYTES_PER_CELL, ProgressiveActivityCardKind, ProgressiveActivityDetailKind,
        ProgressiveActivityPageCursor, activity_document_detail_kind, build_activity_overlay_view,
        build_bounded_document_page,
    };

    fn rendered_text(page: &BoundedDocumentPage) -> String {
        page.lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn long_single_line_materializes_only_the_current_page() {
        let text = "x".repeat(2 * 1024 * 1024);
        let page = build_bounded_document_page(&text, 0, 40, 3);

        assert_eq!(page.lines.len(), 3);
        assert!(page.lines.iter().all(|line| line.width() <= 40));
        assert_eq!(page.end_byte, 120);
        assert_eq!(page.next_page_start, Some(120));
        assert!(page.scanned_bytes <= 40 * 3 * PAGE_SCAN_BYTES_PER_CELL);
    }

    #[test]
    fn newline_flood_consumes_at_most_one_viewport() {
        let text = "\n".repeat(1_000_000);
        let page = build_bounded_document_page(&text, 0, 80, 4);

        assert_eq!(page.lines.len(), 4);
        assert_eq!(page.end_byte, 4);
        assert_eq!(page.next_page_start, Some(4));
        assert_eq!(page.scanned_bytes, 4);
    }

    #[test]
    fn unicode_is_width_aware_and_controls_are_rendered_as_text() {
        let text = "한글\u{1b}[31m\t끝\u{7}";
        let page = build_bounded_document_page(text, 0, 24, 3);
        let rendered = rendered_text(&page);

        assert!(rendered.contains("한글\\x1b[31m\\t끝\\u{7}"));
        assert!(!rendered.contains('\u{1b}'));
        assert!(!rendered.contains('\u{7}'));
        assert!(page.lines.iter().all(|line| line.width() <= 24));
    }

    #[test]
    fn one_cell_viewport_replaces_wide_characters_without_overflow() {
        let page = build_bounded_document_page("한글", 0, 1, 2);

        assert_eq!(rendered_text(&page), "?\n?");
        assert!(page.lines.iter().all(|line| line.width() <= 1));
    }

    #[test]
    fn card_kind_selects_semantic_diff_paging_only_for_diff_content() {
        assert_eq!(
            activity_document_detail_kind(
                Some(ProgressiveActivityCardKind::Diff),
                ProgressiveActivityDetailKind::Output,
            ),
            ProgressiveActivityDetailKind::Diff
        );
        assert_eq!(
            activity_document_detail_kind(
                Some(ProgressiveActivityCardKind::Patch),
                ProgressiveActivityDetailKind::Output,
            ),
            ProgressiveActivityDetailKind::Diff
        );
        assert_eq!(
            activity_document_detail_kind(
                Some(ProgressiveActivityCardKind::Command),
                ProgressiveActivityDetailKind::Diff,
            ),
            ProgressiveActivityDetailKind::Output
        );
    }

    #[test]
    fn metadata_reports_truncation_and_incomplete_history_exactly() {
        let view = build_activity_overlay_view(
            ProgressiveActivityDetailKind::Diff,
            true,
            false,
            Some(ActivityOverlayDocument {
                text: "@@ -1 +1 @@\n-old\n+new",
                source_bytes: 100,
                retained_bytes: 24,
                truncated_bytes: 76,
                history_incomplete: true,
            }),
            ProgressiveActivityPageCursor::at(0),
            80,
            8,
        );
        let status = view
            .header_lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(status.contains("retained:24/100 B"));
        assert!(status.contains("truncated:76 B"));
        assert!(status.contains("history:incomplete"));
        assert!(!status.contains("sequence:"));
    }

    #[test]
    fn complete_retained_document_omits_zero_value_metadata() {
        let view = build_activity_overlay_view(
            ProgressiveActivityDetailKind::Output,
            false,
            true,
            Some(ActivityOverlayDocument {
                text: "complete output",
                source_bytes: 15,
                retained_bytes: 15,
                truncated_bytes: 0,
                history_incomplete: false,
            }),
            ProgressiveActivityPageCursor::at(0),
            48,
            4,
        );
        let header = view
            .header_lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(!view.header_lines.is_empty());
        assert!(!header.contains("truncated:0"));
        assert!(!header.contains("history:complete"));
        assert_eq!(view.key_lines.len(), 3);
        assert!(
            view.key_lines[0].to_string().contains("Esc: close")
                || view
                    .key_lines
                    .iter()
                    .any(|line| line.to_string().contains("Esc: close"))
        );
        assert!(
            view.key_lines
                .iter()
                .any(|line| line.to_string().contains("Tab"))
        );
    }

    #[test]
    fn scan_and_output_work_stay_within_viewport_budgets() {
        let text = "\u{301}".repeat(1_000_000);
        let width = 17usize;
        let height = 5usize;
        let page = build_bounded_document_page(text.as_str(), 0, width as u16, height as u16);
        let output_bytes = page
            .lines
            .iter()
            .map(|line| line.to_string().len())
            .sum::<usize>();

        assert!(page.scanned_bytes <= width * height * PAGE_SCAN_BYTES_PER_CELL);
        assert!(page.rendered_bytes <= width * height * PAGE_OUTPUT_BYTES_PER_CELL);
        assert!(output_bytes <= width * height * PAGE_OUTPUT_BYTES_PER_CELL);
        assert!(page.next_page_start.is_some());
    }

    #[test]
    fn empty_document_has_a_truthful_selected_kind_state() {
        let view = build_activity_overlay_view(
            ProgressiveActivityDetailKind::Output,
            false,
            false,
            None,
            ProgressiveActivityPageCursor::at(usize::MAX),
            48,
            4,
        );

        assert_eq!(view.current_page_cursor.byte_offset, 0);
        assert_eq!(view.next_page_cursor, None);
        assert!(
            view.detail_lines[0]
                .to_string()
                .contains("No retained detail is available")
        );
        assert!(view.header_lines[0].to_string().contains("filter:"));
    }
}
