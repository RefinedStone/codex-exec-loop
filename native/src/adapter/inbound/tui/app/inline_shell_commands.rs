#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InlineShellCommand {
    Diagnostics,
    Sessions,
    Templates,
    NewDraft,
    Help,
}

impl InlineShellCommand {
    const COMMAND_LIST_LINE: &str = "Shell commands: :diag  :sessions  :templates  :new  :help";

    pub(super) fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            ":diag" | ":diagnostics" => Some(Self::Diagnostics),
            ":session" | ":sessions" => Some(Self::Sessions),
            ":template" | ":templates" => Some(Self::Templates),
            ":new" => Some(Self::NewDraft),
            ":help" => Some(Self::Help),
            _ => None,
        }
    }

    pub(super) fn command_list_line() -> &'static str {
        Self::COMMAND_LIST_LINE
    }

    pub(super) fn buffered_hint(self) -> &'static str {
        match self {
            Self::Diagnostics => "Press Enter to open the diagnostics overlay.",
            Self::Sessions => "Press Enter to open the recent-sessions overlay.",
            Self::Templates => "Press Enter to open the template overlay.",
            Self::NewDraft => "Press Enter to open a new draft in the shell.",
            Self::Help => "Press Enter to show the available shell commands.",
        }
    }

    pub(super) fn execution_status(self) -> Option<&'static str> {
        match self {
            Self::Diagnostics => Some("opened diagnostics overlay from :diag"),
            Self::Sessions => Some("opened recent sessions overlay from :sessions"),
            Self::Templates => Some("opened template overlay from :templates"),
            Self::NewDraft => None,
            Self::Help => Some(Self::command_list_line()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::InlineShellCommand;

    #[test]
    fn parse_recognizes_supported_aliases() {
        assert_eq!(
            InlineShellCommand::parse(":diag"),
            Some(InlineShellCommand::Diagnostics)
        );
        assert_eq!(
            InlineShellCommand::parse(":sessions"),
            Some(InlineShellCommand::Sessions)
        );
        assert_eq!(
            InlineShellCommand::parse(":templates"),
            Some(InlineShellCommand::Templates)
        );
        assert_eq!(
            InlineShellCommand::parse(":new"),
            Some(InlineShellCommand::NewDraft)
        );
        assert_eq!(
            InlineShellCommand::parse(":help"),
            Some(InlineShellCommand::Help)
        );
    }

    #[test]
    fn help_status_reuses_command_list_line() {
        assert_eq!(
            InlineShellCommand::Help.execution_status(),
            Some(InlineShellCommand::command_list_line())
        );
    }
}
