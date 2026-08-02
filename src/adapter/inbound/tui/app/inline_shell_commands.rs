use super::language::TuiLanguage;
use super::parallel_mode_shell_command::{
    ParsedParallelModeShellCommand, parse_parallel_mode_shell_argument,
};
use super::planning_overlay_shell_command::parse_planning_overlay_shell_argument;
use super::planning_reset_shell_command::{
    ParsedPlanningResetShellCommand, parse_planning_reset_shell_argument,
};
use super::planning_shell_command::{ParsedPlanningShellCommand, parse_planning_shell_argument};
use super::progressive_activity_overlay_ui::{
    parse_progressive_activity_card_filter, parse_progressive_activity_detail_kind,
};
use super::view_selection_overlay_ui::ConversationViewMode;
use crate::application::service::planning::PlanningResetTarget;
use crate::domain::conversation::ConversationReasoningEffort;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InlineShellCommand {
    Diagnostics,
    Work,
    Parallel,
    Peek,
    Activity,
    Sessions,
    Reviews,
    Queue,
    Directions,
    Turns,
    Stop,
    Model,
    View,
    Language,
    Think,
    Copy,
    Mouse,
    Doctor,
    PlanningInit,
    Reset,
    NewDraft,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InlineShellCommandStartupReadiness {
    Ready,
    Pending,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InlineShellCommandAvailabilityReason {
    StartupChecksRunning,
    StartupDiagnosticsNeedAttention,
    ParallelTransitionInFlight,
    ParallelModeDisabled,
    NoActiveParallelAgents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InlineShellCommandAvailability {
    Ready,
    Pending(InlineShellCommandAvailabilityReason),
    Locked(InlineShellCommandAvailabilityReason),
}

impl InlineShellCommandAvailability {
    pub(crate) fn is_ready(self) -> bool {
        self == Self::Ready
    }

    pub(crate) fn reason(self) -> Option<InlineShellCommandAvailabilityReason> {
        match self {
            Self::Ready => None,
            Self::Pending(reason) | Self::Locked(reason) => Some(reason),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InlineShellCommandCapabilityContext {
    pub(crate) startup_readiness: InlineShellCommandStartupReadiness,
    pub(crate) parallel_mode_enabled: bool,
    pub(crate) parallel_transition_in_flight: bool,
    pub(crate) active_parallel_agent_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InlineShellCommandCapabilitySet {
    entries: Vec<(InlineShellCommand, InlineShellCommandAvailability)>,
    parallel_mode_enabled: bool,
}

impl InlineShellCommandCapabilitySet {
    pub(crate) fn from_application_context(context: InlineShellCommandCapabilityContext) -> Self {
        let entries = INLINE_SHELL_COMMAND_SPECS
            .iter()
            .map(|spec| {
                (
                    spec.command,
                    inline_shell_command_availability(spec.command, context),
                )
            })
            .collect();
        Self {
            entries,
            parallel_mode_enabled: context.parallel_mode_enabled,
        }
    }

    pub(crate) fn availability(
        &self,
        command: InlineShellCommand,
    ) -> InlineShellCommandAvailability {
        self.entries
            .iter()
            .find_map(|(candidate, availability)| (*candidate == command).then_some(*availability))
            .unwrap_or(InlineShellCommandAvailability::Ready)
    }

    pub(crate) fn parallel_mode_enabled(&self) -> bool {
        self.parallel_mode_enabled
    }
}

impl Default for InlineShellCommandCapabilitySet {
    fn default() -> Self {
        Self::from_application_context(InlineShellCommandCapabilityContext {
            startup_readiness: InlineShellCommandStartupReadiness::Ready,
            parallel_mode_enabled: false,
            parallel_transition_in_flight: false,
            active_parallel_agent_count: 0,
        })
    }
}

fn inline_shell_command_availability(
    command: InlineShellCommand,
    context: InlineShellCommandCapabilityContext,
) -> InlineShellCommandAvailability {
    use InlineShellCommandAvailability::{Locked, Pending, Ready};
    use InlineShellCommandAvailabilityReason::{
        NoActiveParallelAgents, ParallelModeDisabled, ParallelTransitionInFlight,
        StartupChecksRunning, StartupDiagnosticsNeedAttention,
    };

    match command {
        InlineShellCommand::Parallel | InlineShellCommand::Peek
            if context.parallel_transition_in_flight =>
        {
            Pending(ParallelTransitionInFlight)
        }
        InlineShellCommand::Peek if !context.parallel_mode_enabled => Locked(ParallelModeDisabled),
        InlineShellCommand::Peek if context.active_parallel_agent_count == 0 => {
            Locked(NoActiveParallelAgents)
        }
        InlineShellCommand::Sessions if !context.parallel_mode_enabled => {
            match context.startup_readiness {
                InlineShellCommandStartupReadiness::Ready => Ready,
                InlineShellCommandStartupReadiness::Pending => Pending(StartupChecksRunning),
                InlineShellCommandStartupReadiness::Blocked => {
                    Locked(StartupDiagnosticsNeedAttention)
                }
            }
        }
        _ => Ready,
    }
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
    buffered_hint: &'static str,
    execution_status: Option<&'static str>,
    requires_argument: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InlineShellCommandHelpEntry {
    pub(crate) usage: &'static str,
    pub(crate) detail: &'static str,
}
const ACTIVITY_USAGE: &str = "Type `:activity [all|diff|output|command|patch|…]` to inspect retained progressive activity cards.";
const RESET_USAGE: &str =
    "Type `:reset <queue|directions|all>` and press Enter to reset planning state.";
const MODEL_USAGE: &str = "Type `:model` to choose the model and think level, or `:model default` to use app-server defaults.";
const VIEW_USAGE: &str = "Type `:view` to choose transcript visibility for tool/status rows.";
const LANGUAGE_USAGE: &str =
    "Type `:language` to choose the TUI language, or `:language english|korean`.";
const THINK_USAGE: &str =
    "Type `:think <none|minimal|low|medium|high|xhigh|default>` to choose reasoning effort.";
const COPY_USAGE: &str =
    "Type `:copy` to copy the selection or latest answer; use `:copy selection|last` explicitly.";
const MOUSE_USAGE: &str =
    "Type `:mouse` for status, or `:mouse on|off|toggle` to control app mouse capture.";
#[cfg(test)]
const COMMAND_LIST_LINE: &str = "Shell commands: :diag  :work  :parallel [off]  :peek  :activity [all|diff|output|command|…]  :sessions  :reviews  :queue  :directions  :turns <positive|infinite|off>  :stop  :model [default]  :view [simple|medium|detail]  :language [english|korean]  :think <none|minimal|low|medium|high|xhigh|default>  :copy [selection|last]  :mouse [on|off|toggle]  :planning [doctor]  :doctor  :reset <queue|directions|all>  :new  :help";
const THINK_SUPPORTED_VALUES: &str = ConversationReasoningEffort::SUPPORTED_LABELS;

const INLINE_SHELL_COMMAND_SPECS: &[InlineShellCommandSpec] = &[
    InlineShellCommandSpec {
        command: InlineShellCommand::Diagnostics,
        primary_name: ":diag",
        aliases: &[":diag", ":diagnostics"],
        buffered_hint: "Press Enter to open the diagnostics inspection.",
        execution_status: Some("opened diagnostics inspection"),
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Work,
        primary_name: ":work",
        aliases: &[":w", ":work"],
        buffered_hint: "Press Enter to open the unified work center.",
        execution_status: Some("opened unified work center"),
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Parallel,
        primary_name: ":parallel",
        aliases: &[":pa", ":parallel"],
        buffered_hint: "Press Enter to enter parallel mode.",
        execution_status: None,
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Peek,
        primary_name: ":peek",
        aliases: &[":peek"],
        buffered_hint: "Press Enter to inspect active parallel agents.",
        execution_status: None,
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Activity,
        primary_name: ":activity",
        aliases: &[":activity", ":act"],
        buffered_hint: ACTIVITY_USAGE,
        execution_status: None,
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Sessions,
        primary_name: ":sessions",
        aliases: &[":session", ":sessions"],
        buffered_hint: "Press Enter to open the recent-sessions inspection.",
        execution_status: Some("opened recent sessions inspection"),
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Reviews,
        primary_name: ":reviews",
        aliases: &[":reviews", ":review"],
        buffered_hint: "Press Enter to open the review center inspection.",
        execution_status: Some("opened review center inspection"),
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Queue,
        primary_name: ":queue",
        aliases: &[":q", ":queue"],
        buffered_hint: "Press Enter to open the planning queue inspection.",
        execution_status: Some("opened planning queue inspection"),
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Directions,
        primary_name: ":directions",
        aliases: &[":directions"],
        buffered_hint: "Press Enter to review or edit planning directions.",
        execution_status: None,
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Turns,
        primary_name: ":turns",
        aliases: &[":turns", ":auto-turns"],
        buffered_hint: "Type `:turns <positive|infinite>` to enable auto-follow, or `:turns off` to disable it.",
        execution_status: None,
        requires_argument: true,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Stop,
        primary_name: ":stop",
        aliases: &[":stop"],
        buffered_hint: "Press Enter to stop active app-server sessions.",
        execution_status: None,
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Model,
        primary_name: ":model",
        aliases: &[":model"],
        buffered_hint: MODEL_USAGE,
        execution_status: None,
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::View,
        primary_name: ":view",
        aliases: &[":view"],
        buffered_hint: VIEW_USAGE,
        execution_status: None,
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Language,
        primary_name: ":language",
        aliases: &[":language", ":lang"],
        buffered_hint: LANGUAGE_USAGE,
        execution_status: None,
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Think,
        primary_name: ":think",
        aliases: &[":think"],
        buffered_hint: THINK_USAGE,
        execution_status: None,
        requires_argument: true,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Copy,
        primary_name: ":copy",
        aliases: &[":copy"],
        buffered_hint: COPY_USAGE,
        execution_status: None,
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Mouse,
        primary_name: ":mouse",
        aliases: &[":mouse"],
        buffered_hint: MOUSE_USAGE,
        execution_status: None,
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Doctor,
        primary_name: ":doctor",
        aliases: &[":doctor"],
        buffered_hint: "Press Enter to inspect planning health.",
        execution_status: None,
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::PlanningInit,
        primary_name: ":planning",
        aliases: &[":planning", ":planning-init"],
        buffered_hint: "Press Enter to open the planning control center.",
        execution_status: None,
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Reset,
        primary_name: ":reset",
        aliases: &[":r", ":reset"],
        buffered_hint: RESET_USAGE,
        execution_status: None,
        requires_argument: true,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::NewDraft,
        primary_name: ":new",
        aliases: &[":new"],
        buffered_hint: "Press Enter to open a new draft in the shell.",
        execution_status: None,
        requires_argument: false,
    },
    InlineShellCommandSpec {
        command: InlineShellCommand::Help,
        primary_name: ":help",
        aliases: &[":help"],
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
            InlineShellCommand::Activity => activity_argument_hint(self.argument()),
            InlineShellCommand::PlanningInit => planning_argument_hint(self.argument()),
            InlineShellCommand::Directions => planning_overlay_argument_hint(
                self.argument(),
                InlineShellCommand::Directions,
                "directions",
            ),
            InlineShellCommand::Turns => match self.argument() {
                Some(value) if value.eq_ignore_ascii_case("off") || value == "0" => {
                    "Press Enter to disable auto-follow.".to_string()
                }
                Some(value) => format!(
                    "Press Enter to explicitly enable auto-follow with turn budget `{value}`."
                ),
                None => self.command.spec().buffered_hint.to_string(),
            },
            InlineShellCommand::Model => model_argument_hint(self.argument()),
            InlineShellCommand::View => view_argument_hint(self.argument()),
            InlineShellCommand::Language => language_argument_hint(self.argument()),
            InlineShellCommand::Think => think_argument_hint(self.argument()),
            InlineShellCommand::Copy => copy_argument_hint(self.argument()),
            InlineShellCommand::Mouse => mouse_argument_hint(self.argument()),
            InlineShellCommand::Queue => {
                planning_overlay_argument_hint(self.argument(), InlineShellCommand::Queue, "queue")
            }
            InlineShellCommand::Reviews => reviews_argument_hint(self.argument()),
            InlineShellCommand::Reset => match parse_reset_argument(self.argument()) {
                Some(parsed) => reset_argument_hint(parsed),
                None => reset_argument_recovery_hint(self.argument()),
            },
            _ => self.command.spec().buffered_hint.to_string(),
        }
    }

    pub(super) fn localized_buffered_hint(&self, language: TuiLanguage) -> String {
        if language == TuiLanguage::English {
            return self.buffered_hint();
        }
        language.inline_shell_command_buffered_hint(
            self.command,
            self.argument(),
            self.command.spec().buffered_hint,
        )
    }
    pub(super) fn execution_status(&self) -> Option<String> {
        // Commands that perform asynchronous or state-dependent work report
        // status from their controller handlers, not the generic command layer.
        match self.command {
            InlineShellCommand::Queue if self.argument().is_some() => None,
            InlineShellCommand::Reviews if self.argument().is_some() => None,
            InlineShellCommand::Turns => None,
            InlineShellCommand::Stop => None,
            InlineShellCommand::Model => None,
            InlineShellCommand::View => None,
            InlineShellCommand::Language => None,
            InlineShellCommand::Think => None,
            InlineShellCommand::Copy => None,
            InlineShellCommand::Mouse => None,
            _ => self.command.spec().execution_status.map(str::to_string),
        }
    }
    pub(super) fn localized_execution_status(&self, language: TuiLanguage) -> Option<String> {
        self.execution_status().map(|english| {
            language.inline_shell_command_execution_status(self.command, english.as_str())
        })
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
        let mut matching_specs: Vec<_> = INLINE_SHELL_COMMAND_SPECS
            .iter()
            .filter(|spec| {
                prefix == ":"
                    || spec
                        .aliases
                        .iter()
                        .any(|alias| alias.starts_with(prefix.as_str()))
            })
            .collect();
        if prefix != ":" {
            // Exact aliases win over longer prefix matches. This preserves the
            // established `:r` reset shortcut after `:reviews` joined the
            // catalog, while still exposing both commands for discovery.
            matching_specs.sort_by_key(|spec| !spec.aliases.contains(&prefix.as_str()));
        }
        matching_specs
            .into_iter()
            .map(|spec| spec.command)
            .collect()
    }
    pub(super) fn suggestion_prefix(input: &str) -> Option<String> {
        suggestion_prefix_token(input)
    }
    pub(super) fn command_name(self) -> &'static str {
        self.spec().primary_name
    }
    pub(super) fn suggestion_detail(self, language: TuiLanguage) -> &'static str {
        language.inline_shell_command_detail(self)
    }
    pub(super) fn requires_argument(self) -> bool {
        self.spec().requires_argument
    }
    pub(super) fn completion_text(self) -> &'static str {
        match self {
            InlineShellCommand::Reset => ":reset ",
            InlineShellCommand::Turns => ":turns ",
            InlineShellCommand::Model => ":model",
            InlineShellCommand::View => ":view",
            InlineShellCommand::Language => ":language",
            InlineShellCommand::Think => ":think ",
            InlineShellCommand::Copy => ":copy",
            InlineShellCommand::Mouse => ":mouse",
            InlineShellCommand::Diagnostics
            | InlineShellCommand::Work
            | InlineShellCommand::Parallel
            | InlineShellCommand::Peek
            | InlineShellCommand::Activity
            | InlineShellCommand::Sessions
            | InlineShellCommand::Reviews
            | InlineShellCommand::Queue
            | InlineShellCommand::Directions
            | InlineShellCommand::Stop
            | InlineShellCommand::Doctor
            | InlineShellCommand::PlanningInit
            | InlineShellCommand::NewDraft
            | InlineShellCommand::Help => self.command_name(),
        }
    }
    pub(super) fn help_entries(language: TuiLanguage) -> Vec<InlineShellCommandHelpEntry> {
        INLINE_SHELL_COMMAND_SPECS
            .iter()
            .map(|spec| InlineShellCommandHelpEntry {
                usage: spec.command.help_usage(),
                detail: spec.command.suggestion_detail(language),
            })
            .collect()
    }
    fn help_usage(self) -> &'static str {
        match self {
            InlineShellCommand::Work => ":work",
            InlineShellCommand::Parallel => ":parallel [off]",
            InlineShellCommand::Peek => ":peek",
            InlineShellCommand::Activity => ":activity [all|diff|output|command|patch|mcp|plan|…]",
            InlineShellCommand::Queue => ":queue",
            InlineShellCommand::Reviews => ":reviews",
            InlineShellCommand::Directions => ":directions",
            InlineShellCommand::Turns => ":turns <positive|infinite|off>",
            InlineShellCommand::Stop => ":stop",
            InlineShellCommand::Model => ":model",
            InlineShellCommand::View => ":view [simple|medium|detail]",
            InlineShellCommand::Language => ":language [english|korean]",
            InlineShellCommand::Think => ":think <none|minimal|low|medium|high|xhigh|default>",
            InlineShellCommand::Copy => ":copy [selection|last]",
            InlineShellCommand::Mouse => ":mouse [on|off|toggle]",
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
            "Press Enter to apply `:parallel {}`. Supported command forms: :parallel, :pa, :parallel off, :pa off.",
            error.argument()
        ),
    }
}
fn activity_argument_hint(argument: Option<&str>) -> String {
    let Some(argument) = argument else {
        return ACTIVITY_USAGE.to_string();
    };
    if parse_progressive_activity_detail_kind(argument).is_some()
        || parse_progressive_activity_card_filter(argument).is_some()
    {
        return format!(
            "Press Enter to inspect retained `{}` activity cards.",
            argument.trim().to_ascii_lowercase()
        );
    }
    format!(
        "`:activity {}` is unsupported; pressing Enter leaves the current overlay unchanged. Supported values: all, diff, output, command, patch, mcp, plan, reason, agent, terminal, token, guardian, moderation, unknown.",
        argument.trim()
    )
}
pub(super) fn is_turn_option_clear_argument(argument: &str) -> bool {
    matches!(
        argument.trim().to_ascii_lowercase().as_str(),
        "default" | "auto" | "clear" | "unset"
    )
}
fn model_argument_hint(argument: Option<&str>) -> String {
    match argument {
        None => MODEL_USAGE.to_string(),
        Some(value) if is_turn_option_clear_argument(value) => {
            "Press Enter to reset model to the app-server default.".to_string()
        }
        Some(_) => {
            "`:model` ignores typed model names; press Enter to open model selection.".to_string()
        }
    }
}
fn view_argument_hint(argument: Option<&str>) -> String {
    let Some(argument) = argument else {
        return VIEW_USAGE.to_string();
    };
    match ConversationViewMode::parse(argument) {
        Some(mode) => format!(
            "Press Enter to set conversation view to `{}`.",
            mode.label()
        ),
        None => format!(
            "Press Enter to apply `:view {}`. Supported values: {}.",
            argument.trim(),
            ConversationViewMode::SUPPORTED_LABELS
        ),
    }
}
fn language_argument_hint(argument: Option<&str>) -> String {
    let Some(argument) = argument else {
        return LANGUAGE_USAGE.to_string();
    };
    match TuiLanguage::parse(argument) {
        Some(language) => format!(
            "Press Enter to set TUI language to {}.",
            language.status_label()
        ),
        None => format!(
            "Press Enter to apply `:language {}`. Supported values: {}.",
            argument.trim(),
            TuiLanguage::SUPPORTED_LABELS
        ),
    }
}
fn think_argument_hint(argument: Option<&str>) -> String {
    let Some(argument) = argument else {
        return THINK_USAGE.to_string();
    };
    if is_turn_option_clear_argument(argument) {
        return "Press Enter to reset think to the app-server default.".to_string();
    }
    match ConversationReasoningEffort::parse(argument) {
        Some(effort) => format!("Press Enter to set think to `{}`.", effort.label()),
        None => format!(
            "Press Enter to apply `:think {}`. Supported values: {THINK_SUPPORTED_VALUES}.",
            argument.trim()
        ),
    }
}
fn copy_argument_hint(argument: Option<&str>) -> String {
    match argument
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None | Some("selection" | "selected" | "last" | "answer" | "response") => {
            COPY_USAGE.to_string()
        }
        Some(value) => {
            format!("`:copy {value}` is unsupported. Supported values: selection, last.")
        }
    }
}
fn mouse_argument_hint(argument: Option<&str>) -> String {
    match argument
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None | Some("status" | "on" | "off" | "toggle" | "enable" | "disable") => {
            MOUSE_USAGE.to_string()
        }
        Some(value) => {
            format!("`:mouse {value}` is unsupported. Supported values: on, off, toggle.")
        }
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
fn planning_overlay_argument_hint(
    argument: Option<&str>,
    command: InlineShellCommand,
    label: &str,
) -> String {
    match parse_planning_overlay_shell_argument(argument) {
        Ok(()) => command.spec().buffered_hint.to_string(),
        Err(error) if command == InlineShellCommand::Queue => format!(
            "`:queue` does not accept arguments (`{}`); press Enter to open queue inspection.",
            error.argument()
        ),
        Err(error) => format!(
            "Press Enter to apply `:{} {}`. Supported command: :{}.",
            command.command_name().trim_start_matches(':'),
            error.argument(),
            label
        ),
    }
}
fn reviews_argument_hint(argument: Option<&str>) -> String {
    match parse_planning_overlay_shell_argument(argument) {
        Ok(()) => InlineShellCommand::Reviews.spec().buffered_hint.to_string(),
        Err(error) => format!(
            "`:reviews` does not accept arguments (`{}`); press Enter to open the review center inspection.",
            error.argument()
        ),
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
