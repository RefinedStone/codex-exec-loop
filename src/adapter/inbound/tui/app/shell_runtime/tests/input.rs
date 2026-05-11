use super::{
    BackgroundMessage, ConversationState, InlineShellCommand, ShellOverlay, StartupState,
    make_dispatch_ready_parallel_runtime, make_test_runtime, sample_startup_diagnostics,
};
use crate::adapter::inbound::tui::app::conversation_runtime::{
    PostTurnContinuationAction, PostTurnEvaluationOutcome, PostTurnEvaluationProvenance,
    PostTurnQueuedPrompt,
};
use crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneBackgroundEvent;
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterEntry, ParallelModeAgentRosterSnapshot, ParallelModeAutomationTrigger,
    ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
    ParallelModeDistributorSnapshot, ParallelModePoolBoardSnapshot, ParallelModePoolSlotSnapshot,
    ParallelModePoolSlotState, ParallelModeReadinessSnapshot, ParallelModeReadinessState,
    ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorSnapshot,
    ParallelModeSupervisorState,
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;
use std::time::Instant;

/*
이 테스트 모듈은 production terminal event loop의 key routing contract를 고정한다.
`ratatui_frontend`는 crossterm `Event`를 그대로 `ShellRuntime::handle_terminal_event`에 넘기고,
runtime은 overlay, inline command palette, conversation input reducer, startup submit guard로
분기한다. 작은 modifier 차이 하나가 prompt text, shell command, refresh shortcut, submit flow 사이를
바꿀 수 있으므로 이 파일은 "어느 surface가 키를 소비하는가"를 직접 검증한다.
*/

fn ready_parallel_mode_readiness_snapshot(
    workspace_directory: &str,
) -> ParallelModeReadinessSnapshot {
    ParallelModeReadinessSnapshot::new(
        workspace_directory,
        ParallelModeReadinessState::Ready,
        vec![ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::Planning,
            ParallelModeCapabilityState::Ready,
            "planning workspace is healthy",
            None,
        )],
        None,
    )
}

#[test]
fn plain_character_input_uses_empty_modifier_check() {
    /*
     * plain character는 modifier가 완전히 비어 있을 때만 prompt buffer로 들어가야 한다.
     * Ctrl/Alt 조합이 일반 입력으로 누수되면 shortcut과 prompt text가 동시에 반응하므로,
     * 이 테스트가 character input route의 기준선을 잡는다.
     */
    let mut runtime = make_test_runtime();
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::empty(),
    )));
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(conversation.input_buffer, "a");
    assert!(runtime.take_redraw_request());
}

#[test]
fn supersession_overlay_blocks_prompt_input_while_loading() {
    /*
     * Supersession overlay는 loading 중에만 일반 prompt 입력을 막는다. 이 시점에는
     * pool reset/reconcile이 진행 중이라 새 prompt 작성과 섞이면 상태를 읽기 어렵다.
     */
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
    runtime.app_mut().set_parallel_mode_enabled_for_test(true);
    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory,
            ParallelModePoolBoardSnapshot::new(0, "loading: pool", "loading", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "loading agent roster"),
            ParallelModeSupervisorDetailSnapshot::new(None, "loading detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "loading", "loading"),
            Some("loading 2/3: pool reconcile".to_string()),
        )));
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::empty(),
    )));
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert!(conversation.input_buffer.is_empty());
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Supersession);
    assert!(runtime.take_redraw_request());
}

#[test]
fn supersession_overlay_allows_prompt_input_after_loading_finishes() {
    /*
     * Loading이 끝나 concrete supervisor snapshot이 들어오면 Supersession board를 열어 둔 채로도
     * prompt editing은 다시 가능해야 한다. Ctrl+R/Ctrl+P 같은 board shortcut만 overlay가 계속 소유한다.
     */
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
    runtime.app_mut().set_parallel_mode_enabled_for_test(true);
    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory,
            ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
            ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
            None,
        )));
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::empty(),
    )));
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(conversation.input_buffer, "a");
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Supersession);
    assert!(runtime.take_redraw_request());
}

#[test]
fn supersession_overlay_allows_space_and_enter_prompt_submit_after_loading_finishes() {
    /*
     * Supersession MUD navigation must not steal ordinary composer keys once the
     * supervisor board is concrete. The footer still advertises Enter send, so a
     * ready board has to let Space edit the prompt and Enter start the turn.
     */
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    runtime.app_mut().startup_state = StartupState::Ready(sample_startup_diagnostics(
        &runtime.app().current_workspace_directory(),
    ));
    runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
    runtime.app_mut().set_parallel_mode_enabled_for_test(true);
    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory,
            ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
            ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
            None,
        )));
    for character in "run".chars() {
        runtime.app_mut().push_input_character(character);
    }
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char(' '),
        KeyModifiers::empty(),
    )));
    for character in "next".chars() {
        runtime.app_mut().push_input_character(character);
    }
    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::empty(),
    )));
    for _ in 0..250 {
        runtime.poll_background_messages();
        let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
            panic!("expected ready conversation state");
        };
        if conversation.input_buffer.is_empty() && conversation.status_text == "starting turn" {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert!(conversation.input_buffer.is_empty());
    assert_eq!(conversation.status_text, "starting turn");
    assert!(
        conversation
            .messages
            .iter()
            .any(|message| message.text == "run next"),
        "Enter should submit the buffered prompt into the transcript"
    );
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Supersession);
    assert!(runtime.take_redraw_request());
}

#[test]
fn supersession_mud_navigation_changes_only_ui_selection_state() {
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    let snapshot = ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        workspace_directory,
        ParallelModePoolBoardSnapshot::new(
            2,
            "/tmp/pool",
            "idle",
            vec![
                ParallelModePoolSlotSnapshot::new(
                    "slot-1",
                    ParallelModePoolSlotState::Idle,
                    "prerelease",
                    "akra-pool/slot-1",
                    "idle",
                ),
                ParallelModePoolSlotSnapshot::new(
                    "slot-2",
                    ParallelModePoolSlotState::Running,
                    "akra-agent/slot-2/mud",
                    "akra-pool/slot-2",
                    "agent-2 / task-2",
                ),
            ],
        ),
        ParallelModeAgentRosterSnapshot::new(
            vec![ParallelModeAgentRosterEntry::new(
                "agent-2",
                "MUD navigation",
                "slot-2",
                "akra-agent/slot-2/mud",
                "running",
                "01m00s",
                "working",
            )],
            "no active agents",
        ),
        ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
        None,
    );
    runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
    runtime.app_mut().set_parallel_mode_enabled_for_test(true);
    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(snapshot.clone()));
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Down,
        KeyModifiers::empty(),
    )));
    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::empty(),
    )));
    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Tab,
        KeyModifiers::empty(),
    )));

    assert_eq!(
        runtime.app().parallel_mode_supervisor_snapshot(),
        snapshot,
        "MUD navigation must not mutate supervisor/domain state"
    );
    assert!(
        !runtime.app().parallel_mode_supervisor_refresh_in_flight(),
        "MUD navigation must not request an application supervisor refresh"
    );
    assert_eq!(
        runtime
            .app()
            .supersession_mud_ui_state
            .selected_room_index(),
        1
    );
    assert_eq!(
        runtime
            .app()
            .supersession_mud_ui_state
            .selected_actor_index(),
        0
    );
    assert!(runtime.take_redraw_request());
}

#[test]
fn parallel_projection_refresh_preserves_supersession_overlay_focus_and_selection() {
    /*
     * Supervisor refresh is an application projection update. It may replace the
     * core board projection, but it must not close the active overlay or reset MUD
     * navigation selection, which are UI-only state.
     */
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    let build_snapshot = |pool_status: &str| {
        ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory.clone(),
            ParallelModePoolBoardSnapshot::new(
                2,
                "/tmp/pool",
                pool_status,
                vec![
                    ParallelModePoolSlotSnapshot::new(
                        "slot-1",
                        ParallelModePoolSlotState::Idle,
                        "prerelease",
                        "slot-1",
                        "idle",
                    ),
                    ParallelModePoolSlotSnapshot::new(
                        "slot-2",
                        ParallelModePoolSlotState::Running,
                        "akra-agent/slot-2/mud",
                        "slot-2",
                        "agent-2",
                    ),
                ],
            ),
            ParallelModeAgentRosterSnapshot::new(
                vec![ParallelModeAgentRosterEntry::new(
                    "agent-2",
                    "MUD navigation",
                    "slot-2",
                    "akra-agent/slot-2/mud",
                    "running",
                    "01m00s",
                    "working",
                )],
                "no active agents",
            ),
            ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
            None,
        )
    };
    runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
    runtime.app_mut().set_parallel_mode_enabled_for_test(true);
    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(build_snapshot("running")));
    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Down,
        KeyModifiers::empty(),
    )));

    let refreshed_snapshot = build_snapshot("running / refreshed");
    let (epoch_id, effect_id) = runtime
        .app_mut()
        .mark_parallel_mode_supervisor_refresh_in_flight_for_test();
    runtime
        .app
        .tx
        .send(BackgroundMessage::ParallelModeControlPlaneEvent(
            ParallelModeControlPlaneBackgroundEvent::SupervisorSnapshotRefreshed {
                workspace_directory,
                epoch_id,
                effect_id,
                supervisor_snapshot: Box::new(refreshed_snapshot.clone()),
                orchestrator_tick_signature: None,
            },
        ))
        .expect("supervisor refresh should enqueue");
    runtime.poll_background_messages();

    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Supersession);
    assert_eq!(
        runtime
            .app()
            .supersession_mud_ui_state
            .selected_room_index(),
        1
    );
    assert_eq!(
        runtime.app().parallel_mode_supervisor_snapshot(),
        refreshed_snapshot.clone()
    );
    assert_eq!(
        runtime
            .app()
            .core_runtime
            .snapshot()
            .planning_parallel
            .parallel_mode
            .supervisor
            .as_deref(),
        Some(&refreshed_snapshot)
    );
}

#[test]
fn parallel_task_update_after_enable_reuses_existing_epoch_for_dispatch() {
    /*
     * Enable is the single application command for :parallel entry and opens the
     * automation epoch before async readiness/pool work returns. Later task
     * updates reuse that epoch instead of creating a second entry path.
     */
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
    runtime.app_mut().set_parallel_mode_enabled_for_test(true);
    runtime
        .app_mut()
        .set_parallel_mode_readiness_snapshot_for_test(Some(
            ready_parallel_mode_readiness_snapshot(&workspace_directory),
        ));
    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory,
            ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
            ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
            None,
        )));

    runtime
        .app_mut()
        .refresh_parallel_mode_dispatch_after_task_update("task-added");

    assert!(
        runtime.app().parallel_mode_automation_epoch_id().is_some(),
        "enabled parallel mode should already own the automation epoch"
    );
    assert_eq!(
        runtime.app().last_parallel_mode_automation_trigger(),
        Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch)
    );
    assert!(
        runtime
            .app()
            .last_parallel_mode_dispatch_withheld_reason()
            .is_none()
    );
}

#[test]
fn parallel_epoch_id_ignores_stale_workspace_epoch() {
    /*
     * 세션 전환으로 shell workspace가 바뀌면 이전 workspace의 epoch를 현재 workspace의
     * 자동화 epoch로 노출하지 않아야 한다. stale epoch가 보이면 후속 dispatch/disable이
     * 잘못된 workspace 상태를 정상으로 오인할 수 있다.
     */
    let mut runtime = make_test_runtime();
    runtime.app_mut().startup_state =
        StartupState::Ready(sample_startup_diagnostics("/tmp/parallel-workspace-a"));
    runtime
        .app_mut()
        .sync_draft_shell_workspace("/tmp/parallel-workspace-a");
    runtime
        .app_mut()
        .set_parallel_mode_automation_epoch_for_test(7);

    runtime.app_mut().startup_state =
        StartupState::Ready(sample_startup_diagnostics("/tmp/parallel-workspace-b"));
    runtime
        .app_mut()
        .sync_draft_shell_workspace("/tmp/parallel-workspace-b");

    assert_eq!(runtime.app().parallel_mode_automation_epoch_id(), None);
}

#[test]
fn bare_parallel_enter_dispatches_ready_queue() {
    /*
     * :parallel entry opens the automation epoch after readiness and pool
     * reconcile, then dispatches an already-ready accepted queue head.
     */
    let fixture = make_dispatch_ready_parallel_runtime("parallel-enter-dispatch");
    let mut runtime = fixture.runtime;
    for character in ":parallel".chars() {
        runtime.app_mut().push_input_character(character);
    }
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));

    let mut final_status = String::new();
    for _ in 0..250 {
        runtime.poll_background_messages();
        if let ConversationState::Ready(conversation) = &runtime.app().conversation_state {
            final_status = conversation.status_text.clone();
            if final_status.contains("auto dispatched 1 worker(s)")
                || final_status.contains("blocked /")
            {
                break;
            }
        }
        thread::sleep(Duration::from_millis(20));
    }

    assert!(
        final_status.contains("auto dispatched 1 worker(s)"),
        "parallel entry should dispatch ready queue, got `{final_status}`"
    );
    assert_eq!(
        fixture.launch_count.load(Ordering::SeqCst),
        1,
        "bare :parallel entry should launch one isolated worker"
    );
}

#[test]
fn post_turn_auto_prompt_opens_parallel_epoch_and_dispatches_workers() {
    /*
     * The main-session post-turn policy is the first legal parallel automation
     * start point. When it returns a queue auto prompt, the TUI suppresses the
     * single-session auto-follow prompt and dispatches through the parallel pool.
     */
    let fixture = make_dispatch_ready_parallel_runtime("post-turn-parallel-dispatch");
    let mut runtime = fixture.runtime;
    let workspace_directory = runtime.app().current_workspace_directory();
    runtime.app_mut().set_parallel_mode_enabled_for_test(true);
    runtime
        .app_mut()
        .set_parallel_mode_readiness_snapshot_for_test(Some(
            ready_parallel_mode_readiness_snapshot(&workspace_directory),
        ));
    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory.clone(),
            ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
            ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
            None,
        )));
    let planning_projection = runtime
        .app()
        .application
        .planning()
        .runtime()
        .load_runtime_projection_or_invalid(&workspace_directory);
    let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state else {
        panic!("expected ready conversation state");
    };
    conversation.thread_id = "thread-1".to_string();
    conversation.turn_activity.last_completed_turn_id = Some("turn-1".to_string());

    runtime
        .app
        .tx
        .send(BackgroundMessage::PostTurnEvaluationCompleted {
            thread_id: "thread-1".to_string(),
            completed_turn_id: "turn-1".to_string(),
            evaluation: Box::new(PostTurnEvaluationOutcome {
                provenance: PostTurnEvaluationProvenance::new("turn-1".to_string()),
                runtime_projection: planning_projection,
                planning_repair_state: None,
                runtime_notices: Vec::new(),
                action: PostTurnContinuationAction::QueueAutoPrompt(Box::new(
                    PostTurnQueuedPrompt {
                        prompt: "run next task".to_string(),
                        mode_label: "test".to_string(),
                        transcript_text: "next-task".to_string(),
                    },
                )),
                operator_alerts: Vec::new(),
            }),
            planning_worker_panel_state: Default::default(),
        })
        .expect("background message should enqueue");

    for _ in 0..750 {
        runtime.poll_background_messages();
        if fixture.launch_count.load(Ordering::SeqCst) > 0 {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    assert!(runtime.app().parallel_mode_automation_epoch_id().is_some());
    assert_eq!(
        runtime.app().last_parallel_mode_automation_trigger(),
        Some(ParallelModeAutomationTrigger::MainTurnPostEvaluation)
    );
    assert_eq!(fixture.launch_count.load(Ordering::SeqCst), 1);
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert!(
        !conversation
            .status_text
            .contains("queued auto-follow with mode test"),
        "parallel mode should suppress the main-session auto-follow submit"
    );
    assert!(
        !conversation.auto_follow_state.has_live_activity(),
        "parallel dispatch conversion must not leave a queued auto turn that can never finish"
    );
    assert_eq!(
        conversation
            .last_auto_follow_activity
            .as_ref()
            .map(|activity| activity.summary.as_str()),
        Some("delegated: parallel dispatch")
    );
}

#[test]
fn supersession_uses_planning_workspace_projection_after_loading_finishes() {
    /*
     * Startup shell workspace와 active draft/thread planning workspace가 다를 수 있다.
     * Supersession은 parallel worker가 사용한 planning workspace snapshot을 기준으로 렌더링해야 하며,
     * shell workspace 불일치 때문에 loading placeholder를 다시 합성하면 안 된다.
     */
    let mut runtime = make_test_runtime();
    let planning_workspace = "/tmp/planning-workspace".to_string();
    runtime.app_mut().startup_state =
        StartupState::Ready(sample_startup_diagnostics("/tmp/startup-workspace"));
    let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state else {
        panic!("expected ready conversation state");
    };
    conversation.draft_workspace_directory = planning_workspace.clone();
    conversation.cwd = planning_workspace.clone();
    runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
    runtime.app_mut().set_parallel_mode_enabled_for_test(true);
    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            planning_workspace.clone(),
            ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
            ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
            None,
        )));

    let snapshot = runtime.app().parallel_mode_supervisor_snapshot();

    assert_eq!(
        runtime.app().current_workspace_directory(),
        "/tmp/startup-workspace"
    );
    assert_eq!(
        runtime.app().planning_workspace_directory(),
        planning_workspace
    );
    assert_eq!(snapshot.pool.pool_root_label, "/tmp/pool");
    assert!(!snapshot.pool.pool_root_label.starts_with("loading:"));
    assert!(snapshot.top_notice.is_none());
}

#[test]
fn supervisor_invalidation_keeps_core_board_visible() {
    /*
     * Worker updates invalidate supervisor data after dispatch. The visible board
     * must not fall back to the loading placeholder while the replacement snapshot
     * is being refreshed in the background.
     */
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    runtime.app_mut().set_parallel_mode_enabled_for_test(true);
    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory,
            ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
            ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
            None,
        )));

    runtime
        .app_mut()
        .invalidate_parallel_mode_supervisor_snapshot();

    assert_eq!(
        runtime
            .app()
            .parallel_mode_supervisor_snapshot()
            .pool
            .configured_size,
        3
    );
}

#[test]
fn parallel_projection_accessors_read_core_snapshot() {
    /*
     * Parallel rendering accessors read the core AppSnapshot projection directly.
     * Test setup uses the same core input path that application events use.
     */
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    runtime.app_mut().set_parallel_mode_enabled_for_test(true);
    runtime
        .app_mut()
        .set_parallel_mode_readiness_snapshot_for_test(Some(
            ready_parallel_mode_readiness_snapshot(&workspace_directory),
        ));
    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory,
            ParallelModePoolBoardSnapshot::new(4, "/tmp/core-pool", "core", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "core roster"),
            ParallelModeSupervisorDetailSnapshot::new(None, "core detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "core", "core"),
            Some("core projection notice".to_string()),
        )));

    let readiness_snapshot = runtime
        .app()
        .parallel_mode_readiness_snapshot()
        .expect("core readiness projection should be visible");
    let supervisor_snapshot = runtime.app().parallel_mode_supervisor_snapshot();

    assert_eq!(
        readiness_snapshot.readiness,
        ParallelModeReadinessState::Ready
    );
    assert_eq!(readiness_snapshot.top_alert, None);
    assert_eq!(supervisor_snapshot.pool.configured_size, 4);
    assert_eq!(supervisor_snapshot.pool.pool_root_label, "/tmp/core-pool");
    assert_eq!(
        supervisor_snapshot.top_notice.as_deref(),
        Some("core projection notice")
    );
}

#[test]
fn supersession_active_worker_requests_live_pulse() {
    /*
     * Active parallel workers need periodic redraws even when no stream event arrives,
     * otherwise the Supersession board looks frozen while a worker is running.
     */
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
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

    assert!(runtime.app().live_activity_pulse(Instant::now()).is_some());
}

#[test]
fn supersession_overlay_blocks_plain_r_prompt_input_while_loading() {
    /*
     * `r`은 Ctrl-R refresh shortcut과 같은 문자다. modifier가 없으면 overlay control도 아니지만,
     * Supersession loading 중에는 prompt text로도 내려가면 안 된다.
     */
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
    runtime.app_mut().set_parallel_mode_enabled_for_test(true);
    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory,
            ParallelModePoolBoardSnapshot::new(0, "loading: pool", "loading", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "loading agent roster"),
            ParallelModeSupervisorDetailSnapshot::new(None, "loading detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "loading", "loading"),
            Some("loading 2/3: pool reconcile".to_string()),
        )));
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('r'),
        KeyModifiers::empty(),
    )));
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert!(conversation.input_buffer.is_empty());
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Supersession);
    assert!(runtime.take_redraw_request());
}

#[test]
fn supersession_overlay_ctrl_r_refreshes_readiness() {
    /*
     * 같은 `r`이라도 Ctrl modifier가 붙으면 supersession overlay의 parallel readiness refresh로 간다.
     * refresh는 status만 갱신해야 하므로 prompt buffer를 비우거나 overlay를 닫는 부작용이 없는지 함께 확인한다.
     */
    let mut runtime = make_test_runtime();
    runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('r'),
        KeyModifiers::CONTROL,
    )));
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert!(conversation.input_buffer.is_empty());
    assert!(
        conversation
            .status_text
            .starts_with("parallel readiness refreshed / state:")
    );
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Supersession);
    assert!(runtime.take_redraw_request());
}

#[test]
fn supersession_overlay_blocks_enter_submit_prompt_while_loading() {
    /*
     * Supersession overlay가 loading 중이면 Enter도 prompt submit으로 내려가지 않는다.
     * startup diagnostics를 Ready로 만든 이유는 startup guard가 아니라 overlay routing을
     * 직접 검증하기 위해서다.
     */
    let mut runtime = make_test_runtime();
    let workspace_directory = runtime.app().current_workspace_directory();
    runtime.app_mut().startup_state = StartupState::Ready(sample_startup_diagnostics(
        &runtime.app().current_workspace_directory(),
    ));
    runtime.app_mut().shell_overlay = ShellOverlay::Supersession;
    runtime.app_mut().set_parallel_mode_enabled_for_test(true);
    runtime
        .app_mut()
        .set_parallel_mode_supervisor_snapshot_for_test(Some(ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            workspace_directory,
            ParallelModePoolBoardSnapshot::new(0, "loading: pool", "loading", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "loading agent roster"),
            ParallelModeSupervisorDetailSnapshot::new(None, "loading detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "loading", "loading"),
            Some("loading 2/3: pool reconcile".to_string()),
        )));
    for character in "run next".chars() {
        runtime.app_mut().push_input_character(character);
    }
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(conversation.input_buffer, "run next");
    assert!(!conversation.has_running_turn());
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Supersession);
    assert!(runtime.take_redraw_request());
}

#[test]
fn enter_executes_selected_inline_command_palette_item() {
    /*
     * colon command palette에서 실행형 항목을 고르면 prompt submit이 아니라 shell command executor로 간다.
     * `:d`는 diagnostics overlay를 여는 대표 side effect라, command execution route가 실제 overlay
     * 상태까지 바꾸는지 확인하기 좋다.
     */
    let mut runtime = make_test_runtime();
    runtime.app_mut().push_input_character(':');
    runtime.app_mut().push_input_character('d');
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Startup);
    assert!(conversation.input_buffer.is_empty());
    assert!(
        conversation
            .status_text
            .contains("opened diagnostics inspection")
    );
}

#[test]
fn down_then_enter_on_palette_item_with_argument_inserts_completion() {
    /*
     * argument가 필요한 palette item은 즉시 실행하지 않고 buffer completion만 삽입한다.
     * `:reset `처럼 공백까지 포함한 입력을 남겨 사용자가 대상 argument를 이어서 칠 수 있게 한다.
     */
    let mut runtime = make_test_runtime();
    runtime.app_mut().push_input_character(':');
    runtime.app_mut().push_input_character('r');
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(conversation.input_buffer, ":reset ");
    assert!(!conversation.inline_shell_command_palette_state.is_active());
    assert_eq!(runtime.app().shell_overlay, ShellOverlay::Hidden);
}

#[test]
fn up_wraps_inline_command_palette_selection() {
    /*
     * Palette selection은 위쪽 이동에서 끝 항목으로 wrap된다. keyboard-only 사용자가 짧은 prefix
     * 상태에서도 모든 command에 접근할 수 있게 하는 navigation contract다.
     */
    let mut runtime = make_test_runtime();
    runtime.app_mut().push_input_character(':');
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(
        conversation
            .inline_shell_command_palette_state
            .selected_command(),
        Some(InlineShellCommand::Help)
    );
}

#[test]
fn escape_dismisses_inline_command_palette_without_clearing_buffer() {
    /*
     * Escape는 palette chrome만 닫고 사용자가 입력한 raw command prefix는 보존한다.
     * 그래야 suggestion을 숨긴 뒤에도 같은 buffer를 일반 prompt text처럼 계속 편집할 수 있다.
     */
    let mut runtime = make_test_runtime();
    runtime.app_mut().push_input_character(':');
    runtime.app_mut().push_input_character('p');
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(conversation.input_buffer, ":p");
    assert!(!conversation.inline_shell_command_palette_state.is_active());
}

#[test]
fn page_navigation_keys_do_not_trigger_transcript_navigation() {
    /*
     * PageUp/PageDown은 예전 transcript navigation과 host terminal scrollback이 충돌하던 키다.
     * 현재 input runtime에서는 redraw도 요구하지 않는 no-op로 고정해 terminal이 가진 scrollback
     * behavior와 앱 내부 navigation이 경쟁하지 않게 한다.
     */
    let mut runtime = make_test_runtime();
    runtime.take_redraw_request();

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::PageUp,
        KeyModifiers::NONE,
    )));

    assert!(!runtime.take_redraw_request());
}

#[test]
fn ctrl_u_clears_buffered_input() {
    /*
     * Ctrl-U는 shell-style line kill shortcut이다. conversation reducer를 거쳐 prompt buffer만 비우고
     * session/overlay 상태는 건드리지 않는지 확인한다.
     */
    let mut runtime = make_test_runtime();
    runtime.app_mut().push_input_character('s');
    runtime.app_mut().push_input_character('h');
    runtime.app_mut().push_input_character('i');
    runtime.app_mut().push_input_character('p');

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('u'),
        KeyModifiers::CONTROL,
    )));
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert!(conversation.input_buffer.is_empty());
}

#[test]
fn ctrl_w_deletes_previous_buffered_word() {
    /*
     * Ctrl-W는 직전 단어만 제거하는 shell-style editing shortcut이다. 공백을 보존한 결과를 확인해
     * 다음 단어 입력이 자연스럽게 이어지는 prompt editing contract를 고정한다.
     */
    let mut runtime = make_test_runtime();
    for character in "ship this next".chars() {
        runtime.app_mut().push_input_character(character);
    }

    runtime.handle_terminal_event(Event::Key(KeyEvent::new(
        KeyCode::Char('w'),
        KeyModifiers::CONTROL,
    )));
    let ConversationState::Ready(conversation) = &runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert_eq!(conversation.input_buffer, "ship this ");
}
