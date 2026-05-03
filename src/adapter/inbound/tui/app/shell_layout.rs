// Test-only shell layout helpers mirror the production frame math used by overlays/base.rs.
// They exist so rendering contract tests can assert scroll and height behavior without snapshot-string guessing.
#[cfg(test)]
use ratatui::text::Line;

// Reuse the real shell budgets so test projections move with production composer/footer constraints.
#[cfg(test)]
use super::{
    MAX_COMPOSER_HEIGHT, MAX_SHELL_STATUS_HEIGHT, MIN_COMPOSER_HEIGHT, MIN_SHELL_STATUS_HEIGHT,
};

// Compute the scroll offset that keeps the newest transcript rows visible in a bounded viewport.
#[cfg(test)]
pub(super) fn build_conversation_scroll_offset(
    lines: &[Line<'static>],
    content_width: u16,
    visible_height: u16,
) -> u16 {
    // No drawable width or height means there is no meaningful "latest row" window to align.
    if content_width == 0 || visible_height == 0 {
        return 0;
    }

    // Count physical rows after wrapping, not logical transcript entries, to match ratatui paragraph layout.
    let rendered_line_count = count_rendered_conversation_lines(lines, content_width);
    let visible_height = visible_height as usize;
    rendered_line_count
        .saturating_sub(visible_height)
        // Ratatui scroll offsets are u16, so extremely long transcripts are clamped at the presentation boundary.
        .min(u16::MAX as usize) as u16
}

// Reconstruct the transcript panel's rendered row count for contract tests.
#[cfg(test)]
fn count_rendered_conversation_lines(lines: &[Line<'static>], content_width: u16) -> usize {
    if content_width == 0 {
        return 0;
    }

    lines
        .iter()
        // Line::width honors ratatui span display width, which is the unit the renderer wraps on.
        .map(|line| count_wrapped_rows(line, content_width))
        .sum()
}

// Composer height is content rows plus chrome, clamped so long prompts do not consume the transcript.
#[cfg(test)]
pub(super) fn build_input_block_height(lines: &[Line<'_>]) -> u16 {
    block_height_for_lines(lines, MIN_COMPOSER_HEIGHT, MAX_COMPOSER_HEIGHT)
}

// Footer/status uses the same block formula with its own budget because it carries denser runtime copy.
#[cfg(test)]
pub(super) fn build_shell_footer_height(lines: &[Line<'_>]) -> u16 {
    block_height_for_lines(lines, MIN_SHELL_STATUS_HEIGHT, MAX_SHELL_STATUS_HEIGHT)
}

// Shared shell block formula: content lines plus two rows of panel chrome, bounded by the caller's budget.
#[cfg(test)]
pub(super) fn block_height_for_lines(lines: &[Line<'_>], min_height: u16, max_height: u16) -> u16 {
    lines
        .len()
        .saturating_add(2)
        .clamp(min_height as usize, max_height as usize) as u16
}

// Convert one styled logical line into the number of physical terminal rows it occupies.
#[cfg(test)]
fn count_wrapped_rows(line: &Line<'static>, content_width: u16) -> usize {
    if content_width == 0 {
        return 0;
    }

    let line_width = line.width();
    // Empty logical lines still render as a blank row in the transcript paragraph.
    if line_width == 0 {
        return 1;
    }

    line_width.div_ceil(content_width as usize)
}

// These tests lock the shell contract: transcript scrolling follows rendered rows after wrapping.
#[cfg(test)]
mod tests {
    use ratatui::text::Line;

    use super::{build_conversation_scroll_offset, count_rendered_conversation_lines};

    // When transcript rows exceed the viewport, the offset discards the oldest rows first.
    #[test]
    fn conversation_scroll_offset_moves_to_latest_rows() {
        let lines = vec![
            Line::from("line-1"),
            Line::from("line-2"),
            Line::from("line-3"),
            Line::from("line-4"),
        ];

        let scroll_offset = build_conversation_scroll_offset(&lines, 20, 2);

        assert_eq!(scroll_offset, 2);
    }

    // A single long logical line can occupy several terminal rows and must affect scroll math.
    #[test]
    fn conversation_scroll_offset_counts_wrapped_rows() {
        let lines = vec![Line::from("1234567890"), Line::from("tail")];

        let rendered_line_count = count_rendered_conversation_lines(&lines, 4);
        let scroll_offset = build_conversation_scroll_offset(&lines, 4, 2);

        assert_eq!(rendered_line_count, 4);
        assert_eq!(scroll_offset, 2);
    }

    // Zero-height areas appear during degenerate terminal sizes and must not underflow scroll math.
    #[test]
    fn conversation_scroll_offset_handles_zero_visible_height() {
        let lines = vec![Line::from("line-1")];

        let scroll_offset = build_conversation_scroll_offset(&lines, 10, 0);

        assert_eq!(scroll_offset, 0);
    }
}
