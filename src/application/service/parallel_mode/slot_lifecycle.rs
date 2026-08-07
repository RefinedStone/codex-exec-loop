use std::path::Path;

use rand::RngCore;

use super::pool::reconcile_pool_board_and_context_with_target_locked;
use crate::application::port::outbound::github_automation_port::GithubRepositoryVisibility;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeDeliveryTargetSnapshot,
    ParallelModePoolSlotState, ParallelModeRepositoryVisibility, ParallelModeSlotLeaseRequest,
    ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
};

use super::pr_validation::transition_pr_validation_remediation_with_ports;
use super::{
    AKRA_AGENT_BRANCH_PREFIX, ParallelModeService, PoolSlotCleanupIdentity,
    PoolSlotCleanupLeaseAuthority, acquire_pool_mutation_lock, allocate_agent_branch_name,
    branch_is_integrated_into, build_pool_slots, cleanup_slot_to_ref_locked,
    command_succeeds_with_runtime, current_branch_name, current_timestamp,
    discard_unstarted_slot_branch, inspect_slot_git_status_with_runtime,
    load_pool_runtime_context_with_runtime, pool_baseline_branch_for_repo,
    record_assigned_session_detail, record_cleanup_pending_session_detail,
    record_failed_start_dispatch_block, record_failed_start_session_detail,
    record_running_session_detail, record_thread_prepared_session_detail,
    resolve_workspace_head_sha_with_runtime, resolve_workspace_slot_lease_with_runtime,
    rollback_slot_lease_write_failure, transition_slot_lease,
    try_parallel_mode_integration_branch_for_repo_with_runtime, try_push_remote_name_with_runtime,
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
        self.acquire_slot_lease_with_hook(workspace_dir, request, None, || {})
    }

    #[cfg(test)]
    pub(super) fn acquire_slot_lease_with_test_hook<F>(
        &self,
        workspace_dir: &str,
        request: ParallelModeSlotLeaseRequest,
        after_checkout_before_lease: F,
    ) -> Result<ParallelModeSlotLeaseSnapshot, String>
    where
        F: FnOnce(),
    {
        self.acquire_slot_lease_with_hook(workspace_dir, request, None, after_checkout_before_lease)
    }

    #[cfg(test)]
    pub(super) fn acquire_slot_lease_with_test_branch_instance_id(
        &self,
        workspace_dir: &str,
        request: ParallelModeSlotLeaseRequest,
        branch_instance_id: &str,
    ) -> Result<ParallelModeSlotLeaseSnapshot, String> {
        self.acquire_slot_lease_with_hook(workspace_dir, request, Some(branch_instance_id), || {})
    }

    fn acquire_slot_lease_with_hook<F>(
        &self,
        workspace_dir: &str,
        request: ParallelModeSlotLeaseRequest,
        test_branch_instance_id: Option<&str>,
        after_checkout_before_lease: F,
    ) -> Result<ParallelModeSlotLeaseSnapshot, String>
    where
        F: FnOnce(),
    {
        let mutation_lock = acquire_pool_mutation_lock(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            workspace_dir,
        )?;
        // Obtain the immutable remote proof before reconcile can create, reset,
        // or clean any slot. A stale tracking ref must never authorize local
        // destructive work merely because a later exact fetch detects drift.
        let fresh_target = self.fetch_fresh_pool_integration_target(workspace_dir)?;
        let (context, _) = reconcile_pool_board_and_context_with_target_locked(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            workspace_dir,
            &fresh_target,
            &mutation_lock,
        )
        .map_err(|error| error.1.clone())?;

        let push_remote = fresh_target.push_remote.clone();
        let integration_branch = fresh_target.integration_branch.clone();
        let credential_redacted_push_url = fresh_target.credential_redacted_push_url.clone();
        let integration_base_commit_sha = fresh_target.commit_sha.clone();
        if integration_base_commit_sha != context.baseline_head {
            return Err(
                "parallel delivery target changed while the slot lease was being acquired"
                    .to_string(),
            );
        }
        let github_repository = self
            .github_automation
            .repository_identity_for_push_url(
                &context.repo_root,
                &push_remote,
                &credential_redacted_push_url,
            )
            .map_err(|error| {
                format!(
                    "GitHub repository identity could not be frozen before worker start: {error}"
                )
            })?;
        let repository_visibility = self
            .github_automation
            .repository_visibility_for_push_url(
                &context.repo_root,
                &push_remote,
                &credential_redacted_push_url,
            )
            .map_err(|error| {
                format!(
                    "GitHub repository visibility could not be frozen before worker start: {error}"
                )
            })?;
        if repository_visibility == GithubRepositoryVisibility::Public
            && !self
                .delivery_safety_policy
                .allow_public_repository
                .clone()?
        {
            return Err(
                "public GitHub repository delivery is blocked before worker start; set AKRA_PARALLEL_ALLOW_PUBLIC_REPOSITORY=1 in the parent process after operator review"
                    .to_string(),
            );
        }
        let verified_push_remote =
            try_push_remote_name_with_runtime(self.parallel_runtime.as_ref(), &context.repo_root)?;
        let verified_integration_branch =
            try_parallel_mode_integration_branch_for_repo_with_runtime(
                self.parallel_runtime.as_ref(),
                &context.repo_root,
            )?;
        let verified_push_url = self
            .github_automation
            .credential_redacted_push_url_for_remote(&context.repo_root, &verified_push_remote)
            .map_err(|error| {
                format!(
                    "credential-redacted GitHub push URL could not be reverified before worker start: {error}"
                )
            })?;
        let mut target_drift = Vec::new();
        if verified_push_remote != push_remote {
            target_drift.push("push remote alias");
        }
        if verified_integration_branch != integration_branch {
            target_drift.push("integration branch");
        }
        if verified_push_url != credential_redacted_push_url {
            target_drift.push("credential-redacted push URL");
        }
        if !target_drift.is_empty() {
            return Err(format!(
                "parallel delivery target changed while the slot lease was being acquired: {}",
                target_drift.join(", ")
            ));
        }
        let delivery_target = ParallelModeDeliveryTargetSnapshot::new(
            push_remote.clone(),
            github_repository,
            match repository_visibility {
                GithubRepositoryVisibility::Private => ParallelModeRepositoryVisibility::Private,
                GithubRepositoryVisibility::Internal => ParallelModeRepositoryVisibility::Internal,
                GithubRepositoryVisibility::Public => ParallelModeRepositoryVisibility::Public,
            },
            integration_branch.clone(),
            integration_base_commit_sha,
        )
        .with_credential_redacted_push_url(credential_redacted_push_url.clone());

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
        let Some((idle_slot, slot_path)) =
            build_pool_slots(self.parallel_runtime.as_ref(), &context)
                .into_iter()
                .filter(|slot| slot.state == ParallelModePoolSlotState::Idle)
                .find_map(|slot| {
                    let slot_path = context.pool_root.join(&slot.slot_id);
                    let detached = current_branch_name(self.parallel_runtime.as_ref(), &slot_path)
                        .as_deref()
                        == Some("HEAD");
                    let exact_head = resolve_workspace_head_sha_with_runtime(
                        self.parallel_runtime.as_ref(),
                        &slot_path,
                    )
                    .as_deref()
                        == Some(context.baseline_head.as_str());
                    (detached && exact_head).then_some((slot, slot_path))
                })
        else {
            return Err("no remote-verified idle slot is available for lease".to_string());
        };
        let slot_path_string = slot_path.display().to_string();
        // branch 이름에는 slot/task 정보가 들어가므로 나중에 GitHub PR, supervisor board,
        // cleanup 로그가 같은 작업을 같은 이름으로 추적할 수 있다.
        let branch_prefix = format!("{AKRA_AGENT_BRANCH_PREFIX}/{}/", idle_slot.slot_id);
        let live_remote_branch_names = self
            .github_automation
            .remote_branch_names_for_prefix_for_delivery_target(
                &context.repo_root,
                &push_remote,
                &credential_redacted_push_url,
                &branch_prefix,
            )
            .map_err(|error| {
                format!(
                    "live agent branches could not be inspected for the frozen delivery target: {error}"
                )
            })?;
        let lease_generation = new_slot_lease_generation()?;
        let branch_instance_id = test_branch_instance_id.unwrap_or(&lease_generation[..16]);
        let branch_name = allocate_agent_branch_name(
            self.parallel_runtime.as_ref(),
            &context.repo_root,
            &idle_slot.slot_id,
            &request.task_slug,
            &request.task_id,
            &request.task_title,
            branch_instance_id,
            &live_remote_branch_names,
        )?;
        mutation_lock.verify_pool_root(&context.pool_root)?;
        self.parallel_runtime
            .ensure_git_execution_safe(&slot_path)
            .map_err(|error| format!("slot checkout blocked: {error}"))?;
        if !command_succeeds_with_runtime(
            self.parallel_runtime.as_ref(),
            "git",
            [
                "-C",
                slot_path_string.as_str(),
                "checkout",
                "-b",
                branch_name.as_str(),
                context.baseline_head.as_str(),
            ],
        ) {
            return Err(format!(
                "failed to create branch `{branch_name}` in slot `{}`",
                idle_slot.slot_id
            ));
        }
        if current_branch_name(self.parallel_runtime.as_ref(), &slot_path).as_deref()
            != Some(branch_name.as_str())
            || resolve_workspace_head_sha_with_runtime(self.parallel_runtime.as_ref(), &slot_path)
                .as_deref()
                != Some(context.baseline_head.as_str())
        {
            let _ = discard_unstarted_slot_branch(
                self.parallel_runtime.as_ref(),
                &context.repo_root,
                &slot_path,
                branch_name.as_str(),
                &context.baseline_head,
                &mutation_lock,
            );
            return Err(format!(
                "slot `{}` moved away from the fetched integration target during lease creation",
                idle_slot.slot_id
            ));
        }
        after_checkout_before_lease();
        mutation_lock.verify_pool_root(&context.pool_root)?;

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
        )
        .with_delivery_target(delivery_target)
        .with_lease_generation(lease_generation);
        if let Err(error) = write_slot_lease(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &lease,
        ) {
            let _ = rollback_slot_lease_write_failure(
                self.planning_authority.as_ref(),
                self.parallel_runtime.as_ref(),
                &context.repo_root,
                &context.pool_root,
                &lease,
            );
            let _ = discard_unstarted_slot_branch(
                self.parallel_runtime.as_ref(),
                &context.repo_root,
                &slot_path,
                branch_name.as_str(),
                &context.baseline_head,
                &mutation_lock,
            );
            return Err(error);
        }

        // session detail 기록 실패는 lease 자체를 실패시키지 않는다. slot 소유권의 source of
        // truth는 lease 파일이고, detail은 roster/detail UI를 위한 관측 보조 자료다.
        let _ = record_assigned_session_detail(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
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
        self.mark_slot_running_inner(workspace_dir, slot_id, agent_id, None)
    }

    fn mark_slot_running_inner(
        &self,
        workspace_dir: &str,
        slot_id: &str,
        agent_id: &str,
        expected_lease: Option<&ParallelModeSlotLeaseSnapshot>,
    ) -> Result<ParallelModeSlotLeaseSnapshot, String> {
        let mutation_lock = acquire_pool_mutation_lock(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            workspace_dir,
        )?;
        let context = load_pool_runtime_context_with_runtime(
            self.parallel_runtime.as_ref(),
            self.planning_authority.as_ref(),
            workspace_dir,
        )
        .map_err(|(_, detail)| detail.to_string())?;
        mutation_lock.verify_pool_root(&context.pool_root)?;
        let mut lease = context
            .slot_leases
            .get(slot_id)
            .cloned()
            .ok_or_else(|| format!("slot `{slot_id}` does not have an active lease"))?;
        if expected_lease.is_some_and(|expected| !lease.same_generation_as(expected)) {
            return Err(format!(
                "slot `{slot_id}` lease generation changed before running transition"
            ));
        }

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
        if current_branch_name(
            self.parallel_runtime.as_ref(),
            Path::new(&lease.worktree_path),
        )
        .as_deref()
            != Some(lease.branch_name.as_str())
        {
            return Err(format!(
                "slot `{slot_id}` is no longer checked out to `{}`",
                lease.branch_name
            ));
        }

        let previous_lease = lease.clone();
        lease.state = ParallelModeSlotLeaseState::Running;
        // Running 전이는 idempotent하게 유지한다. 중복 event가 elapsed 기준을 갱신하면 UI가
        // 작업 시간을 짧게 보이게 되고 timeout 판단도 흔들릴 수 있다.
        if lease.running_started_at.is_none() {
            lease.running_started_at = Some(current_timestamp());
        }
        transition_slot_lease(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &previous_lease,
            &lease,
        )?;

        // roster detail은 best-effort projection이다. lease 저장이 성공했다면 핵심 상태 전이는
        // 끝났고, detail 쓰기 실패가 실행 중인 slot을 되돌리지는 않는다.
        let _ = record_running_session_detail(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &lease,
        );
        transition_pr_validation_remediation_with_ports(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &lease.task_id,
            false,
        )?;
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
        self.record_workspace_slot_thread_prepared_inner(workspace_dir, thread_id, None)
    }

    pub(crate) fn record_workspace_slot_thread_prepared_for_lease(
        &self,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
        thread_id: &str,
    ) -> Result<Option<ParallelModeAgentSessionDetailSnapshot>, String> {
        self.record_workspace_slot_thread_prepared_inner(
            &expected_lease.worktree_path,
            thread_id,
            Some(expected_lease),
        )
    }

    fn record_workspace_slot_thread_prepared_inner(
        &self,
        workspace_dir: &str,
        thread_id: &str,
        expected_lease: Option<&ParallelModeSlotLeaseSnapshot>,
    ) -> Result<Option<ParallelModeAgentSessionDetailSnapshot>, String> {
        let mutation_lock = acquire_pool_mutation_lock(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            workspace_dir,
        )?;
        // turn service는 slot_id를 모르고 launch workspace만 안다. 역해결이 실패하는 것은
        // 오류가 아니라 일반 대화 경로일 수 있으므로 Option으로 바깥에 전달한다.
        let Some(resolution) = resolve_workspace_slot_lease_with_runtime(
            self.parallel_runtime.as_ref(),
            self.planning_authority.as_ref(),
            workspace_dir,
        )?
        else {
            return Ok(None);
        };
        mutation_lock.verify_pool_root(&resolution.context.pool_root)?;
        if expected_lease.is_some_and(|expected| !resolution.lease.same_generation_as(expected)) {
            return Ok(None);
        }

        record_thread_prepared_session_detail(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
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
        let mutation_lock = acquire_pool_mutation_lock(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            workspace_dir,
        )?;
        self.mark_slot_cleanup_pending_locked(workspace_dir, slot_id, agent_id, &mutation_lock)
    }

    pub(super) fn mark_slot_cleanup_pending_locked(
        &self,
        workspace_dir: &str,
        slot_id: &str,
        agent_id: &str,
        mutation_lock: &super::PoolMutationLock,
    ) -> Result<ParallelModeSlotLeaseSnapshot, String> {
        let context = load_pool_runtime_context_with_runtime(
            self.parallel_runtime.as_ref(),
            self.planning_authority.as_ref(),
            workspace_dir,
        )
        .map_err(|(_, detail)| detail.to_string())?;
        mutation_lock.verify_pool_root(&context.pool_root)?;
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

        if current_branch_name(
            self.parallel_runtime.as_ref(),
            Path::new(&lease.worktree_path),
        )
        .as_deref()
            != Some(lease.branch_name.as_str())
        {
            return Err(format!(
                "slot `{slot_id}` is no longer checked out to `{}`",
                lease.branch_name
            ));
        }

        // cleanup은 slot worktree를 baseline으로 되돌리는 파괴적 작업을 준비한다. branch가
        // baseline에 통합됐다는 증거가 없으면 여기서 멈춰 변경 손실을 막는다.
        let integration_target_oid = self
            .fetch_fresh_pool_integration_target(&context.repo_root)?
            .commit_sha;
        if !branch_is_integrated_into(
            self.parallel_runtime.as_ref(),
            &context.repo_root,
            &lease.branch_name,
            &integration_target_oid,
        ) {
            return Err(format!(
                "slot `{slot_id}` branch `{}` is not integrated into `{}` yet",
                lease.branch_name,
                pool_baseline_branch_for_repo(self.parallel_runtime.as_ref(), &context.repo_root)
            ));
        }

        let previous_lease = lease.clone();
        lease.state = ParallelModeSlotLeaseState::CleanupPending;
        transition_slot_lease(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            &context.repo_root,
            &context.pool_root,
            &previous_lease,
            &lease,
        )?;

        // detail 갱신은 supervisor board의 설명을 맞추기 위한 projection이다. lease 전이
        // 성공을 기준으로 cleanup worker가 다음 단계를 진행할 수 있다.
        let _ = record_cleanup_pending_session_detail(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
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
        let Some(resolution) = resolve_workspace_slot_lease_with_runtime(
            self.parallel_runtime.as_ref(),
            self.planning_authority.as_ref(),
            workspace_dir,
        )?
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

    pub(crate) fn mark_workspace_slot_running_for_lease(
        &self,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<Option<ParallelModeSlotLeaseSnapshot>, String> {
        self.mark_slot_running_inner(
            &expected_lease.worktree_path,
            &expected_lease.slot_id,
            &expected_lease.agent_id,
            Some(expected_lease),
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
        self.release_workspace_slot_lease_after_failed_start_with_hook(workspace_dir, None, || {})
    }

    pub(crate) fn release_workspace_slot_lease_after_failed_start_for_lease(
        &self,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<Option<ParallelModeSlotLeaseSnapshot>, String> {
        self.release_workspace_slot_lease_after_failed_start_with_hook(
            &expected_lease.worktree_path,
            Some(expected_lease),
            || {},
        )
    }

    #[cfg(test)]
    pub(super) fn release_workspace_slot_lease_after_failed_start_with_test_hook<F>(
        &self,
        workspace_dir: &str,
        after_dispatch_block: F,
    ) -> Result<Option<ParallelModeSlotLeaseSnapshot>, String>
    where
        F: FnOnce(),
    {
        self.release_workspace_slot_lease_after_failed_start_with_hook(
            workspace_dir,
            None,
            after_dispatch_block,
        )
    }

    fn release_workspace_slot_lease_after_failed_start_with_hook<F>(
        &self,
        workspace_dir: &str,
        expected_lease: Option<&ParallelModeSlotLeaseSnapshot>,
        after_dispatch_block: F,
    ) -> Result<Option<ParallelModeSlotLeaseSnapshot>, String>
    where
        F: FnOnce(),
    {
        let mutation_lock = acquire_pool_mutation_lock(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            workspace_dir,
        )?;
        // slot workspace가 아니라면 실패한 시작 이벤트도 병렬 pool과 무관하다.
        let Some(resolution) = resolve_workspace_slot_lease_with_runtime(
            self.parallel_runtime.as_ref(),
            self.planning_authority.as_ref(),
            workspace_dir,
        )?
        else {
            return Ok(None);
        };
        if expected_lease.is_some_and(|expected| !resolution.lease.same_generation_as(expected)) {
            return Ok(None);
        }

        // Running 이후 실패는 startup failure가 아니라 실행 중단/완료 계열 이벤트가 처리해야
        // 한다. 여기서 branch를 지우면 이미 생성된 작업 산출물을 잃을 수 있다.
        if resolution.lease.state != ParallelModeSlotLeaseState::Leased {
            return Ok(None);
        }

        /*
         * Fence redispatch before any cleanup or best-effort session history.
         * If a later mirror/detail write fails after the lease is removed, the
         * durable task block must already prevent the same unchanged task from
         * being launched again.
         */
        let failed_at = current_timestamp();
        record_failed_start_dispatch_block(
            self.planning_authority.as_ref(),
            &resolution.context.canonical_repo_root.display().to_string(),
            &resolution.lease,
            &failed_at,
        )?;
        after_dispatch_block();

        // cleanup 전에 git 상태를 읽지 못하면 lease를 남긴다. pool을 오염시키는 것보다
        // 사용자가 수동으로 확인할 수 있는 active lease가 안전하다.
        let Ok(slot_status) = inspect_slot_git_status_with_runtime(
            self.parallel_runtime.as_ref(),
            &resolution.workspace_path,
        ) else {
            return Err(format!(
                "slot `{}` could not be inspected after startup failure",
                resolution.lease.slot_id
            ));
        };

        // 시작 전 실패라도 dirty worktree는 의미 있는 산출물이나 진단 파일일 수 있다.
        // 자동 cleanup은 clean baseline에서만 허용한다.
        if !slot_status.is_clean_baseline() {
            return Err(format!(
                "slot `{}` could not be released after startup failure because worktree is not clean: {}",
                resolution.lease.slot_id,
                slot_status.detail_label()
            ));
        }

        // `cleanup_slot`이 lease 제거와 branch 정리를 함께 수행한다. 실패하면 호출자에게
        // 명시적으로 알려 supervisor가 slot을 idle로 오판하지 않게 한다.
        let integration_target_oid = self
            .fetch_fresh_pool_integration_target(&resolution.context.repo_root)?
            .commit_sha;
        if !cleanup_slot_to_ref_locked(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            &PoolSlotCleanupIdentity::new(
                &resolution.context.repo_root,
                &resolution.context.canonical_repo_root,
                &resolution.context.pool_root,
                &resolution.lease.slot_id,
                &resolution.workspace_path,
                &resolution.lease.branch_name,
            ),
            &integration_target_oid,
            PoolSlotCleanupLeaseAuthority::StaleStartup {
                lease: &resolution.lease,
                dispatch_blocked_at: &failed_at,
            },
            &mutation_lock,
        ) {
            return Err(format!(
                "slot `{}` could not be reset to `{}` after startup failure",
                resolution.lease.slot_id,
                pool_baseline_branch_for_repo(
                    self.parallel_runtime.as_ref(),
                    &resolution.context.repo_root,
                )
            ));
        }

        // 실패 기록은 이미 회수한 lease의 사후 설명이다. cleanup 성공 후 기록해 board에는
        // "왜 사라졌는지"가 남고, slot 자체는 즉시 재사용 가능해진다.
        record_failed_start_session_detail(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            &resolution.context.canonical_repo_root.display().to_string(),
            &resolution.context.pool_root,
            &resolution.lease,
            &failed_at,
        )?;
        Ok(Some(resolution.lease))
    }
}

fn new_slot_lease_generation() -> Result<String, String> {
    let mut generation = [0_u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut generation)
        .map_err(|error| {
            format!("operating-system randomness is required for slot leases: {error}")
        })?;
    Ok(generation
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
