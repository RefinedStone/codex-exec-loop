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

        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Ok(Some(resolution.lease))
    }
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn normalized_optional_text(text: Option<&str>) -> Option<&str> {
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
    text.lines().map(str::trim).find(|line| !line.is_empty())
}
