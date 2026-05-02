use super::distributor::{ParallelModeDistributorQueueRecord, ParallelModeDistributorService};
use super::{
    PoolBoardWithContextResult, PoolRuntimeContext, build_pool_board,
    default_authority_refresh_outcome, default_supervisor_notice, default_validation_summary,
    format_elapsed_label_from_timestamp, inspect_pool_board_and_context, lease_session_key,
    pool_operator_recovery_notice, reconcile_pool_board_and_context,
};
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterSnapshot, ParallelModeAgentSessionDetailSnapshot,
    ParallelModeLiveSessionDetailDefaults, ParallelModePoolBoardSnapshot,
    ParallelModeReadinessSnapshot, ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
    ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorSnapshot,
    ParallelModeSupervisorState,
};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Default)]
/*
supervisor service는 병렬 모드의 읽기 전용 화면 모델을 조립한다. `ParallelModeService`가
lease 획득, 슬롯 정리, queue 처리 같은 명령 흐름을 담당한다면, 이 타입은 현재
pool/roster/detail/distributor 상태를 TUI가 한 번에 그릴 수 있는 snapshot으로 투영한다.
그래서 outbound adapter를 직접 다루지 않고 `PlanningAuthorityPort`와 이미 만들어진 domain
snapshot 타입만 사용한다.
*/
pub(super) struct ParallelModeSupervisorService;
impl ParallelModeSupervisorService {
    pub(super) fn new() -> Self {
        Self
    }

    /*
    `build_snapshot`은 화면을 그리기 위한 안전한 읽기 경로다. readiness가 병렬 모드를
    허용하면 pool context를 검사해 실제 slot lease와 session history를 반영하고, 허용하지
    않으면 placeholder roster/detail을 만들어 "왜 비어 있는지"를 화면에 설명한다.

    중요한 점은 이 함수가 reconcile을 실행하지 않는다는 것이다. 단순 화면 refresh가 worktree를
    만들거나 정리하는 부작용을 일으키면 사용자가 상태를 확인하는 행위만으로 저장소가 바뀐다.
    그래서 여기서는 가능한 한 현재 상태를 읽어 snapshot으로만 바꾼다.
    */
    pub(super) fn build_snapshot(
        &self,
        planning_authority: &dyn PlanningAuthorityPort,
        workspace_dir: &str,
        mode_enabled: bool,
        readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
        distributor_service: &ParallelModeDistributorService,
    ) -> ParallelModeSupervisorSnapshot {
        let state = ParallelModeSupervisorState::derive(mode_enabled, readiness_snapshot);
        let workspace_path = readiness_snapshot
            .map(|snapshot| snapshot.workspace_path.clone())
            .unwrap_or_else(|| workspace_dir.to_string());
        let (pool, roster, detail) = match readiness_snapshot {
            Some(snapshot) if snapshot.allows_parallel_mode() => build_supervisor_views(
                inspect_pool_board_and_context(planning_authority, workspace_dir),
                mode_enabled,
            ),
            _ => (
                build_pool_board(planning_authority, workspace_dir, readiness_snapshot),
                build_placeholder_roster(mode_enabled, readiness_snapshot),
                build_supervisor_detail(readiness_snapshot),
            ),
        };
        let top_notice = supervisor_top_notice(&pool, mode_enabled, readiness_snapshot);
        ParallelModeSupervisorSnapshot::new(
            state,
            workspace_path,
            pool,
            roster,
            detail,
            distributor_service.build_snapshot(workspace_dir, mode_enabled, readiness_snapshot),
            top_notice,
        )
    }

    /*
    `reconcile_snapshot`은 사용자가 병렬 모드를 켜거나 감독자 화면에서 복구성 refresh를
    요청했을 때 쓰는 쓰기 가능한 읽기 경로다. mode가 켜져 있고 readiness가 통과된 경우에만
    pool baseline, slot worktree, reusable slot 정리를 맞춘 뒤 같은 supervisor view를 만든다.

    `build_snapshot`과 반환 타입은 같지만 의미는 다르다. 하나는 관찰용이고, 이 함수는 관찰 전에
    pool runtime을 기대 형태로 수렴시키는 orchestration 진입점이다.
    */
    pub(super) fn reconcile_snapshot(
        &self,
        planning_authority: &dyn PlanningAuthorityPort,
        workspace_dir: &str,
        mode_enabled: bool,
        readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
        distributor_service: &ParallelModeDistributorService,
    ) -> ParallelModeSupervisorSnapshot {
        let state = ParallelModeSupervisorState::derive(mode_enabled, readiness_snapshot);
        let workspace_path = readiness_snapshot
            .map(|snapshot| snapshot.workspace_path.clone())
            .unwrap_or_else(|| workspace_dir.to_string());
        let (pool, roster, detail) = match readiness_snapshot {
            Some(snapshot) if snapshot.allows_parallel_mode() => {
                let runtime = if mode_enabled {
                    reconcile_pool_board_and_context(planning_authority, workspace_dir)
                } else {
                    inspect_pool_board_and_context(planning_authority, workspace_dir)
                };
                build_supervisor_views(runtime, mode_enabled)
            }
            _ => (
                build_pool_board(planning_authority, workspace_dir, readiness_snapshot),
                build_placeholder_roster(mode_enabled, readiness_snapshot),
                build_supervisor_detail(readiness_snapshot),
            ),
        };
        let top_notice = supervisor_top_notice(&pool, mode_enabled, readiness_snapshot);
        ParallelModeSupervisorSnapshot::new(
            state,
            workspace_path,
            pool,
            roster,
            detail,
            distributor_service.build_snapshot(workspace_dir, mode_enabled, readiness_snapshot),
            top_notice,
        )
    }
}

/*
supervisor top notice는 화면 상단의 한 줄 상태 메시지를 고르는 우선순위 함수다. readiness
자체가 alert를 제공하면 그것이 가장 구체적인 blocker이고, 그 다음은 pool inspection이 발견한
operator recovery 안내다. 둘 다 없을 때만 기본 안내를 사용한다. 이 순서 덕분에 사용자는
"모드가 꺼짐" 같은 일반 메시지보다 실제 복구해야 할 문제를 먼저 본다.
*/
fn supervisor_top_notice(
    pool: &ParallelModePoolBoardSnapshot,
    mode_enabled: bool,
    readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
) -> Option<String> {
    readiness_snapshot
        .and_then(|snapshot| snapshot.top_alert.clone())
        .or_else(|| pool_operator_recovery_notice(pool))
        .or_else(|| default_supervisor_notice(mode_enabled, readiness_snapshot))
}

/*
readiness가 없거나 실패한 상태에서도 TUI는 roster 영역을 빈 채로 두지 않는다. placeholder
roster는 실제 agent session이 없다는 사실과 함께 다음 행동 힌트를 담아 화면 구조를 안정적으로
유지한다. 이 함수는 "데이터 없음"과 "아직 실행 불가"를 구분해 사용자가 빈 화면을 오류로
오해하지 않게 한다.
*/
fn build_placeholder_roster(
    mode_enabled: bool,
    readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
) -> ParallelModeAgentRosterSnapshot {
    let empty_state = match (mode_enabled, readiness_snapshot) {
        (true, Some(snapshot)) if snapshot.allows_parallel_mode() => {
            "no agent sessions launched in this slice"
        }
        (true, Some(_)) => "readiness must recover before agent launch is allowed",
        (true, None) => "rerun readiness before agent launch is available",
        (false, Some(_)) => "parallel mode is off / agent roster is read-only",
        (false, None) => "parallel mode is off / no supervisor roster loaded",
    };
    ParallelModeAgentRosterSnapshot::new(Vec::new(), empty_state)
}

/*
pool inspection/reconcile은 성공하면 runtime context와 pool board를 함께 반환하고, 실패하면
이미 만들 수 있는 pool board와 오류 detail을 돌려준다. 이 함수는 그 결과를 supervisor가
필요로 하는 세 화면 조각(pool, roster, detail)으로 매핑한다. 오류가 있어도 pool board는
표시하고 roster/detail에는 unavailable 메시지를 넣어, 사용자가 부분 상태라도 볼 수 있게 하는
것이 핵심이다.
*/
fn build_supervisor_views(
    runtime: PoolBoardWithContextResult,
    mode_enabled: bool,
) -> (
    ParallelModePoolBoardSnapshot,
    ParallelModeAgentRosterSnapshot,
    ParallelModeSupervisorDetailSnapshot,
) {
    match runtime {
        Ok((context, pool)) => (
            pool,
            build_agent_roster_from_context(&context, mode_enabled),
            build_supervisor_detail_from_context(&context, mode_enabled),
        ),
        Err(error) => {
            let (pool, detail) = *error;
            (
                pool,
                ParallelModeAgentRosterSnapshot::new(
                    Vec::new(),
                    format!("agent roster unavailable / {detail}"),
                ),
                ParallelModeSupervisorDetailSnapshot::new(
                    None,
                    format!("supervisor detail unavailable / {detail}"),
                ),
            )
        }
    }
}

/*
runtime context에는 현재 lease 파일, 세션 상세 이력, distributor queue가 모두 들어 있다.
roster는 그중 "지금 슬롯에 매달린 agent들"을 중심으로 보여 주는 요약 목록이다. running
lease의 elapsed label을 별도로 계산해 넘기는 이유는 domain projection이 시간 계산 방식에
의존하지 않고, 이미 사람이 읽을 수 있는 라벨만 받아 순수하게 화면 모델을 만들도록 하기
위해서다.
*/
fn build_agent_roster_from_context(
    context: &PoolRuntimeContext,
    mode_enabled: bool,
) -> ParallelModeAgentRosterSnapshot {
    let leases = context.slot_leases.values().cloned().collect::<Vec<_>>();
    let duration_labels = running_duration_labels(&leases);
    ParallelModeAgentRosterSnapshot::project_from_leases(
        leases,
        &context.session_details,
        mode_enabled,
        &duration_labels,
    )
}
fn running_duration_labels(leases: &[ParallelModeSlotLeaseSnapshot]) -> BTreeMap<String, String> {
    leases
        .iter()
        .filter(|lease| lease.state == ParallelModeSlotLeaseState::Running)
        .filter_map(|lease| {
            let label = lease
                .running_started_at
                .as_deref()
                .and_then(format_elapsed_label_from_timestamp)?;
            Some((lease_session_key(lease), label))
        })
        .collect()
}
fn build_supervisor_detail(
    readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
) -> ParallelModeSupervisorDetailSnapshot {
    match readiness_snapshot {
        Some(_) => ParallelModeSupervisorDetailSnapshot::new(
            None,
            "readiness must recover before supervisor detail is available",
        ),
        None => ParallelModeSupervisorDetailSnapshot::new(
            None,
            "rerun readiness before supervisor detail is available",
        ),
    }
}
fn build_supervisor_detail_from_context(
    context: &PoolRuntimeContext,
    mode_enabled: bool,
) -> ParallelModeSupervisorDetailSnapshot {
    let history = context.session_details.clone();
    let queue_records = context.distributor_queue_records.clone();
    let empty_state = if mode_enabled {
        "no agent session history captured yet"
    } else {
        "parallel mode is off / supervisor detail is read-only"
    };
    ParallelModeSupervisorDetailSnapshot::new(
        selected_runtime_session_detail(context, &history, &queue_records),
        empty_state,
    )
}

/*
detail panel은 보통 하나의 세션을 깊게 보여 준다. 선택 기준은 domain 타입에 맡기되,
application 계층은 선택에 필요한 후보를 모아 준다. 현재 lease 목록은 live session 후보이고,
history는 완료되었거나 기록된 세션 후보이며, active queue session key는 통합 대기/진행 중인
결과를 우선 선택하는 힌트다.

이렇게 나누면 supervisor UI는 "지금 실행 중", "통합 큐에 있음", "이전 이력"을 같은 detail
타입으로 다루면서도 선택 정책을 domain projection에 집중시킬 수 있다.
*/
pub(super) fn selected_runtime_session_detail(
    context: &PoolRuntimeContext,
    history: &[ParallelModeAgentSessionDetailSnapshot],
    queue_records: &[ParallelModeDistributorQueueRecord],
) -> Option<ParallelModeAgentSessionDetailSnapshot> {
    let leases = context.slot_leases.values().cloned().collect::<Vec<_>>();
    let active_queue_session_key = queue_records
        .iter()
        .find(|record| record.queue_state.is_active())
        .map(|record| record.session_key.as_str());
    ParallelModeAgentSessionDetailSnapshot::select_runtime_detail(
        &leases,
        history,
        active_queue_session_key,
        live_detail_defaults(),
    )
}
fn live_detail_defaults() -> ParallelModeLiveSessionDetailDefaults<'static> {
    ParallelModeLiveSessionDetailDefaults {
        validation_summary: default_validation_summary(),
        authority_refresh_outcome: default_authority_refresh_outcome(),
    }
}
