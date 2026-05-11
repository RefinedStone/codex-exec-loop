use super::super::tui_testkit;
use super::*;
use crate::adapter::inbound::tui::app::shell_presentation::format_conversation_lines_with_debug;
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
    assert!(rendered.contains("auto: queue/idle"));
    assert!(rendered.contains("done: 0/20"));
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
    assert!(rendered.contains(":queue"));
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

    assert!(rendered.contains("Supersession / inline inspection"));
    assert!(rendered.contains("supervisor: supervise"));
    assert!(rendered.contains("Capabilities"));
    assert!(rendered.contains("Pool Board"));
    assert!(rendered.contains("Agent Roster"));
    assert!(rendered.contains("Distributor Queue"));
    assert!(rendered.contains("loading pool board"));
    assert!(rendered.contains("loading agent roster"));
    assert!(rendered.contains("loading distributor board"));
    assert!(rendered.contains("row shape: agent / task / slot"));
    assert!(!rendered.contains("Transcript /"));
    assert!(!rendered.contains("┌"));
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
fn inline_supersession_narrow_snapshot_keeps_selected_timeline_visible() {
    /*
     * The timeline is the first MUD-style read-only slice: slot/agent topology
     * stays in pool and roster panels, while the selected session lifecycle must
     * remain visible in a narrow inline inspection without adding new controls.
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

    assert!(rendered.contains("timeline: slot-1 /"));
    assert!(rendered.contains("events: 00:00 assigned"));
    assert!(rendered.contains("00:02 official"));
    assert!(rendered.contains("last event: 00:02 official"));
    assert!(rendered.contains("pool board: >[slot-1:RUN]<"));
    assert!(rendered.contains("distributor queue: head idle"));
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
    let queue = shell_presentation::build_queue_overlay_view(&app);
    let directions = shell_presentation::build_directions_maintenance_overlay_view(&app);
    let supersession = shell_presentation::build_supersession_overlay_view(&app);
    app.show_planning_init_overlay();
    let planning = shell_presentation::build_planning_init_overlay_view(&app);
    let task_intake = shell_presentation::build_task_intake_overlay_view(&app);
    for title in [
        startup.header_lines[0].to_string(),
        sessions.header_lines[0].to_string(),
        help.header_lines[0].to_string(),
        queue.header_lines[0].to_string(),
        directions.header_lines[0].to_string(),
        supersession.header_lines[0].to_string(),
        planning.header_lines[0].to_string(),
        task_intake.header_lines[0].to_string(),
    ] {
        assert!(
            title.starts_with("Akra / "),
            "overlay title should carry the shared Akra masthead: {title}"
        );
    }

    assert_eq!(queue.key_lines[0].style.fg, Some(Color::Yellow));
    assert_eq!(task_intake.key_lines[0].style.fg, Some(Color::Yellow));
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
