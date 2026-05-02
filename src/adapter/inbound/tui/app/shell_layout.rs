// 학습 주석: 이 layout helper들은 현재 테스트에서 shell rendering 계약을 검증하기 위해만 컴파일됩니다.
// production renderer와 같은 ratatui Line 폭 계산을 사용해 snapshot 테스트가 실제 wrapping 규칙을 따라가게 합니다.
#[cfg(test)]
use ratatui::text::Line;

// 학습 주석: composer/status 높이 상수는 shell module의 실제 layout budget과 같은 값을 공유합니다.
// 테스트 helper가 자체 숫자를 갖지 않게 해 renderer 계약이 바뀌면 테스트 계산도 함께 바뀝니다.
#[cfg(test)]
use super::{
    MAX_COMPOSER_HEIGHT, MAX_SHELL_STATUS_HEIGHT, MIN_COMPOSER_HEIGHT, MIN_SHELL_STATUS_HEIGHT,
};

// 학습 주석: conversation scroll offset은 transcript가 길어졌을 때 viewport를 최신 row 쪽으로
// 맞추기 위한 테스트 계산입니다. visible height와 wrapping width를 함께 받아 실제 보이는 row 수를 기준으로 합니다.
#[cfg(test)]
pub(super) fn build_conversation_scroll_offset(
    // 학습 주석: transcript area에 들어갈 logical line 목록입니다. 각 Line은 terminal width에 따라 여러 row로 감길 수 있습니다.
    lines: &[Line<'static>],
    // 학습 주석: paragraph가 실제로 wrap할 content 폭입니다. 0이면 renderer가 그릴 공간이 없다는 뜻입니다.
    content_width: u16,
    // 학습 주석: transcript viewport가 노출할 row 수입니다. 이 값만큼은 스크롤하지 않고 보존합니다.
    visible_height: u16,
) -> u16 {
    // 학습 주석: 폭이나 높이가 0이면 최신 row로 맞출 기준이 없으므로 offset을 0으로 고정해
    // underflow와 의미 없는 스크롤을 동시에 피합니다.
    if content_width == 0 || visible_height == 0 {
        return 0;
    }

    // 학습 주석: logical line 개수가 아니라 wrap 이후 row 수를 세야, 긴 markdown/code line이
    // 화면 하단을 밀어내는 실제 렌더링 결과와 scroll offset이 일치합니다.
    let rendered_line_count = count_rendered_conversation_lines(lines, content_width);
    // 학습 주석: 계산은 usize로 하되, ratatui scroll offset 타입인 u16으로 반환할 준비를 합니다.
    let visible_height = visible_height as usize;
    rendered_line_count
        // 학습 주석: 전체 row가 viewport보다 작으면 스크롤할 필요가 없으므로 saturating_sub가 0을 반환합니다.
        .saturating_sub(visible_height)
        // 학습 주석: ratatui offset은 u16이므로, 극단적으로 긴 transcript도 타입 범위 안으로 제한합니다.
        .min(u16::MAX as usize) as u16
}

// 학습 주석: 테스트에서 transcript 전체가 몇 physical row를 차지하는지 재현하는 helper입니다.
#[cfg(test)]
fn count_rendered_conversation_lines(lines: &[Line<'static>], content_width: u16) -> usize {
    // 학습 주석: content width가 0이면 어떤 line도 실제 row로 배치할 수 없다고 보고 0을 돌려
    // scroll 계산의 early return과 같은 의미를 유지합니다.
    if content_width == 0 {
        return 0;
    }

    lines
        // 학습 주석: 각 logical Line은 스타일 span을 포함할 수 있으므로 Line::width 기반 계산에 맡깁니다.
        .iter()
        // 학습 주석: 한 line이 폭보다 길면 여러 physical row로 감기므로 row 수로 변환합니다.
        .map(|line| count_wrapped_rows(line, content_width))
        // 학습 주석: transcript viewport가 감당해야 하는 전체 rendered row 수를 합산합니다.
        .sum()
}

// 학습 주석: composer block 높이는 입력 줄 수에 padding/border 여유를 더하되 shell이 허용한 범위로 제한합니다.
#[cfg(test)]
pub(super) fn build_input_block_height(lines: &[Line<'_>]) -> u16 {
    block_height_for_lines(lines, MIN_COMPOSER_HEIGHT, MAX_COMPOSER_HEIGHT)
}

// 학습 주석: footer/status block도 composer와 같은 높이 산식을 쓰지만, 별도 min/max budget을 적용합니다.
#[cfg(test)]
pub(super) fn build_shell_footer_height(lines: &[Line<'_>]) -> u16 {
    block_height_for_lines(lines, MIN_SHELL_STATUS_HEIGHT, MAX_SHELL_STATUS_HEIGHT)
}

// 학습 주석: block height 계산은 "내용 줄 + 위아래 chrome 2줄"이라는 shell layout 가정을 테스트에서 재사용합니다.
#[cfg(test)]
pub(super) fn block_height_for_lines(lines: &[Line<'_>], min_height: u16, max_height: u16) -> u16 {
    lines
        // 학습 주석: content line 수가 기본 높이 산식의 출발점입니다.
        .len()
        // 학습 주석: shell chrome 또는 padding이 차지하는 2줄을 더합니다. saturating_add로 큰 입력에도 안전합니다.
        .saturating_add(2)
        // 학습 주석: 최소 높이는 빈 상태의 안정된 박스를 보장하고, 최대 높이는 transcript 영역을 보호합니다.
        .clamp(min_height as usize, max_height as usize) as u16
}

// 학습 주석: 한 ratatui Line이 content width에서 차지할 physical row 수를 계산합니다.
#[cfg(test)]
fn count_wrapped_rows(line: &Line<'static>, content_width: u16) -> usize {
    // 학습 주석: width 0은 wrap 기준 자체가 없으므로 caller와 같은 방어 규칙으로 0 row를 반환합니다.
    if content_width == 0 {
        return 0;
    }

    // 학습 주석: Line::width는 span/style을 제외한 표시 폭을 계산하므로 테스트가 terminal row 소비량을
    // 문자열 byte 길이보다 정확하게 재현합니다.
    let line_width = line.width();
    // 학습 주석: 빈 Line도 renderer에서는 한 row의 빈 줄로 남기 때문에 0이 아니라 1 row로 계산합니다.
    if line_width == 0 {
        return 1;
    }

    // 학습 주석: div_ceil은 부분 row를 올림 처리해, 폭 4에서 길이 10인 line이 3 rows를 차지하게 합니다.
    line_width.div_ceil(content_width as usize)
}

// 학습 주석: 이 테스트 모듈은 shell layout helper의 핵심 계약인 "최신 transcript row로 스크롤"과
// "wrapping row 수 반영"을 작은 입력으로 고정합니다.
#[cfg(test)]
mod tests {
    // 학습 주석: 테스트도 production helper와 같은 ratatui Line 타입을 써서 width 계산 차이를 만들지 않습니다.
    use ratatui::text::Line;

    // 학습 주석: 공개 helper와 내부 row counter를 함께 검증해 offset 결과가 어떤 row 계산에서 나온 것인지 드러냅니다.
    use super::{build_conversation_scroll_offset, count_rendered_conversation_lines};

    // 학습 주석: transcript가 viewport보다 길 때 offset이 앞쪽 row를 건너뛰고 최신 row들을 보여 주는지 확인합니다.
    #[test]
    fn conversation_scroll_offset_moves_to_latest_rows() {
        // 학습 주석: 각 logical line이 한 row에 들어가는 단순 transcript를 만들어 기본 offset 산식을 검증합니다.
        let lines = vec![
            Line::from("line-1"),
            Line::from("line-2"),
            Line::from("line-3"),
            Line::from("line-4"),
        ];

        // 학습 주석: 네 row 중 두 row만 보일 수 있으므로 앞의 두 row를 스크롤로 밀어내야 합니다.
        let scroll_offset = build_conversation_scroll_offset(&lines, 20, 2);

        assert_eq!(scroll_offset, 2);
    }

    // 학습 주석: 긴 line wrapping이 scroll offset에 반영되는지 확인해, logical line 수만 세는 회귀를 막습니다.
    #[test]
    fn conversation_scroll_offset_counts_wrapped_rows() {
        // 학습 주석: 폭 4에서 10글자 line은 3 rows, tail은 1 row라 전체 rendered row는 4입니다.
        let lines = vec![Line::from("1234567890"), Line::from("tail")];

        // 학습 주석: 내부 row counter의 결과를 직접 확인해 scroll offset assertion의 근거를 분리합니다.
        let rendered_line_count = count_rendered_conversation_lines(&lines, 4);
        // 학습 주석: rendered 4 rows에서 viewport 2 rows를 빼면 최신 tail을 보이기 위한 offset은 2입니다.
        let scroll_offset = build_conversation_scroll_offset(&lines, 4, 2);

        assert_eq!(rendered_line_count, 4);
        assert_eq!(scroll_offset, 2);
    }

    // 학습 주석: zero-height viewport는 실제 표시 공간이 없으므로 offset을 만들지 않는 방어 계약을 고정합니다.
    #[test]
    fn conversation_scroll_offset_handles_zero_visible_height() {
        // 학습 주석: line이 있어도 visible height가 0이면 scroll 기준이 없다는 상황을 구성합니다.
        let lines = vec![Line::from("line-1")];

        // 학습 주석: helper는 이 경우 overflow나 음수 개념 없이 0 offset을 반환해야 합니다.
        let scroll_offset = build_conversation_scroll_offset(&lines, 10, 0);

        assert_eq!(scroll_offset, 0);
    }
}
