// planner debug block에는 full prompt, model response, generated planning context처럼 긴 원문이 들어올 수 있다.
// 이 helper는 TUI preview만 줄인다. worker input과 기록된 raw debug payload는 다른 경계에 그대로 남아야 한다.
pub(super) fn build_debug_preview_lines(block: &str, max_lines: usize) -> Vec<String> {
    // tail을 선택하려면 전체 line count가 먼저 필요하므로, clone 전에 borrowed line 목록을 만든다.
    let block_lines = block.lines().collect::<Vec<_>>();
    // 짧은 block이나 head + marker + tail을 담기 어려운 작은 cap은 그대로 통과시켜 misleading preview를 만들지 않는다.
    if block_lines.len() <= max_lines || max_lines < 3 {
        return block_lines.into_iter().map(str::to_string).collect();
    }

    // head line은 대체로 debug block의 section title과 초기 instruction/context를 담는다.
    let head_line_count = max_lines / 2;
    // omission marker가 한 visible row를 차지하고, 남은 row는 tail 보존에 쓴다.
    let tail_line_count = max_lines - head_line_count - 1;
    // 정확한 생략 line 수를 보여 줘 display truncation과 prompt truncation을 구분하게 한다.
    let omitted_line_count = block_lines.len() - head_line_count - tail_line_count;

    // truncation path에서는 결과 길이가 정확히 max_lines가 되도록 capacity를 맞춘다.
    let mut lines = Vec::with_capacity(max_lines);
    lines.extend(
        block_lines
            .iter()
            .take(head_line_count)
            .map(|line| (*line).to_string()),
    );

    // marker는 UX contract의 일부다. 줄어든 것은 debug preview뿐이고 worker는 full text를 받았음을 명시한다.
    lines.push(format!(
        "... {omitted_line_count} middle lines omitted in debug preview; worker received full text"
    ));

    // tail line에는 error footer, closing fence, JSON ending, final worker decision처럼 중간보다 유용한 결말 정보가 자주 있다.
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

    // 긴 preview는 tail을 보존해야 한다. debug footer와 structured ending은 중간 내용보다 진단 가치가 높을 때가 많다.
    #[test]
    fn debug_preview_preserves_tail_lines_when_block_is_truncated() {
        // 40개의 numbered line을 쓰면 assertion에서 보존된 head/tail index가 명확하게 드러난다.
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
