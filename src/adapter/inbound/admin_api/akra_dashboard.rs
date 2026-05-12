use crate::application::port::outbound::parallel_mode_runtime_event_log_port::ParallelModeRuntimeEventLogRequest;
use crate::application::service::parallel_agent_profile::{
    ParallelAgentProfileConfig, load_parallel_agent_profile_config,
};
use crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneComposition;
use crate::application::service::planning::PlanningAdminFacadeService;
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterEntry, ParallelModeDistributorQueueItem, ParallelModePoolSlotSnapshot,
    ParallelModePoolSlotState, ParallelModeQueueItemState, ParallelModeReadinessSnapshot,
    ParallelModeReadinessState, ParallelModeRuntimeEventEntry,
    ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorSnapshot,
};
use anyhow::Result;
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
    pub campaign: CampaignView,
    pub event_feed: EventFeedView,
    pub generated_at: String,
    pub generated_time_label: String,
    pub automation_epoch: i64,
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
    pub avatar_class_label: String,
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
    pub newest_sequence: Option<i64>,
    pub empty_state: String,
    pub incremental: bool,
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
    planning_admin: &PlanningAdminFacadeService,
    parallel_mode_control_plane: &ParallelModeControlPlaneComposition,
) -> Result<AkraAdminDashboardView> {
    let workspace_dir = planning_admin.workspace_dir();
    let planning_projection = planning_admin.load_runtime_application_projection()?;
    let snapshot = parallel_mode_control_plane.inspect_dashboard_snapshot_from_projection(
        workspace_dir,
        &planning_projection,
        ParallelModeRuntimeEventLogRequest::recent(DASHBOARD_EVENT_LIMIT),
    );
    let readiness = snapshot.readiness;
    let supervisor = snapshot.supervisor;
    let events = snapshot.events;
    let agent_profiles =
        load_parallel_agent_profile_config(workspace_dir).map_err(anyhow::Error::msg)?;

    let pool = map_pool(&supervisor);
    let agents = map_agents(&supervisor, &agent_profiles);
    let selected_task = map_selected_task(&supervisor);
    let distributor = map_distributor(&supervisor);
    let automation_epoch = events
        .entries
        .first()
        .map(|entry| entry.observed_planning_revision)
        .unwrap_or_default();
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
        workspace: AkraWorkspaceView {
            path: supervisor.workspace_path.clone(),
            branch: current_git_branch(workspace_dir),
            mode: "parallel".to_string(),
            readiness: readiness_label,
            readiness_notice: readiness_notice(&readiness).to_string(),
            blocked_action: blocked_action(&readiness, &pool).to_string(),
            purpose_label: "read-only 운영 관제".to_string(),
            gamification_policy: "MVP는 XP/코인/영구 레벨을 저장하지 않습니다.".to_string(),
            domain_mapping_note: "요원=Agent, 작업=Task, 워크트리 풀=Pool Slot, 분배관=Distributor"
                .to_string(),
            top_notice: supervisor
                .top_notice
                .clone()
                .or_else(|| readiness.top_alert.clone()),
        },
        kpis: AkraKpiView {
            total_tasks: planning_projection
                .has_structured_queue_projection
                .then_some(planning_projection.visible_tasks.len()),
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
        },
        pool,
        agents,
        selected_task,
        distributor,
        events,
        metrics,
        campaign,
        event_feed,
        generated_at: generated_at.to_rfc3339(),
        generated_time_label: generated_at.format("%H:%M:%S").to_string(),
        automation_epoch,
    })
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
            .enumerate()
            .map(|(index, slot)| map_pool_slot(index, slot, supervisor))
            .collect(),
    }
}

fn map_pool_slot(
    index: usize,
    slot: &ParallelModePoolSlotSnapshot,
    supervisor: &ParallelModeSupervisorSnapshot,
) -> PoolSlotView {
    let (owner_agent_id, task_id) = parse_owner_label(&slot.owner_label);
    let roster_entry = supervisor
        .roster
        .entries
        .iter()
        .find(|entry| entry.slot_id == slot.slot_id);
    PoolSlotView {
        slot_id: slot.slot_id.clone(),
        display_slot_label: pool_slot_display_label(&slot.slot_id),
        avatar_class_label: agent_class_label(index).to_string(),
        state: slot.state.label().to_string(),
        label: pool_state_korean_label(slot.state).to_string(),
        branch_name: slot.branch_name.clone(),
        worktree_label: slot.worktree_label.clone(),
        owner_label: slot.owner_label.clone(),
        owner_agent_id,
        task_id,
        note: pool_slot_note(slot),
        severity: pool_state_severity(slot.state).to_string(),
        bubble_label: slot_worker_bubble(slot, roster_entry, &supervisor.detail),
    }
}

fn map_agents(
    supervisor: &ParallelModeSupervisorSnapshot,
    agent_profiles: &ParallelAgentProfileConfig,
) -> AgentRosterView {
    AgentRosterView {
        active_count: supervisor.roster.active_count(),
        empty_state: supervisor.roster.empty_state.clone(),
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
    parallel_mode_control_plane: &ParallelModeControlPlaneComposition,
    limit: usize,
    after_sequence: Option<i64>,
) -> (EventFeedView, Vec<RuntimeEventView>) {
    let request = match after_sequence {
        Some(sequence) => {
            ParallelModeRuntimeEventLogRequest::recent(limit).after_sequence(sequence)
        }
        None => ParallelModeRuntimeEventLogRequest::recent(limit),
    };
    let events = parallel_mode_control_plane.build_runtime_events_snapshot(workspace_dir, request);
    let feed = map_event_feed(&events, limit, after_sequence.is_some());
    let entries = events.entries.iter().map(map_runtime_event).collect();
    (feed, entries)
}

fn map_event_feed(
    events: &crate::domain::parallel_mode::ParallelModeRuntimeEventsSnapshot,
    requested_limit: usize,
    incremental: bool,
) -> EventFeedView {
    EventFeedView {
        limit: requested_limit,
        total_event_count: events.total_event_count,
        visible_event_count: events.visible_count(),
        newest_sequence: events.latest().map(|entry| entry.sequence),
        empty_state: events.empty_state.clone(),
        incremental,
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
        source_label: "derived from read-only supervisor snapshot".to_string(),
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
        "진행 중인 병렬 시도는 없고 read-only 관제 대기 중".to_string()
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
    let progress = progress_percent(agent.lifecycle_state.as_str());
    CampaignLaneView {
        agent_id: agent.agent_id.clone(),
        slot_id: agent.slot_id.clone(),
        class_label: agent.class_label.clone(),
        task_title: agent.task_title.clone(),
        state: agent.lifecycle_state.clone(),
        progress_label: agent.progress_label.clone(),
        summary: agent.latest_summary.clone(),
        severity: agent_status_severity(agent.status.as_str()).to_string(),
        score_label: format!("stage {progress}/100"),
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
                score_label: format!("stage {}/100", progress_percent(entry.state_label.as_str())),
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
    } else {
        "운영 액션 없이 read-only 관제 중입니다."
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
        ParallelModeQueueItemState::Integrating => "prerelease 통합 중",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::service::parallel_agent_profile::ParallelAgentProfile;
    use crate::domain::parallel_mode::{
        ParallelModeAgentRosterSnapshot, ParallelModeAgentSessionDetailSnapshot,
        ParallelModeAgentSessionHistoryEntry, ParallelModeDistributorSnapshot,
        ParallelModeOrchestratorStatus, ParallelModePoolBoardSnapshot,
        ParallelModeRuntimeEventsSnapshot, ParallelModeSupervisorState,
    };
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

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
                ),
                ParallelModeAgentRosterEntry::new(
                    "agent-two",
                    "Investigate dashboard copy",
                    "slot-2",
                    "akra-agent/slot-2/task-2",
                    "running",
                    "2h 10m",
                    "still running",
                ),
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
                ),
                ParallelModeDistributorQueueItem::new(
                    "agent-two",
                    "Investigate dashboard copy",
                    ParallelModeQueueItemState::Blocked,
                    "akra-agent/slot-2/task-2",
                    "def5678",
                    "merge conflict",
                ),
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

    #[test]
    fn dashboard_mapping_projects_rich_supervisor_snapshot() {
        let supervisor = rich_supervisor_snapshot();
        let pool = map_pool(&supervisor);
        let agents = map_agents(&supervisor, &profile_config());
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
        assert_eq!(pool.slots[0].note, "agent-one / task-1 / slot-1");
        assert_eq!(pool.slots[0].bubble_label, "검수 통과");
        assert_eq!(pool.slots[1].owner_agent_id, None);
        assert_eq!(pool.slots[1].note, "slot-2");

        assert_eq!(agents.active_count, 2);
        assert_eq!(agents.entries[0].display_name, "Alpha");
        assert_eq!(agents.entries[0].class_label, "Seer");
        assert_eq!(agents.entries[0].role_label, "Builder");
        assert_eq!(agents.entries[0].progress_label, "75%");
        assert_eq!(agents.entries[0].bubble_label, "공식 승인");
        assert_eq!(agents.entries[1].display_name, "A02");
        assert_eq!(agents.entries[1].class_label, "Scribe");
        assert!(agents.entries[1].overload);

        assert_eq!(selected_task.task_id, "task-1");
        assert_eq!(selected_task.progress_percent, 75);
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
        assert_eq!(campaign.lane_cards[0].score_label, "stage 75/100");
        assert_eq!(campaign.attempts[0].label, "시도 #3");
        assert_eq!(campaign.attempts[0].severity, "success");
        assert_eq!(campaign.intel_cards[0].severity, "danger");
        assert_eq!(campaign.intel_cards[2].severity, "danger");
        assert_eq!(campaign.intel_cards[3].note, "latest #12");
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
        assert_eq!(
            parse_owner_label(" agent-one / task-1 "),
            (Some("agent-one".to_string()), Some("task-1".to_string()))
        );
        assert_eq!(
            parse_owner_label(" - / task-2 "),
            (None, Some("task-2".to_string()))
        );
        assert_eq!(
            parse_owner_label("agent-only"),
            (Some("agent-only".to_string()), None)
        );

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
        assert_eq!(agent_status("unknown"), "idle");
        assert_eq!(agent_bubble("official_refresh_recovery_needed"), "차단됨");
        assert_eq!(agent_bubble("cleanup_pending"), "정리중");
        assert_eq!(agent_bubble("unknown"), "대기중");

        assert_eq!(progress_percent("assigned"), 15);
        assert_eq!(progress_percent("starting"), 25);
        assert_eq!(progress_percent("cleanup_pending"), 100);
        assert_eq!(progress_percent("failed"), 35);
        assert_eq!(progress_percent("unknown"), 10);
        assert_eq!(progress_label("running"), "45%");

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
        assert_eq!(event_severity("worker_failed"), "danger");
        assert_eq!(event_severity("cleanup_completed"), "success");
        assert_eq!(event_severity("worktree_status"), "warning");
        assert_eq!(event_severity("slot_lease_upsert"), "info");
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
                "prerelease 통합 중",
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
        assert_eq!(feed.newest_sequence, Some(42));
        assert!(feed.incremental);

        let event = map_runtime_event(&snapshot.entries[0]);
        assert_eq!(event.icon, "event");
        assert_eq!(event.severity, "danger");
    }
}
