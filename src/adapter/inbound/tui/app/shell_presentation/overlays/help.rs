use super::super::{InlineShellCommand, Line};

pub(crate) struct HelpOverlayView {
    pub(crate) header_lines: Vec<Line<'static>>,
    pub(crate) command_lines: Vec<Line<'static>>,
    pub(crate) key_lines: Vec<Line<'static>>,
}

pub(crate) fn build_help_overlay_view() -> HelpOverlayView {
    let entries = InlineShellCommand::help_entries();
    let usage_width = entries
        .iter()
        .map(|entry| entry.usage.len())
        .max()
        .unwrap_or(0)
        .saturating_add(2);

    HelpOverlayView {
        header_lines: vec![
            Line::from("Shell command help"),
            Line::from("Commands are typed directly into the prompt."),
        ],
        command_lines: entries
            .into_iter()
            .map(|entry| {
                Line::from(format!(
                    "{:<width$}{}",
                    entry.usage,
                    entry.detail,
                    width = usage_width
                ))
            })
            .collect(),
        key_lines: vec![Line::from("Esc/Ctrl+C: close")],
    }
}
