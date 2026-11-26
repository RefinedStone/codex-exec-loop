//! Query the existing task authority, durable kernel usage and delegation ledger.
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::db::sqlite_delegation_store::DelegationStore;
use crate::application::port::outbound::delegation_store_port::DelegationStorePort;
use crate::application::port::outbound::kernel_event_log_port::KernelEventLogPort;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::domain::planning::evidence::validate_evidence_identifier;
use crate::domain::task_usage::{TaskUsageExecution, TaskUsageState, TaskUsageStatus, UsageGap};
use anyhow::{Result, ensure};

pub(crate) fn validate_task(workspace: &str, task_id: &str) -> Result<()> {
    validate_evidence_identifier(task_id).map_err(anyhow::Error::msg)?;
    let snapshot = super::planning_task_list::query(workspace)?;
    ensure!(snapshot.available, "task authority is unavailable");
    ensure!(
        snapshot.tasks.iter().any(|task| task.task_id == task_id),
        "task does not exist"
    );
    Ok(())
}

pub(crate) fn query(workspace: &str, task_id: &str) -> Result<TaskUsageState> {
    validate_evidence_identifier(task_id).map_err(anyhow::Error::msg)?;
    let mut state = TaskUsageState::unavailable(task_id.to_owned());
    let snapshot = super::planning_task_list::query(workspace)?;
    if !snapshot.available {
        return Ok(state);
    }
    if !snapshot.tasks.iter().any(|task| task.task_id == task_id) {
        state.status = TaskUsageStatus::Missing;
        return Ok(state);
    }
    state.status = TaskUsageStatus::Available;
    let store = SqlitePlanningAuthorityAdapter::new();
    let runtime = store.load_runtime_projections(workspace).ok();
    state.runtime_available = runtime.is_some();
    match store.load_task_usage(workspace, task_id) {
        Ok(Some(projection)) => {
            state.usage_available = true;
            state.truncated |= projection.truncated;
            for link in &projection.links {
                if !projection
                    .turns
                    .iter()
                    .any(|turn| turn.thread_id == link.thread_id && turn.op_id == Some(link.op_id))
                {
                    state.executions.push(TaskUsageExecution {
                        execution_id: format!("operation/{}", link.op_id),
                        kind: "kernel_operation".into(),
                        thread_id: Some(link.thread_id.clone()),
                        turn_id: None,
                        op_id: Some(link.op_id),
                        observed_tokens: None,
                        gaps: vec![UsageGap::UsageNotObserved],
                    });
                }
            }
            state
                .executions
                .extend(projection.turns.into_iter().map(|turn| turn.execution()));
        }
        _ => state.gaps.push(UsageGap::SourceUnavailable),
    }
    if let Some(runtime) = runtime {
        for session in runtime
            .session_details
            .iter()
            .filter(|session| session.task_id == task_id)
        {
            state.executions.push(TaskUsageExecution {
                execution_id: session.session_key.clone(),
                kind: "parallel_session".into(),
                thread_id: session.thread_id.clone(),
                turn_id: None,
                op_id: None,
                observed_tokens: None,
                gaps: vec![UsageGap::UsageNotObserved],
            });
        }
    } else {
        state.gaps.push(UsageGap::SourceUnavailable);
    }
    if let Ok(Some(location)) =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_for_read(workspace)
    {
        let delegations = DelegationStore::new(location.runtime_dir.into());
        if let Ok(projection) = delegations.task_usage(task_id) {
            state.delegations_available = true;
            state.unlinked_repository_delegations = projection.unlinked;
            state.truncated |= projection.truncated;
            for delegation in projection.delegations {
                let ids = if delegation.attempts.is_empty() {
                    vec![delegation.id]
                } else {
                    delegation
                        .attempts
                        .into_iter()
                        .map(|attempt| format!("{}/{}", delegation.id, attempt.id))
                        .collect()
                };
                state
                    .executions
                    .extend(ids.into_iter().map(|id| TaskUsageExecution {
                        execution_id: id,
                        kind: "delegation".into(),
                        thread_id: None,
                        turn_id: None,
                        op_id: None,
                        observed_tokens: None,
                        gaps: vec![UsageGap::BackendUsageUnavailable],
                    }));
            }
        }
    }
    if !state.delegations_available {
        state.gaps.push(UsageGap::SourceUnavailable);
    }
    if state.unlinked_repository_delegations > 0 {
        state.gaps.push(UsageGap::TaskLinkMissing);
    }
    if state.executions.is_empty() {
        state.gaps.push(UsageGap::UsageNotObserved);
    }
    state.sum_observations();
    Ok(state)
}
