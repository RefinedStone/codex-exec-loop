use super::super::{AkraTheme, Line, Span, Style};

// modal, directions, planning overlay가 모두 같은 "shortcut + label + detail" row
// grammar를 써야 keyboard focus가 화면마다 다르게 보이지 않는다. caller는 의미
// 상태만 넘기고 marker, style precedence, span 분할은 여기서 고정한다.
pub(crate) fn overlay_option_line(
    shortcut: &str,
    label: &str,
    detail: &str,
    selected: bool,
    disabled: bool,
) -> Line<'static> {
    // disabled는 selected보다 우선한다. controller cursor가 비활성 option 위에
    // 머물러도 사용자가 actionable focus로 오해하지 않도록 subtle style로 누른다.
    let style = match (disabled, selected) {
        (true, _) => AkraTheme::subtle(),
        (false, true) => AkraTheme::selected(),
        (false, false) => Style::default(),
    };
    let marker = if selected {
        AkraTheme::selected_marker()
    } else {
        AkraTheme::idle_marker()
    };

    // marker+shortcut, bold label, detail을 별도 span으로 나눠 label만 scan anchor가
    // 되게 한다. prefix 폭은 marker와 idle marker가 맞춰 주므로 선택 이동 때 row
    // text가 좌우로 밀리지 않는다.
    Line::from(vec![
        Span::styled(format!("{marker}{shortcut}. "), style),
        Span::styled(label.to_string(), style.bold()),
        Span::styled(format!("  {detail}"), style),
    ])
}
