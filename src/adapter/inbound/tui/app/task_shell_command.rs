#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParsedTaskShellCommand<'a> {
    OpenPromptEditor,
    PreviewPrompt { prompt: &'a str },
}

pub(super) fn parse_task_shell_argument(argument: Option<&str>) -> ParsedTaskShellCommand<'_> {
    match argument.map(str::trim).filter(|value| !value.is_empty()) {
        Some(prompt) => ParsedTaskShellCommand::PreviewPrompt { prompt },
        None => ParsedTaskShellCommand::OpenPromptEditor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_shell_argument_maps_to_prompt_intake_command() {
        assert_eq!(
            parse_task_shell_argument(None),
            ParsedTaskShellCommand::OpenPromptEditor
        );
        assert_eq!(
            parse_task_shell_argument(Some("  ")),
            ParsedTaskShellCommand::OpenPromptEditor
        );
        assert_eq!(
            parse_task_shell_argument(Some("  add a release checklist  ")),
            ParsedTaskShellCommand::PreviewPrompt {
                prompt: "add a release checklist"
            }
        );
    }
}
