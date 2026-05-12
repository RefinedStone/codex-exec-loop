use super::*;
use crate::adapter::inbound::tui::app::conversation_model::AutoFollowSkipReason;
use crate::adapter::inbound::tui::app::conversation_runtime::{
    PostTurnContinuationAction, PostTurnEvaluationOutcome, PostTurnEvaluationProvenance,
    PostTurnQueuedPrompt,
};
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneBackgroundEvent;
use crate::application::service::parallel_mode::control_plane::{
    ParallelModeControlPlaneEffectId, ParallelModeControlPlaneEffectKind,
};
use crate::application::service::planning::task_tool::{
    PlanningTaskToolRequest, PlanningTaskToolUpdateRequest, PlanningTaskUpdatePayload,
};
use crate::application::service::planning::{
    PlanningRuntimeProjection, PlanningTaskIntakeCommitResult,
};
use crate::domain::operator_alert::OperatorAlert;
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterSnapshot, ParallelModeAgentSessionDetailSnapshot,
    ParallelModeAutomationTrigger, ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot,
    ParallelModeCapabilityState, ParallelModeControlPlaneWorkerEvent,
    ParallelModeControlPlaneWorkerEventKind, ParallelModeDispatchBlockReason,
    ParallelModeDispatchCommandSnapshot, ParallelModeDispatchCommandState,
    ParallelModeDistributorSnapshot, ParallelModeLiveSessionDetailDefaults,
    ParallelModePoolBoardSnapshot, ParallelModePoolSlotState, ParallelModePostTurnQueueSignal,
    ParallelModeReadinessSnapshot, ParallelModeReadinessState, ParallelModeSlotLeaseRequest,
    ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
    ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorSnapshot,
    ParallelModeSupervisorState, ParallelModeTaskDispatchBlockSnapshot,
};
use crate::domain::planning::TaskStatus;
use anyhow::Result;
use chrono::{DateTime, SecondsFormat, Utc};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

static FLOW_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
const FLOW_POOL_SIZE: usize = 3;
const FLOW_POOL_BASELINE_BRANCH: &str = "prerelease";

fn flow_test_guard() -> MutexGuard<'static, ()> {
    FLOW_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct NativeFlowHarness {
    runtime: ShellRuntime,
    workspace_dir: String,
    authority: Arc<SqlitePlanningAuthorityAdapter>,
    parallel_mode_service: crate::application::service::parallel_mode::ParallelModeService,
    worker_port: Arc<FlowParallelAgentWorkerPort>,
}

#[derive(Debug, Default)]
struct FlowParallelAgentWorkerPort {
    launch_count: AtomicUsize,
    fail_launch: AtomicBool,
    hold_streams: AtomicBool,
    held_streams: Mutex<FlowHeldWorkerState>,
    held_streams_released: Condvar,
    requests: Mutex<Vec<FlowWorkerLaunchRequest>>,
}

#[derive(Debug, Default)]
struct FlowHeldWorkerState {
    active_count: usize,
    terminal_count: usize,
    release_all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FlowWorkerLaunchRequest {
    cwd: String,
    prompt: String,
    developer_instructions: String,
    service_name: String,
}

impl FlowParallelAgentWorkerPort {
    fn launch_count(&self) -> usize {
        self.launch_count.load(Ordering::SeqCst)
    }

    fn set_fail_launch(&self, should_fail: bool) {
        self.fail_launch.store(should_fail, Ordering::SeqCst);
    }

    fn hold_worker_streams(self: &Arc<Self>) -> FlowHeldWorkerReleaseGuard {
        {
            let mut state = self
                .held_streams
                .lock()
                .expect("held stream state mutex should not be poisoned");
            *state = FlowHeldWorkerState::default();
        }
        self.hold_streams.store(true, Ordering::SeqCst);
        FlowHeldWorkerReleaseGuard {
            worker_port: self.clone(),
        }
    }

    fn active_stream_count(&self) -> usize {
        self.held_streams
            .lock()
            .expect("held stream state mutex should not be poisoned")
            .active_count
    }

    fn terminal_stream_count(&self) -> usize {
        self.held_streams
            .lock()
            .expect("held stream state mutex should not be poisoned")
            .terminal_count
    }

    fn release_all_held_streams(&self) {
        self.hold_streams.store(false, Ordering::SeqCst);
        let mut state = self
            .held_streams
            .lock()
            .expect("held stream state mutex should not be poisoned");
        state.release_all = true;
        self.held_streams_released.notify_all();
    }

    fn requests(&self) -> Vec<FlowWorkerLaunchRequest> {
        self.requests
            .lock()
            .expect("worker request mutex should not be poisoned")
            .clone()
    }
}

struct FlowHeldWorkerReleaseGuard {
    worker_port: Arc<FlowParallelAgentWorkerPort>,
}

impl Drop for FlowHeldWorkerReleaseGuard {
    fn drop(&mut self) {
        self.worker_port.release_all_held_streams();
    }
}

impl ParallelAgentWorkerPort for FlowParallelAgentWorkerPort {
    fn run_isolated_new_thread_stream(
        &self,
        request: ParallelAgentWorkerStreamRequest<'_>,
        event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        let launch_index = self.launch_count.fetch_add(1, Ordering::SeqCst) + 1;
        self.requests
            .lock()
            .expect("worker request mutex should not be poisoned")
            .push(FlowWorkerLaunchRequest {
                cwd: request.cwd.to_string(),
                prompt: request.prompt.to_string(),
                developer_instructions: request.developer_instructions.to_string(),
                service_name: request.service_name.to_string(),
            });
        if self.fail_launch.load(Ordering::SeqCst) {
            let _ = event_sender.send(ConversationStreamEvent::Failed {
                message: "flow worker launch failed before stream start".to_string(),
            });
            return Ok(());
        }
        if self.hold_streams.load(Ordering::SeqCst) {
            let _ = event_sender.send(ConversationStreamEvent::ThreadPrepared {
                thread_id: format!("flow-thread-{launch_index}"),
                title: format!("Flow Worker {launch_index}"),
                cwd: request.cwd.to_string(),
            });
            let _ = event_sender.send(ConversationStreamEvent::TurnStarted {
                turn_id: format!("flow-turn-{launch_index}"),
            });

            let mut state = self
                .held_streams
                .lock()
                .expect("held stream state mutex should not be poisoned");
            state.active_count += 1;
            self.held_streams_released.notify_all();
            let deadline = Instant::now() + Duration::from_secs(30);
            while !state.release_all {
                let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                    break;
                };
                let (next_state, _timeout) = self
                    .held_streams_released
                    .wait_timeout(state, remaining.min(Duration::from_millis(100)))
                    .expect("held stream state mutex should not be poisoned");
                state = next_state;
            }
            state.terminal_count += 1;
            drop(state);
            let _ = event_sender.send(ConversationStreamEvent::Failed {
                message: "flow worker stream released by test harness".to_string(),
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
        let codex_port = Arc::new(FakeAppServerPort);
        let parallel_mode_service =
            crate::adapter::inbound::tui::app::test_helpers::test_parallel_mode_service();
        let parallel_mode_control_plane_composition =
            crate::adapter::inbound::tui::app::test_helpers::test_parallel_mode_control_plane_composition_with_worker(
                parallel_mode_service.clone(),
                planning,
                worker_port.clone(),
            );
        let parallel_mode_binding =
            NativeTuiParallelModeBinding::from_composition(parallel_mode_control_plane_composition);
        let mut app = NativeTuiApp::new(
            StartupService::new(codex_port.clone()),
            SessionService::new(codex_port.clone()),
            ConversationService::new(codex_port),
            parallel_mode_binding,
        );
        app.startup_state = StartupState::Ready(sample_startup_diagnostics(&workspace_dir));
        app.sync_draft_shell_workspace(&workspace_dir);
        app.refresh_ready_conversation_planning_runtime_projection_for_workspace(&workspace_dir);

        Self {
            runtime: ShellRuntime::new(app),
            workspace_dir,
            authority,
            parallel_mode_service,
            worker_port,
        }
    }

    fn seed_ready_parallel_mode_projections(&mut self) {
        self.runtime
            .app_mut()
            .set_parallel_mode_readiness_snapshot_for_test(Some(
                ready_parallel_mode_readiness_snapshot(&self.workspace_dir),
            ));
        self.runtime
            .app_mut()
            .set_parallel_mode_supervisor_snapshot_for_test(Some(
                ready_parallel_mode_supervisor_snapshot(&self.workspace_dir),
            ));
    }

    fn seed_loading_parallel_mode_supervisor(&mut self) {
        self.runtime
            .app_mut()
            .set_parallel_mode_readiness_snapshot_for_test(None);
        self.runtime
            .app_mut()
            .set_parallel_mode_supervisor_snapshot_for_test(Some(
                loading_parallel_mode_supervisor_snapshot(&self.workspace_dir),
            ));
    }

    fn committed_ready_task(&self, prompt: &str) -> PlanningTaskIntakeCommitResult {
        let proposal = self
            .runtime
            .app()
            .application
            .planning()
            .runtime()
            .prepare_task_intake(PlanningTaskIntakeRequest {
                workspace_directory: self.workspace_dir.clone(),
                raw_prompt: prompt.to_string(),
                legacy_source_turn_id: None,
                provenance: Default::default(),
                requested_direction_id: None,
                observed_planning_revision: None,
            })
            .expect("task intake proposal should prepare");
        self.runtime
            .app()
            .application
            .planning()
            .runtime()
            .commit_task_intake(&proposal)
            .expect("task intake proposal should commit")
    }

    fn update_task_description(&self, task_id: &str, description: &str) {
        self.runtime
            .app()
            .application
            .planning()
            .task_tool()
            .run(
                &self.workspace_dir,
                PlanningTaskToolRequest::UpdateTask(PlanningTaskToolUpdateRequest {
                    version: 1,
                    apply: true,
                    legacy_source_turn_id: Some("flow-update".to_string()),
                    origin_session_kind: None,
                    thread_id: None,
                    turn_id: None,
                    parent_thread_id: None,
                    parent_turn_id: None,
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

    fn remove_idle_pool_worktrees_except(&self, preserved_slot_id: &str, pool_root: &Path) {
        for slot_number in 1..=FLOW_POOL_SIZE {
            let slot_id = format!("slot-{slot_number}");
            if slot_id == preserved_slot_id {
                continue;
            }
            let slot_path = pool_root.join(&slot_id);
            if !slot_path.exists() {
                continue;
            }
            let slot_path_string = slot_path.display().to_string();
            git_stdout(
                Path::new(&self.workspace_dir),
                &["worktree", "remove", "--force", slot_path_string.as_str()],
            );
            if slot_path.exists() {
                fs::remove_dir_all(&slot_path).expect("idle slot residue should be removable");
            }
        }
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
        let planning_projection = self
            .runtime
            .app()
            .application
            .planning()
            .runtime()
            .load_runtime_projection_or_invalid(&self.workspace_dir);
        let ConversationState::Ready(conversation) = &mut self.runtime.app_mut().conversation_state
        else {
            panic!("expected ready conversation state");
        };
        conversation.thread_id = "thread-1".to_string();
        conversation.turn_activity.last_completed_turn_id = Some(turn_id.to_string());
        mark_core_turn_completed(&mut self.runtime, "thread-1", turn_id);

        self.runtime
            .app
            .tx
            .send(post_turn_evaluation_completed_message(
                "thread-1",
                turn_id,
                PostTurnEvaluationOutcome {
                    provenance: PostTurnEvaluationProvenance::new(turn_id.to_string()),
                    runtime_projection: planning_projection,
                    planning_repair_state: None,
                    runtime_notices: Vec::new(),
                    action: PostTurnContinuationAction::QueueAutoPrompt(Box::new(
                        PostTurnQueuedPrompt {
                            prompt: "run next task".to_string(),
                            mode_label: "flow".to_string(),
                            transcript_text: "next-task".to_string(),
                        },
                    )),
                    operator_alerts: Vec::new(),
                },
                Default::default(),
            ))
            .expect("post-turn evaluation should enqueue");
    }

    fn send_parallel_completion_with_ready_queue_head(&mut self, turn_id: &str) {
        let planning_projection = self
            .runtime
            .app()
            .application
            .planning()
            .runtime()
            .load_runtime_projection_or_invalid(&self.workspace_dir);
        assert!(
            planning_projection.has_actionable_queue_head(),
            "test setup should leave a ready queue head"
        );
        let ConversationState::Ready(conversation) = &mut self.runtime.app_mut().conversation_state
        else {
            panic!("expected ready conversation state");
        };
        conversation.thread_id = "thread-1".to_string();
        conversation.turn_activity.last_completed_turn_id = Some(turn_id.to_string());
        mark_core_turn_completed(&mut self.runtime, "thread-1", turn_id);

        self.runtime
            .app
            .tx
            .send(post_turn_evaluation_completed_message(
                "thread-1",
                turn_id,
                PostTurnEvaluationOutcome {
                    provenance: PostTurnEvaluationProvenance::new(turn_id.to_string())
                        .with_parallel_queue_signal(Some(
                            ParallelModePostTurnQueueSignal::ParallelCompletionFinalized,
                        )),
                    runtime_projection: planning_projection,
                    planning_repair_state: None,
                    runtime_notices: Vec::new(),
                    action: PostTurnContinuationAction::SkipAutoFollow {
                        reason: AutoFollowSkipReason::ParallelSessionCompleted,
                    },
                    operator_alerts: Vec::new(),
                },
                Default::default(),
            ))
            .expect("parallel completion evaluation should enqueue");
    }

    fn send_parallel_completion_with_drained_queue(&mut self, turn_id: &str) {
        let planning_projection = PlanningRuntimeProjection::ready_with_details(
            "Planning Context".to_string(),
            "queue idle: no executable planning task".to_string(),
            None,
            None,
        );
        let ConversationState::Ready(conversation) = &mut self.runtime.app_mut().conversation_state
        else {
            panic!("expected ready conversation state");
        };
        conversation.thread_id = "thread-1".to_string();
        conversation.turn_activity.last_completed_turn_id = Some(turn_id.to_string());
        mark_core_turn_completed(&mut self.runtime, "thread-1", turn_id);

        self.runtime
            .app
            .tx
            .send(post_turn_evaluation_completed_message(
                "thread-1",
                turn_id,
                PostTurnEvaluationOutcome {
                    provenance: PostTurnEvaluationProvenance::new(turn_id.to_string()),
                    runtime_projection: planning_projection,
                    planning_repair_state: None,
                    runtime_notices: Vec::new(),
                    action: PostTurnContinuationAction::SkipAutoFollow {
                        reason: AutoFollowSkipReason::PlanningQueueDrained,
                    },
                    operator_alerts: vec![OperatorAlert::planning_queue_drained()],
                },
                Default::default(),
            ))
            .expect("drained parallel completion evaluation should enqueue");
    }

    fn send_dispatch_request_for_current_epoch(&mut self, trigger: ParallelModeAutomationTrigger) {
        let epoch_id = self
            .runtime
            .app()
            .parallel_mode_automation_epoch_id()
            .expect("automation epoch should be open");
        let workspace_directory = self.workspace_dir.clone();
        self.runtime
            .app_mut()
            .apply_parallel_mode_orchestrator_wake_request(workspace_directory, trigger, epoch_id);
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
            "expected at least {expected_launches} worker launch(es), got {}; projections: {:?}; notices: {:?}",
            self.worker_port.launch_count(),
            self.runtime_projections(),
            self.runtime_notices()
        );
    }

    fn poll_until_worker_streams_active(&mut self, expected_active_streams: usize) {
        for _ in 0..1500 {
            self.runtime.poll_background_messages();
            if self.worker_port.active_stream_count() >= expected_active_streams
                && self.worker_port.terminal_stream_count() == 0
            {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "expected {expected_active_streams} active held stream(s), got active={} terminal={}",
            self.worker_port.active_stream_count(),
            self.worker_port.terminal_stream_count()
        );
    }

    fn poll_until_worker_streams_terminal(&mut self, expected_terminal_streams: usize) {
        for _ in 0..750 {
            self.runtime.poll_background_messages();
            if self.worker_port.terminal_stream_count() >= expected_terminal_streams {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "expected {expected_terminal_streams} terminal held stream(s), got {}",
            self.worker_port.terminal_stream_count()
        );
    }

    fn poll_until_dispatch_idle(&mut self) {
        for _ in 0..750 {
            self.runtime.poll_background_messages();
            if !self
                .runtime
                .app()
                .parallel_mode_orchestrator_wake_in_flight()
            {
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

    fn poll_until_running_slot_leases(
        &mut self,
        expected_running_leases: usize,
    ) -> crate::application::port::outbound::planning_authority_port::PlanningAuthorityRuntimeProjectionSnapshot
    {
        for _ in 0..750 {
            self.runtime.poll_background_messages();
            let projections = self.runtime_projections();
            let running_count = projections
                .slot_leases
                .values()
                .filter(|lease| lease.state == ParallelModeSlotLeaseState::Running)
                .count();
            if running_count >= expected_running_leases {
                return projections;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "expected {expected_running_leases} running slot lease(s), got {:?}",
            self.runtime_projections().slot_leases
        );
    }

    fn wait_until_task_timestamp_can_clear_failed_start_block(&mut self, task_id: &str) {
        let projections = self.runtime_projections();
        let block_timestamp = projections
            .task_dispatch_blocks
            .iter()
            .filter(|block| {
                block.task_id == task_id
                    && block.reason
                        == ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges
            })
            .filter_map(|block| {
                DateTime::parse_from_rfc3339(block.blocked_at.trim())
                    .map(|timestamp| timestamp.timestamp_millis())
                    .ok()
            })
            .chain(
                projections
                    .session_details
                    .iter()
                    .filter(|detail| {
                        detail.task_id == task_id
                            && detail.state_label == "failed"
                            && detail.completion_state_label == "aborted"
                            && detail.latest_summary.contains(
                                "launch failed before the session reached the running state",
                            )
                    })
                    .filter_map(|detail| {
                        DateTime::parse_from_rfc3339(detail.updated_at.trim())
                            .map(|timestamp| timestamp.timestamp_millis())
                            .ok()
                    }),
            )
            .max()
            .expect("failed-start dispatch block should exist before retry update");
        for _ in 0..750 {
            self.runtime.poll_background_messages();
            let next_task_timestamp = Utc::now()
                .to_rfc3339_opts(SecondsFormat::Secs, true)
                .parse::<DateTime<Utc>>()
                .expect("generated task timestamp should parse")
                .timestamp_millis();
            if next_task_timestamp > block_timestamp {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("task timestamp did not advance past failed-start block for `{task_id}`");
    }

    fn runtime_task_authority(&self) -> crate::domain::planning::TaskAuthorityDocument {
        self.authority
            .load_task_authority_snapshot(&self.workspace_dir)
            .expect("task authority should load")
            .expect("task authority snapshot should exist")
            .task_authority
    }

    fn poll_until_idle_pool(&mut self) -> ParallelModePoolBoardSnapshot {
        let mut last_pool = None;
        for _ in 0..750 {
            self.runtime.poll_background_messages();
            let pool = self.runtime.app().parallel_mode_supervisor_snapshot().pool;
            if pool.slots.len() == FLOW_POOL_SIZE && pool.idle_slots == FLOW_POOL_SIZE {
                return pool;
            }
            last_pool = Some(pool);
            thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "pool did not become fully idle; last pool snapshot: {:?}",
            last_pool
        );
    }
}

fn git_stdout(repo_root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git command should spawn");
    assert!(
        output.status.success(),
        "git {:?} failed in {}\nstdout:\n{}\nstderr:\n{}",
        args,
        repo_root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn slot_path_from_board_label(workspace_dir: &str, worktree_label: &str) -> PathBuf {
    let path = PathBuf::from(worktree_label);
    if path.is_absolute() {
        return path;
    }
    PathBuf::from(workspace_dir)
        .parent()
        .expect("workspace should have parent directory")
        .join(path)
}

fn registered_worktree_paths(workspace_dir: &str) -> Vec<PathBuf> {
    git_stdout(
        Path::new(workspace_dir),
        &["worktree", "list", "--porcelain"],
    )
    .lines()
    .filter_map(|line| line.strip_prefix("worktree "))
    .map(PathBuf::from)
    .collect()
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
fn parallel_entry_dispatches_ready_queue_without_main_turn() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-parallel-entry");
    harness.committed_ready_task("verify parallel entry dispatches ready queue");

    harness.enter_parallel();

    harness.poll_until_worker_launches(1);
    let final_status = harness.poll_until_status_contains("auto dispatched 1 worker(s)");
    assert!(
        final_status.contains("dispatch refreshed"),
        "parallel entry should report dispatch refresh, got `{final_status}`"
    );
    assert_eq!(
        harness.worker_port.launch_count(),
        1,
        "bare :parallel entry should launch one isolated worker for the ready queue"
    );
    assert!(
        harness
            .runtime
            .app()
            .parallel_mode_automation_epoch_id()
            .is_some()
    );
}

#[test]
fn parallel_command_from_initial_screen_enables_ready_mode() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-parallel-initial-screen");

    harness.enter_parallel();

    let final_status = harness.poll_until_status_contains("control tower ready");
    assert!(harness.runtime.app().parallel_mode_enabled());
    assert!(final_status.contains("parallel mode: on"));
}

#[test]
fn parallel_entry_initializes_every_pool_slot_as_idle_worktree() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-pool-init");

    harness.enter_parallel();
    harness.poll_until_status_contains("control tower ready");

    let pool = harness.poll_until_idle_pool();
    assert_eq!(pool.configured_size, FLOW_POOL_SIZE);
    assert_eq!(pool.slots.len(), FLOW_POOL_SIZE);
    assert_eq!(pool.idle_slots, FLOW_POOL_SIZE);
    assert_eq!(pool.leased_slots, 0);
    assert_eq!(pool.running_slots, 0);
    assert_eq!(pool.blocked_slots, 0);
    assert_eq!(pool.missing_slots, 0);
    assert_eq!(pool.unavailable_slots, 0);
    assert!(
        harness.runtime_projections().slot_leases.is_empty(),
        "bare :parallel entry should not create active leases"
    );

    let worktree_paths = registered_worktree_paths(&harness.workspace_dir);
    for slot in pool.slots {
        assert_eq!(slot.state, ParallelModePoolSlotState::Idle);
        assert_eq!(slot.owner_label, "idle baseline");
        let slot_path = slot_path_from_board_label(&harness.workspace_dir, &slot.worktree_label);
        assert!(
            slot_path.is_dir(),
            "pool slot `{}` should exist at {}",
            slot.slot_id,
            slot_path.display()
        );
        assert!(
            worktree_paths.iter().any(|path| path == &slot_path),
            "pool slot `{}` should be registered as a git worktree",
            slot.slot_id
        );
        assert_eq!(
            git_stdout(&slot_path, &["status", "--porcelain=v1"]),
            "",
            "pool slot `{}` should be clean",
            slot.slot_id
        );
    }
}

#[test]
fn parallel_entry_leaves_idle_slots_detached_at_pool_baseline_head() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-pool-head");

    harness.enter_parallel();
    harness.poll_until_status_contains("control tower ready");

    let baseline_head = git_stdout(
        Path::new(&harness.workspace_dir),
        &["rev-parse", FLOW_POOL_BASELINE_BRANCH],
    );
    for slot in harness.poll_until_idle_pool().slots {
        let slot_path = slot_path_from_board_label(&harness.workspace_dir, &slot.worktree_label);
        assert_eq!(
            git_stdout(&slot_path, &["rev-parse", "--abbrev-ref", "HEAD"]),
            "HEAD",
            "idle pool slot `{}` should stay detached",
            slot.slot_id
        );
        assert_eq!(
            git_stdout(&slot_path, &["rev-parse", "HEAD"]),
            baseline_head,
            "idle pool slot `{}` should point at the pool baseline head",
            slot.slot_id
        );
        assert_eq!(
            slot.branch_name,
            format!("{FLOW_POOL_BASELINE_BRANCH} (detached)")
        );
    }
}

#[test]
fn parallel_entry_without_prompt_dispatches_ready_tasks_but_keeps_authority_statuses() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-no-prompt-task-status");
    let first = harness.committed_ready_task("ready task should remain ready after bare parallel");
    let second = harness.committed_ready_task("second ready task should also remain ready");
    let _release_guard = harness.worker_port.hold_worker_streams();

    harness.enter_parallel();
    harness.poll_until_worker_streams_active(2);

    let task_authority = harness.runtime_task_authority();
    let task_statuses = task_authority
        .tasks
        .iter()
        .map(|task| (task.id.as_str(), task.status))
        .collect::<Vec<_>>();
    assert!(
        task_statuses
            .iter()
            .all(|(_, status)| *status != TaskStatus::InProgress),
        "bare :parallel entry should not mark any task in progress: {:?}",
        task_statuses
    );
    for task_id in [first.committed_task_id, second.committed_task_id] {
        let task = task_authority
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .expect("committed task should remain in authority");
        assert_eq!(task.status, TaskStatus::Ready);
    }
    assert_eq!(harness.worker_port.launch_count(), 2);
    drop(_release_guard);
    harness.poll_until_worker_streams_terminal(2);
}

#[test]
fn repeated_parallel_entry_while_enabled_does_not_duplicate_worker_for_same_ready_task() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-parallel-entry-duplicate-guard");
    harness.committed_ready_task("same ready task should not launch twice on repeated parallel");
    let _release_guard = harness.worker_port.hold_worker_streams();

    harness.enter_parallel();
    harness.poll_until_worker_streams_active(1);

    harness.enter_parallel();
    for _ in 0..25 {
        harness.runtime.poll_background_messages();
        thread::sleep(Duration::from_millis(20));
    }

    assert_eq!(
        harness.worker_port.launch_count(),
        1,
        "repeating :parallel while enabled must refresh state without dispatching the same task twice"
    );
    drop(_release_guard);
    harness.poll_until_worker_streams_terminal(1);
}

#[test]
fn parallel_reentry_while_enabled_refreshes_without_reset_or_dispatch() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-parallel-reentry");
    harness.committed_ready_task("keep existing leased slot during reentry");
    let lease = harness
        .parallel_mode_service
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
    harness
        .runtime
        .app_mut()
        .set_parallel_mode_enabled_for_test(true);
    harness
        .runtime
        .app_mut()
        .set_parallel_mode_automation_epoch_for_test(1);

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
    harness
        .runtime
        .app_mut()
        .set_parallel_mode_enabled_for_test(true);
    harness.seed_ready_parallel_mode_projections();

    harness.send_post_turn_auto_prompt("turn-1");
    harness.poll_until_worker_launches(1);

    assert!(
        harness
            .runtime
            .app()
            .parallel_mode_automation_epoch_id()
            .is_some()
    );
    assert_eq!(
        harness
            .runtime
            .app()
            .last_parallel_mode_automation_trigger(),
        Some(ParallelModeAutomationTrigger::MainTurnPostEvaluation)
    );
    assert_eq!(harness.worker_port.launch_count(), 1);
    let launch_request = harness
        .worker_port
        .requests()
        .into_iter()
        .next()
        .expect("one worker request should be captured");
    assert_eq!(launch_request.service_name, "akra-parallel-worker");
    assert!(
        launch_request
            .developer_instructions
            .contains("Execute only the queued-task handoff"),
        "worker developer instructions should preserve the sub-session scope boundary: {}",
        launch_request.developer_instructions
    );
    assert!(
        launch_request
            .developer_instructions
            .contains("Do not push, open pull requests, merge"),
        "worker developer instructions should keep distributor delivery out of the worker lane: {}",
        launch_request.developer_instructions
    );
    assert!(
        launch_request
            .prompt
            .starts_with("# akra-sub-session-turn\n")
    );
    assert!(launch_request.prompt.contains("[queued-task-handoff]"));
    assert!(launch_request.prompt.contains("[delivery-boundary]"));
    let ConversationState::Ready(conversation) = &harness.runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert!(
        !conversation
            .status_text
            .contains("queued auto-follow with mode flow"),
        "parallel mode should suppress the main-session auto-follow submit"
    );
}

#[test]
fn post_turn_dispatch_launches_all_idle_slots_concurrently() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-post-turn-concurrent-dispatch");
    let committed_task_ids = [
        "dispatch parallel worker one",
        "dispatch parallel worker two",
        "dispatch parallel worker three",
        "dispatch parallel worker four",
    ]
    .into_iter()
    .map(|prompt| harness.committed_ready_task(prompt).committed_task_id)
    .collect::<Vec<_>>();
    let _release_guard = harness.worker_port.hold_worker_streams();
    harness
        .runtime
        .app_mut()
        .set_parallel_mode_enabled_for_test(true);
    harness.seed_ready_parallel_mode_projections();

    harness.send_post_turn_auto_prompt("turn-1");
    harness.poll_until_worker_streams_active(FLOW_POOL_SIZE);
    let status = harness.poll_until_status_contains("auto dispatched 3 worker(s)");
    let projections = harness.poll_until_running_slot_leases(FLOW_POOL_SIZE);

    assert!(
        status.contains("dispatch refreshed"),
        "dispatch status should report the parallel launch outcome, got `{status}`"
    );
    assert_eq!(harness.worker_port.launch_count(), FLOW_POOL_SIZE);
    assert_eq!(harness.worker_port.active_stream_count(), FLOW_POOL_SIZE);
    assert_eq!(
        harness.worker_port.terminal_stream_count(),
        0,
        "all worker streams should still be active while concurrency is observed"
    );

    let requests = harness.worker_port.requests();
    assert_eq!(requests.len(), FLOW_POOL_SIZE);
    let request_cwds = requests
        .iter()
        .map(|request| request.cwd.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        request_cwds.len(),
        FLOW_POOL_SIZE,
        "parallel workers should launch in distinct slot worktrees: {requests:?}"
    );
    assert!(
        request_cwds.iter().all(|cwd| cwd.contains("slot-")),
        "parallel workers should launch inside pool slots: {request_cwds:?}"
    );

    let running_leases = projections
        .slot_leases
        .values()
        .filter(|lease| lease.state == ParallelModeSlotLeaseState::Running)
        .collect::<Vec<_>>();
    assert_eq!(running_leases.len(), FLOW_POOL_SIZE);
    let running_task_ids = running_leases
        .iter()
        .map(|lease| lease.task_id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        running_task_ids.len(),
        FLOW_POOL_SIZE,
        "running leases should belong to distinct tasks: {running_leases:?}"
    );
    let dispatched_committed_task_count = committed_task_ids
        .iter()
        .filter(|task_id| running_task_ids.contains(task_id.as_str()))
        .count();
    assert_eq!(dispatched_committed_task_count, FLOW_POOL_SIZE);
    assert_eq!(
        committed_task_ids.len() - dispatched_committed_task_count,
        1,
        "one ready task should remain undispatched because only three pool slots are idle"
    );

    harness.worker_port.release_all_held_streams();
    harness.poll_until_worker_streams_terminal(FLOW_POOL_SIZE);
}

#[test]
fn parallel_completion_with_ready_queue_head_dispatches_next_worker() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-parallel-completion-dispatch");
    harness.committed_ready_task("dispatch after parallel completion leaves ready head");
    harness
        .runtime
        .app_mut()
        .set_parallel_mode_enabled_for_test(true);
    harness.seed_ready_parallel_mode_projections();
    harness
        .runtime
        .app_mut()
        .set_parallel_mode_automation_epoch_for_test(1);

    harness.send_parallel_completion_with_ready_queue_head("parallel-turn-1");
    harness.poll_until_worker_launches(1);

    let requests = harness.worker_port.requests();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].cwd.contains("slot-"),
        "ready queue head should dispatch into a pool slot: {requests:?}"
    );
    let projections = harness.runtime_projections();
    assert_eq!(projections.dispatch_commands.len(), 1);
    assert_eq!(
        projections.dispatch_commands[0].state,
        ParallelModeDispatchCommandState::Completed
    );
}

#[test]
fn parallel_completion_with_drained_queue_alerts_without_dispatching() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-parallel-completion-drained");
    harness
        .runtime
        .app_mut()
        .set_parallel_mode_enabled_for_test(true);
    harness.seed_ready_parallel_mode_projections();
    harness
        .runtime
        .app_mut()
        .set_parallel_mode_automation_epoch_for_test(1);

    harness.send_parallel_completion_with_drained_queue("parallel-turn-drained");
    harness.runtime.poll_background_messages();

    let ConversationState::Ready(conversation) = &harness.runtime.app().conversation_state else {
        panic!("expected ready conversation state");
    };
    assert!(
        conversation
            .status_text
            .contains("all planning tasks complete")
    );
    assert!(
        conversation
            .messages
            .iter()
            .any(|message| message.text.contains("ALL PLANNING TASKS COMPLETE"))
    );
    assert!(
        conversation
            .runtime_notices
            .iter()
            .any(|notice| notice.contains("All planning tasks complete"))
    );

    for _ in 0..10 {
        harness.runtime.poll_background_messages();
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(harness.worker_port.requests().len(), 0);
    assert!(harness.runtime_projections().dispatch_commands.is_empty());
}

#[test]
fn pending_durable_dispatch_command_polls_without_visible_activity_pulse() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-pending-dispatch-command-poll");
    let commit = harness.committed_ready_task("pending durable command should dispatch");
    harness
        .runtime
        .app_mut()
        .set_parallel_mode_enabled_for_test(true);
    harness.seed_ready_parallel_mode_projections();
    harness
        .runtime
        .app_mut()
        .set_parallel_mode_automation_epoch_for_test(1);
    assert!(!harness.runtime.app().parallel_mode_activity_pulse_visible());

    let command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
        Some(commit.committed_task_id),
        Some(1),
        Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
    );
    harness
        .authority
        .enqueue_runtime_dispatch_command(&harness.workspace_dir, &command)
        .expect("pending durable command should persist");

    harness.poll_until_worker_launches(1);
    harness.poll_until_dispatch_idle();

    let projections = harness.runtime_projections();
    assert_eq!(projections.dispatch_commands.len(), 1);
    assert_eq!(
        projections.dispatch_commands[0].state,
        ParallelModeDispatchCommandState::Completed
    );
}

#[test]
fn dispatch_requests_during_entry_loading_coalesce_until_ready() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-dispatch-coalesce");
    harness.committed_ready_task("coalesce dispatch while supervisor loads");
    harness
        .runtime
        .app_mut()
        .set_parallel_mode_enabled_for_test(true);
    let (refresh_epoch_id, refresh_effect_id) = harness
        .runtime
        .app_mut()
        .mark_parallel_mode_supervisor_refresh_in_flight_for_test();
    harness.seed_loading_parallel_mode_supervisor();

    harness.send_post_turn_auto_prompt("turn-1");
    harness.runtime.poll_background_messages();
    assert_eq!(harness.worker_port.launch_count(), 0);
    let projections = harness.runtime_projections();
    assert_eq!(projections.dispatch_commands.len(), 1);
    assert_eq!(
        projections.dispatch_commands[0].state,
        ParallelModeDispatchCommandState::Pending
    );

    let epoch_id = harness
        .runtime
        .app()
        .parallel_mode_automation_epoch_id()
        .expect("parallel mode should have an epoch");
    harness
        .runtime
        .app_mut()
        .apply_parallel_mode_orchestrator_wake_request(
            harness.workspace_dir.clone(),
            ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            epoch_id,
        );
    let projections = harness.runtime_projections();
    assert_eq!(projections.dispatch_commands.len(), 1);
    assert_eq!(
        projections.dispatch_commands[0].state,
        ParallelModeDispatchCommandState::Pending
    );

    harness
        .runtime
        .app
        .tx
        .send(BackgroundMessage::ParallelModeControlPlaneEvent(
            ParallelModeControlPlaneBackgroundEvent::SupervisorSnapshotRefreshed {
                workspace_directory: harness.workspace_dir.clone(),
                epoch_id: refresh_epoch_id,
                effect_id: refresh_effect_id,
                supervisor_snapshot: Box::new(ready_parallel_mode_supervisor_snapshot(
                    &harness.workspace_dir,
                )),
                orchestrator_tick_signature: None,
            },
        ))
        .expect("ready supervisor refresh should enqueue");
    harness.runtime.poll_background_messages();
    harness.poll_until_worker_launches(1);
    harness.poll_until_dispatch_idle();

    assert_eq!(harness.worker_port.launch_count(), 1);
}

#[test]
fn live_running_slot_is_preserved_during_off_to_on_pool_reset() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-live-reset-block");
    let lease = harness
        .parallel_mode_service
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
        .parallel_mode_service
        .mark_slot_running(&harness.workspace_dir, &lease.slot_id, "agent-running")
        .expect("slot should be marked running");
    fs::write(
        PathBuf::from(&running_lease.worktree_path).join("keep-running.tmp"),
        "running evidence\n",
    )
    .expect("running evidence should be written");

    harness
        .runtime
        .app_mut()
        .set_parallel_mode_enabled_for_test(true);
    harness
        .runtime
        .app_mut()
        .set_parallel_mode_initial_pool_reset_completed_for_test(true);
    harness.turn_parallel_off();
    harness.enter_parallel();

    let final_status = harness.poll_until_status_contains("preserved 1 live slot");
    assert!(
        final_status.contains("reset 2 pool slot worktree"),
        "reset should preserve live slot while resetting idle slots, got `{final_status}`"
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
        .parallel_mode_service
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
    let pool_root = PathBuf::from(&stale_lease.worktree_path)
        .parent()
        .expect("stale lease worktree should have pool parent")
        .to_path_buf();
    harness.remove_idle_pool_worktrees_except(&stale_lease.slot_id, &pool_root);
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
    harness
        .runtime
        .app_mut()
        .set_parallel_mode_enabled_for_test(true);
    harness.turn_parallel_off();

    harness
        .runtime
        .app
        .tx
        .send(BackgroundMessage::ParallelModeControlPlaneEvent(
            ParallelModeControlPlaneBackgroundEvent::Entered {
                workspace_directory: harness.workspace_dir.clone(),
                epoch_id: 1,
                effect_id: ParallelModeControlPlaneEffectId {
                    sequence: 99,
                    kind: ParallelModeControlPlaneEffectKind::EnterParallelMode,
                },
                mode_was_enabled: false,
                readiness_snapshot: ready_parallel_mode_readiness_snapshot(&harness.workspace_dir),
                supervisor_snapshot: Box::new(ready_parallel_mode_supervisor_snapshot(
                    &harness.workspace_dir,
                )),
                status_text: "parallel mode: on / readiness: ready / control tower ready"
                    .to_string(),
                initial_pool_reset_completed: false,
                has_actionable_queue_head: false,
                orchestrator_tick_signature: None,
            },
        ))
        .expect("late enter result should enqueue");
    harness.runtime.poll_background_messages();

    assert!(
        !harness.runtime.app().parallel_mode_enabled(),
        "late background enter result must not re-enable parallel mode after :parallel off"
    );
}

#[test]
fn stale_worker_event_drops_before_ui_notice_or_dispatch_wake() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-stale-worker-event");
    harness.committed_ready_task("late worker completion should not wake dispatch");
    harness
        .runtime
        .app_mut()
        .set_parallel_mode_enabled_for_test(true);
    harness.seed_ready_parallel_mode_projections();
    harness
        .runtime
        .app_mut()
        .set_parallel_mode_automation_epoch_for_test(2);

    harness
        .runtime
        .app
        .tx
        .send(BackgroundMessage::ParallelModeControlPlaneEvent(
            ParallelModeControlPlaneBackgroundEvent::WorkerEvent {
                event: ParallelModeControlPlaneWorkerEvent::new(
                    harness.workspace_dir.clone(),
                    1,
                    "task-stale-worker",
                    "Stale Worker",
                    ParallelModeControlPlaneWorkerEventKind::Completed,
                    vec!["late completion should be dropped".to_string()],
                ),
                has_actionable_queue_head: true,
            },
        ))
        .expect("stale worker event should enqueue");
    harness.runtime.poll_background_messages();

    assert_eq!(harness.worker_port.launch_count(), 0);
    assert!(
        !harness
            .runtime_notices()
            .iter()
            .any(|notice| notice.contains("late completion should be dropped")),
        "stale worker event notice must not leak into the visible runtime"
    );
    assert!(
        !harness
            .runtime
            .app()
            .parallel_mode_supervisor_refresh_in_flight()
    );
    assert!(
        !harness
            .runtime
            .app()
            .parallel_mode_orchestrator_wake_in_flight()
    );
}

#[test]
fn worker_launch_failure_blocks_task_until_task_update_then_retries() {
    let _guard = flow_test_guard();
    let mut harness = NativeFlowHarness::new("flow-worker-failure-retry");
    let commit = harness.committed_ready_task("retry worker after failed launch");
    harness.worker_port.set_fail_launch(true);
    harness
        .runtime
        .app_mut()
        .set_parallel_mode_enabled_for_test(true);
    harness.seed_ready_parallel_mode_projections();

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

    harness.wait_until_task_timestamp_can_clear_failed_start_block(&commit.committed_task_id);
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
