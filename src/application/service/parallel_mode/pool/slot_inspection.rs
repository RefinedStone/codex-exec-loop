use super::*;
use crate::domain::parallel_mode::{ParallelModePoolSlotSnapshot, ParallelModePoolSlotState};

use super::paths::display_pool_path;

pub(super) fn inspect_pool_slot(
    context: &PoolRuntimeContext,
    slot_id: &str,
) -> ParallelModePoolSlotSnapshot {
    let slot_path = context.pool_root.join(slot_id);
    let base_worktree_label = display_pool_path(&context.canonical_repo_root, &slot_path);
    let slot_lease = context.slot_leases.get(slot_id);

    if context.invalid_slot_leases.contains(slot_id) {
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
        .find(|record| record.path == slot_path)
    else {
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
            );
        }
        if slot_path.exists() {
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
            POOL_BASELINE_BRANCH,
            base_worktree_label,
            "reconcile pending",
        );
    };

    let Some(slot_status) = inspect_slot_git_status(&slot_path) else {
        return ParallelModePoolSlotSnapshot::new(
            slot_id,
            ParallelModePoolSlotState::Blocked,
            slot_lease
                .map(|lease| lease.branch_name.clone())
                .unwrap_or_else(|| "unknown".to_string()),
            annotate_worktree_label(base_worktree_label, "git status inspection failed"),
            slot_lease
                .map(ParallelModeSlotLeaseSnapshot::owner_label)
                .unwrap_or_else(|| "operator recovery".to_string()),
        );
    };

    if worktree_record.branch_name.as_deref() == Some(POOL_BASELINE_BRANCH)
        || (worktree_record.detached && worktree_record.head_sha == context.baseline_head)
    {
        let branch_label = if worktree_record.detached {
            format!("{POOL_BASELINE_BRANCH} (detached)")
        } else {
            POOL_BASELINE_BRANCH.to_string()
        };

        if let Some(slot_lease) = slot_lease {
            return ParallelModePoolSlotSnapshot::new(
                slot_id,
                ParallelModePoolSlotState::Blocked,
                branch_label,
                annotate_worktree_label(base_worktree_label, "lease exists on idle baseline"),
                slot_lease.owner_label(),
            );
        }

        return if slot_status.is_clean_baseline() {
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
            if slot_status.has_pending_operation {
                return ParallelModePoolSlotSnapshot::new(
                    slot_id,
                    ParallelModePoolSlotState::Blocked,
                    branch_name,
                    annotate_worktree_label(base_worktree_label, &slot_status.detail_label()),
                    slot_lease
                        .map(ParallelModeSlotLeaseSnapshot::owner_label)
                        .unwrap_or_else(|| "operator recovery".to_string()),
                );
            }
            let worktree_clean = slot_status.is_clean_baseline();
            let cleanup_ready = slot_lease.is_none()
                && ParallelModePoolSlotCleanupDecision::new(
                    None,
                    worktree_clean,
                    worktree_clean && branch_is_cleanup_ready(&context.repo_root, branch_name),
                )
                .is_cleanup_ready();
            if cleanup_ready {
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
                return ParallelModePoolSlotSnapshot::new(
                    slot_id,
                    ParallelModePoolSlotState::Blocked,
                    branch_name,
                    annotate_worktree_label(
                        base_worktree_label,
                        &orphan_agent_branch_without_lease_detail(
                            &context.repo_root,
                            branch_name,
                            slot_status,
                        ),
                    ),
                    "operator recovery",
                );
            };
            if slot_lease.branch_name != branch_name {
                return ParallelModePoolSlotSnapshot::new(
                    slot_id,
                    ParallelModePoolSlotState::Blocked,
                    branch_name,
                    annotate_worktree_label(
                        base_worktree_label,
                        "lease branch does not match worktree branch",
                    ),
                    slot_lease.owner_label(),
                );
            }
            if slot_lease.worktree_path != slot_path.display().to_string() {
                return ParallelModePoolSlotSnapshot::new(
                    slot_id,
                    ParallelModePoolSlotState::Blocked,
                    branch_name,
                    annotate_worktree_label(
                        base_worktree_label,
                        "lease worktree path does not match slot path",
                    ),
                    slot_lease.owner_label(),
                );
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

        return ParallelModePoolSlotSnapshot::new(
            slot_id,
            ParallelModePoolSlotState::Blocked,
            branch_name,
            annotate_worktree_label(base_worktree_label, detail),
            slot_lease
                .map(ParallelModeSlotLeaseSnapshot::owner_label)
                .unwrap_or_else(|| "operator recovery".to_string()),
        );
    }

    let detached_label = format!("detached@{}", short_sha(&worktree_record.head_sha));
    ParallelModePoolSlotSnapshot::new(
        slot_id,
        ParallelModePoolSlotState::Blocked,
        detached_label,
        annotate_worktree_label(
            base_worktree_label,
            &format!("detached away from `{POOL_BASELINE_BRANCH}` baseline"),
        ),
        slot_lease
            .map(ParallelModeSlotLeaseSnapshot::owner_label)
            .unwrap_or_else(|| "operator recovery".to_string()),
    )
}

pub(super) fn summarize_pool_reconcile_status(
    slots: &[ParallelModePoolSlotSnapshot],
    pool_root: &Path,
    execution: Option<PoolReconcileExecution>,
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
            action_parts.push(format!("created `{POOL_BASELINE_BRANCH}`"));
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

    if blocked_slots > 0 {
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
            "{}cleanup pending / {awaiting_cleanup_slots} slot(s) still need reset to `{POOL_BASELINE_BRANCH}`",
            prefix
        );
    }

    if idle_slots == slots.len() && !slots.is_empty() {
        return format!(
            "{}reconcile complete / all slots are clean on `{POOL_BASELINE_BRANCH}` baseline",
            prefix
        );
    }

    format!(
        "{}reconcile complete / pool root {}",
        prefix,
        pool_root.display()
    )
}

fn orphan_agent_branch_without_lease_detail(
    repo_root: &str,
    branch_name: &str,
    slot_status: SlotGitStatus,
) -> String {
    let mut parts = Vec::new();
    if branch_is_cleanup_ready(repo_root, branch_name) {
        parts.push("cleanup-ready agent branch has no lease metadata".to_string());
    } else {
        parts.push(NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_DETAIL.to_string());
    }
    if !slot_status.is_clean_baseline() {
        parts.push(slot_status.detail_label());
    }

    parts.join(" / ")
}

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

pub(in crate::application::service::parallel_mode) fn pool_operator_recovery_notice(
    pool: &ParallelModePoolBoardSnapshot,
) -> Option<String> {
    let slot = find_non_merged_orphan_slot_branch(&pool.slots)?;
    Some(format!(
        "pool: blocked / cause: {}",
        non_merged_orphan_slot_branch_notice(&slot.slot_id, &slot.branch_name)
    ))
}

fn non_merged_orphan_slot_branch_notice(slot_id: &str, branch_name: &str) -> String {
    format!(
        "{slot_id} branch `{branch_name}` is not integrated into `{POOL_BASELINE_BRANCH}` and has no lease metadata / next action: {NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_NEXT_ACTION}"
    )
}
