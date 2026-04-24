use super::super::{AkraTheme, Line, Span, Style};

pub(crate) fn overlay_option_line(
    shortcut: &str,
    label: &str,
    detail: &str,
    selected: bool,
    disabled: bool,
) -> Line<'static> {
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

    Line::from(vec![
        Span::styled(format!("{marker}{shortcut}. "), style),
        Span::styled(label.to_string(), style.bold()),
        Span::styled(format!("  {detail}"), style),
    ])
}
