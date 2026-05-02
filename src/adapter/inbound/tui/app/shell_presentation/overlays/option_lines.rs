// 학습 주석: option row는 marker, shortcut, bold label, detail을 한 Line 안의 여러 Span으로 조립합니다.
// theme/style type을 가져와 selection overlay와 directions overlay가 같은 row grammar를 쓰게 합니다.
use super::super::{AkraTheme, Line, Span, Style};

// 학습 주석: 이 helper는 modal overlay에서 반복되는 "단축키 + 선택지 이름 + 설명" 한 줄을 만듭니다.
// caller는 의미 값(shortcut/label/detail/selected/disabled)만 넘기고, marker와 styling 규칙은 여기서 통일합니다.
pub(crate) fn overlay_option_line(
    // 학습 주석: shortcut은 사용자가 누를 key 또는 menu index입니다. marker 뒤에 붙어 조작 가능한 row임을 보여 줍니다.
    shortcut: &str,
    // 학습 주석: label은 선택지의 짧은 이름입니다. row 안에서 bold 처리되어 scanning anchor가 됩니다.
    label: &str,
    // 학습 주석: detail은 선택 결과나 tradeoff를 설명하는 보조 문구입니다.
    detail: &str,
    // 학습 주석: selected가 true면 현재 cursor/highlight row로 표시합니다.
    selected: bool,
    // 학습 주석: disabled가 true면 선택 불가능한 row입니다. disabled는 selected보다 우선해 subtle style로 보입니다.
    disabled: bool,
) -> Line<'static> {
    // 학습 주석: style precedence는 disabled > selected > default입니다. 비활성 row가 선택 cursor와 겹쳐도
    // actionable처럼 보이지 않게 disabled style을 먼저 적용합니다.
    let style = match (disabled, selected) {
        // 학습 주석: disabled row는 subtle style로 눌러 보여 action 가능성이 낮음을 전달합니다.
        (true, _) => AkraTheme::subtle(),
        // 학습 주석: selected row는 selection style로 현재 keyboard focus를 표시합니다.
        (false, true) => AkraTheme::selected(),
        // 학습 주석: 기본 row는 surrounding overlay text와 같은 default style을 사용합니다.
        (false, false) => Style::default(),
    };
    // 학습 주석: marker는 row 앞의 시각적 cursor입니다. style과 별도로 marker를 두어 monochrome terminal에서도
    // selected row를 구분할 수 있습니다.
    let marker = if selected {
        // 학습 주석: selected marker는 현재 row에 keyboard focus가 있음을 나타냅니다.
        AkraTheme::selected_marker()
    } else {
        // 학습 주석: idle marker는 선택되지 않은 row의 alignment를 selected row와 맞춥니다.
        AkraTheme::idle_marker()
    };

    // 학습 주석: 하나의 Line 안에서 shortcut, label, detail을 Span으로 나눠 label만 bold하고 나머지는 같은
    // state style을 적용합니다. 이 구조가 overlay list row의 공통 rendering contract입니다.
    Line::from(vec![
        // 학습 주석: 첫 span은 marker와 shortcut을 포함해 row의 조작 key를 가장 앞에 둡니다.
        Span::styled(format!("{marker}{shortcut}. "), style),
        // 학습 주석: 두 번째 span은 option label입니다. bold로 처리해 detail 문구와 분리합니다.
        Span::styled(label.to_string(), style.bold()),
        // 학습 주석: 세 번째 span은 보조 설명입니다. 앞에 두 칸을 넣어 label과 시각적 간격을 유지합니다.
        Span::styled(format!("  {detail}"), style),
    ])
}
