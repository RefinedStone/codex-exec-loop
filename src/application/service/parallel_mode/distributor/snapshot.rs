use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeCompletionFeedEntry,
    ParallelModeDistributorSnapshot, ParallelModeOrchestratorStatus, ParallelModeQueueItemState,
    ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
};

use super::super::supervisor::selected_runtime_session_detail;
use super::super::{
    DISTRIBUTOR_INTEGRATION_BRANCH, PoolRuntimeContext, current_branch_name,
    inspect_slot_git_status, short_sha,
};
use super::{ParallelModeDistributorQueueRecord, matching_lease_for_queue_record};

pub(super) fn build_distributor_snapshot_from_context(
    context: &PoolRuntimeContext,
) -> ParallelModeDistributorSnapshot {
    let history = context.session_details.clone();
    let queue_records = context.distributor_queue_records.clone();
    let queue_items = queue_records
        .iter()
        .filter(|record| record.queue_state.is_active())
        .map(ParallelModeDistributorQueueRecord::display_item)
        .collect::<Vec<_>>();
    let completion_feed = build_distributor_completion_feed(&history);

    if let Some(queue_head) = active_distributor_queue_head(&queue_records) {
        return ParallelModeDistributorSnapshot::new(
            queue_items,
            completion_feed,
            queue_head.queue_state.label(),
            queue_head.integration_note.clone(),
        )
        .with_head_blocked_detail(blocked_head_detail(queue_head))
        .with_head_rebase_provenance(rebase_provenance_label(queue_head))
        .with_orchestrator_status(build_orchestrator_status(context, queue_head));
    }

    let Some(detail) = selected_runtime_session_detail(context, &history, &queue_records) else {
        return build_placeholder_distributor_snapshot(
            ParallelModeQueueItemState::Idle.label(),
            "no distributor queue items are waiting",
        )
        .with_orchestrator_status(build_idle_orchestrator_status(context));
    };

    let (head_summary, note) = match detail.state_label.as_str() {
        "reported_complete" => ("reported".to_string(), detail.latest_summary.clone()),
        "ledger_refreshing" => (
            "ledger refreshing".to_string(),
            detail.authority_refresh_outcome.clone(),
        ),
        "commit_ready" => (
            "official".to_string(),
            detail.distributor_outcome.clone().unwrap_or_else(|| {
                "commit-ready result is waiting for distributor enqueue".to_string()
            }),
        ),
        "failed" if detail_has_history_state(&detail, "reported_complete") => (
            "blocked".to_string(),
            detail.authority_refresh_outcome.clone(),
        ),
        _ => (
            ParallelModeQueueItemState::Idle.label().to_string(),
            "no distributor queue items are waiting".to_string(),
        ),
    };

    ParallelModeDistributorSnapshot::new(queue_items, completion_feed, head_summary, note)
        .with_head_rebase_provenance(history_rebase_provenance(&detail))
        .with_orchestrator_status(build_idle_orchestrator_status(context))
}

fn build_orchestrator_status(
    context: &PoolRuntimeContext,
    queue_head: &ParallelModeDistributorQueueRecord,
) -> ParallelModeOrchestratorStatus {
    let active_record_count = context
        .distributor_queue_records
        .iter()
        .filter(|record| record.queue_state.is_active())
        .count();
    let matching_lease = matching_lease_for_queue_record(context, queue_head);

    ParallelModeOrchestratorStatus {
        queue_head: format!(
            "{} / {} / {}",
            queue_head.agent_id,
            queue_head.task_id,
            queue_head.queue_state.label()
        ),
        barrier_state: orchestrator_barrier_state(queue_head, active_record_count),
        blocked_reason: blocked_head_detail(queue_head).or_else(|| {
            queue_head
                .recovery_note
                .clone()
                .filter(|note| !note.trim().is_empty())
        }),
        conflict_files: queue_head.conflict_files.clone(),
        held_queue_count: active_record_count.saturating_sub(1),
        integration_worktree_readiness: inspect_integration_worktree_readiness(context),
        slot_return_wait_reason: slot_return_wait_reason(queue_head, matching_lease),
    }
}

fn build_idle_orchestrator_status(context: &PoolRuntimeContext) -> ParallelModeOrchestratorStatus {
    let mut status = ParallelModeOrchestratorStatus::idle();
    status.integration_worktree_readiness = inspect_integration_worktree_readiness(context);
    status
}

fn orchestrator_barrier_state(
    queue_head: &ParallelModeDistributorQueueRecord,
    active_record_count: usize,
) -> String {
    match queue_head.queue_state {
        ParallelModeQueueItemState::Blocked | ParallelModeQueueItemState::Failed => {
            "blocked".to_string()
        }
        ParallelModeQueueItemState::Cleaning => "slot return".to_string(),
        _ if active_record_count > 1 => {
            format!(
                "head {} holds later queue items",
                queue_head.queue_state.label()
            )
        }
        _ => format!("head {}", queue_head.queue_state.label()),
    }
}

fn inspect_integration_worktree_readiness(context: &PoolRuntimeContext) -> String {
    let repo_root = context.canonical_repo_root.as_path();
    let Some(branch_name) = current_branch_name(repo_root) else {
        return "unknown: branch could not be inspected".to_string();
    };
    if branch_name != DISTRIBUTOR_INTEGRATION_BRANCH {
        return format!(
            "blocked: expected `{DISTRIBUTOR_INTEGRATION_BRANCH}` but checked out `{branch_name}`"
        );
    }

    let Some(status) = inspect_slot_git_status(repo_root) else {
        return "unknown: git status could not be inspected".to_string();
    };
    if status.is_ready_for_integration() {
        format!("ready: {DISTRIBUTOR_INTEGRATION_BRANCH} worktree clean")
    } else {
        format!("blocked: {}", status.detail_label())
    }
}

fn slot_return_wait_reason(
    queue_head: &ParallelModeDistributorQueueRecord,
    matching_lease: Option<&ParallelModeSlotLeaseSnapshot>,
) -> Option<String> {
    let lease = matching_lease?;
    match (queue_head.queue_state, lease.state) {
        (ParallelModeQueueItemState::Cleaning, ParallelModeSlotLeaseState::CleanupPending) => {
            Some(format!(
                "slot `{}` is waiting for cleanup to return idle",
                lease.slot_id
            ))
        }
        (_, ParallelModeSlotLeaseState::CleanupPending) => Some(format!(
            "slot `{}` is waiting for distributor cleanup",
            lease.slot_id
        )),
        (_, ParallelModeSlotLeaseState::Running)
            if matches!(
                queue_head.queue_state,
                ParallelModeQueueItemState::Queued
                    | ParallelModeQueueItemState::Pushing
                    | ParallelModeQueueItemState::PrPending
                    | ParallelModeQueueItemState::MergePending
                    | ParallelModeQueueItemState::Integrating
            ) =>
        {
            Some(format!(
                "slot `{}` stays running until the queue head is integrated",
                lease.slot_id
            ))
        }
        _ => None,
    }
}

fn active_distributor_queue_head(
    queue_records: &[ParallelModeDistributorQueueRecord],
) -> Option<&ParallelModeDistributorQueueRecord> {
    queue_records
        .iter()
        .find(|record| record.queue_state.is_active())
}

fn blocked_head_detail(record: &ParallelModeDistributorQueueRecord) -> Option<String> {
    (record.queue_state == ParallelModeQueueItemState::Blocked)
        .then(|| record.integration_note.clone())
}

fn rebase_provenance_label(record: &ParallelModeDistributorQueueRecord) -> Option<String> {
    let original_commit_sha = record
        .original_commit_sha
        .as_deref()
        .filter(|commit| !commit.trim().is_empty())
        .unwrap_or(record.commit_sha.as_str());
    (original_commit_sha != record.commit_sha).then(|| {
        format!(
            "rebased {} -> {} onto `{DISTRIBUTOR_INTEGRATION_BRANCH}`",
            short_sha(original_commit_sha),
            short_sha(&record.commit_sha)
        )
    })
}

fn history_rebase_provenance(detail: &ParallelModeAgentSessionDetailSnapshot) -> Option<String> {
    detail
        .history
        .iter()
        .rev()
        .find(|entry| entry.state_label == "integrating" && entry.summary.starts_with("rebased "))
        .map(|entry| entry.summary.clone())
}

fn detail_has_history_state(
    detail: &ParallelModeAgentSessionDetailSnapshot,
    state_label: &str,
) -> bool {
    detail
        .history
        .iter()
        .any(|entry| entry.state_label == state_label)
}

fn build_distributor_completion_feed(
    history: &[ParallelModeAgentSessionDetailSnapshot],
) -> Vec<ParallelModeCompletionFeedEntry> {
    vec![
        ParallelModeCompletionFeedEntry::new(
            "reported",
            latest_history_summary_across_records(history, &["reported_complete"])
                .unwrap_or_else(|| "no agent results reported yet".to_string()),
        ),
        ParallelModeCompletionFeedEntry::new(
            "ledger refreshing",
            latest_history_summary_across_records(history, &["ledger_refreshing"])
                .unwrap_or_else(|| "no official refresh workers are active".to_string()),
        ),
        ParallelModeCompletionFeedEntry::new(
            "official",
            latest_history_summary_across_records(history, &["commit_ready"])
                .unwrap_or_else(|| "nothing is queued for merge".to_string()),
        ),
        ParallelModeCompletionFeedEntry::new(
            "merge queued",
            latest_history_summary_across_records(
                history,
                &[
                    "merge_queued",
                    "pushing",
                    "pr_pending",
                    "merge_pending",
                    "integrating",
                ],
            )
            .unwrap_or_else(|| "no distributor queue items are waiting".to_string()),
        ),
        ParallelModeCompletionFeedEntry::new(
            "merged",
            latest_history_summary_across_records(history, &["merged", "cleaned"]).unwrap_or_else(
                || format!("nothing has been integrated into {DISTRIBUTOR_INTEGRATION_BRANCH} yet"),
            ),
        ),
    ]
}

fn latest_history_summary_across_records(
    history: &[ParallelModeAgentSessionDetailSnapshot],
    state_labels: &[&str],
) -> Option<String> {
    history
        .iter()
        .flat_map(|detail| detail.history.iter())
        .filter(|entry| state_labels.contains(&entry.state_label.as_str()))
        .max_by(|left, right| {
            left.timestamp
                .cmp(&right.timestamp)
                .then_with(|| left.summary.cmp(&right.summary))
        })
        .map(|entry| entry.summary.clone())
}

pub(super) fn build_placeholder_distributor_snapshot(
    head_summary: impl Into<String>,
    note: impl Into<String>,
) -> ParallelModeDistributorSnapshot {
    ParallelModeDistributorSnapshot::new(
        Vec::new(),
        vec![
            ParallelModeCompletionFeedEntry::new("reported", "no agent results reported yet"),
            ParallelModeCompletionFeedEntry::new(
                "ledger refreshing",
                "no official refresh workers are active",
            ),
            ParallelModeCompletionFeedEntry::new("official", "nothing is queued for merge"),
            ParallelModeCompletionFeedEntry::new(
                "merge queued",
                "no distributor queue items are waiting",
            ),
            ParallelModeCompletionFeedEntry::new(
                "merged",
                format!("nothing has been integrated into {DISTRIBUTOR_INTEGRATION_BRANCH} yet"),
            ),
        ],
        head_summary,
        note,
    )
}
