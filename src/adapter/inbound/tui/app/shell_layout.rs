// test-only shell layout helper는 production renderer가 쓰는 frame math를 작게 복제한 계약 표면이다.
// snapshot 문자열을 추측하지 않고도 rendering contract test가 scroll과 height behavior를 직접 검증하게 한다.
#[cfg(test)]
use ratatui::text::Line;

// bounded viewport 안에서 최신 transcript row가 보이도록 scroll offset을 계산한다.
#[cfg(test)]
pub(super) fn build_conversation_scroll_offset(
    lines: &[Line<'static>],
    content_width: u16,
    visible_height: u16,
) -> u16 {
    // 그릴 width나 height가 없으면 정렬할 "latest row" window도 없으므로 offset은 0이다.
    if content_width == 0 || visible_height == 0 {
        return 0;
    }

    // ratatui paragraph layout과 맞추기 위해 logical transcript entry가 아니라 wrap 이후 physical row를 센다.
    let rendered_line_count = count_rendered_conversation_lines(lines, content_width);
    let visible_height = visible_height as usize;
    rendered_line_count
        .saturating_sub(visible_height)
        // ratatui scroll offset은 u16이므로 아주 긴 transcript는 presentation boundary에서 clamp한다.
        .min(u16::MAX as usize) as u16
}

// contract test가 renderer를 우회해 transcript panel의 rendered row count를 재구성할 때 쓰는 helper다.
#[cfg(test)]
fn count_rendered_conversation_lines(lines: &[Line<'static>], content_width: u16) -> usize {
    if content_width == 0 {
        return 0;
    }

    lines
        .iter()
        // Line::width는 ratatui span display width를 따르며 renderer가 wrap하는 단위와 같다.
        .map(|line| count_wrapped_rows(line, content_width))
        .sum()
}

// styled logical line 하나가 차지하는 physical terminal row 수로 변환한다.
#[cfg(test)]
fn count_wrapped_rows(line: &Line<'static>, content_width: u16) -> usize {
    if content_width == 0 {
        return 0;
    }

    let line_width = line.width();
    // empty logical line도 transcript paragraph에서는 blank row 하나로 render된다.
    if line_width == 0 {
        return 1;
    }

    line_width.div_ceil(content_width as usize)
}

// 이 test들은 transcript scrolling이 wrap 이후 rendered row를 따른다는 shell contract를 고정한다.
#[cfg(test)]
mod tests {
    use ratatui::text::Line;

    use super::{build_conversation_scroll_offset, count_rendered_conversation_lines};

    // transcript row가 viewport를 넘으면 offset은 오래된 row부터 버린다.
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

    // 긴 logical line 하나도 여러 terminal row를 차지할 수 있으므로 scroll math에 반영되어야 한다.
    #[test]
    fn conversation_scroll_offset_counts_wrapped_rows() {
        let lines = vec![Line::from("1234567890"), Line::from("tail")];

        let rendered_line_count = count_rendered_conversation_lines(&lines, 4);
        let scroll_offset = build_conversation_scroll_offset(&lines, 4, 2);

        assert_eq!(rendered_line_count, 4);
        assert_eq!(scroll_offset, 2);
    }

    // degenerate terminal size에서는 zero-height area가 생길 수 있으므로 scroll math가 underflow하면 안 된다.
    #[test]
    fn conversation_scroll_offset_handles_zero_visible_height() {
        let lines = vec![Line::from("line-1")];

        let scroll_offset = build_conversation_scroll_offset(&lines, 10, 0);

        assert_eq!(scroll_offset, 0);
    }
}
