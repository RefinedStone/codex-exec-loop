// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::{
    InlineShellCommand, InlineShellCommandInput, InlineShellCommandPaletteState, RESET_USAGE,
};

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn parse_recognizes_supported_aliases() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
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

    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for (input, expected) in cases {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let parsed = InlineShellCommandInput::parse(input)
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .map(|command| (command.command(), command.argument().map(str::to_string)));
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let expected = expected.map(|(command, argument)| (command, argument.map(str::to_string)));
        assert_eq!(parsed, expected, "{input}");
    }
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn suggestions_show_all_commands_for_colon_only() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let suggestions = InlineShellCommand::suggestions(":");

    assert_eq!(
        suggestions,
        vec![
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            InlineShellCommand::Diagnostics,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            InlineShellCommand::Parallel,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            InlineShellCommand::Sessions,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            InlineShellCommand::Queue,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            InlineShellCommand::Directions,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            InlineShellCommand::Task,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            InlineShellCommand::Turns,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            InlineShellCommand::Stop,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            InlineShellCommand::Doctor,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            InlineShellCommand::Init,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            InlineShellCommand::PlanningInit,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            InlineShellCommand::Reset,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            InlineShellCommand::NewDraft,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            InlineShellCommand::Help,
        ]
    );
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn suggestions_filter_by_prefix() {
    assert_eq!(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        InlineShellCommand::suggestions(":p"),
        vec![
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            InlineShellCommand::Parallel,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            InlineShellCommand::PlanningInit
        ]
    );
    assert_eq!(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        InlineShellCommand::suggestions(":q"),
        vec![InlineShellCommand::Queue]
    );
    assert_eq!(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        InlineShellCommand::suggestions(":do"),
        vec![InlineShellCommand::Doctor]
    );
    assert_eq!(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        InlineShellCommand::suggestions(":i"),
        vec![InlineShellCommand::Init]
    );
    assert_eq!(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        InlineShellCommand::suggestions(":re"),
        vec![InlineShellCommand::Reset]
    );
    assert_eq!(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        InlineShellCommand::suggestions(":st"),
        vec![InlineShellCommand::Stop]
    );
    assert_eq!(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        InlineShellCommand::suggestions(":t"),
        vec![InlineShellCommand::Task, InlineShellCommand::Turns]
    );
    assert_eq!(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        InlineShellCommand::suggestions(":tu"),
        vec![InlineShellCommand::Turns]
    );
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn suggestion_prefix_only_stays_active_while_typing_command_name() {
    assert_eq!(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        InlineShellCommand::suggestion_prefix(":planning"),
        Some(":planning".to_string())
    );
    assert_eq!(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        InlineShellCommand::suggestion_prefix("  :p"),
        Some(":p".to_string())
    );
    assert_eq!(InlineShellCommand::suggestion_prefix(":turns "), None);
    assert_eq!(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        InlineShellCommand::suggestion_prefix(":planning doctor"),
        None
    );
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn palette_state_keeps_selected_command_when_input_refines() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
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

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn completion_text_uses_canonical_argument_ready_command_forms() {
    assert_eq!(InlineShellCommand::Diagnostics.completion_text(), ":diag");
    assert_eq!(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
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

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn help_status_uses_short_overlay_status() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let help = InlineShellCommandInput::parse(":help").expect("help command should parse");

    assert_eq!(
        help.execution_status().as_deref(),
        Some("opened shell command help")
    );
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn help_entries_use_renderable_command_forms() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let rendered = InlineShellCommand::help_entries()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .iter()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|entry| format!("{} - {}", entry.usage, entry.detail))
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .collect::<Vec<_>>()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .join("\n");

    assert!(rendered.contains(":diag - diagnostics"));
    assert!(rendered.contains(":parallel [on|off|dispatch] - parallel mode"));
    assert!(rendered.contains(":turns <number|infinite> - auto turn budget"));
    assert!(rendered.contains(":stop - stop active sessions"));
    assert!(!rendered.contains(":auto"));
    assert!(rendered.contains(":help - command help"));
    assert!(!rendered.contains(InlineShellCommand::command_list_line()));
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn planning_command_hint_is_argument_aware() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let plain = InlineShellCommandInput::parse(":planning").expect("command should parse");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
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

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn directions_command_hint_is_argument_aware() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let plain = InlineShellCommandInput::parse(":directions").expect("command should parse");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let invalid =
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
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

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn queue_command_hint_is_argument_aware() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let plain = InlineShellCommandInput::parse(":queue").expect("command should parse");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
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

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn parallel_command_hint_is_argument_aware() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let plain = InlineShellCommandInput::parse(":parallel").expect("command should parse");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let on = InlineShellCommandInput::parse(":parallel on").expect("command should parse");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let off = InlineShellCommandInput::parse(":parallel off").expect("command should parse");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let dispatch =
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        InlineShellCommandInput::parse(":parallel dispatch").expect("command should parse");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
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

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn doctor_and_init_command_hints_use_lifecycle_language() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let doctor = InlineShellCommandInput::parse(":doctor").expect("command should parse");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
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

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn reset_command_hint_is_argument_aware() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let plain = InlineShellCommandInput::parse(":reset").expect("command should parse");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let queue = InlineShellCommandInput::parse(":reset queue").expect("command should parse");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let directions =
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        InlineShellCommandInput::parse(":reset directions").expect("command should parse");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let directions_confirm =
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        InlineShellCommandInput::parse(":reset directions confirm").expect("command should parse");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
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

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn execution_status_stays_alias_neutral() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
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

    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for (input, expected) in cases {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let command =
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            InlineShellCommandInput::parse(input).expect("inline shell command should parse");
        assert_eq!(command.execution_status().as_deref(), expected);
    }
}
