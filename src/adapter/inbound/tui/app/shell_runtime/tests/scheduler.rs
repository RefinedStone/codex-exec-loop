// Scheduler tests inject time directly so redraw deadlines stay deterministic.
// That keeps event-loop regressions visible without sleeping in the test suite.
use std::time::{Duration, Instant};

use crossterm::event::Event;

use super::{
    BACKGROUND_MESSAGE_DRAIN_BUDGET, BackgroundMessage, ConversationState, ShellOverlay,
    TERMINAL_RESIZE_RETRY_DELAY, TuiFrameScheduler, make_test_runtime,
};
use crate::adapter::inbound::tui::app::shell_presentation::{
    ConversationProjectionSample, ConversationScreenModel,
};
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterEntry, ParallelModeAgentRosterSnapshot, ParallelModeDistributorSnapshot,
    ParallelModePoolBoardSnapshot, ParallelModePoolSlotSnapshot, ParallelModePoolSlotState,
    ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorSnapshot,
    ParallelModeSupervisorState,
};

// A new runtime must request the first frame immediately; otherwise the TUI can sit blank until input or background work.
#[test]
fn runtime_starts_with_redraw_requested() {
    let mut runtime = make_test_runtime();

    assert!(runtime.take_redraw_request());
    // Draw requests are one-shot so startup paint cannot trigger duplicate terminal frames.
    assert!(!runtime.take_redraw_request());
}

// Redraw producers share one deadline; the earliest request must win so input, resize, and live pulses stay responsive.
#[test]
fn scheduler_coalesces_immediate_and_delayed_requests() {
    let now = Instant::now();
    let mut scheduler = TuiFrameScheduler {
        focused: true,
        next_deadline: None,
    };

    scheduler.request_delayed(now, Duration::from_secs(10));
    scheduler.request_delayed(now, Duration::from_secs(5));
    assert_eq!(
        scheduler.next_poll_timeout(now, Duration::from_secs(30)),
        Duration::from_secs(5)
    );

    // Immediate redraw preempts the delayed queue and shortens the frontend poll timeout.
    scheduler.request_immediate(now + Duration::from_secs(1));
    assert_eq!(
        scheduler.next_poll_timeout(now, Duration::from_secs(30)),
        Duration::from_secs(1)
    );
    assert!(!scheduler.take_due(now));
    assert!(scheduler.take_due(now + Duration::from_secs(1)));
    // Consuming the due deadline clears it, preserving the scheduler's one-frame-per-request contract.
    assert!(!scheduler.take_due(now + Duration::from_secs(1)));
}

// A due deadline must collapse the event poll timeout to zero so the frontend draws before waiting for input.
#[test]
fn scheduler_reports_zero_timeout_when_draw_is_due() {
    let now = Instant::now();
    let scheduler = TuiFrameScheduler::new(now);

    assert_eq!(
        scheduler.next_poll_timeout(now, Duration::from_millis(100)),
        Duration::ZERO
    );
}

#[test]
fn resize_retry_is_delayed_coalesced_and_preserved_while_unfocused() {
    let mut runtime = make_test_runtime();
    runtime.take_redraw_request();
    let now = Instant::now();

    runtime.request_resize_redraw_retry_at(now);
    runtime.request_resize_redraw_retry_at(now + Duration::from_millis(1));

    assert_eq!(
        runtime.next_event_poll_timeout(now, Duration::from_secs(1)),
        TERMINAL_RESIZE_RETRY_DELAY
    );
    assert!(!runtime.take_due_draw_request(now + Duration::from_millis(15)));
    assert!(runtime.take_due_draw_request(now + TERMINAL_RESIZE_RETRY_DELAY));

    runtime.request_resize_redraw_retry_at(now + TERMINAL_RESIZE_RETRY_DELAY);
    let pending_deadline = runtime.frame_scheduler.next_deadline;
    runtime.handle_terminal_event_at(Event::FocusLost, now + Duration::from_millis(17));
    assert!(!runtime.take_due_draw_request(now + Duration::from_millis(32)));
    assert_eq!(runtime.frame_scheduler.next_deadline, pending_deadline);
    assert_eq!(
        runtime.next_event_poll_timeout(now + Duration::from_millis(32), Duration::from_secs(1)),
        Duration::from_secs(1)
    );

    runtime.handle_terminal_event_at(Event::FocusGained, now + Duration::from_millis(33));
    assert!(runtime.take_due_draw_request(now + Duration::from_millis(33)));
}

#[test]
fn background_message_burst_yields_after_budget() {
    let mut runtime = make_test_runtime();
    runtime.take_redraw_request();
    let queued_message_count = BACKGROUND_MESSAGE_DRAIN_BUDGET + 3;
    for index in 0..queued_message_count {
        runtime
            .app
            .runtime
            .tx
            .send(BackgroundMessage::ConversationRuntimeNotice(format!(
                "notice {index}"
            )))
            .expect("background message should enqueue");
    }
    let now = Instant::now();

    runtime.poll_background_messages_at(now);

    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("expected ready conversation state");
    };
    assert_eq!(
        conversation.runtime_notices.len(),
        BACKGROUND_MESSAGE_DRAIN_BUDGET
    );
    assert_eq!(
        runtime.next_event_poll_timeout(now, Duration::from_millis(100)),
        Duration::ZERO
    );

    runtime.poll_background_messages_at(now + Duration::from_millis(1));

    let ConversationState::Ready(conversation) =
        &runtime.app().conversation.lifecycle.conversation_state
    else {
        panic!("expected ready conversation state");
    };
    assert_eq!(conversation.runtime_notices.len(), queued_message_count);
}

// Focus loss gates drawing but keeps pending layout work; focus return must redraw immediately to resync the terminal.
#[test]
fn focus_lost_blocks_draw_until_focus_returns() {
    // Clear startup paint so the focus transition is the only source of draw pressure in this scenario.
    let mut runtime = make_test_runtime();
    runtime.take_redraw_request();
    let now = Instant::now();

    runtime.handle_terminal_event_at(Event::FocusLost, now);
    runtime.handle_terminal_event_at(Event::Resize(120, 40), now + Duration::from_millis(1));

    // Resize while unfocused should not burn CPU on frames the user cannot see.
    assert!(!runtime.take_due_draw_request(now + Duration::from_millis(1)));

    runtime.handle_terminal_event_at(Event::FocusGained, now + Duration::from_millis(2));

    // Regaining focus schedules a fresh frame at the same timestamp to repair any stale layout.
    assert!(runtime.take_due_draw_request(now + Duration::from_millis(2)));
    assert_eq!(runtime.terminal_focus_reacquire_epoch(), 1);

    runtime.handle_terminal_event_at(Event::FocusGained, now + Duration::from_millis(3));

    // Duplicate focus notifications are not a new visibility transition.
    assert!(!runtime.take_due_draw_request(now + Duration::from_millis(3)));
    assert_eq!(runtime.terminal_focus_reacquire_epoch(), 1);
}

// Supersession can display active worker state without any incoming stream event.
// The runtime must periodically refresh the supervisor projection so stale in-memory
// rows are reconciled against the store and git state.
#[test]
fn active_supersession_supervisor_refreshes_periodically() {
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    runtime.app_mut().shell.chrome.shell_overlay = ShellOverlay::Supersession;
    runtime.app_mut().set_parallel_mode_enabled_for_test(true);
    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory,
            ParallelModePoolBoardSnapshot::new(
                3,
                "/tmp/pool",
                "running",
                vec![ParallelModePoolSlotSnapshot::new(
                    "slot-1",
                    ParallelModePoolSlotState::Running,
                    "akra-agent/slot-1/task-one",
                    "slot-1",
                    "agent-1",
                )],
            ),
            ParallelModeAgentRosterSnapshot::new(
                vec![ParallelModeAgentRosterEntry::new(
                    "agent-1",
                    "Task One",
                    "slot-1",
                    "akra-agent/slot-1/task-one",
                    "running",
                    "12s",
                    "working",
                )],
                "no active agents",
            ),
            ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
            None,
        )));
    let now = Instant::now();

    assert!(
        runtime
            .app()
            .parallel_mode_supervisor_refresh_due_for_test(now)
    );
    runtime.poll_background_messages_at(now);
    assert!(
        runtime.app().parallel_mode_control_effect_in_flight(),
        "the passive pending-dispatch poll should remain correlated until its event is drained"
    );
    assert!(
        !runtime
            .app()
            .parallel_mode_supervisor_refresh_due_for_test(now + Duration::from_millis(999))
    );
    // A passive authority poll may serialize the actual refresh, but it must not
    // suppress the periodic refresh request that queues the newer presentation work.
    assert!(
        runtime
            .app()
            .parallel_mode_supervisor_refresh_due_for_test(now + Duration::from_secs(1))
    );
}

#[test]
fn blocked_supersession_pool_refreshes_periodically() {
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    runtime.app_mut().shell.chrome.shell_overlay = ShellOverlay::Supersession;
    runtime.app_mut().set_parallel_mode_enabled_for_test(true);
    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory,
            ParallelModePoolBoardSnapshot::new(
                3,
                "/tmp/pool",
                "reconcile blocked / blocked: 1",
                vec![ParallelModePoolSlotSnapshot::new(
                    "slot-1",
                    ParallelModePoolSlotState::Blocked,
                    "akra-agent/slot-1/task-one",
                    "slot-1 / blocked",
                    "agent-1",
                )],
            ),
            ParallelModeAgentRosterSnapshot::new(
                vec![ParallelModeAgentRosterEntry::new(
                    "agent-1",
                    "Task One",
                    "slot-1",
                    "akra-agent/slot-1/task-one",
                    "running",
                    "12s",
                    "working",
                )],
                "no active agents",
            ),
            ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
            None,
        )));
    let now = Instant::now();

    assert!(
        runtime
            .app()
            .parallel_mode_supervisor_refresh_due_for_test(now)
    );
    assert!(runtime.app().live_activity_pulse(now).is_some());
}

#[test]
fn sampled_parallel_panel_state_aligns_prompt_pulse_and_tick_refresh() {
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    runtime.app_mut().shell.chrome.shell_overlay = ShellOverlay::Supersession;
    runtime.app_mut().set_parallel_mode_enabled_for_test(true);
    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory.clone(),
            ParallelModePoolBoardSnapshot::new(
                0,
                "loading: supervisor refresh",
                "loading",
                Vec::new(),
            ),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "loading"),
            ParallelModeSupervisorDetailSnapshot::new(None, "loading"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "loading", "loading"),
            Some("loading 3/3: board refresh".to_string()),
        )));
    let sample = ConversationProjectionSample::capture(runtime.app());
    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory,
            ParallelModePoolBoardSnapshot::new(0, "/tmp/pool", "idle", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
            ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
            None,
        )));
    let screen = ConversationScreenModel::from_app_with_sample(runtime.app(), &sample);

    assert!(screen.parallel_mode_loading_prompt_indicator_visible);
    assert!(
        runtime
            .app()
            .parallel_mode_prompt_input_locked_with_sample(sample.parallel_panel_sample())
    );
    assert!(
        runtime
            .app()
            .parallel_mode_activity_pulse_visible_with_sample(sample.parallel_panel_sample())
    );
    let now = Instant::now();
    assert_eq!(
        runtime
            .app()
            .live_activity_pulse_with_sample(now, sample.parallel_panel_sample()),
        Some(0)
    );
    assert!(
        runtime
            .app()
            .parallel_mode_supervisor_refresh_due_with_sample_for_test(
                now,
                sample.parallel_panel_sample(),
            )
    );
    assert!(
        !runtime.app().parallel_mode_prompt_input_locked(),
        "a fresh lightweight sample should observe the non-loading replacement"
    );
    runtime
        .app_mut()
        .tick_parallel_mode_control_plane(now, sample.parallel_panel_sample());
    assert!(
        !runtime
            .app()
            .parallel_mode_supervisor_refresh_due_with_sample_for_test(
                now,
                sample.parallel_panel_sample(),
            ),
        "the tick must consume the same active panel sample and record its supervisor refresh"
    );
}

#[test]
fn in_flight_supersession_supervisor_refresh_blocks_periodic_overlap() {
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    runtime.app_mut().shell.chrome.shell_overlay = ShellOverlay::Supersession;
    runtime.app_mut().set_parallel_mode_enabled_for_test(true);
    runtime
        .app_mut()
        .mark_parallel_mode_supervisor_refresh_in_flight_for_test();
    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory,
            ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "running", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(
                vec![ParallelModeAgentRosterEntry::new(
                    "agent-1",
                    "Task One",
                    "slot-1",
                    "akra-agent/slot-1/task-one",
                    "running",
                    "12s",
                    "working",
                )],
                "no active agents",
            ),
            ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
            None,
        )));
    let now = Instant::now();

    assert!(
        !runtime
            .app()
            .parallel_mode_supervisor_refresh_due_for_test(now)
    );
    assert!(
        !runtime
            .app()
            .parallel_mode_supervisor_refresh_due_for_test(now + Duration::from_secs(5))
    );
}

#[test]
fn empty_non_loading_supersession_snapshot_does_not_refresh_periodically() {
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    runtime.app_mut().shell.chrome.shell_overlay = ShellOverlay::Supersession;
    runtime.app_mut().set_parallel_mode_enabled_for_test(true);
    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory,
            ParallelModePoolBoardSnapshot::new(0, "idle", "idle", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
            ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
            None,
        )));
    let now = Instant::now();

    assert!(
        !runtime
            .app()
            .parallel_mode_supervisor_refresh_due_for_test(now)
    );
}
