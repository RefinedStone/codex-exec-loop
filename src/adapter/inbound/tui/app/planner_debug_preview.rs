pub(super) fn build_debug_preview_lines(block: &str, max_lines: usize) -> Vec<String> {
    let block_lines = block.lines().collect::<Vec<_>>();
    if block_lines.len() <= max_lines || max_lines < 3 {
        return block_lines.into_iter().map(str::to_string).collect();
    }

    let head_line_count = max_lines / 2;
    let tail_line_count = max_lines - head_line_count - 1;
    let omitted_line_count = block_lines.len() - head_line_count - tail_line_count;

    let mut lines = Vec::with_capacity(max_lines);
    lines.extend(
        block_lines
            .iter()
            .take(head_line_count)
            .map(|line| (*line).to_string()),
    );
    lines.push(format!(
        "... {omitted_line_count} middle lines omitted in debug preview; worker received full text"
    ));
    lines.extend(
        block_lines
            .iter()
            .skip(block_lines.len() - tail_line_count)
            .map(|line| (*line).to_string()),
    );
    lines
}

#[cfg(test)]
mod tests {
    use super::build_debug_preview_lines;

    #[test]
    fn debug_preview_preserves_tail_lines_when_block_is_truncated() {
        let block = (0..40)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n");

        let preview = build_debug_preview_lines(&block, 8);

        assert_eq!(
            preview,
            vec![
                "line 0".to_string(),
                "line 1".to_string(),
                "line 2".to_string(),
                "line 3".to_string(),
                "... 33 middle lines omitted in debug preview; worker received full text"
                    .to_string(),
                "line 37".to_string(),
                "line 38".to_string(),
                "line 39".to_string(),
            ]
        );
    }
}
