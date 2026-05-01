use super::{
    InlineShellCommand, InlineShellCommandInput, InlineShellCommandPaletteState, RESET_USAGE,
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
        (":task", Some((InlineShellCommand::Task, None))),
        (
            ":task add a release checklist",
            Some((InlineShellCommand::Task, Some("add a release checklist"))),
        ),
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
        (":auto", None),
        (":automation", None),
        (":doctor", Some((InlineShellCommand::Doctor, None))),
        (":init", Some((InlineShellCommand::Init, None))),
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
    let suggestions = InlineShellCommand::suggestions(":");

    assert_eq!(
        suggestions,
        vec![
            InlineShellCommand::Diagnostics,
            InlineShellCommand::Parallel,
            InlineShellCommand::Sessions,
            InlineShellCommand::Queue,
            InlineShellCommand::Directions,
            InlineShellCommand::Task,
            InlineShellCommand::Turns,
            InlineShellCommand::Stop,
            InlineShellCommand::Doctor,
            InlineShellCommand::Init,
            InlineShellCommand::PlanningInit,
            InlineShellCommand::Reset,
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
        vec![InlineShellCommand::Task, InlineShellCommand::Turns]
    );
    assert_eq!(
        InlineShellCommand::suggestions(":tu"),
        vec![InlineShellCommand::Turns]
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
    assert_eq!(
        InlineShellCommand::suggestion_prefix(":planning doctor"),
        None
    );
}

#[test]
fn palette_state_keeps_selected_command_when_input_refines() {
    let mut state = InlineShellCommandPaletteState::default();
    state.sync_to_input(":", None);
    assert!(state.move_selection(10));
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
    assert_eq!(InlineShellCommand::Task.completion_text(), ":task");
    assert_eq!(InlineShellCommand::Turns.completion_text(), ":turns ");
    assert_eq!(InlineShellCommand::Stop.completion_text(), ":stop");
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
    let rendered = InlineShellCommand::help_entries()
        .iter()
        .map(|entry| format!("{} - {}", entry.usage, entry.detail))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains(":diag - diagnostics"));
    assert!(rendered.contains(":parallel [on|off|dispatch] - parallel mode"));
    assert!(rendered.contains(":turns <number|infinite> - auto turn budget"));
    assert!(rendered.contains(":stop - stop active sessions"));
    assert!(!rendered.contains(":auto"));
    assert!(rendered.contains(":help - command help"));
    assert!(!rendered.contains(InlineShellCommand::command_list_line()));
}

#[test]
fn planning_command_hint_is_argument_aware() {
    let plain = InlineShellCommandInput::parse(":planning").expect("command should parse");
    let doctor = InlineShellCommandInput::parse(":planning doctor").expect("command should parse");

    assert_eq!(
        plain.buffered_hint(),
        "Press Enter to open the planning control center."
    );
    assert_eq!(
        doctor.buffered_hint(),
        "Press Enter to inspect planning health."
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
    let on = InlineShellCommandInput::parse(":parallel on").expect("command should parse");
    let off = InlineShellCommandInput::parse(":parallel off").expect("command should parse");
    let dispatch =
        InlineShellCommandInput::parse(":parallel dispatch").expect("command should parse");
    let invalid = InlineShellCommandInput::parse(":parallel later").expect("command should parse");

    assert_eq!(
        plain.buffered_hint(),
        "Press Enter to inspect parallel mode readiness."
    );
    assert_eq!(
        on.buffered_hint(),
        "Press Enter to inspect readiness and enter parallel mode without dispatching."
    );
    assert_eq!(
        off.buffered_hint(),
        "Press Enter to turn parallel mode off."
    );
    assert_eq!(
        dispatch.buffered_hint(),
        "Press Enter to dispatch the current queue head to an agent slot."
    );
    assert_eq!(
        invalid.buffered_hint(),
        "Press Enter to apply `:parallel later`. Supported arguments: on, off, dispatch."
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
    let directions_confirm =
        InlineShellCommandInput::parse(":reset directions confirm").expect("command should parse");
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
        (":doctor", None),
        (":init", None),
        (":planning", None),
        (":task", None),
        (":turns 5", None),
        (":stop", None),
        (":reset queue", None),
    ];

    for (input, expected) in cases {
        let command =
            InlineShellCommandInput::parse(input).expect("inline shell command should parse");
        assert_eq!(command.execution_status().as_deref(), expected);
    }
}
