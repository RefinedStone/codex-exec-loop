use std::{fs, path::Path};

use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeDispatchBlockReason,
    ParallelModePoolSlotCleanupDecision, ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
};
use crate::git_subprocess;
use chrono::{DateTime, TimeDelta, Utc};

use super::super::git_sequence::{GitCommandStep, GitCommandStepReport, run_git_sequence};
use super::super::readiness::{command_succeeds, run_command};
use super::super::{
    branch_exists, current_timestamp, record_cleaned_session_detail,
    record_failed_start_dispatch_block, record_failed_start_session_detail,
};
#[cfg(test)]
use super::acquire_pool_mutation_lock;
use super::{
    AKRA_AGENT_BRANCH_PREFIX, DEFAULT_POOL_SIZE, GitWorktreeRecord, PoolMutationLock,
    SlotGitStatus, current_branch_name, derive_default_pool_root, inspect_slot_git_status,
    load_worktree_records, orphaned_slot_lease_mirror_matches_identity_or_missing,
    remove_orphaned_slot_lease_mirror_if_matches, remove_slot_lease, slot_id,
    slot_lease_mirror_matches_or_missing, worktree_paths_match,
};

const STALE_LEASED_SLOT_RELEASE_AFTER_SECS: i64 = 120;

pub(in crate::application::service::parallel_mode) enum PoolSlotCleanupLeaseAuthority<'a> {
    CleanupPending(&'a ParallelModeSlotLeaseSnapshot),
    NoLease,
    StaleStartup {
        lease: &'a ParallelModeSlotLeaseSnapshot,
        dispatch_blocked_at: &'a str,
    },
}

impl PoolSlotCleanupLeaseAuthority<'_> {
    fn expected_lease(&self) -> Option<&ParallelModeSlotLeaseSnapshot> {
        match self {
            Self::CleanupPending(lease) | Self::StaleStartup { lease, .. } => Some(lease),
            Self::NoLease => None,
        }
    }

    fn allows_projection(
        &self,
        projection: &crate::application::port::outbound::planning_authority_port::PlanningAuthorityRuntimeProjectionSnapshot,
        slot_id: &str,
    ) -> bool {
        let expected = self.expected_lease();
        if projection.slot_leases.get(slot_id) != expected {
            return false;
        }
        match self {
            Self::CleanupPending(lease) => {
                lease.state == ParallelModeSlotLeaseState::CleanupPending
            }
            Self::NoLease => true,
            Self::StaleStartup {
                lease,
                dispatch_blocked_at,
            } => {
                lease.state == ParallelModeSlotLeaseState::Leased
                    && projection.task_dispatch_blocks.iter().any(|block| {
                        block.task_id == lease.task_id
                            && block.blocked_at == *dispatch_blocked_at
                            && block.reason
                                == ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges
                    })
            }
        }
    }
}

pub(in crate::application::service::parallel_mode) struct PoolSlotCleanupIdentity<'a> {
    repo_root: &'a str,
    canonical_repo_root: &'a Path,
    pool_root: &'a Path,
    slot_id: &'a str,
    slot_path: &'a Path,
    branch_name: &'a str,
}

impl<'a> PoolSlotCleanupIdentity<'a> {
    pub(in crate::application::service::parallel_mode) fn new(
        repo_root: &'a str,
        canonical_repo_root: &'a Path,
        pool_root: &'a Path,
        slot_id: &'a str,
        slot_path: &'a Path,
        branch_name: &'a str,
    ) -> Self {
        Self {
            repo_root,
            canonical_repo_root,
            pool_root,
            slot_id,
            slot_path,
            branch_name,
        }
    }

    pub(in crate::application::service::parallel_mode) fn validate(&self) -> Result<(), String> {
        self.validate_worktree_registration(true)
    }

    fn validate_detached(&self) -> Result<(), String> {
        self.validate_worktree_registration(false)
    }

    fn validate_worktree_registration(
        &self,
        branch_must_be_checked_out: bool,
    ) -> Result<(), String> {
        if !(1..=DEFAULT_POOL_SIZE).any(|number| slot_id(number) == self.slot_id) {
            return Err(format!("invalid pool slot id `{}`", self.slot_id));
        }
        let expected_prefix = format!("{AKRA_AGENT_BRANCH_PREFIX}/{}/", self.slot_id);
        if !self.branch_name.starts_with(&expected_prefix) {
            return Err(format!(
                "branch `{}` is not owned by pool slot `{}`",
                self.branch_name, self.slot_id
            ));
        }

        let expected_pool_root = derive_default_pool_root(self.canonical_repo_root);
        if self.pool_root != expected_pool_root {
            return Err(format!(
                "pool root `{}` is not the generated root `{}` for this repository",
                self.pool_root.display(),
                expected_pool_root.display()
            ));
        }
        let expected_slot_path = expected_pool_root.join(self.slot_id);
        if self.slot_path != expected_slot_path {
            return Err(format!(
                "slot `{}` path `{}` is not the generated pool path `{}`",
                self.slot_id,
                self.slot_path.display(),
                expected_slot_path.display()
            ));
        }
        let managed_sibling_root = expected_pool_root
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| "generated pool root has no managed sibling root".to_string())?;
        let managed_hash_root = expected_pool_root
            .parent()
            .ok_or_else(|| "generated pool root has no managed hash root".to_string())?;
        for managed_path in [
            managed_sibling_root,
            managed_hash_root,
            expected_pool_root.as_path(),
            expected_slot_path.as_path(),
        ] {
            let metadata = fs::symlink_metadata(managed_path).map_err(|error| {
                format!(
                    "managed pool path `{}` could not be inspected: {error}",
                    managed_path.display()
                )
            })?;
            if metadata_is_link_or_reparse(&metadata) {
                return Err(format!(
                    "managed pool path `{}` must not be a symlink or reparse point",
                    managed_path.display()
                ));
            }
        }
        let canonical_expected = fs::canonicalize(&expected_slot_path).map_err(|error| {
            format!(
                "generated pool path `{}` could not be canonicalized: {error}",
                expected_slot_path.display()
            )
        })?;
        let canonical_slot = fs::canonicalize(self.slot_path).map_err(|error| {
            format!(
                "slot cleanup path `{}` could not be canonicalized: {error}",
                self.slot_path.display()
            )
        })?;
        if canonical_slot != canonical_expected {
            return Err(format!(
                "slot `{}` cleanup path does not resolve to its generated pool path",
                self.slot_id
            ));
        }

        let worktree_records = load_worktree_records(self.repo_root)
            .ok_or_else(|| "git worktree inventory could not be loaded for cleanup".to_string())?;
        let registered = worktree_records.iter().any(|record| {
            fs::canonicalize(&record.path).ok().as_ref() == Some(&canonical_slot)
                && if branch_must_be_checked_out {
                    record.branch_name.as_deref() == Some(self.branch_name) && !record.detached
                } else {
                    record.detached
                }
        });
        if !registered {
            return Err(format!(
                "slot `{}` is not a registered agent worktree on branch `{}`",
                self.slot_id, self.branch_name
            ));
        }
        Ok(())
    }
}

#[cfg(windows)]
fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_type().is_symlink()
        || std::os::windows::fs::MetadataExt::file_attributes(metadata)
            & FILE_ATTRIBUTE_REPARSE_POINT
            != 0
}

#[cfg(not(windows))]
fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

pub(super) struct ReconciledPoolCleanupContext<'a> {
    repo_root: &'a str,
    canonical_repo_root: &'a Path,
    pool_root: &'a Path,
    worktree_records: &'a [GitWorktreeRecord],
    slot_leases: &'a std::collections::BTreeMap<String, ParallelModeSlotLeaseSnapshot>,
    integration_target_oid: &'a str,
}

impl<'a> ReconciledPoolCleanupContext<'a> {
    pub(super) fn new(
        repo_root: &'a str,
        canonical_repo_root: &'a Path,
        pool_root: &'a Path,
        worktree_records: &'a [GitWorktreeRecord],
        slot_leases: &'a std::collections::BTreeMap<String, ParallelModeSlotLeaseSnapshot>,
        integration_target_oid: &'a str,
    ) -> Self {
        Self {
            repo_root,
            canonical_repo_root,
            pool_root,
            worktree_records,
            slot_leases,
            integration_target_oid,
        }
    }
}
/*
reusable slot cleanup은 reconcile 과정에서 "이제 pool baseline으로 되돌려도 되는" slot을
찾아 자동으로 정리하는 후처리 경로다. 대상은 slot 번호별 worktree inventory를 기준으로
찾고, agent branch prefix, lease state, worktree clean 여부, branch가 baseline에
통합되었다는 증거를 모두 만족해야 한다.

이 함수가 보수적인 이유는 slot worktree가 사용자의 미완성 변경이나 아직 통합되지 않은
agent branch를 품을 수 있기 때문이다. lease가 Leased/Running이면 건드리지 않고, 통합
증거가 없으면 cleanup하지 않으며, 실제 cleanup도 `cleanup_slot`의 단계별 성공 여부를 보고
count를 올린다.
*/
pub(super) fn cleanup_reusable_slots(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    context: &ReconciledPoolCleanupContext<'_>,
    mutation_lock: &PoolMutationLock,
) -> usize {
    let mut cleaned_slots = 0;

    for slot_number in 1..=DEFAULT_POOL_SIZE {
        let slot_id = slot_id(slot_number);
        let slot_path = context.pool_root.join(&slot_id);
        let Some(worktree_record) = context
            .worktree_records
            .iter()
            .find(|record| worktree_paths_match(&record.path, &slot_path))
        else {
            // git worktree inventory에 없으면 cleanup보다 provisioning/inspection 경로가 먼저 다룬다.
            continue;
        };
        let Some(branch_name) = worktree_record.branch_name.as_deref() else {
            // detached slot은 agent branch가 아니므로 reusable baseline reset 경로의 책임이다.
            continue;
        };
        let expected_agent_prefix = format!("{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/");
        if !branch_name.starts_with(&expected_agent_prefix) {
            // 다른 slot의 agent branch나 사용자 branch를 현재 slot cleanup이 지우지 않게 prefix를 엄격히 맞춘다.
            continue;
        }
        let slot_lease = context.slot_leases.get(&slot_id);
        let lease_state = slot_lease.map(|lease| lease.state);
        // lease가 없을 때만 worktree cleanliness가 cleanup 근거가 된다. lease가 있으면 lease state가 우선이다.
        let worktree_clean = lease_state.is_none()
            && inspect_slot_git_status(&slot_path).is_ok_and(SlotGitStatus::is_clean_baseline);
        // branch integration은 active lease가 아닌 경우에만 확인하며, cleanup pending은 명시적 승인 신호다.
        let branch_integrated = !matches!(
            lease_state,
            Some(ParallelModeSlotLeaseState::Leased | ParallelModeSlotLeaseState::Running)
        ) && (matches!(
            lease_state,
            Some(ParallelModeSlotLeaseState::CleanupPending)
        ) || worktree_clean)
            && branch_patch_is_integrated(
                context.repo_root,
                branch_name,
                context.integration_target_oid,
            );
        // domain decision object가 lease/git/integration 조합의 최종 cleanup 가능 여부를 단일 규칙으로 판정한다.
        let cleanup_ready = ParallelModePoolSlotCleanupDecision::new(
            lease_state,
            worktree_clean,
            branch_integrated,
        )
        .is_cleanup_ready();
        if !cleanup_ready {
            continue;
        }
        let authority = slot_lease.map_or(
            PoolSlotCleanupLeaseAuthority::NoLease,
            PoolSlotCleanupLeaseAuthority::CleanupPending,
        );
        if cleanup_slot_to_ref_locked(
            planning_authority,
            runtime,
            &PoolSlotCleanupIdentity::new(
                context.repo_root,
                context.canonical_repo_root,
                context.pool_root,
                &slot_id,
                &slot_path,
                branch_name,
            ),
            context.integration_target_oid,
            authority,
            mutation_lock,
        ) {
            cleaned_slots += 1;
        }
    }

    cleaned_slots
}

pub(super) fn cleanup_stale_leased_startup_slots(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    context: &ReconciledPoolCleanupContext<'_>,
    session_details: &[ParallelModeAgentSessionDetailSnapshot],
    mutation_lock: &PoolMutationLock,
) -> usize {
    let mut cleaned_slots = 0;

    for lease in context.slot_leases.values() {
        if !stale_leased_startup_slot_can_be_released(lease, session_details) {
            continue;
        }
        let slot_path = context.pool_root.join(&lease.slot_id);
        let Some(worktree_record) = context
            .worktree_records
            .iter()
            .find(|record| worktree_paths_match(&record.path, &slot_path))
        else {
            continue;
        };
        if worktree_record.branch_name.as_deref() != Some(lease.branch_name.as_str()) {
            continue;
        }
        let Ok(slot_status) = inspect_slot_git_status(&slot_path) else {
            continue;
        };
        if !slot_status.is_clean_baseline() {
            continue;
        }
        let failed_at = current_timestamp();
        if record_failed_start_dispatch_block(
            planning_authority,
            context.repo_root,
            lease,
            &failed_at,
        )
        .is_err()
        {
            continue;
        }
        if cleanup_slot_to_ref_locked(
            planning_authority,
            runtime,
            &PoolSlotCleanupIdentity::new(
                context.repo_root,
                context.canonical_repo_root,
                context.pool_root,
                &lease.slot_id,
                &slot_path,
                &lease.branch_name,
            ),
            context.integration_target_oid,
            PoolSlotCleanupLeaseAuthority::StaleStartup {
                lease,
                dispatch_blocked_at: &failed_at,
            },
            mutation_lock,
        ) {
            let _ = record_failed_start_session_detail(
                planning_authority,
                runtime,
                context.repo_root,
                context.pool_root,
                lease,
                &failed_at,
            );
            cleaned_slots += 1;
        }
    }

    cleaned_slots
}

/*
clean baseline split-brain cleanup handles the state where the source-of-truth
lease still says Leased/Running/CleanupPending, but git has already returned
the slot worktree to the pool baseline. This can happen if cleanup deletes or
detaches the branch and then fails before removing the authority lease, or if a
late worker event observes a recycled worktree. A clean baseline with a missing
active agent branch has no remaining worktree result to preserve, and a
CleanupPending branch that is already integrated is safe to close. Other active
branch drift is intentionally left blocked for operator recovery.
*/
pub(super) fn cleanup_clean_baseline_split_brain_leases(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    context: &ReconciledPoolCleanupContext<'_>,
    mutation_lock: &PoolMutationLock,
) -> usize {
    let mut cleaned_slots = 0;

    for lease in context.slot_leases.values() {
        if lease.state != ParallelModeSlotLeaseState::CleanupPending {
            continue;
        }
        let slot_path = context.pool_root.join(&lease.slot_id);
        let Some(worktree_record) = context
            .worktree_records
            .iter()
            .find(|record| worktree_paths_match(&record.path, &slot_path))
        else {
            continue;
        };
        if !worktree_is_clean_reusable_baseline(
            worktree_record,
            context.integration_target_oid,
            &slot_path,
        ) {
            continue;
        }
        let branch_still_exists = branch_exists(context.repo_root, &lease.branch_name);
        if branch_still_exists {
            let identity = PoolSlotCleanupIdentity::new(
                context.repo_root,
                context.canonical_repo_root,
                context.pool_root,
                &lease.slot_id,
                &slot_path,
                &lease.branch_name,
            );
            let Some(source_oid) = resolve_commit_oid(context.repo_root, &lease.branch_name) else {
                continue;
            };
            if identity.validate_detached().is_err()
                || !branch_patch_is_integrated(
                    context.repo_root,
                    &source_oid,
                    context.integration_target_oid,
                )
                || !projection_still_allows_cleanup(
                    planning_authority,
                    &identity,
                    &PoolSlotCleanupLeaseAuthority::CleanupPending(lease),
                )
                || !delete_cleaned_slot_branch_if_unchanged(
                    context.repo_root,
                    &lease.branch_name,
                    &source_oid,
                )
                || resolve_workspace_commit_oid(&slot_path).as_deref()
                    != Some(context.integration_target_oid)
                || !inspect_slot_git_status(&slot_path).is_ok_and(SlotGitStatus::is_clean_baseline)
            {
                continue;
            }
        }
        if mutation_lock.verify_pool_root(context.pool_root).is_err()
            || !remove_slot_lease(
                planning_authority,
                runtime,
                context.repo_root,
                context.pool_root,
                lease,
            )
        {
            continue;
        }
        let _ = record_cleaned_session_detail(
            planning_authority,
            runtime,
            context.repo_root,
            context.pool_root,
            lease,
        );
        cleaned_slots += 1;
    }

    cleaned_slots
}

fn worktree_is_clean_reusable_baseline(
    worktree_record: &GitWorktreeRecord,
    baseline_head: &str,
    slot_path: &Path,
) -> bool {
    inspect_slot_git_status(slot_path).is_ok_and(SlotGitStatus::is_clean_baseline)
        && worktree_record.head_sha == baseline_head
}

fn stale_leased_startup_slot_can_be_released(
    lease: &ParallelModeSlotLeaseSnapshot,
    session_details: &[ParallelModeAgentSessionDetailSnapshot],
) -> bool {
    if lease.state != ParallelModeSlotLeaseState::Leased || !leased_at_is_stale(&lease.leased_at) {
        return false;
    }

    let Some(detail) = session_details
        .iter()
        .find(|detail| detail.session_key == lease.session_key())
    else {
        return false;
    };

    detail.thread_id.is_none()
        && detail.state_label == "assigned"
        && detail.completion_state_label == "in_progress"
}

fn leased_at_is_stale(leased_at: &str) -> bool {
    let Ok(timestamp) = DateTime::parse_from_rfc3339(leased_at) else {
        return false;
    };
    Utc::now().signed_duration_since(timestamp.with_timezone(&Utc))
        >= TimeDelta::seconds(STALE_LEASED_SLOT_RELEASE_AFTER_SECS)
}

/*
cleanup readiness의 핵심 git 질문은 "agent branch의 변경이 pool baseline에 이미
포함되었는가"다. `merge-base --is-ancestor`는 branch tip이 base branch의 조상인지
확인하므로, true이면 branch를 지워도 baseline이 그 변경을 잃지 않는다는 뜻이다.
*/
pub(in crate::application::service::parallel_mode) fn branch_is_integrated_into(
    repo_root: &str,
    branch_name: &str,
    base_branch: &str,
) -> bool {
    command_succeeds(
        "git",
        [
            "-C",
            repo_root,
            "merge-base",
            "--is-ancestor",
            branch_name,
            base_branch,
        ],
    )
}

pub(in crate::application::service::parallel_mode) fn cleanup_slot_to_ref_locked(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    identity: &PoolSlotCleanupIdentity<'_>,
    baseline_ref: &str,
    authority: PoolSlotCleanupLeaseAuthority<'_>,
    mutation_lock: &PoolMutationLock,
) -> bool {
    cleanup_slot_to_ref_with_hooks_locked(
        planning_authority,
        runtime,
        identity,
        baseline_ref,
        authority,
        mutation_lock,
        (|| {}, || {}),
    )
}

fn cleanup_slot_to_ref_with_hooks_locked<BeforeDetach, AfterDetach>(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    identity: &PoolSlotCleanupIdentity<'_>,
    baseline_ref: &str,
    authority: PoolSlotCleanupLeaseAuthority<'_>,
    mutation_lock: &PoolMutationLock,
    hooks: (BeforeDetach, AfterDetach),
) -> bool
where
    BeforeDetach: FnOnce(),
    AfterDetach: FnOnce(),
{
    let (before_detach, after_detach) = hooks;
    if mutation_lock.verify_pool_root(identity.pool_root).is_err() || identity.validate().is_err() {
        return false;
    }
    if crate::git_execution_guard::ensure_host_git_execution_config_safe(identity.slot_path)
        .is_err()
    {
        return false;
    }
    if !mirror_still_allows_cleanup(runtime, identity, &authority) {
        return false;
    }
    let Some(source_oid) = resolve_commit_oid(identity.repo_root, identity.branch_name) else {
        return false;
    };
    let Some(baseline_oid) = resolve_commit_oid(identity.repo_root, baseline_ref) else {
        return false;
    };
    let Ok(initial_status) = inspect_slot_git_status(identity.slot_path) else {
        return false;
    };
    if !initial_status.is_clean_for_frozen_delivery()
        || current_branch_name(identity.slot_path).as_deref() != Some(identity.branch_name)
        || resolve_workspace_commit_oid(identity.slot_path).as_deref() != Some(source_oid.as_str())
        || !branch_patch_is_integrated(identity.repo_root, &source_oid, &baseline_oid)
    {
        return false;
    }
    if !projection_still_allows_cleanup(planning_authority, identity, &authority) {
        return false;
    }

    let ignored_purge_requires_lease = initial_status.has_ignored_output();
    if ignored_purge_requires_lease
        && !projection_has_matching_delivery_lease(planning_authority, identity, &authority)
    {
        return false;
    }
    if !clean_ignored_worker_output(identity.slot_path) {
        return false;
    }
    if !inspect_slot_git_status(identity.slot_path).is_ok_and(SlotGitStatus::is_clean_baseline)
        || current_branch_name(identity.slot_path).as_deref() != Some(identity.branch_name)
        || resolve_workspace_commit_oid(identity.slot_path).as_deref() != Some(source_oid.as_str())
        || resolve_commit_oid(identity.repo_root, identity.branch_name).as_deref()
            != Some(source_oid.as_str())
        || !projection_still_allows_cleanup(planning_authority, identity, &authority)
        || !mirror_still_allows_cleanup(runtime, identity, &authority)
        || (ignored_purge_requires_lease
            && !projection_has_matching_delivery_lease(planning_authority, identity, &authority))
    {
        return false;
    }

    before_detach();
    if !inspect_slot_git_status(identity.slot_path).is_ok_and(SlotGitStatus::is_clean_baseline)
        || current_branch_name(identity.slot_path).as_deref() != Some(identity.branch_name)
        || resolve_workspace_commit_oid(identity.slot_path).as_deref() != Some(source_oid.as_str())
        || resolve_commit_oid(identity.repo_root, identity.branch_name).as_deref()
            != Some(source_oid.as_str())
        || !projection_still_allows_cleanup(planning_authority, identity, &authority)
        || !mirror_still_allows_cleanup(runtime, identity, &authority)
        || (ignored_purge_requires_lease
            && !projection_has_matching_delivery_lease(planning_authority, identity, &authority))
    {
        return false;
    }

    // The source tree is now clean and still CAS-bound. A concurrent late writer after
    // the ignored-output purge makes the following checks fail and remains inspectable.
    if mutation_lock.verify_pool_root(identity.pool_root).is_err()
        || crate::git_execution_guard::ensure_host_git_execution_config_safe(identity.slot_path)
            .is_err()
    {
        return false;
    }
    let slot_path = identity.slot_path.display().to_string();
    if !command_succeeds(
        "git",
        [
            "-C",
            slot_path.as_str(),
            "checkout",
            "--detach",
            baseline_oid.as_str(),
        ],
    ) {
        return false;
    }
    after_detach();
    if current_branch_name(identity.slot_path).as_deref() != Some("HEAD")
        || resolve_workspace_commit_oid(identity.slot_path).as_deref()
            != Some(baseline_oid.as_str())
        || !inspect_slot_git_status(identity.slot_path).is_ok_and(SlotGitStatus::is_clean_baseline)
        || resolve_commit_oid(identity.repo_root, identity.branch_name).as_deref()
            != Some(source_oid.as_str())
        || !projection_still_allows_cleanup(planning_authority, identity, &authority)
        || !mirror_still_allows_cleanup(runtime, identity, &authority)
    {
        return false;
    }
    if !delete_cleaned_slot_branch_if_unchanged(
        identity.repo_root,
        identity.branch_name,
        &source_oid,
    ) {
        return false;
    }
    if current_branch_name(identity.slot_path).as_deref() != Some("HEAD")
        || resolve_workspace_commit_oid(identity.slot_path).as_deref()
            != Some(baseline_oid.as_str())
        || !inspect_slot_git_status(identity.slot_path).is_ok_and(SlotGitStatus::is_clean_baseline)
    {
        return false;
    }
    if let Some(expected_lease) = authority.expected_lease() {
        if !remove_slot_lease(
            planning_authority,
            runtime,
            identity.repo_root,
            identity.pool_root,
            expected_lease,
        ) {
            return false;
        }
    } else {
        if !projection_still_allows_cleanup(planning_authority, identity, &authority)
            || !remove_orphaned_slot_lease_mirror_if_matches(
                runtime,
                identity.pool_root,
                identity.slot_id,
                identity.branch_name,
                identity.slot_path,
            )
        {
            return false;
        }
    }

    // 마지막 git status 재검증은 metadata 제거 성공과 실제 worktree 재사용 가능 상태를 함께 확인한다.
    inspect_slot_git_status(identity.slot_path).is_ok_and(SlotGitStatus::is_clean_baseline)
}

#[cfg(test)]
pub(in crate::application::service::parallel_mode) fn cleanup_slot_to_ref_with_hooks<
    BeforeDetach,
    AfterDetach,
>(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    identity: &PoolSlotCleanupIdentity<'_>,
    baseline_ref: &str,
    before_detach: BeforeDetach,
    after_detach: AfterDetach,
) -> bool
where
    BeforeDetach: FnOnce(),
    AfterDetach: FnOnce(),
{
    let Ok(mutation_lock) = acquire_pool_mutation_lock(planning_authority, identity.repo_root)
    else {
        return false;
    };
    let Ok(projection) = planning_authority.load_runtime_projections(identity.repo_root) else {
        return false;
    };
    let authority = match projection.slot_leases.get(identity.slot_id) {
        Some(lease) if lease.state == ParallelModeSlotLeaseState::CleanupPending => {
            PoolSlotCleanupLeaseAuthority::CleanupPending(lease)
        }
        None => PoolSlotCleanupLeaseAuthority::NoLease,
        Some(_) => return false,
    };
    cleanup_slot_to_ref_with_hooks_locked(
        planning_authority,
        runtime,
        identity,
        baseline_ref,
        authority,
        &mutation_lock,
        (before_detach, after_detach),
    )
}

fn resolve_commit_oid(repo_root: &str, reference: &str) -> Option<String> {
    run_command(
        "git",
        [
            "-C",
            repo_root,
            "rev-parse",
            &format!("{reference}^{{commit}}"),
        ],
        None,
    )
}

fn resolve_workspace_commit_oid(slot_path: &Path) -> Option<String> {
    let slot_path = slot_path.display().to_string();
    resolve_commit_oid(&slot_path, "HEAD")
}

fn projection_still_allows_cleanup(
    planning_authority: &dyn PlanningAuthorityPort,
    identity: &PoolSlotCleanupIdentity<'_>,
    authority: &PoolSlotCleanupLeaseAuthority<'_>,
) -> bool {
    let Ok(projection) = planning_authority.load_runtime_projections(identity.repo_root) else {
        return false;
    };
    authority.allows_projection(&projection, identity.slot_id)
        && authority.expected_lease().is_none_or(|lease| {
            lease.slot_id == identity.slot_id
                && lease.branch_name == identity.branch_name
                && lease.worktree_path == identity.slot_path.display().to_string()
        })
}

fn mirror_still_allows_cleanup(
    runtime: &dyn ParallelModeRuntimePort,
    identity: &PoolSlotCleanupIdentity<'_>,
    authority: &PoolSlotCleanupLeaseAuthority<'_>,
) -> bool {
    match authority.expected_lease() {
        Some(expected) => {
            slot_lease_mirror_matches_or_missing(runtime, identity.pool_root, expected)
        }
        None => orphaned_slot_lease_mirror_matches_identity_or_missing(
            runtime,
            identity.pool_root,
            identity.slot_id,
            identity.branch_name,
            identity.slot_path,
        ),
    }
}

fn projection_has_matching_delivery_lease(
    planning_authority: &dyn PlanningAuthorityPort,
    identity: &PoolSlotCleanupIdentity<'_>,
    authority: &PoolSlotCleanupLeaseAuthority<'_>,
) -> bool {
    let Ok(projection) = planning_authority.load_runtime_projections(identity.repo_root) else {
        return false;
    };
    authority.allows_projection(&projection, identity.slot_id)
        && authority.expected_lease().is_some_and(|lease| {
            lease.slot_id == identity.slot_id
                && lease.branch_name == identity.branch_name
                && lease.worktree_path == identity.slot_path.display().to_string()
                && lease.delivery_target.is_some()
        })
}

fn clean_ignored_worker_output(slot_path: &Path) -> bool {
    let mut command = git_subprocess::command(std::iter::empty::<&str>());
    command
        .arg("-C")
        .arg(slot_path)
        .args(["clean", "-fdX", "--"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    crate::subprocess::command_output(&mut command, "git clean ignored parallel worker output")
        .is_ok_and(|output| output.status.success())
}

pub(in crate::application::service::parallel_mode) fn branch_patch_is_integrated(
    repo_root: &str,
    branch_name: &str,
    baseline_ref: &str,
) -> bool {
    // Exact ancestry is the strongest proof and remains valid for a branch whose
    // history legitimately contains merge commits. Patch-equivalence is only a
    // fallback for rebased/squashed delivery.
    if branch_is_integrated_into(repo_root, branch_name, baseline_ref) {
        return true;
    }

    // `git cherry` deliberately omits merge commits. A merge can carry conflict
    // resolution or other tree changes that are absent from both parents, so an
    // all-`-` cherry result cannot prove such a range is preserved. Treat command
    // failure and every non-empty merge result as unsafe.
    let range = format!("{baseline_ref}..{branch_name}");
    let mut merge_command = git_subprocess::command([
        "-C",
        repo_root,
        "rev-list",
        "--min-parents=2",
        "--max-count=1",
        range.as_str(),
    ]);
    let Ok(merge_output) = crate::subprocess::command_output(
        &mut merge_command,
        "git rev-list --min-parents=2 --max-count=1 <frozen-integration-ref>..<source-branch>",
    ) else {
        return false;
    };
    if !merge_output.status.success() || !merge_output.stdout.is_empty() {
        return false;
    }

    let mut command =
        git_subprocess::command(["-C", repo_root, "cherry", baseline_ref, branch_name]);
    let Ok(output) = crate::subprocess::command_output(
        &mut command,
        "git cherry <frozen-integration-ref> <source-branch>",
    ) else {
        return false;
    };
    output.status.success()
        && String::from_utf8_lossy(&output.stdout)
            .lines()
            .all(|line| line.trim_start().starts_with('-'))
}

pub(in crate::application::service::parallel_mode) fn delete_cleaned_slot_branch_if_unchanged(
    repo_root: &str,
    branch_name: &str,
    expected_source_oid: &str,
) -> bool {
    let branch_ref = format!("refs/heads/{branch_name}");
    command_succeeds(
        "git",
        [
            "-C",
            repo_root,
            "update-ref",
            "-d",
            branch_ref.as_str(),
            expected_source_oid,
        ],
    )
}

/*
pool slot을 baseline으로 되돌리는 git sequence다. late write를 덮어쓰는 reset/clean 없이
baseline을 detached checkout하고, 이어지는 검증에서 exact OID와 clean 상태를 확인한다.
branch를 직접 checkout하지 않고 detached baseline으로 두는 이유는 idle slot이 특정 작업
branch를 소유하지 않는 중립 상태여야 다음 lease가 새 agent branch를 안전하게 만들 수 있기
때문이다.
*/
pub(in crate::application::service::parallel_mode) fn reset_slot_worktree_to_ref(
    slot_path: &Path,
    baseline_ref: &str,
) -> super::super::git_sequence::GitCommandSequenceReport {
    // git sequence API는 argv 조각을 문자열로 받으므로 Path 변환은 sequence 조립 직전에만 수행한다.
    let slot_path_string = slot_path.display().to_string();
    if let Err(error) = crate::git_execution_guard::ensure_host_git_execution_config_safe(slot_path)
    {
        return super::super::git_sequence::GitCommandSequenceReport {
            label: "reset slot worktree to pool baseline".to_string(),
            steps: vec![GitCommandStepReport {
                label: "audit repository Git execution configuration".to_string(),
                args: vec!["git config --name-only".to_string()],
                exit_code: None,
                stdout: String::new(),
                stderr: error.to_string(),
            }],
        };
    }
    let baseline_oid = resolve_commit_oid(&slot_path_string, baseline_ref);
    let mut report = run_git_sequence(
        "reset slot worktree to pool baseline",
        vec![GitCommandStep::new(
            "checkout pool baseline detached without overwriting late writes",
            [
                "-C",
                slot_path_string.as_str(),
                "checkout",
                "--detach",
                baseline_ref,
            ],
        )],
    );
    let verified = baseline_oid.as_deref().is_some_and(|baseline_oid| {
        report.succeeded()
            && current_branch_name(slot_path).as_deref() == Some("HEAD")
            && resolve_workspace_commit_oid(slot_path).as_deref() == Some(baseline_oid)
            && inspect_slot_git_status(slot_path).is_ok_and(SlotGitStatus::is_clean_baseline)
    });
    if !verified && report.succeeded() {
        report.steps.push(GitCommandStepReport {
            label: "verify detached pool baseline without late writes".to_string(),
            args: vec!["status/head verification".to_string()],
            exit_code: Some(1),
            stdout: String::new(),
            stderr: "slot did not remain clean at the exact baseline OID".to_string(),
        });
    }
    report
}
