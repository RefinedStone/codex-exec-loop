use super::{
    ConversationInputState, ConversationState, ShellActionAvailability, StartupState,
    build_inline_tail_lines, build_ready_input_lines, make_test_app, ready_conversation,
    sample_startup_diagnostics,
};

#[test]
fn running_turn_still_shows_buffered_prompt() {
    let mut conversation = ready_conversation();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.input_buffer = "Continue from the last change.".to_string();

    let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("Continue from the last change."));
    assert!(rendered.contains("Ctrl+j inserts a new line"));
}
#[test]
fn empty_existing_session_prompts_for_next_message() {
    let conversation = ready_conversation();

    let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line == "> "));
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("Ready to continue this session."))
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("Ctrl+j for newline"))
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("Shell commands: :diag"))
    );
}
#[test]
fn inline_tail_compacts_empty_session_prompt_copy() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.conversation_state = ConversationState::ready(ready_conversation());

    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("> "));
    assert!(rendered.contains("prompt: session ready"));
    assert!(rendered.contains("Ctrl+j nl"));
    assert!(rendered.contains(":help"));
    assert!(!rendered.contains(":help commands"));
    assert!(!rendered.contains("Ready to continue this session."));
    assert!(!rendered.contains("Shell commands: :diag"));
}

#[test]
fn inline_tail_surfaces_interrupt_truth_while_turn_runs() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let mut conversation = ready_conversation();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-1".to_string());
    conversation.active_turn_started_at = Some(std::time::Instant::now());
    app.conversation_state = ConversationState::ready(conversation);

    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("interrupt unsupported"));
}
#[test]
fn inline_tail_compacts_empty_draft_prompt_copy() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let mut conversation = ready_conversation();
    conversation.thread_id.clear();
    conversation.input_state = ConversationInputState::DraftReady;
    app.conversation_state = ConversationState::ready(conversation);

    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("Akra  |  Workflows: ready"));
    assert!(rendered.contains("workspace: /tmp/root"));
    assert!(rendered.contains("diagnostics: codex ok  |  app-server ok  |  account ok"));
    assert!(rendered.contains("conversation"));
    assert!(rendered.contains("first reply appears here after you send the opening prompt"));
    assert!(rendered.contains("starter: start with a task, file path, or bug summary"));
    assert!(rendered.contains("> "));
    assert!(rendered.contains("prompt: new thread ready"));
    assert!(rendered.contains("Ctrl+j nl"));
    assert!(rendered.contains(":help"));
    assert!(!rendered.contains(":help commands"));
    assert!(!rendered.contains("thread: new draft  |  turn: idle"));
}

#[test]
fn inline_tail_uses_compact_thread_title_instead_of_full_thread_id() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let mut conversation = ready_conversation();
    conversation.thread_id = "019d6e93-818a-7661-9e0d-7dec23c4b84d".to_string();
    conversation.title = "Untitled thread".to_string();
    app.conversation_state = ConversationState::ready(conversation);

    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("thread: Untitled thread"));
    assert!(!rendered.contains("019d6e93-818a-7661-9e0d-7dec23c4b84d"));
}

#[test]
fn inline_tail_command_palette_surfaces_real_keys() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let mut conversation = ready_conversation();
    conversation.input_buffer = ":q".to_string();
    conversation.sync_inline_shell_command_palette();
    app.conversation_state = ConversationState::ready(conversation);

    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("> :q"));
    assert!(rendered.contains("command: palette"));
    assert!(rendered.contains("Up/Down move"));
    assert!(rendered.contains("Enter choose"));
    assert!(rendered.contains("Esc close"));
    assert!(rendered.contains(":queue"));
}

#[test]
fn inline_tail_preserves_korean_wide_char_prompt() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let mut conversation = ready_conversation();
    conversation.input_buffer = "다음 작업 정리\n테스트 확인".to_string();
    app.conversation_state = ConversationState::ready(conversation);

    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("> 다음 작업 정리"));
    assert!(rendered.contains("  테스트 확인"));
    assert!(rendered.contains("buffered prompt"));
    assert!(!rendered.contains('\u{fffd}'));
}

#[test]
fn empty_draft_prompts_for_first_message() {
    let mut conversation = ready_conversation();
    conversation.thread_id.clear();
    conversation.input_state = ConversationInputState::DraftReady;

    let rendered = build_ready_input_lines(&conversation, ShellActionAvailability::Ready)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line == "> "));
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("Ready to start a new thread."))
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("Ctrl+j for newline"))
    );
}
