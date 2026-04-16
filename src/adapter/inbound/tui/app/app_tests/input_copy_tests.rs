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
    assert!(rendered.contains("Ctrl+j for newline"));
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
            .any(|line| line.contains("current state: ready"))
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("cause: this session is ready for the next prompt"))
    );
    assert!(rendered.iter().any(|line| line.contains(
        "next action: type the next prompt, use Ctrl+j for newline, then press Enter to send"
    )));
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("Operator commands: :diag"))
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
    assert!(rendered.contains("operator prompt: session ready"));
    assert!(rendered.contains("Ctrl+j newline"));
    assert!(rendered.contains(":help"));
    assert!(!rendered.contains(":help commands"));
    assert!(!rendered.contains("this session is ready for the next prompt"));
    assert!(!rendered.contains("Operator commands: :diag"));
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

    assert!(rendered.contains("startup: startup ready"));
    assert!(rendered.contains("workspace: /tmp/root"));
    assert!(rendered.contains("current state: ready"));
    assert!(rendered.contains("cause: codex, workspace, app-server, and account access are ready"));
    assert!(rendered.contains(
        "startup checks: codex ready  |  workspace ready  |  app-server ready  |  account ready"
    ));
    assert!(rendered.contains("conversation"));
    assert!(rendered.contains("first reply appears here after you send the opening prompt"));
    assert!(rendered.contains("starter: start with a task, file path, or bug summary"));
    assert!(rendered.contains("> "));
    assert!(rendered.contains("operator prompt: new thread ready"));
    assert!(rendered.contains("Ctrl+j newline"));
    assert!(rendered.contains(":help"));
    assert!(!rendered.contains(":help commands"));
    assert!(!rendered.contains("thread: new draft  |  turn: idle"));
}

#[test]
fn inline_tail_loading_conversation_uses_operator_state_language() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.conversation_state = ConversationState::Loading;

    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("thread: waiting"));
    assert!(rendered.contains("current state: waiting"));
    assert!(rendered.contains("cause: thread history is still loading from codex app-server"));
    assert!(rendered.contains("next action: wait for the thread history to load"));
    assert!(rendered.contains("operator prompt: waiting while thread history loads"));
}

#[test]
fn inline_tail_failed_conversation_uses_operator_state_language() {
    let (mut app, _) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    app.conversation_state = ConversationState::Failed("transport closed".to_string());

    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("thread: blocked"));
    assert!(rendered.contains("current state: blocked"));
    assert!(rendered.contains("cause: thread history is unavailable because loading failed"));
    assert!(rendered.contains("next action: reload the session or open a new draft"));
    assert!(rendered.contains("conversation error: transport closed"));
    assert!(
        rendered
            .contains("operator prompt: blocked until you reload the session or open a new draft")
    );
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
            .any(|line| line.contains("current state: ready"))
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("cause: a new thread draft is ready for the opening prompt"))
    );
    assert!(rendered.iter().any(|line| line.contains(
        "next action: type the first prompt, use Ctrl+j for newline, then press Enter to send"
    )));
}
