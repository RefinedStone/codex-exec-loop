// 학습 주석: build_debug_preview_lines는 긴 planner/debug block을 TUI에 보여 줄 preview 길이로 줄입니다.
// worker에게 전달되는 원문은 줄이지 않고, 화면에만 head/tail 중심 요약을 보여 주는 presentation helper입니다.
pub(super) fn build_debug_preview_lines(block: &str, max_lines: usize) -> Vec<String> {
    // 학습 주석: str::lines는 newline을 기준으로 preview 단위를 나눕니다. Vec로 모아야 전체 줄 수를 먼저 계산하고
    // 뒤쪽 tail 영역을 다시 선택할 수 있습니다.
    let block_lines = block.lines().collect::<Vec<_>>();
    // 학습 주석: 이미 max_lines 이하라면 정보를 숨길 필요가 없습니다. max_lines가 3보다 작을 때도
    // "head + omitted marker + tail" 구조를 안정적으로 만들 수 없으므로 원문 줄을 그대로 반환합니다.
    if block_lines.len() <= max_lines || max_lines < 3 {
        // 학습 주석: &str slice를 owned String으로 바꿔 caller가 block lifetime과 독립적으로 preview를 보관하게 합니다.
        return block_lines.into_iter().map(str::to_string).collect();
    }

    // 학습 주석: 앞부분은 block의 context와 제목/초기 지시문을 보여 주기 위해 절반가량 보존합니다.
    let head_line_count = max_lines / 2;
    // 학습 주석: 한 줄은 omission marker가 차지하므로 tail은 남은 줄 수에서 marker 한 줄을 뺀 값입니다.
    let tail_line_count = max_lines - head_line_count - 1;
    // 학습 주석: 생략 줄 수를 명시해 사용자가 preview가 잘린 표시인지, 실제 worker input이 줄었는지 혼동하지 않게 합니다.
    let omitted_line_count = block_lines.len() - head_line_count - tail_line_count;

    // 학습 주석: 결과 Vec의 최대 길이는 정확히 max_lines입니다. capacity를 맞춰 두면 push/extend 중 재할당을 줄입니다.
    let mut lines = Vec::with_capacity(max_lines);
    // 학습 주석: 먼저 head 영역을 복사합니다. turn_submission_runtime은 이 결과를 log/status panel에 바로 뿌립니다.
    lines.extend(
        block_lines
            // 학습 주석: 전체 block slice에서 앞쪽 줄을 순서대로 봅니다.
            .iter()
            // 학습 주석: 계산된 head_line_count만 preview 앞부분에 유지합니다.
            .take(head_line_count)
            // 학습 주석: Vec<String> 반환 계약에 맞춰 각 &str을 소유 문자열로 복사합니다.
            .map(|line| (*line).to_string()),
    );
    // 학습 주석: 가운데 marker는 잘린 줄 수와 worker가 full text를 받았다는 사실을 함께 말합니다.
    // 이 문구가 없으면 debug preview를 실제 prompt 축약으로 오해할 수 있습니다.
    lines.push(format!(
        "... {omitted_line_count} middle lines omitted in debug preview; worker received full text"
    ));
    // 학습 주석: 마지막 줄들은 error footer, task mutation JSON, closing fence처럼 중요한 단서가 있을 수 있어 보존합니다.
    lines.extend(
        block_lines
            // 학습 주석: 같은 전체 block slice를 다시 순회해 tail 시작점까지 건너뜁니다.
            .iter()
            // 학습 주석: len - tail_line_count가 preview에 들어갈 마지막 구간의 첫 index입니다.
            .skip(block_lines.len() - tail_line_count)
            // 학습 주석: tail 줄도 caller가 소유할 수 있도록 String으로 변환합니다.
            .map(|line| (*line).to_string()),
    );
    lines
}

// 학습 주석: 아래 test module은 production build에는 들어가지 않고, preview truncation contract만 검증합니다.
#[cfg(test)]
// 학습 주석: 같은 파일 안의 private helper를 대상으로 하므로 별도 integration test보다 module-local unit test가 충분합니다.
mod tests {
    // 학습 주석: parent module의 helper를 가져와 test가 presentation truncation만 직접 호출하게 합니다.
    use super::build_debug_preview_lines;

    // 학습 주석: 이 테스트는 긴 block을 8줄 preview로 줄일 때 앞 4줄, marker 1줄, 뒤 3줄이 남는 계약을 고정합니다.
    #[test]
    // 학습 주석: 특히 tail lines 보존은 debug footer나 JSON 끝부분을 잃지 않기 위한 사용자 경험 규칙입니다.
    fn debug_preview_preserves_tail_lines_when_block_is_truncated() {
        // 학습 주석: 40줄짜리 synthetic block을 만들면 max_lines=8에서 반드시 truncation path를 탑니다.
        let block = (0..40)
            // 학습 주석: index를 line text에 넣어 어떤 줄이 head/tail로 남았는지 assertion에서 직접 확인합니다.
            .map(|index| format!("line {index}"))
            // 학습 주석: join 전에 Vec로 모아 owned String 조각들을 newline block으로 합칠 준비를 합니다.
            .collect::<Vec<_>>()
            // 학습 주석: 실제 planner/debug block처럼 newline-separated text로 만듭니다.
            .join("\n");

        // 학습 주석: 8줄 제한은 head 4줄, omission marker 1줄, tail 3줄로 나뉩니다.
        let preview = build_debug_preview_lines(&block, 8);

        assert_eq!(
            preview,
            vec![
                "line 0".to_string(),
                "line 1".to_string(),
                "line 2".to_string(),
                "line 3".to_string(),
                "... 33 middle lines omitted in debug preview; worker received full text"
                    // 학습 주석: marker도 Vec<String>의 한 원소라 expected value에서 String으로 맞춥니다.
                    .to_string(),
                "line 37".to_string(),
                "line 38".to_string(),
                "line 39".to_string(),
            ]
        );
    }
}
