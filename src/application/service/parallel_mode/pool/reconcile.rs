use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::parallel_mode::ParallelModeSlotLeaseSnapshot;

use super::super::git_sequence::{GitCommandStep, run_git_sequence};
use super::super::{DEFAULT_POOL_SIZE, branch_is_integrated_into};
use super::{
    GitWorktreeRecord, NormalizationRecoveryRequest, PoolMutationLock, ensure_directory_exists,
    ensure_normalization_recovery_authority_is_empty,
    has_normalization_replacement_artifact_for_slot, has_target_equivalent_lf_normalization_drift,
    inspect_slot_git_status, normalization_replacement_artifacts_for_slot,
    quarantine_normalization_drift_and_replace_slot, reset_slot_worktree_to_ref, slot_id,
    worktree_paths_match,
};

pub(super) struct ReusableDetachedBaselineResetContext<'a> {
    pub(super) repo_root: &'a str,
    pub(super) pool_root: &'a Path,
    pub(super) worktree_records: &'a [GitWorktreeRecord],
    pub(super) slot_leases: &'a BTreeMap<String, ParallelModeSlotLeaseSnapshot>,
    pub(super) invalid_slot_leases: &'a BTreeSet<String>,
    pub(super) normalization_recovery_is_unowned: bool,
    pub(super) normalization_recovery_artifacts: &'a [PathBuf],
    pub(super) baseline_ref: &'a str,
}

#[derive(Default)]
pub(super) struct ReusableDetachedBaselineResetReport {
    pub(super) reset_slots: usize,
    pub(super) normalization_quarantines: Vec<PathBuf>,
}

/*
reconcile은 pool baseline branch가 configured push remote에 이미 존재한다고 가정하고 slot
worktree를 만든다. 매 실행에서 원격 branch를 명시적으로 fetch한 뒤 local branch와 같은 commit인지
검증한다. Akra는 원격 branch를 현재 HEAD로 seed하거나 drift한 local branch를 덮어쓰지 않는다.

반환값의 bool은 branch를 새로 만들었는지를 나타낸다. 상위 reconcile summary는 이 값을
사용해 사용자가 방금 어떤 pool 구조 변화가 일어났는지 알 수 있게 한다.
*/
/*
missing slot provisioning은 pool size만큼 정해진 slot path를 확인하고, 아직 git worktree
inventory에도 없고 파일시스템에도 없는 slot만 새로 만든다. 이미 디렉터리가 있는데 git
worktree가 아니라면 안전하게 덮어쓸 수 없으므로 provisioning하지 않고, 나중에 slot
inspection에서 Blocked로 보여 준다.

새 slot은 `--detach POOL_BASELINE_BRANCH`로 만들어진다. idle slot은 특정 branch checkout이
아니라 baseline commit에 매달린 중립 worktree여야 lease 획득 시 새 agent branch로 전환하기
쉽기 때문이다.
*/
pub(super) fn provision_missing_slots(
    repo_root: &str,
    canonical_repo_root: &Path,
    pool_root: &Path,
    worktree_records: &[GitWorktreeRecord],
    slot_leases: &BTreeMap<String, ParallelModeSlotLeaseSnapshot>,
    baseline_ref: &str,
    mutation_lock: &PoolMutationLock,
) -> Result<usize, String> {
    mutation_lock.verify_pool_root(pool_root)?;
    crate::git_execution_guard::ensure_host_git_execution_config_safe(Path::new(repo_root))
        .map_err(|error| format!("pool provisioning blocked: {error:#}"))?;
    let canonical_repo_root = std::fs::canonicalize(canonical_repo_root).map_err(|error| {
        format!("canonical repository root could not be pinned before slot provisioning: {error}")
    })?;
    let git_source_root = git_command_directory(&canonical_repo_root)?;
    crate::git_execution_guard::ensure_host_git_execution_config_safe(Path::new(&git_source_root))
        .map_err(|error| format!("pool provisioning blocked: {error:#}"))?;
    /*
    git worktree inventory에는 없지만 slot path가 남아 있으면 그 경로의 소유권을 증명할 수 없다.
    lease 유무와 관계없이 보존하고 slot inspection이 split-brain 상태를 드러내게 둔다. 이전 실패의
    잔여물처럼 보여도 사용자 파일이나 다른 프로세스의 디렉터리일 수 있으므로 provisioning이 재귀
    삭제해서는 안 된다.
    */
    let mut provisioned_slots = 0;
    for slot_number in 1..=DEFAULT_POOL_SIZE {
        let slot_id = slot_id(slot_number);
        if slot_leases.contains_key(&slot_id) {
            continue;
        }
        let slot_path = pool_root.join(&slot_id);
        if worktree_records
            .iter()
            .any(|record| worktree_paths_match(&record.path, &slot_path))
        {
            /*
            worktree inventory에 있으면 이미 git이 관리하는 slot이다. stale/dirty detached 상태는
            provisioning이 아니라 reset_reusable_detached_baseline_slots가 hard reset + clean으로
            회수한다.
            */
            continue;
        }
        let replacement_artifacts =
            normalization_replacement_artifacts_for_slot(pool_root, &slot_id)?;
        if !replacement_artifacts.is_empty() {
            return Err(format!(
                "pool provisioning blocked: normalization recovery for slot `{slot_id}` is incomplete; preserved artifact(s): {}",
                replacement_artifacts
                    .iter()
                    .map(|path| format!("`{}`", path.display()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if slot_path.symlink_metadata().is_ok() {
            continue;
        }

        let Some(slot_parent) = slot_path.parent() else {
            continue;
        };
        ensure_directory_exists(slot_parent).map_err(|error| {
            format!(
                "slot `{slot_id}` parent directory could not be created at `{}`: {error}",
                slot_parent.display()
            )
        })?;

        // Git for Windows는 `\\?\` destination을 worktree path parser에서 거부할 수 있다.
        // 검증된 sibling-relative path를 쓰면 verbatim prefix 없이도 긴 absolute prefix를 피할 수 있다.
        mutation_lock.verify_pool_root(pool_root)?;
        crate::git_execution_guard::ensure_host_git_execution_config_safe(Path::new(
            &git_source_root,
        ))
        .map_err(|error| format!("pool provisioning blocked: {error:#}"))?;
        let git_slot_path = git_worktree_destination(&canonical_repo_root, &slot_path)?;
        let git_slot_path = git_slot_path.to_str().ok_or_else(|| {
            format!("slot `{slot_id}` worktree destination is not valid Unicode for Git")
        })?;
        let report = run_git_sequence(
            format!("provision parallel pool slot `{slot_id}`"),
            vec![GitCommandStep::new(
                "create detached slot worktree",
                [
                    "-C",
                    git_source_root.as_str(),
                    "worktree",
                    "add",
                    "--detach",
                    git_slot_path,
                    baseline_ref,
                ],
            )],
        );
        if !report.succeeded() {
            return Err(format!(
                "slot `{slot_id}` worktree provisioning failed: {}",
                report.failure_summary().unwrap_or_else(|| {
                    "Git worktree creation failed without diagnostics".to_string()
                })
            ));
        }
        provisioned_slots += 1;
    }

    Ok(provisioned_slots)
}

pub(super) fn git_command_directory(canonical_repo_root: &Path) -> Result<String, String> {
    let path = canonical_repo_root.to_str().ok_or_else(|| {
        "canonical repository root is not valid Unicode for Git provisioning".to_string()
    })?;
    #[cfg(not(windows))]
    return Ok(path.to_string());

    #[cfg(windows)]
    {
        if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
            return Ok(format!(r"\\{rest}"));
        }
        Ok(path.strip_prefix(r"\\?\").unwrap_or(path).to_string())
    }
}

pub(super) fn git_worktree_destination(
    canonical_repo_root: &Path,
    slot_path: &Path,
) -> Result<PathBuf, String> {
    if !canonical_repo_root.is_absolute() || !slot_path.is_absolute() {
        return Err("slot worktree destination requires absolute repository paths".to_string());
    }
    if slot_path.starts_with(canonical_repo_root) {
        return Err("slot worktree destination cannot be inside the source repository".to_string());
    }
    let repo_parent = canonical_repo_root
        .parent()
        .ok_or_else(|| "canonical repository root has no parent directory".to_string())?;
    let sibling_relative = slot_path.strip_prefix(repo_parent).map_err(|_| {
        "slot worktree destination is outside the canonical repository sibling root".to_string()
    })?;
    if sibling_relative.as_os_str().is_empty()
        || sibling_relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("slot worktree destination is not a normalized sibling path".to_string());
    }
    #[cfg(not(windows))]
    return Ok(slot_path.to_path_buf());

    #[cfg(windows)]
    {
        Ok(Path::new("..").join(sibling_relative))
    }
}

/*
detached baseline slot은 idle pool의 정상 형태지만, baseline branch가 갱신되면 기존 slot이
이전 commit에 머물 수 있다. 이 함수는 lease가 없는 detached slot만 검사해 현재
`POOL_BASELINE_BRANCH` head와 다르거나 clean하지 않으면 reset sequence를 실행한다.

lease가 있는 slot은 agent 작업이 걸려 있을 수 있으므로 건드리지 않는다. 이 경계가 있어야
reconcile이 pool 위생을 맞추면서도 실행 중인 병렬 작업을 방해하지 않는다.
*/
pub(super) fn reset_reusable_detached_baseline_slots(
    context: ReusableDetachedBaselineResetContext<'_>,
    planning_authority: &dyn PlanningAuthorityPort,
    mutation_lock: &PoolMutationLock,
) -> ReusableDetachedBaselineResetReport {
    /*
    reusable detached reset은 lease가 없는 idle 후보만 대상으로 한다. 이 함수는 "slot worktree가
    detached baseline이어야 한다"는 pool invariant를 baseline branch 이동 뒤에도 유지한다.
    active lease가 있는 slot은 branch/head가 baseline과 달라도 agent 작업일 수 있어 절대
    reset하지 않는다.
    */
    // baseline proof가 없으면 reset 기준이 없으므로 모든 slot을 관찰 전용으로 둔다.
    if context.baseline_ref.is_empty() || mutation_lock.verify_pool_root(context.pool_root).is_err()
    {
        return ReusableDetachedBaselineResetReport::default();
    }

    let mut report = ReusableDetachedBaselineResetReport::default();
    for slot_number in 1..=DEFAULT_POOL_SIZE {
        let slot_id = slot_id(slot_number);
        if context.slot_leases.contains_key(&slot_id)
            || context.invalid_slot_leases.contains(&slot_id)
        {
            /*
            lease record가 있다는 것은 slot 상태 판단의 권위가 runtime projection에 있다는 뜻이다.
            파일시스템만 보고 reset하면 Running agent나 cleanup pending 작업의 산출물을 잃을 수
            있으므로, lease가 있는 slot은 이 helper의 책임 밖으로 둔다.
            */
            continue;
        }
        let slot_path = context.pool_root.join(&slot_id);
        let Some(worktree_record) = context
            .worktree_records
            .iter()
            .find(|record| worktree_paths_match(&record.path, &slot_path))
        else {
            // inventory에 없는 slot은 provisioning/inspection 단계가 다루며, reset 대상이 아니다.
            continue;
        };
        if !worktree_record.detached {
            /*
            branch checkout 상태의 worktree는 idle pool invariant가 이미 깨진 상태일 수 있지만, 이
            함수는 detached baseline refresh 전용이다. branch checkout 불일치는 slot inspection이
            더 구체적인 recovery notice로 보여 주게 남겨 둔다.
            */
            continue;
        }
        if worktree_record.head_sha == context.baseline_ref {
            /*
            head가 현재 baseline이고 git status도 clean이면 reset은 불필요하다. 불필요한 hard
            reset/clean을 피하면 사용자가 보고 있는 idle worktree timestamp나 git metadata churn도
            줄어든다.
            */
            continue;
        }
        if !branch_is_integrated_into(
            context.repo_root,
            &worktree_record.head_sha,
            context.baseline_ref,
        ) {
            continue;
        }
        // head SHA와 worktree dirtiness를 함께 봐야 stale baseline과 dirty idle slot을 모두 잡을 수 있다.
        let Ok(slot_status) = inspect_slot_git_status(&slot_path) else {
            continue;
        };
        let has_normalization_drift = !slot_status.is_clean_baseline()
            && context.normalization_recovery_is_unowned
            && !has_normalization_replacement_artifact_for_slot(
                context.normalization_recovery_artifacts,
                &slot_id,
            )
            && has_target_equivalent_lf_normalization_drift(
                &slot_path,
                slot_status,
                context.baseline_ref,
            );
        if !slot_status.is_clean_baseline() && !has_normalization_drift {
            continue;
        }
        let (reset_report, normalization_quarantine) = if has_normalization_drift {
            if mutation_lock.verify_pool_root(context.pool_root).is_err() {
                continue;
            }
            let recheck_unowned_authority = || {
                ensure_normalization_recovery_authority_is_empty(
                    planning_authority,
                    context.repo_root,
                )
            };
            let outcome = quarantine_normalization_drift_and_replace_slot(
                NormalizationRecoveryRequest {
                    repo_root: context.repo_root,
                    pool_root: context.pool_root,
                    slot_id: &slot_id,
                    slot_path: &slot_path,
                    source_oid: &worktree_record.head_sha,
                    target_ref: context.baseline_ref,
                },
                mutation_lock,
                &recheck_unowned_authority,
            );
            (outcome.report, outcome.quarantine_path)
        } else {
            (
                reset_slot_worktree_to_ref(&slot_path, context.baseline_ref),
                None,
            )
        };
        if reset_report.succeeded() {
            report.reset_slots += 1;
            if let Some(quarantine_path) = normalization_quarantine {
                report.normalization_quarantines.push(quarantine_path);
            }
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::{git_command_directory, git_worktree_destination};
    use std::path::Path;
    #[cfg(windows)]
    use std::path::PathBuf;

    #[test]
    fn git_worktree_destination_stays_with_the_verified_repo_sibling() {
        #[cfg(not(windows))]
        let repo = Path::new("/workspace/project");
        #[cfg(not(windows))]
        let slot = Path::new("/workspace/project-akra-worktrees/hash/akra-pool/slot-1");
        #[cfg(not(windows))]
        let expected = slot.to_path_buf();
        #[cfg(not(windows))]
        let outside = Path::new("/outside/slot-1");

        #[cfg(windows)]
        let repo = Path::new(r"C:\workspace\project");
        #[cfg(windows)]
        let slot = Path::new(r"C:\workspace\project-akra-worktrees\hash\akra-pool\slot-1");
        #[cfg(windows)]
        let expected = PathBuf::from(r"..\project-akra-worktrees\hash\akra-pool\slot-1");
        #[cfg(windows)]
        let outside = Path::new(r"C:\outside\slot-1");

        assert_eq!(
            git_worktree_destination(repo, slot)
                .expect("managed sibling slot should become relative"),
            expected
        );
        assert!(git_worktree_destination(repo, &repo.join("slot-1")).is_err());
        assert!(git_worktree_destination(repo, outside).is_err());
    }

    #[test]
    fn git_command_directory_uses_a_git_compatible_canonical_path() {
        #[cfg(not(windows))]
        assert_eq!(
            git_command_directory(Path::new("/workspace/project"))
                .expect("Unix canonical path should be accepted"),
            "/workspace/project"
        );

        #[cfg(windows)]
        {
            assert_eq!(
                git_command_directory(Path::new(r"\\?\C:\workspace\project"))
                    .expect("Windows drive path should be simplified"),
                r"C:\workspace\project"
            );
            assert_eq!(
                git_command_directory(Path::new(r"\\?\UNC\server\share\project"))
                    .expect("Windows UNC path should be simplified"),
                r"\\server\share\project"
            );
        }
    }
}
