use super::super::tui_testkit;
use super::*;
use crate::adapter::inbound::tui::app::shell_presentation::{
    build_reviews_overlay_view, format_conversation_lines_for_view,
    format_conversation_lines_with_debug,
};
use crate::adapter::inbound::tui::app::shell_runtime::ShellRuntime;
use crate::adapter::inbound::tui::app::test_helpers::{
    sample_planning_runtime_projection, test_native_tui_app_with_review_center_repository,
    test_native_tui_app_with_session_catalog_port,
};
use crate::application::port::outbound::review_center_repository_port::{
    ReviewCenterHistoryEntry, ReviewCenterInboxItem, ReviewCenterRepositoryPort,
    ReviewCenterThreadProjection,
};
use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::domain::conversation::ConversationSnapshot;
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterEntry, ParallelModeAgentRosterSnapshot,
    ParallelModeAgentSessionDetailSnapshot, ParallelModeAgentSessionHistoryEntry,
    ParallelModeCompletionFeedEntry, ParallelModeDistributorQueueItem,
    ParallelModeDistributorSnapshot, ParallelModePoolBoardSnapshot, ParallelModePoolSlotSnapshot,
    ParallelModePoolSlotState, ParallelModeQueueItemState, ParallelModeReadinessState,
    ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorSnapshot,
    ParallelModeSupervisorState,
};
use crate::domain::recent_sessions::{
    RecentSessions, SessionCatalog, SessionCatalogRequest, SessionCatalogTier,
};
use ratatui::Terminal;
use ratatui::backend::{Backend, TestBackend};
use ratatui::layout::Position;
use ratatui::style::Color;
use ratatui::widgets::ListState;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

// Rendering contract tests use TestBackend snapshots instead of golden files so
// each assertion can name the specific TUI invariant it protects.
#[path = "shell_rendering_contract_tests/fixtures.rs"]
mod fixtures;
#[path = "shell_rendering_contract_tests/planning.rs"]
mod planning;

pub(super) use self::fixtures::{
    make_test_app, make_test_app_with_planning, sample_parallel_mode_snapshot,
    sample_planning_editor_session, sample_session, sample_startup_diagnostics,
};

#[derive(Default)]
struct CountingSessionCatalogPort {
    load_count: AtomicUsize,
}

impl SessionCatalogPort for CountingSessionCatalogPort {
    fn load_session_catalog(
        &self,
        _request: SessionCatalogRequest,
    ) -> anyhow::Result<SessionCatalog> {
        self.load_count.fetch_add(1, Ordering::SeqCst);
        Ok(RecentSessions {
            items: Vec::new(),
            warnings: Vec::new(),
            next_cursor: None,
        }
        .into())
    }
}

// Transcript formatting tests protect text-level contracts that feed both the
// bordered TUI and the inline main-buffer renderer.
#[test]
fn centered_rect_clamps_percentages_above_hundred() {
    let area = Rect::new(4, 2, 80, 24);

    assert_eq!(centered_rect(140, 120, area), area);
}
#[test]
fn transcript_debug_detail_is_rendered_in_gray_only_when_enabled() {
    let message = ConversationMessage::new(
        ConversationMessageKind::User,
        "다음 queued-task 1개를 이어서 진행합니다.",
        None,
        None,
    )
    .with_display_label("Auto Follow-up")
    .with_debug_detail("planning worker temporary session: refresh / refresh ok");
    let without_debug = format_conversation_lines(std::slice::from_ref(&message));
    assert!(!without_debug.iter().any(|line| {
        line.to_string()
            .contains("planning worker temporary session")
    }));
    let with_debug = format_conversation_lines_with_debug(&[message], true);
    let detail_line = with_debug
        .iter()
        .find(|line: &&Line<'static>| {
            line.to_string()
                .contains("planning worker temporary session")
        })
        .expect("debug transcript should include the planning worker detail line");

    assert_eq!(
        detail_line.to_string(),
        "  planning worker temporary session: refresh / refresh ok"
    );
    assert_eq!(detail_line.spans[0].style.fg, Some(Color::Gray));
}
#[test]
fn transcript_formatting_expands_tabs_in_content_and_debug_detail() {
    let message = ConversationMessage::new(
        ConversationMessageKind::Agent,
        "let\tok = true;",
        Some("final_answer".to_string()),
        None,
    )
    .with_debug_detail("phase\tfinal");
    let lines = format_conversation_lines_with_debug(&[message], true)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    assert!(lines.contains(&"  let    ok = true;".to_string()));
    assert!(lines.contains(&"  phase    final".to_string()));
}
#[test]
fn transcript_view_modes_filter_tool_and_status_rows() {
    let messages = vec![
        ConversationMessage::new(ConversationMessageKind::User, "hello", None, None),
        ConversationMessage::new(
            ConversationMessageKind::Agent,
            "thinking",
            Some("commentary".to_string()),
            None,
        ),
        ConversationMessage::new(ConversationMessageKind::Agent, "answer", None, None),
        ConversationMessage::new(
            ConversationMessageKind::Tool,
            "command: cargo test",
            None,
            None,
        ),
        ConversationMessage::new(
            ConversationMessageKind::Status,
            "thread status: running",
            None,
            None,
        ),
    ];

    let default = format_conversation_lines(&messages)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(default.contains("◆ ") && default.contains("tool"));
    assert!(default.contains("Status:"));

    let simple = format_conversation_lines_for_view(&messages, ConversationViewMode::Simple, false)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(simple.contains("You:"));
    assert!(simple.contains("Codex Commentary:"));
    assert!(simple.contains("Codex:"));
    assert!(!simple.contains("◆ "));
    assert!(!simple.contains("Status:"));

    let medium = format_conversation_lines_for_view(&messages, ConversationViewMode::Medium, false)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(medium.contains("◆ ") && medium.contains("tool"));
    assert!(medium.contains("Status:"));

    let only_hidden_messages = messages[3..].to_vec();
    let simple_empty = format_conversation_lines_for_view(
        &only_hidden_messages,
        ConversationViewMode::Simple,
        false,
    )
    .into_iter()
    .map(|line| line.to_string())
    .collect::<Vec<_>>()
    .join("\n");
    assert!(simple_empty.contains("No messages visible in simple view."));
    assert!(!simple_empty.contains("No messages in this thread yet."));
}

// Inline main-buffer tests keep the app-server-first mode frameless: the stable
// transcript stays outside the alternate-screen tail, and live rows are cleared
// when state changes.
#[test]
fn inline_main_buffer_rendering_uses_only_the_composer_focus_frame() {
    let mut terminal = tui_testkit::inline_terminal(80, 24);
    let mut app = make_test_app();
    tui_testkit::append_agent_history_message(
        &mut app,
        "stable history should stay above the live region",
    );

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(!rendered.contains("Shell / Ctrl+t new draft"));
    assert!(!rendered.contains("Transcript /"));
    assert!(!rendered.contains("Controls / shell shortcuts and live status"));
    assert!(!rendered.contains("Prompt / ready"));
    assert!(rendered.contains("Akra / root"));
    assert!(!rendered.contains("turn: idle"));
    assert!(!rendered.contains("input: draft"));
    assert!(!rendered.contains("auto: queue/idle"));
    assert!(!rendered.contains("done: off"));
    assert!(!rendered.contains("Plan ready"));
    assert!(!rendered.contains("parallel: off"));
    assert!(!rendered.contains("stable history should stay above the live region"));
    assert!(!rendered.contains("No messages in this thread yet."));
    let compact_hud = rendered
        .lines()
        .filter(|line| line.contains("Akra / root"))
        .collect::<Vec<_>>();
    assert_eq!(compact_hud.len(), 1, "{rendered}");
    assert!(compact_hud[0].contains("pending"), "{rendered}");
    assert!(compact_hud[0].contains("queue: off"), "{rendered}");
    assert!(!compact_hud[0].contains("branch:"), "{rendered}");
    assert_eq!(rendered.matches("╭ Task").count(), 1, "{rendered}");
    assert_eq!(rendered.matches('│').count(), 1, "{rendered}");
    assert_eq!(rendered.matches('╰').count(), 1, "{rendered}");
}
#[test]
fn inline_main_buffer_tail_anchors_below_transcript_area_after_history() {
    let mut terminal = tui_testkit::inline_terminal(80, 24);
    let mut app = make_test_app();
    tui_testkit::append_agent_history_message(&mut app, "latest reply should stay in scrollback");

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);
    let rendered_lines = rendered.lines().collect::<Vec<_>>();
    let thread_line_index = rendered_lines
        .iter()
        .position(|line| line.trim_matches('"').starts_with("Akra / root"))
        .expect("inline viewport should contain visible tail text");
    assert!(
        thread_line_index > 0,
        "tail should leave a transcript-live area above it:\n{rendered}"
    );
    assert!(
        rendered_lines[..thread_line_index]
            .iter()
            .all(|line| line.trim_matches('"').trim().is_empty()),
        "tail should not leave stale transcript text above it:\n{rendered}"
    );
}
#[test]
fn inline_main_buffer_tail_frame_does_not_render_startup_ascii_art_transiently() {
    /*
     * Inline mode uses the main terminal buffer, so a transient ASCII banner would
     * become permanent host scrollback noise. The fixture turns the flag on to
     * prove readiness copy replaces the banner before the first prompt frame.
     */
    let mut terminal = tui_testkit::inline_terminal(80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = true;
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(!rendered.contains(".:  .::    .::  .::.: .:::   .::"));
    assert!(!rendered.contains(".::.::  .::   .::    .::  .::   .::"));
    assert!(rendered.contains("Akra / root"));
    assert!(!rendered.contains("branch: --"));
    assert!(!rendered.contains("ctx: --"));
    assert!(rendered.contains("queue: off"));
    assert!(rendered.contains("Describe a task or type : for commands"));
    assert!(rendered.contains("Type a task  |  : commands"));
    assert!(!rendered.contains("diagnostics:"));
    assert!(!rendered.contains("attachment:"));
    assert!(!rendered.contains("shortcuts:"));
}

#[test]
fn inline_startup_screen_uses_selected_korean_language() {
    let mut terminal = tui_testkit::inline_terminal(80, 24);
    let mut app = make_test_app();
    app.shell.tui_language = TuiLanguage::Korean;
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline startup render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Akra / root"));
    assert!(rendered.contains("준비됨"));
    assert!(rendered.contains("작업을 입력하거나 : 명령을 사용하세요"));
    assert!(rendered.contains("작업 입력  |  : 명령"));
    assert!(!rendered.contains("진단:"));
    assert!(!rendered.contains("단축키:"));
}
#[test]
fn startup_prompt_command_palette_remains_visible_after_colon_input() {
    let mut terminal = tui_testkit::inline_terminal(80, 10);
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should start with a ready draft conversation");
    };
    conversation.composer.input_buffer = ":".to_string();
    conversation.composer.sync_inline_shell_command_palette();

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("> :"));
    assert!(rendered.contains("palette 1/19"));
    assert!(rendered.contains("↑/↓ or Tab select"));
    assert!(rendered.contains(":diag"));
    assert!(rendered.contains(":peek"));

    let mut narrow_terminal = tui_testkit::inline_terminal(48, 10);
    narrow_terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("narrow inline render succeeds");
    let narrow = tui_testkit::screen_text(&narrow_terminal);
    assert!(narrow.contains("palette 1/19"), "{narrow}");
    assert!(narrow.contains("> :diag"), "{narrow}");
    assert!(narrow.contains("↑/↓ or Tab select"), "{narrow}");
}

#[test]
fn startup_prompt_command_palette_uses_selected_korean_language() {
    let mut terminal = tui_testkit::inline_terminal(48, 10);
    let mut app = make_test_app();
    app.shell.tui_language = TuiLanguage::Korean;
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should start with a ready draft conversation");
    };
    conversation.composer.input_buffer = ":".to_string();
    conversation.composer.sync_inline_shell_command_palette();

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("팔레트 1/19"));
    assert!(rendered.contains(":diag  진단"));
    assert!(rendered.contains("↑/↓ 또는 Tab 선택"));

    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready draft conversation");
    };
    conversation.composer.input_buffer = ":zzzz".to_string();
    conversation.composer.sync_inline_shell_command_palette();
    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("no-match palette render succeeds");
    let no_match = tui_testkit::screen_text(&terminal);
    assert!(no_match.contains("팔레트 0/0"), "{no_match}");
    assert!(no_match.contains("Esc 닫기"), "{no_match}");
    assert!(!no_match.contains("Enter 선택"), "{no_match}");
    assert!(!no_match.contains("Down/Tab 다음"), "{no_match}");

    let mut fresh_terminal = tui_testkit::inline_terminal(48, 10);
    fresh_terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("fresh no-match palette render succeeds");
    let fresh_no_match = tui_testkit::screen_text(&fresh_terminal);
    assert!(
        fresh_no_match.contains("일치하는 셸 명령이 없습니다"),
        "{fresh_no_match}"
    );
}
#[test]
fn inline_main_buffer_clears_stale_live_tail_rows_after_turn_finishes() {
    /*
     * TestBackend retains previous cells unless the renderer clears them. A live
     * agent row is rendered first, then removed from state, so the second frame
     * must actively blank the old row instead of relying on shorter replacement text.
     */
    let mut terminal = tui_testkit::inline_terminal(80, 24);
    let mut app = make_test_app();
    tui_testkit::set_live_agent_message(&mut app, "ghost line should disappear");

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("first inline render succeeds");
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should stay in a ready conversation state");
    };
    conversation.live_agent_message = None;
    let mut snapshot = conversation.runtime_snapshot().clone();
    snapshot.active_turn = None;
    conversation.apply_runtime_snapshot(snapshot);

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("second inline render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(!rendered.contains("ghost line should disappear"));
}
#[test]
fn inline_main_buffer_clears_stale_tail_rows_when_overlay_opens() {
    let mut terminal = tui_testkit::inline_terminal(80, 24);
    let mut app = make_test_app();
    tui_testkit::set_live_agent_message(&mut app, "overlay ghost line should disappear");

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("first inline render succeeds");
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should stay in a ready conversation state");
    };
    conversation.live_agent_message = None;
    let mut snapshot = conversation.runtime_snapshot().clone();
    snapshot.active_turn = None;
    conversation.apply_runtime_snapshot(snapshot);
    app.shell.chrome.shell_overlay = ShellOverlay::Startup;
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("overlay inline render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(!rendered.contains("overlay ghost line should disappear"));
}
#[test]
fn inline_render_positions_cursor_on_empty_prompt_line() {
    let mut terminal = tui_testkit::inline_terminal(80, 24);
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline render succeeds");

    terminal
        .backend_mut()
        .assert_cursor_position(Position::new(4, 2));
}

#[test]
fn focused_composer_cursor_uses_terminal_cells_for_korean_and_wide_graphemes() {
    use unicode_width::UnicodeWidthStr;

    let mut terminal = tui_testkit::inline_terminal(80, 24);
    let mut app = make_test_app();
    app.shell.tui_language = TuiLanguage::Korean;
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let input = "한글 👩‍💻 작업";
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready conversation state");
    };
    conversation.composer.input_buffer = input.to_string();
    conversation
        .composer
        .set_input_cursor_byte_index(conversation.composer.input_buffer.len());

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("wide prompt render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);
    let cursor = terminal
        .backend_mut()
        .get_cursor_position()
        .expect("cursor position should be available");
    let (prompt_row, prompt_line) = rendered
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains(input))
        .expect("wide prompt should be visible");
    let prompt_line = prompt_line.trim_matches('"');
    let input_end = prompt_line
        .find(input)
        .expect("wide prompt should have a start")
        .saturating_add(input.len());
    let expected_x = UnicodeWidthStr::width(&prompt_line[..input_end]);

    assert_eq!(
        cursor,
        Position::new(expected_x as u16, prompt_row as u16),
        "cursor should use terminal cells rather than UTF-8 byte length"
    );
    assert!(rendered.contains("Enter 전송  |  Ctrl+J 줄바꿈"));
}

#[test]
fn dense_single_line_prompt_keeps_its_end_and_cursor_visible() {
    let mut terminal = Terminal::new(TestBackend::new(48, 18)).expect("test terminal");
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    tui_testkit::append_agent_history_message(&mut app, "prompt overflow baseline");
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready conversation state");
    };
    let before_middle = "word ".repeat(120);
    conversation.composer.input_buffer = format!(
        "{before_middle}MIDDLE_CURSOR {}CURSOR_END",
        "word ".repeat(120)
    );

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("long prompt render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);
    let cursor = terminal
        .backend_mut()
        .get_cursor_position()
        .expect("cursor position should be available");

    assert!(rendered.contains("CURSOR_END"), "{rendered}");
    assert!(
        rendered.contains("Enter send  |  Ctrl+J newline"),
        "{rendered}"
    );
    let (cursor_text_row, cursor_text_line) = rendered
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains("CURSOR_END"))
        .expect("prompt suffix row should be visible");
    let cursor_text_line = cursor_text_line.trim_matches('"');
    let cursor_marker_end = cursor_text_line
        .find("CURSOR_END")
        .expect("cursor marker should have a column")
        .saturating_add("CURSOR_END".len());
    let expected_cursor_x =
        unicode_width::UnicodeWidthStr::width(&cursor_text_line[..cursor_marker_end]);
    assert_eq!(
        cursor,
        Position::new(expected_cursor_x as u16, cursor_text_row as u16),
        "cursor should follow the word-wrapped prompt suffix:\n{rendered}"
    );

    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready conversation state");
    };
    conversation
        .composer
        .set_input_cursor_byte_index(before_middle.len());
    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("middle cursor render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);
    let cursor = terminal
        .backend_mut()
        .get_cursor_position()
        .expect("middle cursor position should be available");
    let (middle_row, middle_line) = rendered
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains("MIDDLE_CURSOR"))
        .expect("focused middle prompt row should be visible");
    let middle_line = middle_line.trim_matches('"');
    let middle_x = unicode_width::UnicodeWidthStr::width(
        &middle_line[..middle_line
            .find("MIDDLE_CURSOR")
            .expect("middle cursor marker should have a column")],
    );
    assert_eq!(
        cursor,
        Position::new(middle_x as u16, middle_row as u16),
        "cursor should follow a word-wrapped middle edit:\n{rendered}"
    );
}
#[test]
fn inline_queue_overlay_rendering_shows_compact_sections() {
    let mut terminal = tui_testkit::inline_terminal(80, 24);
    let mut app = make_test_app();
    tui_testkit::append_agent_history_message(
        &mut app,
        "stable history stays visible above the queue",
    );
    app.sync_ready_conversation_planning_runtime_projection(
        sample_planning_runtime_projection("Planning Context", "Queue Summary")
            .with_planning_revision(Some(1)),
    );
    app.shell.chrome.shell_overlay = ShellOverlay::Queue;
    app.bind_queue_overlay_authority_for_test(
        1,
        std::collections::BTreeMap::from([
            (
                "task-1".to_string(),
                queue_overlay_ui::QueueOverlayAuthorityToken {
                    status: crate::domain::planning::TaskStatus::Ready,
                    updated_at: "2026-04-10T00:00:00Z".to_string(),
                },
            ),
            (
                "task-2".to_string(),
                queue_overlay_ui::QueueOverlayAuthorityToken {
                    status: crate::domain::planning::TaskStatus::Ready,
                    updated_at: "2026-04-10T01:00:00Z".to_string(),
                },
            ),
        ]),
    );

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("queue render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Planning Queue"));
    assert!(rendered.contains("queued:"));
    assert!(rendered.contains("x/Delete: remove"));
    assert!(!rendered.contains("Review the next actionable work"));
}

// Inline inspection overlays replace the transcript region rather than drawing
// popup frames; these tests pin each overlay family to that composition.
#[test]
fn inline_startup_inspection_replaces_transcript_panel() {
    /*
     * Inline inspections are not popups: they occupy the transcript region inside
     * the shell flow. The negative border/header assertions keep this path from
     * accidentally regressing to alternate-screen popup chrome.
     */
    let mut terminal = Terminal::new(TestBackend::new(96, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.shell.chrome.shell_overlay = ShellOverlay::Startup;

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline inspection render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Diagnostics / inline inspection"));
    assert!(rendered.contains("Checks"));
    assert!(rendered.contains("schema snapshot: snapshot.json"));
    assert!(!rendered.contains("shell inspection"));
    assert!(!rendered.contains("Transcript /"));
    assert!(!rendered.contains("┌"));
}
#[test]
fn inline_sessions_inspection_renders_browser_panels() {
    let mut terminal = Terminal::new(TestBackend::new(96, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.shell.chrome.session_state = SessionState::Ready(
        RecentSessions {
            items: vec![sample_session("thread-1"), sample_session("thread-2")],
            warnings: vec!["cache is stale".to_string()],
            next_cursor: None,
        }
        .into(),
    );
    app.shell.chrome.shell_overlay = ShellOverlay::Sessions;

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline session inspection render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Recent Sessions / inline inspection"));
    assert!(rendered.contains("Threads"));
    assert!(rendered.contains("Selected Session"));
    assert!(rendered.contains("Session Warnings"));
    assert!(!rendered.contains("shell inspection"));
    assert!(!rendered.contains("Transcript /"));
    assert!(!rendered.contains("┌"));
}
#[test]
fn repeated_session_overlay_draws_do_not_load_the_catalog() {
    let session_port = Arc::new(CountingSessionCatalogPort::default());
    let mut terminal = Terminal::new(TestBackend::new(96, 28)).expect("test terminal");
    let mut app = test_native_tui_app_with_session_catalog_port(session_port.clone());
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.shell.chrome.session_state = SessionState::Ready(
        RecentSessions {
            items: vec![sample_session("thread-1")],
            warnings: Vec::new(),
            next_cursor: None,
        }
        .into(),
    );
    app.shell.chrome.shell_overlay = ShellOverlay::Sessions;

    for _ in 0..2 {
        terminal
            .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
            .expect("inline session redraw succeeds");
    }

    assert_eq!(
        session_port.load_count.load(Ordering::SeqCst),
        0,
        "capturing and redrawing the session screen model must not load the catalog"
    );
}
#[test]
fn inline_sessions_inspection_surfaces_attach_only_catalog_without_browser_navigation() {
    /*
     * Attach-only catalogs have enough identity to reattach by handle but not
     * enough metadata for browser navigation. Rendering this tier keeps the
     * session overlay honest about what controls are available.
     */
    let mut terminal = Terminal::new(TestBackend::new(96, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.shell.chrome.session_state = SessionState::Ready(SessionCatalog::unsupported(
        SessionCatalogTier::AttachOnly,
        "session listing is unsupported for this bridge",
        vec!["manual attach only".to_string()],
    ));
    let existing_list_state = ListState::default().with_offset(4).with_selected(Some(5));
    app.shell.session_overlay_ui_state.list_state = existing_list_state;
    app.shell.chrome.shell_overlay = ShellOverlay::Sessions;

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline attach-only session inspection render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("catalog tier: attach-only"));
    assert!(rendered.contains("session listing is unsupported"));
    assert!(rendered.contains("manual attach only"));
    assert!(rendered.contains("Recent-session navigation requires a queryable catalog surface."));
    assert_eq!(
        app.shell.session_overlay_ui_state.list_state, existing_list_state,
        "message-only session catalogs must not rewrite list selection or offset"
    );
}
#[test]
fn inline_help_inspection_renders_command_help() {
    let mut terminal = Terminal::new(TestBackend::new(96, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.shell.chrome.shell_overlay = ShellOverlay::Help;

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline help inspection render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Shell Commands / inline inspection"));
    assert!(rendered.contains(":diag"));
    assert!(rendered.contains("diagnostics"));
    assert!(rendered.contains(":turns"));
    assert!(rendered.contains("auto-follow opt-in; off or 0 disables"));
    assert!(!rendered.contains(":auto"));
    assert!(rendered.contains("PgUp/PgDn: page"));
    assert!(!rendered.contains("Shell commands: :diag  :parallel"));
    assert!(!rendered.contains("Transcript /"));
    assert!(!rendered.contains("┌"));
}

#[test]
fn inline_help_inspection_uses_selected_korean_language() {
    let mut terminal = Terminal::new(TestBackend::new(96, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.shell.tui_language = TuiLanguage::Korean;
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.shell.chrome.shell_overlay = ShellOverlay::Help;

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline help inspection render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("셸 명령 / 인라인 보기"));
    assert!(rendered.contains(":diag"));
    assert!(rendered.contains("진단"));
    assert!(rendered.contains("PgUp/PgDn: 페이지"));
}

#[test]
fn narrow_help_inspection_scrolls_to_the_last_command() {
    let mut terminal = Terminal::new(TestBackend::new(48, 18)).expect("test terminal");
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.shell.chrome.shell_overlay = ShellOverlay::Help;
    app.shell.help_scroll_offset = usize::MAX;

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("narrow help render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert_ne!(app.shell.help_scroll_offset, usize::MAX);
    assert!(
        rendered.contains("command help"),
        "last command detail must be reachable:\n{rendered}"
    );
    assert!(
        rendered.contains("Esc/Ctrl+C: close"),
        "help close action must remain visible:\n{rendered}"
    );
    assert!(
        rendered.contains("Describe a task or type : for commands"),
        "composer tail must remain visible:\n{rendered}"
    );
}

#[derive(Default)]
struct CountingReviewCenterRepository {
    thread_loads: AtomicUsize,
    inbox_loads: AtomicUsize,
    history_loads: AtomicUsize,
    expected_workspace: Option<String>,
    expected_thread_id: Option<String>,
}

impl CountingReviewCenterRepository {
    fn for_context(workspace_directory: impl Into<String>, thread_id: impl Into<String>) -> Self {
        Self {
            expected_workspace: Some(workspace_directory.into()),
            expected_thread_id: Some(thread_id.into()),
            ..Self::default()
        }
    }

    fn load_counts(&self) -> (usize, usize, usize) {
        (
            self.thread_loads.load(Ordering::SeqCst),
            self.inbox_loads.load(Ordering::SeqCst),
            self.history_loads.load(Ordering::SeqCst),
        )
    }

    fn workspace_label(&self, workspace_dir: &str) -> &'static str {
        match self.expected_workspace.as_deref() {
            Some(expected) if expected != workspace_dir => "wrong",
            Some(_) | None => "correct",
        }
    }

    fn thread_label(&self, thread_id: &str) -> &'static str {
        match self.expected_thread_id.as_deref() {
            Some(expected) if expected != thread_id => "wrong",
            Some(_) | None => "correct",
        }
    }
}

impl ReviewCenterRepositoryPort for CountingReviewCenterRepository {
    fn load_thread_reviews(
        &self,
        workspace_dir: &str,
        thread_id: &str,
    ) -> anyhow::Result<Vec<ReviewCenterThreadProjection>> {
        self.thread_loads.fetch_add(1, Ordering::SeqCst);
        Ok(vec![ReviewCenterThreadProjection::new(
            thread_id,
            "review-1",
            "Architecture review",
            "pending",
            format!(
                "{} workspace {} thread review",
                self.workspace_label(workspace_dir),
                self.thread_label(thread_id)
            ),
            "2026-07-16T10:00:00Z",
            "2026-07-16T10:01:00Z",
        )])
    }

    fn load_pending_inbox(
        &self,
        workspace_dir: &str,
    ) -> anyhow::Result<Vec<ReviewCenterInboxItem>> {
        self.inbox_loads.fetch_add(1, Ordering::SeqCst);
        Ok(vec![ReviewCenterInboxItem::new(
            "review-1",
            "thread-1",
            "pending",
            format!("{} workspace inbox", self.workspace_label(workspace_dir)),
            "2026-07-16T10:00:00Z",
            "2026-07-16T10:01:00Z",
        )])
    }

    fn load_recent_history(
        &self,
        workspace_dir: &str,
    ) -> anyhow::Result<Vec<ReviewCenterHistoryEntry>> {
        self.history_loads.fetch_add(1, Ordering::SeqCst);
        Ok(vec![ReviewCenterHistoryEntry::new(
            "review-1",
            "thread-1",
            "review_requested",
            format!("{} workspace history", self.workspace_label(workspace_dir)),
            "2026-07-16T10:02:00Z",
        )])
    }

    fn upsert_thread_review(
        &self,
        _workspace_dir: &str,
        _review: &ReviewCenterThreadProjection,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn replace_pending_inbox(
        &self,
        _workspace_dir: &str,
        _inbox: &[ReviewCenterInboxItem],
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn append_history_entry(
        &self,
        _workspace_dir: &str,
        _entry: &ReviewCenterHistoryEntry,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

struct GatedReviewCenterRepository {
    thread_loads: AtomicUsize,
    first_load_started: mpsc::SyncSender<()>,
    release_first_load: Mutex<mpsc::Receiver<()>>,
}

impl ReviewCenterRepositoryPort for GatedReviewCenterRepository {
    fn load_thread_reviews(
        &self,
        workspace_dir: &str,
        thread_id: &str,
    ) -> anyhow::Result<Vec<ReviewCenterThreadProjection>> {
        let load_index = self.thread_loads.fetch_add(1, Ordering::SeqCst);
        if load_index == 0 {
            self.first_load_started.send(()).map_err(|error| {
                anyhow::anyhow!("first review load start signal failed: {error}")
            })?;
            self.release_first_load
                .lock()
                .expect("first review load release mutex should remain healthy")
                .recv_timeout(Duration::from_secs(2))
                .map_err(|error| anyhow::anyhow!("first review load release failed: {error}"))?;
        }
        Ok(vec![ReviewCenterThreadProjection::new(
            thread_id,
            format!("review-{}", load_index + 1),
            "Architecture review",
            "pending",
            format!("{workspace_dir}::{thread_id}"),
            "2026-07-16T10:00:00Z",
            "2026-07-16T10:01:00Z",
        )])
    }

    fn load_pending_inbox(
        &self,
        _workspace_dir: &str,
    ) -> anyhow::Result<Vec<ReviewCenterInboxItem>> {
        Ok(Vec::new())
    }

    fn load_recent_history(
        &self,
        _workspace_dir: &str,
    ) -> anyhow::Result<Vec<ReviewCenterHistoryEntry>> {
        Ok(Vec::new())
    }

    fn upsert_thread_review(
        &self,
        _workspace_dir: &str,
        _review: &ReviewCenterThreadProjection,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn replace_pending_inbox(
        &self,
        _workspace_dir: &str,
        _inbox: &[ReviewCenterInboxItem],
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn append_history_entry(
        &self,
        _workspace_dir: &str,
        _entry: &ReviewCenterHistoryEntry,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

fn complete_reviews_overlay_load(runtime: &mut ShellRuntime) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        runtime.poll_background_messages();
        if matches!(
            runtime.app().shell.reviews_overlay_ui_state.screen_model(),
            crate::adapter::inbound::tui::app::reviews_overlay_ui::ReviewsOverlayScreenModel::Ready { .. }
        ) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "review overlay authority should load through ShellRuntime"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        runtime.take_redraw_request(),
        "review overlay completion should request a redraw"
    );
}

#[test]
fn inline_reviews_inspection_keeps_current_thread_and_inbox_history_on_same_workspace() {
    let thread_workspace = "/tmp/review-thread-workspace".to_string();
    let repository = Arc::new(CountingReviewCenterRepository::for_context(
        thread_workspace.clone(),
        "thread-1",
    ));
    let mut app = test_native_tui_app_with_review_center_repository(repository.clone());
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should have a ready conversation");
    };
    conversation.record_thread_prepared(
        "thread-1".to_string(),
        "Loaded thread".to_string(),
        thread_workspace.clone(),
    );
    app.show_reviews_overlay();
    let mut runtime = ShellRuntime::new(app);
    assert!(runtime.take_redraw_request());
    complete_reviews_overlay_load(&mut runtime);
    assert_eq!(repository.load_counts(), (1, 1, 1));

    let app = runtime.app_mut();
    let overlay_view =
        build_reviews_overlay_view(app.shell.reviews_overlay_ui_state.screen_model());

    assert!(overlay_view.header_lines.iter().any(|line| {
        line.to_string()
            .contains("Review Center / shell inspection")
    }));
    assert_eq!(overlay_view.current_thread_reviews.len(), 1);
    assert!(
        overlay_view.current_thread_reviews[0]
            .summary_line
            .to_string()
            .contains("correct workspace correct thread review")
    );
    assert_eq!(overlay_view.inbox_reviews.len(), 1);
    assert!(
        overlay_view.inbox_reviews[0]
            .summary_line
            .to_string()
            .contains("correct workspace inbox")
    );
    assert_eq!(overlay_view.history_reviews.len(), 1);
    assert!(
        overlay_view.history_reviews[0]
            .summary_line
            .to_string()
            .contains("correct workspace history")
    );
    assert!(
        !overlay_view.current_thread_reviews[0]
            .summary_line
            .to_string()
            .contains("wrong thread review")
    );
    assert!(
        !overlay_view.inbox_reviews[0]
            .summary_line
            .to_string()
            .contains("wrong workspace inbox")
    );
    assert!(
        !overlay_view.history_reviews[0]
            .summary_line
            .to_string()
            .contains("wrong workspace history")
    );
}

#[test]
fn repeated_reviews_inspection_draws_do_not_reload_application_authority() {
    let repository = Arc::new(CountingReviewCenterRepository::default());
    let mut app = test_native_tui_app_with_review_center_repository(repository.clone());
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should have a ready conversation");
    };
    conversation.record_thread_prepared(
        "thread-1".to_string(),
        "Loaded thread".to_string(),
        "/tmp/root".to_string(),
    );

    app.show_reviews_overlay();
    let mut runtime = ShellRuntime::new(app);
    assert!(runtime.take_redraw_request());
    complete_reviews_overlay_load(&mut runtime);
    assert_eq!(repository.load_counts(), (1, 1, 1));

    let mut terminal = Terminal::new(TestBackend::new(104, 34)).expect("test terminal");
    terminal
        .draw(|frame| {
            draw(
                frame,
                runtime.app_mut(),
                ShellFrontendMode::InlineMainBuffer,
            )
        })
        .expect("first reviews inspection render succeeds");
    let first_render = tui_testkit::screen_text(&terminal);
    assert!(first_render.contains("Review Center"));
    assert_eq!(repository.load_counts(), (1, 1, 1));

    terminal
        .draw(|frame| {
            draw(
                frame,
                runtime.app_mut(),
                ShellFrontendMode::InlineMainBuffer,
            )
        })
        .expect("second reviews inspection render succeeds");
    let second_render = tui_testkit::screen_text(&terminal);

    assert!(second_render.contains("Review Center"));
    assert_eq!(repository.load_counts(), (1, 1, 1));
}

#[test]
fn reviews_identity_drift_reloads_latest_context_through_shell_runtime() {
    let (started_tx, started_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let repository = Arc::new(GatedReviewCenterRepository {
        thread_loads: AtomicUsize::new(0),
        first_load_started: started_tx,
        release_first_load: Mutex::new(release_rx),
    });
    let mut app = test_native_tui_app_with_review_center_repository(repository.clone());
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should have a ready conversation");
    };
    conversation.record_thread_prepared(
        "thread-1".to_string(),
        "First thread".to_string(),
        "/tmp/root".to_string(),
    );
    app.show_reviews_overlay();
    started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("first review authority load should start");

    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should have a ready conversation");
    };
    conversation.record_thread_prepared(
        "thread-2".to_string(),
        "Replacement thread".to_string(),
        "/tmp/other".to_string(),
    );
    let mut runtime = ShellRuntime::new(app);
    assert!(runtime.take_redraw_request());
    release_tx
        .send(())
        .expect("first review authority load should release");

    complete_reviews_overlay_load(&mut runtime);

    assert_eq!(repository.thread_loads.load(Ordering::SeqCst), 2);
    let crate::adapter::inbound::tui::app::reviews_overlay_ui::ReviewsOverlayScreenModel::Ready {
        request,
        authority,
    } = runtime.app().shell.reviews_overlay_ui_state.screen_model()
    else {
        panic!("latest Review Center context should become ready");
    };
    assert_eq!(request.context.workspace_directory, "/tmp/other");
    assert_eq!(
        request
            .context
            .active_thread
            .as_ref()
            .map(|thread| thread.thread_id.as_str()),
        Some("thread-2")
    );
    let reviews = authority
        .current_thread_reviews
        .as_ref()
        .expect("latest thread reviews should load");
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0].thread_id, "thread-2");
    assert_eq!(reviews[0].review_summary, "/tmp/other::thread-2");
}

#[test]
fn ready_reviews_identity_drift_reloads_latest_context_during_runtime_poll() {
    let repository = Arc::new(CountingReviewCenterRepository::default());
    let mut app = test_native_tui_app_with_review_center_repository(repository.clone());
    app.show_reviews_overlay();
    let mut runtime = ShellRuntime::new(app);
    assert!(runtime.take_redraw_request());
    complete_reviews_overlay_load(&mut runtime);
    assert_eq!(repository.load_counts(), (0, 1, 1));

    let correlation = runtime
        .app_mut()
        .runtime
        .client_runtime
        .begin_test_turn_submission();
    runtime
        .app()
        .runtime
        .tx
        .send(BackgroundMessage::ConversationStream {
            correlation,
            event: ConversationStreamEvent::ThreadPrepared {
                thread_id: "thread-2".to_string(),
                title: "Replacement thread".to_string(),
                cwd: "/tmp/other".to_string(),
                runtime_envelope: Box::default(),
            },
        })
        .expect("thread-prepared event should enter the shell runtime");

    runtime.poll_background_messages();

    assert!(matches!(
        runtime.app().shell.reviews_overlay_ui_state.screen_model(),
        crate::adapter::inbound::tui::app::reviews_overlay_ui::ReviewsOverlayScreenModel::Loading(request)
            if request.context.workspace_directory == "/tmp/other"
                && request.context.active_thread.as_ref().map(|thread| thread.thread_id.as_str())
                    == Some("thread-2")
    ));
    complete_reviews_overlay_load(&mut runtime);

    assert_eq!(repository.load_counts(), (1, 2, 2));
    let crate::adapter::inbound::tui::app::reviews_overlay_ui::ReviewsOverlayScreenModel::Ready {
        request,
        authority,
    } = runtime.app().shell.reviews_overlay_ui_state.screen_model()
    else {
        panic!("latest Review Center context should become ready");
    };
    assert_eq!(request.context.workspace_directory, "/tmp/other");
    assert_eq!(
        request
            .context
            .active_thread
            .as_ref()
            .map(|thread| thread.thread_id.as_str()),
        Some("thread-2")
    );
    let reviews = authority
        .current_thread_reviews
        .as_ref()
        .expect("latest thread reviews should load");
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0].thread_id, "thread-2");
}

#[test]
fn inline_model_selection_inspection_renders_model_and_effort_picker() {
    let mut terminal = Terminal::new(TestBackend::new(104, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.show_model_selection_overlay();

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline model selection render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Select Model and Effort / inline inspection"));
    assert!(rendered.contains("Models"));
    assert!(rendered.contains("gpt-5.5"));
    assert!(rendered.contains("default"));
    assert!(rendered.contains("Think Level"));
    assert!(rendered.contains("high"));
    assert!(rendered.contains("Enter/1-7: choose model"));
    assert!(!rendered.contains(":model <"));
    assert!(!rendered.contains("┌"));
}

#[test]
fn inline_view_selection_inspection_renders_visibility_picker() {
    let mut terminal = Terminal::new(TestBackend::new(104, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.show_view_selection_overlay();

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline view selection render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Select Conversation View / inline inspection"));
    assert!(rendered.contains("Views"));
    assert!(rendered.contains("simple"));
    assert!(rendered.contains("medium"));
    assert!(rendered.contains("detail"));
    assert!(rendered.contains("Codex and Codex Commentary stay visible"));
    assert!(rendered.contains("Enter/1-3: apply"));
    assert!(!rendered.contains(":view <"));
    assert!(!rendered.contains("┌"));
}

#[test]
fn inline_language_selection_inspection_renders_language_picker() {
    let mut terminal = Terminal::new(TestBackend::new(104, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.show_language_selection_overlay();

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline language selection render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Select Language / inline inspection"));
    assert!(rendered.contains("Languages"));
    assert!(rendered.contains("English"));
    assert!(rendered.contains("한국어"));
    assert!(
        rendered.contains("User prompts, task titles, and runtime payloads are kept as written.")
    );
    assert!(rendered.contains("Enter/1-2: apply"));
    assert!(!rendered.contains(":language <"));
    assert!(!rendered.contains("┌"));
}

#[test]
fn inline_supersession_inspection_renders_prepare_panels_inside_shell_frame() {
    let mut terminal = Terminal::new(TestBackend::new(96, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_readiness_snapshot_for_test(Some(sample_parallel_mode_snapshot(
        ParallelModeReadinessState::Degraded,
    )));
    app.shell.chrome.shell_overlay = ShellOverlay::Supersession;

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline supersession inspection render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Parallel / inline inspection"));
    assert!(rendered.contains("Overview"));
    assert!(rendered.contains("Delivery"));
    assert!(rendered.contains("Capacity"));
    assert!(rendered.contains("Tasks"));
    assert!(rendered.contains("Parallel Event Stream"));
    assert!(rendered.contains("loading pool board"));
    assert!(rendered.contains("loading distributor board"));
    assert!(rendered.contains("Ctrl+R refresh"));
    assert!(rendered.contains("Ctrl+P off"));
    assert!(rendered.contains(":peek agents"));
    assert!(rendered.contains("Ctrl+O/Esc/Ctrl+C close"));
    assert!(!rendered.contains("Transcript /"));
    assert!(!rendered.contains("┌"));
}

#[test]
fn inline_supersession_deep_actor_and_matching_fallback_stay_visible() {
    let mut terminal = Terminal::new(TestBackend::new(120, 32)).expect("test terminal");
    let mut app = make_test_app();
    app.set_parallel_mode_enabled_for_test(true);
    let slots = (1..=12)
        .map(|index| {
            ParallelModePoolSlotSnapshot::new(
                format!("slot-{index}"),
                ParallelModePoolSlotState::Running,
                format!("agent/{index}"),
                format!("pool/{index}"),
                format!("agent-{index}"),
            )
        })
        .collect();
    let roster = (1..=12)
        .map(|index| {
            ParallelModeAgentRosterEntry::new(
                format!("agent-{index}"),
                format!("Task {index}"),
                format!("slot-{index}"),
                format!("agent/{index}"),
                "running",
                format!("{index}m"),
                format!("testing {index}"),
            )
        })
        .collect();
    let queue_items = (1..=12)
        .map(|index| {
            ParallelModeDistributorQueueItem::new(
                format!("agent-{index}"),
                format!("Task {index}"),
                ParallelModeQueueItemState::Queued,
                format!("agent/{index}"),
                format!("sha{index}"),
                format!("queued {index}"),
            )
        })
        .collect();
    let detail = ParallelModeAgentSessionDetailSnapshot::new(
        "slot-1:task-1",
        "agent-1",
        "task-1",
        "Task 1",
        "slot-1",
        Some("thread-1".to_string()),
        "/tmp/pool/1",
        "agent/1",
        "2026-07-15T00:00:00Z",
        "running",
        "running",
        "testing 1",
        "tests pending",
        "ledger pending",
        None,
        Vec::new(),
        "2026-07-15T00:01:00Z",
    );
    let snapshot = ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/root",
        ParallelModePoolBoardSnapshot::new(12, "/tmp/pool", "running", slots),
        ParallelModeAgentRosterSnapshot::new(roster, "empty"),
        ParallelModeSupervisorDetailSnapshot::new(Some(detail), "no detail"),
        ParallelModeDistributorSnapshot::new(queue_items, Vec::new(), "queued", "queue active"),
        None,
    );
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(snapshot.clone()));
    app.shell.chrome.shell_overlay = ShellOverlay::Supersession;
    app.shell
        .supersession_mud_ui_state
        .move_selection(&snapshot, 10);

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline supersession deep slot render succeeds");
    let pool_rendered = tui_testkit::screen_text(&terminal);
    assert!(
        pool_rendered.contains("> slot-11"),
        "deep selected pool slot must be visible:\n{pool_rendered}"
    );

    app.shell.supersession_mud_ui_state.focus_next_zone();
    app.shell
        .supersession_mud_ui_state
        .move_selection(&snapshot, 10);

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline supersession selected roster render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Current / Diagnostics"));
    assert!(
        rendered.contains("> Task 11"),
        "deep selected roster row must be visible:\n{rendered}"
    );
    assert!(!rendered.contains("> Task 1  "));

    app.shell.supersession_mud_ui_state.focus_next_zone();
    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline supersession selected detail fallback render succeeds");
    let detail_rendered = tui_testkit::screen_text(&terminal);

    assert!(detail_rendered.contains("> Current  Task 11"));
    assert!(detail_rendered.contains("Latest  testing 11"));

    app.shell.supersession_mud_ui_state.focus_next_zone();
    app.shell
        .supersession_mud_ui_state
        .move_selection(&snapshot, 10);
    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline supersession deep distributor render succeeds");
    let distributor_rendered = tui_testkit::screen_text(&terminal);
    assert!(
        distributor_rendered.contains("> Task 11  ·  queued  ·  next"),
        "deep selected distributor item must be visible:\n{distributor_rendered}"
    );
}

#[test]
fn inline_parallel_event_stream_uses_selected_tui_language() {
    let mut terminal = Terminal::new(TestBackend::new(96, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.shell.tui_language = TuiLanguage::English;
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/root",
        ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
        ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
        ParallelModeDistributorSnapshot::new(
            Vec::new(),
            vec![ParallelModeCompletionFeedEntry::new(
                "reported",
                "no agent results reported yet",
            )],
            "idle",
            "queue idle",
        ),
        Some("control tower is live".to_string()),
    )));
    app.shell.chrome.shell_overlay = ShellOverlay::Supersession;

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline supersession language render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("parallel board refreshed. control tower is live"));
    assert!(rendered.contains("reported stage record: no agent results reported yet"));
    assert!(!rendered.contains("상태를 갱신했습니다"));
    assert!(!rendered.contains("단계 기록"));
}

#[test]
fn inline_parallel_peek_picker_keeps_agent_rows_visible_in_compact_main_buffer() {
    /*
     * `:peek` starts as an agent picker. In inline app-server mode the picker must
     * spend the available inspection body on active agents instead of letting the
     * empty conversation preview consume the compact terminal height.
     */
    let mut terminal = Terminal::new(TestBackend::new(80, 18)).expect("test terminal");
    let mut app = make_test_app();
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/root",
        ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "running", Vec::new()),
        ParallelModeAgentRosterSnapshot::new(
            vec![
                ParallelModeAgentRosterEntry::new(
                    "agent-guardian",
                    "Guard peek rows",
                    "slot-1",
                    "akra-agent/slot-1/guard-peek-rows",
                    "running",
                    "active",
                    "checking compact picker rendering",
                )
                .with_thread_id(Some("thread-guardian".to_string()))
                .with_lease_identity(
                    "task-guardian",
                    "session-guardian",
                    Some("generation-guardian".to_string()),
                ),
                ParallelModeAgentRosterEntry::new(
                    "agent-builder",
                    "Build peek rows",
                    "slot-2",
                    "akra-agent/slot-2/build-peek-rows",
                    "starting",
                    "active",
                    "starting the second worker",
                )
                .with_thread_id(Some("thread-builder".to_string()))
                .with_lease_identity(
                    "task-builder",
                    "session-builder",
                    Some("generation-builder".to_string()),
                ),
                ParallelModeAgentRosterEntry::new(
                    "agent-reviewer",
                    "Review peek rows",
                    "slot-3",
                    "akra-agent/slot-3/review-peek-rows",
                    "commit_ready",
                    "official",
                    "official completion is waiting for delivery",
                )
                .with_thread_id(Some("thread-reviewer".to_string()))
                .with_lease_identity(
                    "task-reviewer",
                    "session-reviewer",
                    Some("generation-reviewer".to_string()),
                ),
            ],
            "empty",
        ),
        ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
        None,
    )));
    let active_agents = app.active_parallel_peek_entries();
    app.shell
        .parallel_peek_overlay_ui_state
        .select_initial_agent(&active_agents);
    app.shell.chrome.shell_overlay = ShellOverlay::ParallelPeek;

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline parallel peek picker render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Parallel Peek / inline inspection"));
    assert!(rendered.contains("Active Agents"));
    assert!(rendered.contains("> 1. agent-guardian / slot-1"));
    assert!(rendered.contains("agent-builder / slot-2"));
    assert!(rendered.contains("agent-reviewer / slot-3"));
    assert!(rendered.contains("3 active parallel agent(s) ready for peek"));
    assert!(
        !rendered.contains("Select an active agent and press Enter"),
        "agent list should be the primary compact picker surface:\n{rendered}"
    );
    assert!(!rendered.contains("┌"));
}

#[test]
fn inline_parallel_peek_preview_prioritizes_loaded_transcript_in_compact_main_buffer() {
    /*
     * Once a parallel agent is selected, `:peek` is meant to show the agent
     * conversation. The compact inline viewport should therefore land on the
     * loaded transcript instead of spending the visible rows on preview metadata.
     */
    let mut terminal = Terminal::new(TestBackend::new(80, 14)).expect("test terminal");
    let mut app = make_test_app();
    app.shell.chrome.shell_overlay = ShellOverlay::ParallelPeek;
    app.shell.parallel_peek_overlay_ui_state.open_preview(
        super::super::parallel_peek_overlay_ui::ParallelPeekConversationPreview {
            agent_id: "agent-scribe".to_string(),
            slot_id: "slot-2".to_string(),
            task_title: "Check test updates".to_string(),
            thread_id: Some("thread-scribe".to_string()),
            snapshot: Some(ConversationSnapshot {
                thread_id: "thread-scribe".to_string(),
                title: "Scribe transcript".to_string(),
                cwd: "/tmp/pool/slot-2".to_string(),
                messages: vec![
                    ConversationMessage::new(
                        ConversationMessageKind::User,
                        "please inspect the test changes",
                        None,
                        None,
                    ),
                    ConversationMessage::new(
                        ConversationMessageKind::Agent,
                        "the current test changes need one focused assertion",
                        Some("final_answer".to_string()),
                        None,
                    ),
                ],
                warnings: Vec::new(),
                runtime_notices: Vec::new(),
                item_lifecycle: Default::default(),
            }),
            status_text: "conversation snapshot loaded".to_string(),
        },
    );

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline parallel peek conversation render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Conversation Preview"));
    assert!(rendered.contains("conversation:"));
    assert!(rendered.contains("User: please inspect the test changes"));
    assert!(rendered.contains("Agent: the current test changes need one focused assertion"));
    assert!(
        !rendered.contains("thread: thread-scribe"),
        "compact preview should scroll past metadata to the transcript:\n{rendered}"
    );
    assert!(!rendered.contains("┌"));
}

#[test]
fn inline_parallel_peek_preview_can_scroll_between_oldest_and_latest_messages() {
    /*
     * Long parallel conversations need an in-preview scroll position. The default
     * view stays pinned to the latest transcript lines, while an older scroll
     * position exposes the beginning of the loaded app-server conversation.
     */
    let mut terminal = Terminal::new(TestBackend::new(80, 14)).expect("test terminal");
    let mut app = make_test_app();
    app.shell.chrome.shell_overlay = ShellOverlay::ParallelPeek;

    let mut messages = Vec::new();
    messages.push(ConversationMessage::new(
        ConversationMessageKind::User,
        "early prompt marker",
        None,
        None,
    ));
    for index in 1..=12 {
        messages.push(ConversationMessage::new(
            ConversationMessageKind::Agent,
            format!("middle agent message {index}"),
            Some("final_answer".to_string()),
            None,
        ));
    }
    messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer marker",
        Some("final_answer".to_string()),
        None,
    ));
    app.shell.parallel_peek_overlay_ui_state.open_preview(
        super::super::parallel_peek_overlay_ui::ParallelPeekConversationPreview {
            agent_id: "agent-scribe".to_string(),
            slot_id: "slot-2".to_string(),
            task_title: "Scroll transcript".to_string(),
            thread_id: Some("thread-scribe".to_string()),
            snapshot: Some(ConversationSnapshot {
                thread_id: "thread-scribe".to_string(),
                title: "Scrollable transcript".to_string(),
                cwd: "/tmp/pool/slot-2".to_string(),
                messages,
                warnings: Vec::new(),
                runtime_notices: Vec::new(),
                item_lifecycle: Default::default(),
            }),
            status_text: "conversation snapshot loaded".to_string(),
        },
    );

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline parallel peek latest render succeeds");
    let latest_rendered = tui_testkit::screen_text(&terminal);
    assert!(latest_rendered.contains("latest answer marker"));
    assert!(
        !latest_rendered.contains("early prompt marker"),
        "default preview should stay pinned to latest transcript lines:\n{latest_rendered}"
    );

    app.shell
        .parallel_peek_overlay_ui_state
        .scroll_conversation_to_oldest();
    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline parallel peek oldest render succeeds");
    let oldest_rendered = tui_testkit::screen_text(&terminal);
    assert!(oldest_rendered.contains("early prompt marker"));
    assert!(
        !oldest_rendered.contains("latest answer marker"),
        "oldest scroll should expose the start of the transcript:\n{oldest_rendered}"
    );
}

#[test]
fn inline_parallel_home_replaces_single_mode_transcript_when_overlay_hidden() {
    let mut terminal = Terminal::new(TestBackend::new(104, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_readiness_snapshot_for_test(Some(sample_parallel_mode_snapshot(
        ParallelModeReadinessState::Ready,
    )));
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/root",
        ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
        ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
        None,
    )));
    app.push_parallel_supervisor_event_for_test("00:00:00", "You", "안녕하세요");
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("expected ready conversation state");
    };
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::User,
        "안녕하세요",
        None,
        None,
    ));
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "single mode reply must not own the parallel body",
        Some("final_answer".to_string()),
        None,
    ));

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline parallel home render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Parallel / inline inspection"));
    assert!(rendered.contains("Parallel Event Stream"));
    assert!(rendered.contains("You: 안녕하세요"));
    assert!(!rendered.contains("Operator: first user word"));
    assert!(!rendered.contains("Codex:"));
    assert!(!rendered.contains("single mode reply must not own"));
    assert_eq!(rendered.matches("╭ Task").count(), 1, "{rendered}");
}

#[test]
fn inline_parallel_home_suppresses_startup_banner_on_empty_draft() {
    let mut terminal = Terminal::new(TestBackend::new(104, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = true;
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_readiness_snapshot_for_test(Some(sample_parallel_mode_snapshot(
        ParallelModeReadinessState::Ready,
    )));
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/root",
        ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
        ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
        None,
    )));

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline parallel empty draft render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Parallel / inline inspection"));
    assert!(rendered.contains("Parallel Event Stream"));
    assert!(rendered.contains("Parallel  ready"));
    assert!(!rendered.contains("█████"));
    assert!(!rendered.contains("╚═╝"));
}

#[test]
fn inline_supersession_keeps_buffered_prompt_visible_in_compact_tail() {
    /*
     * Supersession replaces the transcript with a dense inspection board while
     * the prompt remains active below it. The compact tail must therefore keep
     * the prompt suffix visible instead of letting planning detail rows consume
     * the whole tail and leave the cursor over status copy.
     */
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("test terminal");
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_readiness_snapshot_for_test(Some(sample_parallel_mode_snapshot(
        ParallelModeReadinessState::Ready,
    )));
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/root",
        ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
        ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
        None,
    )));
    app.shell.chrome.shell_overlay = ShellOverlay::Supersession;
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("expected ready conversation state");
    };
    conversation.composer.input_buffer = "안녕하세요?".to_string();

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline supersession prompt render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("> 안녕하세요?"));
    assert!(rendered.contains("Enter send  |  Ctrl+J newline"));
    assert!(
        !rendered.contains("now: none"),
        "planning detail rows should be clipped before they can hide the prompt:\n{rendered}"
    );
    assert_eq!(rendered.matches("╭ Task").count(), 1, "{rendered}");
}

#[test]
fn inline_supersession_command_hints_keep_controls_visible_when_compact() {
    /*
     * The command-hint panel often receives only one body row after the parallel
     * event stream takes the remaining live viewport. That visible row must carry
     * the real board controls, not just the first "refresh" hint.
     */
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("test terminal");
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_readiness_snapshot_for_test(Some(sample_parallel_mode_snapshot(
        ParallelModeReadinessState::Ready,
    )));
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/root",
        ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
        ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
        None,
    )));
    app.shell.chrome.shell_overlay = ShellOverlay::Supersession;

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline supersession command hint render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Command Hints"));
    assert!(rendered.contains("Ctrl+R refresh"));
    assert!(
        rendered.contains("Ctrl+P off"),
        "parallel off shortcut must stay visible in compact command hints:\n{rendered}"
    );
    assert!(
        rendered.contains(":peek agents"),
        "agent inspection command must stay visible in compact command hints:\n{rendered}"
    );
    assert!(
        rendered.contains("Ctrl+O/Esc/Ctrl+C close"),
        "close shortcuts must stay visible in compact command hints:\n{rendered}"
    );
}

#[test]
fn inline_supersession_narrow_snapshot_keeps_selected_timeline_visible() {
    /*
     * Parallel Mode keeps the selected session lifecycle in the bottom event
     * stream while pool/distributor/orchestrator state stays in the mid panels.
     */
    let mut terminal = Terminal::new(TestBackend::new(72, 32)).expect("test terminal");
    let mut app = make_test_app();
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_readiness_snapshot_for_test(Some(sample_parallel_mode_snapshot(
        ParallelModeReadinessState::Ready,
    )));
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/root",
        ParallelModePoolBoardSnapshot::new(
            3,
            "/tmp/pool",
            "idle",
            vec![ParallelModePoolSlotSnapshot::new(
                "slot-1",
                ParallelModePoolSlotState::Running,
                "akra-agent/slot-1/timeline",
                "akra-pool/slot-1",
                "agent-1 / task-1",
            )],
        ),
        ParallelModeAgentRosterSnapshot::new(
            vec![ParallelModeAgentRosterEntry::new(
                "agent-1",
                "Timeline UI",
                "slot-1",
                "akra-agent/slot-1/timeline",
                "commit_ready",
                "official",
                "official ledger refresh accepted the completion report",
            )],
            "empty",
        ),
        ParallelModeSupervisorDetailSnapshot::new(
            Some(ParallelModeAgentSessionDetailSnapshot::new(
                "slot-1:task-1",
                "agent-1",
                "task-1",
                "Timeline UI",
                "slot-1",
                Some("thread-1".to_string()),
                "/tmp/pool/slot-1",
                "akra-agent/slot-1/timeline",
                "2026-04-17T00:00:00Z",
                "commit_ready",
                "commit_ready",
                "official ledger refresh accepted the completion report",
                "tests passed",
                "official ledger refresh succeeded",
                Some("commit-ready result accepted into distributor queue".to_string()),
                vec![
                    ParallelModeAgentSessionHistoryEntry::new(
                        "assigned",
                        "2026-04-17T00:00:00Z",
                        "slot lease acquired and branch reserved for launch",
                    ),
                    ParallelModeAgentSessionHistoryEntry::new(
                        "running",
                        "2026-04-17T00:01:00Z",
                        "agent session is active in the leased slot",
                    ),
                    ParallelModeAgentSessionHistoryEntry::new(
                        "commit_ready",
                        "2026-04-17T00:02:00Z",
                        "official ledger refresh accepted the completion report",
                    ),
                ],
                "2026-04-17T00:02:00Z",
            )),
            "empty",
        ),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
        None,
    )));
    app.shell.chrome.shell_overlay = ShellOverlay::Supersession;
    app.shell.supersession_mud_ui_state.focus_next_zone();
    app.shell.supersession_mud_ui_state.focus_next_zone();

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline supersession timeline render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(!rendered.contains("Recent Parallel Events"));
    assert!(rendered.contains("Distributor: slot-1"));
    assert!(rendered.contains("Agent agent-1: Timeline UI"));
    assert!(rendered.contains("Ledger: accepted Timeline UI"));
    assert!(rendered.contains("head: idle"));
    assert!(rendered.contains("Current"));
    assert!(
        rendered.contains("> Current"),
        "selected session detail must survive the narrow layout:\n{rendered}"
    );
    assert!(!rendered.contains("commit_ready"));
}

#[test]
fn inline_tail_adds_only_spinner_to_prompt_during_parallel_loading() {
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.shell.chrome.shell_overlay = ShellOverlay::Supersession;
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/root",
        ParallelModePoolBoardSnapshot::new(0, "loading: test", "loading", Vec::new()),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), "loading"),
        ParallelModeSupervisorDetailSnapshot::new(None, "loading"),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "loading", "loading"),
        None,
    )));
    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
            .iter()
            .any(|frame| rendered.contains(*frame))
    );
    assert!(rendered.contains("Parallel board loading"));
    assert!(!rendered.contains("thinking"));
}

#[test]
fn inline_parallel_home_keeps_loading_spinner_when_overlay_hidden() {
    let mut terminal = Terminal::new(TestBackend::new(104, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_readiness_snapshot_for_test(Some(sample_parallel_mode_snapshot(
        ParallelModeReadinessState::Ready,
    )));
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/root",
        ParallelModePoolBoardSnapshot::new(0, "loading: test", "loading", Vec::new()),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), "loading"),
        ParallelModeSupervisorDetailSnapshot::new(None, "loading"),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "loading", "loading"),
        Some("loading 3/3: board refresh".to_string()),
    )));

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline parallel home loading render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Parallel / inline inspection"));
    assert!(rendered.contains("prompt paused while setup completes"));
    assert!(
        ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
            .iter()
            .any(|frame| rendered.contains(*frame))
    );
    assert!(rendered.contains("Parallel board loading"));
}

#[test]
fn inline_tail_omits_parallel_loading_spinner_after_empty_non_loading_snapshot() {
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.shell.chrome.shell_overlay = ShellOverlay::Supersession;
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/root",
        ParallelModePoolBoardSnapshot::new(0, "idle", "idle", Vec::new()),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
        ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
        None,
    )));
    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    for frame in ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"] {
        assert!(!rendered.contains(frame));
    }
}

// The inline tail is the status surface that remains visible during app-server
// execution, so catalog and parallel-mode summaries are tested without a full
// frame render.
#[test]
fn inline_tail_surfaces_parallel_mode_summary_when_enabled() {
    /*
     * The tail summary is intentionally tested without full-frame rendering so
     * parallel readiness, supervisor, and distributor copy can be verified as a
     * compact status contract independent of layout height.
     */
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    tui_testkit::append_agent_history_message(
        &mut app,
        "parallel summary should render in the live shell",
    );
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_readiness_snapshot_for_test(Some(sample_parallel_mode_snapshot(
        ParallelModeReadinessState::Ready,
    )));
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/root",
        ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
        ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
        None,
    )));
    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("Parallel  ready"));
    assert!(!rendered.contains("agents:"));
    assert!(!rendered.contains("queue: idle"));
    assert!(rendered.contains("parallel alert:"));
}

#[test]
fn inline_tail_shows_syncing_instead_of_idle_before_dispatch_projection_arrives() {
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_readiness_snapshot_for_test(Some(sample_parallel_mode_snapshot(
        ParallelModeReadinessState::Ready,
    )));
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/root",
        ParallelModePoolBoardSnapshot::new(
            1,
            "/tmp/pool",
            "idle",
            vec![ParallelModePoolSlotSnapshot::new(
                "slot-1",
                ParallelModePoolSlotState::Idle,
                "prerelease",
                "/tmp/pool/slot-1",
                "idle",
            )],
        ),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
        ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
        None,
    )));
    let _ = app.mark_parallel_mode_supervisor_refresh_in_flight_for_test();

    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("Parallel  ◐ syncing  ·  1 available"));
    assert!(!rendered.contains("pool: idle"));
}

#[test]
fn inline_tail_omits_legacy_planning_valid_status_in_single_and_parallel_home() {
    fn rendered_tail(parallel_mode_enabled: bool) -> String {
        let mut app = make_test_app();
        app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
        tui_testkit::append_agent_history_message(&mut app, "planning status baseline");
        app.sync_ready_conversation_planning_runtime_projection(
            sample_planning_runtime_projection(
                "Planning Context",
                "queue head: rank 1 / task-1 / Implement shell planning status",
            ),
        );
        if parallel_mode_enabled {
            app.set_parallel_mode_enabled_for_test(true);
            app.set_parallel_mode_readiness_snapshot_for_test(Some(sample_parallel_mode_snapshot(
                ParallelModeReadinessState::Ready,
            )));
            app.set_parallel_mode_supervisor_snapshot_for_test(Some(
                ParallelModeSupervisorSnapshot::new(
                    ParallelModeSupervisorState::Supervise,
                    "/tmp/root",
                    ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
                    ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
                    ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
                    ParallelModeDistributorSnapshot::new(
                        Vec::new(),
                        Vec::new(),
                        "idle",
                        "queue idle",
                    ),
                    None,
                ),
            ));
        }
        build_inline_tail_lines(&app)
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    let single_rendered = rendered_tail(false);
    assert!(!single_rendered.contains("planning: valid"));
    assert!(!single_rendered.contains("queue: queue head: rank 1 / task-1"));
    assert!(single_rendered.contains("now: Implement shell planning status"));

    let parallel_rendered = rendered_tail(true);
    assert!(!parallel_rendered.contains("planning: valid"));
    assert!(parallel_rendered.contains("Parallel  ○ 2 queued"));
    assert!(!parallel_rendered.contains("queue: queue head: rank 1 / task-1"));
    assert!(parallel_rendered.contains("now: Implement shell planning status"));
}

#[test]
fn inline_tail_places_parallel_slot_working_line_between_queue_and_prompt() {
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    tui_testkit::append_agent_history_message(&mut app, "parallel slot status baseline");
    app.sync_ready_conversation_planning_runtime_projection(sample_planning_runtime_projection(
        "Planning Context",
        "queue head: rank 1 / task-1 / Keep slot status visible",
    ));
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_readiness_snapshot_for_test(Some(sample_parallel_mode_snapshot(
        ParallelModeReadinessState::Ready,
    )));
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/root",
        ParallelModePoolBoardSnapshot::new(
            3,
            "/tmp/pool",
            "running",
            vec![ParallelModePoolSlotSnapshot::new(
                "slot-1",
                ParallelModePoolSlotState::Running,
                "akra-agent/slot-1/task-one",
                "akra-pool/slot-1",
                "agent-1 / task-1",
            )],
        ),
        ParallelModeAgentRosterSnapshot::new(
            vec![ParallelModeAgentRosterEntry::new(
                "agent-1",
                "Slot Tail",
                "slot-1",
                "akra-agent/slot-1/task-one",
                "running",
                "42s",
                "rendering the active slot status",
            )],
            "empty",
        ),
        ParallelModeSupervisorDetailSnapshot::new(None, "empty"),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
        None,
    )));
    let lines = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    let queue_index = lines
        .iter()
        .position(|line| line.contains("now: Implement shell planning status"))
        .expect("planning queue line should remain visible");
    let working_index = lines
        .iter()
        .position(|line| line.contains("◦ Working") && line.contains("pool slot-1"))
        .expect("parallel slot working line should identify the active pool slot");
    let prompt_index = lines
        .iter()
        .position(|line| line.trim_start().starts_with('>'))
        .expect("prompt input row should remain visible");

    assert!(
        queue_index < working_index,
        "slot working line should sit below queue status:\n{}",
        lines.join("\n")
    );
    assert!(
        working_index < prompt_index,
        "slot working line should sit above prompt input:\n{}",
        lines.join("\n")
    );
    assert!(lines[working_index].contains("state: running"));
    assert!(lines[working_index].contains("Slot Tail"));
}

#[test]
fn inline_tail_reports_partial_handle_based_session_catalog_status() {
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.shell.chrome.session_state = SessionState::Ready(SessionCatalog::partial(
        SessionCatalogTier::HandleBasedReattach,
        "cached handles are available but provider metadata is stale",
        Vec::new(),
    ));
    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("session: partial"));
}

// Shared chrome tests keep overlay titles and confirmation styling aligned
// across independently-built presentation views.
#[test]
fn overlay_family_uses_shared_akra_chrome_tokens() {
    /*
     * Overlay views are built by separate modules, so shared chrome cannot be
     * assumed from one renderer. This test samples each view DTO before layout and
     * verifies the common masthead and key-line accent at the data boundary.
     */
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let startup_sample = shell_presentation::ConversationProjectionSample::capture(&app);
    let startup = shell_presentation::build_startup_overlay_view(
        &app,
        startup_sample.parallel_mode_enabled(),
    );
    let sessions_model = SessionOverlayScreenModel::capture(&app);
    let sessions = shell_presentation::build_session_overlay_view(&sessions_model);
    let help = shell_presentation::build_help_overlay_view(TuiLanguage::English);
    app.show_model_selection_overlay();
    let model_selection = shell_presentation::build_model_selection_overlay_view(&app);
    app.show_view_selection_overlay();
    let view_selection = shell_presentation::build_view_selection_overlay_view(&app);
    app.show_language_selection_overlay();
    let language_selection = shell_presentation::build_language_selection_overlay_view(&app);
    let queue = shell_presentation::build_queue_overlay_view(&app);
    let directions = shell_presentation::build_directions_maintenance_overlay_view(&app);
    let supersession_sample = shell_presentation::ConversationProjectionSample::capture(&app);
    let supersession_screen = shell_presentation::ConversationScreenModel::from_app_with_sample(
        &app,
        &supersession_sample,
    );
    let supersession = shell_presentation::build_supersession_overlay_view(
        &supersession_screen,
        &app.shell.supersession_mud_ui_state,
    );
    app.show_planning_init_overlay();
    let planning = shell_presentation::build_planning_init_overlay_view(&app);
    for title in [
        startup.header_lines[0].to_string(),
        sessions.header_lines[0].to_string(),
        help.header_lines[0].to_string(),
        model_selection.header_lines[0].to_string(),
        view_selection.header_lines[0].to_string(),
        language_selection.header_lines[0].to_string(),
        queue.header_lines[0].to_string(),
        directions.header_lines[0].to_string(),
        supersession.header_lines[0].to_string(),
        planning.header_lines[0].to_string(),
    ] {
        assert!(
            title.starts_with("Akra / "),
            "overlay title should carry the shared Akra masthead: {title}"
        );
    }

    assert_eq!(queue.key_lines[0].style.fg, Some(Color::Yellow));
}
#[test]
fn exit_confirmation_uses_shared_akra_chrome() {
    for (width, height) in [(24, 12), (48, 18), (80, 24)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
        let mut app = make_test_app();
        app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
        app.dispatch_shell_chrome(ShellChromeEvent::ExitConfirmationShown);

        terminal
            .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
            .expect("exit confirmation render succeeds");
        let rendered = tui_testkit::screen_text(&terminal);

        assert!(rendered.contains("Akra / Confirm Exit"), "{rendered}");
        assert!(rendered.contains("Exit codex-exec-loop?"), "{rendered}");
        assert!(rendered.contains("y: exit    n: stay"), "{rendered}");
        if width == 80 {
            assert!(rendered.contains("Akra / root"));
        }
        assert!(!rendered.contains("████"));
    }
}
#[test]
fn startup_overlay_surfaces_attachment_mode_and_recovery_anchor() {
    let mut app = make_test_app();
    app.shell.chrome.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let sample = shell_presentation::ConversationProjectionSample::capture(&app);
    let view = crate::adapter::inbound::tui::app::shell_presentation::build_startup_overlay_view(
        &app,
        sample.parallel_mode_enabled(),
    );
    let summary = view
        .summary_lines
        .iter()
        .map(|line: &Line<'static>| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let checks = view
        .check_lines
        .iter()
        .map(|line: &Line<'static>| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(summary.contains("attachment: provider-launched  |  recovery: provider-thread-id"));
    assert!(checks.contains("[ok] attachment mode: provider-launched"));
    assert!(checks.contains("[ok] recovery anchor: provider-thread-id"));
}
