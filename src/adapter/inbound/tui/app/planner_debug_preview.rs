// Planner debug blocks can contain full prompts, responses, or generated planning context.
// This helper only trims the TUI preview; the worker input and recorded raw debug payload stay intact elsewhere.
pub(super) fn build_debug_preview_lines(block: &str, max_lines: usize) -> Vec<String> {
    // We need the total line count before selecting the tail, so collect borrowed lines before cloning.
    let block_lines = block.lines().collect::<Vec<_>>();
    // Short blocks, or caps too small to fit head + marker + tail, pass through unchanged to avoid misleading previews.
    if block_lines.len() <= max_lines || max_lines < 3 {
        return block_lines.into_iter().map(str::to_string).collect();
    }

    // Head lines usually contain the section title and initial instruction/context for the debug block.
    let head_line_count = max_lines / 2;
    // Reserve one visible row for the omission marker; the remaining rows preserve the tail.
    let tail_line_count = max_lines - head_line_count - 1;
    // The exact count makes the preview honest and distinguishes display truncation from prompt truncation.
    let omitted_line_count = block_lines.len() - head_line_count - tail_line_count;

    // The result length is exactly max_lines on the truncation path.
    let mut lines = Vec::with_capacity(max_lines);
    lines.extend(
        block_lines
            .iter()
            .take(head_line_count)
            .map(|line| (*line).to_string()),
    );

    // The marker is part of the UX contract: only the debug preview is shortened.
    lines.push(format!(
        "... {omitted_line_count} middle lines omitted in debug preview; worker received full text"
    ));

    // Tail lines often contain error footers, closing fences, JSON endings, or final worker decisions.
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

    // Long previews must keep the tail because debug footers and structured endings are often more useful than the middle.
    #[test]
    fn debug_preview_preserves_tail_lines_when_block_is_truncated() {
        // Forty numbered lines make the retained head/tail indexes obvious in the assertion.
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
