//! Choose a read-only conversation from the current task and exact lease generation.
use anyhow::{Result, bail};

use super::AkraIssueWorkExecutionAdapter;
use super::observation::{self, ObservedExecution};
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeSlotLeaseSnapshot,
};
use crate::domain::planning::TaskStatus;

pub(super) fn thread_id(
    adapter: &AkraIssueWorkExecutionAdapter,
    workspace: &str,
    work_order_id: &str,
) -> Result<Option<String>> {
    let Some(current) = observation::load(adapter, workspace, work_order_id)? else {
        return Ok(None);
    };
    select(&current)
}

fn select(current: &ObservedExecution) -> Result<Option<String>> {
    current.observation.validate().map_err(anyhow::Error::msg)?;
    let Some(active_task_id) = current.observation.active_task_id.as_deref() else {
        bail!("the current issue work task is ambiguous");
    };
    let leases = current
        .runtime
        .slot_leases
        .values()
        .filter(|lease| lease.task_id == active_task_id)
        .collect::<Vec<_>>();
    match leases.as_slice() {
        [lease] => from_current_lease(current, lease),
        [] => {
            if current.observation.question.is_some() {
                bail!("the current work question has no matching lease");
            }
            match current.active_task_status {
                Some(TaskStatus::Ready | TaskStatus::Proposed) => Ok(None),
                Some(TaskStatus::Done | TaskStatus::Cancelled) => historical_thread(
                    current
                        .runtime
                        .session_details
                        .iter()
                        .filter(|detail| detail.task_id == active_task_id),
                ),
                _ => bail!("the active issue work conversation has no current lease"),
            }
        }
        _ => bail!("the current issue work task has multiple leases"),
    }
}

fn from_current_lease(
    current: &ObservedExecution,
    lease: &ParallelModeSlotLeaseSnapshot,
) -> Result<Option<String>> {
    if !lease.has_valid_lease_generation()
        || current.runtime.invalid_slot_leases.contains(&lease.slot_id)
    {
        bail!("the current issue work lease generation is invalid");
    }
    let session_key = lease.session_key();
    let mut candidates = current
        .runtime
        .session_details
        .iter()
        .filter(|detail| detail.session_key == session_key);
    let Some(detail) = candidates.next() else {
        if current.observation.question.is_some() {
            bail!("the current work question session is missing");
        }
        return Ok(None);
    };
    if candidates.next().is_some()
        || detail.task_id != lease.task_id
        || detail.slot_id != lease.slot_id
        || detail.agent_id != lease.agent_id
        || detail.worktree_path != lease.worktree_path
        || detail.branch_name != lease.branch_name
        || detail.lease_started_at != lease.leased_at
    {
        bail!("the current issue work session differs from its lease");
    }
    if let Some(question) = &current.observation.question {
        if current.active_task_status != Some(TaskStatus::AwaitingUser)
            || question.task_id != lease.task_id
            || question.session_key != session_key
            || question.slot_id != lease.slot_id
            || question.worktree_path != lease.worktree_path
            || question.branch_name != lease.branch_name
            || lease.lease_generation.as_deref() != Some(question.lease_generation.as_str())
            || detail.thread_id.as_deref() != Some(question.thread_id.as_str())
        {
            bail!("the current work question belongs to another session");
        }
    }
    // Unknown continuations may be inspected, but this selector never resumes a thread,
    // changes ownership, answers a question, or starts a turn.
    detail.thread_id.as_deref().map(valid_thread).transpose()
}

fn historical_thread<'a>(
    details: impl Iterator<Item = &'a ParallelModeAgentSessionDetailSnapshot>,
) -> Result<Option<String>> {
    let mut threads = std::collections::BTreeSet::new();
    for thread in details.filter_map(|detail| detail.thread_id.as_deref()) {
        threads.insert(valid_thread(thread)?);
    }
    if threads.len() > 1 {
        bail!("the completed issue work task has multiple conversations");
    }
    Ok(threads.into_iter().next())
}

fn valid_thread(thread: &str) -> Result<String> {
    if thread.is_empty()
        || thread.len() > 4096
        || thread
            .chars()
            .any(|value| value.is_control() || value.is_whitespace())
    {
        bail!("the issue work conversation identity is invalid");
    }
    Ok(thread.to_owned())
}

#[cfg(test)]
mod tests;
