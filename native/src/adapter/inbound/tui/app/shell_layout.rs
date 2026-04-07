use ratatui::text::Line;

use super::{
    MAX_COMPOSER_HEIGHT, MAX_SHELL_STATUS_HEIGHT, MIN_COMPOSER_HEIGHT, MIN_SHELL_STATUS_HEIGHT,
};

pub(super) fn build_conversation_scroll_offset(
    lines: &[Line<'static>],
    content_width: u16,
    visible_height: u16,
) -> u16 {
    if content_width == 0 || visible_height == 0 {
        return 0;
    }

    let rendered_line_count = count_rendered_conversation_lines(lines, content_width);
    let visible_height = visible_height as usize;
    rendered_line_count
        .saturating_sub(visible_height)
        .min(u16::MAX as usize) as u16
}

pub(super) fn count_rendered_conversation_lines(
    lines: &[Line<'static>],
    content_width: u16,
) -> usize {
    if content_width == 0 {
        return 0;
    }

    lines
        .iter()
        .map(|line| count_wrapped_rows(line, content_width))
        .sum()
}

pub(super) fn build_input_block_height(lines: &[Line<'_>]) -> u16 {
    (lines.len() as u16 + 2).clamp(MIN_COMPOSER_HEIGHT, MAX_COMPOSER_HEIGHT)
}

pub(super) fn build_shell_footer_height(lines: &[Line<'_>]) -> u16 {
    (lines.len() as u16 + 2).clamp(MIN_SHELL_STATUS_HEIGHT, MAX_SHELL_STATUS_HEIGHT)
}

pub(super) fn block_height_for_lines(lines: &[Line<'_>], min_height: u16, max_height: u16) -> u16 {
    (lines.len() as u16 + 2).clamp(min_height, max_height)
}

fn count_wrapped_rows(line: &Line<'static>, content_width: u16) -> usize {
    if content_width == 0 {
        return 0;
    }

    let line_width = line.width();
    if line_width == 0 {
        return 1;
    }

    line_width.div_ceil(content_width as usize)
}

#[cfg(test)]
mod tests {
    use ratatui::text::Line;

    use super::{build_conversation_scroll_offset, count_rendered_conversation_lines};

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

    #[test]
    fn conversation_scroll_offset_counts_wrapped_rows() {
        let lines = vec![Line::from("1234567890"), Line::from("tail")];

        let rendered_line_count = count_rendered_conversation_lines(&lines, 4);
        let scroll_offset = build_conversation_scroll_offset(&lines, 4, 2);

        assert_eq!(rendered_line_count, 4);
        assert_eq!(scroll_offset, 2);
    }

    #[test]
    fn conversation_scroll_offset_handles_zero_visible_height() {
        let lines = vec![Line::from("line-1")];

        let scroll_offset = build_conversation_scroll_offset(&lines, 10, 0);

        assert_eq!(scroll_offset, 0);
    }
}
