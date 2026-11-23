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

#[derive(Debug, PartialEq, Eq)]
struct DeliveryTarget {
    task_id: String,
    session_key: String,
    worktree: String,
    base: String,
    source: String,
}

pub(super) fn frozen_delivery_source(
    adapter: &AkraIssueWorkExecutionAdapter,
    workspace: &str,
    work_order_id: &str,
) -> Result<Option<String>> {
    observation::load(adapter, workspace, work_order_id)?
        .map(|loaded| frozen_source(&loaded.observation, &loaded.runtime))
        .transpose()
        .map(Option::flatten)
}

pub(super) fn delivery_diff(
    adapter: &AkraIssueWorkExecutionAdapter,
    workspace: &str,
    work_order_id: &str,
    expected_source: &str,
) -> Result<crate::domain::issue_automation::delivery::IssueDeliveryDiff> {
    let target = load_delivery_target(adapter, workspace, work_order_id, expected_source)?;
    let diff = adapter
        .parallel
        .inspect_work_order_delivery_diff(&target.worktree, &target.base, &target.source)
        .map_err(|error| anyhow!(error))?;
    if diff.base_commit_sha != target.base
        || diff.source_commit_sha != target.source
        || load_delivery_target(adapter, workspace, work_order_id, expected_source)? != target
    {
        bail!("the frozen delivery range changed during diff inspection");
    }
    Ok(diff)
}

pub(super) fn verify_delivery_context(
    adapter: &AkraIssueWorkExecutionAdapter,
    workspace: &str,
    work_order_id: &str,
    expected_base: &str,
    expected_source: &str,
) -> Result<()> {
    let target = load_delivery_target(adapter, workspace, work_order_id, expected_source)?;
    if target.base != expected_base {
        bail!("the frozen delivery base changed after diff review");
    }
    Ok(())
}

fn load_delivery_target(
    adapter: &AkraIssueWorkExecutionAdapter,
    workspace: &str,
    work_order_id: &str,
    expected_source: &str,
) -> Result<DeliveryTarget> {
    let observed = observation::load(adapter, workspace, work_order_id)?
        .ok_or_else(|| anyhow!("the delivery execution is unavailable"))?;
    select_delivery_target(&observed.observation, &observed.runtime, expected_source)
}

fn select_delivery_target(
    observation: &IssueWorkExecutionObservation,
    runtime: &PlanningAuthorityRuntimeProjectionSnapshot,
    expected_source: &str,
) -> Result<DeliveryTarget> {
    if frozen_source(observation, runtime)?.as_deref() != Some(expected_source) {
        bail!("the delivery source is no longer the current frozen commit");
    }
    let task_id = observation
        .active_task_id
        .as_deref()
        .ok_or_else(|| anyhow!("the delivery has no current execution"))?;
    let lease = runtime
        .slot_leases
        .values()
        .find(|lease| lease.task_id == task_id);
    let mut selected = None;
    for record in runtime
        .distributor_queue_records
        .iter()
        .filter(|record| record.task_id == task_id)
    {
        if lease.is_some_and(|lease| record.session_key != lease.session_key()) {
            continue;
        }
        if record.effective_source_commit_sha() != expected_source
            || record.worktree_path.is_empty()
        {
            bail!("the frozen delivery worktree or source is unavailable");
        }
        crate::domain::issue_automation::delivery::validate_commit_range(
            &record.source_base_commit_sha,
            expected_source,
        )
        .map_err(|error| anyhow!(error))?;
        if let Some(lease) = lease {
            let target = lease
                .delivery_target
                .as_ref()
                .ok_or_else(|| anyhow!("the delivery lease has no frozen integration base"))?;
            if target.integration_base_commit_sha != record.source_base_commit_sha {
                bail!("the delivery base does not match the current lease generation");
            }
        }
        let candidate = DeliveryTarget {
            task_id: task_id.into(),
            session_key: record.session_key.clone(),
            worktree: record.worktree_path.clone(),
            base: record.source_base_commit_sha.clone(),
            source: expected_source.into(),
        };
        if selected
            .as_ref()
            .is_some_and(|previous| previous != &candidate)
        {
            bail!("the current execution has ambiguous frozen delivery ranges");
        }
        selected = Some(candidate);
    }
    selected.ok_or_else(|| anyhow!("the frozen delivery range is unavailable"))
}

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
