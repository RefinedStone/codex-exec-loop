use super::{
    InlineShellCommand, InlineShellCommandInput, InlineShellCommandPaletteState, RESET_USAGE,
};

/* Inline shell commands are typed directly into the prompt, so these tests pin
 * both parser compatibility and the short copy that appears before execution.
 * The aliases stay broad for operator muscle memory, but suggestions and help
 * deliberately expose only canonical forms.
 */
#[test]
fn parse_recognizes_supported_aliases() {
    /*
    Parsing accepts historical aliases and mixed case because these commands are
    typed from operator memory in the main prompt. Unsupported near-aliases such
    as :auto stay rejected so automation control does not grow ambiguous entry
    points outside the explicit command catalog.
    */
    let cases = [
        (":diag", Some((InlineShellCommand::Diagnostics, None))),
        (
            ":diagnostics",
            Some((InlineShellCommand::Diagnostics, None)),
        ),
        (":parallel", Some((InlineShellCommand::Parallel, None))),
        (":pa", Some((InlineShellCommand::Parallel, None))),
        (":peek", Some((InlineShellCommand::Peek, None))),
        (
            ":parallel off",
            Some((InlineShellCommand::Parallel, Some("off"))),
        ),
        (":pa off", Some((InlineShellCommand::Parallel, Some("off")))),
        (":PA OFF", Some((InlineShellCommand::Parallel, Some("OFF")))),
        (":DIAG", Some((InlineShellCommand::Diagnostics, None))),
        (":session", Some((InlineShellCommand::Sessions, None))),
        (":sessions", Some((InlineShellCommand::Sessions, None))),
        (":q", Some((InlineShellCommand::Queue, None))),
        (":queue", Some((InlineShellCommand::Queue, None))),
        (":directions", Some((InlineShellCommand::Directions, None))),
        (":task", None),
        (":task add a release checklist", None),
        (":turns 5", Some((InlineShellCommand::Turns, Some("5")))),
        (
            ":turns infinite",
            Some((InlineShellCommand::Turns, Some("infinite"))),
        ),
        (
            ":auto-turns 12",
            Some((InlineShellCommand::Turns, Some("12"))),
        ),
        (":turns", Some((InlineShellCommand::Turns, None))),
        (":stop", Some((InlineShellCommand::Stop, None))),
        (":model", Some((InlineShellCommand::Model, None))),
        (
            ":model gpt-5.4",
            Some((InlineShellCommand::Model, Some("gpt-5.4"))),
        ),
        (":view", Some((InlineShellCommand::View, None))),
        (
            ":view detail",
            Some((InlineShellCommand::View, Some("detail"))),
        ),
        (
            ":view midium",
            Some((InlineShellCommand::View, Some("midium"))),
        ),
        (
            ":think high",
            Some((InlineShellCommand::Think, Some("high"))),
        ),
        (
            ":think xhigh",
            Some((InlineShellCommand::Think, Some("xhigh"))),
        ),
        (":auto", None),
        (":automation", None),
        (":doctor", Some((InlineShellCommand::Doctor, None))),
        (":init", None),
        (":planning", Some((InlineShellCommand::PlanningInit, None))),
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
        (":new", Some((InlineShellCommand::NewDraft, None))),
        (":help", Some((InlineShellCommand::Help, None))),
        ("  :help  ", Some((InlineShellCommand::Help, None))),
        (":unknown", None),
    ];
    for (input, expected) in cases {
        let parsed = InlineShellCommandInput::parse(input)
            .map(|command| (command.command(), command.argument().map(str::to_string)));
        let expected = expected.map(|(command, argument)| (command, argument.map(str::to_string)));
        assert_eq!(parsed, expected, "{input}");
    }
}

#[test]
fn suggestions_show_all_commands_for_colon_only() {
    /*
    A lone colon opens the discoverability surface. The ordering here mirrors the
    command catalog rather than alphabetical sorting, preserving high-frequency
    operational commands before setup and help entries.
    */
    let suggestions = InlineShellCommand::suggestions(":");

    assert_eq!(
        suggestions,
        vec![
            InlineShellCommand::Diagnostics,
            InlineShellCommand::Parallel,
            InlineShellCommand::Peek,
            InlineShellCommand::Sessions,
            InlineShellCommand::Queue,
            InlineShellCommand::Directions,
            InlineShellCommand::Turns,
            InlineShellCommand::Stop,
            InlineShellCommand::Model,
            InlineShellCommand::View,
            InlineShellCommand::Think,
            InlineShellCommand::Doctor,
            InlineShellCommand::PlanningInit,
            InlineShellCommand::Reset,
            InlineShellCommand::NewDraft,
            InlineShellCommand::Help,
        ]
    );
}

#[test]
fn suggestions_filter_by_prefix() {
    /*
    Prefix filtering is intentionally command-name only. These assertions keep
    overlapping prefixes stable, especially :p and :t where multiple commands
    compete for the same first letter.
    */
    assert_eq!(
        InlineShellCommand::suggestions(":p"),
        vec![
            InlineShellCommand::Parallel,
            InlineShellCommand::Peek,
            InlineShellCommand::PlanningInit
        ]
    );
    assert_eq!(
        InlineShellCommand::suggestions(":pa"),
        vec![InlineShellCommand::Parallel]
    );
    assert_eq!(
        InlineShellCommand::suggestions(":q"),
        vec![InlineShellCommand::Queue]
    );
    assert_eq!(
        InlineShellCommand::suggestions(":do"),
        vec![InlineShellCommand::Doctor]
    );
    assert_eq!(InlineShellCommand::suggestions(":i"), Vec::new());
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
        vec![InlineShellCommand::Turns, InlineShellCommand::Think]
    );
    assert_eq!(
        InlineShellCommand::suggestions(":tu"),
        vec![InlineShellCommand::Turns]
    );
    assert_eq!(
        InlineShellCommand::suggestions(":th"),
        vec![InlineShellCommand::Think]
    );
    assert_eq!(
        InlineShellCommand::suggestions(":mo"),
        vec![InlineShellCommand::Model]
    );
    assert_eq!(
        InlineShellCommand::suggestions(":v"),
        vec![InlineShellCommand::View]
    );
}

#[test]
fn suggestion_prefix_only_stays_active_while_typing_command_name() {
    // Argument text disables palette filtering so values like task titles or reset
    // confirmations are not treated as command-name prefixes.
    assert_eq!(
        InlineShellCommand::suggestion_prefix(":planning"),
        Some(":planning".to_string())
    );
    assert_eq!(
        InlineShellCommand::suggestion_prefix("  :p"),
        Some(":p".to_string())
    );
    assert_eq!(InlineShellCommand::suggestion_prefix(":turns "), None);
    assert_eq!(
        InlineShellCommand::suggestion_prefix(":planning doctor"),
        None
    );
}

#[test]
fn palette_state_keeps_selected_command_when_input_refines() {
    /*
    Selection memory lets an operator move to :planning from the full menu and
    then type :p without losing the intended command to the first filtered item.
    */
    let mut state = InlineShellCommandPaletteState::default();
    state.sync_to_input(":", None);
    assert!(state.move_selection(12));
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
    /*
    Completion text is what gets inserted into the prompt, so commands that need
    arguments carry a trailing space while simple commands insert the canonical
    executable form. Aliases remain parse-only and never become completion text.
    */
    assert_eq!(InlineShellCommand::Diagnostics.completion_text(), ":diag");
    assert_eq!(
        InlineShellCommand::PlanningInit.completion_text(),
        ":planning"
    );
    assert_eq!(InlineShellCommand::Parallel.completion_text(), ":parallel");
    assert_eq!(InlineShellCommand::Peek.completion_text(), ":peek");
    assert_eq!(InlineShellCommand::Doctor.completion_text(), ":doctor");
    assert_eq!(InlineShellCommand::Turns.completion_text(), ":turns ");
    assert_eq!(InlineShellCommand::Stop.completion_text(), ":stop");
    assert_eq!(InlineShellCommand::Model.completion_text(), ":model");
    assert_eq!(InlineShellCommand::View.completion_text(), ":view");
    assert_eq!(InlineShellCommand::Think.completion_text(), ":think ");
    assert_eq!(InlineShellCommand::Reset.completion_text(), ":reset ");
}

#[test]
fn help_status_uses_short_overlay_status() {
    let help = InlineShellCommandInput::parse(":help").expect("help command should parse");

    assert_eq!(
        help.execution_status().as_deref(),
        Some("opened shell command help")
    );
}

#[test]
fn help_entries_use_renderable_command_forms() {
    /*
    Help entries are rendered in a compact overlay, not a raw alias dump. The
    tests keep broad parser aliases out of help copy while preserving argument
    grammar for commands whose execution depends on typed values.
    */
    let rendered = InlineShellCommand::help_entries()
        .iter()
        .map(|entry| format!("{} - {}", entry.usage, entry.detail))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains(":diag - diagnostics"));
    assert!(rendered.contains(":parallel [off] - parallel mode"));
    assert!(rendered.contains(":peek - parallel agent peek"));
    assert!(!rendered.lines().any(|line| line.starts_with(":pa ")));
    assert!(rendered.contains(":turns <number|infinite> - auto turn budget"));
    assert!(rendered.contains(":stop - stop active sessions"));
    assert!(rendered.contains(":model - model and think"));
    assert!(rendered.contains(":view [simple|medium|detail] - conversation view"));
    assert!(
        rendered.contains(":think <none|minimal|low|medium|high|xhigh|default> - reasoning effort")
    );
    assert!(!rendered.contains(":auto"));
    assert!(rendered.contains(":help - command help"));
    assert!(!rendered.contains(InlineShellCommand::command_list_line()));
}

// The following hint tests protect operator-facing copy at the point where a
// command is still buffered. Invalid arguments should explain the supported shape
// before the user commits the command with Enter.
#[test]
fn planning_command_hint_is_argument_aware() {
    let plain = InlineShellCommandInput::parse(":planning").expect("command should parse");
    let doctor = InlineShellCommandInput::parse(":planning doctor").expect("command should parse");
    let doctor_upper =
        InlineShellCommandInput::parse(":planning DOCTOR").expect("command should parse");
    let invalid = InlineShellCommandInput::parse(":planning status").expect("command should parse");
    let invalid_extra =
        InlineShellCommandInput::parse(":planning doctor now").expect("command should parse");

    assert_eq!(
        plain.buffered_hint(),
        "Press Enter to open the planning control center."
    );
    assert_eq!(
        doctor.buffered_hint(),
        "Press Enter to inspect planning health."
    );
    assert_eq!(
        doctor_upper.buffered_hint(),
        "Press Enter to inspect planning health."
    );
    assert_eq!(
        invalid.buffered_hint(),
        "Press Enter to apply `:planning status`. Supported arguments: doctor."
    );
    assert_eq!(
        invalid_extra.buffered_hint(),
        "Press Enter to apply `:planning doctor now`. Supported arguments: doctor."
    );
}

#[test]
fn directions_command_hint_is_argument_aware() {
    let plain = InlineShellCommandInput::parse(":directions").expect("command should parse");
    let invalid =
        InlineShellCommandInput::parse(":directions later").expect("command should parse");

    assert_eq!(
        plain.buffered_hint(),
        "Press Enter to review or edit planning directions."
    );
    assert_eq!(
        invalid.buffered_hint(),
        "Press Enter to apply `:directions later`. Supported command: :directions."
    );
}

#[test]
fn queue_command_hint_is_argument_aware() {
    let plain = InlineShellCommandInput::parse(":queue").expect("command should parse");
    let invalid = InlineShellCommandInput::parse(":queue later").expect("command should parse");

    assert_eq!(
        plain.buffered_hint(),
        "Press Enter to open the planning queue inspection."
    );
    assert_eq!(
        invalid.buffered_hint(),
        "`:queue` does not accept arguments (`later`); press Enter to open queue inspection."
    );
}

#[test]
fn parallel_command_hint_is_argument_aware() {
    let plain = InlineShellCommandInput::parse(":parallel").expect("command should parse");
    let off = InlineShellCommandInput::parse(":parallel off").expect("command should parse");
    let off_upper = InlineShellCommandInput::parse(":parallel OFF").expect("command should parse");
    let invalid = InlineShellCommandInput::parse(":parallel later").expect("command should parse");
    let invalid_extra =
        InlineShellCommandInput::parse(":parallel off now").expect("command should parse");

    assert_eq!(plain.buffered_hint(), "Press Enter to enter parallel mode.");
    assert_eq!(
        off.buffered_hint(),
        "Press Enter to turn parallel mode off."
    );
    assert_eq!(
        off_upper.buffered_hint(),
        "Press Enter to turn parallel mode off."
    );
    assert_eq!(
        invalid.buffered_hint(),
        "Press Enter to apply `:parallel later`. Supported command forms: :parallel, :pa, :parallel off, :pa off."
    );
    assert_eq!(
        invalid_extra.buffered_hint(),
        "Press Enter to apply `:parallel off now`. Supported command forms: :parallel, :pa, :parallel off, :pa off."
    );
}

#[test]
fn model_view_and_think_command_hints_are_argument_aware() {
    let model_plain = InlineShellCommandInput::parse(":model").expect("command should parse");
    let model_set = InlineShellCommandInput::parse(":model gpt-5.4").expect("command should parse");
    let model_clear =
        InlineShellCommandInput::parse(":model default").expect("command should parse");
    let model_invalid =
        InlineShellCommandInput::parse(":model gpt 5").expect("command should parse");
    let view_plain = InlineShellCommandInput::parse(":view").expect("command should parse");
    let view_medium = InlineShellCommandInput::parse(":view medium").expect("command should parse");
    let view_midium = InlineShellCommandInput::parse(":view midium").expect("command should parse");
    let view_detail = InlineShellCommandInput::parse(":view detail").expect("command should parse");
    let view_invalid = InlineShellCommandInput::parse(":view all").expect("command should parse");
    let think_plain = InlineShellCommandInput::parse(":think").expect("command should parse");
    let think_high = InlineShellCommandInput::parse(":think high").expect("command should parse");
    let think_xhigh =
        InlineShellCommandInput::parse(":think x_high").expect("command should parse");
    let think_clear =
        InlineShellCommandInput::parse(":think default").expect("command should parse");
    let think_invalid =
        InlineShellCommandInput::parse(":think fast").expect("command should parse");

    assert_eq!(
        model_plain.buffered_hint(),
        "Type `:model` to choose the model and think level, or `:model default` to use app-server defaults."
    );
    assert_eq!(
        model_set.buffered_hint(),
        "`:model` ignores typed model names; press Enter to open model selection."
    );
    assert_eq!(
        model_clear.buffered_hint(),
        "Press Enter to reset model to the app-server default."
    );
    assert_eq!(
        model_invalid.buffered_hint(),
        "`:model` ignores typed model names; press Enter to open model selection."
    );
    assert_eq!(
        view_plain.buffered_hint(),
        "Type `:view` to choose transcript visibility for tool/status rows."
    );
    assert_eq!(
        view_medium.buffered_hint(),
        "Press Enter to set conversation view to `medium`."
    );
    assert_eq!(
        view_midium.buffered_hint(),
        "Press Enter to set conversation view to `medium`."
    );
    assert_eq!(
        view_detail.buffered_hint(),
        "Press Enter to set conversation view to `detail`."
    );
    assert_eq!(
        view_invalid.buffered_hint(),
        "Press Enter to apply `:view all`. Supported values: simple, medium, detail."
    );
    assert_eq!(
        think_plain.buffered_hint(),
        "Type `:think <none|minimal|low|medium|high|xhigh|default>` to choose reasoning effort."
    );
    assert_eq!(
        think_high.buffered_hint(),
        "Press Enter to set think to `high`."
    );
    assert_eq!(
        think_xhigh.buffered_hint(),
        "Press Enter to set think to `xhigh`."
    );
    assert_eq!(
        think_clear.buffered_hint(),
        "Press Enter to reset think to the app-server default."
    );
    assert_eq!(
        think_invalid.buffered_hint(),
        "Press Enter to apply `:think fast`. Supported values: none, minimal, low, medium, high, xhigh, default."
    );
}

#[test]
fn doctor_command_hint_uses_lifecycle_language() {
    /*
    Doctor touches planning setup, but its hint must stay inspection-oriented.
    */
    let doctor = InlineShellCommandInput::parse(":doctor").expect("command should parse");

    assert_eq!(
        doctor.buffered_hint(),
        "Press Enter to inspect planning health."
    );
}

#[test]
fn reset_command_hint_is_argument_aware() {
    let plain = InlineShellCommandInput::parse(":reset").expect("command should parse");
    let queue = InlineShellCommandInput::parse(":reset queue").expect("command should parse");
    let directions =
        InlineShellCommandInput::parse(":reset directions").expect("command should parse");
    let directions_confirm =
        InlineShellCommandInput::parse(":reset directions confirm").expect("command should parse");
    let invalid = InlineShellCommandInput::parse(":reset wrong").expect("command should parse");
    let invalid_extra =
        InlineShellCommandInput::parse(":reset queue now").expect("command should parse");

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
    assert_eq!(
        invalid_extra.buffered_hint(),
        "Press Enter to apply `:reset queue now`. Supported arguments: queue, directions, all."
    );
}

#[test]
fn execution_status_stays_alias_neutral() {
    // Execution status is shown after dispatch, so it describes the action instead of
    // echoing whichever alias the operator typed.
    /*
    Commands with longer-running controller flows return no immediate status here;
    their handlers own follow-up copy once they inspect runtime state. The inline
    command layer only emits neutral statuses for instant overlay switches.
    */
    let cases = [
        (":diag", Some("opened diagnostics inspection")),
        (":sessions", Some("opened recent sessions inspection")),
        (":queue", Some("opened planning queue inspection")),
        (":doctor", None),
        (":planning", None),
        (":turns 5", None),
        (":stop", None),
        (":model gpt-5.4", None),
        (":view detail", None),
        (":think high", None),
        (":reset queue", None),
    ];
    for (input, expected) in cases {
        let command =
            InlineShellCommandInput::parse(input).expect("inline shell command should parse");
        assert_eq!(command.execution_status().as_deref(), expected);
    }
}
