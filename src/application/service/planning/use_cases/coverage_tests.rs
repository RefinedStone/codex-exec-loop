use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};

use super::*;
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::application::port::outbound::planning_authority_port::NoopPlanningAuthorityPort;
use crate::application::port::outbound::planning_task_repository_port::{
    NoopPlanningTaskRepositoryPort, PlanningDirectionAuthorityCommit, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_worker_port::{
    NoopPlanningWorkerPort, PlanningWorkerPort, PlanningWorkerRequest, PlanningWorkerResponse,
};
use crate::application::service::planning::PlanningServices;
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, ManualPromptIntakeOutcome,
    ManualPromptIntakeRequest, PLANNING_FORMAT_VERSION, PriorityQueueTask, QueueIdleConfig,
    QueueIdlePolicy, RESULT_OUTPUT_FILE_PATH, TaskStatus,
};

struct ScriptedWorkerPort {
    actions: Mutex<VecDeque<ScriptedWorkerAction>>,
    requests: Mutex<Vec<PlanningWorkerRequest>>,
}

enum ScriptedWorkerAction {
    Message(&'static str),
    Error(&'static str),
}

impl ScriptedWorkerPort {
    fn new(actions: impl IntoIterator<Item = ScriptedWorkerAction>) -> Self {
        Self {
            actions: Mutex::new(actions.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<PlanningWorkerRequest> {
        self.requests
            .lock()
            .expect("worker request log should not be poisoned")
            .clone()
    }
}

impl PlanningWorkerPort for ScriptedWorkerPort {
    fn run_planning_session(
        &self,
        request: PlanningWorkerRequest,
    ) -> Result<PlanningWorkerResponse> {
        self.requests
            .lock()
            .expect("worker request log should not be poisoned")
            .push(request.clone());
        match self
            .actions
            .lock()
            .expect("worker actions should not be poisoned")
            .pop_front()
            .unwrap_or(ScriptedWorkerAction::Message("worker idle"))
        {
            ScriptedWorkerAction::Message(message) => Ok(PlanningWorkerResponse {
                operation: request.operation,
                thread_id: Some("worker-thread".to_string()),
                turn_id: Some("worker-turn".to_string()),
                final_agent_message: Some(message.to_string()),
                changed_planning_file_paths: Vec::new(),
            }),
            ScriptedWorkerAction::Error(message) => Err(anyhow!(message)),
        }
    }
}

struct TempPlanningWorkspace {
    path: PathBuf,
    path_text: String,
}

impl TempPlanningWorkspace {
    fn new(prefix: &str) -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{suffix}"));
        fs::create_dir_all(&path).expect("planning workspace should be created");
        let path_text = path.display().to_string();
        Self { path, path_text }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn path_str(&self) -> &str {
        &self.path_text
    }
}

impl Drop for TempPlanningWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn runtime_facade_wrappers_delegate_prompt_handoff_preview_status_and_manual_intake() {
    let workspace = TempPlanningWorkspace::new("planning-use-cases-runtime-wrappers");
    let planning = planning_services(
        Arc::new(NoopPlanningTaskRepositoryPort),
        Arc::new(NoopPlanningWorkerPort),
    );
    let projection = PlanningRuntimeProjection::ready(
        "planning prompt".to_string(),
        "queue summary".to_string(),
        Some(sample_queue_head()),
    );

    let manual = planning
        .runtime
        .build_manual_prompt("  run the operator request  ", &projection)
        .expect("non-empty manual prompt should render");
    assert!(manual.contains("run the operator request"));

    let intake = planning
        .runtime
        .prepare_manual_prompt_intake(ManualPromptIntakeRequest {
            workspace_directory: workspace.path_str().to_string(),
            raw_prompt: "   ".to_string(),
            legacy_source_turn_id: None,
            parent_thread_id: None,
            parent_turn_id: None,
        });
    assert!(matches!(intake, ManualPromptIntakeOutcome::Rejected { .. }));

    let queued = planning
        .runtime
        .build_queued_task_handoff(&projection)
        .expect("ready queue head should build a main handoff");
    assert_eq!(queued.task.task_id, "task-1");
    assert_eq!(
        planning
            .runtime
            .build_main_session_task_handoff(&sample_queue_head())
            .task
            .task_title,
        "Queue head"
    );
    assert_eq!(
        planning
            .runtime
            .build_sub_session_task_handoff(&sample_queue_head())
            .task
            .task_id,
        "task-1"
    );

    let profile = ParallelAgentProfile {
        agent_id: "agent-coverage".to_string(),
        display_name: "Coverage".to_string(),
        role: "Verifier".to_string(),
        persona_prompt: "Check edge cases.\nKeep notes compact.".to_string(),
        avatar_class: "Scribe".to_string(),
        capabilities: Vec::new(),
        enabled: true,
    };
    let profiled = planning
        .runtime
        .build_sub_session_task_handoff_with_agent_profile(&sample_queue_head(), &profile);
    assert!(profiled.developer_instructions.contains("Check edge cases"));

    let decision = planning
        .runtime
        .decide_auto_follow(PlanningRuntimeAutoFollowRequest {
            stop_keyword: "stop",
            last_message: "completed",
            projection: &projection,
        });
    assert!(matches!(
        decision,
        PlanningRuntimeAutoFollowDecision::QueuePrompt(_)
    ));

    let preview =
        planning
            .runtime
            .build_auto_follow_preview(PlanningRuntimeAutoFollowPreviewRequest {
                stop_keyword: "stop",
                last_message: Some("completed"),
                projection: &projection,
            });
    assert!(preview.rendered_prompt.contains("Queue head"));

    let summary = planning
        .runtime
        .build_summary_line(PlanningRuntimeSummaryLineRequest {
            projection: &projection,
            has_running_turn: false,
            is_repairing: false,
            repair_failure_summary: None,
            repair_attempt: None,
            has_notice: false,
            max_detail_len: 80,
            always_show: true,
        })
        .expect("always_show should render a planning summary");
    assert!(summary.contains("planning"));

    let status = planning.runtime.build_auto_follow_status_projection(
        PlanningRuntimeStatusProjectionRequest {
            projection: &projection,
            has_running_turn: false,
            is_repairing: false,
            repair_failure_summary: None,
            repair_attempt: None,
            max_detail_len: 80,
        },
    );
    assert!(status.planning_status_line.contains("planning"));

    let loaded = planning
        .runtime
        .load_runtime_projection_or_invalid(workspace.path_str());
    assert!(matches!(
        loaded.workspace_status(),
        PlanningRuntimeWorkspaceStatus::ReadyNoTask | PlanningRuntimeWorkspaceStatus::ReadyWithTask
    ));
}

#[test]
fn post_turn_auto_follow_maps_stop_policy_and_runtime_block_reasons() {
    let planning = planning_services(
        Arc::new(NoopPlanningTaskRepositoryPort),
        Arc::new(NoopPlanningWorkerPort),
    );
    let proposal_without_head = PlanningRuntimeProjection::ready_with_details(
        "prompt".to_string(),
        "proposal available".to_string(),
        Some("proposal candidate".to_string()),
        None,
    );

    assert_eq!(
        decide_post_turn_skip(&planning, &proposal_without_head),
        PlanningPostTurnAutoFollowSkipReason::PlanningQueueIdlePolicyStop
    );
    assert_eq!(
        decide_post_turn_skip(&planning, &PlanningRuntimeProjection::invalid("broken")),
        PlanningPostTurnAutoFollowSkipReason::PlanningBlocked
    );
    assert_eq!(
        decide_post_turn_skip(
            &planning,
            &proposal_without_head
                .with_queue_idle_policy(QueueIdlePolicy::ReviewAndEnqueue, Some("prompt".into())),
        ),
        PlanningPostTurnAutoFollowSkipReason::PlanningQueueHeadRequired
    );
    assert_eq!(
        decide_post_turn_skip(
            &planning,
            &PlanningRuntimeProjection::ready(
                "prompt".to_string(),
                "queue summary".to_string(),
                Some(sample_queue_head()),
            )
            .with_auto_follow_pause_reason("same queue head"),
        ),
        PlanningPostTurnAutoFollowSkipReason::PlanningRepeatedQueueHead
    );
}

#[test]
fn queue_refresh_preparation_reports_queue_idle_stop_and_missing_prompt() {
    let stop_workspace = TempPlanningWorkspace::new("planning-use-cases-queue-idle-stop");
    let stop_repo = Arc::new(NoopPlanningTaskRepositoryPort);
    seed_direction_catalog(
        &stop_repo,
        stop_workspace.path_str(),
        QueueIdlePolicy::Stop,
        "",
    );
    let planning = planning_services(stop_repo, Arc::new(NoopPlanningWorkerPort));
    let ready_no_task =
        PlanningRuntimeProjection::ready("prompt".to_string(), "queue empty".to_string(), None);

    let skipped = planning.worker.prepare_post_turn_queue_refresh(
        PlanningPostTurnQueueRefreshPreparationRequest {
            workspace_directory: stop_workspace.path_str(),
            parent_thread_id: Some("thread-1"),
            completed_turn_id: "turn-1",
            latest_user_message: Some("user"),
            latest_main_reply: Some("main reply"),
            previous_handoff_task: None,
            current_runtime_projection: &ready_no_task,
        },
    );
    let PlanningPostTurnQueueRefreshPreparation::Skipped(skipped) = skipped else {
        panic!("queue-idle stop policy should skip worker refresh");
    };
    assert_eq!(
        skipped.reason,
        PlanningPostTurnQueueRefreshSkipReason::QueueIdlePolicyStop
    );
    assert_eq!(skipped.reason.log_label(), "queue_idle_policy_stop");

    let missing_workspace = TempPlanningWorkspace::new("planning-use-cases-queue-idle-missing");
    let missing_repo = Arc::new(NoopPlanningTaskRepositoryPort);
    seed_direction_catalog(
        &missing_repo,
        missing_workspace.path_str(),
        QueueIdlePolicy::ReviewAndEnqueue,
        ".codex-exec-loop/planning/prompts/missing-queue-idle.md",
    );
    let missing = planning_services(missing_repo, Arc::new(NoopPlanningWorkerPort))
        .worker
        .prepare_post_turn_queue_refresh(PlanningPostTurnQueueRefreshPreparationRequest {
            workspace_directory: missing_workspace.path_str(),
            parent_thread_id: Some("thread-2"),
            completed_turn_id: "turn-2",
            latest_user_message: Some("user"),
            latest_main_reply: Some("main reply"),
            previous_handoff_task: None,
            current_runtime_projection: &ready_no_task,
        });
    let PlanningPostTurnQueueRefreshPreparation::Skipped(skipped) = missing else {
        panic!("missing queue-idle prompt should skip worker refresh");
    };
    assert_eq!(
        skipped.reason,
        PlanningPostTurnQueueRefreshSkipReason::QueueIdlePromptMissing
    );
    assert_eq!(skipped.reason.log_label(), "queue_idle_prompt_missing");
}

#[test]
fn reconcile_post_turn_reports_reconciliation_write_failures() {
    let workspace = TempPlanningWorkspace::new("planning-use-cases-reconcile-error");
    let planning_parent = workspace.path().join(".codex-exec-loop");
    fs::create_dir_all(&planning_parent).expect("planning parent should exist");
    fs::write(planning_parent.join("planning"), "not a directory")
        .expect("planning path fixture should be a file");
    let planning = planning_services(
        Arc::new(NoopPlanningTaskRepositoryPort),
        Arc::new(NoopPlanningWorkerPort),
    );
    let changed_paths = vec![RESULT_OUTPUT_FILE_PATH.to_string()];
    let capture = PlanningTurnExecutionSnapshotCapture::ready(
        workspace.path_str(),
        PlanningExecutionSnapshot {
            result_output_markdown: Some("pre-turn result".to_string()),
        },
    );

    let outcome = planning
        .runtime
        .reconcile_post_turn(PlanningPostTurnReconciliationRequest {
            workspace_directory: workspace.path_str(),
            completed_turn_id: "turn-1",
            changed_planning_file_paths: &changed_paths,
            execution_snapshot_capture: Some(&capture),
            current_runtime_projection: &PlanningRuntimeProjection::ready(
                "prompt".to_string(),
                "queue summary".to_string(),
                Some(sample_queue_head()),
            ),
        });

    assert!(
        outcome
            .reconciliation_result
            .auto_follow_block_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("planning reconciliation failed"))
    );
    assert!(outcome.runtime_projection.blocks_auto_follow());
}

#[test]
fn worker_refresh_repair_and_prepared_wrappers_delegate_to_orchestration() {
    let workspace = TempPlanningWorkspace::new("planning-use-cases-worker-wrappers");
    let worker = Arc::new(ScriptedWorkerPort::new([
        ScriptedWorkerAction::Message("refresh summary"),
        ScriptedWorkerAction::Message("prepared refresh summary"),
        ScriptedWorkerAction::Message("repair summary"),
    ]));
    let planning = planning_services(
        Arc::new(NoopPlanningTaskRepositoryPort),
        worker.clone() as Arc<dyn PlanningWorkerPort>,
    );
    let handoff = sample_handoff();

    let refresh = planning
        .worker
        .refresh_queue_from_reply(PlanningQueueRefreshRequest {
            workspace_directory: workspace.path_str(),
            parent_thread_id: Some("thread-refresh"),
            completed_turn_id: "turn-refresh",
            latest_user_message: Some("user"),
            latest_main_reply: "main reply",
            previous_handoff_task: Some(&handoff),
            mode: PlanningQueueRefreshMode::FromLatestMainReply,
        })
        .expect("direct queue refresh should run through worker orchestration");
    assert_eq!(refresh.worker_summary.as_deref(), Some("refresh summary"));

    let prepared = planning.worker.prepare_post_turn_queue_refresh(
        PlanningPostTurnQueueRefreshPreparationRequest {
            workspace_directory: workspace.path_str(),
            parent_thread_id: Some("thread-prepared"),
            completed_turn_id: "turn-prepared",
            latest_user_message: Some("user"),
            latest_main_reply: Some("prepared reply"),
            previous_handoff_task: Some(&handoff),
            current_runtime_projection: &PlanningRuntimeProjection::ready(
                "prompt".to_string(),
                "queue summary".to_string(),
                Some(sample_queue_head()),
            ),
        },
    );
    let PlanningPostTurnQueueRefreshPreparation::Ready(prepared) = prepared else {
        panic!("ready projection should prepare queue refresh");
    };
    let prepared_outcome = planning
        .worker
        .refresh_prepared_queue_from_reply(&prepared)
        .expect("prepared queue refresh should reuse stored request data");
    assert_eq!(
        prepared_outcome.worker_summary.as_deref(),
        Some("prepared refresh summary")
    );

    let repair_request = sample_repair_request();
    let ledger_request = PlanningLedgerRepairRequest {
        workspace_directory: workspace.path_str(),
        parent_thread_id: Some("thread-repair"),
        completed_turn_id: "turn-repair",
        repair_request: &repair_request,
        previous_handoff_task: Some(&handoff),
        attempt_number: 1,
        max_attempts: 2,
        retry_reason: Some(PlanningRepairRetryReason::TaskAuthorityUnchanged),
    };
    let repair_prompt = planning
        .worker
        .render_repair_task_authority_prompt(&ledger_request);
    assert!(repair_prompt.contains("validation failed"));

    let repair = planning
        .worker
        .repair_task_authority(ledger_request)
        .expect("direct repair should run through worker orchestration");
    assert_eq!(repair.worker_summary.as_deref(), Some("repair summary"));

    let requests = worker.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests.iter().all(|request| {
        request.workspace_directory == workspace.path_str() && !request.prompt.trim().is_empty()
    }));
}

#[test]
fn repair_post_turn_task_authority_reports_worker_failure_and_retry_exhaustion() {
    let failure_workspace = TempPlanningWorkspace::new("planning-use-cases-repair-failure");
    let failing = planning_services(
        Arc::new(NoopPlanningTaskRepositoryPort),
        Arc::new(ScriptedWorkerPort::new([ScriptedWorkerAction::Error(
            "worker unavailable",
        )])),
    );
    let repair_request = sample_repair_request();
    let failed = failing
        .worker
        .repair_post_turn_task_authority(PlanningPostTurnRepairRequest {
            workspace_directory: failure_workspace.path_str(),
            parent_thread_id: Some("thread-repair"),
            completed_turn_id: "turn-repair",
            repair_request: &repair_request,
            previous_handoff_task: Some(&sample_handoff()),
            max_attempts: 3,
        });
    assert!(!failed.resolved);
    assert_eq!(failed.attempts.len(), 1);
    assert!(matches!(
        &failed.attempts[0].result,
        PlanningPostTurnRepairAttemptResult::WorkerFailed { detail, error }
            if detail.contains("1/3") && error.contains("worker unavailable")
    ));

    let retry_workspace = TempPlanningWorkspace::new("planning-use-cases-repair-retry");
    let invalid_commands = r#"{"planning_task_commands":{"version":2,"commands":[]}}"#;
    let retrying = planning_services(
        Arc::new(NoopPlanningTaskRepositoryPort),
        Arc::new(ScriptedWorkerPort::new([
            ScriptedWorkerAction::Message(invalid_commands),
            ScriptedWorkerAction::Message(invalid_commands),
        ])),
    );
    let exhausted =
        retrying
            .worker
            .repair_post_turn_task_authority(PlanningPostTurnRepairRequest {
                workspace_directory: retry_workspace.path_str(),
                parent_thread_id: Some("thread-retry"),
                completed_turn_id: "turn-retry",
                repair_request: &repair_request,
                previous_handoff_task: Some(&sample_handoff()),
                max_attempts: 2,
            });

    assert!(!exhausted.resolved);
    assert_eq!(exhausted.attempts.len(), 2);
    assert!(matches!(
        &exhausted.attempts[0].result,
        PlanningPostTurnRepairAttemptResult::WorkerSucceeded {
            next_repair_request: Some(_),
            next_retry_reason: Some(PlanningRepairRetryReason::TaskAuthorityUnchanged),
            resolved: false,
            exhausted: false,
            ..
        }
    ));
    assert_eq!(
        exhausted.attempts[1].retry_reason,
        Some(PlanningRepairRetryReason::TaskAuthorityUnchanged)
    );
    assert!(matches!(
        &exhausted.attempts[1].result,
        PlanningPostTurnRepairAttemptResult::WorkerSucceeded {
            next_repair_request: Some(_),
            next_retry_reason: None,
            resolved: false,
            exhausted: true,
            ..
        }
    ));
}

#[test]
fn queue_refresh_finalization_reports_proposal_promotion_failure() {
    let workspace = TempPlanningWorkspace::new("planning-use-cases-promotion-failure");
    let planning = planning_services(
        Arc::new(NoopPlanningTaskRepositoryPort),
        Arc::new(NoopPlanningWorkerPort),
    );
    let proposal_projection = PlanningRuntimeProjection::ready_with_details(
        "prompt".to_string(),
        "queue empty".to_string(),
        Some("proposal candidate".to_string()),
        None,
    );

    let outcome = planning.worker.finalize_post_turn_queue_refresh(
        PlanningPostTurnQueueRefreshFinalizationRequest {
            workspace_directory: workspace.path_str(),
            previous_handoff_task: None,
            previous_runtime_projection: &proposal_projection,
            refreshed_runtime_projection: &proposal_projection,
            queue_idle_derivation: false,
        },
    );

    assert!(matches!(
        outcome.events.as_slice(),
        [PlanningPostTurnQueueRefreshFinalizationEvent::ProposalPromotionFailed {
            detail,
            runtime_projection
        }] if detail.contains("host proposal promotion failed")
            && runtime_projection.blocks_auto_follow()
    ));
    assert_eq!(
        outcome.runtime_projection.failure_reason(),
        Some(PLANNING_WORKER_REFRESH_FAILURE_BLOCK_REASON)
    );
}

#[test]
fn official_completion_failure_projection_and_repeat_detail_cover_empty_and_changed_edges() {
    let projection = PlanningRuntimeProjection::ready(
        "prompt".to_string(),
        "queue summary".to_string(),
        Some(sample_queue_head()),
    );
    let failure = official_completion_failure_projection(&projection, "   ");
    assert_eq!(
        failure.auto_follow_pause_reason(),
        Some(OFFICIAL_COMPLETION_REFRESH_FAILURE_BLOCK_REASON)
    );

    let previous = projection_with_signature(Some(7));
    let current = projection_with_signature(Some(7));
    let mut different_id = sample_handoff();
    different_id.task_id = "other-task".to_string();
    assert!(repeated_queue_head_detail(Some(&different_id), &previous, &current).is_none());

    let mut changed_title = sample_handoff();
    changed_title.task_title = "Changed title".to_string();
    assert!(repeated_queue_head_detail(Some(&changed_title), &previous, &current).is_none());
}

fn planning_services(
    repository: Arc<dyn PlanningTaskRepositoryPort>,
    worker: Arc<dyn PlanningWorkerPort>,
) -> PlanningServices {
    PlanningServices::from_ports(
        Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
        Arc::new(NoopPlanningAuthorityPort::default()),
        repository,
        worker,
    )
}

fn seed_direction_catalog(
    repository: &NoopPlanningTaskRepositoryPort,
    workspace_dir: &str,
    policy: QueueIdlePolicy,
    prompt_path: &str,
) {
    repository
        .clear_direction_authority_snapshot(workspace_dir)
        .expect("direction authority should clear before seeding");
    repository
        .clear_task_authority_snapshot(workspace_dir)
        .expect("task authority should clear before seeding");
    let directions = DirectionCatalogDocument {
        version: PLANNING_FORMAT_VERSION,
        queue_idle: QueueIdleConfig {
            policy,
            prompt_path: prompt_path.to_string(),
        },
        directions: vec![DirectionDefinition {
            id: "direction-1".to_string(),
            title: "Direction".to_string(),
            summary: "Direction summary".to_string(),
            success_criteria: vec!["Done".to_string()],
            scope_hints: Vec::new(),
            detail_doc_path: String::new(),
            state: DirectionState::Active,
        }],
    };
    repository
        .commit_direction_authority_snapshot(
            workspace_dir,
            PlanningDirectionAuthorityCommit {
                observed_planning_revision: None,
                directions: &directions,
            },
        )
        .expect("direction authority should seed");
}

fn decide_post_turn_skip(
    planning: &PlanningServices,
    projection: &PlanningRuntimeProjection,
) -> PlanningPostTurnAutoFollowSkipReason {
    match planning
        .runtime
        .decide_post_turn_auto_follow(PlanningPostTurnAutoFollowRequest {
            continuation_paused: false,
            can_queue_next: true,
            latest_agent_message: Some("completed"),
            stop_keyword: "stop",
            stop_keyword_matched: false,
            no_file_changes_stop_matched: false,
            runtime_projection: projection,
        }) {
        PlanningPostTurnAutoFollowDecision::Skip(reason) => reason,
        PlanningPostTurnAutoFollowDecision::QueuePrompt(_) => {
            panic!("expected post-turn auto-follow decision to skip")
        }
    }
}

fn sample_queue_head() -> PriorityQueueTask {
    PriorityQueueTask {
        rank: 1,
        task_id: "task-1".to_string(),
        direction_id: "direction-1".to_string(),
        direction_title: "Direction".to_string(),
        task_title: "Queue head".to_string(),
        status: TaskStatus::Ready,
        combined_priority: 80,
        updated_at: "2026-04-23T00:00:00Z".to_string(),
        rank_reasons: vec!["ready".to_string()],
    }
}

fn sample_handoff() -> PlanningTaskHandoff {
    PlanningTaskHandoff {
        task_id: "task-1".to_string(),
        task_title: "Queue head".to_string(),
        direction_id: "direction-1".to_string(),
        combined_priority: 80,
        updated_at: "2026-04-23T00:00:00Z".to_string(),
        status_label: "ready".to_string(),
    }
}

fn sample_repair_request() -> PlanningRepairRequest {
    PlanningRepairRequest {
        failure_summary: "validation failed".to_string(),
        validation_errors: vec!["task status is invalid".to_string()],
        direction_authority_json: "{}".to_string(),
        accepted_task_authority_json: "{}".to_string(),
        accepted_queue_projection_json: "{}".to_string(),
        rejected_task_authority_json: Some("{}".to_string()),
        rejected_archive_path: Some("rejected/task-authority.json".to_string()),
    }
}

fn projection_with_signature(signature: Option<u64>) -> PlanningRuntimeProjection {
    PlanningRuntimeProjection::ready(
        "prompt".to_string(),
        "queue summary".to_string(),
        Some(sample_queue_head()),
    )
    .with_test_signatures(None, signature)
}
