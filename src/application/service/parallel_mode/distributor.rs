use super::pool::{
    acquire_pool_mutation_lock, reconcile_pool_board_and_context_with_target,
    reconcile_pool_board_and_context_with_target_locked,
};
use super::{
    FreshPoolIntegrationTargetProof, ParallelModeDeliverySafetyPolicy, PoolRuntimeContext,
    PoolSlotCleanupIdentity, PoolSlotCleanupLeaseAuthority, WorkspaceSlotLeaseResolution,
    branch_exists, cleanup_slot_to_ref_locked, command_succeeds, current_branch_name,
    current_timestamp, inspect_slot_git_status, lease_session_key, load_pool_runtime_context,
    record_cleaned_session_detail, record_cleanup_pending_session_detail,
    record_integrating_session_detail, record_merge_pending_session_detail,
    record_merge_queued_session_detail, record_official_completion_recovery_needed_session_detail,
    record_pr_pending_session_detail, record_pushing_session_detail, remote_branch_name,
    remote_tracking_branch_ref, resolve_workspace_head_sha, resolve_workspace_slot_lease,
    run_command, short_sha, transition_slot_lease, try_parallel_mode_integration_branch_for_repo,
    try_push_remote_name,
};
use crate::application::port::outbound::github_automation_port::{
    GithubAutomationPort, GithubAutomationPullRequest, GithubRepositoryVisibility,
};
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorDeliveryTarget, PlanningAuthorityDistributorQueueRecord,
    PlanningAuthorityOfficialRefreshRecoveryStatus, PlanningAuthorityPort,
};
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeDeliveryTargetSnapshot,
    ParallelModeDistributorQueueItem, ParallelModeDistributorSnapshot, ParallelModeQueueItemState,
    ParallelModeReadinessSnapshot, ParallelModeRepositoryVisibility, ParallelModeSlotLeaseSnapshot,
    ParallelModeSlotLeaseState,
};
use chrono::{DateTime, TimeDelta, Utc};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

const STALE_LEDGER_REFRESHING_AFTER_SECS: i64 = 300;
const DISTRIBUTOR_RETRY_BASE_DELAY_SECS: i64 = 5;
const DISTRIBUTOR_RETRY_MAX_DELAY_SECS: i64 = 300;
const DISTRIBUTOR_RETRY_MAX_ATTEMPTS: u32 = 8;
const MAX_DISTRIBUTOR_SOURCE_COMMITS: usize = 128;
pub(super) type ParallelModeDistributorQueueRecord = PlanningAuthorityDistributorQueueRecord;
mod delivery;
#[cfg(test)]
pub(super) use self::delivery::install_before_distributor_cleanup_lock_hook;
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
use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;

fn fetch_distributor_integration_target(
    github_automation: &dyn GithubAutomationPort,
    repo_root: &str,
    target: &PlanningAuthorityDistributorDeliveryTarget,
) -> Result<IntegrationTargetProof, String> {
    let push_url = target
        .credential_redacted_push_url
        .as_deref()
        .ok_or_else(|| {
            "immutable distributor target has no credential-redacted push URL".to_string()
        })?;
    fetch_integration_target_proof(
        github_automation,
        repo_root,
        &target.push_remote,
        push_url,
        &target.integration_branch,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IntegrationTargetProof {
    push_remote: String,
    integration_branch: String,
    commit_sha: String,
}

impl IntegrationTargetProof {
    fn matches(&self, target: &PlanningAuthorityDistributorDeliveryTarget) -> bool {
        self.push_remote == target.push_remote
            && self.integration_branch == target.integration_branch
    }
}

fn fetch_integration_target_proof(
    github_automation: &dyn GithubAutomationPort,
    repo_root: &str,
    push_remote: &str,
    credential_redacted_push_url: &str,
    integration_branch: &str,
) -> Result<IntegrationTargetProof, String> {
    let remote_ref = remote_tracking_branch_ref(push_remote, integration_branch);
    let commit_sha = github_automation
        .fetch_branch_to_tracking_ref_for_delivery_target(
            repo_root,
            push_remote,
            credential_redacted_push_url,
            integration_branch,
            &remote_ref,
        )
        .map_err(|error| {
            format!(
                "could not refresh integration target `{push_remote}/{integration_branch}` through the isolated target: {error}"
            )
        })?;
    Ok(IntegrationTargetProof {
        push_remote: push_remote.to_string(),
        integration_branch: integration_branch.to_string(),
        commit_sha,
    })
}

fn freeze_distributor_source_range(
    github_automation: &dyn GithubAutomationPort,
    repo_root: &str,
    target: &PlanningAuthorityDistributorDeliveryTarget,
    expected_integration_base_commit_sha: &str,
    source_tip: &str,
) -> Result<(String, Vec<String>), String> {
    let integration_target =
        fetch_distributor_integration_target(github_automation, repo_root, target).map_err(
            |_| {
                format!(
                    "could not refresh frozen integration target `{}/{}` before enqueue",
                    target.push_remote, target.integration_branch
                )
            },
        )?;
    if integration_target.commit_sha != expected_integration_base_commit_sha {
        return Err(format!(
            "integration target moved from lease-frozen base `{}` to `{}` before enqueue",
            short_sha(expected_integration_base_commit_sha),
            short_sha(&integration_target.commit_sha)
        ));
    }
    let source_base = run_command(
        "git",
        [
            "-C",
            repo_root,
            "merge-base",
            source_tip,
            integration_target.commit_sha.as_str(),
        ],
        None,
    )
    .ok_or_else(|| {
        format!(
            "source result `{}` has no merge-base with `{}`",
            short_sha(source_tip),
            integration_target.commit_sha
        )
    })?;
    let commits = resolve_linear_distributor_source_range(repo_root, &source_base, source_tip)?;
    Ok((source_base, commits))
}

fn resolve_linear_distributor_source_range(
    repo_root: &str,
    source_base: &str,
    source_tip: &str,
) -> Result<Vec<String>, String> {
    if source_base.trim().is_empty() || source_tip.trim().is_empty() || source_base == source_tip {
        return Err("distributor source commit range is empty or incomplete".to_string());
    }
    if !command_succeeds(
        "git",
        [
            "-C",
            repo_root,
            "merge-base",
            "--is-ancestor",
            source_base,
            source_tip,
        ],
    ) {
        return Err("frozen source base is not an ancestor of the source tip".to_string());
    }

    let range = format!("{source_base}..{source_tip}");
    let merge_count = run_command(
        "git",
        [
            "-C",
            repo_root,
            "rev-list",
            "--count",
            "--min-parents=2",
            range.as_str(),
        ],
        None,
    )
    .and_then(|value| value.parse::<usize>().ok())
    .ok_or_else(|| "source merge-commit count could not be resolved".to_string())?;
    if merge_count > 0 {
        return Err(
            "source commit range contains merge commits; rebase or squash it before delivery"
                .to_string(),
        );
    }

    let count = run_command(
        "git",
        ["-C", repo_root, "rev-list", "--count", range.as_str()],
        None,
    )
    .and_then(|value| value.parse::<usize>().ok())
    .ok_or_else(|| "source commit count could not be resolved".to_string())?;
    if count == 0 || count > MAX_DISTRIBUTOR_SOURCE_COMMITS {
        return Err(format!(
            "source commit range must contain 1..={MAX_DISTRIBUTOR_SOURCE_COMMITS} commits; found {count}"
        ));
    }

    let commits = run_command(
        "git",
        [
            "-C",
            repo_root,
            "rev-list",
            "--reverse",
            "--topo-order",
            range.as_str(),
        ],
        None,
    )
    .map(|value| value.lines().map(str::to_string).collect::<Vec<_>>())
    .ok_or_else(|| "source commit range could not be enumerated".to_string())?;
    if commits.len() != count || commits.last().map(String::as_str) != Some(source_tip) {
        return Err("source commit range enumeration did not match its frozen tip".to_string());
    }
    Ok(commits)
}

pub(super) fn distributor_source_cherry_states(
    repo_root: &str,
    upstream: &str,
    record: &ParallelModeDistributorQueueRecord,
) -> Result<Vec<(String, bool)>, String> {
    let source_tip = record.effective_source_commit_sha();
    let commits = resolve_linear_distributor_source_range(
        repo_root,
        &record.source_base_commit_sha,
        &source_tip,
    )?;
    // `git cherry` omits source commits that are already literal ancestors of
    // upstream. Classify those first so the exact frozen range can still be
    // accounted for without attempting to cherry-pick them again.
    let mut states = commits
        .iter()
        .filter(|commit| {
            command_succeeds(
                "git",
                [
                    "-C",
                    repo_root,
                    "merge-base",
                    "--is-ancestor",
                    commit.as_str(),
                    upstream,
                ],
            )
        })
        .map(|commit| (commit.clone(), true))
        .collect::<BTreeMap<_, _>>();
    if states.len() == commits.len() {
        return Ok(commits.into_iter().map(|commit| (commit, true)).collect());
    }

    let output = run_command(
        "git",
        [
            "-C",
            repo_root,
            "cherry",
            upstream,
            source_tip.as_str(),
            record.source_base_commit_sha.as_str(),
        ],
        None,
    )
    .ok_or_else(|| "source commit patch-equivalence could not be inspected".to_string())?;
    for line in output.lines() {
        let mut fields = line.split_whitespace();
        let marker = fields.next().unwrap_or_default();
        let sha = fields.next().unwrap_or_default();
        if fields.next().is_some()
            || !matches!(marker, "+" | "-")
            || !commits.iter().any(|commit| commit == sha)
            || states.insert(sha.to_string(), marker == "-").is_some()
        {
            return Err("git cherry returned an invalid source commit mapping".to_string());
        }
    }
    if states.len() != commits.len() {
        return Err("git cherry did not classify the complete frozen source range".to_string());
    }
    commits
        .into_iter()
        .map(|sha| {
            states
                .remove(&sha)
                .map(|equivalent| (sha, equivalent))
                .ok_or_else(|| "git cherry omitted a frozen source commit".to_string())
        })
        .collect()
}

fn validate_distributor_delivery_target<'a>(
    github_automation: &dyn GithubAutomationPort,
    repo_root: &str,
    record: &'a ParallelModeDistributorQueueRecord,
) -> Result<&'a PlanningAuthorityDistributorDeliveryTarget, String> {
    let Some(target) = record.delivery_target.as_ref() else {
        return Err(
            "legacy distributor queue record has no immutable delivery target; inspect the queued commit and re-enqueue it explicitly"
                .to_string(),
        );
    };
    let Some(frozen_push_url) = target.credential_redacted_push_url.as_deref() else {
        return Err(
            "legacy distributor queue record has no credential-redacted immutable push URL; inspect the queued commit and re-enqueue it explicitly"
                .to_string(),
        );
    };
    let current_push_remote = try_push_remote_name(repo_root)?;
    let current_integration_branch = try_parallel_mode_integration_branch_for_repo(repo_root)?;
    let mut configured_target_drift = Vec::new();
    if target.push_remote != current_push_remote {
        configured_target_drift.push(format!(
            "push remote `{}` -> `{current_push_remote}`",
            target.push_remote
        ));
    }
    if target.integration_branch != current_integration_branch {
        configured_target_drift.push(format!(
            "integration branch `{}` -> `{current_integration_branch}`",
            target.integration_branch
        ));
    }
    if !configured_target_drift.is_empty() {
        return Err(format!(
            "immutable distributor delivery target drifted: {}; restore the original configuration or re-enqueue after operator review",
            configured_target_drift.join(", ")
        ));
    }
    let current_push_url = github_automation
        .credential_redacted_push_url_for_remote(repo_root, &current_push_remote)
        .map_err(|error| {
            format!("credential-redacted GitHub push URL could not be verified: {error}")
        })?;
    let current_repository = github_automation
        .repository_identity_for_push_url(repo_root, &current_push_remote, &current_push_url)
        .map_err(|error| format!("GitHub repository identity could not be verified: {error}"))?;
    let current_visibility = github_automation
        .repository_visibility_for_push_url(repo_root, &current_push_remote, &current_push_url)
        .map_err(|error| format!("GitHub repository visibility could not be verified: {error}"))?;
    let mut drift = Vec::new();
    if frozen_push_url != current_push_url {
        drift.push("credential-redacted push URL changed".to_string());
    }
    if target.github_repository != current_repository {
        drift.push(format!(
            "GitHub repository `{}` -> `{}`",
            target.github_repository, current_repository
        ));
    }
    if target.repository_visibility != current_visibility {
        drift.push(format!(
            "repository visibility `{:?}` -> `{:?}`",
            target.repository_visibility, current_visibility
        ));
    }
    if drift.is_empty() {
        Ok(target)
    } else {
        Err(format!(
            "immutable distributor delivery target drifted: {}; restore the original configuration or re-enqueue after operator review",
            drift.join(", ")
        ))
    }
}

fn validate_distributor_delivery_target_with_policy<'a>(
    github_automation: &dyn GithubAutomationPort,
    repo_root: &str,
    record: &'a ParallelModeDistributorQueueRecord,
    delivery_safety_policy: &ParallelModeDeliverySafetyPolicy,
) -> Result<&'a PlanningAuthorityDistributorDeliveryTarget, String> {
    let target = validate_distributor_delivery_target(github_automation, repo_root, record)?;
    if target.repository_visibility == GithubRepositoryVisibility::Public
        && !delivery_safety_policy.allow_public_repository.clone()?
    {
        return Err(
            "public GitHub repository delivery is blocked; parent-process opt-in is not active"
                .to_string(),
        );
    }
    Ok(target)
}

fn validate_lease_delivery_target(
    github_automation: &dyn GithubAutomationPort,
    repo_root: &str,
    target: &ParallelModeDeliveryTargetSnapshot,
    delivery_safety_policy: &ParallelModeDeliverySafetyPolicy,
) -> Result<PlanningAuthorityDistributorDeliveryTarget, String> {
    let Some(frozen_push_url) = target.credential_redacted_push_url.as_deref() else {
        return Err(
            "legacy slot lease has no credential-redacted immutable push URL; discard the lease and dispatch again after operator review"
                .to_string(),
        );
    };
    let current_push_remote = try_push_remote_name(repo_root)?;
    let current_integration_branch = try_parallel_mode_integration_branch_for_repo(repo_root)?;
    let mut configured_target_drift = Vec::new();
    if target.push_remote != current_push_remote {
        configured_target_drift.push(format!(
            "push remote `{}` -> `{current_push_remote}`",
            target.push_remote
        ));
    }
    if target.integration_branch != current_integration_branch {
        configured_target_drift.push(format!(
            "integration branch `{}` -> `{current_integration_branch}`",
            target.integration_branch
        ));
    }
    if !configured_target_drift.is_empty() {
        return Err(format!(
            "slot lease delivery target drifted after worker start: {}; discard the lease and dispatch again after operator review",
            configured_target_drift.join(", ")
        ));
    }
    let current_push_url = github_automation
        .credential_redacted_push_url_for_remote(repo_root, &current_push_remote)
        .map_err(|error| {
            format!("credential-redacted GitHub push URL could not be verified: {error}")
        })?;
    let current_repository = github_automation
        .repository_identity_for_push_url(repo_root, &current_push_remote, &current_push_url)
        .map_err(|error| format!("GitHub repository identity could not be verified: {error}"))?;
    let current_visibility = github_automation
        .repository_visibility_for_push_url(repo_root, &current_push_remote, &current_push_url)
        .map_err(|error| format!("GitHub repository visibility could not be verified: {error}"))?;
    if current_visibility == GithubRepositoryVisibility::Public
        && !delivery_safety_policy.allow_public_repository.clone()?
    {
        return Err(
            "public GitHub repository delivery is blocked; parent-process opt-in is not active"
                .to_string(),
        );
    }

    let frozen_visibility = match target.repository_visibility {
        ParallelModeRepositoryVisibility::Private => GithubRepositoryVisibility::Private,
        ParallelModeRepositoryVisibility::Internal => GithubRepositoryVisibility::Internal,
        ParallelModeRepositoryVisibility::Public => GithubRepositoryVisibility::Public,
    };
    let mut drift = Vec::new();
    if frozen_push_url != current_push_url {
        drift.push("credential-redacted push URL changed".to_string());
    }
    if target.github_repository != current_repository {
        drift.push(format!(
            "GitHub repository `{}` -> `{current_repository}`",
            target.github_repository
        ));
    }
    if frozen_visibility != current_visibility {
        drift.push(format!(
            "repository visibility `{:?}` -> `{:?}`",
            frozen_visibility, current_visibility
        ));
    }
    if !drift.is_empty() {
        return Err(format!(
            "slot lease delivery target drifted after worker start: {}; discard the lease and dispatch again after operator review",
            drift.join(", ")
        ));
    }

    Ok(PlanningAuthorityDistributorDeliveryTarget::new(
        target.push_remote.clone(),
        target.github_repository.clone(),
        frozen_visibility,
        target.integration_branch.clone(),
    )
    .with_credential_redacted_push_url(frozen_push_url))
}

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
    parallel_runtime: Arc<dyn ParallelModeRuntimePort>,
    pub(super) delivery_safety_policy: ParallelModeDeliverySafetyPolicy,
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
impl DistributorQueueHeadClaimPermit {
    fn renew(&self, stage: &str) -> Result<(), String> {
        match self.planning_authority.renew_distributor_queue_claim(
            &self.workspace_directory,
            &self.queue_item_id,
            &self.owner_token,
        ) {
            Ok(true) => Ok(()),
            Ok(false) => Err(format!(
                "distributor queue claim ownership was lost before {stage}; no further side effect was started"
            )),
            Err(error) => Err(format!(
                "distributor queue claim could not be renewed before {stage}: {error}"
            )),
        }
    }
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
        parallel_runtime: Arc<dyn ParallelModeRuntimePort>,
        delivery_safety_policy: ParallelModeDeliverySafetyPolicy,
    ) -> Self {
        Self {
            github_automation,
            planning_authority,
            parallel_runtime,
            delivery_safety_policy,
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

    record에는 source branch, fetched integration merge-base, reviewed source tip, GitHub
    capability, 검증 요약을 함께 저장한다. delivery 단계가 나중에 재시작되어도 queue
    record만 읽고 검토된 선형 commit 범위 전체와 처리 상태를 복원할 수 있게 하기 위해서이다.
    */
    pub(super) fn enqueue_workspace_commit_ready_result(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeDistributorQueueItem>, String> {
        self.enqueue_workspace_commit_ready_result_with_permit(workspace_dir, None, None)
    }

    pub(super) fn enqueue_workspace_commit_ready_result_for_lease(
        &self,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<Option<ParallelModeDistributorQueueItem>, String> {
        self.enqueue_workspace_commit_ready_result_with_permit(
            &expected_lease.worktree_path,
            Some(expected_lease),
            None,
        )
    }

    pub(super) fn enqueue_workspace_commit_ready_result_guarded(
        &self,
        workspace_dir: &str,
        permit: &super::ParallelModeAutomationPermit,
    ) -> Result<Option<ParallelModeDistributorQueueItem>, String> {
        self.enqueue_workspace_commit_ready_result_with_permit(workspace_dir, None, Some(permit))
    }

    fn enqueue_workspace_commit_ready_result_with_permit(
        &self,
        workspace_dir: &str,
        expected_lease: Option<&ParallelModeSlotLeaseSnapshot>,
        permit: Option<&super::ParallelModeAutomationPermit>,
    ) -> Result<Option<ParallelModeDistributorQueueItem>, String> {
        if permit.is_some_and(|permit| !permit.is_active()) {
            return Ok(None);
        }
        let pool_mutation_lock =
            acquire_pool_mutation_lock(self.planning_authority.as_ref(), workspace_dir)?;
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        pool_mutation_lock.verify_pool_root(&resolution.context.pool_root)?;
        if expected_lease.is_some_and(|expected| !resolution.lease.same_generation_as(expected)) {
            return Ok(None);
        }
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
        let source_status =
            inspect_slot_git_status(&resolution.workspace_path).map_err(|error| {
                format!(
                    "slot `{}` git status could not be inspected for distributor enqueue: {error}",
                    resolution.lease.slot_id
                )
            })?;
        if !source_status.is_clean_for_frozen_delivery() {
            return Err(format!(
                "slot `{}` has nonignored worktree changes and cannot enter the distributor queue",
                resolution.lease.slot_id
            ));
        }
        let commit_sha =
            resolve_workspace_head_sha(&resolution.workspace_path).ok_or_else(|| {
                format!(
                    "slot `{}` workspace head could not be resolved for distributor enqueue",
                    resolution.lease.slot_id
                )
            })?;
        let lease_delivery_target = resolution.lease.delivery_target.as_ref().ok_or_else(|| {
            "legacy slot lease has no immutable delivery target; discard it and dispatch the task again after operator review"
                .to_string()
        })?;
        let delivery_target = validate_lease_delivery_target(
            self.github_automation.as_ref(),
            &resolution.context.repo_root,
            lease_delivery_target,
            &self.delivery_safety_policy,
        )?;
        let (source_base_commit_sha, source_commits) = freeze_distributor_source_range(
            self.github_automation.as_ref(),
            &resolution.context.repo_root,
            &delivery_target,
            &lease_delivery_target.integration_base_commit_sha,
            &commit_sha,
        )?;
        let github_capabilities = self
            .github_automation
            .inspect_capabilities(&resolution.context.repo_root);
        let updated_at = current_timestamp();
        let enqueued_at = DateTime::parse_from_rfc3339(&detail.updated_at)
            .map(|_| detail.updated_at.clone())
            .unwrap_or_else(|_| updated_at.clone());
        /*
        The queue record freezes the remote integration merge-base and source tip
        at enqueue time. Delivery reconstructs this exact linear range, so a
        multi-commit PR and the commits integrated into the target cannot diverge.
        */
        let record = ParallelModeDistributorQueueRecord {
            queue_item_id: distributor_queue_item_id(&resolution.lease, &enqueued_at),
            queue_order_key: queue_order_key_from_timestamp(&enqueued_at),
            session_key,
            slot_id: resolution.lease.slot_id.clone(),
            agent_id: resolution.lease.agent_id.clone(),
            task_id: resolution.lease.task_id.clone(),
            task_title: resolution.lease.task_title.clone(),
            delivery_target: Some(delivery_target),
            source_branch: resolution.lease.branch_name.clone(),
            source_base_commit_sha,
            source_commit_sha: commit_sha.clone(),
            branch_name: resolution.lease.branch_name.clone(),
            worktree_path: resolution.lease.worktree_path.clone(),
            original_commit_sha: Some(commit_sha.clone()),
            commit_sha,
            planning_refresh_state: "done".to_string(),
            integration_state: "queued".to_string(),
            integration_base_commit_sha: None,
            integration_commit_sha: None,
            conflict_files: Vec::new(),
            recovery_note: None,
            validation_summary: detail.validation_summary.clone(),
            authority_refresh_outcome: detail.authority_refresh_outcome.clone(),
            github_capabilities: Some(github_capabilities),
            pull_request_number: None,
            pull_request_url: None,
            queue_state: ParallelModeQueueItemState::Queued,
            integration_note: format!(
                "commit-ready result accepted into distributor queue / source commits: {}",
                source_commits.len()
            ),
            enqueued_at,
            updated_at,
            retry_attempts: 0,
            retry_not_before: None,
        };
        /*
        Queue persistence happens before session detail is marked merge_queued.
        If the history write fails, the durable queue item still exists and the
        next supervisor snapshot can reconstruct distributor state from authority.
        */
        let persist = || -> Result<Option<ParallelModeDistributorQueueItem>, String> {
            write_distributor_queue_record(
                self.planning_authority.as_ref(),
                self.parallel_runtime.as_ref(),
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                &record,
            )?;
            let _ = record_merge_queued_session_detail(
                self.planning_authority.as_ref(),
                self.parallel_runtime.as_ref(),
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                &resolution.lease,
            );
            Ok(Some(record.display_item()))
        };
        match permit {
            Some(permit) => permit.with_active_commit(persist).unwrap_or(Ok(None)),
            None => persist(),
        }
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
        self.process_queue_with_permit(workspace_dir, None)
    }

    pub(super) fn process_queue_guarded(
        &self,
        workspace_dir: &str,
        permit: &super::ParallelModeAutomationPermit,
    ) -> Result<Vec<String>, String> {
        self.process_queue_with_permit(workspace_dir, Some(permit))
    }

    fn process_queue_with_permit(
        &self,
        workspace_dir: &str,
        permit: Option<&super::ParallelModeAutomationPermit>,
    ) -> Result<Vec<String>, String> {
        if permit.is_some_and(|permit| !permit.is_active()) {
            return Ok(vec![
                "parallel automation epoch closed before distributor queue processing".to_string(),
            ]);
        }
        let mut preflight_context =
            load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir)
                .map_err(|(_, detail)| detail.to_string())?;
        /*
        A commit-ready session can survive a crash between the durable session
        transition and queue persistence. Recover that handoff before deciding
        the queue is empty; otherwise an idle distributor can never discover
        the result that it is responsible for delivering.
        */
        self.recover_missing_commit_ready_queue_records(&preflight_context)?;
        preflight_context =
            load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir)
                .map_err(|(_, detail)| detail.to_string())?;
        let Some(mut preflight_head) = preflight_context
            .distributor_queue_records
            .iter()
            .find(|record| record.queue_state != ParallelModeQueueItemState::Done)
            .cloned()
        else {
            return Ok(Vec::new());
        };
        let blocked_head_can_recover = preflight_head.queue_state
            == ParallelModeQueueItemState::Blocked
            && (is_retryable_distributor_block(&preflight_head.integration_note)
                || record_is_cleanup_recovery_candidate(&preflight_head)
                || record_has_frozen_integration_recovery_evidence(&preflight_head));
        if preflight_head.queue_state == ParallelModeQueueItemState::Failed
            || (preflight_head.queue_state == ParallelModeQueueItemState::Blocked
                && !blocked_head_can_recover)
        {
            return Ok(vec![format!(
                "distributor queue head is blocked / agent: {} / task: {} / {}",
                preflight_head.agent_id, preflight_head.task_id, preflight_head.integration_note
            )]);
        }
        if let Err(detail) = validate_distributor_delivery_target_with_policy(
            self.github_automation.as_ref(),
            &preflight_context.repo_root,
            &preflight_head,
            &self.delivery_safety_policy,
        ) {
            let matching_lease =
                matching_lease_for_queue_record(&preflight_context, &preflight_head).cloned();
            let notice = block_distributor_queue_record(
                self.planning_authority.as_ref(),
                self.parallel_runtime.as_ref(),
                &preflight_context.repo_root,
                &preflight_context.pool_root,
                matching_lease.as_ref(),
                &mut preflight_head,
                detail,
            )?;
            return Ok(vec![notice]);
        }
        let frozen_target = preflight_head
            .delivery_target
            .as_ref()
            .ok_or_else(|| "distributor queue head has no immutable delivery target".to_string())?;
        let pool_mutation_lock =
            acquire_pool_mutation_lock(self.planning_authority.as_ref(), workspace_dir)?;
        let integration_target = fetch_distributor_integration_target(
            self.github_automation.as_ref(),
            &preflight_context.repo_root,
            frozen_target,
        )?;
        let fresh_pool_target = FreshPoolIntegrationTargetProof {
            repo_root: preflight_context.repo_root.clone(),
            push_remote: frozen_target.push_remote.clone(),
            integration_branch: frozen_target.integration_branch.clone(),
            credential_redacted_push_url: frozen_target
                .credential_redacted_push_url
                .clone()
                .ok_or_else(|| {
                    "immutable distributor target has no credential-redacted push URL".to_string()
                })?,
            commit_sha: integration_target.commit_sha.clone(),
        };
        let (_reconciled_context, _) = reconcile_pool_board_and_context_with_target_locked(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            workspace_dir,
            &fresh_pool_target,
            &pool_mutation_lock,
        )
        .map_err(|error| error.1)?;
        drop(pool_mutation_lock);
        let context = self.recover_runtime_state_with_proof(workspace_dir, &integration_target)?;
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
        let Some(claim_permit) =
            self.acquire_queue_head_claim(workspace_dir, &head.queue_item_id)?
        else {
            return Ok(vec![format!(
                "distributor queue head is already claimed by another process / agent: {} / task: {}",
                head.agent_id, head.task_id
            )]);
        };
        let delivery_started_in_window = matches!(
            head.queue_state,
            ParallelModeQueueItemState::Queued
                | ParallelModeQueueItemState::Pushing
                | ParallelModeQueueItemState::PrPending
                | ParallelModeQueueItemState::MergePending
                | ParallelModeQueueItemState::Integrating
        );

        let mut notices = process_distributor_queue_record(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            &context.repo_root,
            &context.pool_root,
            head,
            self.github_automation.as_ref(),
            permit,
            &claim_permit,
            &integration_target.commit_sha,
            &self.delivery_safety_policy.allow_autonomous_delivery,
        )?;
        if head.queue_state == ParallelModeQueueItemState::Done
            && let Some(target) = head.delivery_target.as_ref()
            && let Some(credential_redacted_push_url) = target.credential_redacted_push_url.as_ref()
        {
            /*
            The delivery path verifies the pushed integration commit before it
            marks the queue item Done. Use that exact result (or the fetched
            target for cleanup-only recovery) to advance older lease-free idle
            slots in the same tick. Without this pass, every successful queue
            item leaves earlier slots detached at a stale baseline until another
            mutating reconcile happens, so a drained pool appears blocked.
            */
            let refreshed_target = FreshPoolIntegrationTargetProof {
                repo_root: context.repo_root.clone(),
                push_remote: target.push_remote.clone(),
                integration_branch: target.integration_branch.clone(),
                credential_redacted_push_url: credential_redacted_push_url.clone(),
                commit_sha: if delivery_started_in_window {
                    head.integration_commit_sha
                        .clone()
                        .unwrap_or_else(|| integration_target.commit_sha.clone())
                } else {
                    integration_target.commit_sha.clone()
                },
            };
            if let Err(error) = reconcile_pool_board_and_context_with_target(
                self.planning_authority.as_ref(),
                self.parallel_runtime.as_ref(),
                workspace_dir,
                &refreshed_target,
            ) {
                notices.push(format!(
                    "distributor delivery completed but idle pool baseline refresh was blocked: {}",
                    error.1
                ));
            }
        }
        Ok(notices)
    }

    // snapshot 읽기는 실패를 운영 오류로 끌어올리지 않고 placeholder로 접는다.
    // supervisor 화면은 distributor 저장소가 잠시 읽히지 않아도 전체 병렬 모드 상태를 계속 렌더링한다.
    pub(super) fn inspect_snapshot(&self, workspace_dir: &str) -> ParallelModeDistributorSnapshot {
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
        let context = load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir)
            .map_err(|(_, detail)| detail.to_string())?;
        let push_remote = try_push_remote_name(&context.repo_root)?;
        let integration_branch = try_parallel_mode_integration_branch_for_repo(&context.repo_root)?;
        let push_url = self
            .github_automation
            .credential_redacted_push_url_for_remote(&context.repo_root, &push_remote)
            .map_err(|error| error.to_string())?;
        let integration_target = fetch_integration_target_proof(
            self.github_automation.as_ref(),
            &context.repo_root,
            &push_remote,
            &push_url,
            &integration_branch,
        )?;
        self.recover_runtime_state_with_proof(workspace_dir, &integration_target)
    }

    fn recover_runtime_state_with_proof(
        &self,
        workspace_dir: &str,
        integration_target: &IntegrationTargetProof,
    ) -> Result<PoolRuntimeContext, String> {
        let mut context =
            load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir)
                .map_err(|(_, detail)| detail.to_string())?;
        recover_stale_ledger_refreshing_sessions(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            workspace_dir,
            &context,
        )?;
        context = load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir)
            .map_err(|(_, detail)| detail.to_string())?;
        self.recover_missing_commit_ready_queue_records(&context)?;
        context = load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir)
            .map_err(|(_, detail)| detail.to_string())?;
        for index in 0..context.distributor_queue_records.len() {
            let mut record = context.distributor_queue_records[index].clone();
            let matching_lease = matching_lease_for_queue_record(&context, &record).cloned();
            if !matches!(
                record.queue_state,
                ParallelModeQueueItemState::Idle
                    | ParallelModeQueueItemState::Done
                    | ParallelModeQueueItemState::Failed
            ) && let Err(detail) = validate_distributor_delivery_target_with_policy(
                self.github_automation.as_ref(),
                &context.repo_root,
                &record,
                &self.delivery_safety_policy,
            ) {
                let _ = block_distributor_queue_record(
                    self.planning_authority.as_ref(),
                    self.parallel_runtime.as_ref(),
                    &context.repo_root,
                    &context.pool_root,
                    matching_lease.as_ref(),
                    &mut record,
                    detail,
                )?;
                context.distributor_queue_records[index] = record;
                continue;
            }
            if !matches!(
                record.queue_state,
                ParallelModeQueueItemState::Idle
                    | ParallelModeQueueItemState::Done
                    | ParallelModeQueueItemState::Failed
            ) && matching_lease.is_none()
                && let Some(conflicting_lease) =
                    conflicting_lease_for_queue_record(&context, &record)
            {
                let live_session_key = conflicting_lease.session_key();
                let detail = if live_session_key == record.session_key {
                    format!(
                        "lease identity no longer matches distributor session `{}`; live lease for slot `{}` is preserved",
                        record.session_key, conflicting_lease.slot_id
                    )
                } else {
                    format!(
                        "stale distributor session `{}` no longer owns slot `{}`; live session `{}` is preserved",
                        record.session_key, conflicting_lease.slot_id, live_session_key
                    )
                };
                let _ = block_distributor_queue_record(
                    self.planning_authority.as_ref(),
                    self.parallel_runtime.as_ref(),
                    &context.repo_root,
                    &context.pool_root,
                    None,
                    &mut record,
                    detail,
                )?;
                context.distributor_queue_records[index] = record;
                continue;
            }
            /*
            Recovery runs the narrow, non-destructive fixes before broader state
            classification. A clean mismatched checkout or known retryable block
            can become Queued again without inspecting PR/integration state.
            */
            recover_mismatched_slot_worktree(
                self.planning_authority.as_ref(),
                self.parallel_runtime.as_ref(),
                &context.repo_root,
                &context.pool_root,
                matching_lease.as_ref(),
                &mut record,
            )?;
            recover_retryable_blocked_queue_record(
                self.planning_authority.as_ref(),
                self.parallel_runtime.as_ref(),
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
                && queue_record_is_integrated_or_patch_equivalent(
                    &context,
                    &record,
                    integration_target,
                )
            {
                /*
                Reconcile can finish slot cleanup before distributor recovery
                sees the blocked/cleaning record. With no lease and no source
                branch left, the durable queue item should close as recovered
                instead of remaining a permanent blocked head.
                */
                recover_integrated_queue_record(
                    self.planning_authority.as_ref(),
                    self.parallel_runtime.as_ref(),
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
            ) && queue_record_is_integrated_or_patch_equivalent(
                &context,
                &record,
                integration_target,
            ) {
                /*
                Integration proof also recovers cleanup-time blocks. A queue
                item that already landed in prerelease should converge toward
                slot return instead of staying blocked at the head forever.
                */
                recover_integrated_queue_record(
                    self.planning_authority.as_ref(),
                    self.parallel_runtime.as_ref(),
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
                    self.parallel_runtime.as_ref(),
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
                && let Some(target) = record.delivery_target.as_ref()
                && let Some(credential_redacted_push_url) =
                    target.credential_redacted_push_url.as_deref()
                && let Ok(pull_request) = self
                    .github_automation
                    .inspect_pull_request_for_delivery_target(
                        &context.repo_root,
                        &target.push_remote,
                        credential_redacted_push_url,
                        pr_number,
                    )
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
                        self.parallel_runtime.as_ref(),
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
                        self.parallel_runtime.as_ref(),
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
                    self.parallel_runtime.as_ref(),
                    &context.repo_root,
                    &context.pool_root,
                    &record,
                )?;
            }

            context.distributor_queue_records[index] = record;
        }
        Ok(context)
    }

    fn recover_missing_commit_ready_queue_records(
        &self,
        context: &PoolRuntimeContext,
    ) -> Result<(), String> {
        let mut candidates = context
            .session_details
            .iter()
            .filter(|detail| detail.state_label == "commit_ready")
            .filter(|detail| {
                !context
                    .distributor_queue_records
                    .iter()
                    .any(|record| record.session_key == detail.session_key)
            })
            .filter_map(|detail| {
                context
                    .slot_leases
                    .values()
                    .find(|lease| {
                        lease.state == ParallelModeSlotLeaseState::Running
                            && lease_session_key(lease) == detail.session_key
                    })
                    .map(|lease| {
                        (
                            detail.updated_at.clone(),
                            detail.session_key.clone(),
                            lease.worktree_path.clone(),
                        )
                    })
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));

        for (_, session_key, worktree_path) in candidates {
            self.enqueue_workspace_commit_ready_result(&worktree_path)
                .map_err(|error| {
                    format!(
                        "commit-ready distributor enqueue recovery failed for session `{session_key}`: {error}"
                    )
                })?;
        }
        Ok(())
    }
}

fn recover_stale_ledger_refreshing_sessions(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
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
                    runtime,
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
                    runtime,
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
    runtime: &dyn ParallelModeRuntimePort,
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
    let mutation_lock = match acquire_pool_mutation_lock(planning_authority, repo_root) {
        Ok(mutation_lock) => mutation_lock,
        Err(error) => {
            record.integration_note =
                format!("slot checkout recovery could not acquire the pool mutation lock: {error}");
            record.recovery_note = Some(record.integration_note.clone());
            record.updated_at = current_timestamp();
            write_distributor_queue_record(
                planning_authority,
                runtime,
                repo_root,
                pool_root,
                record,
            )?;
            return Ok(());
        }
    };
    let locked_context = load_pool_runtime_context(planning_authority, repo_root)
        .map_err(|(_, detail)| detail.to_string())?;
    mutation_lock.verify_pool_root(&locked_context.pool_root)?;
    let Some(locked_lease) = locked_context.slot_leases.get(&lease.slot_id) else {
        return Ok(());
    };
    let expected_slot_path = locked_context.pool_root.join(&lease.slot_id);
    if !locked_lease.same_generation_as(lease)
        || locked_lease.worktree_path != record.worktree_path
        || Path::new(&record.worktree_path) != expected_slot_path
    {
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
    let Ok(slot_status) = inspect_slot_git_status(Path::new(&record.worktree_path)) else {
        return Ok(());
    };
    if !slot_status.is_clean_for_frozen_delivery() {
        return Ok(());
    }
    if let Err(error) = crate::git_execution_guard::ensure_host_git_execution_config_safe(
        Path::new(&record.worktree_path),
    ) {
        record.integration_note =
            format!("slot checkout recovery was blocked by Git execution configuration: {error}");
        record.recovery_note = Some(record.integration_note.clone());
        record.updated_at = current_timestamp();
        write_distributor_queue_record(planning_authority, runtime, repo_root, pool_root, record)?;
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
    if current_branch_name(Path::new(&record.worktree_path)).as_deref()
        != Some(lease.branch_name.as_str())
    {
        return Ok(());
    }

    record.queue_state = ParallelModeQueueItemState::Queued;
    record.integration_state = "queued".to_string();
    record.recovery_note =
        Some("recovered mismatched clean slot worktree checkout before retry".to_string());
    record.integration_note =
        "recovered clean slot worktree checkout and queued distributor retry".to_string();
    record.updated_at = current_timestamp();
    write_distributor_queue_record(planning_authority, runtime, repo_root, pool_root, record)?;
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
    runtime: &dyn ParallelModeRuntimePort,
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
    let Ok(slot_status) = inspect_slot_git_status(Path::new(&record.worktree_path)) else {
        return Ok(());
    };
    if slot_status.has_pending_operation {
        return Ok(());
    }
    if !slot_status.is_clean_for_frozen_delivery() {
        return Ok(());
    }

    let epoch_rearm = record
        .integration_note
        .contains("parallel automation epoch closed before distributor");
    let human_gate = is_human_review_gate(&record.integration_note);
    if !human_gate && !epoch_rearm && record.retry_attempts >= DISTRIBUTOR_RETRY_MAX_ATTEMPTS {
        return Ok(());
    }
    if !epoch_rearm && record.retry_attempts > 0 {
        if let Some(not_before) = record.retry_not_before.as_deref() {
            let Ok(not_before) = DateTime::parse_from_rfc3339(not_before) else {
                return Ok(());
            };
            if Utc::now() < not_before.with_timezone(&Utc) {
                return Ok(());
            }
        } else {
            let exponent = record.retry_attempts.saturating_sub(1).min(16);
            let delay_seconds = DISTRIBUTOR_RETRY_BASE_DELAY_SECS
                .saturating_mul(1_i64 << exponent)
                .min(DISTRIBUTOR_RETRY_MAX_DELAY_SECS);
            let retry_at = Utc::now() + TimeDelta::seconds(delay_seconds);
            record.retry_not_before = Some(retry_at.to_rfc3339());
            record.integration_note = format!(
                "{} / automatic retry {} of {} is deferred until {}",
                record.integration_note,
                record.retry_attempts + 1,
                DISTRIBUTOR_RETRY_MAX_ATTEMPTS,
                retry_at.to_rfc3339()
            );
            record.updated_at = current_timestamp();
            write_distributor_queue_record(
                planning_authority,
                runtime,
                repo_root,
                pool_root,
                record,
            )?;
            return Ok(());
        }
    }

    record.queue_state = ParallelModeQueueItemState::Queued;
    record.integration_state = "queued".to_string();
    record.recovery_note = Some("recovered retryable distributor block before retry".to_string());
    record.integration_note = "recovered retryable distributor block and queued retry".to_string();
    if !epoch_rearm {
        record.retry_attempts = record.retry_attempts.saturating_add(1);
    }
    record.retry_not_before = None;
    record.updated_at = current_timestamp();
    write_distributor_queue_record(planning_authority, runtime, repo_root, pool_root, record)?;
    Ok(())
}

// retryable block 목록은 delivery가 남기는 integration_note 문구와 맞물린다.
// 영구 복구가 필요한 상태까지 자동 재시도하지 않도록 명시적으로 알려진 임시 실패만 통과시킨다.
fn is_retryable_distributor_block(detail: &str) -> bool {
    detail.contains("pull request ensure failed")
        || detail.contains("could not be inspected")
        || detail.contains("could not cherry-pick")
        || detail.contains("integration worktree must be checked out to `")
        || detail.contains("integration worktree must be clean before cherry-pick delivery")
        || detail.contains("push capability is unavailable for distributor delivery")
        || (detail.contains("source branch `") && detail.contains("` could not be pushed to `"))
        || detail.contains("source branch was pushed but GitHub automation is unavailable")
        || detail.contains("source branch was pushed but pull request workflow is unavailable")
        || detail.contains("pull request workflow is required but unavailable")
        || detail.contains("has not received an explicit APPROVED review decision")
        || detail.contains("merge state is not explicitly CLEAN")
        || detail.contains("required checks are failing or unknown")
        || detail.contains("is still a draft")
        || detail.contains("parallel automation epoch closed before distributor")
}

fn is_human_review_gate(detail: &str) -> bool {
    detail.contains("has not received an explicit APPROVED review decision")
        || detail.contains("merge state is not explicitly CLEAN")
        || detail.contains("required checks are failing or unknown")
        || detail.contains("is still a draft")
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
        || record.integration_note.contains("PR delivery completed")
        || record
            .integration_note
            .contains("direct delivery completed without PR automation")
}

fn record_has_frozen_integration_recovery_evidence(
    record: &ParallelModeDistributorQueueRecord,
) -> bool {
    record.delivery_target.as_ref().is_some_and(|target| {
        target
            .credential_redacted_push_url
            .as_deref()
            .is_some_and(|url| !url.trim().is_empty())
    }) && !record.source_base_commit_sha.trim().is_empty()
        && !record.effective_source_commit_sha().trim().is_empty()
}

fn queue_record_is_integrated_or_patch_equivalent(
    context: &PoolRuntimeContext,
    record: &ParallelModeDistributorQueueRecord,
    integration_target: &IntegrationTargetProof,
) -> bool {
    let Some(target) = record.delivery_target.as_ref() else {
        return false;
    };
    if !integration_target.matches(target) {
        return false;
    }
    distributor_source_cherry_states(&context.repo_root, &integration_target.commit_sha, record)
        .is_ok_and(|states| states.iter().all(|(_, equivalent)| *equivalent))
}

/*
Queue recovery may mutate a live lease, so every immutable identity field must
still match the session that created the record. A branch name or worktree path
can be reused after cleanup and is never sufficient proof of ownership.
*/
fn matching_lease_for_queue_record<'a>(
    context: &'a PoolRuntimeContext,
    record: &ParallelModeDistributorQueueRecord,
) -> Option<&'a ParallelModeSlotLeaseSnapshot> {
    context
        .slot_leases
        .values()
        .find(|lease| queue_record_owns_lease(record, lease))
}

fn queue_record_owns_lease(
    record: &ParallelModeDistributorQueueRecord,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> bool {
    let target_matches = record
        .delivery_target
        .as_ref()
        .zip(lease.delivery_target.as_ref())
        .is_some_and(|(record_target, lease_target)| {
            let lease_visibility = match lease_target.repository_visibility {
                ParallelModeRepositoryVisibility::Private => GithubRepositoryVisibility::Private,
                ParallelModeRepositoryVisibility::Internal => GithubRepositoryVisibility::Internal,
                ParallelModeRepositoryVisibility::Public => GithubRepositoryVisibility::Public,
            };
            record_target.push_remote == lease_target.push_remote
                && record_target.credential_redacted_push_url
                    == lease_target.credential_redacted_push_url
                && record_target.github_repository == lease_target.github_repository
                && record_target.repository_visibility == lease_visibility
                && record_target.integration_branch == lease_target.integration_branch
                && record.source_base_commit_sha == lease_target.integration_base_commit_sha
        });

    lease_session_key(lease) == record.session_key
        && lease.slot_id == record.slot_id
        && lease.task_id == record.task_id
        && lease.agent_id == record.agent_id
        && lease.branch_name == record.branch_name
        && lease.branch_name == record.effective_source_branch()
        && lease.worktree_path == record.worktree_path
        && target_matches
}

fn conflicting_lease_for_queue_record<'a>(
    context: &'a PoolRuntimeContext,
    record: &ParallelModeDistributorQueueRecord,
) -> Option<&'a ParallelModeSlotLeaseSnapshot> {
    context.slot_leases.values().find(|lease| {
        (!record.slot_id.trim().is_empty() && lease.slot_id == record.slot_id)
            || lease.worktree_path == record.worktree_path
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
    runtime: &dyn ParallelModeRuntimePort,
    context: &PoolRuntimeContext,
    matching_lease: Option<&ParallelModeSlotLeaseSnapshot>,
    record: &mut ParallelModeDistributorQueueRecord,
) -> Result<(), String> {
    if let Some(lease) = matching_lease {
        if lease.state == ParallelModeSlotLeaseState::Running {
            let mutation_lock = acquire_pool_mutation_lock(planning_authority, &context.repo_root)?;
            mutation_lock.verify_pool_root(&context.pool_root)?;
            let projection = planning_authority
                .load_runtime_projections(&context.repo_root)
                .map_err(|error| error.to_string())?;
            if projection.slot_leases.get(&lease.slot_id) != Some(lease) {
                return Err(format!(
                    "slot `{}` lease generation changed during distributor recovery",
                    lease.slot_id
                ));
            }
            let mut cleanup_pending_lease = lease.clone();
            cleanup_pending_lease.state = ParallelModeSlotLeaseState::CleanupPending;
            transition_slot_lease(
                planning_authority,
                runtime,
                &context.repo_root,
                &context.pool_root,
                lease,
                &cleanup_pending_lease,
            )?;
            let _ = record_cleanup_pending_session_detail(
                planning_authority,
                runtime,
                &context.repo_root,
                &context.pool_root,
                &cleanup_pending_lease,
            );
        }
    } else if !branch_exists(&context.repo_root, &record.branch_name) {
        let integration_branch = record
            .delivery_target
            .as_ref()
            .map(|target| target.integration_branch.as_str())
            .unwrap_or("the frozen integration branch");
        record.queue_state = ParallelModeQueueItemState::Done;
        record.integration_note = format!(
            "recovered after restart: branch is already integrated into {} and slot cleanup completed",
            integration_branch
        );
        record.updated_at = current_timestamp();
        write_distributor_queue_record(
            planning_authority,
            runtime,
            &context.repo_root,
            &context.pool_root,
            record,
        )?;
        return Ok(());
    }

    record.queue_state = ParallelModeQueueItemState::Cleaning;
    let integration_branch = record
        .delivery_target
        .as_ref()
        .map(|target| target.integration_branch.as_str())
        .unwrap_or("the frozen integration branch");
    record.integration_note = format!(
        "recovered after restart: branch is already integrated into {} and cleanup is pending",
        integration_branch
    );
    record.updated_at = current_timestamp();
    write_distributor_queue_record(
        planning_authority,
        runtime,
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
        assert!(!is_retryable_distributor_block(
            "`prerelease` could not be pushed to `origin`: non-fast-forward"
        ));
        assert!(!is_retryable_distributor_block(
            "integration branch `prerelease` could not be pushed to `origin`: non-fast-forward"
        ));
        assert!(!is_retryable_distributor_block(
            "`feature` could not be pushed to `origin`: unsupported integration branch"
        ));
        assert!(is_retryable_distributor_block(
            "integration worktree must be checked out to `prerelease` before cherry-pick delivery"
        ));
        assert!(is_retryable_distributor_block(
            "source branch was pushed but GitHub automation is unavailable: gh auth missing"
        ));
    }
}
