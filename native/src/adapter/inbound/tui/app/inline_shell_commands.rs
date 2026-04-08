#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InlineShellCommand {
    Diagnostics,
    Sessions,
    Templates,
    NewDraft,
    TranscriptTop,
    TranscriptTail,
    Help,
}

impl InlineShellCommand {
    const COMMAND_LIST_LINE: &str =
        "Shell commands: :diag  :sessions  :templates  :new  :top  :tail  :help";

    pub(super) fn parse(input: &str) -> Option<Self> {
        let trimmed = input.trim();
        if trimmed.eq_ignore_ascii_case(":diag") || trimmed.eq_ignore_ascii_case(":diagnostics") {
            Some(Self::Diagnostics)
        } else if trimmed.eq_ignore_ascii_case(":session")
            || trimmed.eq_ignore_ascii_case(":sessions")
        {
            Some(Self::Sessions)
        } else if trimmed.eq_ignore_ascii_case(":template")
            || trimmed.eq_ignore_ascii_case(":templates")
        {
            Some(Self::Templates)
        } else if trimmed.eq_ignore_ascii_case(":new") {
            Some(Self::NewDraft)
        } else if trimmed.eq_ignore_ascii_case(":top") || trimmed.eq_ignore_ascii_case(":home") {
            Some(Self::TranscriptTop)
        } else if trimmed.eq_ignore_ascii_case(":tail") || trimmed.eq_ignore_ascii_case(":end") {
            Some(Self::TranscriptTail)
        } else if trimmed.eq_ignore_ascii_case(":help") {
            Some(Self::Help)
        } else {
            None
        }
    }

    pub(super) fn command_list_line() -> &'static str {
        Self::COMMAND_LIST_LINE
    }

    pub(super) fn buffered_hint(self) -> &'static str {
        match self {
            Self::Diagnostics => "Press Enter to open the diagnostics inspection.",
            Self::Sessions => "Press Enter to open the recent-sessions inspection.",
            Self::Templates => "Press Enter to open the template inspection.",
            Self::NewDraft => "Press Enter to open a new draft in the shell.",
            Self::TranscriptTop => "Press Enter to jump the transcript viewport to the top.",
            Self::TranscriptTail => "Press Enter to jump the transcript viewport to the tail.",
            Self::Help => "Press Enter to show the available shell commands.",
        }
    }

    pub(super) fn execution_status(self) -> Option<&'static str> {
        match self {
            Self::Diagnostics => Some("opened diagnostics inspection"),
            Self::Sessions => Some("opened recent sessions inspection"),
            Self::Templates => Some("opened template inspection"),
            Self::NewDraft => None,
            Self::TranscriptTop => Some("jumped transcript viewport to top"),
            Self::TranscriptTail => Some("jumped transcript viewport to tail"),
            Self::Help => Some(Self::command_list_line()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::InlineShellCommand;

    #[test]
    fn parse_recognizes_supported_aliases() {
        let cases = [
            (":diag", Some(InlineShellCommand::Diagnostics)),
            (":diagnostics", Some(InlineShellCommand::Diagnostics)),
            (":DIAG", Some(InlineShellCommand::Diagnostics)),
            (":session", Some(InlineShellCommand::Sessions)),
            (":sessions", Some(InlineShellCommand::Sessions)),
            (":template", Some(InlineShellCommand::Templates)),
            (":templates", Some(InlineShellCommand::Templates)),
            (":new", Some(InlineShellCommand::NewDraft)),
            (":top", Some(InlineShellCommand::TranscriptTop)),
            (":home", Some(InlineShellCommand::TranscriptTop)),
            (":tail", Some(InlineShellCommand::TranscriptTail)),
            (":end", Some(InlineShellCommand::TranscriptTail)),
            (":help", Some(InlineShellCommand::Help)),
            ("  :help  ", Some(InlineShellCommand::Help)),
            (":unknown", None),
        ];

        for (input, expected) in cases {
            assert_eq!(InlineShellCommand::parse(input), expected, "{input}");
        }
    }

    #[test]
    fn help_status_reuses_command_list_line() {
        assert_eq!(
            InlineShellCommand::Help.execution_status(),
            Some(InlineShellCommand::command_list_line())
        );
    }

    #[test]
    fn execution_status_stays_alias_neutral() {
        let cases = [
            (
                InlineShellCommand::Diagnostics,
                Some("opened diagnostics inspection"),
            ),
            (
                InlineShellCommand::Sessions,
                Some("opened recent sessions inspection"),
            ),
            (
                InlineShellCommand::Templates,
                Some("opened template inspection"),
            ),
            (
                InlineShellCommand::TranscriptTop,
                Some("jumped transcript viewport to top"),
            ),
            (
                InlineShellCommand::TranscriptTail,
                Some("jumped transcript viewport to tail"),
            ),
        ];

        for (command, expected) in cases {
            assert_eq!(command.execution_status(), expected);
        }
    }
}
