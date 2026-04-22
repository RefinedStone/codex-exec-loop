use super::AutoFollowState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InlineShellCommand {
    Diagnostics,
    Parallel,
    Sessions,
    Queue,
    Directions,
    Stop,
    Automation,
    Doctor,
    Init,
    PlanningInit,
    Reset,
    MaxAutoTurns,
    NewDraft,
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InlineShellCommandInput {
    command: InlineShellCommand,
    argument: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct InlineShellCommandPaletteState {
    active: bool,
    selected_index: usize,
    suggestions: Vec<InlineShellCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InlineShellCommandSpec {
    command: InlineShellCommand,
    primary_name: &'static str,
    aliases: &'static [&'static str],
    suggestion_detail: &'static str,
    buffered_hint: &'static str,
    execution_status: Option<&'static str>,
    requires_argument: bool,
}

const COMMAND_LIST_LINE: &str = "Shell commands: :diag  :parallel [on|off]  :sessions  :queue  :directions [apply]  :stop  :auto  :planning [on|off|doctor]  :doctor  :init  :reset <queue|directions|all>  :turns <n|infinite>  :new  :help";
const MAX_AUTO_TURNS_USAGE: &str =
    "Type `:turns <n|infinite>` and press Enter to update max auto turns.";
const RESET_USAGE: &str =
    "Type `:reset <queue|directions|all>` and press Enter to reset planning state.";

const INLINE_SHELL_COMMAND_SPECS: &[InlineShellCommandSpec] = &[
    InlineShellCommandSpec {
        command: InlineShellCommand::Diagnostics,
        primary_name: ":diag",
        aliases: &[":diag", ":diagnostics"],
        suggestion_detail: "diagnostics",
        buffered_hint: "Press Enter to open the diagnostics inspection.",
        execution_status: Some("opened diagnostics inspection"),
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Parallel,
        primary_name: ":parallel",
        aliases: &[":parallel"],
        suggestion_detail: "parallel mode",
        buffered_hint: "Press Enter to inspect parallel mode readiness.",
        execution_status: None,
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Sessions,
        primary_name: ":sessions",
        aliases: &[":session", ":sessions"],
        suggestion_detail: "recent sessions",
        buffered_hint: "Press Enter to open the recent-sessions inspection.",
        execution_status: Some("opened recent sessions inspection"),
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Queue,
        primary_name: ":queue",
        aliases: &[":q", ":queue"],
        suggestion_detail: "planning queue",
        buffered_hint: "Press Enter to open the planning queue inspection.",
        execution_status: Some("opened planning queue inspection"),
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Directions,
        primary_name: ":directions",
        aliases: &[":directions"],
        suggestion_detail: "directions maintenance",
        buffered_hint: "Press Enter to review or edit planning directions.",
        execution_status: None,
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Stop,
        primary_name: ":stop",
        aliases: &[":stop"],
        suggestion_detail: "stop automation",
        buffered_hint: "Press Enter to stop post-turn automation.",
        execution_status: None,
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Automation,
        primary_name: ":auto",
        aliases: &[":auto", ":automation"],
        suggestion_detail: "automation controls",
        buffered_hint: "Press Enter to open the automation controls.",
        execution_status: Some("opened automation controls"),
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Doctor,
        primary_name: ":doctor",
        aliases: &[":doctor"],
        suggestion_detail: "planning health",
        buffered_hint: "Press Enter to inspect planning health.",
        execution_status: None,
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Init,
        primary_name: ":init",
        aliases: &[":init"],
        suggestion_detail: "planning scaffold",
        buffered_hint: "Press Enter to stage the default planning scaffold.",
        execution_status: None,
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::PlanningInit,
        primary_name: ":planning",
        aliases: &[":planning", ":planning-init"],
        suggestion_detail: "planning control center",
        buffered_hint: "Press Enter to open the planning control center.",
        execution_status: None,
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Reset,
        primary_name: ":reset",
        aliases: &[":reset"],
        suggestion_detail: "planning reset",
        buffered_hint: RESET_USAGE,
        execution_status: None,
        requires_argument: true,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::MaxAutoTurns,
        primary_name: ":turns",
        aliases: &[":turn", ":turns", ":auto-turns"],
        suggestion_detail: "set max auto turns",
        buffered_hint: MAX_AUTO_TURNS_USAGE,
        execution_status: None,
        requires_argument: true,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::NewDraft,
        primary_name: ":new",
        aliases: &[":new"],
        suggestion_detail: "new draft",
        buffered_hint: "Press Enter to open a new draft in the shell.",
        execution_status: None,
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Help,
        primary_name: ":help",
        aliases: &[":help"],
        suggestion_detail: "command help",
        buffered_hint: "Press Enter to show the available shell commands.",
        execution_status: Some(COMMAND_LIST_LINE),
        requires_argument: false,
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
            InlineShellCommand::Parallel => match self.argument() {
                Some(value) if value.eq_ignore_ascii_case("off") => {
                    "Press Enter to turn parallel mode off.".to_string()
                }
                Some(value) if value.eq_ignore_ascii_case("on") => {
                    "Press Enter to inspect readiness and enter parallel mode when allowed."
                        .to_string()
                }
                Some(value) => format!(
                    "Press Enter to apply `:parallel {value}`. Supported arguments: on, off."
                ),
                None => self.command.spec().buffered_hint.to_string(),
            },
            InlineShellCommand::PlanningInit => match self.argument() {
                Some(value) if value.eq_ignore_ascii_case("off") => {
                    "Press Enter to turn Plan off.".to_string()
                }
                Some(value) if value.eq_ignore_ascii_case("on") => {
                    "Press Enter to turn Plan on.".to_string()
                }
                Some(value) if value.eq_ignore_ascii_case("doctor") => {
                    "Press Enter to inspect planning health.".to_string()
                }
                Some(value) => format!(
                    "Press Enter to apply `:planning {value}`. Supported arguments: on, off, doctor."
                ),
                None => self.command.spec().buffered_hint.to_string(),
            },
            InlineShellCommand::Directions => match self.argument() {
                Some(value) if value.eq_ignore_ascii_case("apply") => {
                    "Press Enter to import tracked directions into active planning."
                        .to_string()
                }
                Some(value) => format!(
                    "Press Enter to apply `:directions {value}`. Supported arguments: apply."
                ),
                None => self.command.spec().buffered_hint.to_string(),
            },
            InlineShellCommand::Reset => match parse_reset_argument(self.argument()) {
                ResetArgument::None => RESET_USAGE.to_string(),
                ResetArgument::Queue { .. } => {
                    "Press Enter to reset queue-side planning state.".to_string()
                }
                ResetArgument::Directions { confirmed: true } => {
                    "Press Enter to confirm the directions reset.".to_string()
                }
                ResetArgument::Directions { confirmed: false } => {
                    "Review `:reset directions confirm` before rewriting directions-side planning files.".to_string()
                }
                ResetArgument::All { confirmed: true } => {
                    "Press Enter to confirm the full planning reset.".to_string()
                }
                ResetArgument::All { confirmed: false } => {
                    "Review `:reset all confirm` before replacing the full planning scaffold.".to_string()
                }
                ResetArgument::Invalid(value) => format!(
                    "Press Enter to apply `:reset {value}`. Supported arguments: queue, directions, all."
                ),
            },
            InlineShellCommand::MaxAutoTurns => match self.argument() {
                Some(value) if is_valid_max_auto_turn_argument(value) => {
                    format!("Press Enter to set max auto turns to {value}.")
                }
                Some(value) => format!(
                    "Press Enter to apply `:turns {value}`. Max auto turns must be a whole number greater than 0 or `infinite`."
                ),
                None => MAX_AUTO_TURNS_USAGE.to_string(),
            },
            _ => self.command.spec().buffered_hint.to_string(),
        }
    }

    pub(super) fn execution_status(&self) -> Option<String> {
        self.command.spec().execution_status.map(str::to_string)
    }

    pub(super) fn from_command(command: InlineShellCommand) -> Self {
        Self {
            command,
            argument: None,
        }
    }
}

impl InlineShellCommandPaletteState {
    pub(super) fn sync_to_input(
        &mut self,
        input: &str,
        preferred_selection: Option<InlineShellCommand>,
    ) {
        let Some(_prefix) = suggestion_prefix_token(input) else {
            *self = Self::default();
            return;
        };

        let suggestions = InlineShellCommand::suggestions(input);
        let selected_index = preferred_selection
            .and_then(|command| {
                suggestions
                    .iter()
                    .position(|candidate| *candidate == command)
            })
            .unwrap_or(0);
        self.active = true;
        self.selected_index = selected_index.min(suggestions.len().saturating_sub(1));
        self.suggestions = suggestions;
    }

    pub(super) fn is_active(&self) -> bool {
        self.active
    }

    pub(super) fn dismiss(&mut self) -> bool {
        if !self.active {
            return false;
        }

        *self = Self::default();
        true
    }

    pub(super) fn move_selection(&mut self, delta: isize) -> bool {
        if !self.active || self.suggestions.is_empty() {
            return false;
        }

        let len = self.suggestions.len() as isize;
        let next = (self.selected_index as isize + delta).rem_euclid(len);
        let changed = next as usize != self.selected_index;
        self.selected_index = next as usize;
        changed
    }

    pub(super) fn selected_command(&self) -> Option<InlineShellCommand> {
        self.suggestions.get(self.selected_index).copied()
    }

    pub(super) fn selected_index(&self) -> Option<usize> {
        self.selected_command().map(|_| self.selected_index)
    }

    pub(super) fn suggestions(&self) -> &[InlineShellCommand] {
        &self.suggestions
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

    pub(super) fn suggestion_detail(self) -> &'static str {
        self.spec().suggestion_detail
    }

    pub(super) fn requires_argument(self) -> bool {
        self.spec().requires_argument
    }

    pub(super) fn completion_text(self) -> &'static str {
        match self {
            InlineShellCommand::Reset => ":reset ",
            InlineShellCommand::MaxAutoTurns => ":turns ",
            InlineShellCommand::Diagnostics
            | InlineShellCommand::Parallel
            | InlineShellCommand::Sessions
            | InlineShellCommand::Queue
            | InlineShellCommand::Directions
            | InlineShellCommand::Stop
            | InlineShellCommand::Automation
            | InlineShellCommand::Doctor
            | InlineShellCommand::Init
            | InlineShellCommand::PlanningInit
            | InlineShellCommand::NewDraft
            | InlineShellCommand::Help => self.command_name(),
        }
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
    let trimmed_start = input.trim_start();
    if trimmed_start.is_empty() || !trimmed_start.starts_with(':') {
        return None;
    }

    let command_token_end = trimmed_start
        .find(char::is_whitespace)
        .unwrap_or(trimmed_start.len());
    if command_token_end != trimmed_start.len() {
        return None;
    }

    Some(trimmed_start[..command_token_end].to_ascii_lowercase())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResetArgument<'a> {
    None,
    Queue { confirmed: bool },
    Directions { confirmed: bool },
    All { confirmed: bool },
    Invalid(&'a str),
}

fn parse_reset_argument(argument: Option<&str>) -> ResetArgument<'_> {
    let Some(argument) = argument.map(str::trim).filter(|value| !value.is_empty()) else {
        return ResetArgument::None;
    };
    let mut parts = argument.split_whitespace();
    let Some(target) = parts.next() else {
        return ResetArgument::None;
    };
    let confirmation = parts.next();
    if parts.next().is_some() {
        return ResetArgument::Invalid(target);
    }
    let confirmed = matches!(
        confirmation,
        Some(value) if value.eq_ignore_ascii_case("confirm")
    );
    if confirmation.is_some() && !confirmed {
        return ResetArgument::Invalid(target);
    }
    match target.to_ascii_lowercase().as_str() {
        "queue" => ResetArgument::Queue { confirmed },
        "directions" => ResetArgument::Directions { confirmed },
        "all" => ResetArgument::All { confirmed },
        _ => ResetArgument::Invalid(target),
    }
}

fn is_valid_max_auto_turn_argument(value: &str) -> bool {
    AutoFollowState::normalize_max_auto_turns_candidate(value).is_some()
}

#[cfg(test)]
mod tests {
    use super::{
        InlineShellCommand, InlineShellCommandInput, InlineShellCommandPaletteState,
        MAX_AUTO_TURNS_USAGE, RESET_USAGE,
    };

    #[test]
    fn parse_recognizes_supported_aliases() {
        let cases = [
            (":diag", Some((InlineShellCommand::Diagnostics, None))),
            (
                ":diagnostics",
                Some((InlineShellCommand::Diagnostics, None)),
            ),
            (":parallel", Some((InlineShellCommand::Parallel, None))),
            (
                ":parallel on",
                Some((InlineShellCommand::Parallel, Some("on"))),
            ),
            (
                ":parallel off",
                Some((InlineShellCommand::Parallel, Some("off"))),
            ),
            (":DIAG", Some((InlineShellCommand::Diagnostics, None))),
            (":session", Some((InlineShellCommand::Sessions, None))),
            (":sessions", Some((InlineShellCommand::Sessions, None))),
            (":q", Some((InlineShellCommand::Queue, None))),
            (":queue", Some((InlineShellCommand::Queue, None))),
            (":directions", Some((InlineShellCommand::Directions, None))),
            (
                ":directions apply",
                Some((InlineShellCommand::Directions, Some("apply"))),
            ),
            (":stop", Some((InlineShellCommand::Stop, None))),
            (":auto", Some((InlineShellCommand::Automation, None))),
            (":automation", Some((InlineShellCommand::Automation, None))),
            (":doctor", Some((InlineShellCommand::Doctor, None))),
            (":init", Some((InlineShellCommand::Init, None))),
            (":planning", Some((InlineShellCommand::PlanningInit, None))),
            (
                ":planning off",
                Some((InlineShellCommand::PlanningInit, Some("off"))),
            ),
            (
                ":planning on",
                Some((InlineShellCommand::PlanningInit, Some("on"))),
            ),
            (
                ":planning doctor",
                Some((InlineShellCommand::PlanningInit, Some("doctor"))),
            ),
            (
                ":planning-init",
                Some((InlineShellCommand::PlanningInit, None)),
            ),
            (
                ":reset queue",
                Some((InlineShellCommand::Reset, Some("queue"))),
            ),
            (
                ":reset directions confirm",
                Some((InlineShellCommand::Reset, Some("directions confirm"))),
            ),
            (
                ":turns 5",
                Some((InlineShellCommand::MaxAutoTurns, Some("5"))),
            ),
            (
                ":turns infinite",
                Some((InlineShellCommand::MaxAutoTurns, Some("infinite"))),
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
                InlineShellCommand::Parallel,
                InlineShellCommand::Sessions,
                InlineShellCommand::Queue,
                InlineShellCommand::Directions,
                InlineShellCommand::Stop,
                InlineShellCommand::Automation,
                InlineShellCommand::Doctor,
                InlineShellCommand::Init,
                InlineShellCommand::PlanningInit,
                InlineShellCommand::Reset,
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
            vec![
                InlineShellCommand::Parallel,
                InlineShellCommand::PlanningInit
            ]
        );
        assert_eq!(
            InlineShellCommand::suggestions(":q"),
            vec![InlineShellCommand::Queue]
        );
        assert_eq!(
            InlineShellCommand::suggestions(":do"),
            vec![InlineShellCommand::Doctor]
        );
        assert_eq!(
            InlineShellCommand::suggestions(":i"),
            vec![InlineShellCommand::Init]
        );
        assert_eq!(
            InlineShellCommand::suggestions(":re"),
            vec![InlineShellCommand::Reset]
        );
        assert_eq!(
            InlineShellCommand::suggestions(":st"),
            vec![InlineShellCommand::Stop]
        );
        assert_eq!(
            InlineShellCommand::suggestions(":t"),
            vec![InlineShellCommand::MaxAutoTurns]
        );
    }

    #[test]
    fn suggestion_prefix_only_stays_active_while_typing_command_name() {
        assert_eq!(
            InlineShellCommand::suggestion_prefix(":planning"),
            Some(":planning".to_string())
        );
        assert_eq!(
            InlineShellCommand::suggestion_prefix("  :p"),
            Some(":p".to_string())
        );
        assert_eq!(InlineShellCommand::suggestion_prefix(":turns "), None);
        assert_eq!(InlineShellCommand::suggestion_prefix(":planning off"), None);
    }

    #[test]
    fn palette_state_keeps_selected_command_when_input_refines() {
        let mut state = InlineShellCommandPaletteState::default();
        state.sync_to_input(":", None);
        assert!(state.move_selection(9));
        assert_eq!(
            state.selected_command(),
            Some(InlineShellCommand::PlanningInit)
        );

        state.sync_to_input(":p", state.selected_command());

        assert_eq!(
            state.selected_command(),
            Some(InlineShellCommand::PlanningInit)
        );
    }

    #[test]
    fn completion_text_uses_canonical_argument_ready_command_forms() {
        assert_eq!(InlineShellCommand::Diagnostics.completion_text(), ":diag");
        assert_eq!(
            InlineShellCommand::PlanningInit.completion_text(),
            ":planning"
        );
        assert_eq!(InlineShellCommand::Parallel.completion_text(), ":parallel");
        assert_eq!(InlineShellCommand::Doctor.completion_text(), ":doctor");
        assert_eq!(InlineShellCommand::Init.completion_text(), ":init");
        assert_eq!(InlineShellCommand::Reset.completion_text(), ":reset ");
        assert_eq!(
            InlineShellCommand::MaxAutoTurns.completion_text(),
            ":turns "
        );
    }

    #[test]
    fn max_auto_turn_command_hint_is_argument_aware() {
        let no_arg = InlineShellCommandInput::parse(":turns").expect("command should parse");
        let valid_arg = InlineShellCommandInput::parse(":turns 7").expect("command should parse");
        let infinite_arg =
            InlineShellCommandInput::parse(":turns infinite").expect("command should parse");
        let invalid_arg = InlineShellCommandInput::parse(":turns 0").expect("command should parse");

        assert_eq!(no_arg.buffered_hint(), MAX_AUTO_TURNS_USAGE);
        assert_eq!(
            valid_arg.buffered_hint(),
            "Press Enter to set max auto turns to 7."
        );
        assert_eq!(
            infinite_arg.buffered_hint(),
            "Press Enter to set max auto turns to infinite."
        );
        assert_eq!(
            invalid_arg.buffered_hint(),
            "Press Enter to apply `:turns 0`. Max auto turns must be a whole number greater than 0 or `infinite`."
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
    fn planning_command_hint_is_argument_aware() {
        let plain = InlineShellCommandInput::parse(":planning").expect("command should parse");
        let off = InlineShellCommandInput::parse(":planning off").expect("command should parse");
        let on = InlineShellCommandInput::parse(":planning on").expect("command should parse");
        let doctor =
            InlineShellCommandInput::parse(":planning doctor").expect("command should parse");

        assert_eq!(
            plain.buffered_hint(),
            "Press Enter to open the planning control center."
        );
        assert_eq!(off.buffered_hint(), "Press Enter to turn Plan off.");
        assert_eq!(on.buffered_hint(), "Press Enter to turn Plan on.");
        assert_eq!(
            doctor.buffered_hint(),
            "Press Enter to inspect planning health."
        );
    }

    #[test]
    fn directions_command_hint_is_argument_aware() {
        let plain = InlineShellCommandInput::parse(":directions").expect("command should parse");
        let apply =
            InlineShellCommandInput::parse(":directions apply").expect("command should parse");
        let invalid =
            InlineShellCommandInput::parse(":directions later").expect("command should parse");

        assert_eq!(
            plain.buffered_hint(),
            "Press Enter to review or edit planning directions."
        );
        assert_eq!(
            apply.buffered_hint(),
            "Press Enter to import tracked directions into active planning."
        );
        assert_eq!(
            invalid.buffered_hint(),
            "Press Enter to apply `:directions later`. Supported arguments: apply."
        );
    }

    #[test]
    fn parallel_command_hint_is_argument_aware() {
        let plain = InlineShellCommandInput::parse(":parallel").expect("command should parse");
        let on = InlineShellCommandInput::parse(":parallel on").expect("command should parse");
        let off = InlineShellCommandInput::parse(":parallel off").expect("command should parse");
        let invalid =
            InlineShellCommandInput::parse(":parallel later").expect("command should parse");

        assert_eq!(
            plain.buffered_hint(),
            "Press Enter to inspect parallel mode readiness."
        );
        assert_eq!(
            on.buffered_hint(),
            "Press Enter to inspect readiness and enter parallel mode when allowed."
        );
        assert_eq!(
            off.buffered_hint(),
            "Press Enter to turn parallel mode off."
        );
        assert_eq!(
            invalid.buffered_hint(),
            "Press Enter to apply `:parallel later`. Supported arguments: on, off."
        );
    }

    #[test]
    fn doctor_and_init_command_hints_use_lifecycle_language() {
        let doctor = InlineShellCommandInput::parse(":doctor").expect("command should parse");
        let init = InlineShellCommandInput::parse(":init").expect("command should parse");

        assert_eq!(
            doctor.buffered_hint(),
            "Press Enter to inspect planning health."
        );
        assert_eq!(
            init.buffered_hint(),
            "Press Enter to stage the default planning scaffold."
        );
    }

    #[test]
    fn reset_command_hint_is_argument_aware() {
        let plain = InlineShellCommandInput::parse(":reset").expect("command should parse");
        let queue = InlineShellCommandInput::parse(":reset queue").expect("command should parse");
        let directions =
            InlineShellCommandInput::parse(":reset directions").expect("command should parse");
        let directions_confirm = InlineShellCommandInput::parse(":reset directions confirm")
            .expect("command should parse");
        let invalid = InlineShellCommandInput::parse(":reset wrong").expect("command should parse");

        assert_eq!(plain.buffered_hint(), RESET_USAGE);
        assert_eq!(
            queue.buffered_hint(),
            "Press Enter to reset queue-side planning state."
        );
        assert_eq!(
            directions.buffered_hint(),
            "Review `:reset directions confirm` before rewriting directions-side planning files."
        );
        assert_eq!(
            directions_confirm.buffered_hint(),
            "Press Enter to confirm the directions reset."
        );
        assert_eq!(
            invalid.buffered_hint(),
            "Press Enter to apply `:reset wrong`. Supported arguments: queue, directions, all."
        );
    }

    #[test]
    fn execution_status_stays_alias_neutral() {
        let cases = [
            (":diag", Some("opened diagnostics inspection")),
            (":sessions", Some("opened recent sessions inspection")),
            (":queue", Some("opened planning queue inspection")),
            (":stop", None),
            (":auto", Some("opened automation controls")),
            (":doctor", None),
            (":init", None),
            (":planning", None),
            (":reset queue", None),
            (":turns 5", None),
        ];

        for (input, expected) in cases {
            let command =
                InlineShellCommandInput::parse(input).expect("inline shell command should parse");
            assert_eq!(command.execution_status().as_deref(), expected);
        }
    }
}
