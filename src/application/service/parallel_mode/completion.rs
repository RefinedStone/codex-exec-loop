// completion 서비스는 슬롯 lease 상태와 session detail snapshot을 함께 갱신한다.
// lease는 slot lifecycle의 권위 상태이고, session detail은 TUI/recovery가 읽는 관찰용 projection이다.
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeSlotLeaseSnapshot,
    ParallelModeSlotLeaseState,
};
// official completion은 agent 산출물을 planning ledger refresh 계약으로 넘기는 경계이다.
// 이 타입들이 hidden official worker에게 전달될 payload와 report의 application 계약을 정의한다.
use crate::domain::planning::{
    PlanningOfficialCompletionRefreshContract, PlanningOfficialCompletionRefreshPayload,
};

// pool helper는 slot worktree와 lease projection을 다시 연결하고 cleanup 가능 여부를 판정한다.
use super::pool::{
    branch_is_cleanup_ready, cleanup_slot, resolve_workspace_head_sha, resolve_workspace_slot_lease,
};
// session detail helper들은 completion lifecycle의 UI-visible 상태 전이를 runtime projection에 기록한다.
use super::session_detail::{
    ReportedCompleteSessionDetailUpdate, record_cleaned_session_detail,
    record_commit_ready_session_detail, record_ledger_refreshing_session_detail,
    record_official_completion_failed_session_detail, record_reported_complete_session_detail,
};
// completion 흐름은 parallel mode service 본체와 pool baseline branch, timestamp helper를 공유한다.
use super::{
    POOL_BASELINE_BRANCH, ParallelModeOfficialCompletionReport, ParallelModeService,
    current_timestamp,
};

// 이 impl 조각은 parallel slot이 "작업 실행 완료"에서 "ledger 반영, queue 통합, cleanup"으로
// 넘어가는 완료 파이프라인을 담당한다. lease 상태와 session detail projection을 함께 움직이는 것이 핵심이다.
impl ParallelModeService {
    /*
    official completion 시작은 agent가 낸 최종 응답을 planning ledger 갱신 계약으로
    변환하는 단계이다. slot worktree의 HEAD commit, validation summary, final response 요약,
    refresh order를 모아 `PlanningOfficialCompletionRefreshContract`를 만들고, 동시에 session
    detail에는 reported_complete 상태를 기록한다.

    Running lease가 아니면 None을 반환한다. Leased나 CleanupPending 상태에서 official
    completion이 들어오면 lifecycle 순서가 맞지 않기 때문이다. refresh order를 예약하는
    이유는 여러 hidden official worker가 out-of-order로 시작되어도 ledger 갱신 순서를 안정적으로
    재구성하기 위해서이다.
    */
    // 슬롯 workspace에서 들어온 완료 신호를 official planning refresh 계약으로 변환한다.
    pub fn begin_workspace_official_completion(
        &self,
        // 완료를 보고한 workspace 경로이다. 이 값으로 어떤 slot lease인지 다시 역추적한다.
        workspace_dir: &str,
        // official completion contract를 유발한 완료 turn id이다.
        completed_turn_id: &str,
        // 상위 런타임이 이미 예약한 refresh 순번이다. 없으면 이 함수가 authority store에서 새로 예약한다.
        official_completion_refresh_order: Option<u64>,
        // agent가 사용자에게 낸 최종 응답이다. ledger payload와 session summary 생성에 사용된다.
        final_response_text: Option<&str>,
        // 테스트/검증 결과 요약이다. 없으면 runtime이 보고하지 않았다는 기본 문장을 넣는다.
        validation_summary: Option<&str>,
        // 실패 완료나 제한적 완료일 때 ledger와 UI에 남길 추가 맥락이다.
        failure_context: Option<&str>,
    ) -> Result<Option<ParallelModeOfficialCompletionReport>, String> {
        /*
        이 함수의 첫 번째 책임은 "이 workspace가 정말 병렬 agent slot인가"를
        authoritative lease projection으로 확인하는 것이다. caller는 TUI turn 종료 경로에서
        workspace 문자열만 알고 들어오므로, 여기서는 pool root의 lease record와 실제 worktree
        경로를 다시 연결한다. `None`은 오류가 아니라 "이 workspace는 parallel slot이 아니거나
        지금 완료 전이를 받을 상태가 아니다"라는 신호라서, 상위 turn service가 일반 단일 작업
        흐름으로 계속 돌아갈 수 있다.
        */
        // workspace 경로를 authority projection에 등록된 slot lease로 해석한다.
        // None이면 parallel slot 완료가 아니므로 caller가 일반 completion 경로를 계속 시도할 수 있게 한다.
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        // Running lease만 agent가 실제 작업을 끝냈다고 보고할 수 있다.
        // 다른 상태는 completion lifecycle 순서가 맞지 않으므로 no-op으로 돌려보낸다.
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            return Ok(None);
        }

        // 완료 payload는 branch 이름이 아니라 현재 worktree HEAD commit을 기준으로 고정한다.
        // HEAD를 읽지 못하면 official ledger가 무엇을 반영해야 하는지 알 수 없으므로 오류로 중단한다.
        let commit_sha =
            resolve_workspace_head_sha(&resolution.workspace_path).ok_or_else(|| {
                format!(
                    "slot `{}` workspace head could not be resolved for official completion",
                    resolution.lease.slot_id
                )
            })?;
        /*
        여기서 commit SHA를 즉시 고정하는 이유는 planning ledger가 "무엇을
        완료로 인정했는지"를 branch name 같은 움직이는 참조가 아니라 불변 commit으로 기억해야
        하기 때문이다. 이후 distributor가 rebase, push, PR 생성, integration worktree 병합을
        수행하더라도 official completion contract는 이 순간의 agent 산출물을 가리킨다.
        */
        // completed_at은 session detail, contract payload, later queue state를 연결하는 완료 시각이다.
        let completed_at = current_timestamp();
        // refresh order는 이미 예약된 값이 있으면 재사용하고, 없으면 authority store에서 단조 증가 번호를 받는다.
        // `map(Ok)`는 Option<u64>를 Option<Result<u64, String>> 형태로 맞춰 `unwrap_or_else`의 예약 결과와 결합한다.
        let refresh_order = official_completion_refresh_order
            .map(Ok)
            .unwrap_or_else(|| {
                self.planning_authority
                    .reserve_next_official_refresh_order(&resolution.lease.worktree_path)
                    .map_err(|error| error.to_string())
            })?;
        /*
        refresh order는 hidden official completion worker들이 실제로 시작되거나
        끝나는 순서와 별개로 ledger 반영 순서를 고정하는 번호이다. 이미 상위에서 예약한 번호가
        있으면 그대로 쓰고, 없으면 여기서 authority store를 통해 새 번호를 예약한다. 이 값이
        contract 안으로 들어가기 때문에 recovery가 session detail이나 queue record를 다시 읽을
        때도 "어떤 완료가 먼저 ledger에 들어가야 하는지"를 재구성할 수 있다.
        */
        // 공백뿐인 final response는 없는 값으로 정규화해 ledger payload가 의미 없는 문자열을 들고 가지 않게 한다.
        let final_response_text = normalized_optional_text(final_response_text).map(str::to_string);
        // validation summary는 ledger와 UI에 항상 표시할 문자열이 필요하므로 기본 문장을 제공한다.
        let validation_summary = normalized_optional_text(validation_summary)
            .unwrap_or("validation status was not reported by runtime")
            .to_string();
        // failure context도 공백을 제거한 Option으로 맞춰, 실패 맥락이 없을 때 summary fallback이 깨끗하게 동작한다.
        let failure_context = normalized_optional_text(failure_context).map(str::to_string);
        // UI용 짧은 summary는 final response의 첫 줄을 우선하고, 없으면 failure context를 fallback으로 사용한다.
        let final_response_summary = completion_summary_from_text(
            final_response_text.as_deref(),
            failure_context.as_deref(),
        );

        /*
        session detail은 domain의 lease 상태와 별도로 UI와 recovery가 읽는 runtime
        projection이다. 이 시점에는 lease를 Running으로 유지하면서 detail만
        `reported_complete`로 바꾼다. 그래야 slot은 아직 distributor에게 넘겨지지 않았다는
        사실을 보존하고, supervisor는 "agent는 끝났지만 official ledger refresh가 남았다"는
        중간 단계를 보여 줄 수 있다.
        */
        record_reported_complete_session_detail(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
            ReportedCompleteSessionDetailUpdate {
                // session detail의 reported_complete 시각이다.
                completed_at: &completed_at,
                // TUI 목록에서 긴 final response 대신 보여줄 압축된 완료 설명이다.
                final_response_summary: &final_response_summary,
                // official ledger refresh 전에 이미 관찰된 검증 결과이다.
                validation_summary: &validation_summary,
                // 실패 맥락은 선택 값으로 보존해 성공 완료의 session detail을 불필요하게 오염시키지 않는다.
                failure_context: failure_context.as_deref(),
            },
        )?;

        // 반환값은 hidden official worker가 ledger refresh를 수행할 계약이다.
        // session detail 갱신과 contract 생성이 같은 입력에서 만들어져 recovery 시 서로 맞물린다.
        Ok(Some(PlanningOfficialCompletionRefreshContract::new(
            completed_turn_id,
            refresh_order,
            // payload에는 slot identity, task identity, branch/worktree, 고정 commit, 완료 요약이 모두 들어간다.
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
    hidden planning worker가 official ledger refresh를 실제로 수행하기 시작하면
    supervisor detail은 reported_complete에서 ledger_refreshing으로 넘어간다. 이 함수는 그
    UI-visible 상태만 기록하며, lease 자체는 Running으로 유지한다. 아직 distributor queue에
    넣을 수 있는 commit-ready 결과가 아니기 때문이다.
    */
    // hidden official worker가 ledger refresh를 시작했음을 session projection에 표시한다.
    pub fn mark_workspace_official_completion_refreshing(
        &self,
        // refreshing 표시를 적용할 parallel slot workspace이다.
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeAgentSessionDetailSnapshot>, String> {
        /*
        refreshing 표시는 hidden planning worker가 contract를 받아 실제 authority
        갱신을 수행하기 시작했다는 runtime-only 증거이다. 이 함수가 ledger 자체를 수정하지
        않는 이유는 planning authority 갱신은 별도 official completion worker의 책임이고,
        parallel mode service는 supervisor와 TUI가 읽을 session projection만 관리하기 때문이다.
        */
        // workspace가 parallel slot으로 해석되지 않으면 이 transition의 대상이 아니므로 None으로 빠진다.
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        // Running이 아니면 이미 완료/정리 쪽으로 전이되었거나 아직 실행 중이 아니므로 과거 상태로 되돌리지 않는다.
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            return Ok(None);
        }

        /*
        Running 상태만 허용하는 guard는 중복 또는 늦게 도착한 이벤트를 흡수한다.
        예를 들어 slot이 이미 cleanup pending으로 넘어간 뒤 지연된 refreshing 이벤트가 오면
        session detail을 과거 상태로 되돌리면 안 되므로 `None`으로 무시한다.
        */
        record_ledger_refreshing_session_detail(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
        )
        // helper가 만든 snapshot을 Option으로 감싸 public API의 "대상 없음" 표현과 맞춘다.
        .map(Some)
    }

    /*
    ledger refresh가 완료되어 task authority가 agent 결과를 수용하면 session detail을
    commit_ready로 바꾼다. 이 상태는 아직 통합이 끝났다는 뜻이 아니라, distributor queue에
    넣어도 되는 검증된 결과가 생겼다는 뜻이다. 바로 뒤에서 turn service가
    `enqueue_workspace_commit_ready_result`를 호출해 queue record를 만든다.
    */
    // official ledger refresh 성공 후 slot session을 commit_ready로 표시한다.
    pub fn mark_workspace_commit_ready(
        &self,
        // commit_ready 상태로 표시할 parallel slot workspace이다.
        workspace_dir: &str,
        // planning authority refresh가 어떤 결과로 끝났는지 session history에 남길 문장이다.
        authority_refresh_outcome: &str,
    ) -> Result<Option<ParallelModeAgentSessionDetailSnapshot>, String> {
        /*
        commit_ready는 official ledger refresh가 성공했다는 경계이다. 이 함수는
        아직 queue item을 만들지 않고 session detail만 갱신한다. queue enqueue를 분리해 둔
        이유는 caller가 "ledger 반영 성공"과 "distributor queue 등록 성공"을 각각 다른 runtime
        notice로 보고할 수 있고, 실패 시 어느 단계에서 멈췄는지 운영자가 구분할 수 있기 때문이다.
        */
        // commit_ready도 workspace 문자열을 lease projection으로 다시 해석한 뒤에만 적용한다.
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        // Running lease만 commit_ready로 갈 수 있다.
        // cleanup pending 같은 후속 상태가 된 slot은 늦은 성공 이벤트로 되돌리면 안 된다.
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            return Ok(None);
        }

        /*
        `authority_refresh_outcome`은 planning worker가 어떤 결과로 ledger를 갱신했는지
        사람이 읽을 수 있는 문장으로 남긴다. 이후 distributor snapshot의 completion feed는
        이 detail history를 섞어서 보여 주므로, 단순 상태명보다 원인 문구가 중요하다.
        */
        record_commit_ready_session_detail(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
            authority_refresh_outcome,
        )
        // session detail helper의 Result를 그대로 올리되, 성공 값은 Option으로 감싸 facade 계약을 유지한다.
        .map(Some)
    }

    /*
    commit-ready enqueue는 `completion.rs` 입장에서 distributor service로 넘어가는
    얇은 port 역할이다. public API는 workspace 기반으로 유지해 turn service가 내부 queue
    구조를 몰라도 되게 하고, 실제 중복 queue 방지, session_key 확인, queue record 영속화는
    distributor 모듈의 책임으로 둔다. 이 한 줄 wrapper가 있는 덕분에 completion lifecycle
    caller는 `ParallelModeService`만 의존하면 된다.
    */
    // commit_ready로 표시된 workspace를 distributor queue에 등록하도록 위임한다.
    pub fn enqueue_workspace_commit_ready_result(
        &self,
        // queue item으로 바꿀 slot workspace이다.
        workspace_dir: &str,
    ) -> Result<Option<crate::domain::parallel_mode::ParallelModeDistributorQueueItem>, String>
    {
        // completion service는 public facade만 제공하고, 중복 방지와 queue row 작성은 distributor service가 맡는다.
        self.distributor_service
            .enqueue_workspace_commit_ready_result(workspace_dir)
    }

    /*
    queue processing도 같은 facade 패턴이다. completion lifecycle이 만든
    commit-ready 결과는 distributor queue head에서 push, PR, readiness check, integration,
    cleanup 순서로 소비된다. 여기서는 상세 단계를 숨기고 workspace_dir만 넘겨, TUI command나
    orchestrator tick이 같은 public service API를 호출하게 한다.
    */
    // distributor queue head를 처리하도록 distributor service에 위임한다.
    pub fn process_distributor_queue(&self, workspace_dir: &str) -> Result<Vec<String>, String> {
        // push/PR/integration/cleanup 같은 세부 단계는 distributor module에 숨기고 notice 목록만 반환한다.
        self.distributor_service.process_queue(workspace_dir)
    }

    /*
    official refresh가 실패하면 agent 결과는 distributor로 넘어가면 안 된다.
    이 함수는 Running lease의 session detail을 failed로 기록하고, 실패 원인을 authority refresh
    outcome에 남긴다. lease를 즉시 cleanup하지 않는 이유는 실패 원인을 확인하거나 재시도할
    수 있도록 slot 상태를 보존하기 위해서이다.
    */
    // official ledger refresh 실패를 session projection에 기록하고 distributor 전이를 막는다.
    pub fn mark_workspace_official_completion_failed(
        &self,
        // 실패 상태를 남길 parallel slot workspace이다.
        workspace_dir: &str,
        // official worker가 반환한 실패 원인이다. session history와 UI feed에 남는다.
        failure_detail: &str,
    ) -> Result<Option<ParallelModeAgentSessionDetailSnapshot>, String> {
        /*
        failure 전이는 agent 산출물을 폐기한다는 뜻이 아니라, official ledger에 아직
        신뢰 가능한 완료로 반영되지 않았다는 뜻이다. 그래서 lease는 Running으로 남겨 재시도나
        수동 확인 여지를 둔다. distributor queue에 넣지 않는 것이 핵심 안전장치이다.
        */
        // 실패 전이도 parallel slot workspace로 해석되는 경우에만 적용한다.
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        // Running 상태가 아니면 이미 다른 lifecycle 전이가 적용된 것이므로 실패 이벤트로 되돌리지 않는다.
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            return Ok(None);
        }

        /*
        실패 detail은 session history에 남아 supervisor detail과 completion feed에서
        보인다. 오류를 반환하지 않고 snapshot을 돌려주는 것은 "실패 상태 기록 자체는 성공"한
        것이기 때문에, 상위 runtime notice가 기록 실패와 official completion 실패를 혼동하지
        않게 해 준다.
        */
        record_official_completion_failed_session_detail(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
            failure_detail,
        )
        // 실패 기록이 성공하면 새 session detail snapshot을 Some으로 돌려 caller가 notice를 만들 수 있게 한다.
        .map(Some)
    }

    /*
    이 함수는 distributor나 후속 정리 경로가 "지금 cleanup pending으로 넘겨도 되는가"를
    workspace 기준으로 확인하는 안전 래퍼이다. 이미 CleanupPending이면 그대로 Some을 반환하고,
    Running이면서 branch가 baseline에 통합된 경우에만 `mark_slot_cleanup_pending`으로 전이한다.
    아직 통합되지 않았으면 None을 반환해 slot을 Running으로 유지한다.
    */
    // 통합이 끝난 slot을 cleanup 가능한 상태로 전이할지 workspace 기준으로 판단한다.
    pub fn mark_workspace_slot_cleanup_pending_if_ready(
        &self,
        // cleanup pending 후보가 되는 slot workspace이다.
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeSlotLeaseSnapshot>, String> {
        /*
        cleanup pending은 "agent branch의 산출물이 baseline으로 통합되었고 이제 slot을
        idle baseline으로 되돌릴 수 있다"는 lease 상태이다. 이 함수는 workspace만 아는 호출자를
        위해 lease resolution, state guard, branch merge 여부 확인을 한 번에 수행한다.
        */
        // workspace가 slot lease로 해석되지 않으면 cleanup lifecycle 대상이 아니므로 None이다.
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        // 이미 CleanupPending이면 이 함수는 멱등적으로 현재 lease snapshot을 그대로 돌려준다.
        if resolution.lease.state == ParallelModeSlotLeaseState::CleanupPending {
            return Ok(Some(resolution.lease));
        }
        // Running이 아닌 다른 상태는 cleanup pending으로 바로 갈 수 없는 lifecycle 상태이다.
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            return Ok(None);
        }
        // agent branch가 baseline에 통합되기 전에는 worktree를 정리하면 산출물을 잃을 수 있다.
        if !branch_is_cleanup_ready(&resolution.context.repo_root, &resolution.lease.branch_name) {
            return Ok(None);
        }

        /*
        branch merge 여부를 확인한 뒤에는 slot lifecycle의 canonical 전이 함수를
        호출한다. completion.rs가 lease 파일을 직접 수정하지 않고 `mark_slot_cleanup_pending`을
        재사용하는 이유는 session history, pool board projection, lease mirror 갱신 규칙이
        slot lifecycle 모듈에 모여 있기 때문이다.
        */
        self.mark_slot_cleanup_pending(
            workspace_dir,
            &resolution.lease.slot_id,
            &resolution.lease.agent_id,
        )
        // lifecycle helper의 성공 snapshot을 Option으로 감싸 workspace 기반 facade 계약에 맞춘다.
        .map(Some)
    }

    /*
    cleanup pending 상태의 slot을 실제 idle pool로 반환하는 workspace 기반 경로이다.
    distributor delivery가 integration과 push까지 마친 뒤 호출하거나, recovery가 이미 통합된 branch를
    발견했을 때 사용한다. cleanup 성공 후 cleaned session detail을 남겨 completion feed와
    supervisor detail이 "slot returned to idle"까지 보여 줄 수 있게 한다.
    */
    // CleanupPending slot을 실제로 idle baseline 상태로 되돌린다.
    pub fn cleanup_workspace_slot_if_pending(
        &self,
        // cleanup을 시도할 slot workspace이다.
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeSlotLeaseSnapshot>, String> {
        /*
        실제 cleanup은 destructive에 가까운 작업이다. slot worktree를 baseline으로
        reset하고 lease를 idle로 되돌리는 단계이므로, 이 함수는 반드시 CleanupPending 상태에서만
        움직인다. Running 상태를 여기서 cleanup하면 아직 통합되지 않은 agent 작업을 잃을 수
        있다.
        */
        // cleanup은 authority projection에 등록된 slot에서만 수행한다.
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        // CleanupPending이 아니면 slot 정리를 실행하지 않는다.
        // 이 guard가 Running 작업의 worktree reset을 막는 마지막 방어선이다.
        if resolution.lease.state != ParallelModeSlotLeaseState::CleanupPending {
            return Ok(None);
        }

        /*
        `cleanup_slot`은 git worktree와 authority-backed lease 상태를 함께 정리하는
        낮은 수준의 pool 작업이다. 여기서 false를 오류로 승격하는 이유는 cleanup 실패가
        queue delivery 성공 후 slot 재사용을 막는 운영 문제이기 때문이다. 성공하지 못했는데
        Some을 반환하면 supervisor가 slot을 재사용 가능하다고 오해할 수 있다.
        */
        // pool helper가 false를 돌려주면 baseline reset 또는 lease 정리가 실패한 것이다.
        // caller가 slot을 재사용하지 않도록 오류로 승격한다.
        if !cleanup_slot(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease.slot_id,
            &resolution.workspace_path,
            &resolution.lease.branch_name,
        ) {
            return Err(format!(
                "slot `{}` could not be reset to `{POOL_BASELINE_BRANCH}` after successful completion",
                resolution.lease.slot_id
            ));
        }
        // cleanup 성공 후 session detail에도 cleaned 이벤트를 남긴다.
        // 이 기록은 관찰용이므로 실패해도 이미 완료된 slot reset을 되돌리지 않는다.
        let _ = record_cleaned_session_detail(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
        );
        /*
        cleaned detail 기록은 best-effort이다. slot cleanup 자체가 성공했다면 pool은
        이미 idle로 돌아갔으므로, history 기록 실패 때문에 운영 동작을 실패로 되돌리지 않는다.
        대신 성공한 lease snapshot을 반환해 caller가 cleanup 완료를 기준으로 다음 queue item을
        진행할 수 있게 한다.
        */

        // cleanup이 끝난 slot lease snapshot을 반환해 caller가 완료 notice나 다음 queue tick을 만들 수 있게 한다.
        Ok(Some(resolution.lease))
    }
}

// 외부 runtime에서 들어온 optional text를 "실제 내용이 있는 값"만 남기도록 정규화한다.
fn normalized_optional_text(text: Option<&str>) -> Option<&str> {
    /*
    optional text normalization은 외부 runtime에서 들어오는 빈 문자열과 실제 값의
    차이를 정리한다. `Some("")`을 그대로 저장하면 supervisor detail에 빈 summary가 생기므로,
    공백뿐인 입력은 `None`으로 접어 기본 문구나 fallback summary가 동작하게 한다.
    */
    text.map(str::trim).filter(|value| !value.is_empty())
}

/*
final response summary는 긴 agent 응답을 session detail과 distributor feed에 넣을
짧은 한 줄로 줄인다. 가장 먼저 비어 있지 않은 응답 줄을 쓰고, 응답이 없으면 failure context를
요약으로 사용한다. 둘 다 없을 때도 기본 문구를 만들어 UI가 빈 summary를 표시하지 않게 한다.
*/
// final response와 failure context에서 session feed에 표시할 짧은 completion summary를 고른다.
fn completion_summary_from_text(
    // agent가 남긴 최종 사용자 응답이다. 가장 우선되는 summary 원천이다.
    final_response_text: Option<&str>,
    // final response가 없을 때 실패/후속 조치 맥락을 summary로 승격하기 위한 fallback이다.
    failure_context: Option<&str>,
) -> String {
    /*
    summary 선택 우선순위는 UI의 정보 밀도와 실패 진단을 함께 고려한다. 정상 완료는
    agent final response의 첫 유효 줄이 가장 사용자의 의도와 가깝고, final response가 비어 있는
    실패성 완료는 failure context가 더 진단 가치가 높다. 마지막 기본 문구는 legacy runtime이나
    이상 이벤트에서도 feed가 빈 문자열을 표시하지 않도록 하는 방어선이다.
    */
    // 정상 응답이 있으면 첫 유효 줄을 그대로 summary로 사용한다.
    if let Some(summary) = final_response_text
        .and_then(first_non_empty_line)
        .filter(|summary| !summary.is_empty())
    {
        return summary.to_string();
    }
    // final response가 없으면 실패 맥락을 사용해 왜 완료가 특이한지 feed에 드러낸다.
    if let Some(context) = failure_context {
        return format!("agent session finished with follow-up context: {context}");
    }

    // 둘 다 없을 때도 feed에 빈 문자열이 들어가지 않도록 안정적인 기본 문구를 반환한다.
    "agent session reported completion without a structured final summary".to_string()
}

// 여러 줄 텍스트에서 표시 가능한 첫 번째 줄을 찾는 순수 helper이다.
fn first_non_empty_line(text: &str) -> Option<&str> {
    /*
    multi-line final response를 한 줄 summary로 줄일 때는 markdown 제목, 빈 줄,
    validation log 앞의 공백 같은 형식을 제거해야 한다. 여기서는 의미를 해석하지 않고 가장
    먼저 내용이 있는 줄만 고르므로, domain 상태 전이에 영향을 주지 않는 순수 표시용 helper이다.
    */
    text.lines().map(str::trim).find(|line| !line.is_empty())
}
