//! Choose a read-only conversation from the current task and exact lease generation.
use anyhow::{Result, bail};

use super::AkraIssueWorkExecutionAdapter;
use super::observation::{self, ObservedExecution};
use crate::domain::issue_automation::agent_question::IssueAgentAsk;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeSlotLeaseSnapshot,
    ParallelModeSlotLeaseState,
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

/// The worker knows its branch. One lease names the task; its work order names the issue.
pub(super) fn asking_worker(
    adapter: &AkraIssueWorkExecutionAdapter,
    workspace: &str,
    branch: &str,
) -> Result<Option<(String, IssueAgentAsk)>> {
    let runtime = adapter.authority.load_runtime_projections(workspace)?;
    let mut leases = runtime
        .slot_leases
        .values()
        .filter(|lease| lease.branch_name == branch);
    let (Some(lease), None) = (leases.next(), leases.next()) else {
        return Ok(None);
    };
    let Some(order) = adapter
        .repository
        .load_work_order_for_task(workspace, &lease.task_id)?
    else {
        return Ok(None);
    };
    Ok(
        asking_task(adapter, workspace, &order.work_order_id, &lease.task_id)?
            .map(|ask| (order.work_order_id, ask)),
    )
}

/// The asking task must be the active task with one running lease and one attributable thread.
fn asking_task(
    adapter: &AkraIssueWorkExecutionAdapter,
    workspace: &str,
    work_order_id: &str,
    task_id: &str,
) -> Result<Option<IssueAgentAsk>> {
    let Some(current) = observation::load(adapter, workspace, work_order_id)? else {
        return Ok(None);
    };
    current.observation.validate().map_err(anyhow::Error::msg)?;
    // A waiting question already ended its turn; only a running turn can ask.
    if current.observation.active_task_id.as_deref() != Some(task_id)
        || current.observation.question.is_some()
    {
        return Ok(None);
    }
    let Some(lease) = running_lease(&current, task_id) else {
        return Ok(None);
    };
    let Some(thread_id) = from_current_lease(&current, lease)? else {
        return Ok(None);
    };
    Ok(lease
        .lease_generation
        .clone()
        .map(|lease_generation| IssueAgentAsk {
            task_id: task_id.to_owned(),
            thread_id,
            lease_generation,
        }))
}

pub(super) fn await_question(
    adapter: &AkraIssueWorkExecutionAdapter,
    workspace: &str,
    work_order_id: &str,
    ask: &IssueAgentAsk,
) -> Result<bool> {
    let Some(current) = observation::load(adapter, workspace, work_order_id)? else {
        return Ok(false);
    };
    let Some(lease) = running_lease(&current, &ask.task_id)
        .filter(|lease| lease.lease_generation.as_deref() == Some(ask.lease_generation.as_str()))
    else {
        return Ok(false);
    };
    adapter
        .authority
        .mark_worker_awaiting_question(workspace, lease)
}

fn running_lease<'a>(
    current: &'a ObservedExecution,
    task_id: &str,
) -> Option<&'a ParallelModeSlotLeaseSnapshot> {
    let mut leases = current
        .runtime
        .slot_leases
        .values()
        .filter(|lease| lease.task_id == task_id);
    let lease = leases.next()?;
    (leases.next().is_none() && lease.state == ParallelModeSlotLeaseState::Running).then_some(lease)
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
    if let Some(question) = &current.observation.question
        && (current.active_task_status != Some(TaskStatus::AwaitingUser)
            || question.task_id != lease.task_id
            || question.session_key != session_key
            || question.slot_id != lease.slot_id
            || question.worktree_path != lease.worktree_path
            || question.branch_name != lease.branch_name
            || lease.lease_generation.as_deref() != Some(question.lease_generation.as_str())
            || detail.thread_id.as_deref() != Some(question.thread_id.as_str()))
    {
        bail!("the current work question belongs to another session");
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
