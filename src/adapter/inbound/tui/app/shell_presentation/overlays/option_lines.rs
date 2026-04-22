use super::super::{Color, Line, Span, Style};
use ratatui::prelude::Stylize;

pub(crate) fn overlay_option_line(
    shortcut: &str,
    label: &str,
    detail: &str,
    selected: bool,
    disabled: bool,
) -> Line<'static> {
    let style = match (disabled, selected) {
        (true, _) => Style::default().fg(Color::DarkGray),
        (false, true) => Style::default().fg(Color::Black).bg(Color::Cyan),
        (false, false) => Style::default().fg(Color::White),
    };
    let marker = if selected { ">>" } else { "  " };

    Line::from(vec![
        Span::styled(format!("{marker} {shortcut}. "), style),
        Span::styled(label.to_string(), style.bold()),
        Span::styled(format!("  {detail}"), style),
    ])
}
