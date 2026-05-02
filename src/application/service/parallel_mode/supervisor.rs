// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use std::collections::BTreeMap;

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterSnapshot, ParallelModeAgentSessionDetailSnapshot,
    ParallelModeLiveSessionDetailDefaults, ParallelModePoolBoardSnapshot,
    ParallelModeReadinessSnapshot, ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
    ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorSnapshot,
    ParallelModeSupervisorState,
};

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::distributor::{ParallelModeDistributorQueueRecord, ParallelModeDistributorService};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::{
    PoolBoardWithContextResult, PoolRuntimeContext, build_pool_board,
    default_authority_refresh_outcome, default_supervisor_notice, default_validation_summary,
    format_elapsed_label_from_timestamp, inspect_pool_board_and_context, lease_session_key,
    pool_operator_recovery_notice, reconcile_pool_board_and_context,
};

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone, Default)]
/*
학습 주석: supervisor service는 병렬 모드의 읽기 전용 화면 모델을 조립합니다.
`ParallelModeService`가 lease 획득, 슬롯 정리, queue 처리 같은 명령 흐름을 담당한다면,
이 타입은 현재 pool/roster/detail/distributor 상태를 TUI가 한 번에 그릴 수 있는
snapshot으로 투영합니다. 그래서 outbound adapter를 직접 다루지 않고
`PlanningAuthorityPort`와 이미 만들어진 domain snapshot 타입만 사용합니다.
*/
// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
pub(super) struct ParallelModeSupervisorService;

// 학습 주석: `impl` 블록은 특정 타입이나 trait 구현에 속한 함수들을 한곳에 묶습니다.
impl ParallelModeSupervisorService {
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(super) fn new() -> Self {
        Self
    }

    /*
    학습 주석: `build_snapshot`은 화면을 그리기 위한 안전한 읽기 경로입니다. readiness가
    병렬 모드를 허용하면 pool context를 검사해 실제 slot lease와 session history를
    반영하고, 허용하지 않으면 placeholder roster/detail을 만들어 "왜 비어 있는지"를
    화면에 설명합니다.

    중요한 점은 이 함수가 reconcile을 실행하지 않는다는 것입니다. 단순 화면 refresh가
    worktree를 만들거나 정리하는 부작용을 일으키면 사용자가 상태를 확인하는 행위만으로
    저장소가 바뀝니다. 그래서 여기서는 가능한 한 현재 상태를 읽어 snapshot으로만 바꿉니다.
    */
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(super) fn build_snapshot(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        planning_authority: &dyn PlanningAuthorityPort,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        mode_enabled: bool,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        distributor_service: &ParallelModeDistributorService,
    ) -> ParallelModeSupervisorSnapshot {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let state = ParallelModeSupervisorState::derive(mode_enabled, readiness_snapshot);
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let workspace_path = readiness_snapshot
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .map(|snapshot| snapshot.workspace_path.clone())
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .unwrap_or_else(|| workspace_dir.to_string());
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let (pool, roster, detail) = match readiness_snapshot {
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            Some(snapshot) if snapshot.allows_parallel_mode() => build_supervisor_views(
                inspect_pool_board_and_context(planning_authority, workspace_dir),
                mode_enabled,
            ),
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            _ => (
                build_pool_board(planning_authority, workspace_dir, readiness_snapshot),
                build_placeholder_roster(mode_enabled, readiness_snapshot),
                build_supervisor_detail(readiness_snapshot),
            ),
        };
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let top_notice = supervisor_top_notice(&pool, mode_enabled, readiness_snapshot);

        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
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
    학습 주석: `reconcile_snapshot`은 사용자가 병렬 모드를 켜거나 감독자 화면에서 복구성
    refresh를 요청했을 때 쓰는 쓰기 가능한 읽기 경로입니다. mode가 켜져 있고 readiness가
    통과된 경우에만 pool baseline, slot worktree, reusable slot 정리를 맞춘 뒤 같은
    supervisor view를 만듭니다.

    `build_snapshot`과 반환 타입은 같지만 의미는 다릅니다. 하나는 관찰용이고, 이 함수는
    관찰 전에 pool runtime을 기대 형태로 수렴시키는 orchestration 진입점입니다.
    */
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(super) fn reconcile_snapshot(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        planning_authority: &dyn PlanningAuthorityPort,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        mode_enabled: bool,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        distributor_service: &ParallelModeDistributorService,
    ) -> ParallelModeSupervisorSnapshot {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let state = ParallelModeSupervisorState::derive(mode_enabled, readiness_snapshot);
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let workspace_path = readiness_snapshot
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .map(|snapshot| snapshot.workspace_path.clone())
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .unwrap_or_else(|| workspace_dir.to_string());
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let (pool, roster, detail) = match readiness_snapshot {
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            Some(snapshot) if snapshot.allows_parallel_mode() => {
                // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
                let runtime = if mode_enabled {
                    reconcile_pool_board_and_context(planning_authority, workspace_dir)
                } else {
                    inspect_pool_board_and_context(planning_authority, workspace_dir)
                };
                build_supervisor_views(runtime, mode_enabled)
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            _ => (
                build_pool_board(planning_authority, workspace_dir, readiness_snapshot),
                build_placeholder_roster(mode_enabled, readiness_snapshot),
                build_supervisor_detail(readiness_snapshot),
            ),
        };
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let top_notice = supervisor_top_notice(&pool, mode_enabled, readiness_snapshot);

        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
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
학습 주석: supervisor top notice는 화면 상단의 한 줄 상태 메시지를 고르는 우선순위
함수입니다. readiness 자체가 alert를 제공하면 그것이 가장 구체적인 blocker이고,
그 다음은 pool inspection이 발견한 operator recovery 안내입니다. 둘 다 없을 때만
기본 안내를 사용합니다. 이 순서 덕분에 사용자는 "모드가 꺼짐" 같은 일반 메시지보다
실제 복구해야 할 문제를 먼저 봅니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn supervisor_top_notice(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pool: &ParallelModePoolBoardSnapshot,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    mode_enabled: bool,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
) -> Option<String> {
    readiness_snapshot
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .and_then(|snapshot| snapshot.top_alert.clone())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .or_else(|| pool_operator_recovery_notice(pool))
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .or_else(|| default_supervisor_notice(mode_enabled, readiness_snapshot))
}

/*
학습 주석: readiness가 없거나 실패한 상태에서도 TUI는 roster 영역을 빈 채로 두지
않습니다. placeholder roster는 실제 agent session이 없다는 사실과 함께 다음 행동
힌트를 담아 화면 구조를 안정적으로 유지합니다. 이 함수는 "데이터 없음"과 "아직 실행
불가"를 구분해 사용자가 빈 화면을 오류로 오해하지 않게 합니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn build_placeholder_roster(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    mode_enabled: bool,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
) -> ParallelModeAgentRosterSnapshot {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let empty_state = match (mode_enabled, readiness_snapshot) {
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        (true, Some(snapshot)) if snapshot.allows_parallel_mode() => {
            "no agent sessions launched in this slice"
        }
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        (true, Some(_)) => "readiness must recover before agent launch is allowed",
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        (true, None) => "rerun readiness before agent launch is available",
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        (false, Some(_)) => "parallel mode is off / agent roster is read-only",
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        (false, None) => "parallel mode is off / no supervisor roster loaded",
    };

    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    ParallelModeAgentRosterSnapshot::new(Vec::new(), empty_state)
}

/*
학습 주석: pool inspection/reconcile은 성공하면 runtime context와 pool board를 함께
반환하고, 실패하면 이미 만들 수 있는 pool board와 오류 detail을 돌려줍니다. 이 함수는
그 결과를 supervisor가 필요로 하는 세 화면 조각(pool, roster, detail)으로 매핑합니다.
오류가 있어도 pool board는 표시하고 roster/detail에는 unavailable 메시지를 넣어,
사용자가 부분 상태라도 볼 수 있게 하는 것이 핵심입니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn build_supervisor_views(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    runtime: PoolBoardWithContextResult,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    mode_enabled: bool,
) -> (
    ParallelModePoolBoardSnapshot,
    ParallelModeAgentRosterSnapshot,
    ParallelModeSupervisorDetailSnapshot,
) {
    // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
    match runtime {
        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Ok((context, pool)) => (
            pool,
            build_agent_roster_from_context(&context, mode_enabled),
            build_supervisor_detail_from_context(&context, mode_enabled),
        ),
        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Err(error) => {
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let (pool, detail) = *error;
            (
                pool,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                ParallelModeAgentRosterSnapshot::new(
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    Vec::new(),
                    format!("agent roster unavailable / {detail}"),
                ),
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                ParallelModeSupervisorDetailSnapshot::new(
                    None,
                    format!("supervisor detail unavailable / {detail}"),
                ),
            )
        }
    }
}

/*
학습 주석: runtime context에는 현재 lease 파일, 세션 상세 이력, distributor queue가
모두 들어 있습니다. roster는 그중 "지금 슬롯에 매달린 agent들"을 중심으로 보여 주는
요약 목록입니다. running lease의 elapsed label을 별도로 계산해 넘기는 이유는 domain
projection이 시간 계산 방식에 의존하지 않고, 이미 사람이 읽을 수 있는 라벨만 받아
순수하게 화면 모델을 만들도록 하기 위해서입니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn build_agent_roster_from_context(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    context: &PoolRuntimeContext,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    mode_enabled: bool,
) -> ParallelModeAgentRosterSnapshot {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let leases = context.slot_leases.values().cloned().collect::<Vec<_>>();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let duration_labels = running_duration_labels(&leases);
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    ParallelModeAgentRosterSnapshot::project_from_leases(
        leases,
        &context.session_details,
        mode_enabled,
        &duration_labels,
    )
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn running_duration_labels(leases: &[ParallelModeSlotLeaseSnapshot]) -> BTreeMap<String, String> {
    leases
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .iter()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .filter(|lease| lease.state == ParallelModeSlotLeaseState::Running)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .filter_map(|lease| {
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let label = lease
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .running_started_at
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .as_deref()
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .and_then(format_elapsed_label_from_timestamp)?;
            Some((lease_session_key(lease), label))
        })
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .collect()
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn build_supervisor_detail(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
) -> ParallelModeSupervisorDetailSnapshot {
    // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
    match readiness_snapshot {
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        Some(_) => ParallelModeSupervisorDetailSnapshot::new(
            None,
            "readiness must recover before supervisor detail is available",
        ),
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        None => ParallelModeSupervisorDetailSnapshot::new(
            None,
            "rerun readiness before supervisor detail is available",
        ),
    }
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn build_supervisor_detail_from_context(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    context: &PoolRuntimeContext,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    mode_enabled: bool,
) -> ParallelModeSupervisorDetailSnapshot {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let history = context.session_details.clone();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let queue_records = context.distributor_queue_records.clone();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let empty_state = if mode_enabled {
        "no agent session history captured yet"
    } else {
        "parallel mode is off / supervisor detail is read-only"
    };

    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    ParallelModeSupervisorDetailSnapshot::new(
        selected_runtime_session_detail(context, &history, &queue_records),
        empty_state,
    )
}

/*
학습 주석: detail panel은 보통 하나의 세션을 깊게 보여 줍니다. 선택 기준은 domain 타입에
맡기되, application 계층은 선택에 필요한 후보를 모아 줍니다. 현재 lease 목록은 live
session 후보이고, history는 완료되었거나 기록된 세션 후보이며, active queue session key는
통합 대기/진행 중인 결과를 우선 선택하는 힌트입니다.

이렇게 나누면 supervisor UI는 "지금 실행 중", "통합 큐에 있음", "이전 이력"을 같은
detail 타입으로 다루면서도 선택 정책을 domain projection에 집중시킬 수 있습니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn selected_runtime_session_detail(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    context: &PoolRuntimeContext,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    history: &[ParallelModeAgentSessionDetailSnapshot],
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    queue_records: &[ParallelModeDistributorQueueRecord],
) -> Option<ParallelModeAgentSessionDetailSnapshot> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let leases = context.slot_leases.values().cloned().collect::<Vec<_>>();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let active_queue_session_key = queue_records
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .iter()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .find(|record| record.queue_state.is_active())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|record| record.session_key.as_str());
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    ParallelModeAgentSessionDetailSnapshot::select_runtime_detail(
        &leases,
        history,
        active_queue_session_key,
        live_detail_defaults(),
    )
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn live_detail_defaults() -> ParallelModeLiveSessionDetailDefaults<'static> {
    ParallelModeLiveSessionDetailDefaults {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        validation_summary: default_validation_summary(),
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        authority_refresh_outcome: default_authority_refresh_outcome(),
    }
}
