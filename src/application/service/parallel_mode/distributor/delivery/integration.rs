// delivery 하위 모듈의 공통 타입과 helper를 끌어와, integration worktree 검증이
// queue record 차단, slot lease context, Git 상태 조회와 같은 주변 흐름을 같은 어휘로 다루게 한다.
use super::*;

use crate::application::service::parallel_mode::PoolMutationLock;
use crate::application::service::parallel_mode::git_sequence::{GitCommandStep, run_git_sequence};
use crate::application::service::parallel_mode::pool::{
    git_command_directory, git_worktree_destination,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PreparedIntegrationState {
    Fresh,
    ResumePendingPush,
    AlreadyPushed,
}

pub(super) struct PreparedIntegrationWorktree {
    pub(super) repo_root: String,
    pub(super) state: PreparedIntegrationState,
}
/*
integration worktree readiness는 cherry-pick 직전의 마지막 안전 게이트이다.
distributor는 source branch commit을 integration branch에 로컬 cherry-pick하므로, 현재 worktree가
정확히 `DISTRIBUTOR_INTEGRATION_BRANCH`에 있어야 하고 staged/unstaged/rebase/cherry-pick
메타데이터가 없어야 한다.

조건을 만족하지 않으면 queue record를 즉시 blocked로 바꾼다. 그래야 오케스트레이터가
같은 head를 계속 밀어붙이지 않고, supervisor가 operator에게 어떤 worktree 정리가 필요한지
표시할 수 있다.
*/
pub(super) fn prepare_distributor_integration_worktree(
    // 준비 실패를 발견했을 때 queue record를 blocked로 저장하는 영속 포트이다.
    planning_authority: &dyn PlanningAuthorityPort,
    // queue/session mirror 파일 I/O를 수행하는 outbound runtime boundary이다.
    runtime: &dyn ParallelModeRuntimePort,
    github_automation: &dyn GithubAutomationPort,
    // block 기록에 repo root, pool root, lease id를 같이 남기기 위한 slot 해석 결과이다.
    resolution: &WorkspaceSlotLeaseResolution,
    // 이 함수가 직접 상태를 바꾸는 delivery 대상 queue record이다.
    record: &mut ParallelModeDistributorQueueRecord,
    automation_permit: Option<&ParallelModeAutomationPermit>,
    mutation_lock: &PoolMutationLock,
) -> Result<PreparedIntegrationWorktree, String> {
    let Some(target) = record.delivery_target.clone() else {
        let message = "legacy distributor queue record has no immutable delivery target; inspect the queued commit and re-enqueue it explicitly".to_string();
        let _ = block_distributor_queue_record(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            message.clone(),
        )?;
        return Err(message);
    };
    let canonical_repo_root = resolution.context.canonical_repo_root.display().to_string();
    let integration_path = derive_integration_worktree_path(
        &resolution.context.pool_root,
        &target.push_remote,
        &target.github_repository,
        &target.integration_branch,
    );
    let integration_repo_root = integration_path.display().to_string();

    if let Some(notice) = block_if_automation_epoch_closed(
        planning_authority,
        runtime,
        resolution,
        record,
        automation_permit,
        "integration target fetch",
    )? {
        return Err(notice);
    }
    if !fetch_integration_remote_branch(&canonical_repo_root, &target, github_automation) {
        let message = format!(
            "integration branch `{}` could not be fetched from frozen remote `{}`",
            target.integration_branch, target.push_remote
        );
        let _ = block_distributor_queue_record(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            message.clone(),
        )?;
        return Err(message);
    }

    let remote_ref = remote_tracking_branch_ref(&target.push_remote, &target.integration_branch);
    if !runtime.path_exists(&integration_path) {
        if let Some(notice) = block_if_automation_epoch_closed(
            planning_authority,
            runtime,
            resolution,
            record,
            automation_permit,
            "dedicated integration worktree creation",
        )? {
            return Err(notice);
        }
        let Some(parent) = integration_path.parent() else {
            return Err("dedicated integration worktree parent is unavailable".to_string());
        };
        runtime.ensure_directory_exists(parent).map_err(|error| {
            format!("dedicated integration worktree parent could not be created: {error}")
        })?;
        if let Err(error) = mutation_lock.verify_pool_root(&resolution.context.pool_root) {
            let message = format!(
                "dedicated integration worktree creation lost its pool mutation permit: {error}"
            );
            let _ = block_distributor_queue_record(
                planning_authority,
                runtime,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                message.clone(),
            )?;
            return Err(message);
        }
        if let Err(error) = runtime.ensure_git_execution_safe(Path::new(&canonical_repo_root)) {
            let message = format!(
                "dedicated integration worktree creation was blocked by Git execution configuration: {error}"
            );
            let _ = block_distributor_queue_record(
                planning_authority,
                runtime,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                message.clone(),
            )?;
            return Err(message);
        }
        let git_source_root = git_command_directory(&resolution.context.canonical_repo_root)?;
        let git_integration_path =
            git_worktree_destination(&resolution.context.canonical_repo_root, &integration_path)?;
        let git_integration_path = git_integration_path.to_str().ok_or_else(|| {
            "dedicated integration worktree destination is not valid Unicode for Git".to_string()
        })?;
        let creation = run_git_sequence(
            runtime,
            "create dedicated distributor integration worktree",
            vec![GitCommandStep::new(
                "git worktree add for verified integration",
                [
                    "-C",
                    git_source_root.as_str(),
                    "worktree",
                    "add",
                    "--detach",
                    git_integration_path,
                    remote_ref.as_str(),
                ],
            )],
        );
        if !creation.succeeded() {
            let detail = creation
                .failure_summary()
                .unwrap_or_else(|| "git worktree add failed without a diagnostic".to_string());
            let message = format!(
                "dedicated integration worktree could not be created from `{}/{}`: {detail}",
                target.push_remote, target.integration_branch,
            );
            let _ = block_distributor_queue_record(
                planning_authority,
                runtime,
                &resolution.context.repo_root,
                &resolution.context.pool_root,
                Some(&resolution.lease),
                record,
                message.clone(),
            )?;
            return Err(message);
        }
    }

    if runtime.path_is_symlink(&integration_path).unwrap_or(true)
        || !worktrees_share_common_git_dir(runtime, &canonical_repo_root, &integration_repo_root)
    {
        let message = format!(
            "dedicated integration path `{}` is not the generated worktree registered for this repository",
            integration_path.display()
        );
        let _ = block_distributor_queue_record(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            message.clone(),
        )?;
        return Err(message);
    }

    if command_succeeds_with_runtime(
        runtime,
        "git",
        [
            "-C",
            integration_repo_root.as_str(),
            "symbolic-ref",
            "--quiet",
            "HEAD",
        ],
    ) {
        let message = "dedicated integration worktree must remain detached".to_string();
        let _ = block_distributor_queue_record(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            message.clone(),
        )?;
        return Err(message);
    }

    // Git 상태 조회 자체가 실패한 경우에는 clean 여부를 판단할 수 없으므로, 안전한
    // 기본값으로 delivery를 막고 사람이 worktree를 점검하게 한다.
    let Ok(status) = inspect_slot_git_status_with_runtime(runtime, &integration_path) else {
        // status detail이 없는 실패라서 고정 문구만 남긴다. 이 문구는 block reason과
        // 함수 오류 문자열로 그대로 공유된다.
        let message = "integration worktree git status could not be inspected".to_string();
        // 저장되는 block record는 retry loop가 같은 head를 반복 delivery하지 못하게 하는
        // 제어 신호이기도 한다.
        let _ = block_distributor_queue_record(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            message.clone(),
        )?;
        // status를 못 읽은 상태에서는 이후 `is_ready_for_integration` 판정도 불가능하므로
        // 즉시 실패로 빠져나간다.
        return Err(message);
    };
    // readiness는 unstaged/staged 변경뿐 아니라 rebase, merge, cherry-pick 같은 Git
    // operation metadata까지 포함한다. 남은 작업이 있으면 새 cherry-pick은 기존 복구 상태를 덮을 수 있다.
    if !status.is_clean_baseline() {
        // detail label을 message에 넣어 단순히 "not clean"이 아니라 어떤 Git 상태가
        // 막고 있는지 TUI에서 바로 보이게 한다.
        let message = format!(
            "integration worktree must be clean before cherry-pick delivery: {}",
            status.detail_label()
        );
        // dirty worktree는 자동 복구보다 사람 판단이 필요한 상태라 queue record를 blocked로
        // 굳혀 다음 distributor tick이 같은 위험한 cherry-pick을 반복하지 못하게 한다.
        let _ = block_distributor_queue_record(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            message.clone(),
        )?;
        // block 저장을 완료한 뒤 `Err`를 반환해 상위 delivery 함수가 실패 메시지를
        // 그대로 notice나 history에 연결할 수 있게 한다.
        return Err(message);
    }

    let Some(local_head) = resolve_workspace_head_sha_with_runtime(runtime, &integration_path)
    else {
        return block_integration_preparation(
            planning_authority,
            runtime,
            resolution,
            record,
            "dedicated integration worktree HEAD could not be resolved".to_string(),
        );
    };
    let Some(remote_head) =
        resolve_workspace_head_sha_for_ref(runtime, &canonical_repo_root, &remote_ref)
    else {
        return block_integration_preparation(
            planning_authority,
            runtime,
            resolution,
            record,
            "fetched integration branch HEAD could not be resolved".to_string(),
        );
    };

    let state = classify_prepared_integration_state(
        runtime,
        &integration_repo_root,
        &remote_head,
        &local_head,
        record,
    )
    .map_err(|detail| {
        block_distributor_queue_record(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            Some(&resolution.lease),
            record,
            detail.clone(),
        )
        .unwrap_or(detail)
    })?;

    if record.integration_base_commit_sha.is_none() {
        record.integration_base_commit_sha = Some(remote_head.clone());
        record.integration_note = format!(
            "frozen integration target `{}/{}` at `{}` before cherry-pick",
            target.push_remote,
            target.integration_branch,
            short_sha(&remote_head)
        );
        record.updated_at = current_timestamp();
        write_distributor_queue_record(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            record,
        )?;
    }
    if state == PreparedIntegrationState::ResumePendingPush
        && record.integration_commit_sha.is_none()
    {
        record.integration_commit_sha = Some(local_head.clone());
        record.integration_note = format!(
            "recovered reviewed integration result `{}` from the dedicated worktree after restart",
            short_sha(&local_head)
        );
        record.updated_at = current_timestamp();
        write_distributor_queue_record(
            planning_authority,
            runtime,
            &resolution.context.repo_root,
            &resolution.context.pool_root,
            record,
        )?;
    }

    Ok(PreparedIntegrationWorktree {
        repo_root: integration_repo_root,
        state,
    })
}

fn block_integration_preparation<T>(
    planning_authority: &dyn PlanningAuthorityPort,
    runtime: &dyn ParallelModeRuntimePort,
    resolution: &WorkspaceSlotLeaseResolution,
    record: &mut ParallelModeDistributorQueueRecord,
    message: String,
) -> Result<T, String> {
    let _ = block_distributor_queue_record(
        planning_authority,
        runtime,
        &resolution.context.repo_root,
        &resolution.context.pool_root,
        Some(&resolution.lease),
        record,
        message.clone(),
    )?;
    Err(message)
}

fn classify_prepared_integration_state(
    runtime: &dyn ParallelModeRuntimePort,
    integration_repo_root: &str,
    remote_head: &str,
    local_head: &str,
    record: &ParallelModeDistributorQueueRecord,
) -> Result<PreparedIntegrationState, String> {
    let Some(frozen_base) = record.integration_base_commit_sha.as_deref() else {
        if local_head == remote_head {
            return Ok(PreparedIntegrationState::Fresh);
        }
        return Err(format!(
            "dedicated integration worktree HEAD `{}` differs from fetched target `{}` without a persisted integration base",
            short_sha(local_head),
            short_sha(remote_head)
        ));
    };

    if let Some(integrated_head) = record.integration_commit_sha.as_deref() {
        if remote_head == integrated_head && local_head == integrated_head {
            return Ok(PreparedIntegrationState::AlreadyPushed);
        }
        if remote_head == frozen_base && local_head == integrated_head {
            return Ok(PreparedIntegrationState::ResumePendingPush);
        }
        return Err(format!(
            "dedicated integration recovery drifted from frozen base `{}` and reviewed result `{}` (remote `{}`, local `{}`)",
            short_sha(frozen_base),
            short_sha(integrated_head),
            short_sha(remote_head),
            short_sha(local_head)
        ));
    }

    if remote_head != frozen_base {
        return Err(format!(
            "integration target moved from frozen base `{}` to `{}` before the reviewed result was persisted",
            short_sha(frozen_base),
            short_sha(remote_head)
        ));
    }
    if local_head == frozen_base {
        return Ok(PreparedIntegrationState::Fresh);
    }

    validate_recovered_integration_range(
        runtime,
        integration_repo_root,
        frozen_base,
        local_head,
        record,
    )?;
    Ok(PreparedIntegrationState::ResumePendingPush)
}

fn validate_recovered_integration_range(
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
    frozen_base: &str,
    local_head: &str,
    record: &ParallelModeDistributorQueueRecord,
) -> Result<(), String> {
    let source_states_at_base =
        distributor_source_cherry_states(runtime, repo_root, frozen_base, record)?;
    let pending_source_count = source_states_at_base
        .iter()
        .filter(|(_, equivalent)| !*equivalent)
        .count();
    if pending_source_count == 0 {
        return Err(
            "dedicated integration worktree moved even though no source patch remained".to_string(),
        );
    }

    let recovered_commits =
        resolve_linear_distributor_source_range(runtime, repo_root, frozen_base, local_head)?;
    if recovered_commits.len() != pending_source_count {
        return Err(format!(
            "recovered integration range contains {} commit(s), expected {pending_source_count}",
            recovered_commits.len()
        ));
    }
    if !distributor_source_cherry_states(runtime, repo_root, local_head, record)?
        .iter()
        .all(|(_, equivalent)| *equivalent)
    {
        return Err(
            "recovered integration worktree does not contain every frozen source patch".to_string(),
        );
    }

    // A cherry-pick can reproduce the source commit object exactly when its
    // parent and committer metadata also match. `git cherry` then omits that
    // commit because it is already a literal ancestor of `source_tip`. Treat
    // exact object identity as the strongest equivalence proof before asking
    // `git cherry` to classify only the remaining recovered commits.
    let source_commits = source_states_at_base
        .iter()
        .map(|(commit, _)| commit.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let mut classified = recovered_commits
        .iter()
        .filter(|commit| source_commits.contains(commit.as_str()))
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    if classified.len() == recovered_commits.len() {
        return Ok(());
    }

    let source_tip = record.effective_source_commit_sha();
    let output = run_command_with_runtime(
        runtime,
        "git",
        [
            "-C",
            repo_root,
            "cherry",
            source_tip.as_str(),
            local_head,
            frozen_base,
        ],
        None,
    )
    .ok_or_else(|| {
        "recovered integration commits could not be compared to the source".to_string()
    })?;
    for line in output.lines() {
        let mut fields = line.split_whitespace();
        let marker = fields.next().unwrap_or_default();
        let sha = fields.next().unwrap_or_default();
        if fields.next().is_some()
            || marker != "-"
            || !recovered_commits.iter().any(|commit| commit == sha)
            || !classified.insert(sha.to_string())
        {
            return Err(
                "recovered integration worktree contains a commit not patch-equivalent to the frozen source range"
                    .to_string(),
            );
        }
    }
    if classified.len() != recovered_commits.len() {
        return Err(
            "recovered integration patch comparison omitted one or more commits".to_string(),
        );
    }
    Ok(())
}

fn worktrees_share_common_git_dir(
    runtime: &dyn ParallelModeRuntimePort,
    canonical_repo_root: &str,
    integration_repo_root: &str,
) -> bool {
    let resolve = |repo_root: &str| {
        run_command_with_runtime(
            runtime,
            "git",
            [
                "-C",
                repo_root,
                "rev-parse",
                "--path-format=absolute",
                "--git-common-dir",
            ],
            None,
        )
        .and_then(|path| runtime.canonicalize_path(Path::new(&path)).ok())
    };
    resolve(canonical_repo_root).is_some_and(|canonical_common_dir| {
        resolve(integration_repo_root).as_ref() == Some(&canonical_common_dir)
    })
}

pub(super) fn fetch_integration_remote_branch(
    repo_root: &str,
    target: &PlanningAuthorityDistributorDeliveryTarget,
    github_automation: &dyn GithubAutomationPort,
) -> bool {
    let Some(fetch_source) = target.credential_redacted_push_url.as_deref() else {
        return false;
    };
    let tracking_ref = remote_tracking_branch_ref(&target.push_remote, &target.integration_branch);
    github_automation
        .fetch_branch_to_tracking_ref_for_delivery_target(
            repo_root,
            &target.push_remote,
            fetch_source,
            &target.integration_branch,
            &tracking_ref,
        )
        .is_ok()
}

fn resolve_workspace_head_sha_for_ref(
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
    reference: &str,
) -> Option<String> {
    run_command_with_runtime(
        runtime,
        "git",
        ["-C", repo_root, "rev-parse", reference],
        None,
    )
}

/*
cherry-pick이 충돌하면 Git은 conflicted file 목록을 index에 남긴다. 이 함수는
unmerged file만 수집해 queue record의 conflict_files에 저장할 짧은 목록으로 바꾼다.
그 목록은 supervisor orchestrator status와 blocked notice에서 사용자가 어디를 봐야 하는지
알려 주는 복구 단서가 된다.
*/
pub(super) fn collect_cherry_pick_conflict_files(
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
) -> Vec<String> {
    run_command_with_runtime(
        runtime,
        "git",
        ["-C", repo_root, "diff", "--name-only", "--diff-filter=U"],
        None,
    )
    // 충돌 파일 수집은 차단 message를 보강하는 보조 정보라, Git 명령 실패만으로
    // delivery 실패 이유를 바꾸지 않고 빈 목록으로 낮춘다.
    .unwrap_or_default()
    // `git diff --name-only` 출력은 파일마다 한 줄이라 그대로 record 목록의 후보가 된다.
    .lines()
    // 줄 끝 개행이나 주변 공백이 history에 들어가지 않도록 정규화한다.
    .map(str::trim)
    // 빈 줄은 사용자가 열어볼 수 있는 파일 경로가 아니므로 record에서 제외한다.
    .filter(|line| !line.is_empty())
    // queue record가 owned `String` 목록을 저장하므로 Git 출력 버퍼에서 독립된 값을 만든다.
    .map(str::to_string)
    // caller가 conflict files를 block reason과 별도 필드에 같이 넣을 수 있도록 Vec로 확정한다.
    .collect::<Vec<_>>()
}

/*
conflict suffix는 block message에 붙는 사람이 읽는 요약이다. 충돌 파일이 없으면
불필요한 빈 "conflicts" 문구를 붙이지 않고, 목록이 있으면 한 줄에 합쳐 TUI notice와 queue
record history가 같은 형식으로 원인을 보여 주게 한다.
*/
pub(super) fn format_conflict_file_suffix(conflict_files: &[String]) -> String {
    // 충돌 파일을 찾지 못한 경우에는 원래 block reason만 남겨, 비어 있는 suffix가
    // notice 문장을 어색하게 만들지 않도록 한다.
    if conflict_files.is_empty() {
        String::new()
    } else {
        // 여러 conflict file을 한 줄 suffix로 접어 queue history, notice, test assertion이
        // 모두 같은 사람이 읽는 형식을 공유하게 한다.
        format!(" / conflicts: {}", conflict_files.join(", "))
    }
}
