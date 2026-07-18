use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::super::super::{
    AkraTheme, ProgressiveActivityDiffContinuation, ProgressiveActivityDiffCursor,
    ProgressiveActivityDiffLineKind, ProgressiveActivityPageCursor,
};

const PAGE_SCAN_BYTES_PER_CELL: usize = 8;
const PAGE_OUTPUT_BYTES_PER_CELL: usize = 8;
const MIN_DIFF_HEADER_SCAN_BYTES: usize = 128;

pub(super) struct BoundedDiffPage {
    pub(super) lines: Vec<Line<'static>>,
    pub(super) current_cursor: ProgressiveActivityPageCursor,
    pub(super) end_byte: usize,
    pub(super) next_cursor: Option<ProgressiveActivityPageCursor>,
    #[cfg(test)]
    pub(super) scanned_bytes: usize,
    #[cfg(test)]
    pub(super) rendered_bytes: usize,
}

pub(super) fn build_bounded_diff_page(
    text: &str,
    requested_cursor: ProgressiveActivityPageCursor,
    width: u16,
    height: u16,
) -> BoundedDiffPage {
    let width = usize::from(width);
    let height = usize::from(height);
    if width == 0 || height == 0 {
        return empty_page(clamped_requested_cursor(text, requested_cursor));
    }
    let current_cursor = normalized_diff_cursor(text, requested_cursor);
    if current_cursor.byte_offset == text.len() {
        return empty_page(current_cursor);
    }

    let cell_budget = width.saturating_mul(height);
    let scan_budget = cell_budget
        .saturating_mul(PAGE_SCAN_BYTES_PER_CELL)
        .max(MIN_DIFF_HEADER_SCAN_BYTES);
    let output_budget = cell_budget
        .saturating_mul(PAGE_OUTPUT_BYTES_PER_CELL)
        .max(8);
    let mut cursor = current_cursor;
    let mut lines = Vec::with_capacity(height);
    let mut scanned_bytes = 0usize;
    let mut rendered_bytes = 0usize;

    while cursor.byte_offset < text.len() && lines.len() < height && rendered_bytes < output_budget
    {
        let diff = cursor
            .diff
            .as_mut()
            .expect("normalized diff cursor must carry parser state");
        if let Some(mut continuation) = diff.continuation {
            if cursor.byte_offset == continuation.line_end && !continuation.line_complete {
                let remaining_scan = scan_budget.saturating_sub(scanned_bytes);
                let Some(segment) =
                    bounded_source_segment(text, cursor.byte_offset, remaining_scan)
                else {
                    break;
                };
                continuation.line_end = segment.end;
                continuation.line_complete = segment.complete;
                diff.continuation = Some(continuation);
                scanned_bytes = scanned_bytes.saturating_add(segment.scanned_bytes);
            }
            if continuation.hidden {
                cursor.byte_offset = continuation.line_end;
                if continuation.line_complete {
                    cursor.byte_offset = next_line_start(text, continuation.line_end);
                    diff.continuation = None;
                } else {
                    diff.continuation = Some(continuation);
                }
                continue;
            }
            let (line, next_byte, output_bytes) = render_source_chunk(
                text,
                cursor.byte_offset,
                continuation.line_end,
                width,
                diff.gutter_width,
                continuation.line_number,
                continuation.kind,
                true,
            );
            rendered_bytes = rendered_bytes.saturating_add(output_bytes);
            lines.push(line);
            cursor.byte_offset = next_byte;
            if next_byte == continuation.line_end {
                if continuation.line_complete {
                    cursor.byte_offset = next_line_start(text, continuation.line_end);
                    diff.continuation = None;
                } else {
                    diff.continuation = Some(continuation);
                }
            }
            continue;
        }

        let line_start = cursor.byte_offset;
        let remaining_scan = scan_budget.saturating_sub(scanned_bytes);
        let Some(segment) = bounded_source_segment(text, line_start, remaining_scan) else {
            break;
        };
        let line_end = segment.end;
        let source_line = &text[line_start..line_end];
        let next_start = if segment.complete {
            next_line_start(text, line_end)
        } else {
            line_end
        };
        scanned_bytes = scanned_bytes.saturating_add(segment.scanned_bytes);

        let (file_headers, header_scan_bytes) = if !diff.in_hunk {
            bounded_file_header_pair(
                text,
                line_start,
                line_end,
                segment.complete,
                scan_budget.saturating_sub(scanned_bytes),
            )
        } else {
            (None, 0)
        };
        scanned_bytes = scanned_bytes.saturating_add(header_scan_bytes);
        if let Some((old_path, new_path, after_headers)) = file_headers {
            let summary = file_summary(old_path, new_path);
            lines.push(Line::styled(
                safe_generated_line(&summary, width),
                AkraTheme::diff_hunk(),
            ));
            rendered_bytes = rendered_bytes.saturating_add(summary.len().min(width));
            cursor.byte_offset = after_headers;
            diff.in_hunk = false;
            diff.seen_hunk = false;
            diff.continuation = None;
            continue;
        }

        if segment.complete && source_line.starts_with("diff --git ") {
            diff.in_hunk = false;
            diff.seen_hunk = false;
            diff.old_remaining = 0;
            diff.new_remaining = 0;
            cursor.byte_offset = next_start;
            continue;
        }

        if segment.complete && is_suppressed_diff_metadata(source_line) {
            cursor.byte_offset = next_start;
            continue;
        }

        if let Some((old_start, old_count, new_start, new_count)) = parse_hunk_ranges(source_line) {
            diff.old_line = old_start;
            diff.new_line = new_start;
            diff.old_remaining = old_count;
            diff.new_remaining = new_count;
            diff.in_hunk = old_count > 0 || new_count > 0;
            diff.gutter_width = diff.gutter_width.max(hunk_gutter_width(
                old_start, old_count, new_start, new_count,
            ));
            cursor.byte_offset = next_start;
            if diff.seen_hunk {
                lines.push(hunk_separator_line(diff.gutter_width, width));
                rendered_bytes = rendered_bytes.saturating_add(1);
            }
            diff.seen_hunk = true;
            if !segment.complete {
                diff.continuation = Some(ProgressiveActivityDiffContinuation {
                    kind: ProgressiveActivityDiffLineKind::Metadata,
                    line_number: None,
                    line_end,
                    line_complete: false,
                    hidden: true,
                });
            }
            continue;
        }

        let (kind, line_number, content_start) = classify_diff_line(source_line, line_start, diff);
        let (line, next_byte, output_bytes) = render_source_chunk(
            text,
            content_start,
            line_end,
            width,
            diff.gutter_width,
            line_number,
            kind,
            false,
        );
        rendered_bytes = rendered_bytes.saturating_add(output_bytes);
        lines.push(line);
        if next_byte < line_end {
            cursor.byte_offset = next_byte;
            diff.continuation = Some(ProgressiveActivityDiffContinuation {
                kind,
                line_number,
                line_end,
                line_complete: segment.complete,
                hidden: false,
            });
        } else if !segment.complete {
            cursor.byte_offset = line_end;
            diff.continuation = Some(ProgressiveActivityDiffContinuation {
                kind,
                line_number,
                line_end,
                line_complete: false,
                hidden: false,
            });
        } else {
            cursor.byte_offset = next_start;
        }
    }

    if lines.is_empty() && cursor.byte_offset > current_cursor.byte_offset {
        lines.push(Line::styled(
            safe_generated_line("… diff metadata continues", width),
            AkraTheme::diff_metadata(),
        ));
    }

    let next_cursor = (cursor.byte_offset < text.len()
        && cursor.byte_offset > current_cursor.byte_offset)
        .then_some(cursor);
    BoundedDiffPage {
        lines,
        current_cursor,
        end_byte: cursor.byte_offset,
        next_cursor,
        #[cfg(test)]
        scanned_bytes,
        #[cfg(test)]
        rendered_bytes,
    }
}

fn empty_page(cursor: ProgressiveActivityPageCursor) -> BoundedDiffPage {
    BoundedDiffPage {
        lines: Vec::new(),
        current_cursor: cursor,
        end_byte: cursor.byte_offset,
        next_cursor: None,
        #[cfg(test)]
        scanned_bytes: 0,
        #[cfg(test)]
        rendered_bytes: 0,
    }
}

fn normalized_diff_cursor(
    text: &str,
    requested: ProgressiveActivityPageCursor,
) -> ProgressiveActivityPageCursor {
    let requested = clamped_requested_cursor(text, requested);
    let byte_offset = requested.byte_offset;
    let diff = requested.diff;
    if let Some(diff) = diff {
        return ProgressiveActivityPageCursor {
            byte_offset,
            diff: Some(diff),
        };
    }

    // Cursor-free callers are compatibility/test paths. Production navigation
    // stores the exact parser cursor returned by this builder.
    ProgressiveActivityPageCursor {
        byte_offset: 0,
        diff: Some(ProgressiveActivityDiffCursor {
            old_line: 0,
            new_line: 0,
            old_remaining: 0,
            new_remaining: 0,
            in_hunk: false,
            seen_hunk: false,
            gutter_width: diff_gutter_width(text),
            continuation: None,
        }),
    }
}

fn clamped_requested_cursor(
    text: &str,
    requested: ProgressiveActivityPageCursor,
) -> ProgressiveActivityPageCursor {
    let mut byte_offset = requested.byte_offset.min(text.len());
    while !text.is_char_boundary(byte_offset) {
        byte_offset = byte_offset.saturating_sub(1);
    }
    ProgressiveActivityPageCursor {
        byte_offset,
        diff: requested
            .diff
            .filter(|_| byte_offset == requested.byte_offset),
    }
}

fn hunk_gutter_width(
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
) -> usize {
    [
        range_last_line(old_start, old_count),
        range_last_line(new_start, new_count),
    ]
    .into_iter()
    .max()
    .unwrap_or(0)
    .to_string()
    .len()
    .max(1)
}

fn diff_gutter_width(text: &str) -> usize {
    text.lines()
        .filter_map(parse_hunk_ranges)
        .map(|(old_start, old_count, new_start, new_count)| {
            hunk_gutter_width(old_start, old_count, new_start, new_count)
        })
        .max()
        .unwrap_or(1)
}

fn range_last_line(start: usize, count: usize) -> usize {
    start.saturating_add(count.saturating_sub(1))
}

fn parse_hunk_ranges(line: &str) -> Option<(usize, usize, usize, usize)> {
    let ranges = line.strip_prefix("@@ -")?;
    let (old_range, after_old) = ranges.split_once(" +")?;
    let (new_range, _) = after_old.split_once(" @@")?;
    let (old_start, old_count) = parse_range(old_range)?;
    let (new_start, new_count) = parse_range(new_range)?;
    Some((old_start, old_count, new_start, new_count))
}

fn parse_range(value: &str) -> Option<(usize, usize)> {
    let (start, count) = value
        .split_once(',')
        .map_or((value, "1"), |(start, count)| (start, count));
    Some((start.parse().ok()?, count.parse().ok()?))
}

struct BoundedSourceSegment {
    end: usize,
    complete: bool,
    scanned_bytes: usize,
}

fn bounded_source_segment(
    text: &str,
    start: usize,
    max_scan_bytes: usize,
) -> Option<BoundedSourceSegment> {
    if start == text.len() {
        return Some(BoundedSourceSegment {
            end: start,
            complete: true,
            scanned_bytes: 0,
        });
    }
    let mut scan_end = start.saturating_add(max_scan_bytes).min(text.len());
    while scan_end > start && !text.is_char_boundary(scan_end) {
        scan_end -= 1;
    }
    if scan_end == start {
        return None;
    }
    if let Some(newline_offset) = text[start..scan_end].find('\n') {
        return Some(BoundedSourceSegment {
            end: start + newline_offset,
            complete: true,
            scanned_bytes: newline_offset + 1,
        });
    }
    Some(BoundedSourceSegment {
        end: scan_end,
        complete: scan_end == text.len(),
        scanned_bytes: scan_end - start,
    })
}

fn bounded_file_header_pair(
    text: &str,
    line_start: usize,
    line_end: usize,
    line_complete: bool,
    max_scan_bytes: usize,
) -> (Option<(&str, &str, usize)>, usize) {
    if !line_complete {
        return (None, 0);
    }
    let Some(old_line) = text[line_start..line_end].strip_prefix("--- ") else {
        return (None, 0);
    };
    let new_start = next_line_start(text, line_end);
    let Some(new_segment) = bounded_source_segment(text, new_start, max_scan_bytes) else {
        return (None, 0);
    };
    if !new_segment.complete {
        return (None, new_segment.scanned_bytes);
    }
    let Some(new_line) = text
        .get(new_start..new_segment.end)
        .and_then(|line| line.strip_prefix("+++ "))
    else {
        return (None, new_segment.scanned_bytes);
    };
    (
        Some((
            path_token(old_line),
            path_token(new_line),
            next_line_start(text, new_segment.end),
        )),
        new_segment.scanned_bytes,
    )
}

fn path_token(line: &str) -> &str {
    line.split_once('\t').map_or(line, |(path, _)| path).trim()
}

fn file_summary(old_path: &str, new_path: &str) -> String {
    let old_path = display_diff_path(old_path);
    let new_path = display_diff_path(new_path);
    if old_path == "/dev/null" {
        return format!("• Added {new_path}");
    }
    if new_path == "/dev/null" {
        return format!("• Deleted {old_path}");
    }
    if old_path == new_path {
        format!("• Edited {new_path}")
    } else {
        format!("• Edited {old_path} → {new_path}")
    }
}

fn display_diff_path(path: &str) -> &str {
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
}

fn is_suppressed_diff_metadata(line: &str) -> bool {
    line.starts_with("diff --git ")
        || line.starts_with("index ")
        || line.starts_with("new file mode ")
        || line.starts_with("deleted file mode ")
        || line.starts_with("similarity index ")
        || line.starts_with("rename from ")
        || line.starts_with("rename to ")
}

fn classify_diff_line(
    line: &str,
    line_start: usize,
    cursor: &mut ProgressiveActivityDiffCursor,
) -> (ProgressiveActivityDiffLineKind, Option<usize>, usize) {
    if line.starts_with("\\ ") {
        return (ProgressiveActivityDiffLineKind::Note, None, line_start);
    }
    if !cursor.in_hunk {
        return (ProgressiveActivityDiffLineKind::Metadata, None, line_start);
    }
    if line.starts_with('+') {
        let line_number = cursor.new_line;
        cursor.new_line = cursor.new_line.saturating_add(1);
        cursor.new_remaining = cursor.new_remaining.saturating_sub(1);
        finish_hunk_if_consumed(cursor);
        return (
            ProgressiveActivityDiffLineKind::Insert,
            Some(line_number),
            line_start + 1,
        );
    }
    if line.starts_with('-') {
        let line_number = cursor.old_line;
        cursor.old_line = cursor.old_line.saturating_add(1);
        cursor.old_remaining = cursor.old_remaining.saturating_sub(1);
        finish_hunk_if_consumed(cursor);
        return (
            ProgressiveActivityDiffLineKind::Delete,
            Some(line_number),
            line_start + 1,
        );
    }
    if line.starts_with(' ') {
        let line_number = cursor.new_line;
        cursor.old_line = cursor.old_line.saturating_add(1);
        cursor.new_line = cursor.new_line.saturating_add(1);
        cursor.old_remaining = cursor.old_remaining.saturating_sub(1);
        cursor.new_remaining = cursor.new_remaining.saturating_sub(1);
        finish_hunk_if_consumed(cursor);
        return (
            ProgressiveActivityDiffLineKind::Context,
            Some(line_number),
            line_start + 1,
        );
    }
    (ProgressiveActivityDiffLineKind::Metadata, None, line_start)
}

fn finish_hunk_if_consumed(cursor: &mut ProgressiveActivityDiffCursor) {
    if cursor.old_remaining == 0 && cursor.new_remaining == 0 {
        cursor.in_hunk = false;
    }
}

#[allow(clippy::too_many_arguments)]
fn render_source_chunk(
    text: &str,
    start: usize,
    end: usize,
    width: usize,
    gutter_width: usize,
    line_number: Option<usize>,
    kind: ProgressiveActivityDiffLineKind,
    continuation: bool,
) -> (Line<'static>, usize, usize) {
    let visible_gutter_width = gutter_width.min(width.saturating_sub(3));
    let prefix_width = if width >= 3 {
        visible_gutter_width.saturating_add(2)
    } else {
        0
    };
    let content_width = width.saturating_sub(prefix_width).max(1);
    let (content, next_byte) = display_source_chunk(text, start, end, content_width);
    let output_bytes = content.len();
    if prefix_width == 0 {
        return (
            Line::styled(content, diff_content_style(kind)),
            next_byte,
            output_bytes,
        );
    }

    let gutter = if continuation || visible_gutter_width == 0 {
        " ".repeat(visible_gutter_width + 1)
    } else {
        line_number
            .filter(|number| number.to_string().len() <= visible_gutter_width)
            .map_or_else(
                || " ".repeat(visible_gutter_width + 1),
                |number| format!("{number:>visible_gutter_width$} "),
            )
    };
    let sign = if continuation { " " } else { diff_sign(kind) };
    (
        Line::from(vec![
            Span::styled(gutter, AkraTheme::diff_gutter()),
            Span::styled(sign, diff_sign_style(kind)),
            Span::styled(content, diff_content_style(kind)),
        ]),
        next_byte,
        output_bytes,
    )
}

fn display_source_chunk(text: &str, start: usize, end: usize, width: usize) -> (String, usize) {
    if start == end {
        return (String::new(), end);
    }
    let mut output = String::new();
    let mut output_width = 0usize;
    let mut cursor = start;
    while cursor < end {
        let character = text[cursor..end]
            .chars()
            .next()
            .expect("source cursor must remain on a character boundary");
        let token = display_token(character);
        let token_width = display_token_width(character, &token);
        if output_width.saturating_add(token_width) > width {
            if output.is_empty() {
                output.push('?');
                cursor += character.len_utf8();
            }
            break;
        }
        output.push_str(&token);
        output_width = output_width.saturating_add(token_width);
        cursor += character.len_utf8();
    }
    (output, cursor)
}

fn display_token(character: char) -> String {
    match character {
        '\t' => "    ".to_string(),
        '\r' => "\\r".to_string(),
        '\u{1b}' => "\\x1b".to_string(),
        other if other.is_control() => format!("\\u{{{:x}}}", u32::from(other)),
        other if other.is_ascii() => other.to_string(),
        other => {
            let token = other.to_string();
            if Line::from(token.as_str()).width() == 0 {
                format!("\\u{{{:x}}}", u32::from(other))
            } else {
                token
            }
        }
    }
}

fn display_token_width(character: char, token: &str) -> usize {
    if character.is_ascii() {
        token.len()
    } else {
        Line::from(token).width().max(1)
    }
}

fn hunk_separator_line(gutter_width: usize, width: usize) -> Line<'static> {
    if width == 1 {
        return Line::styled("⋮", AkraTheme::diff_hunk());
    }
    let gutter_width = gutter_width.min(width.saturating_sub(2));
    Line::from(vec![
        Span::styled(" ".repeat(gutter_width + 1), AkraTheme::diff_gutter()),
        Span::styled("⋮", AkraTheme::diff_hunk()),
    ])
}

fn safe_generated_line(text: &str, width: usize) -> String {
    let mut output = String::new();
    let mut output_width = 0usize;
    for character in text.chars() {
        let token = display_token(character);
        let token_width = display_token_width(character, &token);
        if output_width.saturating_add(token_width) > width {
            if width > 0 {
                while Line::from(output.as_str()).width().saturating_add(1) > width {
                    output.pop();
                }
                output.push('…');
            }
            break;
        }
        output.push_str(&token);
        output_width = output_width.saturating_add(token_width);
    }
    output
}

fn diff_sign(kind: ProgressiveActivityDiffLineKind) -> &'static str {
    match kind {
        ProgressiveActivityDiffLineKind::Insert => "+",
        ProgressiveActivityDiffLineKind::Delete => "-",
        ProgressiveActivityDiffLineKind::Context
        | ProgressiveActivityDiffLineKind::Metadata
        | ProgressiveActivityDiffLineKind::Note => " ",
    }
}

fn diff_sign_style(kind: ProgressiveActivityDiffLineKind) -> Style {
    match kind {
        ProgressiveActivityDiffLineKind::Insert => AkraTheme::diff_addition(),
        ProgressiveActivityDiffLineKind::Delete => AkraTheme::diff_deletion(),
        ProgressiveActivityDiffLineKind::Context
        | ProgressiveActivityDiffLineKind::Metadata
        | ProgressiveActivityDiffLineKind::Note => AkraTheme::diff_gutter(),
    }
}

fn diff_content_style(kind: ProgressiveActivityDiffLineKind) -> Style {
    match kind {
        ProgressiveActivityDiffLineKind::Insert => AkraTheme::diff_addition(),
        ProgressiveActivityDiffLineKind::Delete => AkraTheme::diff_deletion(),
        ProgressiveActivityDiffLineKind::Context => Style::default(),
        ProgressiveActivityDiffLineKind::Metadata => AkraTheme::diff_metadata(),
        ProgressiveActivityDiffLineKind::Note => AkraTheme::diff_metadata(),
    }
}

fn next_line_start(text: &str, line_end: usize) -> usize {
    if text.as_bytes().get(line_end) == Some(&b'\n') {
        line_end + 1
    } else {
        line_end
    }
}

#[cfg(test)]
#[path = "activity_diff/tests.rs"]
mod tests;
