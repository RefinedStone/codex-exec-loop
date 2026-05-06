use std::path::Path;

use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeDispatchBlockReason,
    ParallelModePoolSlotState, ParallelModeSlotLeaseRequest, ParallelModeSlotLeaseSnapshot,
    ParallelModeSlotLeaseState, ParallelModeTaskDispatchBlockSnapshot,
};

use super::{
    POOL_BASELINE_BRANCH, ParallelModeService, acquire_pool_allocation_lock,
    allocate_agent_branch_name, branch_is_cleanup_ready, build_pool_slots, cleanup_slot,
    command_succeeds, current_branch_name, current_timestamp, discard_unstarted_slot_branch,
    inspect_slot_git_status, load_pool_runtime_context, reconcile_pool_board,
    record_assigned_session_detail, record_cleanup_pending_session_detail,
    record_failed_start_session_detail, record_running_session_detail,
    record_thread_prepared_session_detail, remove_slot_lease, resolve_workspace_slot_lease,
    write_slot_lease,
};

impl ParallelModeService {
    /*
    슬롯 lease 획득은 병렬 agent 작업의 시작점이다. TUI가 병렬 dispatch를 요청하면
    turn service가 이 함수로 들어오고, 여기서 pool allocation lock을 잡은 뒤 pool을
    reconcile하고, idle slot 하나를 골라 agent branch를 만든다.

    이 함수가 task_id와 agent_id 중복을 모두 거부하는 이유는 병렬 모드의 소유권 단위가
    "작업"과 "agent process" 양쪽에 걸쳐 있기 때문이다. 한 task가 두 slot에서 동시에
    진행되면 distributor merge 순서가 깨지고, 한 agent가 두 lease를 들면 stream event가
    어느 worktree에 속하는지 역추적할 수 없다.

    branch 생성 후 lease 저장이 실패하면 `discard_unstarted_slot_branch`로 방금 만든
    branch를 되돌린다. 아직 stream이 시작되지 않은 상태라 안전하게 폐기할 수 있고, 이
    cleanup이 있어야 실패한 lease 시도가 pool을 오염시키지 않는다. 성공하면 assigned
    session detail을 기록해 supervisor가 "slot이 누구에게 배정되었는지"를 즉시 볼 수 있게
    한다.
    */
    pub fn acquire_slot_lease(
        &self,
        workspace_dir: &str,
        request: ParallelModeSlotLeaseRequest,
    ) -> Result<ParallelModeSlotLeaseSnapshot, String> {
        let _allocation_lock =
            acquire_pool_allocation_lock(self.planning_authority.as_ref(), workspace_dir)?;
        // lock을 잡은 뒤 reconcile을 먼저 돌려 stale lease/slot 상태를 최신 board로 맞춘다.
        // 실패해도 lease 시도 자체를 막지는 않는다. 다음 context load와 idle slot 선택이
        // 현재 파일 상태를 다시 읽어 최종 판단을 하기 때문이다.
        let _ = reconcile_pool_board(self.planning_authority.as_ref(), workspace_dir);
        let context = load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir)
            .map_err(|(_, detail)| detail.to_string())?;

        // task 중복은 같은 backlog item이 두 agent branch에서 별도로 커밋되는 상황을 막는다.
        if context
            .slot_leases
            .values()
            .any(|lease| lease.task_id == request.task_id)
        {
            return Err(format!(
                "task `{}` already has an active slot lease",
                request.task_id
            ));
        }

        // agent 중복은 한 app-server stream의 후속 이벤트가 두 lease로 매핑되는 것을 막는다.
        if context
            .slot_leases
            .values()
            .any(|lease| lease.agent_id == request.agent_id)
        {
            return Err(format!(
                "agent `{}` already owns an active slot lease",
                request.agent_id
            ));
        }

        // slot snapshot은 lease 파일과 worktree 상태를 합친 view다. 여기서 Idle만 고르면
        // cleanup pending이나 dirty slot을 새 작업에 재사용하지 않는다.
        let Some(idle_slot) = build_pool_slots(&context)
            .into_iter()
            .find(|slot| slot.state == ParallelModePoolSlotState::Idle)
        else {
            return Err("no idle slot is available for lease".to_string());
        };
        let slot_path = context.pool_root.join(&idle_slot.slot_id);
        let slot_path_string = slot_path.display().to_string();
        // branch 이름에는 slot/task 정보가 들어가므로 나중에 GitHub PR, supervisor board,
        // cleanup 로그가 같은 작업을 같은 이름으로 추적할 수 있다.
        let branch_name = allocate_agent_branch_name(
            &context.repo_root,
            &idle_slot.slot_id,
            &request.task_slug,
            &request.task_id,
            &request.task_title,
        );
        if !command_succeeds(
            "git",
            [
                "-C",
                slot_path_string.as_str(),
                "checkout",
                "-b",
                branch_name.as_str(),
                POOL_BASELINE_BRANCH,
            ],
        ) {
            return Err(format!(
                "failed to create branch `{branch_name}` in slot `{}`",
                idle_slot.slot_id
            ));
        }

        // lease는 branch checkout이 성공한 뒤에만 기록한다. lease 파일이 존재하는 순간부터
        // supervisor와 workspace 역해결 경로가 이 slot을 active로 취급하기 때문이다.
        let lease = ParallelModeSlotLeaseSnapshot::new(
            idle_slot.slot_id.clone(),
            request.task_id,
            request.task_title,
            request.agent_id,
            branch_name.clone(),
            slot_path_string.clone(),
            ParallelModeSlotLeaseState::Leased,
            current_timestamp(),
            None,
        );
        if let Err(error) = write_slot_lease(
            self.planning_authority.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &lease,
        ) {
            let _ = remove_slot_lease(
                self.planning_authority.as_ref(),
                &context.repo_root,
                &context.pool_root,
                &lease.slot_id,
            );
            let _ =
                discard_unstarted_slot_branch(&context.repo_root, &slot_path, branch_name.as_str());
            return Err(error);
        }

        // session detail 기록 실패는 lease 자체를 실패시키지 않는다. slot 소유권의 source of
        // truth는 lease 파일이고, detail은 roster/detail UI를 위한 관측 보조 자료다.
        let _ = record_assigned_session_detail(
            self.planning_authority.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &lease,
        );
        Ok(lease)
    }

    /*
    `mark_slot_running`은 app-server stream에서 TurnStarted가 관측된 뒤 lease를 Leased에서
    Running으로 전환한다. agent_id와 현재 checkout branch를 다시 확인하는 이유는 slot path가
    다른 작업으로 바뀌었거나 lease 소유자가 어긋난 상태에서 잘못 running으로 승격하지 않기
    위해서다.

    running_started_at은 roster elapsed label의 기준 시간이 된다. 이미 값이 있으면 유지해
    같은 running 이벤트가 중복으로 들어와도 시작 시간이 흔들리지 않게 한다.
    */
    pub fn mark_slot_running(
        &self,
        workspace_dir: &str,
        slot_id: &str,
        agent_id: &str,
    ) -> Result<ParallelModeSlotLeaseSnapshot, String> {
        let context = load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir)
            .map_err(|(_, detail)| detail.to_string())?;
        let mut lease = context
            .slot_leases
            .get(slot_id)
            .cloned()
            .ok_or_else(|| format!("slot `{slot_id}` does not have an active lease"))?;

        // stream event는 agent_id를 통해 lease 소유자와 다시 결합된다. slot_id만 믿으면
        // 재사용된 slot의 늦은 이벤트가 새 작업을 running으로 바꿀 수 있다.
        if lease.agent_id != agent_id {
            return Err(format!(
                "slot `{slot_id}` is leased by `{}` instead of `{agent_id}`",
                lease.agent_id
            ));
        }

        // cleanup pending은 이미 branch 통합 이후의 상태다. 늦게 도착한 TurnStarted가 이
        // 상태를 Running으로 되돌리면 cleanup supervisor가 영원히 slot을 회수하지 못한다.
        if lease.state == ParallelModeSlotLeaseState::CleanupPending {
            return Err(format!("slot `{slot_id}` is already waiting for cleanup",));
        }

        // worktree checkout이 lease branch와 다르면 파일 변경이 어느 branch 소유인지 알 수
        // 없으므로 상태 전이를 중단한다.
        if current_branch_name(Path::new(&lease.worktree_path)).as_deref()
            != Some(lease.branch_name.as_str())
        {
            return Err(format!(
                "slot `{slot_id}` is no longer checked out to `{}`",
                lease.branch_name
            ));
        }

        lease.state = ParallelModeSlotLeaseState::Running;
        // Running 전이는 idempotent하게 유지한다. 중복 event가 elapsed 기준을 갱신하면 UI가
        // 작업 시간을 짧게 보이게 되고 timeout 판단도 흔들릴 수 있다.
        if lease.running_started_at.is_none() {
            lease.running_started_at = Some(current_timestamp());
        }
        write_slot_lease(
            self.planning_authority.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &lease,
        )?;

        // roster detail은 best-effort projection이다. lease 저장이 성공했다면 핵심 상태 전이는
        // 끝났고, detail 쓰기 실패가 실행 중인 slot을 되돌리지는 않는다.
        let _ = record_running_session_detail(
            self.planning_authority.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &lease,
        );
        Ok(lease)
    }

    /*
    ThreadPrepared 이벤트는 아직 turn이 실행되기 전이지만, app-server가 실제 thread id를
    확정했다는 뜻이다. 이 함수는 workspace path로 현재 slot lease를 찾아 session detail에
    thread id와 starting history를 남긴다. lease가 없는 workspace라면 일반 대화이므로
    None을 반환해 병렬 모드 상태를 건드리지 않는다.
    */
    pub fn record_workspace_slot_thread_prepared(
        &self,
        workspace_dir: &str,
        thread_id: &str,
    ) -> Result<Option<ParallelModeAgentSessionDetailSnapshot>, String> {
        // turn service는 slot_id를 모르고 launch workspace만 안다. 역해결이 실패하는 것은
        // 오류가 아니라 일반 대화 경로일 수 있으므로 Option으로 바깥에 전달한다.
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };

        record_thread_prepared_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
            thread_id,
        )
        .map(Some)
    }

    /*
    CleanupPending은 "agent 작업은 끝났고 branch가 baseline에 통합되었지만, 아직 slot
    worktree를 idle baseline으로 되돌리지 않았다"는 중간 상태다. 이 함수는 Running 상태에서만
    진입하게 하고, branch가 `POOL_BASELINE_BRANCH`에 통합되었는지 확인한 뒤 lease와 session
    detail을 함께 갱신한다.

    이 확인 없이 cleanup pending으로 넘기면 distributor가 아직 통합하지 않은 변경을 slot
    cleanup이 삭제할 수 있다. 그래서 branch ancestry 검사는 데이터 보존을 위한 핵심 안전
    장치다.
    */
    pub fn mark_slot_cleanup_pending(
        &self,
        workspace_dir: &str,
        slot_id: &str,
        agent_id: &str,
    ) -> Result<ParallelModeSlotLeaseSnapshot, String> {
        let context = load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir)
            .map_err(|(_, detail)| detail.to_string())?;
        let mut lease = context
            .slot_leases
            .get(slot_id)
            .cloned()
            .ok_or_else(|| format!("slot `{slot_id}` does not have an active lease"))?;

        // cleanup 요청도 agent 소유권을 확인한다. 다른 agent가 slot_id만 알고 cleanup을
        // 요청하면 아직 실행 중인 작업을 회수할 수 있기 때문이다.
        if lease.agent_id != agent_id {
            return Err(format!(
                "slot `{slot_id}` is leased by `{}` instead of `{agent_id}`",
                lease.agent_id
            ));
        }

        // Leased 상태는 아직 TurnStarted가 오지 않은 시작 전 구간이다. 여기서 cleanup pending
        // 으로 넘기면 failed-start release 경로가 branch를 안전하게 제거할 기회를 잃는다.
        if lease.state == ParallelModeSlotLeaseState::Leased {
            return Err(format!(
                "slot `{slot_id}` has not entered running state yet",
            ));
        }

        // 이미 cleanup pending이면 idempotent 성공으로 처리해 supervisor 재시도와 사용자
        // refresh가 같은 상태를 반복 요청해도 오류로 번지지 않게 한다.
        if lease.state == ParallelModeSlotLeaseState::CleanupPending {
            return Ok(lease);
        }

        if current_branch_name(Path::new(&lease.worktree_path)).as_deref()
            != Some(lease.branch_name.as_str())
        {
            return Err(format!(
                "slot `{slot_id}` is no longer checked out to `{}`",
                lease.branch_name
            ));
        }

        // cleanup은 slot worktree를 baseline으로 되돌리는 파괴적 작업을 준비한다. branch가
        // baseline에 통합됐다는 증거가 없으면 여기서 멈춰 변경 손실을 막는다.
        if !branch_is_cleanup_ready(&context.repo_root, &lease.branch_name) {
            return Err(format!(
                "slot `{slot_id}` branch `{}` is not integrated into `{POOL_BASELINE_BRANCH}` yet",
                lease.branch_name
            ));
        }

        lease.state = ParallelModeSlotLeaseState::CleanupPending;
        write_slot_lease(
            self.planning_authority.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &lease,
        )?;

        // detail 갱신은 supervisor board의 설명을 맞추기 위한 projection이다. lease 전이
        // 성공을 기준으로 cleanup worker가 다음 단계를 진행할 수 있다.
        let _ = record_cleanup_pending_session_detail(
            self.planning_authority.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &lease,
        );
        Ok(lease)
    }

    /*
    workspace 기반 running 전이는 turn service가 slot id를 직접 몰라도 되게 하는 편의
    경로다. stream launch 이후 실제 실행 workspace는 slot worktree이므로, 그 경로로 lease를
    역해결한 뒤 `mark_slot_running`에 위임한다. 일반 workspace에서는 None이 되어 호출자가
    병렬 모드와 무관한 이벤트로 처리할 수 있다.
    */
    pub fn mark_workspace_slot_running(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeSlotLeaseSnapshot>, String> {
        // workspace path가 pool slot에 속하지 않으면 병렬 모드 이벤트가 아니다. 에러 대신
        // None을 반환해 상위 stream reducer가 일반 turn으로 계속 진행할 수 있게 한다.
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };

        self.mark_slot_running(
            workspace_dir,
            &resolution.lease.slot_id,
            &resolution.lease.agent_id,
        )
        .map(Some)
    }

    /*
    stream이 TurnStarted 전에 실패하면 slot은 아직 의미 있는 agent 작업을 만들지 못한 상태다.
    이 함수는 Leased 상태인 경우에만, worktree가 clean인지 확인한 뒤 agent branch를 삭제하고
    lease를 제거해 slot을 idle로 되돌린다.

    worktree가 dirty이면 자동 release를 거부한다. 시작 전이라고 해도 파일 변경이 있으면
    원인을 알 수 없으므로, 사용자가 확인하기 전까지 pool이 그 상태를 보존해야 한다.
    */
    pub fn release_workspace_slot_lease_after_failed_start(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeSlotLeaseSnapshot>, String> {
        // slot workspace가 아니라면 실패한 시작 이벤트도 병렬 pool과 무관하다.
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };

        // Running 이후 실패는 startup failure가 아니라 실행 중단/완료 계열 이벤트가 처리해야
        // 한다. 여기서 branch를 지우면 이미 생성된 작업 산출물을 잃을 수 있다.
        if resolution.lease.state != ParallelModeSlotLeaseState::Leased {
            return Ok(None);
        }

        // cleanup 전에 git 상태를 읽지 못하면 lease를 남긴다. pool을 오염시키는 것보다
        // 사용자가 수동으로 확인할 수 있는 active lease가 안전하다.
        let Some(slot_status) = inspect_slot_git_status(&resolution.workspace_path) else {
            let _ = record_failed_start_dispatch_block(
                self.planning_authority.as_ref(),
                &resolution.context.canonical_repo_root.display().to_string(),
                &resolution.lease,
            );
            return Err(format!(
                "slot `{}` could not be inspected after startup failure",
                resolution.lease.slot_id
            ));
        };

        // 시작 전 실패라도 dirty worktree는 의미 있는 산출물이나 진단 파일일 수 있다.
        // 자동 cleanup은 clean baseline에서만 허용한다.
        if !slot_status.is_clean_baseline() {
            let _ = record_failed_start_dispatch_block(
                self.planning_authority.as_ref(),
                &resolution.context.canonical_repo_root.display().to_string(),
                &resolution.lease,
            );
            return Err(format!(
                "slot `{}` could not be released after startup failure because worktree is not clean: {}",
                resolution.lease.slot_id,
                slot_status.detail_label()
            ));
        }

        // `cleanup_slot`이 lease 제거와 branch 정리를 함께 수행한다. 실패하면 호출자에게
        // 명시적으로 알려 supervisor가 slot을 idle로 오판하지 않게 한다.
        if !cleanup_slot(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease.slot_id,
            &resolution.workspace_path,
            &resolution.lease.branch_name,
        ) {
            let _ = record_failed_start_dispatch_block(
                self.planning_authority.as_ref(),
                &resolution.context.canonical_repo_root.display().to_string(),
                &resolution.lease,
            );
            return Err(format!(
                "slot `{}` could not be reset to `{POOL_BASELINE_BRANCH}` after startup failure",
                resolution.lease.slot_id
            ));
        }

        // 실패 기록은 이미 회수한 lease의 사후 설명이다. cleanup 성공 후 기록해 board에는
        // "왜 사라졌는지"가 남고, slot 자체는 즉시 재사용 가능해진다.
        record_failed_start_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.canonical_repo_root.display().to_string(),
            &resolution.context.pool_root,
            &resolution.lease,
        )?;
        Ok(Some(resolution.lease))
    }
}

fn record_failed_start_dispatch_block(
    planning_authority: &dyn crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort,
    workspace_dir: &str,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<(), String> {
    let block = ParallelModeTaskDispatchBlockSnapshot::new(
        lease.task_id.clone(),
        String::new(),
        current_timestamp(),
        ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
    );
    planning_authority
        .upsert_runtime_task_dispatch_block(workspace_dir, &block)
        .map_err(|error| format!("failed to store startup failure dispatch block: {error}"))
}
