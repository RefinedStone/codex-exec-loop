use crate::application::port::inbound::parallel_agent_profile_port::{
    ParallelAgentProfileConfig, ParallelAgentProfilePort,
};
use crate::application::port::inbound::parallel_mode_admin_port::ParallelModeAdminPort;
use crate::application::port::inbound::planning_admin_port::PlanningAdminPort;
use crate::application::port::inbound::pr_validation_query_port::{
    PrValidationAdminPhase, PrValidationAdminRecordSummary, PrValidationBoardRequest,
    PrValidationBoardSnapshot, PrValidationQueryPort,
};
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterEntry, ParallelModeDistributorQueueItem, ParallelModePoolSlotSnapshot,
    ParallelModePoolSlotState, ParallelModeQueueItemState, ParallelModeReadinessSnapshot,
    ParallelModeReadinessState, ParallelModeRuntimeEventEntry,
    ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorSnapshot,
};
use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use std::collections::HashSet;

use super::admin_debug_dashboard::AdminDebugHarnessView;
use crate::git_subprocess;

const DASHBOARD_EVENT_LIMIT: usize = 20;
const ADMIN_RUNTIME_MODE_LABEL: &str = "controlled projection";
const STANDBY_CHARACTER_LIMIT: usize = 3;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AkraAdminDashboardView {
    pub debug_harness: AdminDebugHarnessView,
    pub workspace: AkraWorkspaceView,
    pub kpis: AkraKpiView,
    pub pool: PoolBoardView,
    pub agents: AgentRosterView,
    pub scene: GameSceneView,
    pub selected_task: Option<SelectedTaskView>,
    pub distributor: DistributorView,
    pub events: Vec<RuntimeEventView>,
    pub metrics: GuildMetricsView,
    pub campaign: CampaignView,
    pub event_feed: EventFeedView,
    pub validation: PrValidationBoardSnapshot,
    pub generated_at: String,
    pub generated_time_label: String,
    pub planning_revision: Option<i64>,
    pub planning_revision_label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AkraWorkspaceView {
    pub path: String,
    pub branch: Option<String>,
    pub mode: String,
    pub readiness: String,
    pub readiness_notice: String,
    pub blocked_action: String,
    pub purpose_label: String,
    pub gamification_policy: String,
    pub domain_mapping_note: String,
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
    pub queue_depth_basis: String,
    pub metric_source_label: String,
    pub distributor_state: String,
    pub validation_verifying: usize,
    pub validation_failed: usize,
    pub validation_remediation_queued: usize,
    pub validation_stale: usize,
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
    pub display_slot_label: String,
    pub state: String,
    pub label: String,
    pub branch_name: String,
    pub worktree_label: String,
    pub owner_label: String,
    pub owner_agent_id: Option<String>,
    pub task_id: Option<String>,
    pub owner_session_key: Option<String>,
    pub lease_generation: Option<String>,
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
    pub role_label: String,
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
pub(super) struct GameSceneView {
    pub stations: Vec<GameStationView>,
    pub actors: Vec<GameActorView>,
    pub standby_profile_count: usize,
    pub standby_characters: Vec<GameStandbyCharacterView>,
    pub diagnostics: Vec<GameSceneDiagnosticView>,
    pub validation: GameValidationSceneView,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GameValidationSceneView {
    pub station_state: String,
    pub severity: String,
    pub label: String,
    pub record_key: Option<String>,
    pub phase: Option<String>,
    pub packet_kind: Option<String>,
    pub worker_lease_active: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GameStationView {
    pub station_id: String,
    pub seat_index: usize,
    pub state: String,
    pub severity: String,
    pub actor_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GameActorView {
    pub actor_id: String,
    pub agent_id: String,
    pub task_id: String,
    pub lease_generation: Option<String>,
    pub slot_id: String,
    pub seat_index: usize,
    pub display_name: String,
    pub archetype_key: String,
    pub role_label: String,
    pub visual_state: GameVisualState,
    pub static_pose: GameStaticPose,
    pub severity: String,
    pub status_label: String,
    pub lifecycle_state: String,
    pub task_title: String,
    pub branch_name: String,
    pub progress_label: String,
    pub duration_label: String,
    pub latest_summary: String,
    pub bubble_label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GameStandbyCharacterView {
    pub character_id: String,
    pub agent_id: String,
    pub location_index: usize,
    pub display_name: String,
    pub archetype_key: String,
    pub role_label: String,
    pub presence_kind: String,
    pub visual_state: GameVisualState,
    pub static_pose: GameStaticPose,
    pub severity: String,
    pub status_label: String,
    pub summary: String,
    pub bubble_label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum GameVisualState {
    Idle,
    Starting,
    Working,
    AwaitingReview,
    Blocked,
    Delivering,
    Cleanup,
}

impl GameVisualState {
    fn key(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Starting => "starting",
            Self::Working => "working",
            Self::AwaitingReview => "awaiting_review",
            Self::Blocked => "blocked",
            Self::Delivering => "delivering",
            Self::Cleanup => "cleanup",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Idle => "대기",
            Self::Starting => "시작 준비",
            Self::Working => "작업 중",
            Self::AwaitingReview => "검토 대기",
            Self::Blocked => "차단됨",
            Self::Delivering => "배포 중",
            Self::Cleanup => "정리 중",
        }
    }

    fn severity(self) -> &'static str {
        match self {
            Self::Blocked => "danger",
            Self::Cleanup => "warning",
            Self::Delivering | Self::AwaitingReview | Self::Starting => "info",
            Self::Working => "success",
            Self::Idle => "muted",
        }
    }

    fn pose(self) -> GameStaticPose {
        match self {
            Self::Idle => GameStaticPose::Neutral,
            Self::Starting => GameStaticPose::Neutral,
            Self::Working => GameStaticPose::Laptop,
            Self::AwaitingReview | Self::Delivering => GameStaticPose::Callout,
            Self::Blocked => GameStaticPose::Alert,
            Self::Cleanup => GameStaticPose::Sit,
        }
    }
}

impl std::fmt::Display for GameVisualState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.key())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum GameStaticPose {
    Neutral,
    Laptop,
    Callout,
    Alert,
    Sit,
}

impl std::fmt::Display for GameStaticPose {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Neutral => "neutral",
            Self::Laptop => "laptop",
            Self::Callout => "callout",
            Self::Alert => "alert",
            Self::Sit => "sit",
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GameSceneDiagnosticView {
    pub code: String,
    pub message: String,
    pub agent_id: Option<String>,
    pub slot_id: Option<String>,
    pub severity: String,
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
    pub progress_percent: Option<u8>,
    pub validation_summary: String,
    pub latest_summary: String,
    pub updated_at: String,
    pub trail: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DistributorView {
    pub role_label: String,
    pub head_summary: String,
    pub bubble_label: String,
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
pub(super) struct EventFeedView {
    pub limit: usize,
    pub total_event_count: usize,
    pub visible_event_count: usize,
    pub status_label: String,
    pub newest_sequence: Option<i64>,
    pub event_cursor: Option<i64>,
    pub event_cursor_data: String,
    pub empty_state: String,
    pub incremental: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DistributorQueueItemView {
    pub queue_item_id: Option<String>,
    pub session_key: Option<String>,
    pub slot_id: Option<String>,
    pub task_id: Option<String>,
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
pub(super) struct CampaignView {
    pub summary: String,
    pub attempt_count: usize,
    pub visible_attempt_count: usize,
    pub active_lane_count: usize,
    pub signal_count: usize,
    pub lane_cards: Vec<CampaignLaneView>,
    pub attempts: Vec<CampaignAttemptView>,
    pub intel_cards: Vec<CampaignIntelView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CampaignLaneView {
    pub agent_id: String,
    pub slot_id: String,
    pub class_label: String,
    pub task_title: String,
    pub state: String,
    pub progress_label: String,
    pub summary: String,
    pub severity: String,
    pub score_label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CampaignAttemptView {
    pub label: String,
    pub source: String,
    pub state: String,
    pub timestamp: String,
    pub summary: String,
    pub severity: String,
    pub score_label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CampaignIntelView {
    pub label: String,
    pub value: String,
    pub note: String,
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
    pub source_label: String,
    pub mock_metric_note: String,
    pub badges: Vec<String>,
}

pub(super) fn build_akra_dashboard_view(
    planning_admin: &dyn PlanningAdminPort,
    parallel_mode_admin_port: &dyn ParallelModeAdminPort,
    parallel_agent_profile_port: &dyn ParallelAgentProfilePort,
    pr_validation_query_port: &dyn PrValidationQueryPort,
) -> Result<AkraAdminDashboardView> {
    let workspace_dir = planning_admin.workspace_dir();
    let snapshot =
        parallel_mode_admin_port.load_dashboard_snapshot(workspace_dir, DASHBOARD_EVENT_LIMIT);
    let planning_revision = snapshot.planning_revision;
    let structured_task_count = snapshot.structured_task_count;
    let readiness = snapshot.readiness;
    let supervisor = snapshot.supervisor;
    let events = snapshot.events;
    let agent_profiles = parallel_agent_profile_port
        .load_config(workspace_dir)
        .map_err(anyhow::Error::msg)?;
    let validation = pr_validation_query_port.load_board(PrValidationBoardRequest::default())?;

    let pool = map_pool(&supervisor);
    let agents = map_agents(&supervisor, &agent_profiles);
    let mut scene = map_game_scene(&supervisor, &agent_profiles);
    scene.validation = map_game_validation_scene(&validation);
    let selected_task = map_selected_task(&supervisor);
    let distributor = map_distributor(&supervisor);
    let event_feed = map_event_feed(&events, DASHBOARD_EVENT_LIMIT, false);
    let events = events
        .entries
        .iter()
        .map(map_runtime_event)
        .collect::<Vec<_>>();
    let metrics = map_metrics(&pool, &agents, &distributor);
    let readiness_label = readiness_label(&readiness).to_string();
    let campaign = map_campaign(
        &supervisor,
        &pool,
        &agents,
        &distributor,
        &events,
        &event_feed,
        readiness_label.as_str(),
    );
    let generated_at = Utc::now();
    Ok(AkraAdminDashboardView {
        debug_harness: AdminDebugHarnessView::disabled(),
        workspace: AkraWorkspaceView {
            path: supervisor.workspace_path.clone(),
            branch: current_git_branch(workspace_dir),
            mode: ADMIN_RUNTIME_MODE_LABEL.to_string(),
            readiness: readiness_label,
            readiness_notice: readiness_notice(&readiness).to_string(),
            blocked_action: blocked_action(&readiness, &pool).to_string(),
            purpose_label: "운영 관제 · 하네스 제어".to_string(),
            gamification_policy: "MVP는 XP/코인/영구 레벨을 저장하지 않습니다.".to_string(),
            domain_mapping_note: "요원=Agent, 작업=Task, 워크트리 풀=Pool Slot, 분배관=Distributor"
                .to_string(),
            top_notice: supervisor
                .top_notice
                .clone()
                .or_else(|| readiness.top_alert.clone()),
        },
        kpis: AkraKpiView {
            total_tasks: structured_task_count,
            success_rate: None,
            today_throughput: None,
            active_agents: agents.active_count,
            total_agents: agents.entries.len(),
            pool_configured_size: pool.configured_size,
            pool_idle: pool.summary.idle,
            pool_running: pool.summary.running,
            pool_blocked: pool.summary.blocked,
            queue_depth: distributor.queue_depth,
            queue_depth_basis: "distributor queue depth".to_string(),
            metric_source_label: "snapshot 기반, 미집계 값은 '-'로 표시".to_string(),
            distributor_state: distributor.barrier_state.clone(),
            validation_verifying: validation.summary.verifying,
            validation_failed: validation.summary.failed + validation.summary.blocked,
            validation_remediation_queued: validation.summary.remediation_queued,
            validation_stale: validation.summary.stale,
        },
        pool,
        agents,
        scene,
        selected_task,
        distributor,
        events,
        metrics,
        campaign,
        event_feed,
        validation,
        generated_at: generated_at.to_rfc3339(),
        generated_time_label: generated_at.format("%H:%M:%S").to_string(),
        planning_revision,
        planning_revision_label: planning_revision_label(planning_revision),
    })
}

fn planning_revision_label(planning_revision: Option<i64>) -> String {
    planning_revision
        .map(|revision| format!("rev {revision}"))
        .unwrap_or_else(|| "미집계".to_string())
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
        slots: pool
            .slots
            .iter()
            .map(|slot| map_pool_slot(slot, supervisor))
            .collect(),
    }
}

fn map_pool_slot(
    slot: &ParallelModePoolSlotSnapshot,
    supervisor: &ParallelModeSupervisorSnapshot,
) -> PoolSlotView {
    let owner_agent_id = slot
        .owner_identity
        .as_ref()
        .map(|identity| identity.agent_id.clone());
    let task_id = slot
        .owner_identity
        .as_ref()
        .map(|identity| identity.task_id.clone());
    let owner_session_key = slot
        .owner_identity
        .as_ref()
        .map(|identity| identity.session_key.clone());
    let lease_generation = slot
        .owner_identity
        .as_ref()
        .and_then(|identity| identity.lease_generation.clone());
    let roster_entry = supervisor.roster.entries.iter().find(|entry| {
        entry.slot_id == slot.slot_id && station_roster_identity_error(slot, entry).is_none()
    });
    PoolSlotView {
        slot_id: slot.slot_id.clone(),
        display_slot_label: pool_slot_display_label(&slot.slot_id),
        state: slot.state.label().to_string(),
        label: pool_state_korean_label(slot.state).to_string(),
        branch_name: slot.branch_name.clone(),
        worktree_label: slot.worktree_label.clone(),
        owner_label: slot.owner_label.clone(),
        owner_agent_id,
        task_id,
        owner_session_key,
        lease_generation,
        note: pool_slot_note(slot),
        severity: pool_state_severity(slot.state).to_string(),
        bubble_label: slot_worker_bubble(slot, roster_entry, &supervisor.detail),
    }
}

fn map_agents(
    supervisor: &ParallelModeSupervisorSnapshot,
    agent_profiles: &ParallelAgentProfileConfig,
) -> AgentRosterView {
    let empty_state = if supervisor.roster.entries.is_empty() {
        "자동 루프가 꺼져 있습니다. 시작하면 승인된 작업이 빈 슬롯에 투입됩니다.".to_string()
    } else {
        supervisor.roster.empty_state.clone()
    };
    AgentRosterView {
        active_count: supervisor.roster.active_count(),
        empty_state,
        entries: supervisor
            .roster
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| map_agent(index, entry, agent_profiles))
            .collect(),
    }
}

fn map_agent(
    index: usize,
    entry: &ParallelModeAgentRosterEntry,
    agent_profiles: &ParallelAgentProfileConfig,
) -> AgentView {
    let status = agent_status(entry.state_label.as_str());
    let profile = agent_profiles.profile_for_agent_id(&entry.agent_id);
    let fallback_class = agent_class_label(index).to_string();
    AgentView {
        agent_id: entry.agent_id.clone(),
        display_name: profile
            .as_ref()
            .map(|profile| profile.display_name.clone())
            .unwrap_or_else(|| format!("A{:02}", index + 1)),
        class_label: profile
            .as_ref()
            .map(|profile| profile.avatar_class.clone())
            .unwrap_or_else(|| fallback_class.clone()),
        role_label: profile
            .as_ref()
            .map(|profile| profile.role.clone())
            .unwrap_or(fallback_class),
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

fn map_game_scene(
    supervisor: &ParallelModeSupervisorSnapshot,
    agent_profiles: &ParallelAgentProfileConfig,
) -> GameSceneView {
    let mut actors = Vec::new();
    let mut diagnostics = Vec::new();
    let mut claimed_agent_ids = supervisor
        .roster
        .entries
        .iter()
        .map(|entry| entry.agent_id.clone())
        .collect::<HashSet<_>>();
    claimed_agent_ids.extend(
        supervisor
            .pool
            .slots
            .iter()
            .filter_map(|slot| slot.owner_identity.as_ref())
            .map(|owner| owner.agent_id.clone())
            .filter(|agent_id| !agent_id.trim().is_empty()),
    );
    claimed_agent_ids.extend(
        supervisor
            .distributor
            .queue_items
            .iter()
            .filter(|item| item.queue_state.is_active())
            .map(|item| item.source_agent.clone())
            .filter(|agent_id| !agent_id.trim().is_empty()),
    );
    let mut seen_agent_ids = HashSet::new();
    let mut seen_slot_ids = HashSet::new();

    for entry in &supervisor.roster.entries {
        if !seen_agent_ids.insert(entry.agent_id.clone()) {
            diagnostics.push(game_scene_diagnostic(
                "duplicate_agent",
                format!(
                    "{} 에이전트가 roster에 중복으로 나타났습니다.",
                    entry.agent_id
                ),
                Some(entry.agent_id.clone()),
                Some(entry.slot_id.clone()),
            ));
            continue;
        }
        if !seen_slot_ids.insert(entry.slot_id.clone()) {
            diagnostics.push(game_scene_diagnostic(
                "duplicate_slot_actor",
                format!(
                    "{} 슬롯에 여러 에이전트가 연결되어 있습니다.",
                    entry.slot_id
                ),
                Some(entry.agent_id.clone()),
                Some(entry.slot_id.clone()),
            ));
            continue;
        }

        let Some(identity) = entry.lease_identity.as_ref() else {
            diagnostics.push(game_scene_diagnostic(
                "missing_lease_identity",
                format!(
                    "{} 에이전트의 typed lease identity가 없습니다.",
                    entry.agent_id
                ),
                Some(entry.agent_id.clone()),
                Some(entry.slot_id.clone()),
            ));
            continue;
        };
        let matching_stations = supervisor
            .pool
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.slot_id == entry.slot_id)
            .collect::<Vec<_>>();
        let [(station_index, station)] = matching_stations.as_slice() else {
            diagnostics.push(game_scene_diagnostic(
                "station_identity_mismatch",
                format!(
                    "{} 에이전트의 {} station을 하나로 확정할 수 없습니다.",
                    entry.agent_id, entry.slot_id
                ),
                Some(entry.agent_id.clone()),
                Some(entry.slot_id.clone()),
            ));
            continue;
        };
        if matches!(
            station.state,
            ParallelModePoolSlotState::Idle
                | ParallelModePoolSlotState::Missing
                | ParallelModePoolSlotState::Unavailable
        ) {
            diagnostics.push(game_scene_diagnostic(
                "inactive_station_has_roster_agent",
                format!(
                    "{} station은 {}인데 {} roster agent가 남아 있습니다.",
                    station.slot_id,
                    station.state.label(),
                    entry.agent_id
                ),
                Some(entry.agent_id.clone()),
                Some(entry.slot_id.clone()),
            ));
            continue;
        }
        if station.branch_name != entry.branch_name {
            diagnostics.push(game_scene_diagnostic(
                "station_branch_mismatch",
                format!(
                    "{} station과 {} 에이전트의 branch가 다릅니다.",
                    station.slot_id, entry.agent_id
                ),
                Some(entry.agent_id.clone()),
                Some(entry.slot_id.clone()),
            ));
            continue;
        }
        if let Some(code) = station_roster_identity_error(station, entry) {
            diagnostics.push(game_scene_diagnostic(
                code,
                format!(
                    "{} station owner와 {} roster lease identity가 일치하지 않습니다.",
                    station.slot_id, entry.agent_id
                ),
                Some(entry.agent_id.clone()),
                Some(entry.slot_id.clone()),
            ));
            continue;
        }

        match game_visual_state(entry, station, supervisor) {
            Ok(visual_state) => actors.push(map_game_actor(
                entry,
                identity,
                *station_index,
                visual_state,
                agent_profiles,
            )),
            Err(code) => diagnostics.push(game_scene_diagnostic(
                code,
                format!(
                    "{} 에이전트의 {} lifecycle을 신뢰 가능한 scene state로 투영할 수 없습니다.",
                    entry.agent_id, entry.state_label
                ),
                Some(entry.agent_id.clone()),
                Some(entry.slot_id.clone()),
            )),
        }
    }

    let stations = supervisor
        .pool
        .slots
        .iter()
        .enumerate()
        .map(|(index, slot)| GameStationView {
            station_id: slot.slot_id.clone(),
            seat_index: scene_seat_index(index),
            state: slot.state.label().to_string(),
            severity: pool_state_severity(slot.state).to_string(),
            actor_id: actors
                .iter()
                .find(|actor| actor.slot_id == slot.slot_id)
                .map(|actor| actor.actor_id.clone()),
        })
        .collect();

    let standby_profiles = agent_profiles
        .enabled_profiles()
        .into_iter()
        .filter(|profile| !claimed_agent_ids.contains(&profile.agent_id))
        .collect::<Vec<_>>();
    let standby_profile_count = standby_profiles.len();
    let standby_characters = standby_profiles
        .into_iter()
        .take(STANDBY_CHARACTER_LIMIT)
        .enumerate()
        .map(|(index, profile)| {
            let static_pose = standby_pose_for_avatar_class(&profile.avatar_class);
            GameStandbyCharacterView {
                character_id: format!("standby:{}", profile.agent_id),
                agent_id: profile.agent_id,
                location_index: index + 1,
                display_name: profile.display_name,
                archetype_key: profile.avatar_class,
                role_label: profile.role,
                presence_kind: "configured_standby".to_string(),
                visual_state: GameVisualState::Idle,
                static_pose,
                severity: GameVisualState::Idle.severity().to_string(),
                status_label: "대기 프로필".to_string(),
                summary: "작업 미할당 · runtime actor 아님".to_string(),
                bubble_label: "업무 배정 대기".to_string(),
            }
        })
        .collect();

    GameSceneView {
        stations,
        actors,
        standby_profile_count,
        standby_characters,
        diagnostics,
        validation: GameValidationSceneView {
            station_state: "idle".to_string(),
            severity: "muted".to_string(),
            label: "검증 대기".to_string(),
            record_key: None,
            phase: None,
            packet_kind: None,
            worker_lease_active: false,
        },
    }
}

pub(super) fn map_game_validation_scene(
    validation: &PrValidationBoardSnapshot,
) -> GameValidationSceneView {
    let record = validation
        .records
        .iter()
        .min_by_key(|record| validation_attention_rank(record));
    let Some(record) = record else {
        return GameValidationSceneView {
            station_state: "idle".to_string(),
            severity: "muted".to_string(),
            label: "검증 대기".to_string(),
            record_key: None,
            phase: None,
            packet_kind: None,
            worker_lease_active: false,
        };
    };
    let phase = match record.phase {
        PrValidationAdminPhase::Registered => "registered",
        PrValidationAdminPhase::PreMerge => "pre_merge",
        PrValidationAdminPhase::Integrated => "integrated",
        PrValidationAdminPhase::Verifying => "verifying",
        PrValidationAdminPhase::RemediationQueued => "remediation_queued",
        PrValidationAdminPhase::RemediationRunning => "remediation_running",
        PrValidationAdminPhase::Verified => "verified",
        PrValidationAdminPhase::Blocked => "blocked",
        PrValidationAdminPhase::Failed => "failed",
    };
    let packet_kind = match record.phase {
        PrValidationAdminPhase::Integrated | PrValidationAdminPhase::Verifying => "ci_observation",
        PrValidationAdminPhase::RemediationQueued => "failure_to_queue",
        PrValidationAdminPhase::RemediationRunning => "queue_to_worker",
        PrValidationAdminPhase::Verified => "verified_return",
        PrValidationAdminPhase::Blocked | PrValidationAdminPhase::Failed => "blocked_alert",
        PrValidationAdminPhase::Registered | PrValidationAdminPhase::PreMerge => "pre_merge",
    };
    GameValidationSceneView {
        station_state: phase.to_string(),
        severity: match record.severity {
            crate::application::port::inbound::pr_validation_query_port::PrValidationAdminSeverity::Muted => "muted",
            crate::application::port::inbound::pr_validation_query_port::PrValidationAdminSeverity::Info => "info",
            crate::application::port::inbound::pr_validation_query_port::PrValidationAdminSeverity::Success => "success",
            crate::application::port::inbound::pr_validation_query_port::PrValidationAdminSeverity::Warning => "warning",
            crate::application::port::inbound::pr_validation_query_port::PrValidationAdminSeverity::Danger => "danger",
        }
        .to_string(),
        label: format!("QA/CI · {}", record.phase_label),
        record_key: Some(record.record_key.clone()),
        phase: Some(phase.to_string()),
        packet_kind: Some(packet_kind.to_string()),
        worker_lease_active: record.worker_lease_active,
    }
}

fn validation_attention_rank(record: &PrValidationAdminRecordSummary) -> u8 {
    match () {
        _ if record.provider_blocked => 1,
        _ if record.finding_count > record.remediation_count
            || matches!(
                record.phase,
                PrValidationAdminPhase::Blocked | PrValidationAdminPhase::Failed
            ) =>
        {
            2
        }
        _ if record.stale => 3,
        _ => 4,
    }
}

fn standby_pose_for_avatar_class(avatar_class: &str) -> GameStaticPose {
    match avatar_class {
        "Artificer" | "Seer" | "Guardian" => GameStaticPose::Laptop,
        "Scribe" | "Runner" => GameStaticPose::Sit,
        "Ranger" => GameStaticPose::Neutral,
        _ => GameStaticPose::Sit,
    }
}

fn station_roster_identity_error(
    station: &ParallelModePoolSlotSnapshot,
    entry: &ParallelModeAgentRosterEntry,
) -> Option<&'static str> {
    if station.slot_id.trim().is_empty() || entry.slot_id.trim().is_empty() {
        return Some("missing_slot_identity");
    }
    if station.branch_name.trim().is_empty() || entry.branch_name.trim().is_empty() {
        return Some("missing_branch_identity");
    }
    let Some(owner) = station.owner_identity.as_ref() else {
        return Some("missing_station_owner_identity");
    };
    let Some(identity) = entry.lease_identity.as_ref() else {
        return Some("missing_lease_identity");
    };
    if owner.agent_id.trim().is_empty() {
        return Some("missing_station_owner_agent_id");
    }
    if entry.agent_id.trim().is_empty() {
        return Some("missing_roster_agent_id");
    }
    if owner.task_id.trim().is_empty() {
        return Some("missing_station_owner_task_id");
    }
    if identity.task_id.trim().is_empty() {
        return Some("missing_roster_task_id");
    }
    if owner.session_key.trim().is_empty() {
        return Some("missing_station_owner_session_key");
    }
    if identity.session_key.trim().is_empty() {
        return Some("missing_roster_session_key");
    }
    if owner.agent_id != entry.agent_id {
        return Some("station_owner_agent_mismatch");
    }
    if owner.task_id != identity.task_id {
        return Some("station_owner_task_mismatch");
    }
    if owner.session_key != identity.session_key {
        return Some("station_owner_session_mismatch");
    }
    let Some(station_generation) = owner
        .lease_generation
        .as_deref()
        .filter(|generation| !generation.trim().is_empty())
    else {
        return Some("missing_lease_generation");
    };
    let Some(roster_generation) = identity
        .lease_generation
        .as_deref()
        .filter(|generation| !generation.trim().is_empty())
    else {
        return Some("missing_lease_generation");
    };
    if station_generation == roster_generation {
        None
    } else {
        Some("station_owner_lease_generation_mismatch")
    }
}

fn map_game_actor(
    entry: &ParallelModeAgentRosterEntry,
    identity: &crate::domain::parallel_mode::ParallelModeAgentLeaseIdentity,
    station_index: usize,
    visual_state: GameVisualState,
    agent_profiles: &ParallelAgentProfileConfig,
) -> GameActorView {
    let profile = agent_profiles.profile_for_agent_id(&entry.agent_id);
    let display_name = profile
        .as_ref()
        .map(|profile| profile.display_name.clone())
        .unwrap_or_else(|| entry.agent_id.clone());
    let archetype_key = profile
        .as_ref()
        .map(|profile| profile.avatar_class.clone())
        .unwrap_or_else(|| "Runner".to_string());
    let role_label = profile
        .as_ref()
        .map(|profile| profile.role.clone())
        .unwrap_or_else(|| "Agent".to_string());
    GameActorView {
        actor_id: identity.session_key.clone(),
        agent_id: entry.agent_id.clone(),
        task_id: identity.task_id.clone(),
        lease_generation: identity.lease_generation.clone(),
        slot_id: entry.slot_id.clone(),
        seat_index: scene_seat_index(station_index),
        display_name,
        archetype_key,
        role_label,
        visual_state,
        static_pose: visual_state.pose(),
        severity: visual_state.severity().to_string(),
        status_label: visual_state.label().to_string(),
        lifecycle_state: entry.state_label.clone(),
        task_title: entry.task_title.clone(),
        branch_name: entry.branch_name.clone(),
        progress_label: progress_label(entry.state_label.as_str()),
        duration_label: entry.duration_label.clone(),
        latest_summary: entry.latest_summary.clone(),
        bubble_label: agent_bubble(entry.state_label.as_str()).to_string(),
    }
}

fn game_visual_state(
    entry: &ParallelModeAgentRosterEntry,
    station: &ParallelModePoolSlotSnapshot,
    supervisor: &ParallelModeSupervisorSnapshot,
) -> std::result::Result<GameVisualState, &'static str> {
    let identity = entry
        .lease_identity
        .as_ref()
        .ok_or("missing_lease_identity")?;
    let queue_item = supervisor.distributor.queue_items.iter().find(|item| {
        item.identity.as_ref().is_some_and(|queue_identity| {
            queue_identity.session_key == identity.session_key
                && queue_identity.slot_id == entry.slot_id
                && queue_identity.task_id == identity.task_id
                && item.source_agent == entry.agent_id
        })
    });

    if matches!(
        queue_item.map(|item| item.queue_state),
        Some(ParallelModeQueueItemState::Blocked | ParallelModeQueueItemState::Failed)
    ) || station.state == ParallelModePoolSlotState::Blocked
        || matches!(
            entry.state_label.as_str(),
            "failed" | "official_refresh_recovery_needed"
        )
    {
        return Ok(GameVisualState::Blocked);
    }
    if station.state == ParallelModePoolSlotState::AwaitingCleanup
        || matches!(entry.state_label.as_str(), "cleanup_pending" | "cleaning")
        || matches!(
            queue_item.map(|item| item.queue_state),
            Some(ParallelModeQueueItemState::Cleaning)
        )
    {
        return Ok(GameVisualState::Cleanup);
    }
    if matches!(
        queue_item.map(|item| item.queue_state),
        Some(
            ParallelModeQueueItemState::Queued
                | ParallelModeQueueItemState::Pushing
                | ParallelModeQueueItemState::PrPending
                | ParallelModeQueueItemState::MergePending
                | ParallelModeQueueItemState::Integrating
        )
    ) {
        return Ok(GameVisualState::Delivering);
    }

    match entry.state_label.as_str() {
        "reported_complete" | "ledger_refreshing" | "commit_ready" => {
            Ok(GameVisualState::AwaitingReview)
        }
        "running" => Ok(GameVisualState::Working),
        "assigned" | "starting" => Ok(GameVisualState::Starting),
        "merge_queued" | "pushing" | "pr_pending" | "merge_pending" | "integrating" => {
            Err("delivery_identity_mismatch")
        }
        "done" | "cleaned" | "merged" => Err("terminal_roster_actor"),
        _ => Err("unknown_lifecycle"),
    }
}

fn game_scene_diagnostic(
    code: impl Into<String>,
    message: impl Into<String>,
    agent_id: Option<String>,
    slot_id: Option<String>,
) -> GameSceneDiagnosticView {
    GameSceneDiagnosticView {
        code: code.into(),
        message: message.into(),
        agent_id,
        slot_id,
        severity: "warning".to_string(),
    }
}

fn scene_seat_index(index: usize) -> usize {
    index % SLOT_SEATS_CAPACITY + 1
}

const SLOT_SEATS_CAPACITY: usize = 5;

fn map_selected_task(supervisor: &ParallelModeSupervisorSnapshot) -> Option<SelectedTaskView> {
    let session = supervisor.detail.session.as_ref()?;
    Some(SelectedTaskView {
        task_id: session.task_id.clone(),
        task_title: session.task_title.clone(),
        agent_id: session.agent_id.clone(),
        slot_id: session.slot_id.clone(),
        branch_name: session.branch_name.clone(),
        state: session.state_label.clone(),
        progress_percent: None,
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
        role_label: "배포 관리자 / serialized distributor".to_string(),
        head_summary: distributor.head_summary.clone(),
        bubble_label: distributor_bubble(head_state).to_string(),
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

pub(super) fn build_akra_events_view(
    workspace_dir: &str,
    parallel_mode_admin_port: &dyn ParallelModeAdminPort,
    limit: usize,
    after_sequence: Option<i64>,
) -> (EventFeedView, Vec<RuntimeEventView>) {
    let events = parallel_mode_admin_port.load_runtime_events(workspace_dir, limit, after_sequence);
    let feed = map_event_feed(&events, limit, after_sequence.is_some());
    let entries = events.entries.iter().map(map_runtime_event).collect();
    (feed, entries)
}

fn map_event_feed(
    events: &crate::domain::parallel_mode::ParallelModeRuntimeEventsSnapshot,
    requested_limit: usize,
    incremental: bool,
) -> EventFeedView {
    let visible_event_count = events.visible_count();
    EventFeedView {
        limit: requested_limit,
        total_event_count: events.total_event_count,
        visible_event_count,
        status_label: event_feed_status_label(visible_event_count, events.total_event_count),
        newest_sequence: events.latest().map(|entry| entry.sequence),
        event_cursor: events.event_cursor,
        event_cursor_data: events
            .event_cursor
            .map(|cursor| cursor.to_string())
            .unwrap_or_default(),
        empty_state: events.empty_state.clone(),
        incremental,
    }
}

fn event_feed_status_label(visible_event_count: usize, total_event_count: usize) -> String {
    let total = total_event_count.max(visible_event_count);
    if total > visible_event_count {
        format!("LIVE · 최근 {visible_event_count}개 · 총 {total}개")
    } else {
        format!("LIVE · 총 {total}개")
    }
}

fn map_queue_item(item: &ParallelModeDistributorQueueItem) -> DistributorQueueItemView {
    DistributorQueueItemView {
        queue_item_id: item
            .identity
            .as_ref()
            .map(|identity| identity.queue_item_id.clone()),
        session_key: item
            .identity
            .as_ref()
            .map(|identity| identity.session_key.clone()),
        slot_id: item
            .identity
            .as_ref()
            .map(|identity| identity.slot_id.clone()),
        task_id: item
            .identity
            .as_ref()
            .map(|identity| identity.task_id.clone()),
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
        severity: entry.severity.label().to_string(),
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
        source_label: "derived from authoritative supervisor snapshot".to_string(),
        mock_metric_note: "success_rate, today_throughput, test_success_rate, error_rate are uncollected and rendered as 미집계".to_string(),
        badges,
    }
}

fn map_campaign(
    supervisor: &ParallelModeSupervisorSnapshot,
    pool: &PoolBoardView,
    agents: &AgentRosterView,
    distributor: &DistributorView,
    events: &[RuntimeEventView],
    event_feed: &EventFeedView,
    readiness_label: &str,
) -> CampaignView {
    let lane_cards = agents
        .entries
        .iter()
        .map(map_campaign_lane)
        .collect::<Vec<_>>();
    let (attempt_count, attempts) = map_campaign_attempts(supervisor, distributor, events);
    let active_lane_count = lane_cards.len();
    let signal_count = event_feed.total_event_count.max(events.len());
    let summary = if active_lane_count > 0 {
        format!("{active_lane_count}개 병렬 시도 진행 중 · {signal_count}개 정보 신호 관측")
    } else if distributor.queue_depth > 0 {
        format!(
            "활성 요원은 없지만 분배 큐 {queue_depth}건이 통합 대기 중",
            queue_depth = distributor.queue_depth
        )
    } else {
        "진행 중인 병렬 시도는 없고 브라우저 제어 대기 중".to_string()
    };

    CampaignView {
        summary,
        attempt_count,
        visible_attempt_count: attempts.len(),
        active_lane_count,
        signal_count,
        lane_cards,
        attempts,
        intel_cards: map_campaign_intel(pool, distributor, event_feed, readiness_label),
    }
}

fn map_campaign_lane(agent: &AgentView) -> CampaignLaneView {
    CampaignLaneView {
        agent_id: agent.agent_id.clone(),
        slot_id: agent.slot_id.clone(),
        class_label: agent.class_label.clone(),
        task_title: agent.task_title.clone(),
        state: agent.lifecycle_state.clone(),
        progress_label: agent.progress_label.clone(),
        summary: agent.latest_summary.clone(),
        severity: agent_status_severity(agent.status.as_str()).to_string(),
        score_label: "stage 미집계".to_string(),
    }
}

fn map_campaign_attempts(
    supervisor: &ParallelModeSupervisorSnapshot,
    distributor: &DistributorView,
    events: &[RuntimeEventView],
) -> (usize, Vec<CampaignAttemptView>) {
    if let Some(session) = supervisor.detail.session.as_ref()
        && !session.history.is_empty()
    {
        let total = session.history.len();
        let attempts = session
            .history
            .iter()
            .rev()
            .take(6)
            .enumerate()
            .map(|(index, entry)| CampaignAttemptView {
                label: format!("시도 #{}", total.saturating_sub(index)),
                source: format!("{} / {}", session.agent_id, session.slot_id),
                state: entry.state_label.clone(),
                timestamp: entry.timestamp.clone(),
                summary: entry.summary.clone(),
                severity: lifecycle_severity(entry.state_label.as_str()).to_string(),
                score_label: "stage 미집계".to_string(),
            })
            .collect();
        return (total, attempts);
    }

    if !distributor.queue_items.is_empty() {
        let total = distributor.queue_items.len();
        let attempts = distributor
            .queue_items
            .iter()
            .take(6)
            .enumerate()
            .map(|(index, item)| CampaignAttemptView {
                label: format!("큐 시도 #{}", index + 1),
                source: item.source_agent.clone(),
                state: item.queue_state.clone(),
                timestamp: item.commit_short_sha.clone(),
                summary: item.integration_note.clone(),
                severity: queue_state_severity(item.queue_state.as_str()).to_string(),
                score_label: item.branch_name.clone(),
            })
            .collect();
        return (total, attempts);
    }

    let total = events.len();
    let attempts = events
        .iter()
        .take(6)
        .map(|event| CampaignAttemptView {
            label: format!("신호 #{}", event.sequence),
            source: format!("{}:{}", event.projection_kind, event.projection_key),
            state: event.event_kind.clone(),
            timestamp: event.recorded_at.clone(),
            summary: event.summary.clone(),
            severity: event.severity.clone(),
            score_label: format!("rev {}", event.observed_planning_revision),
        })
        .collect();
    (total, attempts)
}

fn map_campaign_intel(
    pool: &PoolBoardView,
    distributor: &DistributorView,
    event_feed: &EventFeedView,
    readiness_label: &str,
) -> Vec<CampaignIntelView> {
    vec![
        CampaignIntelView {
            label: "Readiness".to_string(),
            value: readiness_label.to_string(),
            note: "parallel capability gate".to_string(),
            severity: readiness_severity(readiness_label).to_string(),
        },
        CampaignIntelView {
            label: "Pool Pressure".to_string(),
            value: format!("{}/{}", pool.summary.running, pool.configured_size),
            note: format!(
                "idle {} / blocked {} / cleanup {}",
                pool.summary.idle, pool.summary.blocked, pool.summary.cleanup
            ),
            severity: pool_pressure_severity(pool).to_string(),
        },
        CampaignIntelView {
            label: "Distributor".to_string(),
            value: distributor.barrier_state.clone(),
            note: distributor
                .blocked_reason
                .clone()
                .unwrap_or_else(|| distributor.head_summary.clone()),
            severity: distributor_severity(distributor).to_string(),
        },
        CampaignIntelView {
            label: "Event Feed".to_string(),
            value: format!(
                "{}/{}",
                event_feed.visible_event_count, event_feed.total_event_count
            ),
            note: event_feed
                .newest_sequence
                .map(|sequence| format!("latest #{sequence}"))
                .unwrap_or_else(|| event_feed.empty_state.clone()),
            severity: "info".to_string(),
        },
    ]
}

fn agent_status_severity(status: &str) -> &'static str {
    match status {
        "blocked" => "danger",
        "cleanup" => "warning",
        "running" => "success",
        _ => "info",
    }
}

fn lifecycle_severity(state_label: &str) -> &'static str {
    match state_label {
        "failed" | "official_refresh_recovery_needed" => "danger",
        "cleanup_pending" | "integrating" | "cleaning" => "warning",
        "done" | "cleaned" | "merged" => "success",
        "reported_complete" | "commit_ready" | "merge_queued" | "pushing" | "pr_pending"
        | "merge_pending" | "running" => "success",
        _ => "info",
    }
}

fn queue_state_severity(state_label: &str) -> &'static str {
    match state_label {
        "blocked" | "failed" => "danger",
        "cleaning" | "integrating" => "warning",
        "done" => "success",
        _ => "info",
    }
}

fn readiness_severity(readiness_label: &str) -> &'static str {
    match readiness_label {
        "ready" => "success",
        "blocked" => "danger",
        "degraded" | "repairing" => "warning",
        _ => "info",
    }
}

fn pool_pressure_severity(pool: &PoolBoardView) -> &'static str {
    if pool.summary.blocked + pool.summary.missing + pool.summary.unavailable > 0 {
        "danger"
    } else if pool.exhausted || pool.summary.cleanup > 0 {
        "warning"
    } else if pool.summary.running > 0 {
        "success"
    } else {
        "info"
    }
}

fn distributor_severity(distributor: &DistributorView) -> &'static str {
    if distributor.blocked_reason.is_some() {
        "danger"
    } else if distributor.barrier_state != "idle" || distributor.queue_depth > 0 {
        "warning"
    } else {
        "success"
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

fn readiness_notice(readiness: &ParallelModeReadinessSnapshot) -> &'static str {
    match readiness.readiness {
        ParallelModeReadinessState::Ready => {
            "준비 완료: 모든 필수 병렬 모드 capability가 통과했습니다."
        }
        ParallelModeReadinessState::Degraded => "주의 필요: 일부 capability가 degraded 상태입니다.",
        ParallelModeReadinessState::Repairing => "복구 중: 병렬 모드 capability가 수렴 중입니다.",
        ParallelModeReadinessState::Blocked => {
            "차단됨: readiness blocker를 해결하기 전에는 병렬 작업을 진행하지 않습니다."
        }
    }
}

fn blocked_action(readiness: &ParallelModeReadinessSnapshot, pool: &PoolBoardView) -> &'static str {
    if readiness.readiness == ParallelModeReadinessState::Blocked {
        "readiness blocker를 확인하고 integration checkout/worktree 상태를 복구하세요."
    } else if pool.summary.blocked > 0 {
        "blocked slot은 operator recovery 또는 명시적 pool reset으로 복구하세요."
    } else if pool.summary.missing > 0 || pool.summary.unavailable > 0 {
        "missing/unavailable slot은 worktree 경로와 권한을 확인하세요."
    } else if readiness.readiness == ParallelModeReadinessState::Degraded {
        "degraded capability와 최근 진단을 확인하고 복구하세요."
    } else if readiness.readiness == ParallelModeReadinessState::Repairing {
        "capability 복구가 끝나고 readiness가 ready로 수렴하는지 확인하세요."
    } else {
        "브라우저 하네스 제어를 대기 중입니다."
    }
}

fn pool_state_korean_label(state: ParallelModePoolSlotState) -> &'static str {
    match state {
        ParallelModePoolSlotState::Idle => "여유",
        ParallelModePoolSlotState::Leased => "예약됨",
        ParallelModePoolSlotState::Running => "작업중",
        ParallelModePoolSlotState::AwaitingCleanup => "정리 대기",
        ParallelModePoolSlotState::Blocked => "차단됨",
        ParallelModePoolSlotState::Missing => "사라짐",
        ParallelModePoolSlotState::Unavailable => "사용 불가",
    }
}

fn pool_state_bubble(state: ParallelModePoolSlotState) -> &'static str {
    match state {
        ParallelModePoolSlotState::Idle => "노는중",
        ParallelModePoolSlotState::Leased => "점유됨",
        ParallelModePoolSlotState::Running => "작업중",
        ParallelModePoolSlotState::AwaitingCleanup => "정리 대기",
        ParallelModePoolSlotState::Blocked => "막힘",
        ParallelModePoolSlotState::Missing => "확인 필요",
        ParallelModePoolSlotState::Unavailable => "잠금",
    }
}

fn slot_worker_bubble(
    slot: &ParallelModePoolSlotSnapshot,
    roster_entry: Option<&ParallelModeAgentRosterEntry>,
    detail: &ParallelModeSupervisorDetailSnapshot,
) -> String {
    if matches!(
        slot.state,
        ParallelModePoolSlotState::Blocked
            | ParallelModePoolSlotState::AwaitingCleanup
            | ParallelModePoolSlotState::Missing
            | ParallelModePoolSlotState::Unavailable
    ) {
        return pool_state_bubble(slot.state).to_string();
    }

    if let Some(session) = detail
        .session
        .as_ref()
        .filter(|session| session.slot_id == slot.slot_id)
        && let Some(label) = session
            .history
            .iter()
            .rev()
            .find_map(|entry| worker_lifecycle_bubble(entry.state_label.as_str()))
            .or_else(|| worker_lifecycle_bubble(session.state_label.as_str()))
    {
        return label.to_string();
    }

    roster_entry
        .and_then(|entry| worker_lifecycle_bubble(entry.state_label.as_str()))
        .unwrap_or_else(|| pool_state_bubble(slot.state))
        .to_string()
}

fn worker_lifecycle_bubble(state_label: &str) -> Option<&'static str> {
    match state_label {
        "assigned" => Some("작업 배정됨"),
        "starting" => Some("세션 준비 중"),
        "running" => Some("작업중"),
        "reported_complete" => Some("결과 제출함"),
        "ledger_refreshing" => Some("검수 중"),
        "commit_ready" => Some("검수 통과"),
        "failed" => Some("실패"),
        "official_refresh_recovery_needed" => Some("복구 필요"),
        _ => None,
    }
}

fn distributor_bubble(state: ParallelModeQueueItemState) -> &'static str {
    match state {
        ParallelModeQueueItemState::Idle => "배포 파이프라인",
        ParallelModeQueueItemState::Queued => "배포 대기",
        ParallelModeQueueItemState::Pushing => "origin push 중",
        ParallelModeQueueItemState::PrPending => "PR 확인 중",
        ParallelModeQueueItemState::MergePending => "merge 준비 중",
        ParallelModeQueueItemState::Integrating => "통합 브랜치 반영 중",
        ParallelModeQueueItemState::Cleaning => "slot 정리 요청",
        ParallelModeQueueItemState::Done => "배포 완료",
        ParallelModeQueueItemState::Blocked | ParallelModeQueueItemState::Failed => "배포 막힘",
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

fn pool_slot_display_label(slot_id: &str) -> String {
    if let Some(number) = slot_id.strip_prefix("slot-")
        && !number.is_empty()
        && number.chars().all(|character| character.is_ascii_digit())
    {
        return format!("슬롯 {number}");
    }
    slot_id.to_string()
}

fn agent_status(state_label: &str) -> &'static str {
    match state_label {
        "failed" | "official_refresh_recovery_needed" => "blocked",
        "cleanup_pending" | "integrating" | "cleaning" => "cleanup",
        "reported_complete" | "commit_ready" | "merge_queued" | "pushing" | "pr_pending"
        | "merge_pending" => "running",
        "assigned" | "starting" | "running" => "running",
        _ => "unknown",
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
        _ => "상태 확인 필요",
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

fn progress_label(_state_label: &str) -> String {
    "미집계".to_string()
}

fn event_icon(event_kind: &str) -> &'static str {
    match event_kind {
        "slot_lease_upsert" => "seat",
        "session_detail_upsert" => "agent",
        "distributor_queue" => "route",
        "worktree_status" => "git",
        "cleanup_completed" => "clean",
        "pr_validation_phase_changed"
        | "pr_validation_poll_deferred"
        | "pr_validation_poll_scheduled"
        | "pr_validation_remediation_admitted"
        | "pr_validation_verified" => "check",
        _ => "event",
    }
}

fn current_git_branch(workspace_dir: &str) -> Option<String> {
    let mut command = git_subprocess::command(["-C", workspace_dir, "branch", "--show-current"]);
    crate::subprocess::command_output(&mut command, "git branch --show-current")
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|branch| branch.trim().to_string())
        .filter(|branch| !branch.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::parallel_agent_profile::ParallelAgentProfile;
    use crate::domain::parallel_mode::{
        ParallelModeAgentRosterSnapshot, ParallelModeAgentSessionDetailSnapshot,
        ParallelModeAgentSessionHistoryEntry, ParallelModeDistributorSnapshot,
        ParallelModeOrchestratorStatus, ParallelModePoolBoardSnapshot,
        ParallelModeRuntimeEventsSnapshot, ParallelModeSupervisorState,
    };
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn admin_mode_label_does_not_claim_the_separate_tui_runtime_is_enabled() {
        assert_eq!(ADMIN_RUNTIME_MODE_LABEL, "controlled projection");
        assert_eq!(planning_revision_label(Some(17)), "rev 17");
        assert_eq!(planning_revision_label(None), "미집계");
    }

    fn temp_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "akra-dashboard-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time should be after epoch")
                .as_nanos()
        ))
    }

    fn profile_config() -> ParallelAgentProfileConfig {
        ParallelAgentProfileConfig {
            profiles: vec![ParallelAgentProfile {
                agent_id: "agent-one".to_string(),
                display_name: "Alpha".to_string(),
                role: "Builder".to_string(),
                persona_prompt: "Ship small slices".to_string(),
                avatar_class: "Seer".to_string(),
                capabilities: vec!["tests".to_string()],
                enabled: true,
            }],
        }
    }

    fn rich_session_detail() -> ParallelModeAgentSessionDetailSnapshot {
        ParallelModeAgentSessionDetailSnapshot::new(
            "session-1",
            "agent-one",
            "task-1",
            "Cover dashboard mapping",
            "slot-1",
            Some("thread-1".to_string()),
            "/tmp/slot-1",
            "akra-agent/slot-1/task-1",
            "2026-05-12T01:00:00Z",
            "commit_ready",
            "approved",
            "ready for distributor",
            "cargo test passed",
            "official refresh accepted",
            Some("pull request is ready".to_string()),
            vec![
                ParallelModeAgentSessionHistoryEntry::new(
                    "assigned",
                    "2026-05-12T01:00:00Z",
                    "slot lease acquired",
                ),
                ParallelModeAgentSessionHistoryEntry::new(
                    "running",
                    "2026-05-12T01:05:00Z",
                    "worker is running",
                ),
                ParallelModeAgentSessionHistoryEntry::new(
                    "commit_ready",
                    "2026-05-12T01:10:00Z",
                    "official completion accepted",
                ),
            ],
            "2026-05-12T01:10:00Z",
        )
    }

    fn rich_supervisor_snapshot() -> ParallelModeSupervisorSnapshot {
        let pool = ParallelModePoolBoardSnapshot::new(
            3,
            "pool-root",
            "ready",
            vec![
                ParallelModePoolSlotSnapshot::new(
                    "slot-1",
                    ParallelModePoolSlotState::Running,
                    "akra-agent/slot-1/task-1",
                    "slot-1",
                    "agent-one / task-1",
                )
                .with_owner_identity(
                    "agent-one",
                    "task-1",
                    "session-one",
                    Some("a".repeat(64)),
                ),
                ParallelModePoolSlotSnapshot::new(
                    "slot-2",
                    ParallelModePoolSlotState::Idle,
                    "prerelease",
                    "slot-2",
                    "-",
                ),
                ParallelModePoolSlotSnapshot::new(
                    "slot-3",
                    ParallelModePoolSlotState::Blocked,
                    "akra-agent/slot-3/task-3",
                    "slot-3",
                    "agent-three / task-3",
                )
                .with_owner_identity(
                    "agent-three",
                    "task-3",
                    "session-three",
                    Some("c".repeat(64)),
                ),
            ],
        );
        let roster = ParallelModeAgentRosterSnapshot::new(
            vec![
                ParallelModeAgentRosterEntry::new(
                    "agent-one",
                    "Cover dashboard mapping",
                    "slot-1",
                    "akra-agent/slot-1/task-1",
                    "commit_ready",
                    "45m",
                    "ready for distributor",
                )
                .with_lease_identity("task-1", "session-one", Some("a".repeat(64))),
                ParallelModeAgentRosterEntry::new(
                    "agent-two",
                    "Investigate dashboard copy",
                    "slot-2",
                    "akra-agent/slot-2/task-2",
                    "running",
                    "2h 10m",
                    "still running",
                )
                .with_lease_identity("task-2", "session-two", Some("b".repeat(64))),
            ],
            "no active agents",
        );
        let detail =
            ParallelModeSupervisorDetailSnapshot::new(Some(rich_session_detail()), "no detail");
        let status = ParallelModeOrchestratorStatus {
            queue_head: "agent-one/task-1".to_string(),
            barrier_state: "blocked".to_string(),
            blocked_reason: Some("orchestrator blocked".to_string()),
            conflict_files: vec!["src/lib.rs".to_string()],
            held_queue_count: 2,
            integration_worktree_readiness: "dirty".to_string(),
            slot_return_wait_reason: None,
        };
        let distributor = ParallelModeDistributorSnapshot::new(
            vec![
                ParallelModeDistributorQueueItem::new(
                    "agent-one",
                    "Cover dashboard mapping",
                    ParallelModeQueueItemState::Pushing,
                    "akra-agent/slot-1/task-1",
                    "abc1234",
                    "pushing source branch",
                )
                .with_identity("queue-one", "session-one", "slot-1", "task-1"),
                ParallelModeDistributorQueueItem::new(
                    "agent-two",
                    "Investigate dashboard copy",
                    ParallelModeQueueItemState::Blocked,
                    "akra-agent/slot-2/task-2",
                    "def5678",
                    "merge conflict",
                )
                .with_identity("queue-two", "session-two", "slot-2", "task-2"),
            ],
            Vec::new(),
            "agent-one ready",
            "integration queue active",
        )
        .with_head_blocked_detail(Some("head blocked by conflict".to_string()))
        .with_orchestrator_status(status);

        ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            "/tmp/workspace",
            pool,
            roster,
            detail,
            distributor,
            Some("parallel mode running".to_string()),
        )
    }

    fn runtime_events() -> Vec<RuntimeEventView> {
        vec![
            map_runtime_event(&ParallelModeRuntimeEventEntry::new(
                12,
                "cleanup_completed",
                "slot",
                "slot-1",
                7,
                "cleanup finished",
                "2026-05-12T01:20:00Z",
            )),
            map_runtime_event(&ParallelModeRuntimeEventEntry::new(
                11,
                "worktree_status_blocked",
                "worktree",
                "integration",
                7,
                "worktree blocked",
                "2026-05-12T01:19:00Z",
            )),
        ]
    }

    fn scene_supervisor(
        lifecycle_state: &str,
        pool_state: ParallelModePoolSlotState,
    ) -> ParallelModeSupervisorSnapshot {
        let mut supervisor = rich_supervisor_snapshot();
        supervisor.pool = ParallelModePoolBoardSnapshot::new(
            1,
            "pool-root",
            "ready",
            vec![
                ParallelModePoolSlotSnapshot::new(
                    "slot-1",
                    pool_state,
                    "akra-agent/slot-1/task-1",
                    "slot-1",
                    "agent-one / task-1",
                )
                .with_owner_identity(
                    "agent-one",
                    "task-1",
                    "session-one",
                    Some("a".repeat(64)),
                ),
            ],
        );
        supervisor.roster = ParallelModeAgentRosterSnapshot::new(
            vec![
                ParallelModeAgentRosterEntry::new(
                    "agent-one",
                    "Cover dashboard mapping",
                    "slot-1",
                    "akra-agent/slot-1/task-1",
                    lifecycle_state,
                    "45m",
                    "latest summary",
                )
                .with_lease_identity("task-1", "session-one", Some("a".repeat(64))),
            ],
            "no active agents",
        );
        supervisor.distributor =
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "no queue");
        supervisor
    }

    #[test]
    fn dashboard_mapping_projects_rich_supervisor_snapshot() {
        let supervisor = rich_supervisor_snapshot();
        let pool = map_pool(&supervisor);
        let agents = map_agents(&supervisor, &profile_config());
        let scene = map_game_scene(&supervisor, &profile_config());
        let selected_task = map_selected_task(&supervisor).expect("selected task should map");
        let distributor = map_distributor(&supervisor);
        let events_snapshot = ParallelModeRuntimeEventsSnapshot::new(
            vec![
                ParallelModeRuntimeEventEntry::new(
                    12,
                    "cleanup_completed",
                    "slot",
                    "slot-1",
                    7,
                    "cleanup finished",
                    "2026-05-12T01:20:00Z",
                ),
                ParallelModeRuntimeEventEntry::new(
                    11,
                    "worktree_status_blocked",
                    "worktree",
                    "integration",
                    7,
                    "worktree blocked",
                    "2026-05-12T01:19:00Z",
                ),
            ],
            9,
            "no events",
        );
        let event_feed = map_event_feed(&events_snapshot, DASHBOARD_EVENT_LIMIT, false);
        let events = events_snapshot
            .entries
            .iter()
            .map(map_runtime_event)
            .collect::<Vec<_>>();
        let metrics = map_metrics(&pool, &agents, &distributor);
        let campaign = map_campaign(
            &supervisor,
            &pool,
            &agents,
            &distributor,
            &events,
            &event_feed,
            "blocked",
        );

        assert_eq!(pool.configured_size, 3);
        assert_eq!(pool.summary.idle, 1);
        assert_eq!(pool.summary.running, 1);
        assert_eq!(pool.summary.blocked, 1);
        assert_eq!(pool.slots[0].display_slot_label, "슬롯 1");
        assert_eq!(pool.slots[0].owner_agent_id.as_deref(), Some("agent-one"));
        assert_eq!(pool.slots[0].task_id.as_deref(), Some("task-1"));
        assert_eq!(
            pool.slots[0].owner_session_key.as_deref(),
            Some("session-one")
        );
        assert_eq!(
            pool.slots[0].lease_generation.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(pool.slots[0].note, "agent-one / task-1 / slot-1");
        assert_eq!(pool.slots[0].bubble_label, "검수 통과");
        assert_eq!(pool.slots[1].owner_agent_id, None);
        assert_eq!(pool.slots[1].note, "slot-2");

        assert_eq!(agents.active_count, 2);
        assert_eq!(agents.entries[0].display_name, "Alpha");
        assert_eq!(agents.entries[0].class_label, "Seer");
        assert_eq!(agents.entries[0].role_label, "Builder");
        assert_eq!(agents.entries[0].progress_label, "미집계");
        assert_eq!(agents.entries[0].bubble_label, "공식 승인");
        assert_eq!(agents.entries[1].display_name, "A02");
        assert_eq!(agents.entries[1].class_label, "Scribe");
        assert!(agents.entries[1].overload);

        assert_eq!(scene.stations.len(), 3);
        assert_eq!(scene.actors.len(), 1);
        assert_eq!(scene.actors[0].actor_id, "session-one");
        assert_eq!(scene.actors[0].agent_id, "agent-one");
        assert_eq!(scene.actors[0].task_id, "task-1");
        assert_eq!(scene.actors[0].slot_id, "slot-1");
        assert_eq!(scene.actors[0].seat_index, 1);
        assert_eq!(scene.actors[0].visual_state, GameVisualState::Delivering);
        assert_eq!(scene.actors[0].static_pose, GameStaticPose::Callout);
        assert_eq!(scene.stations[0].actor_id.as_deref(), Some("session-one"));
        assert_eq!(scene.stations[1].actor_id, None);
        assert_eq!(scene.stations[2].actor_id, None);
        assert_eq!(scene.standby_profile_count, 0);
        assert!(scene.standby_characters.is_empty());
        assert_eq!(scene.diagnostics.len(), 1);
        assert_eq!(
            scene.diagnostics[0].code,
            "inactive_station_has_roster_agent"
        );

        assert_eq!(selected_task.task_id, "task-1");
        assert_eq!(selected_task.progress_percent, None);
        assert_eq!(
            selected_task.trail,
            vec![
                "assigned".to_string(),
                "running".to_string(),
                "commit_ready".to_string()
            ]
        );

        assert_eq!(
            distributor.blocked_reason.as_deref(),
            Some("head blocked by conflict")
        );
        assert_eq!(distributor.queue_depth, 2);
        assert_eq!(distributor.conflict_files, vec!["src/lib.rs".to_string()]);
        assert_eq!(distributor.queue_items[0].queue_state, "pushing");
        assert_eq!(distributor.pipeline[0].state, "done");
        assert_eq!(distributor.pipeline[1].state, "active");
        assert_eq!(distributor.pipeline[2].state, "active");

        assert_eq!(event_feed.total_event_count, 9);
        assert_eq!(event_feed.visible_event_count, 2);
        assert_eq!(event_feed.status_label, "LIVE · 최근 2개 · 총 9개");
        assert_eq!(event_feed.newest_sequence, Some(12));
        assert_eq!(events[0].icon, "clean");
        assert_eq!(events[0].severity, "success");

        assert_eq!(metrics.pool_utilization_percent, 66);
        assert_eq!(metrics.active_agent_count, 2);
        assert_eq!(metrics.waiting_task_count, 2);
        assert_eq!(metrics.blocked_slot_count, 1);
        assert!(metrics.badges.contains(&"복구 필요".to_string()));
        assert!(!metrics.badges.contains(&"풀 관리자".to_string()));

        assert_eq!(campaign.active_lane_count, 2);
        assert_eq!(campaign.attempt_count, 3);
        assert_eq!(campaign.visible_attempt_count, 3);
        assert_eq!(campaign.signal_count, 9);
        assert!(campaign.summary.contains("2개 병렬 시도 진행 중"));
        assert_eq!(campaign.lane_cards[0].score_label, "stage 미집계");
        assert_eq!(campaign.attempts[0].label, "시도 #3");
        assert_eq!(campaign.attempts[0].severity, "success");
        assert_eq!(campaign.intel_cards[0].severity, "danger");
        assert_eq!(campaign.intel_cards[2].severity, "danger");
        assert_eq!(campaign.intel_cards[3].note, "latest #12");
    }

    #[test]
    fn game_scene_projects_configured_standby_profiles_without_fabricating_runtime_actors() {
        let mut supervisor = scene_supervisor("running", ParallelModePoolSlotState::Running);
        supervisor.roster.entries.clear();
        let mut profiles = ParallelAgentProfileConfig::default();
        profiles.profiles.push(ParallelAgentProfile {
            agent_id: "agent-overflow".to_string(),
            display_name: "오버플로".to_string(),
            role: "예비 담당".to_string(),
            persona_prompt: "Wait for assignment".to_string(),
            avatar_class: "Runner".to_string(),
            capabilities: Vec::new(),
            enabled: true,
        });

        let scene = map_game_scene(&supervisor, &profiles);

        assert!(scene.actors.is_empty());
        assert!(scene.diagnostics.is_empty());
        assert_eq!(scene.standby_profile_count, 4);
        assert_eq!(scene.standby_characters.len(), STANDBY_CHARACTER_LIMIT);
        assert_eq!(
            scene
                .standby_characters
                .iter()
                .map(|character| character.agent_id.as_str())
                .collect::<Vec<_>>(),
            vec!["agent-artificer", "agent-scribe", "agent-guardian"]
        );
        assert_eq!(
            scene
                .standby_characters
                .iter()
                .map(|character| character.location_index)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(
            scene
                .standby_characters
                .iter()
                .map(|character| character.static_pose)
                .collect::<Vec<_>>(),
            vec![
                GameStaticPose::Laptop,
                GameStaticPose::Sit,
                GameStaticPose::Laptop
            ]
        );
        for character in &scene.standby_characters {
            assert_eq!(character.presence_kind, "configured_standby");
            assert_eq!(character.visual_state, GameVisualState::Idle);
            assert_eq!(character.severity, "muted");
            assert_eq!(character.status_label, "대기 프로필");
        }
        let standby_json = serde_json::to_value(&scene.standby_characters[0])
            .expect("standby character should serialize");
        for runtime_identity in [
            "actorId",
            "taskId",
            "slotId",
            "sessionKey",
            "ownerAgentId",
            "ownerSessionKey",
            "leaseGeneration",
            "branchName",
            "queueItemId",
        ] {
            assert!(
                standby_json.get(runtime_identity).is_none(),
                "standby presence must not fabricate {runtime_identity}"
            );
        }
        assert!(
            scene
                .stations
                .iter()
                .all(|station| station.actor_id.is_none())
        );
    }

    #[test]
    fn game_scene_excludes_every_profile_claimed_by_pool_or_active_delivery() {
        let mut supervisor = scene_supervisor("running", ParallelModePoolSlotState::Running);
        supervisor.roster.entries.clear();
        supervisor.pool.slots[0]
            .owner_identity
            .as_mut()
            .expect("scene fixture should have typed station ownership")
            .agent_id = "agent-artificer".to_string();
        supervisor.distributor.queue_items = vec![ParallelModeDistributorQueueItem::new(
            "agent-scribe",
            "Deliver queued work",
            ParallelModeQueueItemState::Queued,
            "akra-agent/slot-2/task-2",
            "abc1234",
            "queued",
        )];

        let claimed = map_game_scene(&supervisor, &ParallelAgentProfileConfig::default());
        assert_eq!(claimed.standby_profile_count, 1);
        assert_eq!(claimed.standby_characters.len(), 1);
        assert_eq!(claimed.standby_characters[0].agent_id, "agent-guardian");

        supervisor.distributor.queue_items[0].queue_state = ParallelModeQueueItemState::Done;
        let terminal_queue = map_game_scene(&supervisor, &ParallelAgentProfileConfig::default());
        assert_eq!(terminal_queue.standby_profile_count, 2);
        assert_eq!(
            terminal_queue
                .standby_characters
                .iter()
                .map(|character| character.agent_id.as_str())
                .collect::<Vec<_>>(),
            vec!["agent-scribe", "agent-guardian"]
        );
    }

    #[test]
    fn game_scene_gives_ranger_standby_an_explicit_neutral_pose() {
        let mut supervisor = scene_supervisor("running", ParallelModePoolSlotState::Running);
        supervisor.roster.entries.clear();
        let profiles = ParallelAgentProfileConfig {
            profiles: vec![ParallelAgentProfile {
                agent_id: "agent-ranger".to_string(),
                display_name: "레인저".to_string(),
                role: "탐색 담당".to_string(),
                persona_prompt: "Wait at the lounge".to_string(),
                avatar_class: "Ranger".to_string(),
                capabilities: Vec::new(),
                enabled: true,
            }],
        };

        let scene = map_game_scene(&supervisor, &profiles);

        assert_eq!(scene.standby_profile_count, 1);
        assert_eq!(
            scene.standby_characters[0].static_pose,
            GameStaticPose::Neutral
        );
    }

    #[test]
    fn game_scene_maps_closed_lifecycle_states_to_static_poses() {
        let cases = [
            (
                "assigned",
                ParallelModePoolSlotState::Leased,
                GameVisualState::Starting,
                GameStaticPose::Neutral,
            ),
            (
                "starting",
                ParallelModePoolSlotState::Leased,
                GameVisualState::Starting,
                GameStaticPose::Neutral,
            ),
            (
                "running",
                ParallelModePoolSlotState::Running,
                GameVisualState::Working,
                GameStaticPose::Laptop,
            ),
            (
                "reported_complete",
                ParallelModePoolSlotState::Running,
                GameVisualState::AwaitingReview,
                GameStaticPose::Callout,
            ),
            (
                "commit_ready",
                ParallelModePoolSlotState::Running,
                GameVisualState::AwaitingReview,
                GameStaticPose::Callout,
            ),
            (
                "failed",
                ParallelModePoolSlotState::Blocked,
                GameVisualState::Blocked,
                GameStaticPose::Alert,
            ),
            (
                "cleanup_pending",
                ParallelModePoolSlotState::AwaitingCleanup,
                GameVisualState::Cleanup,
                GameStaticPose::Sit,
            ),
        ];

        for (lifecycle, pool_state, expected_state, expected_pose) in cases {
            let scene = map_game_scene(&scene_supervisor(lifecycle, pool_state), &profile_config());
            assert_eq!(scene.actors.len(), 1, "{lifecycle}");
            assert_eq!(scene.actors[0].visual_state, expected_state, "{lifecycle}");
            assert_eq!(scene.actors[0].static_pose, expected_pose, "{lifecycle}");
            assert!(scene.diagnostics.is_empty(), "{lifecycle}");
        }
    }

    #[test]
    fn game_scene_applies_station_overlay_precedence() {
        let blocked = map_game_scene(
            &scene_supervisor("running", ParallelModePoolSlotState::Blocked),
            &profile_config(),
        );
        assert_eq!(blocked.actors[0].visual_state, GameVisualState::Blocked);

        let cleanup = map_game_scene(
            &scene_supervisor("commit_ready", ParallelModePoolSlotState::AwaitingCleanup),
            &profile_config(),
        );
        assert_eq!(cleanup.actors[0].visual_state, GameVisualState::Cleanup);
    }

    #[test]
    fn game_scene_requires_typed_delivery_identity() {
        let mut supervisor = scene_supervisor("pushing", ParallelModePoolSlotState::Running);
        let unmatched = map_game_scene(&supervisor, &profile_config());
        assert!(unmatched.actors.is_empty());
        assert_eq!(unmatched.diagnostics[0].code, "delivery_identity_mismatch");

        supervisor.distributor.queue_items = vec![
            ParallelModeDistributorQueueItem::new(
                "agent-one",
                "Cover dashboard mapping",
                ParallelModeQueueItemState::Pushing,
                "akra-agent/slot-1/task-1",
                "abc1234",
                "pushing source branch",
            )
            .with_identity("queue-one", "session-one", "slot-1", "task-1"),
        ];
        let matched = map_game_scene(&supervisor, &profile_config());
        assert_eq!(matched.actors.len(), 1);
        assert_eq!(matched.actors[0].visual_state, GameVisualState::Delivering);
    }

    #[test]
    fn game_scene_rejects_unknown_terminal_and_mismatched_actors() {
        for lifecycle in ["unknown", "done", "cleaned", "merged"] {
            let scene = map_game_scene(
                &scene_supervisor(lifecycle, ParallelModePoolSlotState::Running),
                &profile_config(),
            );
            assert!(scene.actors.is_empty(), "{lifecycle}");
            assert_eq!(scene.diagnostics.len(), 1, "{lifecycle}");
        }

        let mut missing_identity = scene_supervisor("running", ParallelModePoolSlotState::Running);
        missing_identity.roster.entries[0].lease_identity = None;
        let scene = map_game_scene(&missing_identity, &profile_config());
        assert!(scene.actors.is_empty());
        assert_eq!(scene.diagnostics[0].code, "missing_lease_identity");

        let inactive = map_game_scene(
            &scene_supervisor("running", ParallelModePoolSlotState::Idle),
            &profile_config(),
        );
        assert!(inactive.actors.is_empty());
        assert_eq!(
            inactive.diagnostics[0].code,
            "inactive_station_has_roster_agent"
        );

        let mut branch_mismatch = scene_supervisor("running", ParallelModePoolSlotState::Running);
        branch_mismatch.pool.slots[0].branch_name = "other-branch".to_string();
        let scene = map_game_scene(&branch_mismatch, &profile_config());
        assert!(scene.actors.is_empty());
        assert!(scene.standby_characters.is_empty());
        assert_eq!(scene.diagnostics[0].code, "station_branch_mismatch");
    }

    #[test]
    fn game_scene_requires_exact_station_owner_and_roster_lease_identity() {
        let base = scene_supervisor("running", ParallelModePoolSlotState::Running);

        let mut missing_owner = base.clone();
        missing_owner.pool.slots[0].owner_identity = None;
        let scene = map_game_scene(&missing_owner, &profile_config());
        assert!(scene.actors.is_empty());
        assert_eq!(scene.diagnostics[0].code, "missing_station_owner_identity");

        let cases = [
            ("agent", "station_owner_agent_mismatch"),
            ("task", "station_owner_task_mismatch"),
            ("session", "station_owner_session_mismatch"),
            ("generation", "station_owner_lease_generation_mismatch"),
        ];
        for (field, expected_code) in cases {
            let mut supervisor = base.clone();
            let owner = supervisor.pool.slots[0]
                .owner_identity
                .as_mut()
                .expect("scene fixture should have typed station ownership");
            match field {
                "agent" => owner.agent_id = "other-agent".to_string(),
                "task" => owner.task_id = "other-task".to_string(),
                "session" => owner.session_key = "other-session".to_string(),
                "generation" => owner.lease_generation = Some("b".repeat(64)),
                _ => unreachable!(),
            }
            let scene = map_game_scene(&supervisor, &profile_config());
            assert!(scene.actors.is_empty(), "{field}");
            assert_eq!(scene.diagnostics[0].code, expected_code, "{field}");
        }

        let missing_cases = [
            ("station_agent", "missing_station_owner_agent_id"),
            ("roster_agent", "missing_roster_agent_id"),
            ("station_task", "missing_station_owner_task_id"),
            ("roster_task", "missing_roster_task_id"),
            ("station_session", "missing_station_owner_session_key"),
            ("roster_session", "missing_roster_session_key"),
            ("station_generation", "missing_lease_generation"),
            ("roster_generation", "missing_lease_generation"),
        ];
        for (field, expected_code) in missing_cases {
            let mut supervisor = base.clone();
            match field {
                "station_agent" => supervisor.pool.slots[0]
                    .owner_identity
                    .as_mut()
                    .expect("scene fixture should have typed station ownership")
                    .agent_id
                    .clear(),
                "roster_agent" => supervisor.roster.entries[0].agent_id.clear(),
                "station_task" => supervisor.pool.slots[0]
                    .owner_identity
                    .as_mut()
                    .expect("scene fixture should have typed station ownership")
                    .task_id
                    .clear(),
                "roster_task" => supervisor.roster.entries[0]
                    .lease_identity
                    .as_mut()
                    .expect("scene fixture should have typed roster identity")
                    .task_id
                    .clear(),
                "station_session" => supervisor.pool.slots[0]
                    .owner_identity
                    .as_mut()
                    .expect("scene fixture should have typed station ownership")
                    .session_key
                    .clear(),
                "roster_session" => supervisor.roster.entries[0]
                    .lease_identity
                    .as_mut()
                    .expect("scene fixture should have typed roster identity")
                    .session_key
                    .clear(),
                "station_generation" => {
                    supervisor.pool.slots[0]
                        .owner_identity
                        .as_mut()
                        .expect("scene fixture should have typed station ownership")
                        .lease_generation = Some(String::new())
                }
                "roster_generation" => {
                    supervisor.roster.entries[0]
                        .lease_identity
                        .as_mut()
                        .expect("scene fixture should have typed roster identity")
                        .lease_generation = Some(String::new())
                }
                _ => unreachable!(),
            }
            let scene = map_game_scene(&supervisor, &profile_config());
            assert!(scene.actors.is_empty(), "{field}");
            assert_eq!(scene.diagnostics[0].code, expected_code, "{field}");
        }

        let mut missing_generation = base;
        missing_generation.pool.slots[0]
            .owner_identity
            .as_mut()
            .expect("scene fixture should have typed station ownership")
            .lease_generation = None;
        let scene = map_game_scene(&missing_generation, &profile_config());
        assert!(scene.actors.is_empty());
        assert_eq!(scene.diagnostics[0].code, "missing_lease_generation");
    }

    #[test]
    fn campaign_attempts_fall_back_to_distributor_queue_then_runtime_events() {
        let mut supervisor = rich_supervisor_snapshot();
        supervisor.detail = ParallelModeSupervisorDetailSnapshot::new(None, "no detail");
        let distributor = map_distributor(&supervisor);
        let events = runtime_events();

        let (queue_total, queue_attempts) =
            map_campaign_attempts(&supervisor, &distributor, &events);
        assert_eq!(queue_total, 2);
        assert_eq!(queue_attempts[0].label, "큐 시도 #1");
        assert_eq!(queue_attempts[0].source, "agent-one");
        assert_eq!(queue_attempts[0].severity, "info");
        assert_eq!(queue_attempts[1].severity, "danger");

        supervisor.distributor =
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "no queue");
        let empty_distributor = map_distributor(&supervisor);
        let (event_total, event_attempts) =
            map_campaign_attempts(&supervisor, &empty_distributor, &events);
        assert_eq!(event_total, 2);
        assert_eq!(event_attempts[0].label, "신호 #12");
        assert_eq!(event_attempts[0].state, "cleanup_completed");
        assert_eq!(event_attempts[0].severity, "success");
        assert_eq!(event_attempts[1].severity, "danger");
    }

    #[test]
    fn dashboard_copy_helpers_cover_status_progress_and_severity_edges() {
        let blank_owner_slot = ParallelModePoolSlotSnapshot::new(
            "slot-x",
            ParallelModePoolSlotState::Idle,
            "prerelease",
            "slot-x",
            " ",
        );
        assert_eq!(pool_slot_note(&blank_owner_slot), "slot-x");
        assert_eq!(agent_class_label(6), "Artificer");

        assert_eq!(agent_status("failed"), "blocked");
        assert_eq!(agent_status("cleanup_pending"), "cleanup");
        assert_eq!(agent_status("assigned"), "running");
        assert_eq!(agent_status("unknown"), "unknown");
        assert_eq!(agent_bubble("official_refresh_recovery_needed"), "차단됨");
        assert_eq!(agent_bubble("cleanup_pending"), "정리중");
        assert_eq!(agent_bubble("unknown"), "상태 확인 필요");

        assert_eq!(progress_label("running"), "미집계");

        assert_eq!(lifecycle_severity("failed"), "danger");
        assert_eq!(lifecycle_severity("integrating"), "warning");
        assert_eq!(lifecycle_severity("done"), "success");
        assert_eq!(lifecycle_severity("running"), "success");
        assert_eq!(lifecycle_severity("unknown"), "info");
        assert_eq!(queue_state_severity("failed"), "danger");
        assert_eq!(queue_state_severity("cleaning"), "warning");
        assert_eq!(queue_state_severity("done"), "success");
        assert_eq!(queue_state_severity("queued"), "info");
        assert_eq!(readiness_severity("ready"), "success");
        assert_eq!(readiness_severity("repairing"), "warning");
        assert_eq!(readiness_severity("blocked"), "danger");

        let mut pool = map_pool(&rich_supervisor_snapshot());
        assert_eq!(pool_pressure_severity(&pool), "danger");
        pool.summary.blocked = 0;
        pool.summary.missing = 0;
        pool.summary.unavailable = 0;
        pool.summary.cleanup = 1;
        assert_eq!(pool_pressure_severity(&pool), "warning");
        pool.summary.cleanup = 0;
        pool.exhausted = false;
        pool.summary.running = 1;
        assert_eq!(pool_pressure_severity(&pool), "success");
        pool.summary.running = 0;
        assert_eq!(pool_pressure_severity(&pool), "info");

        let mut distributor = map_distributor(&rich_supervisor_snapshot());
        assert_eq!(distributor_severity(&distributor), "danger");
        distributor.blocked_reason = None;
        assert_eq!(distributor_severity(&distributor), "warning");
        distributor.barrier_state = "idle".to_string();
        distributor.queue_items.clear();
        distributor.queue_depth = 0;
        assert_eq!(distributor_severity(&distributor), "success");

        assert_eq!(event_icon("slot_lease_upsert"), "seat");
        assert_eq!(event_icon("session_detail_upsert"), "agent");
        assert_eq!(event_icon("distributor_queue"), "route");
        assert_eq!(event_icon("worktree_status"), "git");
        assert_eq!(event_icon("unknown"), "event");
        assert_eq!(
            ParallelModeRuntimeEventEntry::new(
                1,
                "worker_failed",
                "worker",
                "slot-1",
                1,
                "failed",
                "2026-08-10T00:00:00Z",
            )
            .severity
            .label(),
            "danger"
        );
        assert_eq!(
            ParallelModeRuntimeEventEntry::new(
                2,
                "cleanup_completed",
                "worker",
                "slot-1",
                1,
                "clean",
                "2026-08-10T00:00:00Z",
            )
            .severity
            .label(),
            "success"
        );
        assert_eq!(
            ParallelModeRuntimeEventEntry::new(
                3,
                "worktree_status",
                "worker",
                "slot-1",
                1,
                "status",
                "2026-08-10T00:00:00Z",
            )
            .severity
            .label(),
            "warning"
        );
        assert_eq!(
            ParallelModeRuntimeEventEntry::new(
                4,
                "slot_lease_upsert",
                "worker",
                "slot-1",
                1,
                "lease",
                "2026-08-10T00:00:00Z",
            )
            .severity
            .label(),
            "info"
        );
    }

    #[test]
    fn current_git_branch_reads_git_branch_and_ignores_non_repositories() {
        let non_repo = temp_path("non-repo");
        std::fs::create_dir_all(&non_repo).expect("non-repo temp dir should be created");
        assert_eq!(
            current_git_branch(non_repo.to_string_lossy().as_ref()),
            None
        );
        std::fs::remove_dir_all(&non_repo).expect("non-repo temp dir should be removed");

        let repo = temp_path("repo");
        std::fs::create_dir_all(&repo).expect("repo temp dir should be created");
        let status = Command::new("git")
            .args(["init", "-b", "dashboard-test"])
            .arg(&repo)
            .env("GIT_TERMINAL_PROMPT", "0")
            .status()
            .expect("git init should run");
        assert!(status.success());

        assert_eq!(
            current_git_branch(repo.to_string_lossy().as_ref()).as_deref(),
            Some("dashboard-test")
        );
        std::fs::remove_dir_all(&repo).expect("repo temp dir should be removed");
    }

    #[test]
    fn pool_slot_state_mapping_covers_all_states() {
        let cases = [
            (ParallelModePoolSlotState::Idle, "여유", "normal", "노는중"),
            (
                ParallelModePoolSlotState::Leased,
                "예약됨",
                "info",
                "점유됨",
            ),
            (
                ParallelModePoolSlotState::Running,
                "작업중",
                "normal",
                "작업중",
            ),
            (
                ParallelModePoolSlotState::AwaitingCleanup,
                "정리 대기",
                "warning",
                "정리 대기",
            ),
            (
                ParallelModePoolSlotState::Blocked,
                "차단됨",
                "danger",
                "막힘",
            ),
            (
                ParallelModePoolSlotState::Missing,
                "사라짐",
                "muted",
                "확인 필요",
            ),
            (
                ParallelModePoolSlotState::Unavailable,
                "사용 불가",
                "muted",
                "잠금",
            ),
        ];

        for (state, label, severity, bubble) in cases {
            assert_eq!(pool_state_korean_label(state), label);
            assert_eq!(pool_state_severity(state), severity);
            assert_eq!(pool_state_bubble(state), bubble);
        }
    }

    #[test]
    fn pool_slot_display_label_hides_raw_slot_prefix_when_possible() {
        assert_eq!(pool_slot_display_label("slot-2"), "슬롯 2");
        assert_eq!(pool_slot_display_label("slot-12"), "슬롯 12");
        assert_eq!(pool_slot_display_label("integration"), "integration");
    }

    #[test]
    fn distributor_pipeline_maps_queue_state_progression_and_blocks() {
        let queued = map_pipeline(ParallelModeQueueItemState::Queued);
        assert_eq!(queued[0].state, "active");
        assert_eq!(queued[1].state, "waiting");

        let merge_pending = map_pipeline(ParallelModeQueueItemState::MergePending);
        assert_eq!(merge_pending[0].state, "done");
        assert_eq!(merge_pending[4].state, "active");

        let blocked = map_pipeline(ParallelModeQueueItemState::Blocked);
        assert!(blocked.iter().all(|step| step.state == "blocked"));

        let failed = map_pipeline(ParallelModeQueueItemState::Failed);
        assert!(failed.iter().all(|step| step.state == "failed"));
    }

    #[test]
    fn worker_lifecycle_bubble_excludes_distributor_delivery_states() {
        assert_eq!(worker_lifecycle_bubble("assigned"), Some("작업 배정됨"));
        assert_eq!(worker_lifecycle_bubble("starting"), Some("세션 준비 중"));
        assert_eq!(worker_lifecycle_bubble("running"), Some("작업중"));
        assert_eq!(
            worker_lifecycle_bubble("reported_complete"),
            Some("결과 제출함")
        );
        assert_eq!(
            worker_lifecycle_bubble("ledger_refreshing"),
            Some("검수 중")
        );
        assert_eq!(worker_lifecycle_bubble("commit_ready"), Some("검수 통과"));

        for distributor_state in [
            "merge_queued",
            "pushing",
            "pr_pending",
            "merge_pending",
            "integrating",
            "cleanup_pending",
            "cleaned",
        ] {
            assert_eq!(
                worker_lifecycle_bubble(distributor_state),
                None,
                "{distributor_state} should not be spoken by slot workers"
            );
        }
    }

    #[test]
    fn slot_worker_bubble_prefers_slot_cleanup_state_over_old_worker_history() {
        let slot = ParallelModePoolSlotSnapshot::new(
            "slot-1",
            ParallelModePoolSlotState::AwaitingCleanup,
            "akra-agent/slot-1/task",
            "slot-1",
            "agent-1 / task-1",
        );
        let roster_entry = ParallelModeAgentRosterEntry::new(
            "agent-1",
            "task",
            "slot-1",
            "akra-agent/slot-1/task",
            "pr_pending",
            "1m",
            "pull request is open",
        );
        let detail = ParallelModeSupervisorDetailSnapshot::new(
            None,
            "no agent session history captured yet",
        );

        assert_eq!(
            slot_worker_bubble(&slot, Some(&roster_entry), &detail),
            "정리 대기"
        );
    }

    #[test]
    fn distributor_bubble_owns_delivery_pipeline_copy() {
        let cases = [
            (ParallelModeQueueItemState::Idle, "배포 파이프라인"),
            (ParallelModeQueueItemState::Queued, "배포 대기"),
            (ParallelModeQueueItemState::Pushing, "origin push 중"),
            (ParallelModeQueueItemState::PrPending, "PR 확인 중"),
            (ParallelModeQueueItemState::MergePending, "merge 준비 중"),
            (
                ParallelModeQueueItemState::Integrating,
                "통합 브랜치 반영 중",
            ),
            (ParallelModeQueueItemState::Cleaning, "slot 정리 요청"),
            (ParallelModeQueueItemState::Done, "배포 완료"),
            (ParallelModeQueueItemState::Blocked, "배포 막힘"),
            (ParallelModeQueueItemState::Failed, "배포 막힘"),
        ];

        for (state, label) in cases {
            assert_eq!(distributor_bubble(state), label);
        }
    }

    #[test]
    fn readiness_copy_defines_ready_and_blocked_operator_guidance() {
        let ready = ParallelModeReadinessSnapshot::new(
            "/tmp/workspace",
            ParallelModeReadinessState::Ready,
            Vec::new(),
            None,
        );
        let blocked = ParallelModeReadinessSnapshot::new(
            "/tmp/workspace",
            ParallelModeReadinessState::Blocked,
            Vec::new(),
            Some("integration checkout blocked".to_string()),
        );
        let degraded = ParallelModeReadinessSnapshot::new(
            "/tmp/workspace",
            ParallelModeReadinessState::Degraded,
            Vec::new(),
            Some("push readiness degraded".to_string()),
        );
        let pool = PoolBoardView {
            configured_size: 3,
            reconcile_status: "ready".to_string(),
            exhausted: false,
            summary: PoolSummaryView {
                idle: 3,
                leased: 0,
                running: 0,
                cleanup: 0,
                blocked: 0,
                missing: 0,
                unavailable: 0,
            },
            slots: Vec::new(),
        };

        assert!(readiness_notice(&ready).contains("준비 완료"));
        assert!(readiness_notice(&blocked).contains("차단됨"));
        assert!(blocked_action(&blocked, &pool).contains("readiness blocker"));
        assert!(blocked_action(&degraded, &pool).contains("최근 진단"));
    }

    #[test]
    fn runtime_event_mapping_keeps_incremental_metadata() {
        let snapshot = crate::domain::parallel_mode::ParallelModeRuntimeEventsSnapshot::new(
            vec![ParallelModeRuntimeEventEntry::new(
                42,
                "distributor_queue_blocked",
                "distributor_queue",
                "head",
                7,
                "blocked by conflict",
                "2026-05-06T17:00:00Z",
            )],
            50,
            "empty",
        );

        let feed = map_event_feed(&snapshot, 50, true);
        assert_eq!(feed.limit, 50);
        assert_eq!(feed.total_event_count, 50);
        assert_eq!(feed.visible_event_count, 1);
        assert_eq!(feed.status_label, "LIVE · 최근 1개 · 총 50개");
        assert_eq!(feed.newest_sequence, Some(42));
        assert_eq!(feed.event_cursor, Some(42));
        assert_eq!(feed.event_cursor_data, "42");
        assert!(feed.incremental);

        let unknown_cursor = map_event_feed(
            &ParallelModeRuntimeEventsSnapshot::empty("empty"),
            50,
            false,
        );
        assert_eq!(unknown_cursor.event_cursor, None);
        assert!(unknown_cursor.event_cursor_data.is_empty());

        let event = map_runtime_event(&snapshot.entries[0]);
        assert_eq!(event.icon, "event");
        assert_eq!(event.severity, "danger");
    }

    #[test]
    fn event_feed_status_label_omits_visible_count_when_feed_is_not_capped() {
        assert_eq!(event_feed_status_label(0, 0), "LIVE · 총 0개");
        assert_eq!(event_feed_status_label(2, 2), "LIVE · 총 2개");
        assert_eq!(event_feed_status_label(3, 1), "LIVE · 총 3개");
    }
}
