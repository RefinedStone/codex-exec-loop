use super::super::{InlineShellCommand, Line};

pub(crate) struct HelpOverlayView {
    pub(crate) header_lines: Vec<Line<'static>>,
    pub(crate) command_lines: Vec<Line<'static>>,
    pub(crate) key_lines: Vec<Line<'static>>,
}

pub(crate) fn build_help_overlay_view() -> HelpOverlayView {
    HelpOverlayView {
        header_lines: vec![
            Line::from("Shell command help"),
            Line::from("Commands are typed directly into the prompt."),
        ],
        command_lines: InlineShellCommand::help_entries()
            .into_iter()
            .map(|entry| Line::from(format!("{:<34} {}", entry.usage, entry.detail)))
            .collect(),
        key_lines: vec![
            Line::from("Enter: run command    Up/Down: move palette selection"),
            Line::from("Esc/Ctrl+C: close"),
        ],
    }
}
