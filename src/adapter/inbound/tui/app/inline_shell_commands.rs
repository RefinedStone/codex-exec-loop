use super::parallel_mode_shell_command::{
    ParsedParallelModeShellCommand, parse_parallel_mode_shell_argument,
};
use super::planning_reset_shell_command::{
    ParsedPlanningResetShellCommand, parse_planning_reset_shell_argument,
};
use super::planning_shell_command::{ParsedPlanningShellCommand, parse_planning_shell_argument};
use super::task_shell_command::{ParsedTaskShellCommand, parse_task_shell_argument};
use crate::application::service::planning::PlanningResetTarget;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InlineShellCommand {
    Diagnostics,
    Parallel,
    Sessions,
    Queue,
    Directions,
    Task,
    Turns,
    Stop,
    Doctor,
    PlanningInit,
    Reset,
    NewDraft,
    Help,
}

// Parsed command input keeps the canonical command separate from the free-form
// argument tail so controllers can share one execution path for typed commands
// and palette-accepted commands.
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

// The spec table is the command registry: aliases, palette labels, buffered
// hints, command help, and completion behavior all derive from these entries.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InlineShellCommandHelpEntry {
    pub(crate) usage: &'static str,
    pub(crate) detail: &'static str,
}
#[cfg(test)]
const COMMAND_LIST_LINE: &str = "Shell commands: :diag  :parallel [off]  :sessions  :queue  :directions  :task [prompt]  :turns <number|infinite>  :stop  :planning [doctor]  :doctor  :reset <queue|directions|all>  :new  :help";
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
        buffered_hint: "Press Enter to enter parallel mode.",
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
        command: InlineShellCommand::Task,
        primary_name: ":task",
        aliases: &[":task"],
        suggestion_detail: "task intake",
        buffered_hint: "Press Enter to draft a runtime planning task.",
        execution_status: None,
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Turns,
        primary_name: ":turns",
        aliases: &[":turns", ":auto-turns"],
        suggestion_detail: "auto turn budget",
        buffered_hint: "Type `:turns <number|infinite>` to set the auto-follow turn budget.",
        execution_status: None,
        requires_argument: true,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Stop,
        primary_name: ":stop",
        aliases: &[":stop"],
        suggestion_detail: "stop active sessions",
        buffered_hint: "Press Enter to stop active app-server sessions.",
        execution_status: None,
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
        buffered_hint: "Press Enter to open shell command help.",
        execution_status: Some("opened shell command help"),
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

    // Buffered hints are intentionally command-aware instead of purely
    // spec-driven because several commands have operator-sensitive arguments.
    pub(super) fn buffered_hint(&self) -> String {
        match self.command {
            InlineShellCommand::Parallel => parallel_argument_hint(self.argument()),
            InlineShellCommand::PlanningInit => planning_argument_hint(self.argument()),
            InlineShellCommand::Directions => match self.argument() {
                Some(value) => format!(
                    "Press Enter to apply `:directions {value}`. Supported command: :directions."
                ),
                None => self.command.spec().buffered_hint.to_string(),
            },
            InlineShellCommand::Task => task_argument_hint(self.argument()),
            InlineShellCommand::Turns => match self.argument() {
                Some(value) => {
                    format!("Press Enter to set the auto-follow turn budget to `{value}`.")
                }
                None => self.command.spec().buffered_hint.to_string(),
            },
            InlineShellCommand::Queue => match self.argument() {
                Some(value) => format!(
                    "`:queue` does not accept arguments (`{value}`); press Enter to open queue inspection."
                ),
                None => self.command.spec().buffered_hint.to_string(),
            },
            InlineShellCommand::Reset => match parse_reset_argument(self.argument()) {
                Some(parsed) => reset_argument_hint(parsed),
                None => reset_argument_recovery_hint(self.argument()),
            },
            _ => self.command.spec().buffered_hint.to_string(),
        }
    }
    pub(super) fn execution_status(&self) -> Option<String> {
        // Commands that perform asynchronous or state-dependent work report
        // status from their controller handlers, not the generic command layer.
        match self.command {
            InlineShellCommand::Queue if self.argument().is_some() => None,
            InlineShellCommand::Turns => None,
            InlineShellCommand::Stop => None,
            _ => self.command.spec().execution_status.map(str::to_string),
        }
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

        // Keep keyboard selection stable across input edits when the previously
        // selected command still exists in the filtered suggestion list.
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
        // Palette navigation wraps so repeated up/down keys stay inside the
        // current filtered command list.
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

        // A bare colon opens the full command palette; partial tokens filter by
        // aliases but arguments deliberately close suggestions.
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
            InlineShellCommand::Turns => ":turns ",
            InlineShellCommand::Diagnostics
            | InlineShellCommand::Parallel
            | InlineShellCommand::Sessions
            | InlineShellCommand::Queue
            | InlineShellCommand::Directions
            | InlineShellCommand::Task
            | InlineShellCommand::Stop
            | InlineShellCommand::Doctor
            | InlineShellCommand::PlanningInit
            | InlineShellCommand::NewDraft
            | InlineShellCommand::Help => self.command_name(),
        }
    }
    pub(crate) fn help_entries() -> Vec<InlineShellCommandHelpEntry> {
        INLINE_SHELL_COMMAND_SPECS
            .iter()
            .map(|spec| InlineShellCommandHelpEntry {
                usage: spec.command.help_usage(),
                detail: spec.suggestion_detail,
            })
            .collect()
    }
    fn help_usage(self) -> &'static str {
        match self {
            InlineShellCommand::Parallel => ":parallel [off]",
            InlineShellCommand::Queue => ":queue",
            InlineShellCommand::Directions => ":directions",
            InlineShellCommand::Task => ":task [prompt]",
            InlineShellCommand::Turns => ":turns <number|infinite>",
            InlineShellCommand::Stop => ":stop",
            InlineShellCommand::PlanningInit => ":planning [doctor]",
            InlineShellCommand::Reset => ":reset <queue|directions|all>",
            InlineShellCommand::Diagnostics
            | InlineShellCommand::Sessions
            | InlineShellCommand::Doctor
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
    // Execution parsing accepts surrounding whitespace and preserves everything
    // after the command token as the argument string.
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
    // Suggestions are only for the command token currently being typed. Once
    // whitespace appears, the user is editing an argument and the palette hides.
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
fn parallel_argument_hint(argument: Option<&str>) -> String {
    match parse_parallel_mode_shell_argument(argument) {
        Ok(ParsedParallelModeShellCommand::Enable) => InlineShellCommand::Parallel
            .spec()
            .buffered_hint
            .to_string(),
        Ok(ParsedParallelModeShellCommand::Disable) => {
            "Press Enter to turn parallel mode off.".to_string()
        }
        Err(error) => format!(
            "Press Enter to apply `:parallel {}`. Supported command forms: :parallel, :parallel off.",
            error.argument()
        ),
    }
}
fn planning_argument_hint(argument: Option<&str>) -> String {
    match parse_planning_shell_argument(argument) {
        Ok(ParsedPlanningShellCommand::OpenControlCenter) => InlineShellCommand::PlanningInit
            .spec()
            .buffered_hint
            .to_string(),
        Ok(ParsedPlanningShellCommand::Doctor) => {
            "Press Enter to inspect planning health.".to_string()
        }
        Err(error) => format!(
            "Press Enter to apply `:planning {}`. Supported arguments: doctor.",
            error.argument()
        ),
    }
}
fn task_argument_hint(argument: Option<&str>) -> String {
    match parse_task_shell_argument(argument) {
        ParsedTaskShellCommand::OpenPromptEditor => {
            InlineShellCommand::Task.spec().buffered_hint.to_string()
        }
        ParsedTaskShellCommand::PreviewPrompt { prompt } => {
            format!("Press Enter to preview a runtime task for `{prompt}`.")
        }
    }
}
fn parse_reset_argument(argument: Option<&str>) -> Option<ParsedPlanningResetShellCommand> {
    parse_planning_reset_shell_argument(argument).ok()
}

fn reset_argument_hint(parsed: ParsedPlanningResetShellCommand) -> String {
    match (parsed.target, parsed.confirmed) {
        (PlanningResetTarget::Queue, _) => {
            "Press Enter to reset queue-side planning state.".to_string()
        }
        (PlanningResetTarget::Directions, true) => {
            "Press Enter to confirm the directions reset.".to_string()
        }
        (PlanningResetTarget::Directions, false) => {
            "Review `:reset directions confirm` before rewriting directions-side planning files."
                .to_string()
        }
        (PlanningResetTarget::All, true) => {
            "Press Enter to confirm the full planning reset.".to_string()
        }
        (PlanningResetTarget::All, false) => {
            "Review `:reset all confirm` before replacing the full planning scaffold.".to_string()
        }
    }
}

fn reset_argument_recovery_hint(argument: Option<&str>) -> String {
    let Some(argument) = argument.map(str::trim).filter(|value| !value.is_empty()) else {
        return RESET_USAGE.to_string();
    };
    format!(
        "Press Enter to apply `:reset {argument}`. Supported arguments: queue, directions, all."
    )
}
#[cfg(test)]
#[path = "inline_shell_commands/tests.rs"]
mod tests;
