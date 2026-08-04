use super::paths::display_pool_path;
use super::*;
use crate::domain::parallel_mode::{ParallelModePoolSlotSnapshot, ParallelModePoolSlotState};

const INTEGRATION_PROOF_UNAVAILABLE_DETAIL: &str =
    "integration proof unavailable until a successful remote reconcile fetch";

/*
pool slot inspection은 git worktree 상태와 lease metadata를 합쳐 하나의 화면용 slot snapshot으로
바꾸는 판정기다. 같은 slot path라도 "worktree 없음", "baseline에 있지만 lease가 남음", "agent
branch가 있는데 lease가 없음", "lease와 branch가 일치함"처럼 여러 의미가 있을 수 있다. 이
함수는 위험한 상태를 먼저 Blocked로 분류하고, 마지막에만 Idle이나 lease 기반 상태를 반환한다.

판정 순서가 중요하다. invalid lease metadata, missing worktree, git status 실패 같은 운영자가
복구해야 하는 조건을 먼저 처리해야 뒤쪽의 정상 branch 판정이 잘못 덮어쓰지 않는다. 이 함수의
출력은 supervisor pool board, reconcile summary, cleanup 후보 판단에 연결된다.
*/
pub(super) fn inspect_pool_slot(
    runtime: &dyn ParallelModeRuntimePort,
    context: &PoolRuntimeContext,
    slot_id: &str,
) -> ParallelModePoolSlotSnapshot {
    let slot_path = context.pool_root.join(slot_id);
    let baseline_branch = pool_baseline_branch_for_repo(runtime, &context.repo_root);
    let base_worktree_label = display_pool_path(&context.canonical_repo_root, &slot_path);
    let slot_lease = context.slot_leases.get(slot_id);
    if context.invalid_slot_leases.contains(slot_id) {
        /*
        invalid lease metadata는 가장 먼저 Blocked로 분류한다. JSON/schema 파싱이 실패했거나
        authority projection에서 slot id와 맞지 않는 lease가 들어온 경우에는 branch나 worktree
        상태를 아무리 검사해도 owner/task/branch 관계를 신뢰할 수 없다. 그래서 operator가
        metadata를 고치기 전까지 자동 cleanup이나 새 lease 배정을 막는다.
        */
        return ParallelModePoolSlotSnapshot::new(
            slot_id,
            ParallelModePoolSlotState::Blocked,
            "unknown",
            annotate_worktree_label(base_worktree_label, "invalid lease metadata"),
            "operator recovery",
        );
    }
    let Some(worktree_record) = context
        .worktree_records
        .iter()
        .find(|record| worktree_paths_match_with_runtime(runtime, &record.path, &slot_path))
    else {
        /*
        worktree inventory에 slot path가 없다는 것은 세 가지로 나뉜다. lease가 있으면 runtime은
        slot을 사용 중으로 보는데 git worktree가 사라진 위험 상태이고, path만 있으면 git이
        관리하지 않는 충돌 directory다. 둘 다 자동으로 덮어쓰면 사용자 파일이나 실행 중 작업을
        잃을 수 있어 Blocked로 둔다. path도 lease도 없을 때만 Missing으로 보고 reconcile이 새
        idle worktree를 만들 수 있게 한다.
        */
        if let Some(slot_lease) = slot_lease {
            return ParallelModePoolSlotSnapshot::new(
                slot_id,
                ParallelModePoolSlotState::Blocked,
                slot_lease.branch_name.clone(),
                annotate_worktree_label(
                    base_worktree_label,
                    "lease exists but worktree is missing",
                ),
                slot_lease.owner_label(),
            )
            .with_owner_identity_from_lease(slot_lease);
        }
        if runtime.path_exists(&slot_path) {
            return ParallelModePoolSlotSnapshot::new(
                slot_id,
                ParallelModePoolSlotState::Blocked,
                "unknown",
                annotate_worktree_label(
                    base_worktree_label,
                    "directory exists outside git worktree inventory",
                ),
                "operator recovery",
            );
        }
        return ParallelModePoolSlotSnapshot::new(
            slot_id,
            ParallelModePoolSlotState::Missing,
            baseline_branch.clone(),
            base_worktree_label,
            "reconcile pending",
        );
    };
    let Ok(slot_status) = inspect_slot_git_status_with_runtime(runtime, &slot_path) else {
        /*
        git status를 읽지 못하면 이 slot이 clean baseline인지, rebase/cherry-pick 중인지,
        untracked 파일을 품고 있는지 알 수 없다. unknown 상태에서 idle이나 cleanup-ready로 분류하면
        reset/clean 같은 destructive 작업으로 이어질 수 있으므로 Blocked를 반환한다.
        */
        return preserve_slot_owner_identity(
            ParallelModePoolSlotSnapshot::new(
                slot_id,
                ParallelModePoolSlotState::Blocked,
                slot_lease
                    .map(|lease| lease.branch_name.clone())
                    .unwrap_or_else(|| "unknown".to_string()),
                annotate_worktree_label(base_worktree_label, "git status inspection failed"),
                slot_lease
                    .map(ParallelModeSlotLeaseSnapshot::owner_label)
                    .unwrap_or_else(|| "operator recovery".to_string()),
            ),
            slot_lease,
        );
    };
    if worktree_record.branch_name.as_deref() == Some(baseline_branch.as_str())
        || (worktree_record.detached && worktree_record.head_sha == context.baseline_head)
    {
        /*
        baseline branch checkout 또는 baseline head detached 상태는 idle slot의 정상 후보다. 하지만
        lease metadata가 같이 있으면 runtime projection은 누군가 이 slot을 소유한다고 말하고 git은
        idle이라고 말하는 split-brain 상태다. 이 경우에는 lease owner를 표시하면서 Blocked로 올려
        자동 재사용을 막는다.
        */
        let branch_label = if worktree_record.detached {
            format!("{} (detached)", baseline_branch)
        } else {
            baseline_branch.clone()
        };
        if let Some(slot_lease) = slot_lease {
            return ParallelModePoolSlotSnapshot::new(
                slot_id,
                ParallelModePoolSlotState::Blocked,
                branch_label,
                annotate_worktree_label(base_worktree_label, "lease exists on idle baseline"),
                slot_lease.owner_label(),
            )
            .with_owner_identity_from_lease(slot_lease);
        }
        return if slot_status.is_clean_baseline() {
            /*
            clean baseline만 Idle이다. branch/head가 baseline이어도 staged, unstaged, untracked
            파일이나 pending operation이 있으면 다음 agent에게 넘길 수 없는 오염된 slot이다.
            `SlotGitStatus::is_clean_baseline`이 그 마지막 안전 게이트다.
            */
            ParallelModePoolSlotSnapshot::new(
                slot_id,
                ParallelModePoolSlotState::Idle,
                branch_label,
                base_worktree_label,
                "idle baseline",
            )
        } else {
            ParallelModePoolSlotSnapshot::new(
                slot_id,
                ParallelModePoolSlotState::Blocked,
                branch_label,
                annotate_worktree_label(base_worktree_label, &slot_status.detail_label()),
                "operator recovery",
            )
        };
    }
    if let Some(branch_name) = worktree_record.branch_name.as_deref() {
        let expected_agent_prefix = format!("{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/");
        if branch_name.starts_with(&expected_agent_prefix) {
            /*
            `akra-agent/<slot-id>/...` branch는 이 slot이 agent 작업을 수행했거나 수행 중인 흔적이다.
            여기부터는 lease metadata와 branch/worktree가 서로 맞는지 검증한다. 같은 prefix라도
            lease가 없으면 orphan, lease branch가 다르면 metadata drift, lease path가 다르면 slot
            path drift로 각각 다른 복구 메시지를 낸다.
            */
            if slot_status.has_pending_operation {
                /*
                pending git operation은 lease 유무와 관계없이 Blocked다. rebase, merge, cherry-pick
                중인 worktree는 commit graph가 아직 안정되지 않았고, cleanup readiness나 owner
                상태보다 먼저 수동 복구가 필요하다.
                */
                return preserve_slot_owner_identity(
                    ParallelModePoolSlotSnapshot::new(
                        slot_id,
                        ParallelModePoolSlotState::Blocked,
                        branch_name,
                        annotate_worktree_label(base_worktree_label, &slot_status.detail_label()),
                        slot_lease
                            .map(ParallelModeSlotLeaseSnapshot::owner_label)
                            .unwrap_or_else(|| "operator recovery".to_string()),
                    ),
                    slot_lease,
                );
            }
            let worktree_clean = slot_status.is_clean_baseline();
            let cleanup_ready = slot_lease.is_none()
                && context.integration_target_proof_is_fresh
                && ParallelModePoolSlotCleanupDecision::new(
                    None,
                    worktree_clean,
                    worktree_clean
                        && branch_patch_is_integrated(
                            runtime,
                            &context.repo_root,
                            branch_name,
                            &context.baseline_head,
                        ),
                )
                .is_cleanup_ready();
            if cleanup_ready {
                /*
                lease가 없는 agent branch라도 clean하고 baseline에 통합되어 있으면 사용자가 잃을
                작업이 없으므로 AwaitingCleanup으로 낮출 수 있다. 이 상태는 Blocked가 아니라
                reconcile/cleanup path가 slot을 idle baseline으로 회수할 수 있는 회색 지대다.
                */
                return ParallelModePoolSlotSnapshot::new(
                    slot_id,
                    ParallelModePoolSlotState::AwaitingCleanup,
                    branch_name,
                    annotate_worktree_label(base_worktree_label, &slot_status.detail_label()),
                    slot_lease
                        .map(ParallelModeSlotLeaseSnapshot::owner_label)
                        .unwrap_or_else(|| "cleanup pending".to_string()),
                );
            }
            let Some(slot_lease) = slot_lease else {
                /*
                lease 없이 남은 agent branch가 cleanup-ready도 아니면 operator recovery가 필요하다.
                branch가 아직 prerelease에 통합되지 않았을 수 있으므로, 자동 reset이나 branch
                delete를 하면 agent 산출물을 잃을 수 있다.
                */
                return ParallelModePoolSlotSnapshot::new(
                    slot_id,
                    ParallelModePoolSlotState::Blocked,
                    branch_name,
                    annotate_worktree_label(
                        base_worktree_label,
                        &orphan_agent_branch_without_lease_detail(
                            runtime,
                            context,
                            branch_name,
                            slot_status,
                        ),
                    ),
                    "operator recovery",
                );
            };
            if slot_lease.branch_name != branch_name {
                /*
                lease branch와 실제 worktree branch가 다르면 runtime projection과 git 상태가 서로
                다른 작업을 가리킨다. 이 mismatch를 무시하면 dispatcher가 잘못된 branch를
                완료/cleanup할 수 있으므로 Blocked로 고정한다.
                */
                return ParallelModePoolSlotSnapshot::new(
                    slot_id,
                    ParallelModePoolSlotState::Blocked,
                    branch_name,
                    annotate_worktree_label(
                        base_worktree_label,
                        "lease branch does not match worktree branch",
                    ),
                    slot_lease.owner_label(),
                )
                .with_owner_identity_from_lease(slot_lease);
            }
            if !worktree_paths_match_with_runtime(
                runtime,
                Path::new(&slot_lease.worktree_path),
                &slot_path,
            ) {
                /*
                lease worktree path mismatch는 같은 slot id라도 실제 디렉터리 연결이 어긋났다는
                뜻이다. nested workspace resolve나 cleanup 경로가 path를 기준으로 동작하므로,
                branch가 맞아도 path가 다르면 안전하게 작업을 계속할 수 없다.
                */
                return ParallelModePoolSlotSnapshot::new(
                    slot_id,
                    ParallelModePoolSlotState::Blocked,
                    branch_name,
                    annotate_worktree_label(
                        base_worktree_label,
                        "lease worktree path does not match slot path",
                    ),
                    slot_lease.owner_label(),
                )
                .with_owner_identity_from_lease(slot_lease);
            }
            return ParallelModePoolSlotSnapshot::from_lease(
                slot_id,
                branch_name,
                annotate_worktree_label(base_worktree_label, &slot_status.detail_label()),
                slot_lease,
            );
        }
        let detail = if branch_name.starts_with(&format!("{AKRA_AGENT_BRANCH_PREFIX}/")) {
            "agent branch belongs to a different slot"
        } else {
            "unexpected branch for pool slot"
        };

        /*
        branch가 현재 slot prefix와 맞지 않으면 idle도 active lease도 아니다. 다른 slot의 agent
        branch가 checkout되어 있으면 pool invariant가 깨진 것이고, 일반 unexpected branch면
        사용자가 수동으로 checkout한 상태일 수 있다. 둘 다 자동 reset 대상이 아니므로 Blocked로
        표시한다.
        */
        return preserve_slot_owner_identity(
            ParallelModePoolSlotSnapshot::new(
                slot_id,
                ParallelModePoolSlotState::Blocked,
                branch_name,
                annotate_worktree_label(base_worktree_label, detail),
                slot_lease
                    .map(ParallelModeSlotLeaseSnapshot::owner_label)
                    .unwrap_or_else(|| "operator recovery".to_string()),
            ),
            slot_lease,
        );
    }
    let detached_label = format!("detached@{}", short_sha(&worktree_record.head_sha));
    /*
    마지막 fallback은 branch가 없고 baseline head도 아닌 detached worktree다. 이는 과거 baseline,
    수동 checkout, 실패한 reset 등 여러 원인이 가능하지만 자동으로 어떤 commit인지 해석하지 않는다.
    short sha를 label로 노출해 운영자가 실제 commit을 확인하게 한다.
    */
    preserve_slot_owner_identity(
        ParallelModePoolSlotSnapshot::new(
            slot_id,
            ParallelModePoolSlotState::Blocked,
            detached_label,
            annotate_worktree_label(
                base_worktree_label,
                &format!("detached away from `{}` baseline", baseline_branch),
            ),
            slot_lease
                .map(ParallelModeSlotLeaseSnapshot::owner_label)
                .unwrap_or_else(|| "operator recovery".to_string()),
        ),
        slot_lease,
    )
}

fn preserve_slot_owner_identity(
    snapshot: ParallelModePoolSlotSnapshot,
    slot_lease: Option<&ParallelModeSlotLeaseSnapshot>,
) -> ParallelModePoolSlotSnapshot {
    match slot_lease {
        Some(lease) => snapshot.with_owner_identity_from_lease(lease),
        None => snapshot,
    }
}

/*
reconcile status 문구는 pool board의 여러 slot 상태를 한 줄의 운영 상태로 압축한다. 단순 count만
세는 것이 아니라, 실행한 reconcile action이 있으면 prefix로 붙이고, non-merged orphan branch처럼
실제 복구 행동이 필요한 원인을 우선 노출한다.

이 문자열은 TUI의 supervisor top/detail에서 사람이 바로 읽는 상태다. 따라서 Missing,
AwaitingCleanup, Blocked, Idle의 조합을 사용자가 다음 행동으로 옮길 수 있는 문장으로 바꾸는
adapter 역할을 한다.
*/
pub(super) fn summarize_pool_reconcile_status(
    slots: &[ParallelModePoolSlotSnapshot],
    pool_root: &Path,
    baseline_branch: &str,
    execution: Option<PoolReconcileExecution>,
    normalization_recovery_artifacts: &[PathBuf],
) -> String {
    let idle_slots = slots
        .iter()
        .filter(|slot| slot.state == ParallelModePoolSlotState::Idle)
        .count();
    let awaiting_cleanup_slots = slots
        .iter()
        .filter(|slot| slot.state == ParallelModePoolSlotState::AwaitingCleanup)
        .count();
    let blocked_slots = slots
        .iter()
        .filter(|slot| slot.state == ParallelModePoolSlotState::Blocked)
        .count();
    let missing_slots = slots
        .iter()
        .filter(|slot| slot.state == ParallelModePoolSlotState::Missing)
        .count();
    let mut prefix = String::new();
    if let Some(execution) = execution.filter(|execution| execution.has_actions()) {
        let mut action_parts = Vec::new();
        if execution.created_baseline_branch {
            action_parts.push(format!("created `{baseline_branch}`"));
        }
        if execution.created_pool_root {
            action_parts.push("created pool root".to_string());
        }
        if execution.provisioned_slots > 0 {
            action_parts.push(format!("provisioned {}", execution.provisioned_slots));
        }
        if execution.cleaned_slots > 0 {
            action_parts.push(format!("cleaned {}", execution.cleaned_slots));
        }
        prefix = format!("actions: {} / ", action_parts.join(", "));
    }
    if let Some(first_artifact) = normalization_recovery_artifacts.first() {
        let additional = normalization_recovery_artifacts.len().saturating_sub(1);
        let additional = if additional == 0 {
            String::new()
        } else {
            format!(" (+{additional} more)")
        };
        prefix.push_str(&format!(
            "preserved normalization recovery: `{}`{additional} / ",
            first_artifact.display()
        ));
    }
    if blocked_slots > 0 {
        if let Some(slot) = find_proof_unavailable_orphan_slot_branch(slots) {
            return format!(
                "{}reconcile blocked / cause: {} / blocked: {blocked_slots} / missing: {missing_slots} / cleanup: {awaiting_cleanup_slots} / root {}",
                prefix,
                proof_unavailable_orphan_slot_branch_notice(&slot.slot_id, &slot.branch_name),
                pool_root.display()
            );
        }
        if let Some(slot) = find_non_merged_orphan_slot_branch(slots) {
            return format!(
                "{}reconcile blocked / cause: {} / blocked: {blocked_slots} / missing: {missing_slots} / cleanup: {awaiting_cleanup_slots} / root {}",
                prefix,
                non_merged_orphan_slot_branch_notice(&slot.slot_id, &slot.branch_name),
                pool_root.display()
            );
        }
        return format!(
            "{}reconcile blocked / blocked: {blocked_slots} / missing: {missing_slots} / cleanup: {awaiting_cleanup_slots} / root {}",
            prefix,
            pool_root.display()
        );
    }
    if missing_slots > 0 && awaiting_cleanup_slots > 0 {
        return format!(
            "{}reconcile pending / missing: {missing_slots} / cleanup pending: {awaiting_cleanup_slots} / root {}",
            prefix,
            pool_root.display()
        );
    }
    if missing_slots > 0 {
        return format!(
            "{}reconcile pending / create {missing_slots} missing slot(s) under {}",
            prefix,
            pool_root.display()
        );
    }
    if awaiting_cleanup_slots > 0 {
        return format!(
            "{}cleanup pending / {awaiting_cleanup_slots} slot(s) still need reset to `{}`",
            prefix, baseline_branch
        );
    }
    if idle_slots == slots.len() && !slots.is_empty() {
        return format!(
            "{}reconcile complete / all slots are clean on `{}` baseline",
            prefix, baseline_branch
        );
    }

    format!(
        "{}reconcile complete / pool root {}",
        prefix,
        pool_root.display()
    )
}

/*
agent branch가 있는데 lease metadata가 없으면 두 가지 가능성이 있다. 이미 baseline에 통합되어
cleanup만 남은 branch이거나, 아직 통합되지 않은 작업 branch가 원장 없이 남은 위험 상태다. 이
함수는 git ancestry와 worktree 청결도를 합쳐 어떤 복구 문구를 보여 줄지 결정한다.
*/
fn orphan_agent_branch_without_lease_detail(
    runtime: &dyn ParallelModeRuntimePort,
    context: &PoolRuntimeContext,
    branch_name: &str,
    slot_status: SlotGitStatus,
) -> String {
    let mut parts = Vec::new();
    if !context.integration_target_proof_is_fresh {
        parts.push(INTEGRATION_PROOF_UNAVAILABLE_DETAIL.to_string());
    } else if branch_patch_is_integrated(
        runtime,
        &context.repo_root,
        branch_name,
        &context.baseline_head,
    ) {
        parts.push("cleanup-ready agent branch has no lease metadata".to_string());
    } else {
        parts.push(NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_DETAIL.to_string());
    }
    if !slot_status.is_clean_baseline() {
        parts.push(slot_status.detail_label());
    }

    parts.join(" / ")
}

/*
pool 전체 공지에서는 가장 위험한 orphan slot branch를 먼저 찾아야 한다. lease가 없고 아직
baseline에 통합되지 않은 agent branch는 자동 cleanup 대상이 아니며, 사용자 작업을 잃지 않으려면
운영자가 직접 통합하거나 삭제 판단을 해야 한다.
*/
fn find_non_merged_orphan_slot_branch(
    slots: &[ParallelModePoolSlotSnapshot],
) -> Option<&ParallelModePoolSlotSnapshot> {
    slots.iter().find(|slot| {
        slot.state == ParallelModePoolSlotState::Blocked
            && slot.owner_label == "operator recovery"
            && slot
                .worktree_label
                .contains(NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_DETAIL)
    })
}

fn find_proof_unavailable_orphan_slot_branch(
    slots: &[ParallelModePoolSlotSnapshot],
) -> Option<&ParallelModePoolSlotSnapshot> {
    slots.iter().find(|slot| {
        slot.state == ParallelModePoolSlotState::Blocked
            && slot.owner_label == "operator recovery"
            && slot
                .worktree_label
                .contains(INTEGRATION_PROOF_UNAVAILABLE_DETAIL)
    })
}

fn proof_unavailable_orphan_slot_branch_notice(slot_id: &str, branch_name: &str) -> String {
    format!(
        "{slot_id} branch `{branch_name}` has no lease metadata and its remote integration proof is unavailable / next action: run a remote reconcile fetch before cleanup"
    )
}

/*
supervisor 상단 notice는 pool board 전체에서 가장 시급한 operator recovery 메시지를 하나만 고른다.
여기서는 non-merged orphan branch를 별도 notice로 승격한다. 이 상태는 reconcile을 반복해도
자동으로 해결되지 않으므로, 일반 blocked count보다 구체적인 원인과 next action을 보여 주는 것이
중요하다.
*/
pub(in crate::application::service::parallel_mode) fn pool_operator_recovery_notice(
    pool: &ParallelModePoolBoardSnapshot,
) -> Option<String> {
    if let Some(slot) = find_proof_unavailable_orphan_slot_branch(&pool.slots) {
        return Some(format!(
            "pool: blocked / cause: {}",
            proof_unavailable_orphan_slot_branch_notice(&slot.slot_id, &slot.branch_name)
        ));
    }
    let slot = find_non_merged_orphan_slot_branch(&pool.slots)?;
    Some(format!(
        "pool: blocked / cause: {}",
        non_merged_orphan_slot_branch_notice(&slot.slot_id, &slot.branch_name)
    ))
}
fn non_merged_orphan_slot_branch_notice(slot_id: &str, branch_name: &str) -> String {
    format!(
        "{slot_id} branch `{branch_name}` is not integrated into the configured integration branch and has no lease metadata / next action: {NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_NEXT_ACTION}"
    )
}

#[cfg(test)]
mod owner_identity_tests {
    use super::*;
    use crate::domain::parallel_mode::ParallelModeSlotLeaseState;

    #[test]
    fn blocked_slot_projection_keeps_typed_lease_owner_identity() {
        let lease = ParallelModeSlotLeaseSnapshot::new(
            "slot-1",
            "task-1",
            "Task One",
            "agent-1",
            "akra-agent/slot-1/task-1",
            "/tmp/slot-1",
            ParallelModeSlotLeaseState::Running,
            "2026-07-13T00:00:00Z",
            Some("2026-07-13T00:00:01Z".to_string()),
        )
        .with_lease_generation("a".repeat(64));
        let snapshot = ParallelModePoolSlotSnapshot::new(
            "slot-1",
            ParallelModePoolSlotState::Blocked,
            lease.branch_name.clone(),
            "lease exists but worktree is missing",
            lease.owner_label(),
        );

        let projected = preserve_slot_owner_identity(snapshot, Some(&lease));
        let owner = projected
            .owner_identity
            .expect("blocked lease-backed slot must retain typed identity");

        assert_eq!(owner.agent_id, "agent-1");
        assert_eq!(owner.task_id, "task-1");
        assert_eq!(owner.session_key, lease.session_key());
        assert_eq!(owner.lease_generation, lease.lease_generation);
    }
}
