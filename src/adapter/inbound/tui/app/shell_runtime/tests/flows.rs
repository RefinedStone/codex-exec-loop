use super::*;
use crate::adapter::inbound::tui::app::conversation_runtime::{
    ConversationPostTurnAction, ConversationPostTurnEvaluation, QueuedAutoPrompt,
};
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::service::planning::PlanningTaskIntakeCommitResult;
use crate::application::service::planning::task_tool::{
    PlanningTaskToolRequest, PlanningTaskToolUpdateRequest, PlanningTaskUpdatePayload,
};
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterSnapshot, ParallelModeAgentSessionDetailSnapshot,
    ParallelModeAutomationTrigger, ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot,
    ParallelModeCapabilityState, ParallelModeDispatchBlockReason, ParallelModeDistributorSnapshot,
    ParallelModeLiveSessionDetailDefaults, ParallelModePoolBoardSnapshot,
    ParallelModeReadinessSnapshot, ParallelModeReadinessState, ParallelModeSlotLeaseRequest,
    ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
    ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorSnapshot,
    ParallelModeSupervisorState, ParallelModeTaskDispatchBlockSnapshot,
};
use crate::domain::planning::TaskStatus;
use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{MutexGuard, OnceLock};
use std::thread;
use std::time::Duration;

static FLOW_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn flow_test_guard() -> MutexGuard<'static, ()> {
    FLOW_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("flow test lock should not be poisoned")
}

struct NativeFlowHarness {
    runtime: ShellRuntime,
    workspace_dir: String,
    authority: Arc<SqlitePlanningAuthorityAdapter>,
    worker_port: Arc<FlowParallelAgentWorkerPort>,
}

#[derive(Debug, Default)]
struct FlowParallelAgentWorkerPort {
    launch_count: AtomicUsize,
    fail_launch: AtomicBool,
    requests: Mutex<Vec<FlowWorkerLaunchRequest>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FlowWorkerLaunchRequest {
    cwd: String,
    prompt: String,
    service_name: String,
}

impl FlowParallelAgentWorkerPort {
    fn launch_count(&self) -> usize {
        self.launch_count.load(Ordering::SeqCst)
    }

    fn set_fail_launch(&self, should_fail: bool) {
        self.fail_launch.store(should_fail, Ordering::SeqCst);
    }

    fn requests(&self) -> Vec<FlowWorkerLaunchRequest> {
        self.requests
            .lock()
            .expect("worker request mutex should not be poisoned")
            .clone()
    }
}

impl ParallelAgentWorkerPort for FlowParallelAgentWorkerPort {
    fn run_isolated_new_thread_stream(
        &self,
        request: ParallelAgentWorkerStreamRequest<'_>,
        event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        self.launch_count.fetch_add(1, Ordering::SeqCst);
        self.requests
            .lock()
            .expect("worker request mutex should not be poisoned")
            .push(FlowWorkerLaunchRequest {
                cwd: request.cwd.to_string(),
                prompt: request.prompt.to_string(),
                service_name: request.service_name.to_string(),
            });
        if self.fail_launch.load(Ordering::SeqCst) {
            let _ = event_sender.send(ConversationStreamEvent::Failed {
                message: "flow worker launch failed before stream start".to_string(),
            });
        }
        Ok(())
    }
}

impl NativeFlowHarness {
    fn new(prefix: &str) -> Self {
        let workspace_dir = create_temp_git_repo(prefix);
        let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let planning = PlanningServices::from_ports(
            Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
            authority.clone(),
            authority.clone(),
            Arc::new(NoopPlanningWorkerPort),
        );
        bootstrap_active_planning_workspace_with_services(&planning, &workspace_dir);
        let worker_port = Arc::new(FlowParallelAgentWorkerPort::default());
        let codex_port = Arc::new(FakeCodexAppServerPort);
        let mut app = NativeTuiApp::new(
            StartupService::new(codex_port.clone()),
            SessionService::new(codex_port.clone()),
            ConversationService::new(codex_port),
            worker_port.clone(),
            crate::adapter::inbound::tui::app::test_helpers::test_parallel_mode_service(),
            planning,
        );
        app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir));
        app.sync_draft_shell_workspace(&workspace_dir);
        app.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(&workspace_dir);

        Self {
            runtime: ShellRuntime::new(app),
            workspace_dir,
            authority,
            worker_port,
        }
    }

    fn committed_ready_task(&self, prompt: &str) -> PlanningTaskIntakeCommitResult {
        let proposal = self
            .runtime
            .app()
            .planning
            .runtime
            .prepare_task_intake(PlanningTaskIntakeRequest {
                workspace_directory: self.workspace_dir.clone(),
                raw_prompt: prompt.to_string(),
                active_turn_id: None,
                requested_direction_id: None,
                observed_planning_revision: None,
            })
            .expect("task intake proposal should prepare");
        self.runtime
            .app()
            .planning
            .runtime
            .commit_task_intake(&proposal)
            .expect("task intake proposal should commit")
    }

    fn update_task_description(&self, task_id: &str, description: &str) {
        self.runtime
            .app()
            .planning
            .task_tool
            .run(
                &self.workspace_dir,
                PlanningTaskToolRequest::UpdateTask(PlanningTaskToolUpdateRequest {
                    version: 1,
                    apply: true,
                    source_turn_id: Some("flow-update".to_string()),
                    input: PlanningTaskUpdatePayload {
                        task_id: task_id.to_string(),
                        direction_id: None,
                        direction_relation_note: None,
                        title: None,
                        description: Some(description.to_string()),
                        status: Some(TaskStatus::Ready),
                        base_priority: None,
                        dynamic_priority_delta: Some(1),
                        priority_reason: Some(
                            "Flow test retry after worker launch failure".to_string(),
                        ),
                        depends_on: None,
                        blocked_by: None,
                    },
                }),
            )
            .expect("task update should commit");
    }

    fn make_lease_stale_for_startup_reset(
        &self,
        lease: &ParallelModeSlotLeaseSnapshot,
    ) -> ParallelModeSlotLeaseSnapshot {
        let mut stale_lease = lease.clone();
        stale_lease.leased_at = "2026-01-01T00:00:00Z".to_string();
        self.authority
            .upsert_runtime_slot_lease(&self.workspace_dir, &stale_lease)
            .expect("stale lease projection should persist");
        let pool_root = PathBuf::from(&stale_lease.worktree_path)
            .parent()
            .expect("slot worktree should have pool parent")
            .to_path_buf();
        let lease_path = pool_root
            .join(".leases")
            .join(format!("{}.json", stale_lease.slot_id));
        fs::create_dir_all(
            lease_path
                .parent()
                .expect("lease path should have parent directory"),
        )
        .expect("lease directory should be created");
        fs::write(
            lease_path,
            serde_json::to_string_pretty(&stale_lease).expect("stale lease should serialize"),
        )
        .expect("stale lease mirror should persist");
        let detail = ParallelModeAgentSessionDetailSnapshot::assigned_for_lease(
            &stale_lease,
            ParallelModeLiveSessionDetailDefaults {
                validation_summary: "validation summary is not recorded in runtime yet",
                authority_refresh_outcome: "no official completion has been reported yet",
            },
        );
        self.authority
            .upsert_runtime_session_detail(&self.workspace_dir, &detail)
            .expect("stale assigned session detail should persist");
        stale_lease
    }

    fn enter_parallel(&mut self) {
        self.submit_inline_command(":parallel");
    }

    fn turn_parallel_off(&mut self) {
        self.submit_inline_command(":parallel off");
    }

    fn submit_inline_command(&mut self, command: &str) {
        for character in command.chars() {
            self.runtime.app_mut().push_input_character(character);
        }
        self.runtime.take_redraw_request();
        self.runtime.handle_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
    }

    fn send_post_turn_auto_prompt(&mut self, turn_id: &str) {
        let planning_snapshot = self
            .runtime
            .app()
            .planning
            .runtime
            .load_runtime_snapshot_or_invalid(&self.workspace_dir);
        let ConversationState::Ready(conversation) = &mut self.runtime.app_mut().conversation_state
        else {
            panic!("expected ready conversation state");
        };
        conversation.thread_id = "thread-1".to_string();
        conversation.turn_activity.last_completed_turn_id = Some(turn_id.to_string());

        self.runtime
            .app
            .tx
            .send(BackgroundMessage::PostTurnEvaluated {
                thread_id: "thread-1".to_string(),
                queued_from_turn_id: turn_id.to_string(),
                evaluation: Box::new(ConversationPostTurnEvaluation {
                    planning_runtime_snapshot: planning_snapshot,
                    planning_repair_state: None,
                    runtime_notices: Vec::new(),
                    action: ConversationPostTurnAction::QueueAutoPrompt(Box::new(
                        QueuedAutoPrompt {
                            prompt: "run next task".to_string(),
                            queued_from_turn_id: turn_id.to_string(),
                            mode_label: "flow".to_string(),
                            transcript_text: "next-task".to_string(),
                            handoff_task: None,
                        },
                    )),
                }),
                planner_worker_panel_state: Default::default(),
            })
            .expect("post-turn evaluation should enqueue");
    }

    fn send_dispatch_request_for_current_epoch(&self, trigger: ParallelModeAutomationTrigger) {
        let epoch_id = self
            .runtime
            .app()
            .parallel_mode_automation_epoch_id
            .expect("automation epoch should be open");
        self.runtime
            .app
            .tx
            .send(BackgroundMessage::RequestParallelModeDispatch {
                workspace_directory: self.workspace_dir.clone(),
                trigger,
                epoch_id,
            })
            .expect("dispatch request should enqueue");
    }

    fn apply_ready_entered_snapshot(&mut self, status_text: &str) {
        self.runtime.app_mut().apply_parallel_mode_entered(
            &self.workspace_dir,
            ready_parallel_mode_readiness_snapshot(&self.workspace_dir),
            ready_parallel_mode_supervisor_snapshot(&self.workspace_dir),
            status_text.to_string(),
        );
    }

    fn poll_until_status_contains(&mut self, expected: &str) -> String {
        let mut final_status = String::new();
        for _ in 0..750 {
            self.runtime.poll_background_messages();
            if let ConversationState::Ready(conversation) = &self.runtime.app().conversation_state {
                final_status = conversation.status_text.clone();
                if final_status.contains(expected) {
                    return final_status;
                }
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("status did not contain `{expected}`; last status was `{final_status}`");
    }

    fn poll_until_worker_launches(&mut self, expected_launches: usize) {
        for _ in 0..750 {
            self.runtime.poll_background_messages();
            if self.worker_port.launch_count() >= expected_launches {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "expected at least {expected_launches} worker launch(es), got {}",
            self.worker_port.launch_count()
        );
    }

    fn poll_until_dispatch_idle(&mut self) {
        for _ in 0..750 {
            self.runtime.poll_background_messages();
            if !self.runtime.app().parallel_mode_dispatch_refresh_in_flight {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("parallel dispatch refresh did not become idle");
    }

    fn poll_until_runtime_blocks(&mut self, expected_blocks: usize) {
        for _ in 0..750 {
            self.runtime.poll_background_messages();
            let projections = self.runtime_projections();
            if projections.task_dispatch_blocks.len() >= expected_blocks {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "expected at least {expected_blocks} task dispatch block(s), got {:?}; notices: {:?}",
            self.runtime_projections().task_dispatch_blocks,
            self.runtime_notices()
        );
    }

    fn runtime_notices(&self) -> Vec<String> {
        let ConversationState::Ready(conversation) = &self.runtime.app().conversation_state else {
            return Vec::new();
        };
        conversation.runtime_notices.clone()
    }

    fn runtime_projections(
        &self,
    ) -> crate::application::port::outbound::planning_authority_port::PlanningAuthorityRuntimeProjectionSnapshot
    {
        self.authority
            .load_runtime_projections(&self.workspace_dir)
            .expect("runtime projections should load")
    }
}

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

fn ready_parallel_mode_supervisor_snapshot(
    workspace_directory: &str,
) -> ParallelModeSupervisorSnapshot {
    ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        workspace_directory.to_string(),
        ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
        ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
        None,
    )
}

fn loading_parallel_mode_supervisor_snapshot(
    workspace_directory: &str,
) -> ParallelModeSupervisorSnapshot {
    ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::Supervise,
        workspace_directory.to_string(),
        ParallelModePoolBoardSnapshot::new(0, "loading: pool reconcile", "loading", Vec::new()),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), "loading agent roster"),
        ParallelModeSupervisorDetailSnapshot::new(None, "loading detail"),
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "loading", "loading"),
        Some("loading 2/3: pool reconcile".to_string()),
    )
}

#[test]
fn parallel_entry_reaches_ready_without_dispatching_ready_queue() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-parallel-entry");
    harness.committed_ready_task("verify parallel entry does not auto dispatch");

    harness.enter_parallel();

    let final_status = harness.poll_until_status_contains("control tower ready");
    assert!(
        final_status.contains("parallel mode: on"),
        "parallel entry should finish successfully, got `{final_status}`"
    );
    assert_eq!(
        harness.worker_port.launch_count(),
        0,
        "bare :parallel entry must not launch isolated workers"
    );
}

#[test]
fn parallel_reentry_while_enabled_refreshes_without_reset_or_dispatch() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-parallel-reentry");
    harness.committed_ready_task("keep existing leased slot during reentry");
    let lease = harness
        .runtime
        .app()
        .parallel_mode_service()
        .acquire_slot_lease(
            &harness.workspace_dir,
            ParallelModeSlotLeaseRequest::new(
                "task-existing",
                "Existing Slot Task",
                "agent-existing",
                "existing-slot-task",
            ),
        )
        .expect("existing slot lease should be acquired");
    harness.runtime.app_mut().parallel_mode_enabled = true;

    harness.enter_parallel();
    harness.poll_until_status_contains("control tower ready");

    let projections = harness.runtime_projections();
    let persisted = projections
        .slot_leases
        .get(&lease.slot_id)
        .expect("enabled reentry should preserve existing lease");
    assert_eq!(persisted.state, ParallelModeSlotLeaseState::Leased);
    assert_eq!(harness.worker_port.launch_count(), 0);
}

#[test]
fn post_turn_auto_prompt_opens_parallel_epoch_and_dispatches_once() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-post-turn-dispatch");
    harness.committed_ready_task("dispatch one worker after main turn");
    harness.runtime.app_mut().parallel_mode_enabled = true;
    harness.runtime.app_mut().parallel_mode_readiness_snapshot = Some(
        ready_parallel_mode_readiness_snapshot(&harness.workspace_dir),
    );
    harness.runtime.app_mut().parallel_mode_supervisor_snapshot = Some(
        ready_parallel_mode_supervisor_snapshot(&harness.workspace_dir),
    );

    harness.send_post_turn_auto_prompt("turn-1");
    harness.poll_until_worker_launches(1);

    assert!(
        harness
            .runtime
            .app()
            .parallel_mode_automation_epoch_id
            .is_some()
    );
    assert_eq!(
        harness.runtime.app().last_parallel_mode_automation_trigger,
        Some(ParallelModeAutomationTrigger::MainTurnPostEvaluation)
    );
    assert_eq!(harness.worker_port.launch_count(), 1);
    let ConversationState::Ready(conversation) = &harness.runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert!(
        !conversation
            .status_text
            .contains("queued auto follow-up with mode flow"),
        "parallel mode should suppress the main-session auto-follow submit"
    );
}

#[test]
fn dispatch_requests_during_entry_loading_coalesce_until_ready() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-dispatch-coalesce");
    harness.committed_ready_task("coalesce dispatch while supervisor loads");
    harness.runtime.app_mut().parallel_mode_enabled = true;
    harness.runtime.app_mut().parallel_mode_readiness_snapshot = None;
    harness.runtime.app_mut().parallel_mode_supervisor_snapshot = Some(
        loading_parallel_mode_supervisor_snapshot(&harness.workspace_dir),
    );

    harness.send_post_turn_auto_prompt("turn-1");
    harness.runtime.poll_background_messages();
    assert_eq!(harness.worker_port.launch_count(), 0);
    assert_eq!(
        harness.runtime.app().pending_parallel_mode_dispatch_trigger,
        Some(ParallelModeAutomationTrigger::MainTurnPostEvaluation)
    );

    harness
        .runtime
        .app_mut()
        .refresh_parallel_mode_dispatch_after_task_update("task-added-while-loading");
    assert_eq!(
        harness.runtime.app().pending_parallel_mode_dispatch_trigger,
        Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch)
    );

    harness
        .apply_ready_entered_snapshot("parallel mode: on / readiness: ready / control tower ready");
    harness.poll_until_worker_launches(1);
    harness.poll_until_dispatch_idle();

    assert_eq!(harness.worker_port.launch_count(), 1);
    assert_eq!(
        harness.runtime.app().last_parallel_mode_automation_trigger,
        Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch)
    );
}

#[test]
fn live_running_slot_blocks_off_to_on_pool_reset() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-live-reset-block");
    let lease = harness
        .runtime
        .app()
        .parallel_mode_service()
        .acquire_slot_lease(
            &harness.workspace_dir,
            ParallelModeSlotLeaseRequest::new(
                "task-running",
                "Running Slot Task",
                "agent-running",
                "running-slot-task",
            ),
        )
        .expect("slot lease should be acquired");
    let running_lease = harness
        .runtime
        .app()
        .parallel_mode_service()
        .mark_slot_running(&harness.workspace_dir, &lease.slot_id, "agent-running")
        .expect("slot should be marked running");
    fs::write(
        PathBuf::from(&running_lease.worktree_path).join("keep-running.tmp"),
        "running evidence\n",
    )
    .expect("running evidence should be written");

    harness.runtime.app_mut().parallel_mode_enabled = true;
    harness.turn_parallel_off();
    harness.enter_parallel();

    let final_status = harness.poll_until_status_contains("pool reset failed");
    assert!(
        final_status.contains("live slot"),
        "reset should report live blocker, got `{final_status}`"
    );
    let projections = harness.runtime_projections();
    let persisted = projections
        .slot_leases
        .get(&lease.slot_id)
        .expect("running slot lease should be preserved");
    assert_eq!(persisted.state, ParallelModeSlotLeaseState::Running);
    assert!(
        PathBuf::from(&running_lease.worktree_path)
            .join("keep-running.tmp")
            .exists(),
        "off-to-on reset must not clean live running worktree evidence"
    );
    assert_eq!(harness.worker_port.launch_count(), 0);
}

#[test]
fn stale_leased_slot_reset_preserves_failed_start_dispatch_block() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-stale-lease-reset");
    let lease = harness
        .runtime
        .app()
        .parallel_mode_service()
        .acquire_slot_lease(
            &harness.workspace_dir,
            ParallelModeSlotLeaseRequest::new(
                "task-stale",
                "Stale Leased Slot Task",
                "agent-stale",
                "stale-leased-slot-task",
            ),
        )
        .expect("stale slot lease should be acquired");
    let stale_lease = harness.make_lease_stale_for_startup_reset(&lease);
    harness
        .authority
        .upsert_runtime_task_dispatch_block(
            &harness.workspace_dir,
            &ParallelModeTaskDispatchBlockSnapshot::new(
                "task-failed-start",
                "2026-05-04T11:00:00Z",
                "2026-05-04T12:00:00Z",
                ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
            ),
        )
        .expect("failed-start dispatch block should persist");

    harness.enter_parallel();
    harness.poll_until_status_contains("control tower ready");

    let projections = harness.runtime_projections();
    assert!(
        !projections.slot_leases.contains_key(&stale_lease.slot_id),
        "stale leased slot should be reset during off-to-on entry"
    );
    assert_eq!(projections.task_dispatch_blocks.len(), 1);
    assert_eq!(
        projections.task_dispatch_blocks[0].task_id,
        "task-failed-start"
    );
    assert_eq!(harness.worker_port.launch_count(), 0);
}

#[test]
fn late_enter_result_after_parallel_off_does_not_reenable_mode() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-late-enter-result");
    harness.runtime.app_mut().parallel_mode_enabled = true;
    harness.turn_parallel_off();

    harness
        .runtime
        .app
        .tx
        .send(BackgroundMessage::ParallelModeEntered {
            workspace_directory: harness.workspace_dir.clone(),
            readiness_snapshot: ready_parallel_mode_readiness_snapshot(&harness.workspace_dir),
            supervisor_snapshot: Box::new(ready_parallel_mode_supervisor_snapshot(
                &harness.workspace_dir,
            )),
            status_text: "parallel mode: on / readiness: ready / control tower ready".to_string(),
        })
        .expect("late enter result should enqueue");
    harness.runtime.poll_background_messages();

    assert!(
        !harness.runtime.app().parallel_mode_enabled,
        "late background enter result must not re-enable parallel mode after :parallel off"
    );
}

#[test]
fn worker_launch_failure_blocks_task_until_task_update_then_retries() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-worker-failure-retry");
    let commit = harness.committed_ready_task("retry worker after failed launch");
    harness.worker_port.set_fail_launch(true);
    harness.runtime.app_mut().parallel_mode_enabled = true;
    harness.runtime.app_mut().parallel_mode_readiness_snapshot = Some(
        ready_parallel_mode_readiness_snapshot(&harness.workspace_dir),
    );
    harness.runtime.app_mut().parallel_mode_supervisor_snapshot = Some(
        ready_parallel_mode_supervisor_snapshot(&harness.workspace_dir),
    );

    harness.send_post_turn_auto_prompt("turn-1");
    harness.poll_until_worker_launches(1);
    harness.poll_until_runtime_blocks(1);
    harness.poll_until_dispatch_idle();

    harness.send_dispatch_request_for_current_epoch(
        ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
    );
    harness.poll_until_dispatch_idle();
    assert_eq!(
        harness.worker_port.launch_count(),
        1,
        "failed-start block should exclude unchanged task from redispatch"
    );

    thread::sleep(Duration::from_millis(10));
    harness.update_task_description(
        &commit.committed_task_id,
        "Updated after the worker failed before startup.",
    );
    harness.send_dispatch_request_for_current_epoch(
        ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
    );
    harness.poll_until_worker_launches(2);

    assert_eq!(harness.worker_port.launch_count(), 2);
    assert!(
        harness
            .worker_port
            .requests()
            .iter()
            .all(|request| request.cwd.contains("slot-")),
        "parallel workers should launch inside slot worktrees"
    );
}
