#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InlineShellCommand {
    Diagnostics,
    Sessions,
    Templates,
    PlanningInit,
    NewDraft,
    TranscriptTopLegacy,
    TranscriptTailLegacy,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InlineShellCommandSpec {
    command: InlineShellCommand,
    primary_name: &'static str,
    aliases: &'static [&'static str],
    suggestion_detail: &'static str,
    buffered_hint: &'static str,
    execution_status: Option<&'static str>,
}

const COMMAND_LIST_LINE: &str =
    "Shell commands: :diag  :sessions  :templates  :planning  :new  :help";

const INLINE_SHELL_COMMAND_SPECS: &[InlineShellCommandSpec] = &[
    InlineShellCommandSpec {
        command: InlineShellCommand::Diagnostics,
        primary_name: ":diag",
        aliases: &[":diag", ":diagnostics"],
        suggestion_detail: "diagnostics",
        buffered_hint: "Press Enter to open the diagnostics inspection.",
        execution_status: Some("opened diagnostics inspection"),
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Sessions,
        primary_name: ":sessions",
        aliases: &[":session", ":sessions"],
        suggestion_detail: "recent sessions",
        buffered_hint: "Press Enter to open the recent-sessions inspection.",
        execution_status: Some("opened recent sessions inspection"),
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Templates,
        primary_name: ":templates",
        aliases: &[":template", ":templates"],
        suggestion_detail: "template inspection",
        buffered_hint: "Press Enter to open the template inspection.",
        execution_status: Some("opened template inspection"),
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::PlanningInit,
        primary_name: ":planning",
        aliases: &[":planning", ":planning-init"],
        suggestion_detail: "planning mode",
        buffered_hint: "Press Enter to open the planning mode selector.",
        execution_status: Some("opened planning initialization selector"),
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::NewDraft,
        primary_name: ":new",
        aliases: &[":new"],
        suggestion_detail: "new draft",
        buffered_hint: "Press Enter to open a new draft in the shell.",
        execution_status: None,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::TranscriptTopLegacy,
        primary_name: ":top",
        aliases: &[":top", ":home"],
        suggestion_detail: "legacy jump top",
        buffered_hint: "Press Enter to see where transcript jump controls moved.",
        execution_status: Some(
            "use host terminal scroll in inline mode; alternate-screen keeps PageUp/PageDown/Home/End",
        ),
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::TranscriptTailLegacy,
        primary_name: ":tail",
        aliases: &[":tail", ":end"],
        suggestion_detail: "legacy jump tail",
        buffered_hint: "Press Enter to see where transcript jump controls moved.",
        execution_status: Some(
            "use host terminal scroll in inline mode; alternate-screen keeps PageUp/PageDown/Home/End",
        ),
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Help,
        primary_name: ":help",
        aliases: &[":help"],
        suggestion_detail: "command help",
        buffered_hint: "Press Enter to show the available shell commands.",
        execution_status: Some(COMMAND_LIST_LINE),
    },
];

impl InlineShellCommand {
    fn spec(self) -> &'static InlineShellCommandSpec {
        INLINE_SHELL_COMMAND_SPECS
            .iter()
            .find(|spec| spec.command == self)
            .expect("inline shell command spec should exist")
    }

    fn normalized_candidate(input: &str) -> Option<String> {
        let trimmed = input.trim();
        if trimmed.is_empty() || !trimmed.starts_with(':') {
            return None;
        }
        if trimmed.chars().any(|character| character.is_whitespace()) {
            return None;
        }
        Some(trimmed.to_ascii_lowercase())
    }

    pub(super) fn parse(input: &str) -> Option<Self> {
        let normalized = Self::normalized_candidate(input)?;
        INLINE_SHELL_COMMAND_SPECS
            .iter()
            .find(|spec| spec.aliases.iter().any(|alias| *alias == normalized))
            .map(|spec| spec.command)
    }

    pub(super) fn suggestions(input: &str) -> Vec<Self> {
        let Some(prefix) = Self::normalized_candidate(input) else {
            return Vec::new();
        };

        INLINE_SHELL_COMMAND_SPECS
            .iter()
            .filter(|spec| {
                prefix == ":"
                    || spec
                        .aliases
                        .iter()
                        .any(|alias| alias.starts_with(prefix.as_str()))
            })
            .map(|spec| spec.command)
            .collect()
    }

    pub(super) fn suggestion_prefix(input: &str) -> Option<String> {
        Self::normalized_candidate(input)
    }

    pub(super) fn command_name(self) -> &'static str {
        self.spec().primary_name
    }

    pub(super) fn suggestion_detail(self) -> &'static str {
        self.spec().suggestion_detail
    }

    pub(super) fn command_list_line() -> &'static str {
        COMMAND_LIST_LINE
    }

    pub(super) fn buffered_hint(self) -> &'static str {
        self.spec().buffered_hint
    }

    pub(super) fn execution_status(self) -> Option<&'static str> {
        self.spec().execution_status
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
            (":planning", Some(InlineShellCommand::PlanningInit)),
            (":planning-init", Some(InlineShellCommand::PlanningInit)),
            (":new", Some(InlineShellCommand::NewDraft)),
            (":top", Some(InlineShellCommand::TranscriptTopLegacy)),
            (":home", Some(InlineShellCommand::TranscriptTopLegacy)),
            (":tail", Some(InlineShellCommand::TranscriptTailLegacy)),
            (":end", Some(InlineShellCommand::TranscriptTailLegacy)),
            (":help", Some(InlineShellCommand::Help)),
            ("  :help  ", Some(InlineShellCommand::Help)),
            (":unknown", None),
        ];

        for (input, expected) in cases {
            assert_eq!(InlineShellCommand::parse(input), expected, "{input}");
        }
    }

    #[test]
    fn suggestions_show_all_commands_for_colon_only() {
        let suggestions = InlineShellCommand::suggestions(":");

        assert_eq!(
            suggestions,
            vec![
                InlineShellCommand::Diagnostics,
                InlineShellCommand::Sessions,
                InlineShellCommand::Templates,
                InlineShellCommand::PlanningInit,
                InlineShellCommand::NewDraft,
                InlineShellCommand::TranscriptTopLegacy,
                InlineShellCommand::TranscriptTailLegacy,
                InlineShellCommand::Help,
            ]
        );
    }

    #[test]
    fn suggestions_filter_by_prefix() {
        assert_eq!(
            InlineShellCommand::suggestions(":p"),
            vec![InlineShellCommand::PlanningInit]
        );
        assert_eq!(
            InlineShellCommand::suggestions(":t"),
            vec![
                InlineShellCommand::Templates,
                InlineShellCommand::TranscriptTopLegacy,
                InlineShellCommand::TranscriptTailLegacy,
            ]
        );
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
                InlineShellCommand::PlanningInit,
                Some("opened planning initialization selector"),
            ),
            (
                InlineShellCommand::TranscriptTopLegacy,
                Some(
                    "use host terminal scroll in inline mode; alternate-screen keeps PageUp/PageDown/Home/End",
                ),
            ),
            (
                InlineShellCommand::TranscriptTailLegacy,
                Some(
                    "use host terminal scroll in inline mode; alternate-screen keeps PageUp/PageDown/Home/End",
                ),
            ),
        ];

        for (command, expected) in cases {
            assert_eq!(command.execution_status(), expected);
        }
    }
}
