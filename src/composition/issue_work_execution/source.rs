//! 현재 작업과 lease generation에만 source를 결속한다. 과거 보완 작업의 SHA를 섞지 않는다.
use super::{AkraIssueWorkExecutionAdapter, observation};
use crate::application::port::outbound::issue_work_execution_port::IssueWorkExecutionSnapshot;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityRuntimeProjectionSnapshot;
use crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkPermitBasis;
use crate::domain::issue_automation::execution::{
    IssueExecutionPhase, IssueWorkExecutionObservation,
};
use crate::domain::planning::TaskStatus;
use anyhow::{Result, anyhow, bail};

pub(super) fn snapshot(
    adapter: &AkraIssueWorkExecutionAdapter,
    workspace: &str,
    work_order_id: &str,
) -> Result<IssueWorkExecutionSnapshot> {
    let observed = observation::load(adapter, workspace, work_order_id)?;
    let basis = adapter
        .repository
        .load_work_order_permit_basis(workspace, work_order_id)?;
    project(observed.as_ref(), basis.as_ref(), || {
        adapter
            .parallel
            .work_order_dispatch_source_sha(workspace)
            .map_err(|_| anyhow!("the configured work dispatch source is unavailable"))
    })
}

fn project(
    observed: Option<&observation::ObservedExecution>,
    basis: Option<&PlanningWorkPermitBasis>,
    fetch_dispatch_source: impl FnOnce() -> Result<String>,
) -> Result<IssueWorkExecutionSnapshot> {
    let frozen_source = observed
        .map(|loaded| frozen_source(&loaded.observation, &loaded.runtime))
        .transpose()?
        .flatten();
    let task_ids = observed
        .map(|loaded| loaded.observation.task_ids.clone())
        .unwrap_or_default();
    // 초기/보완 dispatch 직전에는 저장된 과거 basis가 있어도 현재 remote/base를 재검증한다.
    // 실제 실행 중이거나 전달 source가 고정된 작업은 해당 source를 유지한다.
    let pending_dispatch = observed.is_none_or(|loaded| {
        loaded.observation.phase == IssueExecutionPhase::Queued
            && matches!(
                loaded.active_task_status,
                Some(TaskStatus::Ready | TaskStatus::Proposed)
            )
    });
    // Leased도 배정 완료다. basis 부재가 현재 lease의 source를 교체하게 두지 않는다.
    let current_task_has_lease = observed.is_some_and(|loaded| {
        loaded.runtime.slot_leases.values().any(|lease| {
            Some(lease.task_id.as_str()) == loaded.observation.active_task_id.as_deref()
        })
    });
    let dispatch_source = if frozen_source.is_none()
        && !current_task_has_lease
        && (pending_dispatch || basis.is_none())
    {
        Some(fetch_dispatch_source()?)
    } else {
        None
    };
    Ok(IssueWorkExecutionSnapshot {
        task_ids,
        dispatch_source,
        frozen_source,
        saved_source: basis.map(|basis| basis.source_sha.clone()),
        saved_gate_revision: basis.map(|basis| basis.gate_revision).unwrap_or(0),
        saved_authorization_epoch: basis.map(|basis| basis.authorization_epoch).unwrap_or(0),
        blocked_reason: None,
    })
}

fn frozen_source(
    observation: &IssueWorkExecutionObservation,
    runtime: &PlanningAuthorityRuntimeProjectionSnapshot,
) -> Result<Option<String>> {
    let Some(task_id) = observation.active_task_id.as_deref() else {
        if observation.task_ids.is_empty() {
            return Ok(None);
        }
        bail!("the work has no unambiguous current execution");
    };
    if !observation.task_ids.iter().any(|id| id == task_id) {
        bail!("the current execution does not belong to its work");
    }
    let mut leases = runtime
        .slot_leases
        .values()
        .filter(|lease| lease.task_id == task_id);
    let current_lease = leases.next();
    if leases.next().is_some() {
        bail!("the current execution has ambiguous lease generations");
    }
    let mut sources = Vec::new();
    for record in runtime
        .distributor_queue_records
        .iter()
        .filter(|record| record.task_id == task_id)
    {
        if let Some(lease) = current_lease {
            if record.session_key != lease.session_key() {
                continue;
            }
            if record.slot_id != lease.slot_id
                || record.agent_id != lease.agent_id
                || record.branch_name != lease.branch_name
                || record.worktree_path != lease.worktree_path
            {
                bail!("the frozen source does not match the current lease generation");
            }
        }
        let source = record.effective_source_commit_sha();
        if !source.is_empty() {
            sources.push(source);
        }
    }
    sources.sort();
    sources.dedup();
    if sources.len() > 1 {
        bail!("the current execution has ambiguous frozen source commits");
    }
    Ok(sources.pop())
}

#[cfg(test)]
mod tests;
