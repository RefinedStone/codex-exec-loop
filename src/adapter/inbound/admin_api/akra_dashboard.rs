use crate::application::port::outbound::parallel_mode_runtime_event_log_port::ParallelModeRuntimeEventLogRequest;
use crate::application::service::parallel_mode::ParallelModeService;
use crate::application::service::planning::PlanningServices;
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterEntry, ParallelModeDistributorQueueItem, ParallelModePoolSlotSnapshot,
    ParallelModePoolSlotState, ParallelModeQueueItemState, ParallelModeReadinessSnapshot,
    ParallelModeReadinessState, ParallelModeRuntimeEventEntry, ParallelModeSupervisorSnapshot,
};
use chrono::Utc;
use serde::Serialize;

const DASHBOARD_EVENT_LIMIT: usize = 20;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AkraAdminDashboardView {
    pub workspace: AkraWorkspaceView,
    pub kpis: AkraKpiView,
    pub pool: PoolBoardView,
    pub agents: AgentRosterView,
    pub selected_task: Option<SelectedTaskView>,
    pub distributor: DistributorView,
    pub events: Vec<RuntimeEventView>,
    pub metrics: GuildMetricsView,
    pub generated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AkraWorkspaceView {
    pub path: String,
    pub branch: Option<String>,
    pub mode: String,
    pub readiness: String,
    pub top_notice: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AkraKpiView {
    pub total_tasks: Option<usize>,
    pub success_rate: Option<f64>,
    pub today_throughput: Option<usize>,
    pub active_agents: usize,
    pub total_agents: usize,
    pub pool_configured_size: usize,
    pub pool_idle: usize,
    pub pool_running: usize,
    pub pool_blocked: usize,
    pub queue_depth: usize,
    pub distributor_state: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PoolBoardView {
    pub configured_size: usize,
    pub reconcile_status: String,
    pub exhausted: bool,
    pub summary: PoolSummaryView,
    pub slots: Vec<PoolSlotView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PoolSummaryView {
    pub idle: usize,
    pub leased: usize,
    pub running: usize,
    pub cleanup: usize,
    pub blocked: usize,
    pub missing: usize,
    pub unavailable: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PoolSlotView {
    pub slot_id: String,
    pub state: String,
    pub label: String,
    pub branch_name: String,
    pub worktree_label: String,
    pub owner_label: String,
    pub owner_agent_id: Option<String>,
    pub task_id: Option<String>,
    pub note: String,
    pub severity: String,
    pub bubble_label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AgentRosterView {
    pub active_count: usize,
    pub empty_state: String,
    pub entries: Vec<AgentView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AgentView {
    pub agent_id: String,
    pub display_name: String,
    pub class_label: String,
    pub slot_id: String,
    pub task_title: String,
    pub branch_name: String,
    pub lifecycle_state: String,
    pub progress_label: String,
    pub duration_label: String,
    pub latest_summary: String,
    pub status: String,
    pub overload: bool,
    pub bubble_label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SelectedTaskView {
    pub task_id: String,
    pub task_title: String,
    pub agent_id: String,
    pub slot_id: String,
    pub branch_name: String,
    pub state: String,
    pub progress_percent: u8,
    pub validation_summary: String,
    pub latest_summary: String,
    pub updated_at: String,
    pub trail: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DistributorView {
    pub head_summary: String,
    pub note: String,
    pub queue_depth: usize,
    pub barrier_state: String,
    pub blocked_reason: Option<String>,
    pub integration_worktree_readiness: String,
    pub held_queue_count: usize,
    pub conflict_files: Vec<String>,
    pub queue_items: Vec<DistributorQueueItemView>,
    pub pipeline: Vec<DistributorPipelineStep>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DistributorQueueItemView {
    pub source_agent: String,
    pub task_title: String,
    pub queue_state: String,
    pub branch_name: String,
    pub commit_short_sha: String,
    pub integration_note: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DistributorPipelineStep {
    pub key: String,
    pub label: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RuntimeEventView {
    pub sequence: i64,
    pub event_kind: String,
    pub projection_kind: String,
    pub projection_key: String,
    pub observed_planning_revision: i64,
    pub summary: String,
    pub recorded_at: String,
    pub icon: String,
    pub severity: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GuildMetricsView {
    pub pool_utilization_percent: usize,
    pub test_success_rate: Option<f64>,
    pub average_queue_depth: Option<f64>,
    pub error_rate: Option<f64>,
    pub active_agent_count: usize,
    pub waiting_task_count: usize,
    pub blocked_slot_count: usize,
    pub badges: Vec<String>,
}

pub(super) fn build_akra_dashboard_view(
    workspace_dir: &str,
    planning: &PlanningServices,
    parallel_mode: &ParallelModeService,
) -> AkraAdminDashboardView {
    let planning_snapshot = planning
        .runtime
        .load_runtime_snapshot_or_invalid(workspace_dir);
    let readiness = parallel_mode.inspect_readiness(workspace_dir, &planning_snapshot);
    let supervisor = parallel_mode.build_supervisor_snapshot(workspace_dir, true, Some(&readiness));
    let events = parallel_mode.build_runtime_events_snapshot(
        workspace_dir,
        ParallelModeRuntimeEventLogRequest::recent(DASHBOARD_EVENT_LIMIT),
    );

    let pool = map_pool(&supervisor);
    let agents = map_agents(&supervisor);
    let selected_task = map_selected_task(&supervisor);
    let distributor = map_distributor(&supervisor);
    let events = events
        .entries
        .iter()
        .map(map_runtime_event)
        .collect::<Vec<_>>();
    let metrics = map_metrics(&pool, &agents, &distributor);
    let readiness_label = readiness_label(&readiness).to_string();

    AkraAdminDashboardView {
        workspace: AkraWorkspaceView {
            path: supervisor.workspace_path.clone(),
            branch: current_git_branch(workspace_dir),
            mode: "parallel".to_string(),
            readiness: readiness_label,
            top_notice: supervisor
                .top_notice
                .clone()
                .or_else(|| readiness.top_alert.clone()),
        },
        kpis: AkraKpiView {
            total_tasks: planning_snapshot
                .queue_projection()
                .map(|projection| projection.visible_tasks(usize::MAX).len()),
            success_rate: None,
            today_throughput: None,
            active_agents: agents.active_count,
            total_agents: agents.entries.len(),
            pool_configured_size: pool.configured_size,
            pool_idle: pool.summary.idle,
            pool_running: pool.summary.running,
            pool_blocked: pool.summary.blocked,
            queue_depth: distributor.queue_depth,
            distributor_state: distributor.barrier_state.clone(),
        },
        pool,
        agents,
        selected_task,
        distributor,
        events,
        metrics,
        generated_at: Utc::now().to_rfc3339(),
    }
}

fn map_pool(supervisor: &ParallelModeSupervisorSnapshot) -> PoolBoardView {
    let pool = &supervisor.pool;
    PoolBoardView {
        configured_size: pool.configured_size,
        reconcile_status: pool.reconcile_status.clone(),
        exhausted: pool.exhausted,
        summary: PoolSummaryView {
            idle: pool.idle_slots,
            leased: pool.leased_slots,
            running: pool.running_slots,
            cleanup: pool.awaiting_cleanup_slots,
            blocked: pool.blocked_slots,
            missing: pool.missing_slots,
            unavailable: pool.unavailable_slots,
        },
        slots: pool.slots.iter().map(map_pool_slot).collect(),
    }
}

fn map_pool_slot(slot: &ParallelModePoolSlotSnapshot) -> PoolSlotView {
    let (owner_agent_id, task_id) = parse_owner_label(&slot.owner_label);
    PoolSlotView {
        slot_id: slot.slot_id.clone(),
        state: slot.state.label().to_string(),
        label: pool_state_korean_label(slot.state).to_string(),
        branch_name: slot.branch_name.clone(),
        worktree_label: slot.worktree_label.clone(),
        owner_label: slot.owner_label.clone(),
        owner_agent_id,
        task_id,
        note: pool_slot_note(slot),
        severity: pool_state_severity(slot.state).to_string(),
        bubble_label: pool_state_bubble(slot.state).to_string(),
    }
}

fn map_agents(supervisor: &ParallelModeSupervisorSnapshot) -> AgentRosterView {
    AgentRosterView {
        active_count: supervisor.roster.active_count(),
        empty_state: supervisor.roster.empty_state.clone(),
        entries: supervisor
            .roster
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| map_agent(index, entry))
            .collect(),
    }
}

fn map_agent(index: usize, entry: &ParallelModeAgentRosterEntry) -> AgentView {
    let status = agent_status(entry.state_label.as_str());
    AgentView {
        agent_id: entry.agent_id.clone(),
        display_name: format!("A{:02}", index + 1),
        class_label: agent_class_label(index).to_string(),
        slot_id: entry.slot_id.clone(),
        task_title: entry.task_title.clone(),
        branch_name: entry.branch_name.clone(),
        lifecycle_state: entry.state_label.clone(),
        progress_label: progress_label(entry.state_label.as_str()),
        duration_label: entry.duration_label.clone(),
        latest_summary: entry.latest_summary.clone(),
        status: status.to_string(),
        overload: entry.duration_label.contains("h "),
        bubble_label: agent_bubble(entry.state_label.as_str()).to_string(),
    }
}

fn map_selected_task(supervisor: &ParallelModeSupervisorSnapshot) -> Option<SelectedTaskView> {
    let session = supervisor.detail.session.as_ref()?;
    Some(SelectedTaskView {
        task_id: session.task_id.clone(),
        task_title: session.task_title.clone(),
        agent_id: session.agent_id.clone(),
        slot_id: session.slot_id.clone(),
        branch_name: session.branch_name.clone(),
        state: session.state_label.clone(),
        progress_percent: progress_percent(session.state_label.as_str()),
        validation_summary: session.validation_summary.clone(),
        latest_summary: session.latest_summary.clone(),
        updated_at: session.updated_at.clone(),
        trail: session
            .history
            .iter()
            .map(|entry| entry.state_label.clone())
            .collect(),
    })
}

fn map_distributor(supervisor: &ParallelModeSupervisorSnapshot) -> DistributorView {
    let distributor = &supervisor.distributor;
    let head_state = distributor
        .queue_items
        .first()
        .map(|item| item.queue_state)
        .unwrap_or(ParallelModeQueueItemState::Idle);
    DistributorView {
        head_summary: distributor.head_summary.clone(),
        note: distributor.note.clone(),
        queue_depth: distributor.queue_depth(),
        barrier_state: distributor.orchestrator_status.barrier_state.clone(),
        blocked_reason: distributor
            .head_blocked_detail
            .clone()
            .or_else(|| distributor.orchestrator_status.blocked_reason.clone()),
        integration_worktree_readiness: distributor
            .orchestrator_status
            .integration_worktree_readiness
            .clone(),
        held_queue_count: distributor.orchestrator_status.held_queue_count,
        conflict_files: distributor.orchestrator_status.conflict_files.clone(),
        queue_items: distributor.queue_items.iter().map(map_queue_item).collect(),
        pipeline: map_pipeline(head_state),
    }
}

fn map_queue_item(item: &ParallelModeDistributorQueueItem) -> DistributorQueueItemView {
    DistributorQueueItemView {
        source_agent: item.source_agent.clone(),
        task_title: item.task_title.clone(),
        queue_state: item.queue_state.label().to_string(),
        branch_name: item.branch_name.clone(),
        commit_short_sha: item.commit_short_sha.clone(),
        integration_note: item.integration_note.clone(),
    }
}

fn map_runtime_event(entry: &ParallelModeRuntimeEventEntry) -> RuntimeEventView {
    RuntimeEventView {
        sequence: entry.sequence,
        event_kind: entry.event_kind.clone(),
        projection_kind: entry.projection_kind.clone(),
        projection_key: entry.projection_key.clone(),
        observed_planning_revision: entry.observed_planning_revision,
        summary: entry.summary.clone(),
        recorded_at: entry.recorded_at.clone(),
        icon: event_icon(entry.event_kind.as_str()).to_string(),
        severity: event_severity(entry.event_kind.as_str()).to_string(),
    }
}

fn map_metrics(
    pool: &PoolBoardView,
    agents: &AgentRosterView,
    distributor: &DistributorView,
) -> GuildMetricsView {
    let occupied = pool.configured_size.saturating_sub(pool.summary.idle);
    let pool_utilization_percent = (occupied * 100)
        .checked_div(pool.configured_size)
        .unwrap_or(0);
    let mut badges = Vec::new();
    if pool.summary.blocked + pool.summary.missing + pool.summary.unavailable == 0 {
        badges.push("풀 관리자".to_string());
    }
    if distributor.barrier_state == "idle" {
        badges.push("분배 안정".to_string());
    }
    if pool_utilization_percent >= 90 {
        badges.push("과부하 경보".to_string());
    }
    if pool.summary.cleanup > 0 {
        badges.push("정리 필요".to_string());
    }
    if pool.summary.blocked + pool.summary.missing + pool.summary.unavailable > 0 {
        badges.push("복구 필요".to_string());
    }

    GuildMetricsView {
        pool_utilization_percent,
        test_success_rate: None,
        average_queue_depth: Some(distributor.queue_depth as f64),
        error_rate: None,
        active_agent_count: agents.active_count,
        waiting_task_count: distributor.queue_depth,
        blocked_slot_count: pool.summary.blocked,
        badges,
    }
}

fn map_pipeline(head_state: ParallelModeQueueItemState) -> Vec<DistributorPipelineStep> {
    let steps = [
        ("review", "검토", ParallelModeQueueItemState::Queued),
        (
            "gate_check",
            "게이트 체크",
            ParallelModeQueueItemState::Pushing,
        ),
        ("push", "Push", ParallelModeQueueItemState::Pushing),
        ("pr", "PR", ParallelModeQueueItemState::PrPending),
        ("merge", "Merge", ParallelModeQueueItemState::MergePending),
        ("cleanup", "정리", ParallelModeQueueItemState::Cleaning),
        ("done", "완료", ParallelModeQueueItemState::Done),
    ];
    let head_rank = queue_state_rank(head_state);
    steps
        .iter()
        .map(|(key, label, step_state)| {
            let state = match head_state {
                ParallelModeQueueItemState::Blocked => "blocked",
                ParallelModeQueueItemState::Failed => "failed",
                _ if head_state == *step_state => "active",
                _ if queue_state_rank(*step_state) < head_rank => "done",
                _ => "waiting",
            };
            DistributorPipelineStep {
                key: (*key).to_string(),
                label: (*label).to_string(),
                state: state.to_string(),
            }
        })
        .collect()
}

fn queue_state_rank(state: ParallelModeQueueItemState) -> u8 {
    match state {
        ParallelModeQueueItemState::Idle => 0,
        ParallelModeQueueItemState::Queued => 1,
        ParallelModeQueueItemState::Pushing => 2,
        ParallelModeQueueItemState::PrPending => 3,
        ParallelModeQueueItemState::MergePending => 4,
        ParallelModeQueueItemState::Integrating => 5,
        ParallelModeQueueItemState::Cleaning => 6,
        ParallelModeQueueItemState::Done => 7,
        ParallelModeQueueItemState::Blocked | ParallelModeQueueItemState::Failed => 8,
    }
}

fn readiness_label(readiness: &ParallelModeReadinessSnapshot) -> &'static str {
    match readiness.readiness {
        ParallelModeReadinessState::Ready => "ready",
        ParallelModeReadinessState::Degraded => "degraded",
        ParallelModeReadinessState::Repairing => "degraded",
        ParallelModeReadinessState::Blocked => "blocked",
    }
}

fn pool_state_korean_label(state: ParallelModePoolSlotState) -> &'static str {
    match state {
        ParallelModePoolSlotState::Idle => "여유",
        ParallelModePoolSlotState::Leased => "예약됨",
        ParallelModePoolSlotState::Running => "작업중",
        ParallelModePoolSlotState::AwaitingCleanup => "정리중",
        ParallelModePoolSlotState::Blocked => "차단됨",
        ParallelModePoolSlotState::Missing => "사라짐",
        ParallelModePoolSlotState::Unavailable => "사용 불가",
    }
}

fn pool_state_bubble(state: ParallelModePoolSlotState) -> &'static str {
    match state {
        ParallelModePoolSlotState::Idle => "여유",
        ParallelModePoolSlotState::Leased => "점유됨",
        ParallelModePoolSlotState::Running => "작업중",
        ParallelModePoolSlotState::AwaitingCleanup => "정리중",
        ParallelModePoolSlotState::Blocked => "막힘",
        ParallelModePoolSlotState::Missing => "확인 필요",
        ParallelModePoolSlotState::Unavailable => "잠금",
    }
}

fn pool_state_severity(state: ParallelModePoolSlotState) -> &'static str {
    match state {
        ParallelModePoolSlotState::Idle | ParallelModePoolSlotState::Running => "normal",
        ParallelModePoolSlotState::Leased => "info",
        ParallelModePoolSlotState::AwaitingCleanup => "warning",
        ParallelModePoolSlotState::Blocked => "danger",
        ParallelModePoolSlotState::Missing | ParallelModePoolSlotState::Unavailable => "muted",
    }
}

fn pool_slot_note(slot: &ParallelModePoolSlotSnapshot) -> String {
    if slot.owner_label.trim().is_empty() || slot.owner_label == "-" {
        return slot.worktree_label.clone();
    }
    format!("{} / {}", slot.owner_label, slot.worktree_label)
}

fn parse_owner_label(owner_label: &str) -> (Option<String>, Option<String>) {
    let mut parts = owner_label.split('/').map(str::trim);
    let agent = parts
        .next()
        .filter(|value| !value.is_empty() && *value != "-");
    let task = parts
        .next()
        .filter(|value| !value.is_empty() && *value != "-");
    (agent.map(str::to_string), task.map(str::to_string))
}

fn agent_status(state_label: &str) -> &'static str {
    match state_label {
        "failed" | "official_refresh_recovery_needed" => "blocked",
        "cleanup_pending" | "integrating" | "cleaning" => "cleanup",
        "reported_complete" | "commit_ready" | "merge_queued" | "pushing" | "pr_pending"
        | "merge_pending" => "running",
        "assigned" | "starting" | "running" => "running",
        _ => "idle",
    }
}

fn agent_bubble(state_label: &str) -> &'static str {
    match state_label {
        "running" | "starting" | "assigned" => "작업중",
        "reported_complete" => "보고 완료",
        "commit_ready" => "공식 승인",
        "failed" => "실패",
        "official_refresh_recovery_needed" => "차단됨",
        "cleanup_pending" => "정리중",
        _ => "대기중",
    }
}

fn agent_class_label(index: usize) -> &'static str {
    match index % 6 {
        0 => "Artificer",
        1 => "Scribe",
        2 => "Ranger",
        3 => "Guardian",
        4 => "Seer",
        _ => "Runner",
    }
}

fn progress_label(state_label: &str) -> String {
    format!("{}%", progress_percent(state_label))
}

fn progress_percent(state_label: &str) -> u8 {
    match state_label {
        "assigned" => 15,
        "starting" => 25,
        "running" => 45,
        "reported_complete" => 65,
        "ledger_refreshing" | "commit_ready" => 75,
        "merge_queued" | "pushing" | "pr_pending" | "merge_pending" | "integrating" => 88,
        "cleanup_pending" | "done" => 100,
        "failed" | "official_refresh_recovery_needed" => 35,
        _ => 10,
    }
}

fn event_icon(event_kind: &str) -> &'static str {
    match event_kind {
        "slot_lease_upsert" => "seat",
        "session_detail_upsert" => "agent",
        "distributor_queue" => "route",
        "worktree_status" => "git",
        "cleanup_completed" => "clean",
        _ => "event",
    }
}

fn event_severity(event_kind: &str) -> &'static str {
    if event_kind.contains("failed") || event_kind.contains("blocked") {
        "danger"
    } else if event_kind.contains("cleanup") {
        "success"
    } else if event_kind.contains("status") {
        "warning"
    } else {
        "info"
    }
}

fn current_git_branch(workspace_dir: &str) -> Option<String> {
    std::process::Command::new("git")
        .args(["-C", workspace_dir, "branch", "--show-current"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|branch| branch.trim().to_string())
        .filter(|branch| !branch.is_empty())
}
