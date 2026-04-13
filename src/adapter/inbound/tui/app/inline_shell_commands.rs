#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InlineShellCommand {
    Diagnostics,
    Sessions,
    Queue,
    Directions,
    Stop,
    Templates,
    PlanningInit,
    MaxAutoTurns,
    NewDraft,
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InlineShellCommandInput {
    command: InlineShellCommand,
    argument: Option<String>,
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

const COMMAND_LIST_LINE: &str = "Shell commands: :diag  :sessions  :queue  :directions  :stop  :templates  :planning  :turns <n>  :new  :help";
const MAX_AUTO_TURNS_USAGE: &str = "Type `:turns <1-50>` and press Enter to update max auto turns.";

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
        command: InlineShellCommand::Queue,
        primary_name: ":queue",
        aliases: &[":q", ":queue"],
        suggestion_detail: "planning queue",
        buffered_hint: "Press Enter to open the planning queue inspection.",
        execution_status: Some("opened planning queue inspection"),
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Directions,
        primary_name: ":directions",
        aliases: &[":directions"],
        suggestion_detail: "directions maintenance",
        buffered_hint: "Press Enter to review or edit planning directions.",
        execution_status: None,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Stop,
        primary_name: ":stop",
        aliases: &[":stop"],
        suggestion_detail: "stop automation",
        buffered_hint: "Press Enter to stop post-turn automation.",
        execution_status: None,
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
        command: InlineShellCommand::MaxAutoTurns,
        primary_name: ":turns",
        aliases: &[":turn", ":turns", ":auto-turns"],
        suggestion_detail: "set max auto turns",
        buffered_hint: MAX_AUTO_TURNS_USAGE,
        execution_status: None,
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
        command: InlineShellCommand::Help,
        primary_name: ":help",
        aliases: &[":help"],
        suggestion_detail: "command help",
        buffered_hint: "Press Enter to show the available shell commands.",
        execution_status: Some(COMMAND_LIST_LINE),
    },
];

impl InlineShellCommandInput {
    pub(super) fn parse(input: &str) -> Option<Self> {
        let (command_token, argument) = tokenize_inline_command_input(input)?;
        InlineShellCommand::from_alias(&command_token).map(|command| Self { command, argument })
    }

    pub(super) fn command(&self) -> InlineShellCommand {
        self.command
    }

    pub(super) fn argument(&self) -> Option<&str> {
        self.argument.as_deref()
    }

    pub(super) fn buffered_hint(&self) -> String {
        match self.command {
            InlineShellCommand::MaxAutoTurns => match self.argument() {
                Some(value) if is_valid_max_auto_turn_argument(value) => {
                    format!("Press Enter to set max auto turns to {value}.")
                }
                Some(value) => {
                    format!("Press Enter to apply `:turns {value}`. Max auto turns must be 1-50.")
                }
                None => MAX_AUTO_TURNS_USAGE.to_string(),
            },
            _ => self.command.spec().buffered_hint.to_string(),
        }
    }

    pub(super) fn execution_status(&self) -> Option<String> {
        self.command.spec().execution_status.map(str::to_string)
    }
}

impl InlineShellCommand {
    fn spec(self) -> &'static InlineShellCommandSpec {
        INLINE_SHELL_COMMAND_SPECS
            .iter()
            .find(|spec| spec.command == self)
            .expect("inline shell command spec should exist")
    }

    fn from_alias(alias: &str) -> Option<Self> {
        INLINE_SHELL_COMMAND_SPECS
            .iter()
            .find(|spec| spec.aliases.contains(&alias))
            .map(|spec| spec.command)
    }

    pub(super) fn suggestions(input: &str) -> Vec<Self> {
        let Some(prefix) = suggestion_prefix_token(input) else {
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
        suggestion_prefix_token(input)
    }

    pub(super) fn command_name(self) -> &'static str {
        self.spec().primary_name
    }

    #[cfg(test)]
    pub(super) fn suggestion_detail(self) -> &'static str {
        self.spec().suggestion_detail
    }

    #[cfg(test)]
    pub(super) fn command_list_line() -> &'static str {
        COMMAND_LIST_LINE
    }
}

fn tokenize_inline_command_input(input: &str) -> Option<(String, Option<String>)> {
    let trimmed = input.trim();
    if trimmed.is_empty() || !trimmed.starts_with(':') {
        return None;
    }

    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let command_token = parts.next()?.to_ascii_lowercase();
    let argument = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    Some((command_token, argument))
}

fn suggestion_prefix_token(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() || !trimmed.starts_with(':') {
        return None;
    }
    let command_token = trimmed
        .split_whitespace()
        .next()
        .expect("trimmed shell command input should have a first token");
    Some(command_token.to_ascii_lowercase())
}

fn is_valid_max_auto_turn_argument(value: &str) -> bool {
    value
        .trim()
        .parse::<usize>()
        .map(|candidate| (1..=50).contains(&candidate))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{InlineShellCommand, InlineShellCommandInput, MAX_AUTO_TURNS_USAGE};

    #[test]
    fn parse_recognizes_supported_aliases() {
        let cases = [
            (":diag", Some((InlineShellCommand::Diagnostics, None))),
            (
                ":diagnostics",
                Some((InlineShellCommand::Diagnostics, None)),
            ),
            (":DIAG", Some((InlineShellCommand::Diagnostics, None))),
            (":session", Some((InlineShellCommand::Sessions, None))),
            (":sessions", Some((InlineShellCommand::Sessions, None))),
            (":q", Some((InlineShellCommand::Queue, None))),
            (":queue", Some((InlineShellCommand::Queue, None))),
            (":stop", Some((InlineShellCommand::Stop, None))),
            (":template", Some((InlineShellCommand::Templates, None))),
            (":templates", Some((InlineShellCommand::Templates, None))),
            (":planning", Some((InlineShellCommand::PlanningInit, None))),
            (
                ":planning-init",
                Some((InlineShellCommand::PlanningInit, None)),
            ),
            (
                ":turns 5",
                Some((InlineShellCommand::MaxAutoTurns, Some("5"))),
            ),
            (
                ":auto-turns 12",
                Some((InlineShellCommand::MaxAutoTurns, Some("12"))),
            ),
            (":turns", Some((InlineShellCommand::MaxAutoTurns, None))),
            (":new", Some((InlineShellCommand::NewDraft, None))),
            (":help", Some((InlineShellCommand::Help, None))),
            ("  :help  ", Some((InlineShellCommand::Help, None))),
            (":unknown", None),
        ];

        for (input, expected) in cases {
            let parsed = InlineShellCommandInput::parse(input)
                .map(|command| (command.command(), command.argument().map(str::to_string)));
            let expected =
                expected.map(|(command, argument)| (command, argument.map(str::to_string)));
            assert_eq!(parsed, expected, "{input}");
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
                InlineShellCommand::Queue,
                InlineShellCommand::Directions,
                InlineShellCommand::Stop,
                InlineShellCommand::Templates,
                InlineShellCommand::PlanningInit,
                InlineShellCommand::MaxAutoTurns,
                InlineShellCommand::NewDraft,
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
            InlineShellCommand::suggestions(":q"),
            vec![InlineShellCommand::Queue]
        );
        assert_eq!(
            InlineShellCommand::suggestions(":st"),
            vec![InlineShellCommand::Stop]
        );
        assert_eq!(
            InlineShellCommand::suggestions(":t"),
            vec![
                InlineShellCommand::Templates,
                InlineShellCommand::MaxAutoTurns,
            ]
        );
    }

    #[test]
    fn max_auto_turn_command_hint_is_argument_aware() {
        let no_arg = InlineShellCommandInput::parse(":turns").expect("command should parse");
        let valid_arg = InlineShellCommandInput::parse(":turns 7").expect("command should parse");
        let invalid_arg =
            InlineShellCommandInput::parse(":turns 70").expect("command should parse");

        assert_eq!(no_arg.buffered_hint(), MAX_AUTO_TURNS_USAGE);
        assert_eq!(
            valid_arg.buffered_hint(),
            "Press Enter to set max auto turns to 7."
        );
        assert_eq!(
            invalid_arg.buffered_hint(),
            "Press Enter to apply `:turns 70`. Max auto turns must be 1-50."
        );
    }

    #[test]
    fn help_status_reuses_command_list_line() {
        let help = InlineShellCommandInput::parse(":help").expect("help command should parse");

        assert_eq!(
            help.execution_status().as_deref(),
            Some(InlineShellCommand::command_list_line())
        );
    }

    #[test]
    fn execution_status_stays_alias_neutral() {
        let cases = [
            (":diag", Some("opened diagnostics inspection")),
            (":sessions", Some("opened recent sessions inspection")),
            (":queue", Some("opened planning queue inspection")),
            (":stop", None),
            (":templates", Some("opened template inspection")),
            (":planning", Some("opened planning initialization selector")),
            (":turns 5", None),
        ];

        for (input, expected) in cases {
            let command =
                InlineShellCommandInput::parse(input).expect("inline shell command should parse");
            assert_eq!(command.execution_status().as_deref(), expected);
        }
    }
}
