use super::{
    DEFAULT_PUSH_REMOTE_NAME, DISTRIBUTOR_INTEGRATION_BRANCH, PoolRuntimeContext,
    WorkspaceSlotLeaseResolution, branch_exists, branch_is_integrated_into, cleanup_slot,
    command_succeeds, current_branch_name, current_timestamp, inspect_slot_git_status,
    lease_session_key, load_pool_runtime_context, reconcile_pool_board,
    record_cleaned_session_detail, record_cleanup_pending_session_detail,
    record_integrating_session_detail, record_merge_pending_session_detail,
    record_merge_queued_session_detail, record_official_completion_recovery_needed_session_detail,
    record_pr_pending_session_detail, record_pushing_session_detail, remote_branch_name,
    remote_tracking_branch_ref, resolve_workspace_head_sha, resolve_workspace_slot_lease,
    run_command, short_sha, write_slot_lease,
};
use crate::application::port::outbound::github_automation_port::GithubAutomationPort;
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityOfficialRefreshRecoveryStatus,
    PlanningAuthorityPort,
};
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeDistributorQueueItem,
    ParallelModeDistributorSnapshot, ParallelModeQueueItemState, ParallelModeReadinessSnapshot,
    ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
};
use chrono::{DateTime, TimeDelta, Utc};
use std::path::Path;
use std::sync::Arc;

const STALE_LEDGER_REFRESHING_AFTER_SECS: i64 = 300;
const INTEGRATION_BRANCH_PUSH_BLOCK_FRAGMENT: &str = "`prerelease` could not be pushed to `origin`";
pub(super) type ParallelModeDistributorQueueRecord = PlanningAuthorityDistributorQueueRecord;
mod delivery;
mod queue_keys;
mod snapshot;
mod store;
use self::delivery::process_distributor_queue_record;
use self::queue_keys::distributor_claim_owner_token;
use self::snapshot::{
    build_distributor_snapshot_from_context, build_placeholder_distributor_snapshot,
};
#[cfg(test)]
pub(super) use self::store::load_distributor_queue_records;
use self::store::{
    block_distributor_queue_record, distributor_queue_item_id, queue_order_key_from_timestamp,
    write_distributor_queue_record,
};

#[derive(Clone)]
/*
distributor service는 병렬 agent가 만든 commit-ready 결과를 `prerelease`
통합 흐름으로 한 줄씩 흘려보내는 application 서비스이다. 병렬 실행은 여러 슬롯에서
동시에 일어나지만, 실제 통합 브랜치에 cherry-pick/push/cleanup을 수행하는 단계는
직렬이어야 한다. 그래서 이 서비스는 planning authority에 저장된 queue record를
읽고, queue head 하나만 claim한 뒤 delivery 하위 모듈에 처리를 위임한다.

`GithubAutomationPort`는 push/PR/close 같은 원격 협업 동작을 담당하고,
`PlanningAuthorityPort`는 queue record와 session detail 같은 로컬 실행 원장을
담당한다. 이 둘을 주입받는 구조 덕분에 distributor 정책은 adapter 구현과 분리된다.
*/
pub(super) struct ParallelModeDistributorService {
    github_automation: Arc<dyn GithubAutomationPort>,
    planning_authority: Arc<dyn PlanningAuthorityPort>,
}

/*
queue head claim은 "이 프로세스가 지금 queue head를 처리 중"이라는 짧은
락이다. permit 타입이 `Drop`에서 claim을 release하므로, 정상 반환뿐 아니라 중간
오류로 함수가 빠져나가도 claim이 남아 다음 tick을 영구히 막지 않는다. Rust의 RAII
패턴을 application-level 분산 락에 적용한 예이다.
*/
struct DistributorQueueHeadClaimPermit {
    planning_authority: Arc<dyn PlanningAuthorityPort>,
    workspace_directory: String,
    queue_item_id: String,
    owner_token: String,
}
impl Drop for DistributorQueueHeadClaimPermit {
    fn drop(&mut self) {
        let _ = self.planning_authority.release_distributor_queue_claim(
            &self.workspace_directory,
            &self.queue_item_id,
            &self.owner_token,
        );
    }
}
impl ParallelModeDistributorService {
    pub(super) fn with_planning_authority(
        github_automation: Arc<dyn GithubAutomationPort>,
        planning_authority: Arc<dyn PlanningAuthorityPort>,
    ) -> Self {
        Self {
            github_automation,
            planning_authority,
        }
    }

    /*
    supervisor snapshot 안의 distributor 영역을 만들 때 호출되는 읽기 경로이다.
    mode가 켜져 있고 readiness가 통과된 상태에서만 실제 queue를 검사한다. 그 외에는
    placeholder snapshot을 반환해 화면은 안정적으로 유지하되, 사용자가 왜 queue 처리가
    멈춰 있는지 알 수 있게 한다.
    */
    pub(super) fn build_snapshot(
        &self,
        workspace_dir: &str,
        mode_enabled: bool,
        readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
    ) -> ParallelModeDistributorSnapshot {
        match readiness_snapshot {
            Some(snapshot) if mode_enabled && snapshot.allows_parallel_mode() => {
                self.inspect_snapshot(workspace_dir)
            }
            Some(_) if mode_enabled => build_placeholder_distributor_snapshot(
                "paused",
                "distributor waits for readiness recovery before queue processing",
            ),
            None if mode_enabled => build_placeholder_distributor_snapshot(
                "pending",
                "rerun readiness before distributor state can be trusted",
            ),
            Some(_) => build_placeholder_distributor_snapshot(
                "inactive",
                "enable parallel mode to surface live distributor activity",
            ),
            None => build_placeholder_distributor_snapshot("inactive", "parallel mode is off"),
        }
    }

    /*
    official completion이 "이 슬롯 결과는 commit-ready"라고 기록한 뒤,
    그 결과를 distributor queue record로 변환하는 함수이다. 여기서 lease 상태가
    Running인지, session detail이 commit_ready 계열인지, 같은 session_key의 queue record가
    이미 있는지를 차례로 확인한다. 이 방어선들은 중복 enqueue와 아직 준비되지 않은
    슬롯 결과의 조기 통합을 막는다.

    record에는 source branch, source commit sha, GitHub capability, 검증 요약을 함께
    저장한다. delivery 단계가 나중에 재시작되어도 queue record만 읽고 어떤 commit을
    어디까지 처리했는지 복원할 수 있게 하기 위해서이다.
    */
    pub(super) fn enqueue_workspace_commit_ready_result(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeDistributorQueueItem>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            return Ok(None);
        }
        let session_key = lease_session_key(&resolution.lease);
        let detail = resolution
            .context
            .session_details
            .iter()
            .find(|detail| detail.session_key == session_key)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "slot `{}` does not have a persisted session detail record",
                    resolution.lease.slot_id
                )
            })?;
        if !matches!(
            detail.state_label.as_str(),
            "commit_ready" | "merge_queued" | "integrating"
        ) {
            return Ok(None);
        }
        if let Some(existing) = find_distributor_queue_record_by_session_key(
            &resolution.context.distributor_queue_records,
            &session_key,
        ) {
            return Ok(Some(existing.display_item()));
        }
        let commit_sha =
            resolve_workspace_head_sha(&resolution.workspace_path).ok_or_else(|| {
                format!(
                    "slot `{}` workspace head could not be resolved for distributor enqueue",
                    resolution.lease.slot_id
                )
            })?;
        let github_capabilities = self
            .github_automation
            .inspect_capabilities(&resolution.context.repo_root);
        let timestamp = current_timestamp();
        /*
        The queue record freezes the source commit at enqueue time. Delivery may
        later rebase or rewrite commit_sha, while original_commit_sha preserves
        provenance for supervisor snapshots and operator recovery messages.
        */
        let record = ParallelModeDistributorQueueRecord {
            queue_item_id: distributor_queue_item_id(&resolution.lease, &timestamp),
            queue_order_key: queue_order_key_from_timestamp(&timestamp),
            session_key,
            root_turn_id: None,
            slot_id: resolution.lease.slot_id.clone(),
            agent_id: resolution.lease.agent_id.clone(),
            task_id: resolution.lease.task_id.clone(),
            task_title: resolution.lease.task_title.clone(),
            source_branch: resolution.lease.branch_name.clone(),
            source_commit_sha: commit_sha.clone(),
            branch_name: resolution.lease.branch_name.clone(),
            worktree_path: resolution.lease.worktree_path.clone(),
            original_commit_sha: Some(commit_sha.clone()),
            commit_sha,
            planning_refresh_state: "done".to_string(),
            integration_state: "queued".to_string(),
            conflict_files: Vec::new(),
            recovery_note: None,
            validation_summary: detail.validation_summary.clone(),
            authority_refresh_outcome: detail.authority_refresh_outcome.clone(),
            github_capabilities: Some(github_capabilities),
            pull_request_number: None,
            pull_request_url: None,
            queue_state: ParallelModeQueueItemState::Queued,
            integration_note: "commit-ready result accepted into distributor queue".to_string(),
            enqueued_at: timestamp.clone(),
            updated_at: timestamp,
        };
        /*
        Queue persistence happens before session detail is marked merge_queued.
        If the history write fails, the durable queue item still exists and the
        next supervisor snapshot can reconstruct distributor state from authority.
        */
        write_distributor_queue_record(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &record,
        )?;
        let _ = record_merge_queued_session_detail(
            self.planning_authority.as_ref(),
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            &resolution.lease,
        );
        Ok(Some(record.display_item()))
    }

    /*
    queue processing은 distributor의 실제 tick이다. 먼저 pool reconcile과
    runtime recovery를 수행해 재시작 이후의 record/lease 상태를 가능한 만큼 정상화한다.
    그 다음 Done이 아닌 첫 record만 head로 잡는다. 뒤쪽 queue item을 건너뛰지 않는
    이유는 통합 브랜치가 순서 의존적인 공유 자원이기 때문이다.

    head가 Blocked/Failed이면 사람이 복구해야 하므로 notice만 반환한다. 처리 가능한
    head라면 planning authority claim을 획득한 프로세스만 delivery를 진행한다.
    */
    pub(super) fn process_queue(&self, workspace_dir: &str) -> Result<Vec<String>, String> {
        let _ = reconcile_pool_board(self.planning_authority.as_ref(), workspace_dir);
        let context = self.recover_runtime_state(workspace_dir)?;
        let mut records = context.distributor_queue_records.clone();
        let Some(head_index) = records
            .iter()
            .position(|record| record.queue_state != ParallelModeQueueItemState::Done)
        else {
            return Ok(Vec::new());
        };
        let head = &mut records[head_index];
        /*
        Done records stay in the durable trace, so "first not Done" is the queue
        head. This preserves historical ordering while still preventing later
        queued work from jumping ahead of a blocked or cleaning item.
        */
        if matches!(
            head.queue_state,
            ParallelModeQueueItemState::Blocked | ParallelModeQueueItemState::Failed
        ) {
            return Ok(vec![format!(
                "distributor queue head is blocked / agent: {} / task: {} / {}",
                head.agent_id, head.task_id, head.integration_note
            )]);
        }
        let Some(_claim_permit) =
            self.acquire_queue_head_claim(workspace_dir, &head.queue_item_id)?
        else {
            return Ok(vec![format!(
                "distributor queue head is already claimed by another process / agent: {} / task: {}",
                head.agent_id, head.task_id
            )]);
        };

        process_distributor_queue_record(
            self.planning_authority.as_ref(),
            &context.repo_root,
            &context.pool_root,
            head,
            self.github_automation.as_ref(),
        )
    }

    // snapshot 읽기는 실패를 운영 오류로 끌어올리지 않고 placeholder로 접는다.
    // supervisor 화면은 distributor 저장소가 잠시 읽히지 않아도 전체 병렬 모드 상태를 계속 렌더링한다.
    fn inspect_snapshot(&self, workspace_dir: &str) -> ParallelModeDistributorSnapshot {
        match load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir) {
            Ok(context) => build_distributor_snapshot_from_context(&context),
            Err((_, detail)) => build_placeholder_distributor_snapshot(
                "unavailable",
                format!("distributor snapshot unavailable / {detail}"),
            ),
        }
    }

    // queue head claim은 delivery 직전에만 잡는다. recovery와 snapshot 작업은 claim 없이
    // 수행해 긴 선점 시간을 만들지 않고, 실제 공유 브랜치 변경 구간만 단일 처리자로 제한한다.
    fn acquire_queue_head_claim(
        &self,
        workspace_dir: &str,
        queue_item_id: &str,
    ) -> Result<Option<DistributorQueueHeadClaimPermit>, String> {
        let owner_token = distributor_claim_owner_token(queue_item_id);
        let acquired = self
            .planning_authority
            .try_acquire_distributor_queue_claim(workspace_dir, queue_item_id, &owner_token)
            .map_err(|error| error.to_string())?;
        if !acquired {
            return Ok(None);
        }
        Ok(Some(DistributorQueueHeadClaimPermit {
            planning_authority: self.planning_authority.clone(),
            workspace_directory: workspace_dir.to_string(),
            queue_item_id: queue_item_id.to_string(),
            owner_token,
        }))
    }

    /*
    runtime recovery는 queue tick 전에 저장된 queue record와 현재 git 상태를
    맞추는 재시작 복구 단계이다. 앱이 꺼진 사이에 PR 상태가 바뀌었거나, branch가 이미
    integration 브랜치에 들어갔거나, slot worktree checkout이 어긋난 상황을 감지해
    다시 queued/blocked/cleaning 같은 명시적 상태로 정리한다.

    이 복구가 process_queue 앞에 있는 이유는 delivery 로직이 "현재 record가 현실을
    충분히 반영한다"는 전제 위에서 단순한 상태 전이를 수행할 수 있게 하기 위해서이다.
    */
    pub(super) fn recover_runtime_state(
        &self,
        workspace_dir: &str,
    ) -> Result<PoolRuntimeContext, String> {
        let mut context =
            load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir)
                .map_err(|(_, detail)| detail.to_string())?;
        recover_stale_ledger_refreshing_sessions(
            self.planning_authority.as_ref(),
            workspace_dir,
            &context,
        )?;
        context = load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir)
            .map_err(|(_, detail)| detail.to_string())?;
        for index in 0..context.distributor_queue_records.len() {
            let mut record = context.distributor_queue_records[index].clone();
            let matching_lease = matching_lease_for_queue_record(&context, &record).cloned();
            /*
            Recovery runs the narrow, non-destructive fixes before broader state
            classification. A clean mismatched checkout or known retryable block
            can become Queued again without inspecting PR/integration state.
            */
            recover_mismatched_slot_worktree(
                self.planning_authority.as_ref(),
                &context.repo_root,
                &context.pool_root,
                matching_lease.as_ref(),
                &mut record,
            )?;
            recover_retryable_blocked_queue_record(
                self.planning_authority.as_ref(),
                &context.repo_root,
                &context.pool_root,
                matching_lease.as_ref(),
                &mut record,
            )?;
            context.distributor_queue_records[index] = record.clone();
            if !matches!(
                record.queue_state,
                ParallelModeQueueItemState::Idle
                    | ParallelModeQueueItemState::Done
                    | ParallelModeQueueItemState::Failed
            ) && matching_lease.is_none()
                && record_is_cleanup_recovery_candidate(&record)
                && !branch_exists(&context.repo_root, &record.branch_name)
            {
                /*
                Reconcile can finish slot cleanup before distributor recovery
                sees the blocked/cleaning record. With no lease and no source
                branch left, the durable queue item should close as recovered
                instead of remaining a permanent blocked head.
                */
                recover_integrated_queue_record(
                    self.planning_authority.as_ref(),
                    &context,
                    None,
                    &mut record,
                )?;
                context.distributor_queue_records[index] = record;
                continue;
            }
            if !matches!(
                record.queue_state,
                ParallelModeQueueItemState::Idle
                    | ParallelModeQueueItemState::Done
                    | ParallelModeQueueItemState::Failed
            ) && branch_is_integrated_into(
                &context.repo_root,
                &record.branch_name,
                DISTRIBUTOR_INTEGRATION_BRANCH,
            ) {
                /*
                Integration proof also recovers cleanup-time blocks. A queue
                item that already landed in prerelease should converge toward
                slot return instead of staying blocked at the head forever.
                */
                recover_integrated_queue_record(
                    self.planning_authority.as_ref(),
                    &context,
                    matching_lease.as_ref(),
                    &mut record,
                )?;
                context.distributor_queue_records[index] = record;
                continue;
            }
            if matches!(
                record.queue_state,
                ParallelModeQueueItemState::Idle
                    | ParallelModeQueueItemState::Done
                    | ParallelModeQueueItemState::Blocked
                    | ParallelModeQueueItemState::Failed
            ) {
                /*
                Terminal or operator-owned states are left alone. Blocked/Failed
                records need human recovery, while Done/Idle should not be
                rewritten by restart heuristics.
                */
                continue;
            }
            if !Path::new(&record.worktree_path).exists() {
                let _ = block_distributor_queue_record(
                    self.planning_authority.as_ref(),
                    &context.repo_root,
                    &context.pool_root,
                    matching_lease.as_ref(),
                    &mut record,
                    "recovered after restart: source worktree is missing; distributor cannot continue"
                        .to_string(),
                )?;
                context.distributor_queue_records[index] = record;
                continue;
            }
            if let Some(pr_number) = record.pull_request_number
                && let Ok(pull_request) = self
                    .github_automation
                    .inspect_pull_request(&context.repo_root, pr_number)
            {
                /*
                PR inspection is opportunistic recovery data. A fetch failure is
                ignored here so transient GitHub outages do not turn an otherwise
                processable queue record into a fresh block.
                */
                record.pull_request_url = Some(pull_request.url.clone());
                if !pull_request.state.eq_ignore_ascii_case("open") {
                    let _ = block_distributor_queue_record(
                        self.planning_authority.as_ref(),
                        &context.repo_root,
                        &context.pool_root,
                        matching_lease.as_ref(),
                        &mut record,
                        format!(
                            "recovered after restart: pull request #{pr_number} is `{}` before integration",
                            pull_request.state
                        ),
                    )?;
                    context.distributor_queue_records[index] = record;
                    continue;
                }
                if pull_request.is_draft {
                    let _ = block_distributor_queue_record(
                        self.planning_authority.as_ref(),
                        &context.repo_root,
                        &context.pool_root,
                        matching_lease.as_ref(),
                        &mut record,
                        format!(
                            "recovered after restart: pull request #{pr_number} is still a draft"
                        ),
                    )?;
                    context.distributor_queue_records[index] = record;
                    continue;
                }
                write_distributor_queue_record(
                    self.planning_authority.as_ref(),
                    &context.repo_root,
                    &context.pool_root,
                    &record,
                )?;
            }

            context.distributor_queue_records[index] = record;
        }
        Ok(context)
    }
}

fn recover_stale_ledger_refreshing_sessions(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    context: &PoolRuntimeContext,
) -> Result<(), String> {
    let pipeline_outcome = abandon_stale_official_refresh_order(
        planning_authority,
        workspace_dir,
        "stale ledger refresh recovery",
    )?;
    let mut recovered_order_consumed = false;
    for detail in &context.session_details {
        if detail.state_label != "ledger_refreshing" || !ledger_refreshing_detail_is_stale(detail) {
            continue;
        }
        let Some(lease) = context
            .slot_leases
            .values()
            .find(|lease| lease.session_key() == detail.session_key)
        else {
            continue;
        };
        if context.distributor_queue_records.iter().any(|record| {
            record.session_key == detail.session_key && record.queue_state.is_active()
        }) {
            continue;
        }

        let recovery_reason = format!(
            "stale official ledger refresh for `{}` exceeded {} seconds without queue handoff",
            detail.task_title, STALE_LEDGER_REFRESHING_AFTER_SECS
        );
        match pipeline_outcome {
            StaleOfficialRefreshRecoveryOutcome::WaitingForActiveClaim => continue,
            StaleOfficialRefreshRecoveryOutcome::NoPendingOrder => {
                record_official_completion_recovery_needed_session_detail(
                    planning_authority,
                    &context.repo_root,
                    &context.pool_root,
                    lease,
                    &format!(
                        "{recovery_reason}; manual official refresh recovery is needed before distributor handoff"
                    ),
                )?;
            }
            StaleOfficialRefreshRecoveryOutcome::RecoveredOrder => {
                if recovered_order_consumed {
                    continue;
                }
                recovered_order_consumed = true;
                record_official_completion_recovery_needed_session_detail(
                    planning_authority,
                    &context.repo_root,
                    &context.pool_root,
                    lease,
                    &format!(
                        "{recovery_reason}; manual official refresh recovery is needed before distributor handoff"
                    ),
                )?;
            }
        }
    }

    Ok(())
}

enum StaleOfficialRefreshRecoveryOutcome {
    RecoveredOrder,
    NoPendingOrder,
    WaitingForActiveClaim,
}

fn abandon_stale_official_refresh_order(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
    recovery_reason: &str,
) -> Result<StaleOfficialRefreshRecoveryOutcome, String> {
    match planning_authority
        .abandon_next_official_refresh_order(workspace_dir, recovery_reason)
        .map_err(|error| error.to_string())?
    {
        PlanningAuthorityOfficialRefreshRecoveryStatus::Recovered { .. } => {
            Ok(StaleOfficialRefreshRecoveryOutcome::RecoveredOrder)
        }
        PlanningAuthorityOfficialRefreshRecoveryStatus::NoPendingOrder => {
            Ok(StaleOfficialRefreshRecoveryOutcome::NoPendingOrder)
        }
        PlanningAuthorityOfficialRefreshRecoveryStatus::WaitingForActiveClaim => {
            Ok(StaleOfficialRefreshRecoveryOutcome::WaitingForActiveClaim)
        }
    }
}

fn ledger_refreshing_detail_is_stale(detail: &ParallelModeAgentSessionDetailSnapshot) -> bool {
    let Ok(timestamp) = DateTime::parse_from_rfc3339(&detail.updated_at) else {
        return true;
    };
    Utc::now().signed_duration_since(timestamp.with_timezone(&Utc))
        >= TimeDelta::seconds(STALE_LEDGER_REFRESHING_AFTER_SECS)
}

/*
blocked record가 실제 lease와 같은 branch/worktree를 가리키는데 worktree만
다른 브랜치에 체크아웃되어 있으면, 깨끗한 worktree에 한해 원래 lease branch로 되돌리고
queued로 복구한다. 사용자의 변경이 있는 슬롯을 자동 checkout하지 않는 이유는 복구가
사용자 작업을 덮어쓰는 동작이 되면 안 되기 때문이다.
*/
fn recover_mismatched_slot_worktree(
    planning_authority: &dyn PlanningAuthorityPort,
    repo_root: &str,
    pool_root: &Path,
    matching_lease: Option<&ParallelModeSlotLeaseSnapshot>,
    record: &mut ParallelModeDistributorQueueRecord,
) -> Result<(), String> {
    let Some(lease) = matching_lease else {
        return Ok(());
    };
    if record.queue_state != ParallelModeQueueItemState::Blocked {
        return Ok(());
    }
    if record.branch_name != lease.branch_name || record.worktree_path != lease.worktree_path {
        return Ok(());
    }
    if !Path::new(&record.worktree_path).exists() {
        return Ok(());
    }
    if !branch_exists(repo_root, &lease.branch_name) {
        return Ok(());
    }
    if current_branch_name(Path::new(&record.worktree_path)).as_deref()
        == Some(lease.branch_name.as_str())
    {
        return Ok(());
    }
    let Some(slot_status) = inspect_slot_git_status(Path::new(&record.worktree_path)) else {
        return Ok(());
    };
    if !slot_status.is_clean_baseline() {
        return Ok(());
    }
    if !command_succeeds(
        "git",
        [
            "-C",
            record.worktree_path.as_str(),
            "checkout",
            lease.branch_name.as_str(),
        ],
    ) {
        return Ok(());
    }

    record.queue_state = ParallelModeQueueItemState::Queued;
    record.integration_state = "queued".to_string();
    record.recovery_note =
        Some("recovered mismatched clean slot worktree checkout before retry".to_string());
    record.integration_note =
        "recovered clean slot worktree checkout and queued distributor retry".to_string();
    record.updated_at = current_timestamp();
    write_distributor_queue_record(planning_authority, repo_root, pool_root, record)?;
    Ok(())
}

/*
모든 block이 영구 실패는 아니다. GitHub inspection 실패, PR 생성 실패,
일시적인 cherry-pick/clean worktree 문제처럼 사용자가 상태를 바로잡거나 외부 조건이
회복되면 같은 queue item을 다시 시도할 수 있는 block이 있다. 이 함수는 그런 record를
안전 조건이 맞을 때 다시 Queued로 돌려 다음 tick에서 delivery가 이어지게 한다.
*/
fn recover_retryable_blocked_queue_record(
    planning_authority: &dyn PlanningAuthorityPort,
    repo_root: &str,
    pool_root: &Path,
    matching_lease: Option<&ParallelModeSlotLeaseSnapshot>,
    record: &mut ParallelModeDistributorQueueRecord,
) -> Result<(), String> {
    let Some(lease) = matching_lease else {
        return Ok(());
    };
    if record.queue_state != ParallelModeQueueItemState::Blocked {
        return Ok(());
    }
    if !is_retryable_distributor_block(&record.integration_note) {
        return Ok(());
    }
    if record.branch_name != lease.branch_name || record.worktree_path != lease.worktree_path {
        return Ok(());
    }
    if current_branch_name(Path::new(&record.worktree_path)).as_deref()
        != Some(lease.branch_name.as_str())
    {
        return Ok(());
    }
    let Some(slot_status) = inspect_slot_git_status(Path::new(&record.worktree_path)) else {
        return Ok(());
    };
    if slot_status.has_pending_operation {
        return Ok(());
    }

    record.queue_state = ParallelModeQueueItemState::Queued;
    record.integration_state = "queued".to_string();
    record.recovery_note = Some("recovered retryable distributor block before retry".to_string());
    record.integration_note = "recovered retryable distributor block and queued retry".to_string();
    record.updated_at = current_timestamp();
    write_distributor_queue_record(planning_authority, repo_root, pool_root, record)?;
    Ok(())
}

// retryable block 목록은 delivery가 남기는 integration_note 문구와 맞물린다.
// 영구 복구가 필요한 상태까지 자동 재시도하지 않도록 명시적으로 알려진 임시 실패만 통과시킨다.
fn is_retryable_distributor_block(detail: &str) -> bool {
    detail.contains("pull request ensure failed")
        || detail.contains("could not be inspected")
        || detail.contains("could not cherry-pick")
        || detail.contains("integration worktree must be clean before cherry-pick delivery")
        || detail.contains("push capability is unavailable for distributor delivery")
        || detail.contains("source branch `") && detail.contains("` could not be pushed to `")
        || detail.contains(INTEGRATION_BRANCH_PUSH_BLOCK_FRAGMENT)
        || detail.contains("source branch was pushed but GitHub automation is unavailable")
}

fn record_is_cleanup_recovery_candidate(record: &ParallelModeDistributorQueueRecord) -> bool {
    record.queue_state == ParallelModeQueueItemState::Cleaning
        || record
            .integration_note
            .contains("cleanup failed after distributor delivery")
        || record.integration_note.contains("slot is entering cleanup")
        || record.integration_note.contains("slot returned to idle")
        || record
            .integration_note
            .contains("GitHub delivery completed")
}

/*
queue record와 live lease를 연결할 때 session_key가 1차 키이다. 오래된
record나 복구 중 생성된 record가 session_key만으로 맞지 않을 수 있어, branch/worktree
조합을 보조 키로 한 번 더 찾는다. 이 보조 매칭은 재시작 복구에서 cleanup pending lease를
찾아 queue 상태를 끝까지 수렴시키는 데 필요하다.
*/
fn matching_lease_for_queue_record<'a>(
    context: &'a PoolRuntimeContext,
    record: &ParallelModeDistributorQueueRecord,
) -> Option<&'a ParallelModeSlotLeaseSnapshot> {
    context
        .slot_leases
        .values()
        .find(|lease| lease_session_key(lease) == record.session_key)
        .or_else(|| {
            context.slot_leases.values().find(|lease| {
                lease.branch_name == record.branch_name
                    && lease.worktree_path == record.worktree_path
            })
        })
}

/*
앱 재시작 후 source branch가 이미 integration branch에 포함되어 있다면,
delivery는 "통합 완료 후 cleanup만 남은 상태"로 복구해야 한다. matching lease가 있으면
lease를 CleanupPending으로 옮겨 슬롯 반환 경로를 태우고, lease가 없고 branch도 없으면
이미 정리가 끝난 것으로 보고 record를 Done으로 닫는다.
*/
fn recover_integrated_queue_record(
    planning_authority: &dyn PlanningAuthorityPort,
    context: &PoolRuntimeContext,
    matching_lease: Option<&ParallelModeSlotLeaseSnapshot>,
    record: &mut ParallelModeDistributorQueueRecord,
) -> Result<(), String> {
    if let Some(lease) = matching_lease {
        if lease.state == ParallelModeSlotLeaseState::Running {
            let mut cleanup_pending_lease = lease.clone();
            cleanup_pending_lease.state = ParallelModeSlotLeaseState::CleanupPending;
            write_slot_lease(
                planning_authority,
                &context.repo_root,
                &context.pool_root,
                &cleanup_pending_lease,
            )?;
            let _ = record_cleanup_pending_session_detail(
                planning_authority,
                &context.repo_root,
                &context.pool_root,
                &cleanup_pending_lease,
            );
        }
    } else if !branch_exists(&context.repo_root, &record.branch_name) {
        record.queue_state = ParallelModeQueueItemState::Done;
        record.integration_note = format!(
            "recovered after restart: branch is already integrated into {DISTRIBUTOR_INTEGRATION_BRANCH} and slot cleanup completed"
        );
        record.updated_at = current_timestamp();
        write_distributor_queue_record(
            planning_authority,
            &context.repo_root,
            &context.pool_root,
            record,
        )?;
        return Ok(());
    }

    record.queue_state = ParallelModeQueueItemState::Cleaning;
    record.integration_note = format!(
        "recovered after restart: branch is already integrated into {DISTRIBUTOR_INTEGRATION_BRANCH} and cleanup is pending"
    );
    record.updated_at = current_timestamp();
    write_distributor_queue_record(
        planning_authority,
        &context.repo_root,
        &context.pool_root,
        record,
    )?;
    Ok(())
}

// enqueue는 session_key를 idempotency key로 쓴다. 같은 slot completion이 재전달되어도
// 새 queue item을 만들지 않고 기존 display row를 돌려 중복 통합을 막는다.
fn find_distributor_queue_record_by_session_key(
    queue_records: &[ParallelModeDistributorQueueRecord],
    session_key: &str,
) -> Option<ParallelModeDistributorQueueRecord> {
    queue_records
        .iter()
        .find(|record| record.session_key == session_key)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::is_retryable_distributor_block;

    #[test]
    fn retryable_push_block_matching_accepts_known_delivery_pushes() {
        assert!(is_retryable_distributor_block(
            "source branch `akra-agent/slot-1/task-one` could not be pushed to `origin`: temporary remote failure"
        ));
        assert!(is_retryable_distributor_block(
            "`prerelease` could not be pushed to `origin`: non-fast-forward"
        ));
        assert!(!is_retryable_distributor_block(
            "`feature` could not be pushed to `origin`: unsupported integration branch"
        ));
    }
}
