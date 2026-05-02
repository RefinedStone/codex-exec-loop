// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeSlotLeaseSnapshot,
    ParallelModeSlotLeaseState,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::domain::planning::{
    PlanningOfficialCompletionRefreshContract, PlanningOfficialCompletionRefreshPayload,
};

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::pool::{
    branch_is_cleanup_ready, cleanup_slot, resolve_workspace_head_sha, resolve_workspace_slot_lease,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::session_detail::{
    ReportedCompleteSessionDetailUpdate, record_cleaned_session_detail,
    record_commit_ready_session_detail, record_ledger_refreshing_session_detail,
    record_official_completion_failed_session_detail, record_reported_complete_session_detail,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::{
    POOL_BASELINE_BRANCH, ParallelModeOfficialCompletionReport, ParallelModeService,
    current_timestamp,
};

// 학습 주석: `impl` 블록은 특정 타입이나 trait 구현에 속한 함수들을 한곳에 묶습니다.
impl ParallelModeService {
    /*
    학습 주석: official completion 시작은 agent가 낸 최종 응답을 planning ledger 갱신 계약으로
    변환하는 단계입니다. slot worktree의 HEAD commit, validation summary, final response 요약,
    refresh order를 모아 `PlanningOfficialCompletionRefreshContract`를 만들고, 동시에 session
    detail에는 reported_complete 상태를 기록합니다.

    Running lease가 아니면 None을 반환합니다. Leased나 CleanupPending 상태에서 official
    completion이 들어오면 lifecycle 순서가 맞지 않기 때문입니다. refresh order를 예약하는
    이유는 여러 hidden official worker가 out-of-order로 시작되어도 ledger 갱신 순서를 안정적으로
    재구성하기 위해서입니다.
    */
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn begin_workspace_official_completion(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        root_turn_id: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        official_completion_refresh_order: Option<u64>,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        final_response_text: Option<&str>,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        validation_summary: Option<&str>,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        failure_context: Option<&str>,
    ) -> Result<Option<ParallelModeOfficialCompletionReport>, String> {
        /*
        학습 주석: 이 함수의 첫 번째 책임은 "이 workspace가 정말 병렬 agent slot인가"를
        authoritative lease projection으로 확인하는 것입니다. caller는 TUI turn 종료 경로에서
        workspace 문자열만 알고 들어오므로, 여기서는 pool root의 lease record와 실제 worktree
        경로를 다시 연결합니다. `None`은 오류가 아니라 "이 workspace는 parallel slot이 아니거나
        지금 완료 전이를 받을 상태가 아니다"라는 신호라서, 상위 turn service가 일반 단일 작업
        흐름으로 계속 돌아갈 수 있습니다.
        */
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        // 학습 주석: `else` 분기는 앞 조건이 실패했을 때 실행되어 흐름의 대안을 제공합니다.
        else {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(None);
        };
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(None);
        }

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let commit_sha =
            resolve_workspace_head_sha(&resolution.workspace_path).ok_or_else(|| {
                format!(
                    "slot `{}` workspace head could not be resolved for official completion",
                    resolution.lease.slot_id
                )
            })?;
        /*
        학습 주석: 여기서 commit SHA를 즉시 고정하는 이유는 planning ledger가 "무엇을
        완료로 인정했는지"를 branch name 같은 움직이는 참조가 아니라 불변 commit으로 기억해야
        하기 때문입니다. 이후 distributor가 rebase, push, PR 생성, integration worktree 병합을
        수행하더라도 official completion contract는 이 순간의 agent 산출물을 가리킵니다.
        */
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let completed_at = current_timestamp();
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let refresh_order = official_completion_refresh_order
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .map(Ok)
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .unwrap_or_else(|| {
                self.planning_authority
                    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                    .reserve_next_official_refresh_order(&resolution.lease.worktree_path)
                    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                    .map_err(|error| error.to_string())
            })?;
        /*
        학습 주석: refresh order는 hidden official completion worker들이 실제로 시작되거나
        끝나는 순서와 별개로 ledger 반영 순서를 고정하는 번호입니다. 이미 상위에서 예약한 번호가
        있으면 그대로 쓰고, 없으면 여기서 authority store를 통해 새 번호를 예약합니다. 이 값이
        contract 안으로 들어가기 때문에 recovery가 session detail이나 queue record를 다시 읽을
        때도 "어떤 완료가 먼저 ledger에 들어가야 하는지"를 재구성할 수 있습니다.
        */
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let final_response_text = normalized_optional_text(final_response_text).map(str::to_string);
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let validation_summary = normalized_optional_text(validation_summary)
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .unwrap_or("validation status was not reported by runtime")
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string();
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let failure_context = normalized_optional_text(failure_context).map(str::to_string);
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let final_response_summary = completion_summary_from_text(
            final_response_text.as_deref(),
            failure_context.as_deref(),
        );

        /*
        학습 주석: session detail은 domain의 lease 상태와 별도로 UI와 recovery가 읽는 runtime
        projection입니다. 이 시점에는 lease를 Running으로 유지하면서 detail만
        `reported_complete`로 바꿉니다. 그래야 slot은 아직 distributor에게 넘겨지지 않았다는
        사실을 보존하고, supervisor는 "agent는 끝났지만 official ledger refresh가 남았다"는
        중간 단계를 보여 줄 수 있습니다.
        */
        record_reported_complete_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
            ReportedCompleteSessionDetailUpdate {
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                completed_at: &completed_at,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                final_response_summary: &final_response_summary,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                validation_summary: &validation_summary,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                failure_context: failure_context.as_deref(),
            },
        )?;

        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Ok(Some(PlanningOfficialCompletionRefreshContract::new(
            root_turn_id,
            refresh_order,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            PlanningOfficialCompletionRefreshPayload::new(
                resolution.lease.agent_id,
                resolution.lease.task_id,
                resolution.lease.task_title,
                resolution.lease.branch_name,
                resolution.lease.worktree_path,
                commit_sha,
                validation_summary,
                final_response_summary,
                final_response_text,
                failure_context,
                completed_at,
            ),
        )))
    }

    /*
    학습 주석: hidden planning worker가 official ledger refresh를 실제로 수행하기 시작하면
    supervisor detail은 reported_complete에서 ledger_refreshing으로 넘어갑니다. 이 함수는 그
    UI-visible 상태만 기록하며, lease 자체는 Running으로 유지합니다. 아직 distributor queue에
    넣을 수 있는 commit-ready 결과가 아니기 때문입니다.
    */
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn mark_workspace_official_completion_refreshing(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeAgentSessionDetailSnapshot>, String> {
        /*
        학습 주석: refreshing 표시는 hidden planning worker가 contract를 받아 실제 authority
        갱신을 수행하기 시작했다는 runtime-only 증거입니다. 이 함수가 ledger 자체를 수정하지
        않는 이유는 planning authority 갱신은 별도 official completion worker의 책임이고,
        parallel mode service는 supervisor와 TUI가 읽을 session projection만 관리하기 때문입니다.
        */
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        // 학습 주석: `else` 분기는 앞 조건이 실패했을 때 실행되어 흐름의 대안을 제공합니다.
        else {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(None);
        };
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(None);
        }

        /*
        학습 주석: Running 상태만 허용하는 guard는 중복 또는 늦게 도착한 이벤트를 흡수합니다.
        예를 들어 slot이 이미 cleanup pending으로 넘어간 뒤 지연된 refreshing 이벤트가 오면
        session detail을 과거 상태로 되돌리면 안 되므로 `None`으로 무시합니다.
        */
        record_ledger_refreshing_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(Some)
    }

    /*
    학습 주석: ledger refresh가 완료되어 task authority가 agent 결과를 수용하면 session detail을
    commit_ready로 바꿉니다. 이 상태는 아직 통합이 끝났다는 뜻이 아니라, distributor queue에
    넣어도 되는 검증된 결과가 생겼다는 뜻입니다. 바로 뒤에서 turn service가
    `enqueue_workspace_commit_ready_result`를 호출해 queue record를 만듭니다.
    */
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn mark_workspace_commit_ready(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        authority_refresh_outcome: &str,
    ) -> Result<Option<ParallelModeAgentSessionDetailSnapshot>, String> {
        /*
        학습 주석: commit_ready는 official ledger refresh가 성공했다는 경계입니다. 이 함수는
        아직 queue item을 만들지 않고 session detail만 갱신합니다. queue enqueue를 분리해 둔
        이유는 caller가 "ledger 반영 성공"과 "distributor queue 등록 성공"을 각각 다른 runtime
        notice로 보고할 수 있고, 실패 시 어느 단계에서 멈췄는지 운영자가 구분할 수 있기 때문입니다.
        */
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        // 학습 주석: `else` 분기는 앞 조건이 실패했을 때 실행되어 흐름의 대안을 제공합니다.
        else {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(None);
        };
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(None);
        }

        /*
        학습 주석: `authority_refresh_outcome`은 planning worker가 어떤 결과로 ledger를 갱신했는지
        사람이 읽을 수 있는 문장으로 남깁니다. 이후 distributor snapshot의 completion feed는
        이 detail history를 섞어서 보여 주므로, 단순 상태명보다 원인 문구가 중요합니다.
        */
        record_commit_ready_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
            authority_refresh_outcome,
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(Some)
    }

    /*
    학습 주석: commit-ready enqueue는 `completion.rs` 입장에서 distributor service로 넘어가는
    얇은 port 역할입니다. public API는 workspace 기반으로 유지해 turn service가 내부 queue
    구조를 몰라도 되게 하고, 실제 중복 queue 방지, session_key 확인, queue record 영속화는
    distributor 모듈의 책임으로 둡니다. 이 한 줄 wrapper가 있는 덕분에 completion lifecycle
    caller는 `ParallelModeService`만 의존하면 됩니다.
    */
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn enqueue_workspace_commit_ready_result(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
    ) -> Result<Option<crate::domain::parallel_mode::ParallelModeDistributorQueueItem>, String>
    {
        self.distributor_service
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .enqueue_workspace_commit_ready_result(workspace_dir)
    }

    /*
    학습 주석: queue processing도 같은 facade 패턴입니다. completion lifecycle이 만든
    commit-ready 결과는 distributor queue head에서 push, PR, readiness check, integration,
    cleanup 순서로 소비됩니다. 여기서는 상세 단계를 숨기고 workspace_dir만 넘겨, TUI command나
    orchestrator tick이 같은 public service API를 호출하게 합니다.
    */
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn process_distributor_queue(&self, workspace_dir: &str) -> Result<Vec<String>, String> {
        self.distributor_service.process_queue(workspace_dir)
    }

    /*
    학습 주석: official refresh가 실패하면 agent 결과는 distributor로 넘어가면 안 됩니다.
    이 함수는 Running lease의 session detail을 failed로 기록하고, 실패 원인을 authority refresh
    outcome에 남깁니다. lease를 즉시 cleanup하지 않는 이유는 실패 원인을 확인하거나 재시도할
    수 있도록 slot 상태를 보존하기 위해서입니다.
    */
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn mark_workspace_official_completion_failed(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        failure_detail: &str,
    ) -> Result<Option<ParallelModeAgentSessionDetailSnapshot>, String> {
        /*
        학습 주석: failure 전이는 agent 산출물을 폐기한다는 뜻이 아니라, official ledger에 아직
        신뢰 가능한 완료로 반영되지 않았다는 뜻입니다. 그래서 lease는 Running으로 남겨 재시도나
        수동 확인 여지를 둡니다. distributor queue에 넣지 않는 것이 핵심 안전장치입니다.
        */
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        // 학습 주석: `else` 분기는 앞 조건이 실패했을 때 실행되어 흐름의 대안을 제공합니다.
        else {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(None);
        };
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(None);
        }

        /*
        학습 주석: 실패 detail은 session history에 남아 supervisor detail과 completion feed에서
        보입니다. 오류를 반환하지 않고 snapshot을 돌려주는 것은 "실패 상태 기록 자체는 성공"한
        것이기 때문에, 상위 runtime notice가 기록 실패와 official completion 실패를 혼동하지
        않게 해 줍니다.
        */
        record_official_completion_failed_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
            failure_detail,
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(Some)
    }

    /*
    학습 주석: 이 함수는 distributor나 후속 정리 경로가 "지금 cleanup pending으로 넘겨도 되는가"를
    workspace 기준으로 확인하는 안전 래퍼입니다. 이미 CleanupPending이면 그대로 Some을 반환하고,
    Running이면서 branch가 baseline에 통합된 경우에만 `mark_slot_cleanup_pending`으로 전이합니다.
    아직 통합되지 않았으면 None을 반환해 slot을 Running으로 유지합니다.
    */
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn mark_workspace_slot_cleanup_pending_if_ready(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeSlotLeaseSnapshot>, String> {
        /*
        학습 주석: cleanup pending은 "agent branch의 산출물이 baseline으로 통합되었고 이제 slot을
        idle baseline으로 되돌릴 수 있다"는 lease 상태입니다. 이 함수는 workspace만 아는 호출자를
        위해 lease resolution, state guard, branch merge 여부 확인을 한 번에 수행합니다.
        */
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        // 학습 주석: `else` 분기는 앞 조건이 실패했을 때 실행되어 흐름의 대안을 제공합니다.
        else {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(None);
        };
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if resolution.lease.state == ParallelModeSlotLeaseState::CleanupPending {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(Some(resolution.lease));
        }
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(None);
        }
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if !branch_is_cleanup_ready(&resolution.context.repo_root, &resolution.lease.branch_name) {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(None);
        }

        /*
        학습 주석: branch merge 여부를 확인한 뒤에는 slot lifecycle의 canonical 전이 함수를
        호출합니다. completion.rs가 lease 파일을 직접 수정하지 않고 `mark_slot_cleanup_pending`을
        재사용하는 이유는 session history, pool board projection, lease mirror 갱신 규칙이
        slot lifecycle 모듈에 모여 있기 때문입니다.
        */
        self.mark_slot_cleanup_pending(
            workspace_dir,
            &resolution.lease.slot_id,
            &resolution.lease.agent_id,
        )
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(Some)
    }

    /*
    학습 주석: cleanup pending 상태의 slot을 실제 idle pool로 반환하는 workspace 기반 경로입니다.
    distributor delivery가 integration과 push까지 마친 뒤 호출하거나, recovery가 이미 통합된 branch를
    발견했을 때 사용합니다. cleanup 성공 후 cleaned session detail을 남겨 completion feed와
    supervisor detail이 "slot returned to idle"까지 보여 줄 수 있게 합니다.
    */
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn cleanup_workspace_slot_if_pending(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeSlotLeaseSnapshot>, String> {
        /*
        학습 주석: 실제 cleanup은 destructive에 가까운 작업입니다. slot worktree를 baseline으로
        reset하고 lease를 idle로 되돌리는 단계이므로, 이 함수는 반드시 CleanupPending 상태에서만
        움직입니다. Running 상태를 여기서 cleanup하면 아직 통합되지 않은 agent 작업을 잃을 수
        있습니다.
        */
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        // 학습 주석: `else` 분기는 앞 조건이 실패했을 때 실행되어 흐름의 대안을 제공합니다.
        else {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(None);
        };
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if resolution.lease.state != ParallelModeSlotLeaseState::CleanupPending {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(None);
        }

        /*
        학습 주석: `cleanup_slot`은 git worktree와 authority-backed lease 상태를 함께 정리하는
        낮은 수준의 pool 작업입니다. 여기서 false를 오류로 승격하는 이유는 cleanup 실패가
        queue delivery 성공 후 slot 재사용을 막는 운영 문제이기 때문입니다. 성공하지 못했는데
        Some을 반환하면 supervisor가 slot을 재사용 가능하다고 오해할 수 있습니다.
        */
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if !cleanup_slot(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease.slot_id,
            &resolution.workspace_path,
            &resolution.lease.branch_name,
        ) {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Err(format!(
                "slot `{}` could not be reset to `{POOL_BASELINE_BRANCH}` after successful completion",
                resolution.lease.slot_id
            ));
        }
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let _ = record_cleaned_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
        );
        /*
        학습 주석: cleaned detail 기록은 best-effort입니다. slot cleanup 자체가 성공했다면 pool은
        이미 idle로 돌아갔으므로, history 기록 실패 때문에 운영 동작을 실패로 되돌리지 않습니다.
        대신 성공한 lease snapshot을 반환해 caller가 cleanup 완료를 기준으로 다음 queue item을
        진행할 수 있게 합니다.
        */

        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Ok(Some(resolution.lease))
    }
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn normalized_optional_text(text: Option<&str>) -> Option<&str> {
    /*
    학습 주석: optional text normalization은 외부 runtime에서 들어오는 빈 문자열과 실제 값의
    차이를 정리합니다. `Some("")`을 그대로 저장하면 supervisor detail에 빈 summary가 생기므로,
    공백뿐인 입력은 `None`으로 접어 기본 문구나 fallback summary가 동작하게 합니다.
    */
    text.map(str::trim).filter(|value| !value.is_empty())
}

/*
학습 주석: final response summary는 긴 agent 응답을 session detail과 distributor feed에 넣을
짧은 한 줄로 줄입니다. 가장 먼저 비어 있지 않은 응답 줄을 쓰고, 응답이 없으면 failure context를
요약으로 사용합니다. 둘 다 없을 때도 기본 문구를 만들어 UI가 빈 summary를 표시하지 않게 합니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn completion_summary_from_text(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    final_response_text: Option<&str>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    failure_context: Option<&str>,
) -> String {
    /*
    학습 주석: summary 선택 우선순위는 UI의 정보 밀도와 실패 진단을 함께 고려합니다. 정상 완료는
    agent final response의 첫 유효 줄이 가장 사용자의 의도와 가깝고, final response가 비어 있는
    실패성 완료는 failure context가 더 진단 가치가 높습니다. 마지막 기본 문구는 legacy runtime이나
    이상 이벤트에서도 feed가 빈 문자열을 표시하지 않도록 하는 방어선입니다.
    */
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if let Some(summary) = final_response_text
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .and_then(first_non_empty_line)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .filter(|summary| !summary.is_empty())
    {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return summary.to_string();
    }
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if let Some(context) = failure_context {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return format!("agent session finished with follow-up context: {context}");
    }

    "agent session reported completion without a structured final summary".to_string()
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn first_non_empty_line(text: &str) -> Option<&str> {
    /*
    학습 주석: multi-line final response를 한 줄 summary로 줄일 때는 markdown 제목, 빈 줄,
    validation log 앞의 공백 같은 형식을 제거해야 합니다. 여기서는 의미를 해석하지 않고 가장
    먼저 내용이 있는 줄만 고르므로, domain 상태 전이에 영향을 주지 않는 순수 표시용 helper입니다.
    */
    text.lines().map(str::trim).find(|line| !line.is_empty())
}
