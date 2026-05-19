use super::super::tui_testkit;
use super::*;
use crate::adapter::inbound::tui::app::shell_presentation::{
    format_conversation_lines_for_view, format_conversation_lines_with_debug,
};
use crate::adapter::inbound::tui::app::test_helpers::sample_planning_runtime_projection;
use crate::domain::conversation::ConversationSnapshot;
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterEntry, ParallelModeAgentRosterSnapshot,
    ParallelModeAgentSessionDetailSnapshot, ParallelModeAgentSessionHistoryEntry,
    ParallelModeDistributorSnapshot, ParallelModePoolBoardSnapshot, ParallelModePoolSlotSnapshot,
    ParallelModePoolSlotState, ParallelModeReadinessState, ParallelModeSupervisorDetailSnapshot,
    ParallelModeSupervisorSnapshot, ParallelModeSupervisorState,
};
use crate::domain::recent_sessions::{RecentSessions, SessionCatalog, SessionCatalogTier};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Position;
use ratatui::style::Color;

// Rendering contract tests use TestBackend snapshots instead of golden files so
// each assertion can name the specific TUI invariant it protects.
#[path = "shell_rendering_contract_tests/fixtures.rs"]
mod fixtures;
#[path = "shell_rendering_contract_tests/planning.rs"]
mod planning;

pub(super) use self::fixtures::{
    make_test_app, sample_parallel_mode_snapshot, sample_planning_editor_session, sample_session,
    sample_startup_diagnostics,
};

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
    assert!(default.contains("Tool:"));
    assert!(default.contains("Status:"));

    let simple = format_conversation_lines_for_view(&messages, ConversationViewMode::Simple, false)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(simple.contains("You:"));
    assert!(simple.contains("Codex Commentary:"));
    assert!(simple.contains("Codex:"));
    assert!(!simple.contains("Tool:"));
    assert!(!simple.contains("Status:"));

    let medium = format_conversation_lines_for_view(&messages, ConversationViewMode::Medium, false)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(medium.contains("Tool:"));
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
fn inline_main_buffer_rendering_avoids_box_borders() {
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
    assert!(rendered.contains("Akra  |  thread: new draft  |  turn: idle"));
    assert!(rendered.contains("input: draft"));
    assert!(!rendered.contains("auto: queue/idle"));
    assert!(!rendered.contains("done: 0/20"));
    assert!(!rendered.contains("Plan ready"));
    assert!(!rendered.contains("parallel: off"));
    assert!(!rendered.contains("stable history should stay above the live region"));
    assert!(!rendered.contains("No messages in this thread yet."));
    assert!(!rendered.contains("┌"));
    assert!(!rendered.contains("│"));
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
        .position(|line| {
            line.trim_matches('"')
                .starts_with("Akra  |  thread: new draft  |  turn: idle")
        })
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
    app.show_startup_ascii_art = true;
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(!rendered.contains(".:  .::    .::  .::.: .:::   .::"));
    assert!(!rendered.contains(".::.::  .::   .::    .::  .::   .::"));
    assert!(rendered.contains("Akra  |  Workflows: ready"));
    assert!(rendered.contains("workspace: /tmp/root"));
    assert!(rendered.contains("diagnostics: codex ok  |  app-server ok  |  account ok"));
    assert!(rendered.contains("attachment: provider-launched  |  recovery: provider-thread-id"));
    assert!(rendered.contains("conversation"));
    assert!(rendered.contains("first reply appears here after you send the opening prompt"));
    assert!(rendered.contains("prompt: new thread ready"));
}
#[test]
fn startup_prompt_command_palette_remains_visible_after_colon_input() {
    let mut terminal = tui_testkit::inline_terminal(80, 10);
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start with a ready draft conversation");
    };
    conversation.input_buffer = ":".to_string();
    conversation.sync_inline_shell_command_palette();

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("> :"));
    assert!(rendered.contains("command: palette"));
    assert!(rendered.contains(":diag"));
    assert!(rendered.contains(":peek"));
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
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should stay in a ready conversation state");
    };
    conversation.live_agent_message = None;
    conversation.active_turn_id = None;
    conversation.active_turn_started_at = None;
    conversation.input_state = ConversationInputState::ReadyToContinue;

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
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should stay in a ready conversation state");
    };
    conversation.live_agent_message = None;
    conversation.active_turn_id = None;
    conversation.active_turn_started_at = None;
    conversation.input_state = ConversationInputState::ReadyToContinue;
    app.shell_overlay = ShellOverlay::Startup;
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());

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
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline render succeeds");

    terminal
        .backend_mut()
        .assert_cursor_position(Position::new(2, 14));
}
#[test]
fn inline_queue_overlay_rendering_shows_compact_sections() {
    let mut terminal = tui_testkit::inline_terminal(80, 24);
    let mut app = make_test_app();
    tui_testkit::append_agent_history_message(
        &mut app,
        "stable history stays visible above the queue",
    );
    app.shell_overlay = ShellOverlay::Queue;

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("queue render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Ready Queue"));
    assert!(rendered.contains("Proposals"));
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
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.shell_overlay = ShellOverlay::Startup;

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
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.session_state = SessionState::Ready(
        RecentSessions {
            items: vec![sample_session("thread-1"), sample_session("thread-2")],
            warnings: vec!["cache is stale".to_string()],
            next_cursor: None,
        }
        .into(),
    );
    app.shell_overlay = ShellOverlay::Sessions;

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
fn inline_sessions_inspection_surfaces_attach_only_catalog_without_browser_navigation() {
    /*
     * Attach-only catalogs have enough identity to reattach by handle but not
     * enough metadata for browser navigation. Rendering this tier keeps the
     * session overlay honest about what controls are available.
     */
    let mut terminal = Terminal::new(TestBackend::new(96, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.session_state = SessionState::Ready(SessionCatalog::unsupported(
        SessionCatalogTier::AttachOnly,
        "session listing is unsupported for this bridge",
        vec!["manual attach only".to_string()],
    ));
    app.shell_overlay = ShellOverlay::Sessions;

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline attach-only session inspection render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("catalog tier: attach-only"));
    assert!(rendered.contains("session listing is unsupported"));
    assert!(rendered.contains("manual attach only"));
    assert!(rendered.contains("Recent-session navigation requires a queryable catalog surface."));
}
#[test]
fn inline_help_inspection_renders_command_help() {
    let mut terminal = Terminal::new(TestBackend::new(96, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.shell_overlay = ShellOverlay::Help;

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline help inspection render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Shell Commands / inline inspection"));
    assert!(rendered.contains(":diag"));
    assert!(rendered.contains("diagnostics"));
    assert!(rendered.contains(":turns"));
    assert!(rendered.contains("auto turn budget"));
    assert!(!rendered.contains(":auto"));
    assert!(rendered.contains("Esc/Ctrl+C: close"));
    assert!(!rendered.contains("Shell commands: :diag  :parallel"));
    assert!(!rendered.contains("Transcript /"));
    assert!(!rendered.contains("┌"));
}

#[test]
fn inline_model_selection_inspection_renders_model_and_effort_picker() {
    let mut terminal = Terminal::new(TestBackend::new(104, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
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
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
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
fn inline_supersession_inspection_renders_prepare_panels_inside_shell_frame() {
    let mut terminal = Terminal::new(TestBackend::new(96, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_readiness_snapshot_for_test(Some(sample_parallel_mode_snapshot(
        ParallelModeReadinessState::Degraded,
    )));
    app.shell_overlay = ShellOverlay::Supersession;

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline supersession inspection render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Parallel Mode / inline inspection"));
    assert!(rendered.contains("board: supervise"));
    assert!(rendered.contains("Basic Info"));
    assert!(rendered.contains("Distributor"));
    assert!(rendered.contains("Pool"));
    assert!(rendered.contains("Orchestrator"));
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
                .with_thread_id(Some("thread-guardian".to_string())),
                ParallelModeAgentRosterEntry::new(
                    "agent-builder",
                    "Build peek rows",
                    "slot-2",
                    "akra-agent/slot-2/build-peek-rows",
                    "starting",
                    "active",
                    "starting the second worker",
                )
                .with_thread_id(Some("thread-builder".to_string())),
                ParallelModeAgentRosterEntry::new(
                    "agent-reviewer",
                    "Review peek rows",
                    "slot-3",
                    "akra-agent/slot-3/review-peek-rows",
                    "commit_ready",
                    "official",
                    "official completion is waiting for delivery",
                )
                .with_thread_id(Some("thread-reviewer".to_string())),
            ],
            "empty",
        ),
        ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
        None,
    )));
    app.shell_overlay = ShellOverlay::ParallelPeek;

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
    app.shell_overlay = ShellOverlay::ParallelPeek;
    app.parallel_peek_overlay_ui_state.open_preview(
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
    app.shell_overlay = ShellOverlay::ParallelPeek;

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
    app.parallel_peek_overlay_ui_state.open_preview(
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

    app.parallel_peek_overlay_ui_state
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
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
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
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
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
    conversation.refresh_conversation_lines();

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline parallel home render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Parallel Mode / inline inspection"));
    assert!(rendered.contains("Parallel Event Stream"));
    assert!(rendered.contains("You: 안녕하세요"));
    assert!(!rendered.contains("Operator: first user word"));
    assert!(!rendered.contains("Codex:"));
    assert!(!rendered.contains("single mode reply must not own"));
    assert!(!rendered.contains("┌"));
}

#[test]
fn inline_parallel_home_suppresses_startup_banner_on_empty_draft() {
    let mut terminal = Terminal::new(TestBackend::new(104, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.show_startup_ascii_art = true;
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
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

    assert!(rendered.contains("Parallel Mode / inline inspection"));
    assert!(rendered.contains("Parallel Event Stream"));
    assert!(rendered.contains("parallel: ready  |  mode: parallel"));
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
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
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
    app.shell_overlay = ShellOverlay::Supersession;
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("expected ready conversation state");
    };
    conversation.input_buffer = "안녕하세요?".to_string();

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline supersession prompt render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("> 안녕하세요?"));
    assert!(rendered.contains("buffered prompt  |  Enter send  |  Ctrl+j nl"));
    assert!(
        !rendered.contains("now: none"),
        "planning detail rows should be clipped before they can hide the prompt:\n{rendered}"
    );
    assert!(!rendered.contains("┌"));
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
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
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
    app.shell_overlay = ShellOverlay::Supersession;

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
    app.shell_overlay = ShellOverlay::Supersession;

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline supersession timeline render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(!rendered.contains("Parallel Event Stream"));
    assert!(!rendered.contains("Recent Parallel Events"));
    assert!(rendered.contains("Distributor: slot-1"));
    assert!(rendered.contains("Agent agent-1: Timeline UI"));
    assert!(rendered.contains("Ledger: Timeline UI"));
    assert!(rendered.contains("head: idle"));
    assert!(!rendered.contains("commit_ready"));
}

#[test]
fn inline_tail_adds_only_spinner_to_prompt_during_parallel_loading() {
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.shell_overlay = ShellOverlay::Supersession;
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
        [
            "⠋ >", "⠙ >", "⠹ >", "⠸ >", "⠼ >", "⠴ >", "⠦ >", "⠧ >", "⠇ >", "⠏ >"
        ]
        .iter()
        .any(|frame| rendered.contains(frame))
    );
    assert!(!rendered.contains("thinking"));
}

#[test]
fn inline_parallel_home_keeps_loading_spinner_when_overlay_hidden() {
    let mut terminal = Terminal::new(TestBackend::new(104, 28)).expect("test terminal");
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
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

    assert!(rendered.contains("Parallel Mode / inline inspection"));
    assert!(rendered.contains("prompt locked while parallel loading is active"));
    assert!(
        [
            "⠋ >", "⠙ >", "⠹ >", "⠸ >", "⠼ >", "⠴ >", "⠦ >", "⠧ >", "⠇ >", "⠏ >"
        ]
        .iter()
        .any(|frame| rendered.contains(frame))
    );
}

#[test]
fn inline_tail_omits_parallel_loading_spinner_after_empty_non_loading_snapshot() {
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.shell_overlay = ShellOverlay::Supersession;
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
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
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

    assert!(rendered.contains("parallel: ready  |  mode: parallel"));
    assert!(rendered.contains("agents: 0 active"));
    assert!(rendered.contains("queue: idle"));
    assert!(rendered.contains("parallel alert:"));
}

#[test]
fn inline_tail_omits_legacy_planning_valid_status_in_single_and_parallel_home() {
    fn rendered_tail(parallel_mode_enabled: bool) -> String {
        let mut app = make_test_app();
        app.startup_state = StartupState::Ready(sample_startup_diagnostics());
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
    assert!(single_rendered.contains("queue: queue head: rank 1 / task-1"));
    assert!(single_rendered.contains("now: Implement shell planning status"));

    let parallel_rendered = rendered_tail(true);
    assert!(!parallel_rendered.contains("planning: valid"));
    assert!(parallel_rendered.contains("parallel: ready  |  mode: parallel"));
    assert!(parallel_rendered.contains("queue: queue head: rank 1 / task-1"));
    assert!(parallel_rendered.contains("now: Implement shell planning status"));
}

#[test]
fn inline_tail_places_parallel_slot_working_line_between_queue_and_prompt() {
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
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
        .position(|line| line.contains("queue: queue head: rank 1 / task-1"))
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
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.session_state = SessionState::Ready(SessionCatalog::partial(
        SessionCatalogTier::HandleBasedReattach,
        "cached handles are available but provider metadata is stale",
        Vec::new(),
    ));
    let rendered = build_inline_tail_lines(&app)
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("handle-based reattach: partial catalog"));
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
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let startup = shell_presentation::build_startup_overlay_view(&app);
    let sessions = shell_presentation::build_session_overlay_view(&app);
    let help = shell_presentation::build_help_overlay_view();
    app.show_model_selection_overlay();
    let model_selection = shell_presentation::build_model_selection_overlay_view(&app);
    app.show_view_selection_overlay();
    let view_selection = shell_presentation::build_view_selection_overlay_view(&app);
    let queue = shell_presentation::build_queue_overlay_view(&app);
    let directions = shell_presentation::build_directions_maintenance_overlay_view(&app);
    let supersession = shell_presentation::build_supersession_overlay_view(&app);
    app.show_planning_init_overlay();
    let planning = shell_presentation::build_planning_init_overlay_view(&app);
    for title in [
        startup.header_lines[0].to_string(),
        sessions.header_lines[0].to_string(),
        help.header_lines[0].to_string(),
        model_selection.header_lines[0].to_string(),
        view_selection.header_lines[0].to_string(),
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
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    app.dispatch_shell_chrome(ShellChromeEvent::ExitConfirmationShown);

    terminal
        .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
        .expect("exit confirmation render succeeds");
    let rendered = tui_testkit::screen_text(&terminal);

    assert!(rendered.contains("Akra / Confirm Exit"));
    assert!(rendered.contains("Exit codex-exec-loop?"));
}
#[test]
fn startup_overlay_surfaces_attachment_mode_and_recovery_anchor() {
    let mut app = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics());
    let view =
        crate::adapter::inbound::tui::app::shell_presentation::build_startup_overlay_view(&app);
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
