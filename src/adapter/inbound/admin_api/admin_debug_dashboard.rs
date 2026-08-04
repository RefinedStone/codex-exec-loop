use super::AdminAppState;
use super::admin_debug_dashboard_support::{
    DEBUG_AGENT_COUNT, actor_bubble, actor_progress, actor_summary, debug_profile, event_icon,
    stage_progress, stage_severity, static_pose, visual_label, visual_severity, visual_state_key,
};
use super::akra_dashboard::{
    AgentRosterView, AgentView, AkraAdminDashboardView, CampaignAttemptView, CampaignIntelView,
    CampaignLaneView, CampaignView, DistributorPipelineStep, DistributorQueueItemView,
    DistributorView, EventFeedView, GameActorView, GameSceneView, GameStandbyCharacterView,
    GameStaticPose, GameStationView, GameVisualState, GuildMetricsView, PoolBoardView,
    PoolSlotView, PoolSummaryView, RuntimeEventView, SelectedTaskView, build_akra_dashboard_view,
    build_akra_events_view,
};
use crate::application::port::inbound::admin_debug_port::{
    AdminDebugHarnessProjection, AdminDebugScenarioOption, AdminDebugStage,
};
use chrono::Utc;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AdminDebugScenarioView {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AdminDebugHarnessView {
    pub enabled: bool,
    pub playing: bool,
    pub scenario_key: String,
    pub scenario_label: String,
    pub stage_key: String,
    pub stage_label: String,
    pub stage_summary: String,
    pub stage_index: usize,
    pub stage_count: usize,
    pub progress_percent: u8,
    pub revision: u64,
    pub step_interval_ms: u64,
    pub scenarios: Vec<AdminDebugScenarioView>,
}

impl AdminDebugHarnessView {
    pub(super) fn disabled() -> Self {
        Self {
            enabled: false,
            playing: false,
            scenario_key: String::new(),
            scenario_label: String::new(),
            stage_key: String::new(),
            stage_label: String::new(),
            stage_summary: String::new(),
            stage_index: 0,
            stage_count: 0,
            progress_percent: 0,
            revision: 0,
            step_interval_ms: 0,
            scenarios: Vec::new(),
        }
    }
}

pub(super) fn build_admin_dashboard_view(
    state: &AdminAppState,
) -> anyhow::Result<AkraAdminDashboardView> {
    let mut dashboard = build_akra_dashboard_view(
        state.facade.as_ref(),
        state.parallel_mode_admin_port.as_ref(),
        state.parallel_agent_profile_port.as_ref(),
    )?;
    let projection = state.admin_debug_port.projection();
    apply_admin_debug_harness(&mut dashboard, &projection);
    Ok(dashboard)
}

pub(super) fn build_admin_events_view(
    state: &AdminAppState,
    limit: usize,
    after_sequence: Option<i64>,
) -> (EventFeedView, Vec<RuntimeEventView>) {
    let projection = state.admin_debug_port.projection();
    if projection.enabled {
        build_debug_events_view(&projection, limit, after_sequence)
    } else {
        build_akra_events_view(
            state.facade.workspace_dir(),
            state.parallel_mode_admin_port.as_ref(),
            limit,
            after_sequence,
        )
    }
}

pub(super) fn apply_admin_debug_harness(
    dashboard: &mut AkraAdminDashboardView,
    projection: &AdminDebugHarnessProjection,
) {
    dashboard.debug_harness = map_harness_view(projection);
    if !projection.enabled {
        return;
    }

    let now = Utc::now();
    let active_count = active_agent_count(projection.stage);
    let actor_states = (0..active_count)
        .map(|index| actor_visual_state(projection.stage, index))
        .collect::<Vec<_>>();
    let agents = build_agents(projection, &actor_states);
    let pool = build_pool(projection, &actor_states);
    let distributor = build_distributor(projection);
    let events = build_all_debug_events(projection);
    let event_feed = build_event_feed(&events, events.len(), false);

    dashboard.workspace.mode = "application fake harness".to_string();
    dashboard.workspace.readiness = stage_readiness(projection.stage).to_string();
    dashboard.workspace.readiness_notice = format!(
        "DEBUG HARNESS · {} · {}",
        projection.scenario.label(),
        projection.stage.label()
    );
    dashboard.workspace.blocked_action = projection.stage.summary().to_string();
    dashboard.workspace.top_notice = Some(
        "Synthetic projection only: real planning authority and parallel control remain unchanged."
            .to_string(),
    );
    dashboard.planning_revision = Some(projection.revision as i64);
    dashboard.planning_revision_label = format!("fake rev {}", projection.revision);
    dashboard.generated_at = now.to_rfc3339();
    dashboard.generated_time_label = now.format("%H:%M:%S").to_string();
    dashboard.kpis.total_tasks = Some(DEBUG_AGENT_COUNT);
    dashboard.kpis.success_rate = stage_success_rate(projection.stage);
    dashboard.kpis.today_throughput =
        (projection.stage == AdminDebugStage::Complete).then_some(DEBUG_AGENT_COUNT);
    dashboard.kpis.active_agents = agents.active_count;
    dashboard.kpis.total_agents = DEBUG_AGENT_COUNT;
    dashboard.kpis.pool_configured_size = DEBUG_AGENT_COUNT;
    dashboard.kpis.pool_idle = pool.summary.idle;
    dashboard.kpis.pool_running = pool.summary.running;
    dashboard.kpis.pool_blocked = pool.summary.blocked;
    dashboard.kpis.queue_depth = distributor.queue_depth;
    dashboard.kpis.queue_depth_basis = "debug scenario application projection".to_string();
    dashboard.kpis.metric_source_label = "deterministic fake harness".to_string();
    dashboard.kpis.distributor_state = distributor.barrier_state.clone();
    dashboard.pool = pool;
    dashboard.scene = build_scene(projection, &actor_states);
    dashboard.selected_task = build_selected_task(projection, &actor_states);
    dashboard.agents = agents;
    dashboard.distributor = distributor;
    dashboard.events = events;
    dashboard.event_feed = event_feed;
    dashboard.metrics = build_metrics(projection, active_count);
    dashboard.campaign = build_campaign(projection, &actor_states);
}

pub(super) fn build_debug_events_view(
    projection: &AdminDebugHarnessProjection,
    limit: usize,
    after_sequence: Option<i64>,
) -> (EventFeedView, Vec<RuntimeEventView>) {
    let all_events = build_all_debug_events(projection);
    let filtered = all_events
        .iter()
        .filter(|event| {
            after_sequence
                .map(|sequence| event.sequence > sequence)
                .unwrap_or(true)
        })
        .take(limit)
        .cloned()
        .collect::<Vec<_>>();
    let total_event_count = if after_sequence.is_some() {
        filtered.len()
    } else {
        all_events.len()
    };
    (
        build_event_feed(&filtered, total_event_count, after_sequence.is_some()),
        filtered,
    )
}

pub(super) fn map_harness_view(projection: &AdminDebugHarnessProjection) -> AdminDebugHarnessView {
    AdminDebugHarnessView {
        enabled: projection.enabled,
        playing: projection.playing,
        scenario_key: projection.scenario.key().to_string(),
        scenario_label: projection.scenario.label().to_string(),
        stage_key: projection.stage.key().to_string(),
        stage_label: projection.stage.label().to_string(),
        stage_summary: projection.stage.summary().to_string(),
        stage_index: projection.stage_index,
        stage_count: projection.stage_count,
        progress_percent: projection.progress_percent(),
        revision: projection.revision,
        step_interval_ms: projection.step_interval_ms,
        scenarios: projection
            .scenarios
            .iter()
            .map(map_scenario_option)
            .collect(),
    }
}

fn map_scenario_option(option: &AdminDebugScenarioOption) -> AdminDebugScenarioView {
    AdminDebugScenarioView {
        key: option.key.to_string(),
        label: option.label.to_string(),
    }
}

fn active_agent_count(stage: AdminDebugStage) -> usize {
    match stage {
        AdminDebugStage::Ready
        | AdminDebugStage::Intake
        | AdminDebugStage::QueuePressure
        | AdminDebugStage::Complete => 0,
        _ => DEBUG_AGENT_COUNT,
    }
}

pub(super) fn stage_readiness(stage: AdminDebugStage) -> &'static str {
    match stage {
        AdminDebugStage::Blocked => "blocked",
        AdminDebugStage::QueuePressure | AdminDebugStage::Recovering => "degraded",
        _ => "ready",
    }
}

pub(super) fn actor_visual_state(stage: AdminDebugStage, index: usize) -> GameVisualState {
    match stage {
        AdminDebugStage::Dispatching | AdminDebugStage::Recovering if index == 1 => {
            GameVisualState::Starting
        }
        AdminDebugStage::Dispatching => GameVisualState::Starting,
        AdminDebugStage::Working | AdminDebugStage::Recovering => GameVisualState::Working,
        AdminDebugStage::Reviewing if index == 0 => GameVisualState::AwaitingReview,
        AdminDebugStage::Reviewing => GameVisualState::Working,
        AdminDebugStage::Blocked if index == 1 => GameVisualState::Blocked,
        AdminDebugStage::Blocked if index == 0 => GameVisualState::AwaitingReview,
        AdminDebugStage::Blocked => GameVisualState::Working,
        AdminDebugStage::Delivering if index == 0 => GameVisualState::Delivering,
        AdminDebugStage::Delivering if index == 1 => GameVisualState::AwaitingReview,
        AdminDebugStage::Delivering => GameVisualState::Working,
        AdminDebugStage::Cleanup if index == 0 => GameVisualState::Cleanup,
        AdminDebugStage::Cleanup if index == 1 => GameVisualState::Delivering,
        AdminDebugStage::Cleanup => GameVisualState::AwaitingReview,
        _ => GameVisualState::Idle,
    }
}

pub(super) fn build_pool(
    projection: &AdminDebugHarnessProjection,
    actor_states: &[GameVisualState],
) -> PoolBoardView {
    let running = actor_states
        .iter()
        .filter(|state| !matches!(state, GameVisualState::Cleanup | GameVisualState::Blocked))
        .count();
    let cleanup = actor_states
        .iter()
        .filter(|state| **state == GameVisualState::Cleanup)
        .count();
    let blocked = actor_states
        .iter()
        .filter(|state| **state == GameVisualState::Blocked)
        .count();
    let idle = DEBUG_AGENT_COUNT.saturating_sub(actor_states.len());
    let summary = PoolSummaryView {
        idle,
        leased: 0,
        running,
        cleanup,
        blocked,
        missing: 0,
        unavailable: 0,
    };
    PoolBoardView {
        configured_size: DEBUG_AGENT_COUNT,
        reconcile_status: format!(
            "debug scenario {} · stage {}",
            projection.scenario.key(),
            projection.stage.key()
        ),
        exhausted: actor_states.len() == DEBUG_AGENT_COUNT,
        slots: (0..DEBUG_AGENT_COUNT)
            .map(|index| build_pool_slot(index, actor_states.get(index).copied()))
            .collect(),
        summary,
    }
}

fn build_pool_slot(index: usize, visual_state: Option<GameVisualState>) -> PoolSlotView {
    let profile = debug_profile(index);
    let (state, label, severity, note, bubble) = match visual_state {
        Some(GameVisualState::Starting) => (
            "leased",
            "시작 준비",
            "info",
            "가짜 임대와 세션을 연결하는 중",
            "작업 준비!",
        ),
        Some(GameVisualState::Working) => (
            "running",
            "실행 중",
            "success",
            "시뮬레이션 워커가 작업 중",
            "구현 중",
        ),
        Some(GameVisualState::AwaitingReview) => (
            "running",
            "검토 대기",
            "info",
            "완료 보고와 검증 결과 대기",
            "검토 부탁!",
        ),
        Some(GameVisualState::Blocked) => (
            "blocked",
            "차단됨",
            "danger",
            "테스트 실패를 주입한 복구 시나리오",
            "도움 필요!",
        ),
        Some(GameVisualState::Delivering) => (
            "running",
            "배포 중",
            "info",
            "push와 PR 통합 단계를 시뮬레이션",
            "배포 중",
        ),
        Some(GameVisualState::Cleanup) => (
            "awaiting_cleanup",
            "정리 중",
            "warning",
            "병합된 fake worktree를 정리하는 중",
            "정리할게요",
        ),
        Some(GameVisualState::Idle) | None => (
            "idle",
            "대기",
            "muted",
            "시나리오 작업을 기다리는 빈 슬롯",
            "대기 중",
        ),
    };
    let occupied = visual_state.is_some();
    PoolSlotView {
        slot_id: format!("slot-{}", index + 1),
        display_slot_label: format!("슬롯 {}", index + 1),
        state: state.to_string(),
        label: label.to_string(),
        branch_name: if occupied {
            format!("akra-agent/slot-{}", index + 1)
        } else {
            String::new()
        },
        worktree_label: if occupied {
            format!("debug-worktree-{}", index + 1)
        } else {
            String::new()
        },
        owner_label: if occupied {
            profile.display_name
        } else {
            "없음"
        }
        .to_string(),
        owner_agent_id: occupied.then(|| profile.agent_id.to_string()),
        task_id: occupied.then(|| profile.task_id.to_string()),
        owner_session_key: occupied.then(|| format!("debug-session-{}", index + 1)),
        lease_generation: occupied.then(|| format!("debug-generation-{}", index + 1)),
        note: note.to_string(),
        severity: severity.to_string(),
        bubble_label: bubble.to_string(),
    }
}

fn build_agents(
    projection: &AdminDebugHarnessProjection,
    actor_states: &[GameVisualState],
) -> AgentRosterView {
    let entries = actor_states
        .iter()
        .copied()
        .enumerate()
        .map(|(index, state)| {
            let profile = debug_profile(index);
            AgentView {
                agent_id: profile.agent_id.to_string(),
                display_name: profile.display_name.to_string(),
                class_label: profile.archetype.to_string(),
                role_label: profile.role.to_string(),
                slot_id: format!("slot-{}", index + 1),
                task_title: profile.task_title.to_string(),
                branch_name: format!("akra-agent/slot-{}", index + 1),
                lifecycle_state: visual_state_key(state).to_string(),
                progress_label: format!("{}%", actor_progress(projection.stage, index)),
                duration_label: format!("{}s", projection.stage_index * 3 + index),
                latest_summary: actor_summary(state).to_string(),
                status: visual_state_key(state).to_string(),
                overload: false,
                bubble_label: actor_bubble(state).to_string(),
            }
        })
        .collect::<Vec<_>>();
    AgentRosterView {
        active_count: entries.len(),
        empty_state: "디버그 시나리오가 시작되면 워커가 스튜디오로 이동합니다.".to_string(),
        entries,
    }
}

pub(super) fn build_scene(
    projection: &AdminDebugHarnessProjection,
    actor_states: &[GameVisualState],
) -> GameSceneView {
    let actors = actor_states
        .iter()
        .copied()
        .enumerate()
        .map(|(index, state)| {
            let profile = debug_profile(index);
            GameActorView {
                actor_id: format!("debug-session-{}", index + 1),
                agent_id: profile.agent_id.to_string(),
                task_id: profile.task_id.to_string(),
                lease_generation: Some(format!("debug-generation-{}", index + 1)),
                slot_id: format!("slot-{}", index + 1),
                seat_index: index + 1,
                display_name: profile.display_name.to_string(),
                archetype_key: profile.archetype.to_string(),
                role_label: profile.role.to_string(),
                visual_state: state,
                static_pose: static_pose(state),
                severity: visual_severity(state).to_string(),
                status_label: visual_label(state).to_string(),
                lifecycle_state: visual_state_key(state).to_string(),
                task_title: profile.task_title.to_string(),
                branch_name: format!("akra-agent/slot-{}", index + 1),
                progress_label: format!("{}%", actor_progress(projection.stage, index)),
                duration_label: format!("{}s", projection.stage_index * 3 + index),
                latest_summary: actor_summary(state).to_string(),
                bubble_label: actor_bubble(state).to_string(),
            }
        })
        .collect::<Vec<_>>();
    let stations = (0..DEBUG_AGENT_COUNT)
        .map(|index| GameStationView {
            station_id: format!("slot-{}", index + 1),
            seat_index: index + 1,
            state: actor_states
                .get(index)
                .copied()
                .map(visual_state_key)
                .unwrap_or("idle")
                .to_string(),
            severity: actor_states
                .get(index)
                .copied()
                .map(visual_severity)
                .unwrap_or("muted")
                .to_string(),
            actor_id: actor_states
                .get(index)
                .map(|_| format!("debug-session-{}", index + 1)),
        })
        .collect();
    let standby_characters = (actor_states.len()..DEBUG_AGENT_COUNT)
        .map(|index| {
            let profile = debug_profile(index);
            GameStandbyCharacterView {
                character_id: format!("debug-standby-{}", index + 1),
                agent_id: profile.agent_id.to_string(),
                location_index: index + 1,
                display_name: profile.display_name.to_string(),
                archetype_key: profile.archetype.to_string(),
                role_label: profile.role.to_string(),
                presence_kind: "debug_standby".to_string(),
                visual_state: GameVisualState::Idle,
                static_pose: GameStaticPose::Neutral,
                severity: "muted".to_string(),
                status_label: "대기".to_string(),
                summary: "애플리케이션 Fake 시나리오 입력 대기".to_string(),
                bubble_label: "준비 완료".to_string(),
            }
        })
        .collect();
    GameSceneView {
        stations,
        actors,
        standby_profile_count: DEBUG_AGENT_COUNT,
        standby_characters,
        diagnostics: Vec::new(),
    }
}

fn build_selected_task(
    projection: &AdminDebugHarnessProjection,
    actor_states: &[GameVisualState],
) -> Option<SelectedTaskView> {
    let state = actor_states.first().copied()?;
    let profile = debug_profile(0);
    Some(SelectedTaskView {
        task_id: profile.task_id.to_string(),
        task_title: profile.task_title.to_string(),
        agent_id: profile.agent_id.to_string(),
        slot_id: "slot-1".to_string(),
        branch_name: "akra-agent/slot-1".to_string(),
        state: visual_state_key(state).to_string(),
        progress_percent: Some(actor_progress(projection.stage, 0)),
        validation_summary: if state == GameVisualState::Blocked {
            "UI contract test failed"
        } else {
            "debug contract checks passing"
        }
        .to_string(),
        latest_summary: actor_summary(state).to_string(),
        updated_at: Utc::now().to_rfc3339(),
        trail: projection
            .history
            .iter()
            .map(|record| record.stage.label().to_string())
            .collect(),
    })
}

fn build_distributor(projection: &AdminDebugHarnessProjection) -> DistributorView {
    let queue_depth = debug_queue_depth(projection.stage);
    let blocked = projection.stage == AdminDebugStage::Blocked;
    let queue_state = match projection.stage {
        AdminDebugStage::Reviewing => "review",
        AdminDebugStage::Delivering => "integrating",
        AdminDebugStage::Cleanup => "cleaning",
        AdminDebugStage::Complete => "merged",
        AdminDebugStage::Blocked => "blocked",
        _ => "idle",
    };
    let queue_items = if matches!(
        projection.stage,
        AdminDebugStage::Reviewing
            | AdminDebugStage::Delivering
            | AdminDebugStage::Cleanup
            | AdminDebugStage::Blocked
    ) {
        let profile = debug_profile(0);
        vec![DistributorQueueItemView {
            queue_item_id: Some(format!("debug-queue-{}", projection.run_id)),
            session_key: Some("debug-session-1".to_string()),
            slot_id: Some("slot-1".to_string()),
            task_id: Some(profile.task_id.to_string()),
            source_agent: profile.agent_id.to_string(),
            task_title: profile.task_title.to_string(),
            queue_state: queue_state.to_string(),
            branch_name: "akra-agent/slot-1".to_string(),
            commit_short_sha: "f4ke123".to_string(),
            integration_note: projection.stage.summary().to_string(),
        }]
    } else {
        Vec::new()
    };
    DistributorView {
        role_label: "Fake distributor · application scenario".to_string(),
        head_summary: projection.stage.summary().to_string(),
        bubble_label: if blocked {
            "게이트 차단"
        } else if projection.stage == AdminDebugStage::Complete {
            "통합 완료"
        } else {
            "시나리오 진행"
        }
        .to_string(),
        note: "실제 Git/GitHub 작업 없이 전달 상태만 투영합니다.".to_string(),
        queue_depth,
        barrier_state: if blocked { "blocked" } else { queue_state }.to_string(),
        blocked_reason: blocked.then(|| "Injected UI contract failure".to_string()),
        integration_worktree_readiness: if blocked { "needs_recovery" } else { "ready" }
            .to_string(),
        held_queue_count: usize::from(blocked),
        conflict_files: if blocked {
            vec!["assets/admin/scripts/akra-dashboard.js".to_string()]
        } else {
            Vec::new()
        },
        queue_items,
        pipeline: build_pipeline(projection.stage),
    }
}

fn build_pipeline(stage: AdminDebugStage) -> Vec<DistributorPipelineStep> {
    let active_index = match stage {
        AdminDebugStage::Reviewing => 0,
        AdminDebugStage::Blocked => 1,
        AdminDebugStage::Delivering => 4,
        AdminDebugStage::Cleanup => 5,
        AdminDebugStage::Complete => 6,
        _ => usize::MAX,
    };
    ["review", "gate", "push", "pr", "merge", "cleanup", "done"]
        .into_iter()
        .zip(["검토", "게이트", "Push", "PR", "Merge", "정리", "완료"])
        .enumerate()
        .map(|(index, (key, label))| DistributorPipelineStep {
            key: key.to_string(),
            label: label.to_string(),
            state: if active_index == usize::MAX {
                "waiting"
            } else if index < active_index || stage == AdminDebugStage::Complete {
                "complete"
            } else if index == active_index {
                if stage == AdminDebugStage::Blocked {
                    "blocked"
                } else {
                    "active"
                }
            } else {
                "waiting"
            }
            .to_string(),
        })
        .collect()
}

fn build_all_debug_events(projection: &AdminDebugHarnessProjection) -> Vec<RuntimeEventView> {
    let base = (projection.run_id.saturating_mul(1_000)) as i64;
    projection
        .history
        .iter()
        .rev()
        .map(|record| RuntimeEventView {
            sequence: base + record.index as i64 + 1,
            event_kind: format!("debug_{}", record.stage.key()),
            projection_kind: "admin_debug_harness".to_string(),
            projection_key: projection.scenario.key().to_string(),
            observed_planning_revision: projection.revision as i64,
            summary: record.stage.summary().to_string(),
            recorded_at: format!("stage {:02}", record.index + 1),
            icon: event_icon(record.stage).to_string(),
            severity: stage_severity(record.stage).to_string(),
        })
        .collect()
}

fn build_event_feed(
    visible_events: &[RuntimeEventView],
    total_event_count: usize,
    incremental: bool,
) -> EventFeedView {
    let newest_sequence = visible_events.iter().map(|event| event.sequence).max();
    EventFeedView {
        limit: visible_events.len(),
        total_event_count,
        visible_event_count: visible_events.len(),
        status_label: format!(
            "DEBUG LIVE · {} / {}",
            visible_events.len(),
            total_event_count
        ),
        newest_sequence,
        event_cursor: newest_sequence,
        event_cursor_data: newest_sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_default(),
        empty_state: "디버그 시나리오 이벤트가 없습니다.".to_string(),
        incremental,
    }
}

fn build_metrics(
    projection: &AdminDebugHarnessProjection,
    active_count: usize,
) -> GuildMetricsView {
    GuildMetricsView {
        pool_utilization_percent: active_count * 100 / DEBUG_AGENT_COUNT,
        test_success_rate: stage_success_rate(projection.stage),
        average_queue_depth: Some(debug_queue_depth(projection.stage) as f64),
        error_rate: Some(if projection.stage == AdminDebugStage::Blocked {
            33.3
        } else {
            0.0
        }),
        active_agent_count: active_count,
        waiting_task_count: debug_queue_depth(projection.stage),
        blocked_slot_count: usize::from(projection.stage == AdminDebugStage::Blocked),
        source_label: "application fake harness".to_string(),
        mock_metric_note: "Deterministic debug metrics; not production telemetry.".to_string(),
        badges: vec![
            "FAKE DATA".to_string(),
            projection.scenario.label().to_string(),
            projection.stage.label().to_string(),
        ],
    }
}

fn build_campaign(
    projection: &AdminDebugHarnessProjection,
    actor_states: &[GameVisualState],
) -> CampaignView {
    let lane_cards = actor_states
        .iter()
        .copied()
        .enumerate()
        .map(|(index, state)| {
            let profile = debug_profile(index);
            CampaignLaneView {
                agent_id: profile.agent_id.to_string(),
                slot_id: format!("slot-{}", index + 1),
                class_label: profile.archetype.to_string(),
                task_title: profile.task_title.to_string(),
                state: visual_state_key(state).to_string(),
                progress_label: format!("{}%", actor_progress(projection.stage, index)),
                summary: actor_summary(state).to_string(),
                severity: visual_severity(state).to_string(),
                score_label: format!("lane {}", index + 1),
            }
        })
        .collect::<Vec<_>>();
    CampaignView {
        summary: format!(
            "{} · {}",
            projection.scenario.label(),
            projection.stage.summary()
        ),
        attempt_count: projection.history.len(),
        visible_attempt_count: projection.history.len(),
        active_lane_count: lane_cards.len(),
        signal_count: projection.history.len(),
        lane_cards,
        attempts: projection
            .history
            .iter()
            .rev()
            .map(|record| CampaignAttemptView {
                label: record.stage.label().to_string(),
                source: "debug harness".to_string(),
                state: record.stage.key().to_string(),
                timestamp: format!("stage {:02}", record.index + 1),
                summary: record.stage.summary().to_string(),
                severity: stage_severity(record.stage).to_string(),
                score_label: format!("{}%", stage_progress(record.index, projection.stage_count)),
            })
            .collect(),
        intel_cards: vec![
            CampaignIntelView {
                label: "Scenario".to_string(),
                value: projection.scenario.label().to_string(),
                note: "application-owned deterministic state".to_string(),
                severity: "info".to_string(),
            },
            CampaignIntelView {
                label: "Stage".to_string(),
                value: projection.stage.label().to_string(),
                note: projection.stage.summary().to_string(),
                severity: stage_severity(projection.stage).to_string(),
            },
        ],
    }
}

fn debug_queue_depth(stage: AdminDebugStage) -> usize {
    match stage {
        AdminDebugStage::Intake => 3,
        AdminDebugStage::QueuePressure => 8,
        AdminDebugStage::Dispatching => 1,
        AdminDebugStage::Blocked => 2,
        _ => 0,
    }
}

fn stage_success_rate(stage: AdminDebugStage) -> Option<f64> {
    match stage {
        AdminDebugStage::Ready | AdminDebugStage::Intake | AdminDebugStage::Dispatching => None,
        AdminDebugStage::Blocked => Some(66.7),
        AdminDebugStage::Complete => Some(100.0),
        _ => Some(92.0),
    }
}
