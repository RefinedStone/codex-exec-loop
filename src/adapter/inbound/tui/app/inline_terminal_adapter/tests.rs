use super::super::tui_testkit;
use super::{
    FrameCacheState, HistoryInsertionMode, InlineConversationFrameProjection, InlineResizeBackend,
    InlineTerminalBackend, InlineTerminalState, ShellRuntime, TerminalViewportState,
    current_inline_history_lines, draw_inline_frame, draw_inline_transaction, sync_inline_viewport,
    terminal_options_for_render_mode,
};
use crate::adapter::inbound::tui::app::conversation_input::InputCursorMovement;
use crate::adapter::inbound::tui::app::ratatui_frontend::prepare_runtime_for_due_draw;
use crate::adapter::inbound::tui::app::shell_presentation::{
    ConversationProjectionSample, ConversationScreenModel, TurnSteerConfirmationScreenModel,
    build_inline_live_transcript_lines,
};
use crate::adapter::inbound::tui::app::{
    ConversationIntentEvent, ConversationLifecycleEvent, ConversationMessage,
    ConversationMessageKind, ConversationState, ConversationViewMode, ConversationViewModel,
    INLINE_VIEWPORT_HEIGHT, InlineHistoryRenderMode, NativeTuiApp, PlanningWorkerVisibility,
    ProgressiveActivityDetailKind, TuiLanguage,
};
use crate::adapter::inbound::tui::shell_chrome::{ShellChromeEvent, ShellOverlay};
use crate::application::port::outbound::github_review_poller_port::GithubReviewPollerPort;
use crate::application::service::github_review_poller_service::GithubReviewPollerService;
use crate::core::app::{
    ActiveTurnPhase, ActiveTurnSnapshot, ApprovalAuthorityPhase, ApprovalAuthoritySnapshot,
    ConversationSnapshot as CoreConversationSnapshot, CorePromptOrigin, PostTurnAuthoritySnapshot,
    PostTurnEvaluationCorrelation, PostTurnRouteResolution, TurnSubmissionCorrelation,
};
use crate::domain::conversation::{
    ConversationApprovalRequest, ConversationApprovalRequestKind,
    ConversationSnapshot as DomainConversationSnapshot, ConversationTurnSteerRequest,
};
use crate::domain::github_review::{GithubPullRequestActivitySnapshot, GithubPullRequestTarget};
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterEntry, ParallelModeAgentRosterSnapshot,
    ParallelModeAgentSessionDetailSnapshot, ParallelModeAgentSessionHistoryEntry,
    ParallelModeCompletionFeedEntry, ParallelModeDistributorSnapshot,
    ParallelModePoolBoardSnapshot, ParallelModePoolSlotSnapshot, ParallelModePoolSlotState,
    ParallelModeRuntimeEventFeedEntry, ParallelModeSupervisorDetailSnapshot,
    ParallelModeSupervisorSnapshot, ParallelModeSupervisorState,
};
use crate::domain::session_summary::SessionSummary;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::{Backend, ClearType, TestBackend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Rect, Size};
use ratatui::{Terminal, Viewport};
use std::cell::Cell as StdCell;
use std::collections::VecDeque;
use std::convert::Infallible;
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

fn projected_live_transcript_lines(app: &NativeTuiApp) -> Vec<ratatui::text::Line<'static>> {
    let screen_model = ConversationScreenModel::from_app(app);
    build_inline_live_transcript_lines(&screen_model)
}

fn frame_projection(app: &NativeTuiApp, width: u16) -> InlineConversationFrameProjection {
    InlineConversationFrameProjection::from_app(app, width)
}

fn frame_cache_should_draw(
    cache: &mut FrameCacheState,
    app: &NativeTuiApp,
    viewport: &TerminalViewportState,
    width: u16,
    height: u16,
) -> bool {
    cache.should_draw_inline_frame(&frame_projection(app, width), viewport, width, height)
}

fn inline_state_should_draw(
    state: &mut InlineTerminalState,
    app: &NativeTuiApp,
    width: u16,
    height: u16,
) -> bool {
    state.should_draw_inline_frame(&frame_projection(app, width), width, height)
}

// These tests pin the terminal-adapter contract between committed host
// history and the live inline tail. They intentionally exercise both
// TestBackend and VT100-backed paths because resize/scrollback behavior
// differs by backend.
#[path = "tests/fixtures.rs"]
mod fixtures;
#[path = "tests/history_flush.rs"]
mod history_flush;
use self::fixtures::{make_test_app, make_test_app_with_github_review_setup_loader};

fn set_running_turn(conversation: &mut ConversationViewModel, turn_id: &str) {
    let mut snapshot = conversation.runtime_snapshot().clone();
    snapshot.active_turn = Some(ActiveTurnSnapshot {
        correlation: TurnSubmissionCorrelation::new(1),
        phase: ActiveTurnPhase::Running,
        workspace_directory: conversation.cwd.clone(),
        turn_id: Some(turn_id.to_string()),
        prompt_origin: CorePromptOrigin::Manual,
        started_at: Instant::now(),
    });
    conversation.apply_runtime_snapshot(snapshot);
    conversation.record_turn_started(turn_id.to_string());
}

fn set_pending_approval(
    conversation: &mut ConversationViewModel,
    request: ConversationApprovalRequest,
) {
    let mut snapshot = conversation.runtime_snapshot().clone();
    snapshot.approval = Some(ApprovalAuthoritySnapshot {
        request,
        decision: None,
        phase: ApprovalAuthorityPhase::Pending,
    });
    conversation.apply_runtime_snapshot(snapshot);
}

fn begin_post_turn_evaluation(conversation: &mut ConversationViewModel, turn_id: &str) {
    let workspace_directory = conversation.cwd.clone();
    let mut snapshot = conversation.runtime_snapshot().clone();
    snapshot.active_turn = None;
    snapshot.post_turn = PostTurnAuthoritySnapshot::Evaluating {
        correlation: PostTurnEvaluationCorrelation::new(
            1,
            conversation.thread_id.clone(),
            turn_id,
            workspace_directory.clone(),
            workspace_directory,
        ),
        started_at: Instant::now(),
    };
    conversation.apply_runtime_snapshot(snapshot);
    conversation.begin_post_turn_settlement(turn_id);
}

fn settle_post_turn(conversation: &mut ConversationViewModel, turn_id: &str) {
    let workspace_directory = conversation.cwd.clone();
    let mut snapshot = conversation.runtime_snapshot().clone();
    snapshot.active_turn = None;
    snapshot.post_turn = PostTurnAuthoritySnapshot::Settled {
        correlation: PostTurnEvaluationCorrelation::new(
            1,
            conversation.thread_id.clone(),
            turn_id,
            workspace_directory.clone(),
            workspace_directory,
        ),
        resolution: PostTurnRouteResolution::NoContinuation,
    };
    conversation.apply_runtime_snapshot(snapshot);
}

struct FirstFrameGithubReviewPollerPort;

impl GithubReviewPollerPort for FirstFrameGithubReviewPollerPort {
    fn load_pull_request_activity(
        &self,
        target: &GithubPullRequestTarget,
    ) -> anyhow::Result<GithubPullRequestActivitySnapshot> {
        Ok(GithubPullRequestActivitySnapshot {
            target: target.clone(),
            title: "First-frame review".to_string(),
            url: "https://example.invalid/acme/widgets/pull/42".to_string(),
            head_branch: "feature".to_string(),
            base_branch: "prerelease".to_string(),
            events: Vec::new(),
        })
    }
}

#[test]
fn github_setup_starts_once_only_after_the_first_successful_frame() {
    let setup_calls = Arc::new(AtomicUsize::new(0));
    let calls = setup_calls.clone();
    let (setup_started_tx, setup_started_rx) = mpsc::sync_channel(1);
    let mut app = make_test_app_with_github_review_setup_loader(move |request| {
        calls.fetch_add(1, Ordering::SeqCst);
        setup_started_tx
            .send(())
            .expect("setup start signal should have a receiver");
        thread::sleep(Duration::from_millis(650));
        let target = request
            .mode
            .explicit_target()
            .expect("fixture setup should be explicit")
            .clone();
        Ok(Some((
            target,
            GithubReviewPollerService::new(Arc::new(FirstFrameGithubReviewPollerPort)),
        )))
    });
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::ViewportReplay;
    let mut inner = CursorQueryCountingBackend::new(TestBackend::new(80, 40));
    inner
        .set_cursor_position(Position::new(0, 39))
        .expect("fixture cursor should start at the physical bottom");
    let backend = InlineTerminalBackend::new(inner);
    let mut terminal = Terminal::with_options(
        backend,
        terminal_options_for_render_mode(InlineHistoryRenderMode::ViewportReplay),
    )
    .expect("inline terminal should initialize");
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    terminal
        .backend_mut()
        .inner_mut()
        .resize_on_next_flush(48, 10);
    assert!(
        !draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("resize-raced first draw should return cleanly")
    );
    assert_eq!(
        setup_calls.load(Ordering::SeqCst),
        0,
        "a failed draw must not start setup"
    );

    let first_success_started = Instant::now();
    assert!(
        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("next stable draw should succeed")
    );
    assert!(
        first_success_started.elapsed() < Duration::from_millis(300),
        "first successful frame must not wait for the 650ms setup provider"
    );
    setup_started_rx
        .recv_timeout(Duration::from_millis(300))
        .expect("successful frame should start setup");
    assert_eq!(setup_calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        runtime.app().runtime.github_review_polling_state,
        super::super::github_polling::GithubReviewPollingState::Discovering { .. }
    ));

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('x'),
        KeyModifiers::NONE,
    )));
    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("fixture should keep a ready conversation");
    };
    assert_eq!(conversation.composer.input_buffer, "x");
    assert!(
        runtime.take_redraw_request(),
        "input and setup-start state must remain redrawable while setup is blocked"
    );
    assert!(
        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("input redraw should succeed while setup is blocked")
    );
    assert_eq!(
        setup_calls.load(Ordering::SeqCst),
        1,
        "later successful draws must not duplicate setup"
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline
        && matches!(
            runtime.app().runtime.github_review_polling_state,
            super::super::github_polling::GithubReviewPollingState::Discovering { .. }
        )
    {
        runtime.poll_background_messages();
        thread::sleep(Duration::from_millis(5));
    }
    assert!(matches!(
        runtime.app().runtime.github_review_polling_state,
        super::super::github_polling::GithubReviewPollingState::Active(_)
    ));
}

// Host history sync must insert only committed transcript rows; live agent
// deltas stay in the active tail until the turn is completed.
#[test]
fn host_history_sync_keeps_live_agent_delta_out_of_inserted_history() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 160, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    append_history_message(&mut app, "committed answer belongs in host history");
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should start in a ready conversation state");
    };
    set_running_turn(conversation, "turn-1");
    conversation.push_live_agent_delta(
        "agent-live".to_string(),
        Some("final_answer".to_string()),
        "live answer stays in tail".to_string(),
    );
    let mut runtime = ShellRuntime::new(app);
    let mut inline_viewport = InlineTerminalState::default();

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_viewport).unwrap());
    let inserted_history = tui_testkit::inline_terminal_history_text(&terminal);
    assert!(inserted_history.contains("committed answer belongs in host history"));
    assert!(!inserted_history.contains("live answer stays in tail"));

    draw_test_frame(&mut terminal, &mut runtime, &mut inline_viewport);
    let live_frame = tui_testkit::screen_text(&terminal);
    assert!(live_frame.contains("live answer stays in tail"));
}

#[test]
fn host_history_sync_keeps_progressive_activity_rail_transient() {
    let secret = "AKRA_PROGRESSIVE_SCROLLBACK_SECRET";
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 160, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    append_history_message(&mut app, "committed history remains durable");
    tui_testkit::set_progressive_command_activity(
        &mut app,
        &format!("{secret}\nsecond line"),
        false,
    );
    let mut runtime = ShellRuntime::new(app);
    let mut inline_viewport = InlineTerminalState::default();
    let mut frames = tui_testkit::InlineFrameRecorder::default();

    frames.draw_and_record(
        "wide-live",
        &mut terminal,
        &mut runtime,
        &mut inline_viewport,
    );
    tui_testkit::resize_inline_history_terminal(&mut terminal, 48, 10);
    frames.draw_and_record(
        "narrow-live",
        &mut terminal,
        &mut runtime,
        &mut inline_viewport,
    );
    tui_testkit::resize_inline_history_terminal(&mut terminal, 160, 24);
    frames.draw_and_record(
        "restored-live",
        &mut terminal,
        &mut runtime,
        &mut inline_viewport,
    );

    for label in ["wide-live", "narrow-live", "restored-live"] {
        let frame = frames.frame(label);
        assert!(frame.screen_text.contains("notice: activity:"), "{label}");
        assert!(frame.screen_text.contains("active:command"), "{label}");
        assert!(!frame.screen_text.contains(secret), "{label}");
        assert!(!frame.host_scrollback_text.contains("activity:"), "{label}");
        assert!(!frame.host_scrollback_text.contains(secret), "{label}");
    }
    for label in ["wide-live", "restored-live"] {
        let screen = &frames.frame(label).screen_text;
        assert!(screen.contains("cmd:2"), "{label}");
        assert!(screen.contains("model:gpt-5.5"), "{label}");
        assert!(screen.contains("task:P0-D3 rail"), "{label}");
        assert!(!screen.contains("lane:"), "{label}");
        assert!(!screen.contains("requested-model-hidden"), "{label}");
    }
    let narrow = &frames.frame("narrow-live").screen_text;
    assert!(!narrow.contains("cmd:2"));
    assert!(!narrow.contains("model:"));
    assert!(!narrow.contains("task:"));
    assert!(!narrow.contains("lane:"));
    assert!(
        frames
            .frame("wide-live")
            .terminal_history_text
            .contains("committed history remains durable")
    );

    let ConversationState::Ready(conversation) =
        &mut runtime.app_mut().conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready conversation state");
    };
    conversation.fail_turn(Some("turn-1"), "command failed".to_string());
    frames.draw_and_record("cleared", &mut terminal, &mut runtime, &mut inline_viewport);
    let cleared = frames.frame("cleared");
    assert!(
        cleared
            .screen_text
            .contains("notice: activity: terminal:runtime-failed")
    );
    assert!(!cleared.screen_text.contains("cmd:2"));
    assert!(!cleared.screen_text.contains("active:command"));
    assert!(
        cleared
            .terminal_history_text
            .contains("terminal:runtime-failed")
    );
    assert!(!cleared.terminal_history_text.contains("cmd:2"));
    assert!(!cleared.terminal_history_text.contains("active:command"));
    assert!(!cleared.host_scrollback_text.contains("activity:"));
    assert!(!cleared.terminal_history_text.contains(secret));
    assert!(!cleared.host_scrollback_text.contains(secret));
}

#[test]
fn vt100_progressive_activity_rail_stays_transient_across_resize() {
    let secret = "AKRA_PROGRESSIVE_VT100_SECRET";
    let mut terminal = tui_testkit::inline_history_vt100_terminal(
        InlineHistoryRenderMode::HostScrollback,
        160,
        24,
    );
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    append_history_message(&mut app, "committed VT100 history remains durable");
    tui_testkit::set_progressive_command_activity(
        &mut app,
        &format!("{secret}\nsecond line"),
        false,
    );
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    for (width, height) in [(160, 24), (48, 10), (160, 24)] {
        tui_testkit::resize_inline_history_vt100_terminal(&mut terminal, width, height);
        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("progressive VT100 draw transaction");
        let screen = tui_testkit::screen_text(&terminal);
        assert!(screen.contains("notice: activity:"), "{width}x{height}");
        assert!(screen.contains("active:command"), "{width}x{height}");
        assert!(!screen.contains(secret), "{width}x{height}");
        if width == 160 {
            assert!(screen.contains("cmd:2"), "{width}x{height}");
            assert!(screen.contains("model:gpt-5.5"), "{width}x{height}");
            assert!(screen.contains("task:P0-D3 rail"), "{width}x{height}");
            assert!(!screen.contains("lane:"), "{width}x{height}");
            assert!(
                !screen.contains("requested-model-hidden"),
                "{width}x{height}"
            );
        } else {
            assert!(!screen.contains("cmd:2"), "{width}x{height}");
            assert!(!screen.contains("model:"), "{width}x{height}");
            assert!(!screen.contains("task:"), "{width}x{height}");
            assert!(!screen.contains("lane:"), "{width}x{height}");
        }
        let host_scrollback = tui_testkit::inline_vt100_host_scrollback_text(&mut terminal);
        assert!(!host_scrollback.contains("activity:"), "{width}x{height}");
        assert!(!host_scrollback.contains(secret), "{width}x{height}");
    }

    let ConversationState::Ready(conversation) =
        &mut runtime.app_mut().conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready conversation state");
    };
    conversation.fail_turn(None, "command failed".to_string());
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("cleared progressive VT100 draw transaction");
    let screen = tui_testkit::screen_text(&terminal);
    assert!(screen.contains("notice: activity: terminal:runtime-failed"));
    assert!(!screen.contains("cmd:2"));
    assert!(!screen.contains("active:command"));
    let host_scrollback = tui_testkit::inline_vt100_host_scrollback_text(&mut terminal);
    assert!(!host_scrollback.contains("activity:"));
    assert!(!host_scrollback.contains(secret));
}

#[test]
fn activity_inspector_pages_resize_and_approval_stay_out_of_host_scrollback() {
    let secret = "AKRA_ACTIVITY_INSPECTOR_SCROLLBACK_SECRET";
    let detail = format!(
        "{secret}\u{1b}[31m\n{}",
        (0..160)
            .map(|index| format!("bounded activity row {index:03}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    append_history_message(&mut app, "committed history remains durable");
    let _core_snapshot = tui_testkit::set_progressive_command_activity(&mut app, &detail, true);
    assert!(app.show_progressive_activity_overlay(ProgressiveActivityDetailKind::Diff));
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();
    let mut frames = tui_testkit::InlineFrameRecorder::default();

    frames.draw_and_record("open", &mut terminal, &mut runtime, &mut inline_terminal);
    let next_page_start = runtime
        .app()
        .shell
        .progressive_activity_overlay_ui_state
        .next_page_start()
        .expect("long detail should expose a second page");
    let next_page_fragment = detail[next_page_start..]
        .lines()
        .next()
        .expect("second page should start inside retained detail")
        .to_string();
    assert!(
        runtime
            .app_mut()
            .handle_shell_overlay_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE,))
    );
    assert_eq!(
        runtime
            .app()
            .shell
            .progressive_activity_overlay_ui_state
            .current_page_start(),
        next_page_start
    );
    frames.draw_and_record("paged", &mut terminal, &mut runtime, &mut inline_terminal);
    assert!(
        runtime
            .app_mut()
            .handle_shell_overlay_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE,))
    );
    frames.draw_and_record("page-up", &mut terminal, &mut runtime, &mut inline_terminal);
    assert!(
        runtime
            .app_mut()
            .handle_shell_overlay_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE,))
    );
    assert!(
        runtime
            .app_mut()
            .handle_shell_overlay_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE,))
    );
    frames.draw_and_record("home", &mut terminal, &mut runtime, &mut inline_terminal);
    assert!(
        runtime
            .app_mut()
            .handle_shell_overlay_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE,))
    );
    tui_testkit::resize_inline_history_terminal(&mut terminal, 48, 10);
    frames.draw_and_record("narrow", &mut terminal, &mut runtime, &mut inline_terminal);
    assert_eq!(
        runtime
            .app()
            .shell
            .progressive_activity_overlay_ui_state
            .current_page_start(),
        0
    );
    tui_testkit::resize_inline_history_terminal(&mut terminal, 80, 24);
    frames.draw_and_record(
        "restored",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );

    let ConversationState::Ready(conversation) =
        &mut runtime.app_mut().conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready conversation state");
    };
    set_pending_approval(
        conversation,
        ConversationApprovalRequest {
            approval_id: "approval-activity".to_string(),
            server_request_id: "server-activity".to_string(),
            method: "item/fileChange/requestApproval".to_string(),
            kind: ConversationApprovalRequestKind::FileChange,
            summary: "Review the pending file change.".to_string(),
            details: vec!["A bounded approval detail remains inspectable.".to_string()],
        },
    );
    runtime
        .app_mut()
        .dispatch_shell_chrome(ShellChromeEvent::ApprovalOverlayShown);
    frames.draw_and_record(
        "approval",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    runtime
        .app_mut()
        .dispatch_shell_chrome(ShellChromeEvent::ApprovalOverlayClosed);
    frames.draw_and_record("closed", &mut terminal, &mut runtime, &mut inline_terminal);

    let open = frames.frame("open");
    assert!(open.screen_text.contains(secret));
    assert!(open.screen_text.contains("\\x1b[31m"));
    assert!(!open.screen_text.contains('\u{1b}'));
    assert!(
        open.screen_text.contains("| 0-") || open.screen_text.contains("diff ·"),
        "{}",
        open.screen_text
    );
    let paged = frames.frame("paged");
    assert!(
        paged.screen_text.contains(&format!("| {next_page_start}-")),
        "expected paged detail window at {next_page_start}: {}",
        paged.screen_text
    );
    assert!(paged.screen_text.contains(&next_page_fragment));
    // Card list headers may still show the bounded title summary, but the paged
    // detail body must have moved past the opening secret fragment.
    assert!(
        !paged.screen_text.contains(&format!("{secret}\\x1b[31m"))
            || paged.screen_text.contains(&next_page_fragment),
        "{}",
        paged.screen_text
    );
    assert!(frames.frame("page-up").screen_text.contains(secret));
    assert!(frames.frame("page-up").screen_text.contains("| 0-"));
    assert!(frames.frame("home").screen_text.contains(secret));
    assert!(frames.frame("home").screen_text.contains("| 0-"));
    assert!(frames.frame("narrow").screen_text.contains(secret));
    // Narrow viewports keep the card/filter chrome; detail byte range may be clipped.
    assert!(
        frames.frame("narrow").screen_text.contains("filter:")
            || frames.frame("narrow").screen_text.contains("diff"),
        "{}",
        frames.frame("narrow").screen_text
    );
    assert!(
        frames.frame("restored").screen_text.contains("| 0-")
            || frames.frame("restored").screen_text.contains("filter:"),
        "{}",
        frames.frame("restored").screen_text
    );
    assert!(
        frames
            .frame("approval")
            .screen_text
            .contains("Approval Required")
    );
    assert!(
        !frames
            .frame("approval")
            .screen_text
            .contains("Activity / inline inspection")
    );
    assert!(
        !frames
            .frame("closed")
            .screen_text
            .contains("Activity / inline inspection")
    );
    for label in [
        "open", "paged", "page-up", "home", "narrow", "restored", "approval", "closed",
    ] {
        let frame = frames.frame(label);
        assert!(!frame.host_scrollback_text.contains(secret), "{label}");
        assert!(
            !frame
                .host_scrollback_text
                .contains("Activity / inline inspection"),
            "{label}"
        );
        assert!(
            !frame.host_scrollback_text.contains("activity row"),
            "{label}"
        );
    }
    assert!(
        frames
            .frame("open")
            .terminal_history_text
            .contains("committed history remains durable")
    );
}

#[test]
fn structured_activity_timeline_survives_retry_redraw_and_resize_without_scrollback_leak() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 120, 30);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    append_history_message(
        &mut app,
        "durable transcript stays outside the activity overlay",
    );
    let fixture = tui_testkit::set_structured_activity_timeline(&mut app);
    assert!(app.show_progressive_activity_overlay_all());
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();
    let mut frames = tui_testkit::InlineFrameRecorder::default();

    frames.draw_and_record(
        "initial-timeline",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    frames.draw_and_record(
        "steady-redraw",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    let ConversationState::Ready(conversation) =
        &mut runtime.app_mut().conversation.lifecycle.conversation_state
    else {
        panic!("structured activity frame fixture should remain ready");
    };
    conversation
        .progressive_activity
        .record_turn_retrying("server overloaded; retry scheduled");
    frames.draw_and_record(
        "retrying",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    let ConversationState::Ready(conversation) =
        &mut runtime.app_mut().conversation.lifecycle.conversation_state
    else {
        panic!("structured activity frame fixture should remain ready");
    };
    conversation.progressive_activity.clear_turn_retrying();
    tui_testkit::resize_inline_history_terminal(&mut terminal, 80, 30);
    frames.draw_and_record(
        "narrow-after-retry",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );

    for label in [
        "initial-timeline",
        "steady-redraw",
        "retrying",
        "narrow-after-retry",
    ] {
        let frame = frames.frame(label);
        assert!(frame.screen_text.contains("complete"), "{label}");
        assert!(frame.screen_text.contains("failed"), "{label}");
        assert!(
            !frame
                .host_scrollback_text
                .contains("Activity / inline inspection"),
            "{label}"
        );
        assert!(
            !frame
                .host_scrollback_text
                .contains("Reviewing retry and approval paths"),
            "{label}"
        );
    }
    for label in ["initial-timeline", "steady-redraw", "retrying"] {
        let frame = frames.frame(label);
        assert!(frame.screen_text.contains("active"), "{label}");
        assert!(frame.screen_text.contains("2.5s"), "{label}");
        assert!(frame.screen_text.contains("800ms"), "{label}");
    }
    assert!(
        frames
            .frame("initial-timeline")
            .screen_text
            .contains("waiting for subagent")
    );
    assert_eq!(
        frames
            .frame("steady-redraw")
            .screen_text
            .matches("Strict lint failed")
            .count(),
        1
    );
    assert!(
        frames
            .frame("retrying")
            .screen_text
            .contains("retrying turn")
    );
    assert!(
        !frames
            .frame("retrying")
            .screen_text
            .contains("waiting for subagent")
    );
    assert!(
        frames
            .frame("narrow-after-retry")
            .screen_text
            .contains("waiting for subagent")
    );
    assert_eq!(std::sync::Arc::strong_count(&fixture.progressive), 1);
    assert_eq!(std::sync::Arc::strong_count(&fixture.lifecycle), 1);
}

#[test]
fn vt100_activity_inspector_stays_transient_through_resize_and_approval() {
    let secret = "AKRA_ACTIVITY_INSPECTOR_VT100_SECRET";
    let detail = format!(
        "{secret}\u{1b}[32m\n{}",
        (0..80)
            .map(|index| format!("vt100 activity row {index:03}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    let _core_snapshot = tui_testkit::set_progressive_command_activity(&mut app, &detail, false);
    assert!(app.show_progressive_activity_overlay(ProgressiveActivityDetailKind::Output));
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    for (width, height) in [(80, 24), (48, 10), (80, 24)] {
        tui_testkit::resize_inline_history_vt100_terminal(&mut terminal, width, height);
        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("activity VT100 draw transaction");
        let screen = tui_testkit::screen_text(&terminal);
        assert!(
            screen.contains("command ·")
                || screen.contains("| 0-")
                || screen.contains("filter: command")
                || screen.contains("> Output"),
            "{width}x{height}: {screen:?}"
        );
        assert!(!screen.contains('\u{1b}'), "{width}x{height}");
        let host_scrollback = tui_testkit::inline_vt100_host_scrollback_text(&mut terminal);
        assert!(!host_scrollback.contains(secret), "{width}x{height}");
        assert!(
            !host_scrollback.contains("vt100 activity row"),
            "{width}x{height}"
        );
    }

    let ConversationState::Ready(conversation) =
        &mut runtime.app_mut().conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready conversation state");
    };
    set_pending_approval(
        conversation,
        ConversationApprovalRequest {
            approval_id: "approval-vt100-activity".to_string(),
            server_request_id: "server-vt100-activity".to_string(),
            method: "item/commandExecution/requestApproval".to_string(),
            kind: ConversationApprovalRequestKind::CommandExecution,
            summary: "Review the VT100 command request.".to_string(),
            details: vec!["Command: cargo test --lib".to_string()],
        },
    );
    runtime
        .app_mut()
        .dispatch_shell_chrome(ShellChromeEvent::ApprovalOverlayShown);
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("approval-preempted activity VT100 draw transaction");
    let approval_screen = tui_testkit::screen_text(&terminal);
    assert!(approval_screen.contains("Approval Required"));
    assert!(approval_screen.contains("Y: approve once"));
    assert!(
        !approval_screen.contains("Activity / inline inspection"),
        "{approval_screen}"
    );
    let host_scrollback = tui_testkit::inline_vt100_host_scrollback_text(&mut terminal);
    assert!(!host_scrollback.contains(secret));
    assert!(!host_scrollback.contains("vt100 activity row"));
}

// Any direct history insertion invalidates ratatui's back buffer. The next
// frame draw must rebuild the buffer before incremental diffs are trusted.
#[test]
fn history_insert_invalidates_back_buffer_until_frame_draw() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.history_insert_mode = HistoryInsertionMode::StandardScrollRegion;
    append_history_message(&mut app, "history insert should invalidate diff state");
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());

    assert_eq!(
        inline_terminal.last_known_screen_size(),
        Some(Size {
            width: 80,
            height: 24
        })
    );
    assert_eq!(
        inline_terminal.viewport_area(),
        Some(terminal.get_frame().area())
    );
    assert_eq!(
        inline_terminal.insert_mode(),
        HistoryInsertionMode::StandardScrollRegion
    );
    assert_eq!(
        inline_terminal.last_known_cursor_pos(),
        Some(terminal.get_cursor_position().unwrap())
    );
    assert!(!inline_terminal.back_buffer_trustworthy());

    draw_test_frame(&mut terminal, &mut runtime, &mut inline_terminal);

    assert!(inline_terminal.back_buffer_trustworthy());
}

// Newline fallback uses a different insertion path, but it must leave the same
// redraw contract as scroll-region insertion.
#[test]
fn newline_fallback_history_insert_invalidates_back_buffer() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.history_insert_mode = HistoryInsertionMode::NewlineFallback;
    append_history_message(&mut app, "newline fallback committed history");
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());

    assert_eq!(
        inline_terminal.insert_mode(),
        HistoryInsertionMode::NewlineFallback
    );
    assert_eq!(
        inline_terminal.last_known_cursor_pos(),
        Some(terminal.get_cursor_position().unwrap())
    );
    assert!(!inline_terminal.back_buffer_trustworthy());

    draw_test_frame(&mut terminal, &mut runtime, &mut inline_terminal);

    assert!(inline_terminal.back_buffer_trustworthy());
    assert!(
        tui_testkit::inline_terminal_history_text(&terminal)
            .contains("newline fallback committed history")
    );
}

// A draw transaction flushes committed history and live tail together while
// preserving the split between host scrollback and active viewport content.
#[test]
fn draw_transaction_flushes_history_and_live_tail_together() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    append_history_message(&mut app, "committed history in transaction");
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should start in a ready conversation state");
    };
    set_running_turn(conversation, "turn-1");
    conversation.push_live_agent_delta(
        "agent-live".to_string(),
        Some("final_answer".to_string()),
        "live tail in same transaction".to_string(),
    );
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("draw transaction");
    let terminal_history = tui_testkit::inline_terminal_history_text(&terminal);
    assert!(terminal_history.contains("committed history in transaction"));
    let screen_text = tui_testkit::screen_text(&terminal);
    assert!(screen_text.contains("live tail in same transaction"));
    assert!(inline_terminal.back_buffer_trustworthy());
    assert!(
        !tui_testkit::inline_scrollback_text(&terminal).contains("live tail in same transaction")
    );
}

#[test]
fn conversation_identity_switch_replays_shared_prefix_once_without_loading_frame() {
    for insert_mode in [
        HistoryInsertionMode::StandardScrollRegion,
        HistoryInsertionMode::NewlineFallback,
    ] {
        assert_conversation_identity_switch_replays_shared_prefix_once(insert_mode);
    }
}

fn assert_conversation_identity_switch_replays_shared_prefix_once(
    insert_mode: HistoryInsertionMode,
) {
    const COMMON_PREFIX: &str = "THREAD_COMMON_PREFIX";
    const SHARED_AGENT_ANSWER: &str = "SHARED_AGENT_ANSWER";
    const THREAD_B_SUFFIX: &str = "THREAD_B_SUFFIX";
    const SAME_THREAD_SUFFIX: &str = "SAME_THREAD_SUFFIX";
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 48, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.shell.history_insert_mode = insert_mode;
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should start ready");
    };
    conversation.thread_id = "thread-a".to_string();
    conversation.title = "Thread A".to_string();
    append_user_history_message(
        &mut app,
        &format!("{COMMON_PREFIX} / 한글 재연결 공통 문장이 좁은 화면에서 줄바꿈됩니다"),
    );
    append_history_message(&mut app, SHARED_AGENT_ANSWER);
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();
    let mut frames = tui_testkit::InlineFrameRecorder::default();

    frames.draw_and_record(
        "thread-a",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    assert_eq!(
        frames
            .frame("thread-a")
            .terminal_history_text
            .matches(COMMON_PREFIX)
            .count(),
        1
    );

    let next_messages = {
        let ConversationState::Ready(conversation) =
            &runtime.app().conversation.lifecycle.conversation_state
        else {
            panic!("thread A should remain ready");
        };
        let mut messages = conversation.messages.clone();
        messages.push(ConversationMessage::new(
            ConversationMessageKind::Agent,
            THREAD_B_SUFFIX.to_string(),
            None,
            None,
        ));
        messages
    };
    load_history_snapshot(runtime.app_mut(), "thread-b", next_messages);

    frames.draw_and_record(
        "thread-b-without-loading-frame",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    let switched = frames.frame("thread-b-without-loading-frame");
    assert_eq!(
        switched
            .terminal_history_text
            .matches(COMMON_PREFIX)
            .count(),
        2,
        "a different conversation must deliver its complete shared prefix:\n{}",
        switched.terminal_history_text
    );
    assert_eq!(
        switched
            .terminal_history_text
            .matches(THREAD_B_SUFFIX)
            .count(),
        1
    );
    assert_eq!(
        switched
            .terminal_history_text
            .matches(SHARED_AGENT_ANSWER)
            .count(),
        2,
        "the complete shared prefix must be delivered once per conversation"
    );
    assert!(!switched.host_scrollback_text.contains("prompt:"));

    let same_thread_snapshot = {
        let ConversationState::Ready(conversation) =
            &runtime.app().conversation.lifecycle.conversation_state
        else {
            panic!("thread B should remain ready");
        };
        (**conversation).clone()
    };
    runtime.app_mut().conversation.lifecycle.conversation_state = ConversationState::Loading;
    frames.draw_and_record(
        "same-thread-loading",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    runtime.app_mut().conversation.lifecycle.conversation_state =
        ConversationState::ready(same_thread_snapshot);
    frames.draw_and_record(
        "same-thread-reattached",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    let reattached = frames.frame("same-thread-reattached");
    assert_eq!(
        reattached
            .terminal_history_text
            .matches(COMMON_PREFIX)
            .count(),
        2
    );
    assert_eq!(
        reattached
            .terminal_history_text
            .matches(THREAD_B_SUFFIX)
            .count(),
        1,
        "a transient same-thread Loading frame must not replay committed history"
    );
    assert_eq!(
        reattached
            .terminal_history_text
            .matches(SHARED_AGENT_ANSWER)
            .count(),
        2
    );

    append_history_message(runtime.app_mut(), SAME_THREAD_SUFFIX);
    frames.draw_and_record(
        "same-thread-append",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    frames.draw_and_record(
        "stable-same-thread-redraw",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    let stable = frames.frame("stable-same-thread-redraw");
    assert_eq!(
        stable.terminal_history_text.matches(COMMON_PREFIX).count(),
        2
    );
    assert_eq!(
        stable
            .terminal_history_text
            .matches(THREAD_B_SUFFIX)
            .count(),
        1
    );
    assert_eq!(
        stable
            .terminal_history_text
            .matches(SAME_THREAD_SUFFIX)
            .count(),
        1
    );
    assert_eq!(
        stable
            .terminal_history_text
            .matches(SHARED_AGENT_ANSWER)
            .count(),
        2
    );
    assert!(inline_terminal.back_buffer_trustworthy());
}

fn released_handoff_app(
    render_mode: InlineHistoryRenderMode,
    parallel_mode_enabled: bool,
    shell_overlay: ShellOverlay,
) -> NativeTuiApp {
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = render_mode;
    app.shell.chrome.shell_overlay = shell_overlay;
    append_user_history_message(&mut app, "released handoff prompt");
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should start in a ready conversation state");
    };
    set_running_turn(conversation, "turn-handoff-redraw");
    conversation.push_live_agent_delta(
        "agent-handoff-redraw".to_string(),
        Some("final_answer".to_string()),
        "released handoff answer".to_string(),
    );
    assert!(conversation.complete_live_agent_message(
        "agent-handoff-redraw".to_string(),
        Some("final_answer".to_string()),
        "released handoff answer".to_string(),
    ));
    conversation.finish_turn("turn-handoff-redraw", &[]);
    begin_post_turn_evaluation(conversation, "turn-handoff-redraw");
    settle_post_turn(conversation, "turn-handoff-redraw");
    assert!(conversation.complete_post_turn_settlement("turn-handoff-redraw"));
    app.set_parallel_mode_enabled_for_test(parallel_mode_enabled);
    app
}

#[test]
fn successful_viewport_handoff_ack_schedules_one_follow_up_frame() {
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::ViewportReplay, 80, 24);
    let app = released_handoff_app(
        InlineHistoryRenderMode::ViewportReplay,
        true,
        ShellOverlay::Hidden,
    );
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();
    assert!(
        runtime.take_redraw_request(),
        "consume initial frame request"
    );

    assert!(
        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("handoff delivery frame")
    );
    let delivered_screen = tui_testkit::screen_text(&terminal);
    assert!(delivered_screen.contains("released handoff answer"));
    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("handoff delivery should keep a ready conversation");
    };
    assert!(!conversation.has_pending_viewport_transcript_handoff());
    assert!(
        !inline_terminal.back_buffer_trustworthy(),
        "ACK must invalidate the frame that still contains the handoff"
    );
    assert!(
        runtime.take_redraw_request(),
        "successful ACK must schedule the post-handoff frame"
    );
    assert!(
        !runtime.take_redraw_request(),
        "one ACK must schedule only one immediate frame"
    );

    assert!(
        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("post-handoff frame")
    );
    assert!(
        inline_terminal.back_buffer_trustworthy(),
        "the scheduled frame must reconcile the post-handoff viewport"
    );
    assert!(
        !runtime.take_redraw_request(),
        "the post-handoff frame must not schedule another ACK draw"
    );
}

#[test]
fn parallel_host_handoff_reprojects_overlay_frame_after_ack() {
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let app = released_handoff_app(
        InlineHistoryRenderMode::HostScrollback,
        true,
        ShellOverlay::Help,
    );
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();
    assert!(
        runtime.take_redraw_request(),
        "consume initial frame request"
    );

    assert!(
        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("parallel host handoff frame")
    );
    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("parallel host delivery should keep a ready conversation");
    };
    assert!(!conversation.has_pending_viewport_transcript_handoff());
    let delivered_screen = tui_testkit::screen_text(&terminal);
    assert!(delivered_screen.contains("Response held while the dialog is open"));
    assert!(
        runtime.take_redraw_request(),
        "host ACK must schedule the post-handoff parallel frame"
    );
    assert!(!runtime.take_redraw_request());

    assert!(
        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("parallel post-handoff overlay frame")
    );
    let settled_screen = tui_testkit::screen_text(&terminal);
    assert!(
        !settled_screen.contains("Response held while the dialog is open"),
        "the scheduled overlay frame must reflect the host ACK: {settled_screen}"
    );
    assert!(!runtime.take_redraw_request());
}

#[test]
fn parallel_handoff_draw_resize_preserves_retry_backoff() {
    let mut inner = CursorQueryCountingBackend::new(TestBackend::new(80, 40));
    inner
        .set_cursor_position(Position::new(0, 39))
        .expect("fixture cursor should start at the physical bottom");
    let backend = InlineTerminalBackend::new(inner);
    let mut terminal = Terminal::with_options(
        backend,
        terminal_options_for_render_mode(InlineHistoryRenderMode::HostScrollback),
    )
    .expect("parallel host terminal should initialize");
    let app = released_handoff_app(
        InlineHistoryRenderMode::HostScrollback,
        true,
        ShellOverlay::Help,
    );
    let sample = ConversationProjectionSample::capture(&app);
    let delivered_conversation = super::parallel_conversation_handoff_projection(&app, &sample)
        .expect("released handoff should have a conversation projection");
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();
    inline_terminal.history_flush.rendered_lines = delivered_conversation.lines;
    assert!(
        runtime.take_redraw_request(),
        "consume initial frame request"
    );
    terminal
        .backend_mut()
        .inner_mut()
        .resize_on_next_flush(48, 10);

    assert!(
        !draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("draw-time resize should defer the post-handoff frame")
    );
    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("parallel host delivery should keep a ready conversation");
    };
    assert!(!conversation.has_pending_viewport_transcript_handoff());
    assert!(
        !runtime.take_redraw_request(),
        "post-ACK redraw must not bypass the resize retry delay"
    );
    assert_resize_retry_scheduled(
        &mut runtime,
        "draw-time resize must retain the delayed post-ACK retry",
    );
}

#[test]
fn stale_conversation_identity_receipt_cannot_ack_current_handoff() {
    let app = released_handoff_app(
        InlineHistoryRenderMode::HostScrollback,
        false,
        ShellOverlay::Hidden,
    );
    let sample = ConversationProjectionSample::capture(&app);
    let stale_token = super::TranscriptHandoffDeliveryToken::from_sample(&sample)
        .expect("released handoff should have a delivery token");
    let mut runtime = ShellRuntime::new(app);

    runtime
        .app_mut()
        .conversation
        .conversation_history_identity_revision = runtime
        .app()
        .conversation
        .conversation_history_identity_revision
        .wrapping_add(1)
        .max(1);
    assert!(!super::acknowledge_transcript_handoff_after_delivery(
        &mut runtime,
        Some(&stale_token),
    ));
    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("test app should retain its ready conversation");
    };
    assert!(conversation.has_pending_viewport_transcript_handoff());

    let current_sample = ConversationProjectionSample::capture(runtime.app());
    let current_token = super::TranscriptHandoffDeliveryToken::from_sample(&current_sample)
        .expect("current handoff should have a refreshed delivery token");
    assert!(super::acknowledge_transcript_handoff_after_delivery(
        &mut runtime,
        Some(&current_token),
    ));
}

#[test]
fn failed_terminal_draw_keeps_handoff_pending_and_forces_retry() {
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::ViewportReplay, 80, 24);
    let app = released_handoff_app(
        InlineHistoryRenderMode::ViewportReplay,
        false,
        ShellOverlay::Hidden,
    );
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();
    runtime
        .app_mut()
        .planning
        .queue_overlay_ui_state
        .bind_receipt_undo_hit_area(Some(Rect::new(4, 7, 14, 1)));
    terminal.backend_mut().inner_mut().fail_next_draw();

    assert!(
        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal).is_err(),
        "an injected terminal draw error must escape the transaction"
    );
    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("test app should retain its ready conversation");
    };
    assert!(conversation.has_pending_viewport_transcript_handoff());
    assert!(!inline_terminal.back_buffer_trustworthy());
    assert_eq!(inline_terminal.latest_frame_render_attempt(), 1);
    assert_eq!(inline_terminal.last_committed_frame_render_attempt(), None);
    assert_eq!(
        runtime
            .app()
            .planning
            .queue_overlay_ui_state
            .receipt_undo_hit_area(),
        None,
        "a failed draw must fail closed instead of retaining clickable geometry"
    );

    assert!(
        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("the unchanged handoff should redraw after the transient failure")
    );
    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("test app should retain its ready conversation");
    };
    assert!(!conversation.has_pending_viewport_transcript_handoff());
    assert_eq!(inline_terminal.latest_frame_render_attempt(), 2);
    assert_eq!(
        inline_terminal.last_committed_frame_render_attempt(),
        Some(2),
        "the stable retry must commit only its own render receipt"
    );
}

#[test]
fn failed_terminal_flush_discards_render_receipt_and_stable_retry_commits_once() {
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::ViewportReplay, 80, 24);
    let app = released_handoff_app(
        InlineHistoryRenderMode::ViewportReplay,
        false,
        ShellOverlay::Hidden,
    );
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();
    runtime
        .app_mut()
        .planning
        .queue_overlay_ui_state
        .bind_receipt_undo_hit_area(Some(Rect::new(4, 7, 14, 1)));
    terminal.backend_mut().inner_mut().fail_next_flush();

    assert!(
        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal).is_err(),
        "an injected terminal flush error must escape the transaction"
    );
    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("test app should retain its ready conversation");
    };
    assert!(conversation.has_pending_viewport_transcript_handoff());
    assert_eq!(inline_terminal.latest_frame_render_attempt(), 1);
    assert_eq!(inline_terminal.last_committed_frame_render_attempt(), None);
    assert_eq!(
        runtime
            .app()
            .planning
            .queue_overlay_ui_state
            .receipt_undo_hit_area(),
        None,
        "a failed flush cannot authorize a hit target from an unproven frame"
    );
    assert!(!inline_terminal.back_buffer_trustworthy());

    assert!(
        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("the unchanged frame should redraw after the transient flush failure")
    );
    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("stable retry should retain its ready conversation");
    };
    assert!(!conversation.has_pending_viewport_transcript_handoff());
    assert_eq!(inline_terminal.latest_frame_render_attempt(), 2);
    assert_eq!(
        inline_terminal.last_committed_frame_render_attempt(),
        Some(2)
    );
}

#[test]
fn frame_attempt_gate_rejects_stale_and_duplicate_receipts() {
    let mut app = make_test_app();
    app.shell.chrome.shell_overlay = ShellOverlay::Help;
    app.shell.help_scroll_offset = usize::MAX;
    let area = Rect::new(0, 0, 80, 24);
    let capture_receipt = |app: &NativeTuiApp| {
        let projection = frame_projection(app, area.width);
        let model = super::capture_inline_shell_frame_model(
            app,
            super::ShellFrontendMode::InlineMainBuffer,
            area,
            projection,
        );
        let (_, _, receipt) = model.into_parts();
        receipt
    };
    let mut inline_terminal = InlineTerminalState::default();

    let receipt_a = capture_receipt(&app);
    let attempt_a = inline_terminal.begin_frame_render_attempt();
    let receipt_b = capture_receipt(&app);
    let duplicate_receipt_b = capture_receipt(&app);
    let attempt_b = inline_terminal.begin_frame_render_attempt();
    let initial_help_scroll = app.shell.help_scroll_offset;

    assert!(!inline_terminal.commit_frame_render_receipt(
        super::PendingInlineFrameRenderReceipt {
            attempt: attempt_a,
            receipt: receipt_a,
        },
        |receipt| super::apply_inline_frame_render_receipt(&mut app, receipt),
    ));
    assert_eq!(app.shell.help_scroll_offset, initial_help_scroll);
    assert_eq!(inline_terminal.last_committed_frame_render_attempt(), None);

    assert!(inline_terminal.commit_frame_render_receipt(
        super::PendingInlineFrameRenderReceipt {
            attempt: attempt_b,
            receipt: receipt_b,
        },
        |receipt| super::apply_inline_frame_render_receipt(&mut app, receipt),
    ));
    let committed_help_scroll = app.shell.help_scroll_offset;
    assert!(committed_help_scroll < initial_help_scroll);
    assert_eq!(
        inline_terminal.last_committed_frame_render_attempt(),
        Some(2)
    );

    assert!(!inline_terminal.commit_frame_render_receipt(
        super::PendingInlineFrameRenderReceipt {
            attempt: attempt_b,
            receipt: duplicate_receipt_b,
        },
        |receipt| super::apply_inline_frame_render_receipt(&mut app, receipt),
    ));
    assert_eq!(app.shell.help_scroll_offset, committed_help_scroll);
    assert_eq!(
        inline_terminal.last_committed_frame_render_attempt(),
        Some(2)
    );
}

#[test]
fn post_draw_size_failure_discards_render_receipt_and_hit_target() {
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::ViewportReplay, 80, 24);
    let app = released_handoff_app(
        InlineHistoryRenderMode::ViewportReplay,
        false,
        ShellOverlay::Hidden,
    );
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
    let width = terminal.size().expect("terminal size").width;
    let projection = frame_projection(runtime.app(), width);
    let resize_snapshot = terminal
        .backend()
        .resize_snapshot()
        .expect("stable pre-draw resize snapshot");
    runtime
        .app_mut()
        .planning
        .queue_overlay_ui_state
        .bind_receipt_undo_hit_area(Some(Rect::new(4, 7, 14, 1)));
    let draw_calls_before = terminal.backend().inner().draw_call_count();
    // Arm the first size proof after backend draw. Terminal::draw must finish
    // its cell write and flush before the adapter can accept the receipt.
    terminal
        .backend_mut()
        .inner_mut()
        .fail_size_after_next_draw();

    assert!(
        draw_inline_frame(
            &mut terminal,
            &mut runtime,
            &mut inline_terminal,
            projection,
            false,
            resize_snapshot,
        )
        .is_err(),
        "a post-draw size error must escape without applying its receipt"
    );
    assert_eq!(
        terminal.backend().inner().draw_call_count(),
        draw_calls_before + 1,
        "the injected size error must happen after backend draw"
    );
    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("test app should retain its ready conversation");
    };
    assert!(conversation.has_pending_viewport_transcript_handoff());
    assert_eq!(inline_terminal.latest_frame_render_attempt(), 1);
    assert_eq!(inline_terminal.last_committed_frame_render_attempt(), None);
    assert_eq!(
        runtime
            .app()
            .planning
            .queue_overlay_ui_state
            .receipt_undo_hit_area(),
        None,
        "a failed post-draw geometry proof must clear clickable geometry"
    );
    assert!(!inline_terminal.back_buffer_trustworthy());

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
    assert!(draw_test_frame(
        &mut terminal,
        &mut runtime,
        &mut inline_terminal
    ));
    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("stable retry should retain its ready conversation");
    };
    assert!(!conversation.has_pending_viewport_transcript_handoff());
    assert_eq!(inline_terminal.latest_frame_render_attempt(), 2);
    assert_eq!(
        inline_terminal.last_committed_frame_render_attempt(),
        Some(2),
        "the stable retry must be the first committed receipt"
    );
}

#[test]
fn completed_agent_handoff_flushes_at_settlement_and_only_once() {
    const FINAL_MARKER: &str = "FINAL_ANSWER_HANDOFF_MARKER";
    const TOOL_MARKER: &str = "ORDERED_TOOL_SUFFIX_MARKER";
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::HostScrollback, 48, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.conversation.conversation_view_mode = ConversationViewMode::Detail;
    append_user_history_message(&mut app, "handoff prompt");
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.thread_id = "thread-handoff".to_string();
    set_running_turn(conversation, "turn-handoff");
    conversation.push_live_agent_delta(
        "agent-handoff".to_string(),
        Some("final_answer".to_string()),
        format!("{FINAL_MARKER} streaming"),
    );
    conversation.buffer_tool_message(TOOL_MARKER);
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("streaming handoff draw transaction");
    let ConversationState::Ready(conversation) =
        &mut runtime.app_mut().conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready conversation state");
    };
    assert!(conversation.complete_live_agent_message(
        "agent-handoff".to_string(),
        Some("final_answer".to_string()),
        format!("{FINAL_MARKER} final"),
    ));

    // Agent completion and turn completion are distinct app-server transactions.
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("agent completion handoff draw transaction");
    let completed_screen = tui_testkit::screen_text(&terminal);
    let completed_host = tui_testkit::inline_vt100_host_scrollback_text(&mut terminal);
    assert_eq!(completed_screen.matches(FINAL_MARKER).count(), 1);
    assert_eq!(completed_host.matches(FINAL_MARKER).count(), 0);
    assert_eq!(
        inline_terminal
            .history_flush
            .rendered_lines
            .iter()
            .filter(|line| line.to_string().contains(FINAL_MARKER))
            .count(),
        0
    );

    let ConversationState::Ready(conversation) =
        &mut runtime.app_mut().conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready conversation state");
    };
    conversation.finish_turn("turn-handoff", &[]);
    begin_post_turn_evaluation(conversation, "turn-handoff");
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("settlement handoff draw transaction");
    let settlement_screen = tui_testkit::screen_text(&terminal);
    let settlement_host = tui_testkit::inline_vt100_host_scrollback_text(&mut terminal);
    assert_eq!(settlement_screen.matches(FINAL_MARKER).count(), 0);
    assert_eq!(settlement_host.matches(FINAL_MARKER).count(), 1);
    assert_eq!(
        settlement_screen.matches("settling planning queue").count(),
        1
    );
    assert!(!settlement_screen.contains("auto:"));
    assert!(!settlement_screen.contains("done:"));
    assert!(!settlement_screen.contains("status: turn completed"));
    assert!(!settlement_screen.contains("Enter send"));
    assert!(!settlement_screen.contains("Enter when ready"));
    assert!(settlement_screen.find("◦ Working") < settlement_screen.rfind("╭ Task"));

    let ConversationState::Ready(conversation) =
        &mut runtime.app_mut().conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready conversation state");
    };
    assert!(!conversation.has_pending_viewport_transcript_handoff());
    settle_post_turn(conversation, "turn-handoff");
    assert!(conversation.complete_post_turn_settlement("turn-handoff"));
    assert!(!conversation.has_pending_viewport_transcript_handoff());
    assert!(conversation.can_accept_manual_prompt());
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("released handoff draw transaction");
    let released_screen = tui_testkit::screen_text(&terminal);
    assert!(projected_live_transcript_lines(runtime.app()).is_empty());
    assert!(released_screen.matches(FINAL_MARKER).count() <= 1);
    assert_eq!(
        inline_terminal
            .history_flush
            .rendered_lines
            .iter()
            .filter(|line| line.to_string().contains(FINAL_MARKER))
            .count(),
        1
    );
    let released_history = inline_terminal
        .history_flush
        .rendered_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(released_history.matches(TOOL_MARKER).count(), 1);
    assert!(released_history.find(FINAL_MARKER) < released_history.find(TOOL_MARKER));

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("stable released handoff draw transaction");
    let stable_history = tui_testkit::inline_vt100_scrollback_text(&mut terminal);
    assert_eq!(stable_history.matches(FINAL_MARKER).count(), 1);
}

#[test]
fn settlement_flushes_long_completed_answer_to_host_scrollback() {
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should start in a ready conversation state");
    };
    set_running_turn(conversation, "turn-long-settlement");
    let answer = (0..80)
        .map(|index| match index {
            0 => "SETTLEMENT_LONG_MARKER_FIRST".to_string(),
            40 => "SETTLEMENT_LONG_MARKER_MIDDLE".to_string(),
            79 => "SETTLEMENT_LONG_MARKER_LAST".to_string(),
            _ => format!("settlement completion filler {index}"),
        })
        .collect::<Vec<_>>()
        .join("\n");
    conversation.complete_live_agent_message(
        "agent-long-settlement".to_string(),
        Some("final_answer".to_string()),
        answer,
    );
    conversation.finish_turn("turn-long-settlement", &[]);
    begin_post_turn_evaluation(conversation, "turn-long-settlement");
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("settlement draw transaction");

    let host_scrollback = tui_testkit::inline_vt100_host_scrollback_text(&mut terminal);
    for marker in [
        "SETTLEMENT_LONG_MARKER_FIRST",
        "SETTLEMENT_LONG_MARKER_MIDDLE",
        "SETTLEMENT_LONG_MARKER_LAST",
    ] {
        assert!(
            host_scrollback.contains(marker),
            "settlement withheld {marker} inside the fixed inline viewport:\n{host_scrollback}"
        );
    }
    assert!(
        tui_testkit::screen_text(&terminal).contains("settling planning queue"),
        "settlement status should remain visible after the answer handoff"
    );
}

#[test]
fn manual_preparation_failure_is_delivered_before_new_draft_can_replace_it() {
    const PROMPT_MARKER: &str = "FAILED_PREPARATION_PROMPT";
    const FAILURE_STATUS: &str = "turn preparation failed / workspace unavailable";
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::HostScrollback, 48, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should start in a ready conversation state");
    };
    conversation
        .record_manual_preparation_failure(PROMPT_MARKER.to_string(), FAILURE_STATUS.to_string());
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    runtime
        .app_mut()
        .dispatch_conversation_intent(ConversationIntentEvent::NewDraftRequested);
    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("blocked navigation should keep the failed draft ready");
    };
    assert!(conversation.has_pending_viewport_transcript_handoff());
    assert!(
        conversation
            .messages
            .iter()
            .any(|message| message.text == PROMPT_MARKER)
    );
    assert!(
        conversation
            .status_text
            .starts_with("conversation is busy;")
    );

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("manual preparation failure draw transaction");
    let delivered_history = tui_testkit::inline_vt100_scrollback_text(&mut terminal);
    assert_eq!(delivered_history.matches(PROMPT_MARKER).count(), 1);
    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("history delivery should keep the failed draft ready");
    };
    assert!(!conversation.has_pending_viewport_transcript_handoff());
    assert!(conversation.can_accept_manual_prompt());
    assert_eq!(conversation.status_text, FAILURE_STATUS);

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("stable manual preparation failure draw transaction");
    let stable_history = tui_testkit::inline_vt100_scrollback_text(&mut terminal);
    assert_eq!(stable_history.matches(PROMPT_MARKER).count(), 1);
}

#[test]
fn viewport_replay_does_not_duplicate_completed_agent_handoff() {
    const FINAL_MARKER: &str = "VIEWPORT_REPLAY_FINAL_MARKER";
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::ViewportReplay, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::ViewportReplay;
    append_user_history_message(&mut app, "viewport replay prompt");
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should start in a ready conversation state");
    };
    set_running_turn(conversation, "turn-viewport-replay");
    conversation.push_live_agent_delta(
        "agent-viewport-replay".to_string(),
        Some("final_answer".to_string()),
        format!("{FINAL_MARKER} streaming"),
    );
    conversation.complete_live_agent_message(
        "agent-viewport-replay".to_string(),
        Some("final_answer".to_string()),
        format!("{FINAL_MARKER} final"),
    );
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("viewport replay completion draw transaction");

    let screen = tui_testkit::screen_text(&terminal);
    assert_eq!(screen.matches(FINAL_MARKER).count(), 1, "{screen}");
    let live_handoff = projected_live_transcript_lines(runtime.app())
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(live_handoff.matches(FINAL_MARKER).count(), 1);

    let ConversationState::Ready(conversation) =
        &mut runtime.app_mut().conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready conversation state");
    };
    conversation.finish_turn("turn-viewport-replay", &[]);
    begin_post_turn_evaluation(conversation, "turn-viewport-replay");
    settle_post_turn(conversation, "turn-viewport-replay");
    assert!(conversation.complete_post_turn_settlement("turn-viewport-replay"));
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("viewport replay release draw transaction");

    let released_screen = tui_testkit::screen_text(&terminal);
    assert_eq!(released_screen.matches(FINAL_MARKER).count(), 1);
    assert!(projected_live_transcript_lines(runtime.app()).is_empty());
}

#[test]
fn frame_cache_invalidates_when_only_live_agent_text_changes() {
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should start in a ready conversation state");
    };
    set_running_turn(conversation, "turn-cache");
    conversation.push_live_agent_delta(
        "agent-cache".to_string(),
        Some("final_answer".to_string()),
        "first".to_string(),
    );
    let mut cache = FrameCacheState::default();
    let viewport = TerminalViewportState::default();

    assert!(frame_cache_should_draw(&mut cache, &app, &viewport, 80, 24));
    assert!(!frame_cache_should_draw(
        &mut cache, &app, &viewport, 80, 24
    ));
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready conversation state");
    };
    conversation.push_live_agent_delta(
        "agent-cache".to_string(),
        Some("final_answer".to_string()),
        " second".to_string(),
    );

    assert!(frame_cache_should_draw(&mut cache, &app, &viewport, 80, 24));
}

#[test]
fn width_resize_uses_host_guard_rows_when_the_cursor_does_not_move() {
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    let mut state = InlineTerminalState::default();
    state.record_terminal_viewport(
        Size::new(120, 30),
        Rect::new(0, 1, 120, 4),
        Position::new(29, 3),
    );
    assert!(inline_state_should_draw(&mut state, &app, 120, 30));

    state.observe_physical_resize_reflow(Size::new(48, 18), Position::new(29, 3), true);
    assert_eq!(
        state.resize_reflow_rows_to_clear(),
        2,
        "the host separators must protect cleanup when tmux reflows without moving the cursor"
    );

    state.mark_frame_drawn(
        Size::new(48, 18),
        Rect::new(0, 1, 48, 4),
        Position::new(29, 3),
    );
    assert_eq!(
        state.resize_reflow_rows_to_clear(),
        0,
        "a delivered frame consumes the one-shot reflow cleanup"
    );
    assert!(inline_state_should_draw(&mut state, &app, 48, 18));

    state.observe_physical_resize_reflow(Size::new(120, 30), Position::new(29, 3), true);
    assert_eq!(
        state.resize_reflow_rows_to_clear(),
        2,
        "restoring width must also remove the former narrow HUD row above the viewport"
    );

    state.observe_physical_resize_reflow(Size::new(48, 18), Position::new(29, 9), false);
    assert_eq!(
        state.resize_reflow_rows_to_clear(),
        0,
        "viewport replay has no durable host separator and must not clear above its viewport"
    );
}

#[test]
fn frame_cache_invalidates_when_only_handoff_delivery_phase_changes() {
    let app = released_handoff_app(
        InlineHistoryRenderMode::ViewportReplay,
        false,
        ShellOverlay::Hidden,
    );
    let mut held = frame_projection(&app, 80);
    let released = frame_projection(&app, 80);
    assert!(released.renders_viewport_transcript_handoff);
    held.renders_viewport_transcript_handoff = false;
    assert_eq!(held.tail_view.lines, released.tail_view.lines);
    assert_eq!(held.live_transcript_lines, released.live_transcript_lines);

    let mut cache = FrameCacheState::default();
    let viewport = TerminalViewportState::default();
    assert!(cache.should_draw_inline_frame(&held, &viewport, 80, 24));
    assert!(!cache.should_draw_inline_frame(&held, &viewport, 80, 24));
    assert!(cache.should_draw_inline_frame(&released, &viewport, 80, 24));
}

#[test]
fn frame_cache_never_reuses_a_dialog_frame_and_redraws_after_close() {
    let app = make_test_app();
    let mut cache = FrameCacheState::default();
    let viewport = TerminalViewportState::default();
    let baseline = frame_projection(&app, 80);

    assert!(cache.should_draw_inline_frame(&baseline, &viewport, 80, 24));
    assert!(!cache.should_draw_inline_frame(&baseline, &viewport, 80, 24));

    let mut dialog = frame_projection(&app, 80);
    dialog.turn_steer_confirmation = Some(Box::new(TurnSteerConfirmationScreenModel {
        language: TuiLanguage::English,
        request: ConversationTurnSteerRequest {
            thread_id: "thread-cache".to_string(),
            expected_turn_id: "turn-cache".to_string(),
            prompt: "steer cache frame".to_string(),
        },
    }));
    assert!(cache.should_draw_inline_frame(&dialog, &viewport, 80, 24));
    assert!(cache.should_draw_inline_frame(&dialog, &viewport, 80, 24));

    let restored = frame_projection(&app, 80);
    assert!(cache.should_draw_inline_frame(&restored, &viewport, 80, 24));
}

#[test]
fn frame_cache_invalidates_when_only_the_cjk_prompt_cursor_moves() {
    let mut app = make_test_app();
    assert!(app.insert_input_text("가나다".to_string()));
    let mut cache = FrameCacheState::default();
    let viewport = TerminalViewportState::default();
    let before = frame_projection(&app, 80);

    assert!(cache.should_draw_inline_frame(&before, &viewport, 80, 24));
    assert!(!cache.should_draw_inline_frame(&before, &viewport, 80, 24));

    app.move_input_cursor(InputCursorMovement::PreviousCharacter);
    let after = frame_projection(&app, 80);
    assert_eq!(before.tail_view.lines, after.tail_view.lines);
    assert_ne!(
        before.tail_view.prompt_cursor_offset,
        after.tail_view.prompt_cursor_offset
    );
    assert!(cache.should_draw_inline_frame(&after, &viewport, 80, 24));
}

const FOCUS_REACQUIRE_HISTORY_MARKER: &str = "FOCUS_REACQUIRE_HISTORY_MARKER";
const FOCUS_REACQUIRE_PROMPT_MARKER: &str = "포커스 복귀 한글";

#[test]
fn focus_reacquire_repaints_visible_frame_without_replaying_host_scrollback() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 100, 30);
    let app = focus_reacquire_test_app();
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();
    let mut frames = tui_testkit::InlineFrameRecorder::default();

    frames.draw_and_record(
        "before-focus-loss",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    assert!(runtime.take_redraw_request());
    let before = frames.frame("before-focus-loss");
    let expected_screen = before.screen_text.clone();
    let expected_host_scrollback = before.host_scrollback_text.clone();
    let expected_cursor = terminal
        .get_cursor_position()
        .expect("baseline cursor should be readable");
    assert!(
        expected_screen.contains(FOCUS_REACQUIRE_PROMPT_MARKER),
        "{expected_screen}"
    );
    assert_eq!(
        expected_host_scrollback
            .matches(FOCUS_REACQUIRE_HISTORY_MARKER)
            .count(),
        1
    );

    runtime.handle_terminal_event(Event::FocusLost);
    let viewport_area = inline_terminal
        .viewport_area()
        .expect("initial draw should record the inline viewport");
    replace_visible_inline_frame(&mut terminal, viewport_area, expected_cursor);
    assert!(!tui_testkit::screen_text(&terminal).contains(FOCUS_REACQUIRE_PROMPT_MARKER));
    runtime.handle_terminal_event(Event::FocusGained);
    assert_eq!(runtime.terminal_focus_reacquire_epoch(), 1);
    assert!(runtime.take_redraw_request());

    frames.draw_and_record(
        "after-focus-reacquire",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    let after = frames.frame("after-focus-reacquire");
    assert_eq!(after.screen_text, expected_screen);
    assert_eq!(after.host_scrollback_text, expected_host_scrollback);
    assert_eq!(
        after
            .host_scrollback_text
            .matches(FOCUS_REACQUIRE_HISTORY_MARKER)
            .count(),
        1
    );
    assert_eq!(
        terminal
            .get_cursor_position()
            .expect("restored cursor should be readable"),
        expected_cursor
    );

    runtime.handle_terminal_event(Event::FocusGained);
    assert_eq!(runtime.terminal_focus_reacquire_epoch(), 1);
    assert!(!runtime.take_redraw_request());
    assert!(!sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
}

#[test]
fn vt100_focus_reacquire_repaints_once_across_cjk_resize() {
    let mut terminal = tui_testkit::inline_history_vt100_terminal(
        InlineHistoryRenderMode::HostScrollback,
        100,
        30,
    );
    let mut app = focus_reacquire_test_app();
    app.shell.history_insert_mode = HistoryInsertionMode::NewlineFallback;
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("initial VT100 frame should draw");
    assert!(runtime.take_redraw_request());
    tui_testkit::resize_inline_history_vt100_terminal(&mut terminal, 48, 18);
    runtime.handle_terminal_event(Event::Resize(48, 18));
    assert!(runtime.take_redraw_request());
    let narrow_projection = frame_projection(runtime.app(), 48);
    assert!(narrow_projection.tail_view.composer_surface.is_some());
    assert!(
        narrow_projection.tail_view.prompt_cursor_offset.is_some(),
        "{}",
        narrow_projection
            .tail_view
            .lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("narrow VT100 frame transaction should succeed")
    );

    let expected_screen = tui_testkit::screen_text(&terminal);
    let expected_host_scrollback = tui_testkit::inline_vt100_host_scrollback_text(&mut terminal);
    let expected_terminal_history = tui_testkit::inline_vt100_scrollback_text(&mut terminal);
    let expected_cursor = terminal
        .get_cursor_position()
        .expect("narrow VT100 cursor should be readable");
    let draw_calls_before_focus = terminal.backend().inner().draw_call_count();
    assert!(
        expected_screen.contains(FOCUS_REACQUIRE_PROMPT_MARKER),
        "viewport={:?} cursor={:?} composer={} prompt_cursor={:?}\nlines:\n{}\nscreen:\n{expected_screen}",
        inline_terminal.viewport_area(),
        expected_cursor,
        narrow_projection.tail_view.composer_surface.is_some(),
        narrow_projection.tail_view.prompt_cursor_offset,
        narrow_projection
            .tail_view
            .lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    );
    assert_eq!(
        expected_terminal_history
            .matches(FOCUS_REACQUIRE_HISTORY_MARKER)
            .count(),
        1
    );

    runtime.handle_terminal_event(Event::FocusLost);
    let viewport_area = inline_terminal
        .viewport_area()
        .expect("narrow draw should record the VT100 viewport");
    replace_visible_inline_frame(&mut terminal, viewport_area, expected_cursor);
    assert!(!tui_testkit::screen_text(&terminal).contains(FOCUS_REACQUIRE_PROMPT_MARKER));
    runtime.handle_terminal_event(Event::FocusGained);
    assert!(runtime.take_redraw_request());
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("focus reacquire should repaint the VT100 frame");

    assert_eq!(
        terminal.backend().inner().draw_call_count(),
        draw_calls_before_focus + 1,
        "one focus transition must produce exactly one terminal draw"
    );
    assert_eq!(tui_testkit::screen_text(&terminal), expected_screen);
    assert_eq!(
        tui_testkit::inline_vt100_host_scrollback_text(&mut terminal),
        expected_host_scrollback
    );
    assert_eq!(
        tui_testkit::inline_vt100_scrollback_text(&mut terminal),
        expected_terminal_history
    );
    assert_eq!(
        terminal.backend().inner().parser_cursor_position(),
        expected_cursor
    );

    runtime.handle_terminal_event(Event::FocusGained);
    assert!(!runtime.take_redraw_request());
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("duplicate focus notification should leave the VT100 frame stable");
    assert_eq!(
        terminal.backend().inner().draw_call_count(),
        draw_calls_before_focus + 1,
        "duplicate focus notification must not draw another frame"
    );

    tui_testkit::resize_inline_history_vt100_terminal(&mut terminal, 100, 30);
    runtime.handle_terminal_event(Event::Resize(100, 30));
    assert!(runtime.take_redraw_request());
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("restored VT100 frame should draw");
    let restored_screen = tui_testkit::screen_text(&terminal);
    let restored_host_scrollback = tui_testkit::inline_vt100_host_scrollback_text(&mut terminal);
    let restored_terminal_history = tui_testkit::inline_vt100_scrollback_text(&mut terminal);
    assert!(restored_screen.contains(FOCUS_REACQUIRE_PROMPT_MARKER));
    assert_eq!(
        restored_terminal_history
            .matches(FOCUS_REACQUIRE_HISTORY_MARKER)
            .count(),
        1
    );
    assert_eq!(
        restored_host_scrollback
            .matches(FOCUS_REACQUIRE_PROMPT_MARKER)
            .count(),
        0
    );
}

fn focus_reacquire_test_app() -> NativeTuiApp {
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    append_history_message(&mut app, FOCUS_REACQUIRE_HISTORY_MARKER);
    for index in 0..40 {
        append_history_message(&mut app, &format!("focus history filler {index:02}"));
    }
    assert!(app.insert_input_text(FOCUS_REACQUIRE_PROMPT_MARKER.to_string()));
    app
}

fn replace_visible_inline_frame<B: Backend>(
    terminal: &mut Terminal<InlineTerminalBackend<B>>,
    viewport_area: ratatui::layout::Rect,
    cursor_position: Position,
) where
    B::Error: std::fmt::Debug,
{
    let backend = terminal.backend_mut().inner_mut();
    backend
        .set_cursor_position(viewport_area.as_position())
        .expect("fixture should position at the live viewport");
    backend
        .clear_region(ClearType::AfterCursor)
        .expect("fixture should simulate a replaced visible frame");
    backend
        .set_cursor_position(cursor_position)
        .expect("fixture should preserve the physical cursor");
}

#[test]
fn late_completion_across_agent_items_flushes_each_final_once_in_order() {
    const FIRST_MARKER: &str = "FIRST_AGENT_HANDOFF_MARKER";
    const SECOND_MARKER: &str = "SECOND_AGENT_HANDOFF_MARKER";
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    append_user_history_message(&mut app, "multi-item handoff prompt");
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should start in a ready conversation state");
    };
    set_running_turn(conversation, "turn-multi-item");
    conversation.push_live_agent_delta(
        "agent-first".to_string(),
        Some("commentary".to_string()),
        format!("{FIRST_MARKER} draft"),
    );
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("first agent draft draw transaction");

    let ConversationState::Ready(conversation) =
        &mut runtime.app_mut().conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready conversation state");
    };
    conversation.complete_live_agent_message(
        "agent-first".to_string(),
        Some("commentary".to_string()),
        format!("{FIRST_MARKER} initial"),
    );
    conversation.push_live_agent_delta(
        "agent-second".to_string(),
        Some("final_answer".to_string()),
        format!("{SECOND_MARKER} draft"),
    );
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("second agent draft draw transaction");
    let host_during_second = tui_testkit::inline_vt100_host_scrollback_text(&mut terminal);
    assert!(!host_during_second.contains(FIRST_MARKER));
    let screen_during_second = tui_testkit::screen_text(&terminal);
    assert_eq!(screen_during_second.matches(FIRST_MARKER).count(), 1);
    assert_eq!(screen_during_second.matches(SECOND_MARKER).count(), 1);

    let ConversationState::Ready(conversation) =
        &mut runtime.app_mut().conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready conversation state");
    };
    conversation.complete_live_agent_message(
        "agent-first".to_string(),
        Some("commentary".to_string()),
        format!("{FIRST_MARKER} final"),
    );
    conversation.sync_live_agent_draft(
        "agent-second".to_string(),
        Some("final_answer".to_string()),
        format!("{SECOND_MARKER} progressive"),
    );
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("interleaved late completion draw transaction");
    let interleaved_screen = tui_testkit::screen_text(&terminal);
    assert_eq!(interleaved_screen.matches(FIRST_MARKER).count(), 1);
    assert_eq!(interleaved_screen.matches(SECOND_MARKER).count(), 1);

    let ConversationState::Ready(conversation) =
        &mut runtime.app_mut().conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready conversation state");
    };
    conversation.complete_live_agent_message(
        "agent-second".to_string(),
        Some("final_answer".to_string()),
        format!("{SECOND_MARKER} final"),
    );
    conversation.finish_turn("turn-multi-item", &[]);
    begin_post_turn_evaluation(conversation, "turn-multi-item");
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("multi-item settlement draw transaction");
    let settlement_screen = tui_testkit::screen_text(&terminal);
    let settlement_host = tui_testkit::inline_vt100_host_scrollback_text(&mut terminal);
    assert_eq!(settlement_screen.matches(FIRST_MARKER).count(), 0);
    assert_eq!(settlement_screen.matches(SECOND_MARKER).count(), 0);
    assert_eq!(settlement_host.matches(FIRST_MARKER).count(), 1);
    assert_eq!(settlement_host.matches(SECOND_MARKER).count(), 1);
    assert!(settlement_host.find(FIRST_MARKER) < settlement_host.find(SECOND_MARKER));

    let ConversationState::Ready(conversation) =
        &mut runtime.app_mut().conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready conversation state");
    };
    settle_post_turn(conversation, "turn-multi-item");
    assert!(conversation.complete_post_turn_settlement("turn-multi-item"));
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("multi-item release draw transaction");
    let released_history = inline_terminal
        .history_flush
        .rendered_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(released_history.matches(FIRST_MARKER).count(), 1);
    assert_eq!(released_history.matches(SECOND_MARKER).count(), 1);
    assert!(released_history.find(FIRST_MARKER) < released_history.find(SECOND_MARKER));

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("stable multi-item release draw transaction");
    let stable_history = tui_testkit::inline_vt100_scrollback_text(&mut terminal);
    assert_eq!(stable_history.matches(FIRST_MARKER).count(), 1);
    assert_eq!(stable_history.matches(SECOND_MARKER).count(), 1);
}

#[test]
fn long_committed_commentary_does_not_push_current_live_item_below_viewport() {
    const LATEST_MARKER: &str = "LATEST_LIVE_ITEM_REMAINS_VISIBLE";
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::HostScrollback, 48, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    append_user_history_message(&mut app, "long commentary prompt");
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should start in a ready conversation state");
    };
    set_running_turn(conversation, "turn-long-commentary");
    conversation.push_live_agent_delta(
        "agent-long-commentary".to_string(),
        Some("commentary".to_string()),
        (0..40)
            .map(|index| format!("earlier commentary row {index:02}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    conversation.complete_live_agent_message(
        "agent-long-commentary".to_string(),
        Some("commentary".to_string()),
        (0..40)
            .map(|index| format!("completed commentary row {index:02}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    conversation.push_live_agent_delta(
        "agent-latest-answer".to_string(),
        Some("final_answer".to_string()),
        LATEST_MARKER.to_string(),
    );
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("long commentary draw transaction");

    let screen = tui_testkit::screen_text(&terminal);
    assert_eq!(screen.matches(LATEST_MARKER).count(), 1, "{screen}");
    let host = tui_testkit::inline_vt100_host_scrollback_text(&mut terminal);
    assert!(!host.contains(LATEST_MARKER));
}

#[test]
fn parallel_projection_delivers_conversation_handoff_before_unlocking_prompt() {
    for (render_mode, insert_mode) in [
        (
            InlineHistoryRenderMode::HostScrollback,
            HistoryInsertionMode::StandardScrollRegion,
        ),
        (
            InlineHistoryRenderMode::HostScrollback,
            HistoryInsertionMode::NewlineFallback,
        ),
        (
            InlineHistoryRenderMode::ViewportReplay,
            HistoryInsertionMode::StandardScrollRegion,
        ),
    ] {
        assert_parallel_projection_delivers_conversation_handoff(render_mode, insert_mode);
    }
}

fn assert_parallel_projection_delivers_conversation_handoff(
    render_mode: InlineHistoryRenderMode,
    insert_mode: HistoryInsertionMode,
) {
    const PROMPT_MARKER: &str = "PARALLEL_PROMPT_HANDOFF_MARKER";
    const COMMENTARY_MARKER: &str = "PARALLEL_COMMENTARY_HANDOFF_MARKER";
    const FINAL_MARKER: &str = "PARALLEL_PENDING_HANDOFF_MARKER";
    let mut terminal = tui_testkit::inline_history_vt100_terminal(render_mode, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = render_mode;
    app.shell.history_insert_mode = insert_mode;
    append_user_history_message(&mut app, PROMPT_MARKER);
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should start in a ready conversation state");
    };
    set_running_turn(conversation, "turn-parallel-handoff");
    conversation.push_live_agent_delta(
        "agent-parallel-commentary".to_string(),
        Some("commentary".to_string()),
        COMMENTARY_MARKER.to_string(),
    );
    conversation.complete_live_agent_message(
        "agent-parallel-commentary".to_string(),
        Some("commentary".to_string()),
        COMMENTARY_MARKER.to_string(),
    );
    conversation.push_live_agent_delta(
        "agent-parallel-handoff".to_string(),
        Some("final_answer".to_string()),
        format!("{FINAL_MARKER} streaming"),
    );
    conversation.complete_live_agent_message(
        "agent-parallel-handoff".to_string(),
        Some("final_answer".to_string()),
        format!("{FINAL_MARKER} final"),
    );
    conversation.finish_turn("turn-parallel-handoff", &[]);
    begin_post_turn_evaluation(conversation, "turn-parallel-handoff");
    settle_post_turn(conversation, "turn-parallel-handoff");
    assert!(conversation.complete_post_turn_settlement("turn-parallel-handoff"));
    app.set_parallel_mode_enabled_for_test(true);
    for index in 0..40 {
        app.push_parallel_supervisor_event_for_test(
            "11:45:02",
            "Task Intake",
            format!("PARALLEL_BASELINE_EVENT_{index:02}"),
        );
    }
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("parallel handoff draw transaction");
    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready conversation state");
    };
    assert!(!conversation.has_pending_viewport_transcript_handoff());
    assert!(conversation.can_accept_manual_prompt());
    let parallel_history = tui_testkit::inline_vt100_scrollback_text(&mut terminal);
    let durable_host = tui_testkit::inline_vt100_host_scrollback_text(&mut terminal);
    let parallel_screen = tui_testkit::screen_text(&terminal);
    let visible_event_marker = (0..40)
        .map(|index| format!("PARALLEL_BASELINE_EVENT_{index:02}"))
        .find(|marker| parallel_history.contains(marker));
    match render_mode {
        InlineHistoryRenderMode::HostScrollback => {
            let diagnostic = format!(
                "{render_mode:?}/{insert_mode:?}\nterminal history:\n{parallel_history}\nhost scrollback:\n{durable_host}"
            );
            assert_eq!(
                parallel_history.matches(PROMPT_MARKER).count(),
                1,
                "{diagnostic}"
            );
            assert_eq!(
                parallel_history.matches(COMMENTARY_MARKER).count(),
                1,
                "{diagnostic}"
            );
            assert_eq!(
                parallel_history.matches(FINAL_MARKER).count(),
                1,
                "{diagnostic}"
            );
            assert!(
                parallel_history.find(PROMPT_MARKER) < parallel_history.find(COMMENTARY_MARKER)
            );
            assert!(parallel_history.find(COMMENTARY_MARKER) < parallel_history.find(FINAL_MARKER));
            let event_marker = visible_event_marker
                .as_deref()
                .expect("parallel event history should remain visible");
            assert_eq!(parallel_history.matches(event_marker).count(), 1);
            assert!(!durable_host.contains("Command Hints"));
            assert!(!durable_host.contains("parallel: preparing"));
            let conversation_baseline = inline_terminal
                .history_flush
                .rendered_lines
                .iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(conversation_baseline.contains(PROMPT_MARKER));
            assert!(conversation_baseline.contains(COMMENTARY_MARKER));
            assert!(conversation_baseline.contains(FINAL_MARKER));
        }
        InlineHistoryRenderMode::ViewportReplay => {
            assert_eq!(parallel_screen.matches(COMMENTARY_MARKER).count(), 1);
            assert_eq!(parallel_screen.matches(FINAL_MARKER).count(), 1);
            assert!(parallel_screen.find(COMMENTARY_MARKER) < parallel_screen.find(FINAL_MARKER));
        }
    }

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("stable parallel handoff draw transaction");
    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready conversation state");
    };
    assert!(!conversation.has_pending_viewport_transcript_handoff());
    match render_mode {
        InlineHistoryRenderMode::HostScrollback => {
            let stable_history = tui_testkit::inline_vt100_scrollback_text(&mut terminal);
            assert_eq!(stable_history.matches(PROMPT_MARKER).count(), 1);
            assert_eq!(stable_history.matches(COMMENTARY_MARKER).count(), 1);
            assert_eq!(stable_history.matches(FINAL_MARKER).count(), 1);
            let event_marker = visible_event_marker
                .as_deref()
                .expect("parallel event history should remain visible");
            assert_eq!(stable_history.matches(event_marker).count(), 1);

            runtime.app_mut().set_parallel_mode_enabled_for_test(false);
            draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
                .expect("parallel-off conversation draw transaction");
            let conversation_history = tui_testkit::inline_vt100_scrollback_text(&mut terminal);
            assert_eq!(conversation_history.matches(PROMPT_MARKER).count(), 1);
            assert_eq!(conversation_history.matches(COMMENTARY_MARKER).count(), 1);
            assert_eq!(conversation_history.matches(FINAL_MARKER).count(), 1);
        }
        InlineHistoryRenderMode::ViewportReplay => {
            let stable_screen = tui_testkit::screen_text(&terminal);
            assert_eq!(stable_screen.matches(COMMENTARY_MARKER).count(), 1);
            assert_eq!(stable_screen.matches(FINAL_MARKER).count(), 1);
        }
    }
}

#[test]
fn viewport_handoff_waits_for_a_successful_draw_before_ack() {
    assert_viewport_handoff_waits_for_a_successful_draw(false);
    assert_viewport_handoff_waits_for_a_successful_draw(true);
}

#[test]
fn draw_time_resize_aba_keeps_handoff_pending_until_stable_retry() {
    const FINAL_MARKER: &str = "released handoff answer";
    let mut inner = CursorQueryCountingBackend::new(TestBackend::new(80, 40));
    inner
        .set_cursor_position(Position::new(0, 39))
        .expect("fixture cursor should start at the physical bottom");
    let backend = InlineTerminalBackend::new(inner);
    let mut terminal = Terminal::with_options(
        backend,
        terminal_options_for_render_mode(InlineHistoryRenderMode::ViewportReplay),
    )
    .expect("viewport replay terminal should initialize");
    let app = released_handoff_app(
        InlineHistoryRenderMode::ViewportReplay,
        false,
        ShellOverlay::Hidden,
    );
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
    let width = terminal.size().expect("terminal size").width;
    let projection = frame_projection(runtime.app(), width);
    let resize_snapshot = terminal
        .backend()
        .resize_snapshot()
        .expect("stable pre-draw resize snapshot");
    let observation_epoch = terminal.backend().resize_observation_epoch();
    // The expected snapshot is already frozen. The next two draw-time size
    // observations emulate a physical shrink and same-size restore.
    terminal
        .backend_mut()
        .inner_mut()
        .report_resize_then_restore(2, Size::new(48, 10), 3, Size::new(80, 40));
    runtime
        .app_mut()
        .planning
        .queue_overlay_ui_state
        .bind_receipt_undo_hit_area(Some(Rect::new(4, 7, 14, 1)));

    assert!(
        !draw_inline_frame(
            &mut terminal,
            &mut runtime,
            &mut inline_terminal,
            projection,
            false,
            resize_snapshot,
        )
        .expect("ABA-raced draw should return cleanly"),
        "same-size ABA geometry must not commit the drawn frame"
    );
    assert_eq!(
        terminal.backend().resize_observation_epoch(),
        observation_epoch + 2,
        "the draw must observe both sides of the resize ABA"
    );
    assert_eq!(terminal.size().unwrap(), Size::new(80, 40));
    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("deferred handoff should keep a ready conversation");
    };
    assert!(conversation.has_pending_viewport_transcript_handoff());
    assert!(!inline_terminal.back_buffer_trustworthy());
    assert_eq!(inline_terminal.latest_frame_render_attempt(), 1);
    assert_eq!(inline_terminal.last_committed_frame_render_attempt(), None);
    assert_eq!(
        runtime
            .app()
            .planning
            .queue_overlay_ui_state
            .receipt_undo_hit_area(),
        None,
        "a same-size resize ABA must discard stale interaction geometry"
    );
    assert_resize_retry_scheduled(
        &mut runtime,
        "same-size ABA geometry must schedule a stable rebuild",
    );

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
    assert!(draw_test_frame(
        &mut terminal,
        &mut runtime,
        &mut inline_terminal
    ));
    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("stable retry should keep a ready conversation");
    };
    assert!(!conversation.has_pending_viewport_transcript_handoff());
    assert_eq!(inline_terminal.latest_frame_render_attempt(), 2);
    assert_eq!(
        inline_terminal.last_committed_frame_render_attempt(),
        Some(2),
        "only the stable retry may commit its render receipt"
    );
    let screen = tui_testkit::buffer_text(terminal.backend().inner().inner.buffer());
    assert_eq!(screen.matches(FINAL_MARKER).count(), 1, "{screen}");
}

fn assert_viewport_handoff_waits_for_a_successful_draw(parallel_mode_enabled: bool) {
    const FINAL_MARKER: &str = "RESIZE_GUARDED_VIEWPORT_HANDOFF";
    let mut inner = CursorQueryCountingBackend::new(TestBackend::new(80, 40));
    inner
        .set_cursor_position(Position::new(0, 39))
        .expect("fixture cursor should start at the physical bottom");
    let backend = InlineTerminalBackend::new(inner);
    let mut terminal = Terminal::with_options(
        backend,
        terminal_options_for_render_mode(InlineHistoryRenderMode::ViewportReplay),
    )
    .expect("viewport replay terminal should initialize");
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::ViewportReplay;
    append_user_history_message(&mut app, "resize guarded handoff prompt");
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should start in a ready conversation state");
    };
    set_running_turn(conversation, "turn-resize-handoff");
    conversation.push_live_agent_delta(
        "agent-resize-handoff".to_string(),
        Some("final_answer".to_string()),
        FINAL_MARKER.to_string(),
    );
    conversation.complete_live_agent_message(
        "agent-resize-handoff".to_string(),
        Some("final_answer".to_string()),
        FINAL_MARKER.to_string(),
    );
    conversation.finish_turn("turn-resize-handoff", &[]);
    begin_post_turn_evaluation(conversation, "turn-resize-handoff");
    settle_post_turn(conversation, "turn-resize-handoff");
    assert!(conversation.complete_post_turn_settlement("turn-resize-handoff"));
    app.set_parallel_mode_enabled_for_test(parallel_mode_enabled);
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    runtime.app_mut().shell.chrome.shell_overlay = ShellOverlay::Help;
    runtime
        .app_mut()
        .dispatch_conversation_intent(ConversationIntentEvent::NewDraftRequested);
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("overlay-obscured viewport handoff draw transaction");
    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready conversation state");
    };
    assert!(conversation.has_pending_viewport_transcript_handoff());
    assert!(!conversation.can_accept_manual_prompt());
    let overlay_screen = tui_testkit::buffer_text(terminal.backend().inner().inner.buffer());
    assert!(!overlay_screen.contains(FINAL_MARKER));
    assert!(!overlay_screen.contains("Enter send"));
    assert!(overlay_screen.contains("Response held while the dialog is open"));
    runtime.app_mut().shell.chrome.shell_overlay = ShellOverlay::Hidden;

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
    terminal
        .backend_mut()
        .inner_mut()
        .resize_on_next_flush(48, 10);
    assert!(!draw_test_frame(
        &mut terminal,
        &mut runtime,
        &mut inline_terminal
    ));
    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready conversation state");
    };
    assert!(conversation.has_pending_viewport_transcript_handoff());
    assert!(!conversation.can_accept_manual_prompt());

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
    assert!(draw_test_frame(
        &mut terminal,
        &mut runtime,
        &mut inline_terminal
    ));
    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("test app should keep a ready conversation state");
    };
    assert!(!conversation.has_pending_viewport_transcript_handoff());
    assert!(conversation.can_accept_manual_prompt());
    assert!(
        !conversation
            .status_text
            .starts_with("conversation is busy;")
    );
    let screen = tui_testkit::buffer_text(terminal.backend().inner().inner.buffer());
    assert_eq!(screen.matches(FINAL_MARKER).count(), 1, "{screen}");
    assert!(!screen.contains("status: conversation is busy"), "{screen}");
    if parallel_mode_enabled {
        assert!(screen.contains("Parallel board loading"), "{screen}");
    } else {
        assert!(screen.contains("submission waits for startup"), "{screen}");
    }

    assert!(
        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("stable viewport handoff draw transaction")
    );
    let stable_screen = tui_testkit::buffer_text(terminal.backend().inner().inner.buffer());
    assert_eq!(stable_screen.matches(FINAL_MARKER).count(), 1);
    assert!(!stable_screen.contains("conversation is busy"));
}

#[test]
fn parallel_event_stream_flushes_rows_without_live_panel_chrome() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.set_parallel_mode_enabled_for_test(true);
    append_history_message(
        &mut app,
        "single mode history must not own parallel scrollback",
    );
    for index in 0..40 {
        app.push_parallel_supervisor_event_for_test(
            "11:45:02",
            "Task Intake",
            format!("parallel-event-{index:02}"),
        );
    }
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("parallel event draw transaction");

    let terminal_scrollback = tui_testkit::inline_scrollback_text(&terminal);
    assert!(
        !terminal_scrollback.contains("Parallel Event Stream"),
        "the stream title belongs to the live inline section, not durable host scrollback:\n{terminal_scrollback}"
    );
    assert!(
        !terminal_scrollback.contains("Recent Parallel Events"),
        "the live-tail title must never be persisted into durable host scrollback:\n{terminal_scrollback}"
    );
    assert!(
        terminal_scrollback.contains("parallel-event-00"),
        "parallel board events should be written into host scrollback for operator scrollback:\n{terminal_scrollback}"
    );
    assert!(
        !terminal_scrollback.contains("single mode history must not own parallel scrollback"),
        "parallel mode should suppress the hidden single-mode transcript:\n{terminal_scrollback}"
    );
    let screen_text = tui_testkit::screen_text(&terminal);
    assert!(
        !screen_text.contains("Parallel Event Stream")
            && !screen_text.contains("Recent Parallel Events"),
        "overflowed stream rows should continue without inline panel titles between scrollback and live tail:\n{screen_text}"
    );
    assert!(
        [
            "parallel-event-37",
            "parallel-event-38",
            "parallel-event-39"
        ]
        .iter()
        .any(|event| screen_text.contains(event)),
        "live stream should show the recent event tail:\n{screen_text}"
    );
    assert!(
        !screen_text.contains("parallel-event-00"),
        "old parallel events should not force the live viewport to show every row:\n{screen_text}"
    );

    for index in 40..60 {
        runtime.app_mut().push_parallel_supervisor_event_for_test(
            "11:45:03",
            "Task Intake",
            format!("parallel-event-{index:02}"),
        );
    }
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("parallel event delta draw transaction");

    let terminal_scrollback = tui_testkit::inline_scrollback_text(&terminal);
    assert!(
        !terminal_scrollback.contains("Parallel Event Stream")
            && !terminal_scrollback.contains("Recent Parallel Events"),
        "redraws must not replay the live section title into host scrollback:\n{terminal_scrollback}"
    );
    assert_eq!(
        terminal_scrollback.matches("parallel-event-00").count(),
        1,
        "existing parallel events should not duplicate on later redraws:\n{terminal_scrollback}"
    );
    assert_eq!(
        terminal_scrollback.matches("parallel-event-40").count(),
        1,
        "new parallel events should append into host scrollback exactly once:\n{terminal_scrollback}"
    );
}

#[test]
fn parallel_live_tail_continues_scrollback_without_inline_title() {
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.set_parallel_mode_enabled_for_test(true);
    for index in 0..48 {
        app.push_parallel_supervisor_event_for_test(
            "11:45:02",
            "Task Intake",
            format!("tail-title-event-{index:02}"),
        );
    }
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();
    let mut frame_recorder = tui_testkit::InlineFrameRecorder::default();

    frame_recorder.draw_and_record_vt100(
        "overflowed-tail",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );

    let frame = frame_recorder.frame("overflowed-tail");
    assert!(
        !frame.screen_text.contains("Recent Parallel Events"),
        "overflowed stream must not insert a live-tail title between durable history and current rows:\n{}",
        frame.screen_text
    );
    assert!(
        !frame.screen_text.contains("Parallel Event Stream"),
        "overflowed stream must not draw the full-stream title at the scrollback/live boundary:\n{}",
        frame.screen_text
    );
    assert!(
        !frame
            .host_scrollback_text
            .contains("Recent Parallel Events")
            && !frame.host_scrollback_text.contains("Parallel Event Stream"),
        "durable stream rows should stay data-only without live panel titles:\n{}",
        frame.host_scrollback_text
    );
    let old_event_index = frame
        .terminal_history_text
        .find("tail-title-event-00")
        .expect("old stream event should be retained in terminal history");
    let recent_event_index = frame
        .terminal_history_text
        .find("tail-title-event-47")
        .expect("recent stream event should be present in terminal history view");
    assert!(
        old_event_index < recent_event_index,
        "the terminal history should read as one continuous event stream without an inserted title:\n{}",
        frame.terminal_history_text
    );
    for index in 0..48 {
        let marker = format!("tail-title-event-{index:02}");
        assert_eq!(
            frame.terminal_history_text.matches(&marker).count(),
            1,
            "the sampled scrollback/live split must retain each event exactly once: {marker}\n{}",
            frame.terminal_history_text
        );
    }
}

#[test]
fn focused_parallel_operations_survives_resize_and_close_without_chrome_in_scrollback() {
    let mut terminal = tui_testkit::inline_history_vt100_terminal(
        InlineHistoryRenderMode::HostScrollback,
        120,
        30,
    );
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.set_parallel_mode_enabled_for_test(true);
    app.shell.chrome.shell_overlay = ShellOverlay::Supersession;
    for index in 0..36 {
        app.push_parallel_supervisor_event_for_test(
            "12:10:00",
            "Supervisor",
            format!("operations-resize-event-{index:02}"),
        );
    }
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();
    let mut recorder = tui_testkit::InlineFrameRecorder::default();

    recorder.draw_and_record_vt100(
        "operations-wide",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    let wide = recorder.frame("operations-wide");
    assert!(
        wide.screen_text
            .contains("Parallel Operations / inline inspection")
    );
    assert_eq!(
        terminal
            .get_cursor_position()
            .expect("focused operations cursor should be readable")
            .y,
        29,
        "the hidden operations cursor must park at the physical bottom before resize"
    );
    for index in 0..36 {
        let marker = format!("operations-resize-event-{index:02}");
        assert_eq!(
            wide.terminal_history_text.matches(&marker).count(),
            1,
            "the first focused frame must retain each event exactly once: {marker}\n{}",
            wide.terminal_history_text
        );
    }

    let expected_host_scrollback = wide.host_scrollback_text.clone();
    let expected_cursor = terminal
        .get_cursor_position()
        .expect("focused operations cursor should be readable");
    let _ = runtime.take_redraw_request();
    runtime.handle_terminal_event(Event::FocusLost);
    let viewport_area = inline_terminal
        .viewport_area()
        .expect("focused operations draw should record its viewport");
    replace_visible_inline_frame(&mut terminal, viewport_area, expected_cursor);
    runtime.handle_terminal_event(Event::FocusGained);
    assert_eq!(runtime.terminal_focus_reacquire_epoch(), 1);
    assert!(runtime.take_redraw_request());
    recorder.draw_and_record_vt100(
        "operations-refocused",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    let refocused = recorder.frame("operations-refocused");
    assert!(
        refocused
            .screen_text
            .contains("Parallel Operations / inline inspection")
    );
    assert_eq!(refocused.host_scrollback_text, expected_host_scrollback);
    assert_eq!(
        refocused
            .terminal_history_text
            .matches("operations-resize-event-35")
            .count(),
        1,
        "focus reacquire must repaint the live event without replaying it:\n{}",
        refocused.terminal_history_text
    );

    tui_testkit::resize_inline_history_vt100_terminal(&mut terminal, 72, 24);
    recorder.draw_and_record_vt100(
        "operations-narrow",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    let narrow = recorder.frame("operations-narrow");
    assert!(narrow.screen_text.contains("Agent Lanes"));
    assert!(narrow.screen_text.contains("Selected Lane"));
    assert!(
        !narrow.screen_text.contains("Enter send"),
        "focused board must not expose a hidden composer after resize:\n{}",
        narrow.screen_text
    );
    for index in 0..36 {
        let marker = format!("operations-resize-event-{index:02}");
        assert_eq!(
            narrow.app_event_stream_text.matches(&marker).count(),
            1,
            "the narrow focused frame must retain each canonical event exactly once: {marker}\n{}",
            narrow.app_event_stream_text
        );
    }
    assert_eq!(
        narrow
            .terminal_history_text
            .matches("operations-resize-event-35")
            .count(),
        1,
        "the narrow focused frame must render the latest event once:\n{}",
        narrow.terminal_history_text
    );

    runtime.app_mut().shell.chrome.shell_overlay = ShellOverlay::Hidden;
    tui_testkit::resize_inline_history_vt100_terminal(&mut terminal, 120, 30);
    recorder.draw_and_record_vt100(
        "operations-closed",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    let closed = recorder.frame("operations-closed");
    assert!(closed.screen_text.contains("Ctrl+O board"));
    assert!(
        closed
            .screen_text
            .contains("Describe a task or type : for commands")
            && closed.screen_text.contains("input paused"),
        "closing focused operations must restore the composer:\n{}",
        closed.screen_text
    );
    for chrome in [
        "Parallel Operations",
        "Agent Lanes",
        "Selected Lane",
        "Accepted Queue",
        "Command Hints",
        "Parallel Event Stream",
    ] {
        assert!(
            !closed.host_scrollback_text.contains(chrome),
            "live board chrome must stay out of host scrollback after resize/close: {chrome}\n{}",
            closed.host_scrollback_text
        );
    }
    for index in 0..36 {
        let marker = format!("operations-resize-event-{index:02}");
        assert_eq!(
            closed.app_event_stream_text.matches(&marker).count(),
            1,
            "the canonical stream must remain continuous across focused/narrow/passive frames: {marker}\n{}",
            closed.app_event_stream_text
        );
    }
    assert_eq!(
        closed
            .terminal_history_text
            .matches("operations-resize-event-35")
            .count(),
        1,
        "restoring geometry and closing the board must retain the latest event exactly once:\n{}",
        closed.terminal_history_text
    );

    runtime.app_mut().push_parallel_supervisor_event_for_test(
        "12:10:01",
        "Supervisor",
        "operations-after-reopen",
    );
    runtime.app_mut().shell.chrome.shell_overlay = ShellOverlay::Supersession;
    recorder.draw_and_record_vt100(
        "operations-reopened",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    assert!(
        inline_terminal
            .history_flush
            .has_trailing_reflow_guard_rows(),
        "the newly durable boundary event needs physical guard rows before another resize"
    );
    tui_testkit::resize_inline_history_vt100_terminal(&mut terminal, 48, 18);
    recorder.draw_and_record_vt100(
        "operations-reopened-narrow",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    tui_testkit::resize_inline_history_vt100_terminal(&mut terminal, 120, 30);
    recorder.draw_and_record_vt100(
        "operations-reopened-restored",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    let reopened_restored = recorder.frame("operations-reopened-restored");
    assert_eq!(
        reopened_restored
            .terminal_history_text
            .matches("operations-resize-event-35")
            .count(),
        1,
        "a newly durable boundary event must survive reopen plus a second resize cycle:\n{}",
        reopened_restored.terminal_history_text
    );
    assert_eq!(
        reopened_restored
            .terminal_history_text
            .matches("operations-after-reopen")
            .count(),
        1,
        "the new live event must survive reopen plus a second resize cycle:\n{}",
        reopened_restored.terminal_history_text
    );
    assert_eq!(
        terminal
            .get_cursor_position()
            .expect("reopened operations cursor should be readable")
            .y,
        29,
        "the reopened focused board must keep the hidden cursor at the physical bottom"
    );
}

#[test]
fn vt100_focused_parallel_operations_keeps_chrome_out_of_scrollback_after_width_resize() {
    let mut terminal = tui_testkit::inline_history_vt100_terminal(
        InlineHistoryRenderMode::HostScrollback,
        120,
        30,
    );
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.set_parallel_mode_enabled_for_test(true);
    app.shell.chrome.shell_overlay = ShellOverlay::Supersession;
    for index in 0..36 {
        app.push_parallel_supervisor_event_for_test(
            "12:10:00",
            "Supervisor",
            format!("vt100-operations-resize-event-{index:02}"),
        );
    }
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("wide vt100 operations frame");
    tui_testkit::resize_inline_history_vt100_terminal(&mut terminal, 72, 24);
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("narrow vt100 operations frame");

    let host_scrollback = tui_testkit::inline_vt100_host_scrollback_text(&mut terminal);
    for chrome in [
        "Parallel Operations",
        "Agent Lanes",
        "Selected Lane",
        "Accepted Queue",
        "Command Hints",
    ] {
        assert!(
            !host_scrollback.contains(chrome),
            "focused board chrome must remain transient after vt100 width resize: {chrome}\n{host_scrollback}"
        );
    }
    let screen = tui_testkit::screen_text(&terminal);
    assert!(
        screen.contains("vt100-operations-resize-event-35"),
        "the resized focused board must retain the latest live event:\n{screen}"
    );
    let event_stream = runtime
        .app()
        .parallel_supervisor_event_lines()
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    for index in 0..36 {
        let marker = format!("vt100-operations-resize-event-{index:02}");
        assert_eq!(
            event_stream.matches(&marker).count(),
            1,
            "vt100 width resize must not mutate the canonical event projection: {marker}\n{event_stream}"
        );
    }
}

#[test]
fn parallel_bootstrap_and_task_intake_stream_does_not_insert_tail_title() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = true;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(
        status_runtime_feed_supervisor_snapshot(Vec::new()),
    ));
    app.push_parallel_supervisor_event_for_test("05:58:04", "You", "안녕하세요");
    app.push_parallel_supervisor_event_for_test(
        "05:58:04",
        "Task Intake",
        "task generation started from the operator prompt.",
    );
    app.push_parallel_supervisor_event_for_test(
        "05:58:04",
        "Task Intake",
        "committed task task-user-20260519T055804Z-398bb0bebece / rev 63 / 안녕하세요",
    );
    app.push_parallel_supervisor_event_for_test(
        "05:58:04",
        "Orchestrator",
        "task intake dispatch requested for the parallel pool.",
    );
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();
    let mut frame_recorder = tui_testkit::InlineFrameRecorder::default();

    frame_recorder.draw_and_record(
        "bootstrap-task-intake",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );

    let frame = frame_recorder.frame("bootstrap-task-intake");
    assert!(
        frame
            .terminal_history_text
            .contains("control tower is live")
            && frame.terminal_history_text.contains("You: 안녕하세요")
            && frame
                .terminal_history_text
                .contains("task generation started from the operator prompt"),
        "the regression fixture should match the mixed bootstrap and task-intake stream:\n{}",
        frame.terminal_history_text
    );
    assert!(
        !frame.screen_text.contains("Parallel Event Stream")
            && !frame.screen_text.contains("Recent Parallel Events"),
        "no stream title should be inserted between bootstrap rows and task-intake rows:\n{}",
        frame.screen_text
    );
}

#[test]
fn parallel_runtime_feed_primes_baseline_without_scrollback_duplication() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(runtime_feed_supervisor_snapshot(
        vec![
            inline_runtime_feed_entry(2, "seed runtime event two"),
            inline_runtime_feed_entry(1, "seed runtime event one"),
        ],
    )));
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("initial runtime feed draw transaction");

    let terminal_scrollback = tui_testkit::inline_scrollback_text(&terminal);
    assert!(
        !terminal_scrollback.contains("seed runtime event one"),
        "old runtime feed rows should not be replayed above the live parallel board:\n{terminal_scrollback}"
    );
    let screen_text = tui_testkit::screen_text(&terminal);
    assert!(
        !screen_text.contains("seed runtime event one"),
        "old runtime feed rows should not be replayed into the live event stream:\n{screen_text}"
    );
    assert_eq!(
        screen_text.matches("seed runtime event one").count(),
        0,
        "initial runtime feed should stay hidden after the baseline is primed:\n{screen_text}"
    );

    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(runtime_feed_supervisor_snapshot(
            vec![
                inline_runtime_feed_entry(3, "new runtime event three"),
                inline_runtime_feed_entry(2, "seed runtime event two"),
                inline_runtime_feed_entry(1, "seed runtime event one"),
            ],
        )));
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("runtime feed delta draw transaction");

    let app_scrollback = runtime
        .app_mut()
        .parallel_supervisor_event_scrollback_lines()
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        app_scrollback.contains("new runtime event three"),
        "new runtime events should be retained in the durable stream history:\n{app_scrollback}"
    );
    assert!(
        !app_scrollback.contains("seed runtime event one"),
        "primed runtime events must not be backfilled on later redraws:\n{app_scrollback}"
    );
    let terminal_scrollback = tui_testkit::inline_scrollback_text(&terminal);
    assert!(
        !terminal_scrollback.contains("new runtime event three"),
        "new runtime events should stay out of durable host scrollback:\n{terminal_scrollback}"
    );
    let screen_text = tui_testkit::screen_text(&terminal);
    assert!(
        screen_text.contains("new runtime event three"),
        "new runtime events should append to the live event stream after the primed baseline:\n{screen_text}"
    );
    assert!(
        !screen_text.contains("seed runtime event one"),
        "primed runtime events must not be backfilled into the live event stream:\n{screen_text}"
    );
}

#[test]
fn parallel_stream_preserves_initial_status_rows_as_runtime_events_advance() {
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(
        status_runtime_feed_supervisor_snapshot(vec![inline_runtime_feed_entry(
            1,
            "seed runtime event one",
        )]),
    ));
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("initial status stream draw transaction");

    let initial_stream = runtime
        .app_mut()
        .parallel_supervisor_event_lines()
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        initial_stream.contains("parallel board refreshed"),
        "initial board status should be recorded as stream history:\n{initial_stream}"
    );
    assert!(
        initial_stream.contains("reported stage record: no agent results reported yet"),
        "initial ledger status should be recorded as stream history:\n{initial_stream}"
    );

    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(
            status_runtime_feed_supervisor_snapshot(
                (1..=40)
                    .map(|sequence| {
                        inline_runtime_feed_entry(
                            sequence,
                            format!("runtime stream marker {sequence:02}"),
                        )
                    })
                    .collect(),
            ),
        ));
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("runtime stream tail draw transaction");

    let screen_text = tui_testkit::screen_text(&terminal);
    assert!(
        screen_text.contains("runtime stream marker 40"),
        "live stream should keep following new runtime events:\n{screen_text}"
    );
    let terminal_scrollback = tui_testkit::inline_vt100_host_scrollback_text(&mut terminal);
    assert!(
        terminal_scrollback.contains("control tower is live"),
        "initial board status should move into durable terminal history instead of disappearing:\n{terminal_scrollback}"
    );
    assert!(
        terminal_scrollback.contains("no agent results reported yet"),
        "initial ledger status should move into durable terminal history instead of being replaced:\n{terminal_scrollback}"
    );
    assert_eq!(
        terminal_scrollback
            .matches("no agent results reported yet")
            .count(),
        1,
        "snapshot status rows should not be replayed on every redraw:\n{terminal_scrollback}"
    );
    assert!(
        !terminal_scrollback.contains("Parallel Event Stream")
            && !terminal_scrollback.contains("Recent Parallel Events"),
        "durable stream history must not include live panel chrome:\n{terminal_scrollback}"
    );
}

#[test]
fn direct_frame_recorder_keeps_parallel_status_rows_across_runtime_redraw() {
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(
        status_runtime_feed_supervisor_snapshot(vec![inline_runtime_feed_entry(
            1,
            "seed runtime event one",
        )]),
    ));
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();
    let mut frame_recorder = tui_testkit::InlineFrameRecorder::default();

    frame_recorder.draw_and_record_vt100(
        "initial-status",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    let initial_frame = frame_recorder.frame("initial-status");
    assert!(
        !initial_frame.screen_text.contains("Parallel Event Stream")
            && !initial_frame.screen_text.contains("Recent Parallel Events"),
        "overflowed initial status stream should render without panel title chrome:\n{}",
        initial_frame.screen_text
    );
    assert!(
        initial_frame
            .app_event_stream_text
            .contains("control tower is live"),
        "initial status rows should be recorded in the app stream immediately:\n{}",
        initial_frame.app_event_stream_text
    );
    assert!(
        initial_frame
            .app_event_stream_text
            .contains("no agent results reported yet"),
        "initial ledger rows should be recorded in the app stream immediately:\n{}",
        initial_frame.app_event_stream_text
    );

    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(
            status_runtime_feed_supervisor_snapshot(
                (1..=40)
                    .map(|sequence| {
                        inline_runtime_feed_entry(
                            sequence,
                            format!("runtime stream marker {sequence:02}"),
                        )
                    })
                    .collect(),
            ),
        ));
    frame_recorder.draw_and_record_vt100(
        "runtime-tail",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    let runtime_tail_frame = frame_recorder.frame("runtime-tail");
    assert!(
        runtime_tail_frame
            .screen_text
            .contains("runtime stream marker 40"),
        "redraw should keep following the newest runtime event:\n{}",
        runtime_tail_frame.screen_text
    );
    assert!(
        !runtime_tail_frame
            .screen_text
            .contains("Recent Parallel Events")
            && !runtime_tail_frame
                .screen_text
                .contains("Parallel Event Stream"),
        "redraw with durable scrollback prefix should keep the visible tail data-only:\n{}",
        runtime_tail_frame.screen_text
    );
    assert!(
        runtime_tail_frame
            .terminal_history_text
            .contains("control tower is live"),
        "initial board status should remain in terminal history after redraw:\n{}",
        runtime_tail_frame.terminal_history_text
    );
    assert!(
        runtime_tail_frame
            .terminal_history_text
            .contains("no agent results reported yet"),
        "initial ledger status should remain in terminal history after redraw:\n{}",
        runtime_tail_frame.terminal_history_text
    );
    assert_eq!(
        runtime_tail_frame
            .terminal_history_text
            .matches("no agent results reported yet")
            .count(),
        1,
        "redraw should not duplicate snapshot status rows:\n{}",
        runtime_tail_frame.terminal_history_text
    );
    assert!(
        !runtime_tail_frame
            .host_scrollback_text
            .contains("Parallel Event Stream")
            && !runtime_tail_frame
                .host_scrollback_text
                .contains("Recent Parallel Events"),
        "host scrollback should persist stream rows but not live panel chrome:\n{}",
        runtime_tail_frame.host_scrollback_text
    );
    assert!(
        runtime_tail_frame
            .app_event_stream_text
            .contains("control tower is live")
            && runtime_tail_frame
                .app_event_stream_text
                .contains("runtime stream marker 40"),
        "app-side stream should contain both the initial status and latest runtime event:\n{}",
        runtime_tail_frame.app_event_stream_text
    );
}

#[test]
fn direct_frame_recorder_catches_wrapped_parallel_stream_split_at_live_boundary() {
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::HostScrollback, 72, 30);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(active_runtime_feed_snapshot(
        ParallelModePoolSlotState::Leased,
        "starting",
        vec![long_runtime_feed_entry(1, "seed runtime event one")],
    )));
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();
    let mut frame_recorder = tui_testkit::InlineFrameRecorder::default();

    frame_recorder.draw_and_record_vt100(
        "leased",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(active_runtime_feed_snapshot(
            ParallelModePoolSlotState::Running,
            "running",
            (1..=12)
                .map(|sequence| {
                    long_runtime_feed_entry(
                        sequence,
                        format!("session detail runtime write marker {sequence:02}"),
                    )
                })
                .collect(),
        )));
    frame_recorder.draw_and_record_vt100(
        "running-runtime-tail",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );

    let runtime_tail_frame = frame_recorder.frame("running-runtime-tail");
    let body_rows = parallel_event_stream_body_rows(&runtime_tail_frame.screen_text);
    let first_body_row = body_rows.first().unwrap_or_else(|| {
        panic!(
            "parallel stream body should have rows:\n{}",
            runtime_tail_frame.screen_text
        )
    });
    assert!(
        first_body_row.trim_start().starts_with('['),
        "the live stream must not start in the middle of a wrapped runtime row:\n{}",
        runtime_tail_frame.screen_text
    );
    assert!(
        !runtime_tail_frame
            .host_scrollback_text
            .contains("Parallel Event Stream")
            && !runtime_tail_frame
                .host_scrollback_text
                .contains("Recent Parallel Events"),
        "host scrollback should never receive live panel chrome:\n{}",
        runtime_tail_frame.host_scrollback_text
    );
    for sequence in 2..=12 {
        let marker = format!("runtime write marker {sequence:02}");
        assert_eq!(
            runtime_tail_frame
                .terminal_history_text
                .matches(&marker)
                .count(),
            1,
            "Ratatui word wrapping must not duplicate or drop a logical event at the sampled split: {marker}\n{}",
            runtime_tail_frame.terminal_history_text
        );
    }
}

#[test]
fn direct_frame_recorder_keeps_long_github_identity_error_exactly_once_through_refresh_and_resize()
{
    const ERROR_MARKER: &str = "GHID2061";
    let error_feed = || {
        vec![
            long_runtime_feed_entry(1, "runtime feed priming event"),
            long_runtime_feed_entry(
                2,
                format!(
                    "{ERROR_MARKER}: GitHub identity error: expected repo-local RefinedStone \
                     credentials but the asynchronous probe returned a different account"
                ),
            ),
        ]
    };

    let mut terminal = tui_testkit::inline_history_vt100_terminal(
        InlineHistoryRenderMode::HostScrollback,
        100,
        30,
    );
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(active_runtime_feed_snapshot(
        ParallelModePoolSlotState::Leased,
        "starting",
        vec![long_runtime_feed_entry(1, "runtime feed priming event")],
    )));
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();
    let mut recorder = tui_testkit::InlineFrameRecorder::default();

    recorder.draw_and_record_vt100(
        "github-identity-primed",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(active_runtime_feed_snapshot(
            ParallelModePoolSlotState::Running,
            "github identity check failed",
            error_feed(),
        )));
    recorder.draw_and_record_vt100(
        "github-identity-live",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );

    let live = recorder.frame("github-identity-live");
    assert_eq!(
        live.app_event_stream_text.matches(ERROR_MARKER).count(),
        1,
        "one authority sequence must produce one canonical event:\n{}",
        live.app_event_stream_text
    );
    assert_eq!(
        live.screen_text.matches(ERROR_MARKER).count(),
        1,
        "the long identity error should begin in the live partition:\n{}",
        live.screen_text
    );
    assert_eq!(
        live.host_scrollback_text.matches(ERROR_MARKER).count(),
        0,
        "the live event must not also be present in host scrollback:\n{}",
        live.host_scrollback_text
    );
    let error_event_id = runtime
        .app()
        .shell
        .parallel_event_stream
        .snapshot()
        .events()
        .iter()
        .find(|event| event.line().to_string().contains(ERROR_MARKER))
        .expect("the authority error should retain one typed stream event")
        .id();
    assert!(
        !inline_terminal
            .parallel_delivery
            .has_delivered(error_event_id),
        "the authority error should begin beyond the committed host frontier"
    );

    for refresh_state in [
        "github identity retry queued",
        "github identity retry running",
        "github identity retry waiting",
        "github identity retry observed",
    ] {
        runtime
            .app_mut()
            .set_parallel_mode_supervisor_snapshot_for_test(Some(active_runtime_feed_snapshot(
                ParallelModePoolSlotState::Running,
                refresh_state,
                error_feed(),
            )));
        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("asynchronous supervisor refresh draw");
    }
    tui_testkit::resize_inline_history_vt100_terminal(&mut terminal, 42, 18);
    recorder.draw_and_record_vt100(
        "github-identity-after-refresh-and-shrink",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );

    let shifted = recorder.frame("github-identity-after-refresh-and-shrink");
    assert_eq!(
        shifted.app_event_stream_text.matches(ERROR_MARKER).count(),
        1,
        "repeated snapshots with the same authority sequence must not append the error again:\n{}",
        shifted.app_event_stream_text
    );
    assert!(
        inline_terminal
            .parallel_delivery
            .has_delivered(error_event_id),
        "the event should cross the exact typed host frontier after refresh and shrink"
    );
    assert_eq!(
        shifted.terminal_history_text.matches(ERROR_MARKER).count(),
        1,
        "combined terminal history must contain the wrapped authority event exactly once:\n{}",
        shifted.terminal_history_text
    );

    tui_testkit::resize_inline_history_vt100_terminal(&mut terminal, 100, 30);
    recorder.draw_and_record_vt100(
        "github-identity-restored",
        &mut terminal,
        &mut runtime,
        &mut inline_terminal,
    );
    let restored = recorder.frame("github-identity-restored");
    assert_eq!(
        restored.terminal_history_text.matches(ERROR_MARKER).count(),
        1,
        "restoring terminal geometry must preserve the committed frontier:\n{}",
        restored.terminal_history_text
    );
}

#[test]
fn parallel_history_fit_clears_live_panel_before_scrollback_adjustment() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.set_parallel_mode_enabled_for_test(true);
    for index in 0..12 {
        app.push_parallel_supervisor_event_for_test(
            "11:45:02",
            "Task Intake",
            format!("fit-event-{index:02}"),
        );
    }
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("initial parallel event draw transaction");
    let screen_text = tui_testkit::screen_text(&terminal);
    assert!(
        !screen_text.contains("Parallel Event Stream")
            && !screen_text.contains("Recent Parallel Events"),
        "overflowed stream should not insert a title between durable history and visible rows:\n{screen_text}"
    );

    let viewport_top = terminal.get_frame().area().top();
    inline_terminal.history_flush.visible_history_rows = viewport_top.saturating_add(2);
    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());

    let terminal_scrollback = tui_testkit::inline_scrollback_text(&terminal);
    assert!(
        !terminal_scrollback.contains("Parallel Event Stream")
            && !terminal_scrollback.contains("Recent Parallel Events"),
        "viewport fitting must not push the live stream title into host scrollback:\n{terminal_scrollback}"
    );
    assert!(
        !terminal_scrollback.contains("Command Hints"),
        "viewport fitting must not push live panel footer chrome into host scrollback:\n{terminal_scrollback}"
    );
}

fn runtime_feed_supervisor_snapshot(
    runtime_event_feed: Vec<ParallelModeRuntimeEventFeedEntry>,
) -> ParallelModeSupervisorSnapshot {
    ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/root",
        ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
        ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle")
            .with_runtime_event_feed(runtime_event_feed),
        None,
    )
}

fn status_runtime_feed_supervisor_snapshot(
    runtime_event_feed: Vec<ParallelModeRuntimeEventFeedEntry>,
) -> ParallelModeSupervisorSnapshot {
    ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/root",
        ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
        ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
        ParallelModeDistributorSnapshot::new(
            Vec::new(),
            vec![
                ParallelModeCompletionFeedEntry::new("reported", "no agent results reported yet"),
                ParallelModeCompletionFeedEntry::new(
                    "ledger refreshing",
                    "no official refresh workers are active",
                ),
                ParallelModeCompletionFeedEntry::new("official", "nothing is queued for merge"),
                ParallelModeCompletionFeedEntry::new(
                    "merge queued",
                    "no distributor queue items are waiting",
                ),
                ParallelModeCompletionFeedEntry::new(
                    "merged",
                    "nothing has been integrated into prerelease yet",
                ),
            ],
            "idle",
            "queue idle",
        )
        .with_runtime_event_feed(runtime_event_feed),
        Some("control tower is live in read-only supervisor mode".to_string()),
    )
}

fn active_runtime_feed_snapshot(
    slot_state: ParallelModePoolSlotState,
    detail_state: &'static str,
    runtime_event_feed: Vec<ParallelModeRuntimeEventFeedEntry>,
) -> ParallelModeSupervisorSnapshot {
    ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        "/tmp/root",
        ParallelModePoolBoardSnapshot::new(
            3,
            "/tmp/pool",
            "idle",
            vec![ParallelModePoolSlotSnapshot::new(
                "slot-1",
                slot_state,
                "akra-agent/slot-1/task-user-20260518T062218Z-398bb0bebece",
                "/tmp/pool/slot-1",
                "agent-artificer / task-user-20260518T062218Z-398bb0bebece",
            )],
        ),
        ParallelModeAgentRosterSnapshot::new(
            vec![ParallelModeAgentRosterEntry::new(
                "agent-artificer",
                "안녕하세요",
                "slot-1",
                "akra-agent/slot-1/task-user-20260518T062218Z-398bb0bebece",
                detail_state,
                detail_state,
                "agent session entered the running state",
            )],
            "no active agents",
        ),
        ParallelModeSupervisorDetailSnapshot::new(
            Some(ParallelModeAgentSessionDetailSnapshot::new(
                "slot-1@2026-05-19T02:30:35.874316+00:00",
                "agent-artificer",
                "task-user-20260518T062218Z-398bb0bebece",
                "안녕하세요",
                "slot-1",
                Some("019e3e12-224e-71f0-b...".to_string()),
                "/tmp/pool/slot-1",
                "akra-agent/slot-1/task-user-20260518T062218Z-398bb0bebece",
                "2026-05-19T02:30:35.874316+00:00",
                detail_state,
                "in_progress",
                "agent session entered the running state",
                "not reported yet",
                "not refreshed yet",
                None,
                vec![
                    ParallelModeAgentSessionHistoryEntry::new(
                        "assigned",
                        "2026-05-19T02:30:35.874316+00:00",
                        "slot lease acquired and branch reserved for launch",
                    ),
                    ParallelModeAgentSessionHistoryEntry::new(
                        "starting",
                        "2026-05-19T02:30:36.131687+00:00",
                        "thread prepared for the leased session",
                    ),
                ],
                "2026-05-19T02:30:37.131687+00:00",
            )),
            "no detail",
        ),
        ParallelModeDistributorSnapshot::new(
            Vec::new(),
            vec![
                ParallelModeCompletionFeedEntry::new("reported", "no agent results reported yet"),
                ParallelModeCompletionFeedEntry::new(
                    "ledger refreshing",
                    "no official refresh workers are active",
                ),
                ParallelModeCompletionFeedEntry::new("official", "nothing is queued for merge"),
                ParallelModeCompletionFeedEntry::new(
                    "merge queued",
                    "no distributor queue items are waiting",
                ),
                ParallelModeCompletionFeedEntry::new(
                    "merged",
                    "nothing has been integrated into prerelease yet",
                ),
            ],
            "idle",
            "queue idle",
        )
        .with_runtime_event_feed(runtime_event_feed),
        Some("control tower is live in read-only supervisor mode".to_string()),
    )
}

fn inline_runtime_feed_entry(
    sequence: i64,
    summary: impl Into<String>,
) -> ParallelModeRuntimeEventFeedEntry {
    ParallelModeRuntimeEventFeedEntry::new(
        sequence,
        "parallel_runtime_reset",
        "parallel_runtime",
        "pool",
        60,
        summary,
        format!("2026-05-13T11:45:{sequence:02}+00:00"),
    )
}

fn long_runtime_feed_entry(
    sequence: i64,
    summary: impl Into<String>,
) -> ParallelModeRuntimeEventFeedEntry {
    ParallelModeRuntimeEventFeedEntry::new(
        sequence,
        "session_detail_upsert",
        "session_detail",
        "slot-1@2026-05-19T02:30:35.874316+00:00",
        61,
        format!(
            "{} / runtime session detail stored / session: slot-1@2026-05-19T02:30:35.874316+00:00",
            summary.into(),
        ),
        format!("2026-05-19T02:30:{sequence:02}+00:00"),
    )
}

fn parallel_event_stream_body_rows(screen_text: &str) -> Vec<String> {
    let lines: Vec<&str> = screen_text.lines().collect();
    let start_index = lines
        .iter()
        .position(|line| {
            let row = line.trim_start().trim_start_matches('"').trim_start();
            line.contains("Parallel Event Stream")
                || line.contains("Recent Parallel Events")
                || row.starts_with('[')
        })
        .map(|index| {
            if lines[index].contains("Parallel Event Stream")
                || lines[index].contains("Recent Parallel Events")
            {
                index + 1
            } else {
                index
            }
        })
        .unwrap_or(lines.len());

    lines
        .into_iter()
        .skip(start_index)
        .take_while(|line| !line.contains("Command Hints"))
        .map(|line| {
            line.trim_end()
                .trim_start()
                .trim_start_matches('"')
                .trim_start()
                .to_string()
        })
        .filter(|line| !line.trim().is_empty())
        .collect()
}

#[test]
fn vt100_parallel_history_fit_does_not_push_live_panel_chrome() {
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.set_parallel_mode_enabled_for_test(true);
    for index in 0..12 {
        app.push_parallel_supervisor_event_for_test(
            "11:45:02",
            "Task Intake",
            format!("vt100-fit-event-{index:02}"),
        );
    }
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("initial vt100 parallel event draw transaction");

    let viewport_top = terminal.get_frame().area().top();
    inline_terminal.history_flush.visible_history_rows = viewport_top.saturating_add(2);
    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());

    let host_scrollback = tui_testkit::inline_vt100_host_scrollback_text(&mut terminal);
    assert!(
        !host_scrollback.contains("Parallel Event Stream")
            && !host_scrollback.contains("Recent Parallel Events"),
        "vt100 viewport fitting must not push the live stream title into host scrollback:\n{host_scrollback}"
    );
    assert!(
        !host_scrollback.contains("Command Hints"),
        "vt100 viewport fitting must not push live panel footer chrome into host scrollback:\n{host_scrollback}"
    );
}

#[test]
fn vt100_newline_fallback_parallel_delta_keeps_chrome_out_of_host_scrollback() {
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.shell.history_insert_mode = HistoryInsertionMode::NewlineFallback;
    app.set_parallel_mode_enabled_for_test(true);
    for index in 0..20 {
        app.push_parallel_supervisor_event_for_test(
            "11:45:02",
            "Task Intake",
            format!("fallback-event-{index:02}"),
        );
    }
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("initial vt100 fallback parallel event draw transaction");
    for index in 20..40 {
        runtime.app_mut().push_parallel_supervisor_event_for_test(
            "11:45:03",
            "Task Intake",
            format!("fallback-event-{index:02}"),
        );
    }
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("delta vt100 fallback parallel event draw transaction");

    let host_scrollback = tui_testkit::inline_vt100_host_scrollback_text(&mut terminal);
    assert!(
        host_scrollback.contains("fallback-event-00"),
        "fallback insertion should preserve historical event rows:\n{host_scrollback}"
    );
    assert!(
        host_scrollback.contains("fallback-event-20"),
        "fallback insertion should append delta event rows:\n{host_scrollback}"
    );
    assert!(
        !host_scrollback.contains("Parallel Event Stream")
            && !host_scrollback.contains("Recent Parallel Events"),
        "fallback insertion must not push live stream title into host scrollback:\n{host_scrollback}"
    );
    assert!(
        !host_scrollback.contains("Command Hints"),
        "fallback insertion must not push live footer chrome into host scrollback:\n{host_scrollback}"
    );
}

// VT100 coverage catches terminal-app behavior that TestBackend cannot:
// newline fallback history must survive live completion and viewport resize
// without duplicating markers.
#[test]
fn vt100_terminal_app_preserves_newline_fallback_history_after_live_resize() {
    let mut terminal =
        tui_testkit::inline_history_vt100_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    app.shell.history_insert_mode = HistoryInsertionMode::NewlineFallback;
    append_user_history_message(&mut app, "HISTORY_MARKER_ONE user prompt");
    append_history_message(&mut app, "HISTORY_MARKER_TWO committed answer");
    append_user_history_message(&mut app, "HISTORY_MARKER_THREE follow-up prompt");
    append_history_message(&mut app, "HISTORY_MARKER_FOUR committed answer");
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should start in a ready conversation state");
    };
    set_running_turn(conversation, "turn-1");
    conversation.push_live_agent_delta(
        "agent-live".to_string(),
        Some("final_answer".to_string()),
        "LIVE_MARKER_FIVE streaming answer".to_string(),
    );
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("initial vt100 draw transaction");
    let terminal_history_before_completion =
        tui_testkit::inline_vt100_scrollback_text(&mut terminal);
    assert!(
        terminal_history_before_completion.contains("HISTORY_MARKER_TWO"),
        "committed history should be in terminal app history before live completion: {terminal_history_before_completion:?}"
    );
    let host_scrollback_before_completion =
        tui_testkit::inline_vt100_host_scrollback_text(&mut terminal);
    assert!(
        !host_scrollback_before_completion.contains("LIVE_MARKER_FIVE"),
        "live streaming rows must not be flushed into host scrollback before completion: {host_scrollback_before_completion:?}"
    );
    let app = runtime.app_mut();
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should stay in a ready conversation state");
    };
    assert!(conversation.complete_live_agent_message(
        "agent-live".to_string(),
        Some("final_answer".to_string()),
        "LIVE_MARKER_FIVE final answer".to_string(),
    ));

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("completion vt100 draw transaction");
    tui_testkit::resize_inline_history_vt100_terminal(&mut terminal, 80, 8);
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("short vt100 draw transaction");
    tui_testkit::resize_inline_history_vt100_terminal(&mut terminal, 80, 24);
    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("restored vt100 draw transaction");
    let terminal_history = tui_testkit::inline_vt100_scrollback_text(&mut terminal);
    for marker in [
        "HISTORY_MARKER_ONE",
        "HISTORY_MARKER_TWO",
        "HISTORY_MARKER_THREE",
        "HISTORY_MARKER_FOUR",
        "LIVE_MARKER_FIVE",
    ] {
        assert!(
            terminal_history.contains(marker),
            "terminal app history lost {marker} after live commit and resize:\n{terminal_history}"
        );
        assert_eq!(
            terminal_history.matches(marker).count(),
            1,
            "terminal app history duplicated {marker} after live commit and resize:\n{terminal_history}"
        );
    }
}

// Viewport replay mode repaints the active viewport instead of inserting
// history into host scrollback; host-scrollback mode should do the opposite.
#[test]
fn viewport_replay_sync_skips_host_scrollback_insertions() {
    const THREAD_A_MARKER: &str = "REPLAY_THREAD_A_MARKER";
    const THREAD_B_MARKER: &str = "REPLAY_THREAD_B_MARKER";
    let mut replay_terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::ViewportReplay, 80, 24);
    let mut replay_app = make_test_app();
    replay_app.shell.show_startup_ascii_art = false;
    replay_app.shell.inline_history_render_mode = InlineHistoryRenderMode::ViewportReplay;
    append_history_message(&mut replay_app, THREAD_A_MARKER);
    let mut replay_runtime = ShellRuntime::new(replay_app);
    let mut replay_viewport = InlineTerminalState::default();
    let mut replay_frames = tui_testkit::InlineFrameRecorder::default();

    replay_frames.draw_and_record(
        "replay-thread-a",
        &mut replay_terminal,
        &mut replay_runtime,
        &mut replay_viewport,
    );
    assert!(
        replay_frames
            .frame("replay-thread-a")
            .screen_text
            .contains(THREAD_A_MARKER)
    );
    let replay_scrollback_before = replay_frames
        .frame("replay-thread-a")
        .host_scrollback_text
        .clone();

    load_history_snapshot(
        replay_runtime.app_mut(),
        "replay-thread-b",
        vec![ConversationMessage::new(
            ConversationMessageKind::Agent,
            THREAD_B_MARKER,
            None,
            None,
        )],
    );
    replay_frames.draw_and_record(
        "replay-thread-b",
        &mut replay_terminal,
        &mut replay_runtime,
        &mut replay_viewport,
    );
    let replay_switched = replay_frames.frame("replay-thread-b");
    assert_eq!(
        replay_switched.screen_text.matches(THREAD_B_MARKER).count(),
        1
    );
    assert!(!replay_switched.screen_text.contains(THREAD_A_MARKER));
    assert_eq!(
        replay_switched.host_scrollback_text, replay_scrollback_before,
        "replay identity switches must repaint without host insertion"
    );
    assert!(
        replay_viewport.back_buffer_trustworthy(),
        "the replacement replay frame should establish a fresh trusted buffer"
    );
    let mut host_terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut host_app = make_test_app();
    host_app.shell.show_startup_ascii_art = false;
    host_app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    append_history_message(&mut host_app, "history should be inserted in host mode");
    let mut host_runtime = ShellRuntime::new(host_app);
    let mut host_viewport = InlineTerminalState::default();

    assert!(
        sync_inline_viewport(&mut host_terminal, &mut host_runtime, &mut host_viewport).unwrap()
    );
    assert!(
        tui_testkit::inline_terminal_history_text(&host_terminal)
            .contains("history should be inserted")
    );
}
#[test]
fn viewport_replay_ignores_stale_host_scrollback_row_accounting() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::ViewportReplay;
    append_history_message(
        &mut app,
        "replay mode should not consume stale host scrollback accounting",
    );
    let mut runtime = ShellRuntime::new(app);
    let mut inline_viewport = InlineTerminalState::default();
    inline_viewport.history_flush.visible_history_rows = 12;
    let scrollback_rows_before = terminal.backend().inner().scrollback().area.height;

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_viewport).unwrap());

    assert_eq!(
        terminal.backend().inner().scrollback().area.height,
        scrollback_rows_before,
        "viewport replay should not append blank host scrollback rows when stale host accounting survives: {:?}",
        tui_testkit::inline_scrollback_text(&terminal)
    );
}
#[test]
fn viewport_replay_keeps_inline_viewport_for_shell_positioning() {
    assert_eq!(
        terminal_options_for_render_mode(InlineHistoryRenderMode::ViewportReplay).viewport,
        Viewport::Inline(INLINE_VIEWPORT_HEIGHT)
    );
    assert_eq!(
        terminal_options_for_render_mode(InlineHistoryRenderMode::HostScrollback).viewport,
        Viewport::Inline(INLINE_VIEWPORT_HEIGHT)
    );
}

// The adapter tracks cursor position after the initial query so append_lines
// and cursor reads do not repeatedly call through to the inner backend.
#[test]
fn inline_backend_reuses_tracked_cursor_after_initial_query() {
    let backend =
        InlineTerminalBackend::new(CursorQueryCountingBackend::new(TestBackend::new(80, 24)));
    let mut terminal = Terminal::with_options(
        backend,
        terminal_options_for_render_mode(InlineHistoryRenderMode::HostScrollback),
    )
    .expect("inline terminal should initialize");
    let initial_query_count = terminal.backend_mut().inner().cursor_query_count();

    terminal
        .set_cursor_position(Position::new(3, 4))
        .expect("cursor should move");
    assert_eq!(
        terminal.get_cursor_position().expect("cursor should read"),
        Position::new(3, 4)
    );
    assert_eq!(
        terminal.backend_mut().inner().cursor_query_count(),
        initial_query_count,
        "tracked cursor reads should not call the inner crossterm position query again"
    );

    terminal
        .backend_mut()
        .append_lines(2)
        .expect("append should scroll from tracked cursor");
    assert_eq!(
        terminal.get_cursor_position().expect("cursor should read"),
        Position::new(0, 6)
    );
    assert_eq!(
        terminal.backend_mut().inner().cursor_query_count(),
        initial_query_count,
        "append-line cursor tracking should still avoid another terminal query"
    );

    terminal
        .backend_mut()
        .inner_mut()
        .report_resize_after_size_queries(1, 80, 5);
    terminal
        .backend_mut()
        .append_lines(1)
        .expect("append should tolerate a concurrent physical resize");
    assert_eq!(
        terminal
            .get_cursor_position()
            .expect("cursor should refresh")
            .y,
        4,
        "refreshed cursor must be clamped inside the reported physical height"
    );
    assert_eq!(
        terminal.backend_mut().inner().cursor_query_count(),
        initial_query_count + 1,
        "append-time resize must discard the estimated cursor and query the physical position"
    );
}

#[test]
fn physical_shrink_requeries_cursor_before_inline_autoresize() {
    let mut inner = CursorQueryCountingBackend::new(TestBackend::new(80, 40));
    inner
        .set_cursor_position(Position::new(0, 39))
        .expect("fixture cursor should start at the physical bottom");
    let backend = InlineTerminalBackend::new(inner);
    let mut terminal = Terminal::with_options(
        backend,
        terminal_options_for_render_mode(InlineHistoryRenderMode::HostScrollback),
    )
    .expect("bottom-anchored inline terminal should initialize");
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    if let ConversationState::Ready(conversation) =
        &mut app.conversation.lifecycle.conversation_state
    {
        conversation.composer.input_buffer = "physical shrink prompt".to_string();
    }
    append_history_message(&mut app, "physical shrink history");
    let mut runtime = ShellRuntime::new(app);
    let mut inline_viewport = InlineTerminalState::default();

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_viewport).unwrap());
    draw_test_frame(&mut terminal, &mut runtime, &mut inline_viewport);
    let cursor_queries_before_resize = terminal.backend_mut().inner().cursor_query_count();
    let appended_lines_before_resize = terminal.backend_mut().inner().appended_lines();

    terminal
        .backend_mut()
        .inner_mut()
        .resize_and_clamp_cursor(48, 10);
    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_viewport).unwrap());
    draw_test_frame(&mut terminal, &mut runtime, &mut inline_viewport);

    assert_eq!(
        terminal.backend_mut().inner().cursor_query_count(),
        cursor_queries_before_resize + 1,
        "physical resize must refresh the cursor used to anchor the inline viewport"
    );
    assert_eq!(
        terminal.backend_mut().inner().appended_lines(),
        appended_lines_before_resize,
        "physical resize must not append a second history-fit adjustment"
    );
    let viewport_area = terminal.get_frame().area();
    assert!(
        viewport_area.bottom() <= 10,
        "shrunk inline viewport must stay inside the physical screen: {viewport_area:?}"
    );
    let screen_text = tui_testkit::buffer_text(terminal.backend().inner().inner.buffer());
    assert_eq!(
        screen_text.matches("> physical shrink prompt").count(),
        1,
        "shrink redraw must keep one visible prompt without a stale tail: {screen_text:?}"
    );

    let cursor_queries_after_shrink = terminal.backend_mut().inner().cursor_query_count();
    let _ = sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_viewport).unwrap();
    assert_eq!(
        terminal.backend_mut().inner().cursor_query_count(),
        cursor_queries_after_shrink,
        "unchanged geometry must continue using the refreshed cursor cache"
    );

    terminal
        .backend_mut()
        .inner_mut()
        .resize_and_clamp_cursor(80, 40);
    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_viewport).unwrap());
    draw_test_frame(&mut terminal, &mut runtime, &mut inline_viewport);
    assert_eq!(
        terminal.backend_mut().inner().cursor_query_count(),
        cursor_queries_after_shrink + 1,
        "physical restore must refresh the cursor exactly once"
    );
    let restored_screen = tui_testkit::buffer_text(terminal.backend().inner().inner.buffer());
    assert_eq!(
        restored_screen.matches("> physical shrink prompt").count(),
        1,
        "restore redraw must keep one visible prompt without stale tail replay: {restored_screen:?}"
    );
}

#[test]
fn draw_resize_races_defer_physical_history_reconciliation() {
    for resize_after_flush in [false, true] {
        let mut inner = CursorQueryCountingBackend::new(TestBackend::new(80, 40));
        inner
            .set_cursor_position(Position::new(0, 39))
            .expect("fixture cursor should start at the physical bottom");
        let backend = InlineTerminalBackend::new(inner);
        let mut terminal = Terminal::with_options(
            backend,
            terminal_options_for_render_mode(InlineHistoryRenderMode::HostScrollback),
        )
        .expect("bottom-anchored inline terminal should initialize");
        let mut app = make_test_app();
        app.shell.show_startup_ascii_art = false;
        let mut runtime = ShellRuntime::new(app);
        let mut inline_terminal = InlineTerminalState::default();

        assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
        assert!(draw_test_frame(
            &mut terminal,
            &mut runtime,
            &mut inline_terminal
        ));
        assert!(runtime.take_redraw_request());
        inline_terminal.history_flush.visible_history_rows = 30;

        if resize_after_flush {
            terminal
                .backend_mut()
                .inner_mut()
                .resize_on_next_flush(48, 10);
        } else {
            terminal
                .backend_mut()
                .inner_mut()
                .resize_and_clamp_cursor(48, 10);
        }
        assert!(!draw_test_frame(
            &mut terminal,
            &mut runtime,
            &mut inline_terminal
        ));
        assert_resize_retry_scheduled(
            &mut runtime,
            "draw-time resize must schedule a follow-up frame",
        );

        assert_eq!(
            inline_terminal.last_known_screen_size(),
            Some(Size::new(80, 40)),
            "draw-time resize must leave the prior geometry for the next sync; after_flush={resize_after_flush}"
        );
        assert!(
            !inline_terminal.back_buffer_trustworthy(),
            "draw-time resize must invalidate the mixed-geometry frame; after_flush={resize_after_flush}"
        );
        let appended_lines = terminal.backend_mut().inner().appended_lines();

        assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
        assert_eq!(
            terminal.backend_mut().inner().appended_lines(),
            appended_lines,
            "deferred physical resize must reconcile accounting without appending lines; after_flush={resize_after_flush}"
        );
        draw_test_frame(&mut terminal, &mut runtime, &mut inline_terminal);
        assert!(
            terminal.get_frame().area().bottom() <= 10,
            "reconciled viewport must stay inside the shrunken terminal; after_flush={resize_after_flush}"
        );
    }
}

#[test]
fn sync_resize_race_uses_one_stable_geometry_snapshot() {
    let mut inner = CursorQueryCountingBackend::new(TestBackend::new(80, 40));
    inner
        .set_cursor_position(Position::new(0, 39))
        .expect("fixture cursor should start at the physical bottom");
    let backend = InlineTerminalBackend::new(inner);
    let mut terminal = Terminal::with_options(
        backend,
        terminal_options_for_render_mode(InlineHistoryRenderMode::HostScrollback),
    )
    .expect("bottom-anchored inline terminal should initialize");
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
    draw_test_frame(&mut terminal, &mut runtime, &mut inline_terminal);
    inline_terminal.history_flush.visible_history_rows = 30;
    let appended_lines = terminal.backend_mut().inner().appended_lines();
    terminal
        .backend_mut()
        .inner_mut()
        .report_resize_after_size_queries(3, 48, 10);

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
    assert_eq!(
        inline_terminal.last_known_screen_size(),
        Some(Size::new(48, 10))
    );
    assert!(
        inline_terminal
            .viewport_area()
            .is_some_and(|area| area.bottom() <= 10),
        "sync must not combine the new physical size with the old viewport: {:?}",
        inline_terminal.viewport_area()
    );
    assert_eq!(
        terminal.backend_mut().inner().appended_lines(),
        appended_lines,
        "a resize observed during autoresize must not be applied again as a history fit"
    );
}

#[test]
fn resize_after_stability_snapshot_defers_history_mutation() {
    let mut inner = CursorQueryCountingBackend::new(TestBackend::new(80, 40));
    inner
        .set_cursor_position(Position::new(0, 39))
        .expect("fixture cursor should start at the physical bottom");
    let backend = InlineTerminalBackend::new(inner);
    let mut terminal = Terminal::with_options(
        backend,
        terminal_options_for_render_mode(InlineHistoryRenderMode::HostScrollback),
    )
    .expect("bottom-anchored inline terminal should initialize");
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.history_insert_mode = HistoryInsertionMode::StandardScrollRegion;
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
    draw_test_frame(&mut terminal, &mut runtime, &mut inline_terminal);
    inline_terminal.history_flush.visible_history_rows = 30;
    append_history_message(runtime.app_mut(), "post-snapshot resize history");
    let rendered_lines = inline_terminal.history_flush.rendered_lines.clone();
    let appended_lines = terminal.backend_mut().inner().appended_lines();
    terminal
        .backend_mut()
        .inner_mut()
        .report_resize_after_size_queries(4, 48, 10);

    assert!(!sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
    assert_eq!(
        terminal.backend_mut().inner().appended_lines(),
        appended_lines
    );
    assert_eq!(inline_terminal.history_flush.visible_history_rows, 30);
    assert_eq!(inline_terminal.history_flush.rendered_lines, rendered_lines);
    assert_eq!(
        inline_terminal.last_known_screen_size(),
        Some(Size::new(80, 40))
    );
    assert!(!inline_terminal.back_buffer_trustworthy());

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
    assert_eq!(
        inline_terminal.last_known_screen_size(),
        Some(Size::new(48, 10))
    );
    assert_eq!(
        terminal.backend_mut().inner().appended_lines(),
        appended_lines
    );
    assert!(
        inline_terminal
            .history_flush
            .rendered_lines
            .iter()
            .any(|line| line.to_string().contains("post-snapshot resize history"))
    );
}

#[test]
fn resize_after_history_insertion_commits_once_and_marks_row_accounting_dirty() {
    let mut inner = CursorQueryCountingBackend::new(TestBackend::new(80, 40));
    inner
        .set_cursor_position(Position::new(0, 39))
        .expect("fixture cursor should start at the physical bottom");
    let backend = InlineTerminalBackend::new(inner);
    let mut terminal = Terminal::with_options(
        backend,
        terminal_options_for_render_mode(InlineHistoryRenderMode::HostScrollback),
    )
    .expect("bottom-anchored inline terminal should initialize");
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.history_insert_mode = HistoryInsertionMode::StandardScrollRegion;
    append_history_message(&mut app, "stable baseline history");
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
    draw_test_frame(&mut terminal, &mut runtime, &mut inline_terminal);
    inline_terminal.history_flush.visible_history_rows = 5;
    append_history_message(runtime.app_mut(), "resize-during-insert history");
    let scroll_region_up_count = terminal.backend_mut().inner().scroll_region_up_count();
    terminal
        .backend_mut()
        .inner_mut()
        .resize_after_next_scroll_region_up(48, 40);

    assert!(!sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
    let committed_visible_rows = inline_terminal.history_flush.visible_history_rows;
    assert!(committed_visible_rows > 5);
    assert!(inline_terminal.history_flush.visible_history_rows_dirty);
    assert!(
        inline_terminal
            .history_flush
            .rendered_lines
            .iter()
            .any(|line| line.to_string().contains("resize-during-insert history"))
    );
    assert_eq!(
        terminal.backend_mut().inner().scroll_region_up_count(),
        scroll_region_up_count + 1
    );
    assert_eq!(
        inline_terminal.last_known_screen_size(),
        Some(Size::new(80, 40))
    );
    assert!(!inline_terminal.back_buffer_trustworthy());

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
    assert_eq!(
        inline_terminal.last_known_screen_size(),
        Some(Size::new(48, 40))
    );
    assert!(!inline_terminal.history_flush.visible_history_rows_dirty);
    assert_eq!(
        inline_terminal.history_flush.visible_history_rows,
        committed_visible_rows
    );
    assert_eq!(
        terminal.backend_mut().inner().scroll_region_up_count(),
        scroll_region_up_count + 1,
        "a completed insertion must not replay after resize reconciliation"
    );
    assert!(
        inline_terminal
            .history_flush
            .rendered_lines
            .iter()
            .any(|line| line.to_string().contains("resize-during-insert history"))
    );
}

#[test]
fn due_draw_drains_shrink_restore_events_before_history_accounting() {
    let mut inner = CursorQueryCountingBackend::new(TestBackend::new(80, 40));
    inner
        .set_cursor_position(Position::new(0, 39))
        .expect("fixture cursor should start at the physical bottom");
    let backend = InlineTerminalBackend::new(inner);
    let mut terminal = Terminal::with_options(
        backend,
        terminal_options_for_render_mode(InlineHistoryRenderMode::HostScrollback),
    )
    .expect("bottom-anchored inline terminal should initialize");
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
    draw_test_frame(&mut terminal, &mut runtime, &mut inline_terminal);
    inline_terminal.history_flush.visible_history_rows = 30;
    let cursor_queries = terminal.backend_mut().inner().cursor_query_count();
    let appended_lines = terminal.backend_mut().inner().appended_lines();

    terminal
        .backend_mut()
        .inner_mut()
        .resize_and_clamp_cursor(48, 10);
    terminal
        .backend_mut()
        .inner_mut()
        .resize_and_clamp_cursor(80, 40);
    assert_eq!(
        runtime.next_event_poll_timeout(Instant::now(), Duration::from_secs(1)),
        Duration::ZERO,
        "a draw must already be due before the queued resize events are drained"
    );
    let mut ready_events = VecDeque::from([Event::Resize(48, 10), Event::Resize(80, 40)]);

    assert!(
        prepare_runtime_for_due_draw(&mut runtime, || Ok(ready_events.pop_front())).unwrap(),
        "the due draw should be consumed only after both resize events"
    );
    assert!(ready_events.is_empty());

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
    assert_eq!(runtime.terminal_resize_epoch(), 2);
    assert_eq!(
        terminal.backend_mut().inner().cursor_query_count(),
        cursor_queries + 1,
        "coalesced resize events must invalidate the same-size cursor cache"
    );
    assert_eq!(
        terminal.backend_mut().inner().appended_lines(),
        appended_lines
    );
    assert_eq!(inline_terminal.history_flush.visible_history_rows, 24);
    assert!(!inline_terminal.back_buffer_trustworthy());
}

#[test]
fn autoresize_retries_sampled_shrink_restore_before_history_mutation() {
    let mut inner = CursorQueryCountingBackend::new(TestBackend::new(80, 40));
    inner
        .set_cursor_position(Position::new(0, 39))
        .expect("fixture cursor should start at the physical bottom");
    let backend = InlineTerminalBackend::new(inner);
    let mut terminal = Terminal::with_options(
        backend,
        terminal_options_for_render_mode(InlineHistoryRenderMode::HostScrollback),
    )
    .expect("bottom-anchored inline terminal should initialize");
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
    draw_test_frame(&mut terminal, &mut runtime, &mut inline_terminal);
    inline_terminal.history_flush.visible_history_rows = 30;
    let appended_lines = terminal.backend_mut().inner().appended_lines();
    terminal
        .backend_mut()
        .inner_mut()
        .report_resize_then_restore(2, Size::new(48, 10), 3, Size::new(80, 40));

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
    assert_eq!(
        terminal.backend_mut().inner().appended_lines(),
        appended_lines
    );
    assert_eq!(
        inline_terminal.last_known_screen_size(),
        Some(Size::new(80, 40))
    );
    let viewport_area = inline_terminal
        .viewport_area()
        .expect("restored viewport should be recorded");
    assert_eq!(viewport_area.width, 80);
    assert_eq!(viewport_area.height, 16);
    assert!(viewport_area.bottom() <= 40);
    assert_eq!(
        inline_terminal.history_flush.visible_history_rows,
        viewport_area.top()
    );
    assert!(!inline_terminal.back_buffer_trustworthy());
}

#[test]
fn unstable_resize_retries_before_pending_quit_and_history_reconciliation() {
    let mut inner = CursorQueryCountingBackend::new(TestBackend::new(80, 40));
    inner
        .set_cursor_position(Position::new(0, 39))
        .expect("fixture cursor should start at the physical bottom");
    let backend = InlineTerminalBackend::new(inner);
    let mut terminal = Terminal::with_options(
        backend,
        terminal_options_for_render_mode(InlineHistoryRenderMode::HostScrollback),
    )
    .expect("bottom-anchored inline terminal should initialize");
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_terminal).unwrap());
    draw_test_frame(&mut terminal, &mut runtime, &mut inline_terminal);
    assert!(runtime.take_redraw_request());
    inline_terminal.history_flush.visible_history_rows = 30;
    runtime
        .app_mut()
        .dispatch_shell_chrome(ShellChromeEvent::ExitConfirmationShown);
    let transaction_completed =
        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("exit confirmation should draw before confirmation");
    assert!(transaction_completed);
    let modal_screen = tui_testkit::buffer_text(terminal.backend().inner().inner.buffer());
    assert!(modal_screen.contains("Akra / Confirm Exit"));
    let appended_lines = terminal.backend_mut().inner().appended_lines();
    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('y'),
        KeyModifiers::NONE,
    )));
    terminal.backend_mut().inner_mut().report_changing_sizes();

    assert!(runtime.take_redraw_request());
    let transaction_completed =
        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("unstable resize transaction should defer without failing");
    assert!(!transaction_completed);
    runtime.finish_pending_quit_after_transaction(transaction_completed);
    assert!(!runtime.should_quit());
    assert_resize_retry_scheduled(
        &mut runtime,
        "unstable autoresize must schedule a follow-up frame",
    );
    assert_eq!(
        inline_terminal.last_known_screen_size(),
        Some(Size::new(80, 40)),
        "unstable geometry must leave the previous screen observation intact"
    );
    assert!(!inline_terminal.back_buffer_trustworthy());
    assert_eq!(
        terminal.backend_mut().inner().appended_lines(),
        appended_lines,
        "unstable autoresize must keep resize append suppression active"
    );

    terminal
        .backend_mut()
        .inner_mut()
        .finish_reported_resize(Size::new(48, 10));
    let transaction_completed =
        draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
            .expect("scheduled resize retry should reconcile stable geometry");
    assert!(transaction_completed);
    runtime.finish_pending_quit_after_transaction(transaction_completed);
    assert!(runtime.should_quit());
    let completed_screen = tui_testkit::buffer_text(terminal.backend().inner().inner.buffer());
    assert!(!completed_screen.contains("Akra / Confirm Exit"));
    assert_eq!(
        inline_terminal.last_known_screen_size(),
        Some(Size::new(48, 10))
    );
}

fn assert_resize_retry_scheduled(runtime: &mut ShellRuntime, message: &str) {
    let now = Instant::now();
    assert!(
        runtime.next_event_poll_timeout(now, Duration::from_secs(1)) < Duration::from_secs(1),
        "{message}: frontend poll must wake for the retry"
    );
    assert!(
        runtime.take_due_draw_request(now + Duration::from_secs(1)),
        "{message}: scheduled deadline must become due"
    );
}

// Resize paths are regression-prone because committed history and live tail
// are both redrawn; this helper asserts the live prompt remains in viewport
// and never leaks into scrollback.
#[test]
fn viewport_replay_resize_does_not_push_tail_into_scrollback() {
    assert_resize_sequence_does_not_leak_live_tail(
        InlineHistoryRenderMode::ViewportReplay,
        "resize replay should stay in the active viewport",
    );
}
#[test]
fn host_scrollback_resize_does_not_push_tail_into_scrollback() {
    assert_resize_sequence_does_not_leak_live_tail(
        InlineHistoryRenderMode::HostScrollback,
        "resize host history stays committed",
    );
}
#[test]
fn draw_internal_resize_does_not_push_tail_into_scrollback() {
    assert_draw_internal_resize_does_not_leak_live_tail(
        InlineHistoryRenderMode::HostScrollback,
        "host history before draw-time resize",
    );
    assert_draw_internal_resize_does_not_leak_live_tail(
        InlineHistoryRenderMode::ViewportReplay,
        "replay history before draw-time resize",
    );
}

fn assert_resize_sequence_does_not_leak_live_tail(
    render_mode: InlineHistoryRenderMode,
    history_message: &str,
) {
    let mut terminal = tui_testkit::inline_history_terminal(render_mode, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = render_mode;
    app.shell.history_insert_mode = HistoryInsertionMode::StandardScrollRegion;
    if let ConversationState::Ready(conversation) =
        &mut app.conversation.lifecycle.conversation_state
    {
        conversation.composer.input_buffer = "live prompt must not move to scrollback".to_string();
    }
    append_history_message(&mut app, history_message);
    let mut runtime = ShellRuntime::new(app);
    let mut inline_viewport = InlineTerminalState::default();

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_viewport).unwrap());
    draw_test_frame(&mut terminal, &mut runtime, &mut inline_viewport);
    tui_testkit::resize_inline_history_terminal(&mut terminal, 80, 8);
    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_viewport).unwrap());
    draw_test_frame(&mut terminal, &mut runtime, &mut inline_viewport);
    tui_testkit::resize_inline_history_terminal(&mut terminal, 80, 24);
    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_viewport).unwrap());
    draw_test_frame(&mut terminal, &mut runtime, &mut inline_viewport);

    assert_no_live_tail_leak(&terminal, render_mode);
}
fn assert_draw_internal_resize_does_not_leak_live_tail(
    render_mode: InlineHistoryRenderMode,
    history_message: &str,
) {
    let mut terminal = tui_testkit::inline_history_terminal(render_mode, 80, 24);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = render_mode;
    app.shell.history_insert_mode = HistoryInsertionMode::StandardScrollRegion;
    if let ConversationState::Ready(conversation) =
        &mut app.conversation.lifecycle.conversation_state
    {
        conversation.composer.input_buffer = "live prompt must not move to scrollback".to_string();
    }
    append_history_message(&mut app, history_message);
    let mut runtime = ShellRuntime::new(app);
    let mut inline_viewport = InlineTerminalState::default();

    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_viewport).unwrap());
    draw_test_frame(&mut terminal, &mut runtime, &mut inline_viewport);
    tui_testkit::resize_inline_history_terminal(&mut terminal, 80, 8);
    assert!(sync_inline_viewport(&mut terminal, &mut runtime, &mut inline_viewport).unwrap());
    tui_testkit::resize_inline_history_terminal(&mut terminal, 80, 12);
    draw_test_frame(&mut terminal, &mut runtime, &mut inline_viewport);

    assert_no_live_tail_leak(&terminal, render_mode);
}
fn assert_no_live_tail_leak(
    terminal: &Terminal<InlineTerminalBackend<TestBackend>>,
    render_mode: InlineHistoryRenderMode,
) {
    let scrollback_text = tui_testkit::inline_scrollback_text(terminal);
    assert!(
        !scrollback_text.contains("live prompt must not move to scrollback"),
        "{render_mode:?} should not leak live prompt rows into scrollback after resize: {scrollback_text:?}"
    );
    assert!(
        !scrollback_text.contains("thread: new draft"),
        "{render_mode:?} should not leak live status rows into scrollback after resize: {scrollback_text:?}"
    );
    let screen_text = tui_testkit::screen_text(terminal);
    assert!(
        screen_text.contains("> live prompt must not move to scrollback"),
        "{render_mode:?} should keep the active prompt visible after shrink/restore: {screen_text:?}"
    );
    assert_eq!(
        screen_text
            .matches("> live prompt must not move to scrollback")
            .count(),
        1,
        "{render_mode:?} should not duplicate the active prompt after shrink/restore: {screen_text:?}"
    );
}
#[test]
fn hidden_inline_tail_skips_redundant_frame_draws() {
    let app = make_test_app();
    let mut inline_viewport = InlineTerminalState::default();

    assert!(inline_state_should_draw(&mut inline_viewport, &app, 80, 24));
    assert!(!inline_state_should_draw(
        &mut inline_viewport,
        &app,
        80,
        24
    ));
    assert!(inline_state_should_draw(&mut inline_viewport, &app, 96, 24));
}

#[test]
fn reused_projection_sample_keeps_parallel_frame_facts_stable_after_handle_mutation() {
    let mut app = make_test_app();
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(runtime_feed_supervisor_snapshot(
        vec![inline_runtime_feed_entry(1, "sampled event")],
    )));
    let sampled_planning =
        crate::adapter::inbound::tui::app::test_helpers::sample_planning_runtime_projection(
            "sampled planning",
            "sampled queue",
        );
    app.sync_ready_conversation_planning_runtime_projection(sampled_planning.clone());
    let sample = ConversationProjectionSample::capture(&app);
    let (
        sampled_revision,
        sampled_rendered_at,
        sampled_animation_millis,
        sampled_supervisor,
        sampled_runtime,
    ) = {
        let first = ConversationScreenModel::from_app_with_sample(&app, &sample);
        (
            first.core_revision,
            first.rendered_at,
            first.animation_elapsed_millis,
            first.parallel_mode_supervisor.clone(),
            first.planning_runtime_projection.clone(),
        )
    };

    app.set_parallel_mode_enabled_for_test(false);
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(runtime_feed_supervisor_snapshot(
        vec![inline_runtime_feed_entry(2, "new event")],
    )));
    let fresh_planning =
        crate::adapter::inbound::tui::app::test_helpers::sample_planning_runtime_projection(
            "fresh planning",
            "fresh queue",
        );
    app.sync_ready_conversation_planning_runtime_projection(fresh_planning.clone());
    {
        let reused = ConversationScreenModel::from_app_with_sample(&app, &sample);
        assert_eq!(reused.core_revision, sampled_revision);
        assert_eq!(reused.rendered_at, sampled_rendered_at);
        assert_eq!(reused.animation_elapsed_millis, sampled_animation_millis);
        assert_eq!(reused.parallel_mode_supervisor, sampled_supervisor);
        assert_eq!(reused.planning_runtime_projection, sampled_runtime);
        assert_eq!(reused.planning_runtime_projection, sampled_planning);
        assert!(reused.parallel_mode_enabled);
    }
    let sampled_frame = InlineConversationFrameProjection::from_app_with_sample(&app, 80, &sample);
    assert!(sampled_frame.parallel_mode_enabled);
    let sampled_parallel_view = sampled_frame
        .supersession_overlay_view
        .expect("sampled enabled mode must retain one owned supervisor view");
    let sampled_header = sampled_parallel_view
        .header_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let sampled_keys = sampled_parallel_view
        .key_lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(sampled_header.contains("composer available"));
    assert!(sampled_keys.contains("Ctrl+O board"));
    assert!(sampled_keys.contains(":parallel off"));

    let fresh = ConversationScreenModel::from_app(&app);
    assert!(fresh.core_revision > sampled_revision);
    assert_ne!(fresh.parallel_mode_supervisor, sampled_supervisor);
    assert_eq!(fresh.planning_runtime_projection, fresh_planning);
    assert!(!fresh.parallel_mode_enabled);
}

#[test]
fn parallel_runtime_live_event_invalidates_hidden_frame_cache() {
    let mut app = make_test_app();
    app.set_parallel_mode_enabled_for_test(true);
    app.set_parallel_mode_supervisor_snapshot_for_test(Some(runtime_feed_supervisor_snapshot(
        vec![inline_runtime_feed_entry(1, "seed runtime event one")],
    )));
    let mut inline_viewport = InlineTerminalState::default();

    assert!(inline_state_should_draw(&mut inline_viewport, &app, 80, 24));
    assert!(!inline_state_should_draw(
        &mut inline_viewport,
        &app,
        80,
        24
    ));

    app.set_parallel_mode_supervisor_snapshot_for_test(Some(runtime_feed_supervisor_snapshot(
        vec![
            inline_runtime_feed_entry(2, "new runtime event two"),
            inline_runtime_feed_entry(1, "seed runtime event one"),
        ],
    )));
    let live_events = app
        .parallel_supervisor_event_lines()
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(live_events.contains("new runtime event two"));
    assert!(inline_state_should_draw(&mut inline_viewport, &app, 80, 24));
}

#[test]
fn overlay_cycle_resets_hidden_tail_redraw_cache() {
    let mut app = make_test_app();
    let mut inline_viewport = InlineTerminalState::default();

    assert!(inline_state_should_draw(&mut inline_viewport, &app, 80, 24));
    assert!(!inline_state_should_draw(
        &mut inline_viewport,
        &app,
        80,
        24
    ));

    app.shell.chrome.shell_overlay = ShellOverlay::Startup;
    assert!(inline_state_should_draw(&mut inline_viewport, &app, 80, 24));

    app.shell.chrome.shell_overlay = ShellOverlay::Hidden;
    assert!(inline_state_should_draw(&mut inline_viewport, &app, 80, 24));
}
#[test]
fn inline_history_uses_startup_banner_while_typing_in_new_draft() {
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = true;
    if let crate::adapter::inbound::tui::app::ConversationState::Ready(conversation) =
        &mut app.conversation.lifecycle.conversation_state
    {
        conversation.composer.input_buffer = "hello banner".to_string();
    }
    let lines = current_inline_history_lines(&app)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    let rendered = lines.join("\n");

    assert!(rendered.contains(" █████╗ ██╗  ██╗██████╗  █████╗"));
    assert!(rendered.contains("╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝"));
    assert!(!rendered.contains("No messages in this thread yet."));
}

#[test]
fn inline_history_does_not_flush_startup_banner_while_parallel_home_is_active() {
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = true;
    app.set_parallel_mode_enabled_for_test(true);

    let rendered = current_inline_history_lines(&app)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(!rendered.contains(" █████╗ ██╗  ██╗██████╗  █████╗"));
    assert!(!rendered.contains("╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝"));
    assert!(!rendered.contains("No messages in this thread yet."));
    assert!(rendered.is_empty());
}
#[test]
fn inline_history_shows_planning_worker_debug_detail_when_visibility_is_debug() {
    let mut app = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.messages.push(
        ConversationMessage::new(
            ConversationMessageKind::User,
            "다음 queued-task 1개를 이어서 진행합니다.",
            None,
            None,
        )
        .with_display_label("Auto Follow-up")
        .with_debug_detail("planning worker temporary session: refresh / refresh ok"),
    );
    let normal_lines = current_inline_history_lines(&app)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!normal_lines.contains("planning worker temporary session"));

    app.planning.planning_worker_visibility = PlanningWorkerVisibility::Debug;
    let debug_lines = current_inline_history_lines(&app)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(debug_lines.contains("planning worker temporary session: refresh / refresh ok"));
}

#[test]
fn inline_history_view_mode_controls_tool_and_status_rows() {
    let mut app = make_test_app();
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "visible codex reply",
        Some("commentary".to_string()),
        None,
    ));
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Tool,
        "command: cargo test [completed]",
        None,
        None,
    ));
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Status,
        "thread status: running",
        None,
        None,
    ));

    let simple_lines = current_inline_history_lines(&app)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(simple_lines.contains("Codex Commentary:"));
    assert!(!simple_lines.contains("◆ "));
    assert!(!simple_lines.contains("tool     "));
    assert!(!simple_lines.contains("Status:"));

    app.conversation.conversation_view_mode = ConversationViewMode::Medium;
    let medium_lines = current_inline_history_lines(&app)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        medium_lines.contains("◆ ") && medium_lines.contains("cargo test"),
        "{medium_lines}"
    );
    assert!(medium_lines.contains("Status:"));
}

#[test]
fn host_scrollback_preserves_long_single_completion_beyond_screen_cap() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 100, 30);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    let body = (0..180)
        .map(|index| match index {
            0 => "LONG_SINGLE_MARKER_FIRST".to_string(),
            90 => "LONG_SINGLE_MARKER_MIDDLE".to_string(),
            179 => "LONG_SINGLE_MARKER_LAST".to_string(),
            _ => format!("long completion filler {index}"),
        })
        .collect::<Vec<_>>()
        .join("\n");
    append_history_message(&mut app, &body);
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("draw transaction");
    let terminal_history = tui_testkit::inline_terminal_history_text(&terminal);
    for marker in [
        "LONG_SINGLE_MARKER_FIRST",
        "LONG_SINGLE_MARKER_MIDDLE",
        "LONG_SINGLE_MARKER_LAST",
    ] {
        assert!(
            terminal_history.contains(marker),
            "host scrollback lost {marker} from uncapped completion:\n{terminal_history}"
        );
        assert_eq!(
            terminal_history.matches(marker).count(),
            1,
            "host scrollback duplicated {marker} from uncapped completion:\n{terminal_history}"
        );
    }
}

#[test]
fn vt100_host_scrollback_preserves_long_single_completion_beyond_screen_cap() {
    let mut terminal = tui_testkit::inline_history_vt100_terminal(
        InlineHistoryRenderMode::HostScrollback,
        100,
        30,
    );
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    let body = (0..180)
        .map(|index| match index {
            0 => "VT100_LONG_SINGLE_MARKER_FIRST".to_string(),
            90 => "VT100_LONG_SINGLE_MARKER_MIDDLE".to_string(),
            179 => "VT100_LONG_SINGLE_MARKER_LAST".to_string(),
            _ => format!("long completion filler {index}"),
        })
        .collect::<Vec<_>>()
        .join("\n");
    append_history_message(&mut app, &body);
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("draw transaction");
    let terminal_history = tui_testkit::inline_vt100_scrollback_text(&mut terminal);
    for marker in [
        "VT100_LONG_SINGLE_MARKER_FIRST",
        "VT100_LONG_SINGLE_MARKER_MIDDLE",
        "VT100_LONG_SINGLE_MARKER_LAST",
    ] {
        assert!(
            terminal_history.contains(marker),
            "VT100 host scrollback lost {marker} from uncapped completion:\n{terminal_history}"
        );
    }
}

#[test]
fn host_scrollback_preserves_multiturn_history_beyond_screen_cap_without_duplicates() {
    let mut terminal =
        tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 100, 30);
    let mut app = make_test_app();
    app.shell.show_startup_ascii_art = false;
    app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
    let mut runtime = ShellRuntime::new(app);
    let mut inline_terminal = InlineTerminalState::default();
    let markers = (0..75)
        .map(|index| format!("MULTITURN_MARKER_{index:02}"))
        .collect::<Vec<_>>();

    for (index, marker) in markers.iter().enumerate() {
        append_user_history_message(runtime.app_mut(), &format!("prompt {marker}"));
        append_history_message(runtime.app_mut(), &format!("answer {marker}"));
        if index % 5 == 0 {
            draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
                .expect("incremental draw transaction");
        }
    }

    draw_inline_transaction(&mut terminal, &mut runtime, &mut inline_terminal)
        .expect("final draw transaction");
    let terminal_history = tui_testkit::inline_terminal_history_text(&terminal);
    for marker in markers {
        assert!(
            terminal_history.contains(&marker),
            "host scrollback lost {marker} from multiturn history:\n{terminal_history}"
        );
        assert_eq!(
            terminal_history.matches(&marker).count(),
            2,
            "host scrollback should contain prompt and answer once for {marker}:\n{terminal_history}"
        );
    }
}

fn draw_test_frame<B>(
    terminal: &mut Terminal<InlineTerminalBackend<B>>,
    runtime: &mut ShellRuntime,
    inline_terminal: &mut InlineTerminalState,
) -> bool
where
    InlineTerminalBackend<B>: InlineResizeBackend,
    <InlineTerminalBackend<B> as Backend>::Error: std::fmt::Debug,
{
    let width = terminal.size().expect("terminal size").width;
    let projection = frame_projection(runtime.app(), width);
    let resize_snapshot = terminal
        .backend()
        .resize_snapshot()
        .expect("stable resize snapshot");
    draw_inline_frame(
        terminal,
        runtime,
        inline_terminal,
        projection,
        false,
        resize_snapshot,
    )
    .expect("draw test frame")
}
fn append_history_message(app: &mut NativeTuiApp, text: &str) {
    append_message(app, ConversationMessageKind::Agent, text);
}
fn append_user_history_message(app: &mut NativeTuiApp, text: &str) {
    append_message(app, ConversationMessageKind::User, text);
}
fn append_message(app: &mut NativeTuiApp, kind: ConversationMessageKind, text: &str) {
    let ConversationState::Ready(conversation) = &mut app.conversation.lifecycle.conversation_state
    else {
        panic!("test app should start in a ready conversation state");
    };
    conversation
        .messages
        .push(ConversationMessage::new(kind, text.to_string(), None, None));
}

fn load_history_snapshot(
    app: &mut NativeTuiApp,
    thread_id: &str,
    messages: Vec<ConversationMessage>,
) {
    app.dispatch_conversation_lifecycle(ConversationLifecycleEvent::SessionChosen {
        session: SessionSummary {
            id: thread_id.to_string(),
            name: Some(thread_id.to_string()),
            preview: format!("{thread_id} preview"),
            cwd: "/tmp/root".to_string(),
            source: "test".to_string(),
            model_provider: "test".to_string(),
            updated_at_epoch: 1,
            status_type: "idle".to_string(),
            path: format!("/tmp/root/{thread_id}"),
            git_branch: None,
        },
        fallback_workspace_directory: "/tmp/root".to_string(),
    });
    app.apply_core_conversation_snapshot(CoreConversationSnapshot::Ready(Box::new(
        crate::core::app::ConversationReadySnapshot::from(DomainConversationSnapshot {
            thread_id: thread_id.to_string(),
            title: thread_id.to_string(),
            cwd: "/tmp/root".to_string(),
            messages,
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
            item_lifecycle: Default::default(),
        }),
    )));
}

// Counting backend is a probe for adapter behavior: it exposes accidental
// cursor-position queries without depending on a real terminal.
struct CursorQueryCountingBackend {
    inner: TestBackend,
    cursor_query_count: usize,
    appended_lines: u16,
    scroll_region_up_count: usize,
    resize_on_flush: Option<Size>,
    size_query_count: StdCell<usize>,
    reported_size: StdCell<Option<Size>>,
    resize_on_size_query: StdCell<Option<(usize, Size)>>,
    restore_on_size_query: StdCell<Option<(usize, Size)>>,
    resize_after_scroll_region_up: Option<Size>,
    changing_sizes_start: StdCell<Option<usize>>,
}
impl CursorQueryCountingBackend {
    fn new(inner: TestBackend) -> Self {
        Self {
            inner,
            cursor_query_count: 0,
            appended_lines: 0,
            scroll_region_up_count: 0,
            resize_on_flush: None,
            size_query_count: StdCell::new(0),
            reported_size: StdCell::new(None),
            resize_on_size_query: StdCell::new(None),
            restore_on_size_query: StdCell::new(None),
            resize_after_scroll_region_up: None,
            changing_sizes_start: StdCell::new(None),
        }
    }
    fn cursor_query_count(&self) -> usize {
        self.cursor_query_count
    }
    fn appended_lines(&self) -> u16 {
        self.appended_lines
    }
    fn scroll_region_up_count(&self) -> usize {
        self.scroll_region_up_count
    }
    fn resize_and_clamp_cursor(&mut self, width: u16, height: u16) {
        let cursor = self
            .inner
            .get_cursor_position()
            .expect("fixture cursor should be readable before resize");
        self.inner.resize(width, height);
        self.inner
            .set_cursor_position(Position::new(
                cursor.x.min(width.saturating_sub(1)),
                cursor.y.min(height.saturating_sub(1)),
            ))
            .expect("fixture cursor clamp should succeed");
    }
    fn resize_on_next_flush(&mut self, width: u16, height: u16) {
        self.resize_on_flush = Some(Size::new(width, height));
    }
    fn report_resize_after_size_queries(&mut self, query_count: usize, width: u16, height: u16) {
        self.resize_on_size_query.set(Some((
            self.size_query_count.get() + query_count,
            Size::new(width, height),
        )));
    }
    fn report_resize_then_restore(
        &mut self,
        resize_query_count: usize,
        resize_size: Size,
        restore_query_count: usize,
        restore_size: Size,
    ) {
        let current_query_count = self.size_query_count.get();
        self.resize_on_size_query.set(Some((
            current_query_count + resize_query_count,
            resize_size,
        )));
        self.restore_on_size_query.set(Some((
            current_query_count + restore_query_count,
            restore_size,
        )));
    }
    fn resize_after_next_scroll_region_up(&mut self, width: u16, height: u16) {
        self.resize_after_scroll_region_up = Some(Size::new(width, height));
    }
    fn current_reported_size(&self) -> Size {
        self.reported_size
            .get()
            .unwrap_or_else(|| self.inner.size().expect("fixture size should be readable"))
    }
    fn report_changing_sizes(&mut self) {
        self.changing_sizes_start
            .set(Some(self.size_query_count.get() + 1));
    }
    fn finish_reported_resize(&mut self, size: Size) {
        self.changing_sizes_start.set(None);
        self.reported_size.set(Some(size));
    }
}
impl Backend for CursorQueryCountingBackend {
    type Error = Infallible;
    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        self.inner.draw(content)
    }
    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.hide_cursor()
    }
    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.show_cursor()
    }
    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        self.cursor_query_count += 1;
        let cursor = self.inner.get_cursor_position()?;
        let size = self.current_reported_size();
        Ok(Position::new(
            cursor.x.min(size.width.saturating_sub(1)),
            cursor.y.min(size.height.saturating_sub(1)),
        ))
    }
    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        self.inner.set_cursor_position(position)
    }
    fn clear(&mut self) -> Result<(), Self::Error> {
        self.inner.clear()
    }
    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        self.inner.clear_region(clear_type)
    }
    fn append_lines(&mut self, line_count: u16) -> Result<(), Self::Error> {
        self.appended_lines = self.appended_lines.saturating_add(line_count);
        self.inner.append_lines(line_count)
    }
    fn size(&self) -> Result<Size, Self::Error> {
        let query_count = self.size_query_count.get() + 1;
        self.size_query_count.set(query_count);
        if let Some(first_query) = self.changing_sizes_start.get() {
            let offset = u16::try_from(query_count - first_query).unwrap_or(u16::MAX);
            let size = Size::new(80_u16.saturating_sub(offset).max(1), 40);
            self.reported_size.set(Some(size));
            return Ok(size);
        }
        if let Some((trigger_query, size)) = self.resize_on_size_query.get()
            && query_count >= trigger_query
        {
            self.reported_size.set(Some(size));
            self.resize_on_size_query.set(None);
        }
        if let Some((trigger_query, size)) = self.restore_on_size_query.get()
            && query_count >= trigger_query
        {
            self.reported_size.set(Some(size));
            self.restore_on_size_query.set(None);
        }
        Ok(self.current_reported_size())
    }
    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        self.inner.window_size()
    }
    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush()?;
        if let Some(size) = self.resize_on_flush.take() {
            self.resize_and_clamp_cursor(size.width, size.height);
        }
        Ok(())
    }
    fn scroll_region_up(&mut self, region: Range<u16>, line_count: u16) -> Result<(), Self::Error> {
        self.scroll_region_up_count = self.scroll_region_up_count.saturating_add(1);
        self.inner.scroll_region_up(region, line_count)?;
        if let Some(size) = self.resize_after_scroll_region_up.take() {
            self.resize_and_clamp_cursor(size.width, size.height);
        }
        Ok(())
    }
    fn scroll_region_down(
        &mut self,
        region: Range<u16>,
        line_count: u16,
    ) -> Result<(), Self::Error> {
        self.inner.scroll_region_down(region, line_count)
    }
}
