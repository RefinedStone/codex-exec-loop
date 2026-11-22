//! 영속 작업 계보/lease와 살아 있는 app-server stream을 함께 확인한다.
use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
use crate::application::port::outbound::issue_work_steering_port::{
    IssueWorkSteerRequest, IssueWorkSteeringPort,
};
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityPort, PlanningAuthorityRuntimeProjectionSnapshot,
};
use crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkOrderRepositoryPort;
use crate::domain::conversation::ConversationTurnSteerRequest;
use crate::domain::issue_automation::intervention::IssueSteerDelivery;
use crate::domain::parallel_mode::{ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState};
use anyhow::{Result, bail, ensure};
use std::collections::BTreeSet;
use std::sync::Arc;

pub(crate) struct AkraIssueWorkSteeringAdapter {
    repository: Arc<dyn PlanningWorkOrderRepositoryPort>,
    authority: Arc<dyn PlanningAuthorityPort>,
    runtime: Arc<dyn InteractiveTurnRuntimePort>,
}

impl AkraIssueWorkSteeringAdapter {
    pub(crate) fn new(
        repository: Arc<dyn PlanningWorkOrderRepositoryPort>,
        authority: Arc<dyn PlanningAuthorityPort>,
        runtime: Arc<dyn InteractiveTurnRuntimePort>,
    ) -> Self {
        Self {
            repository,
            authority,
            runtime,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveWorkTarget {
    lease: ParallelModeSlotLeaseSnapshot,
    thread_id: String,
}

fn live_work_target(
    projection: &PlanningAuthorityRuntimeProjectionSnapshot,
    task_ids: &[String],
) -> Result<LiveWorkTarget> {
    let mut targets = Vec::new();
    for lease in projection.slot_leases.values() {
        if !task_ids.contains(&lease.task_id) || lease.state != ParallelModeSlotLeaseState::Running
        {
            continue;
        }
        ensure!(
            !projection.invalid_slot_leases.contains(&lease.slot_id),
            "issue worker ownership is invalid"
        );
        let details = projection
            .session_details
            .iter()
            .filter(|detail| {
                detail.session_key == lease.session_key()
                    && detail.task_id == lease.task_id
                    && detail.slot_id == lease.slot_id
                    && detail.agent_id == lease.agent_id
                    && detail.branch_name == lease.branch_name
                    && detail.worktree_path == lease.worktree_path
                    && detail.lease_started_at == lease.leased_at
            })
            .collect::<Vec<_>>();
        ensure!(
            details.len() == 1,
            "issue worker session does not match its current lease"
        );
        let thread_id = details[0]
            .thread_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("issue worker has no current thread"))?;
        targets.push(LiveWorkTarget {
            lease: lease.clone(),
            thread_id: thread_id.to_owned(),
        });
    }
    ensure!(
        targets.len() == 1,
        "issue work requires exactly one live worker to steer"
    );
    Ok(targets.remove(0))
}

impl IssueWorkSteeringPort for AkraIssueWorkSteeringAdapter {
    fn steer(&self, request: &IssueWorkSteerRequest) -> Result<IssueSteerDelivery> {
        ensure!(
            !request.command_id.trim().is_empty() && !request.message.trim().is_empty(),
            "issue steering requires a command identity and message"
        );
        let work = self
            .repository
            .load_work_order(&request.workspace_dir, &request.work_order_id)?
            .ok_or_else(|| anyhow::anyhow!("issue work order is unavailable"))?;
        let authoritative = work.task_ids.iter().collect::<BTreeSet<_>>();
        let requested = request.task_ids.iter().collect::<BTreeSet<_>>();
        ensure!(
            !authoritative.is_empty()
                && authoritative == requested
                && requested.len() == request.task_ids.len(),
            "issue steering lineage changed"
        );
        let target = live_work_target(
            &self
                .authority
                .load_runtime_projections(&request.workspace_dir)?,
            &work.task_ids,
        )?;
        let turn_id = self
            .runtime
            .active_worker_turn_id(&target.thread_id)?
            .ok_or_else(|| anyhow::anyhow!("the issue worker no longer owns a live turn"))?;
        // 조회 사이의 slot 재사용도 잡는다. 마지막 thread/turn 검증은 stream broker가 수행한다.
        let current = live_work_target(
            &self
                .authority
                .load_runtime_projections(&request.workspace_dir)?,
            &work.task_ids,
        )?;
        ensure!(
            target == current,
            "issue worker ownership changed before steering"
        );
        let latest_work = self
            .repository
            .load_work_order(&request.workspace_dir, &request.work_order_id)?
            .ok_or_else(|| anyhow::anyhow!("issue work order is unavailable before steering"))?;
        ensure!(
            latest_work.task_ids == work.task_ids,
            "issue steering lineage changed before delivery"
        );
        let control = self
            .authority
            .work_order_control_for_task(&request.workspace_dir, &target.lease.task_id)?
            .ok_or_else(|| anyhow::anyhow!("issue work control is unavailable before steering"))?;
        // Hold는 이미 실행 중인 턴의 사람 지시를 허용한다. 취소/인수 이후에는 보내지 않는다.
        ensure!(
            !control.cancelled && !control.taken_over,
            "issue work ownership changed before steering"
        );
        let receipt = self.runtime.steer_turn(ConversationTurnSteerRequest {
            thread_id: target.thread_id,
            expected_turn_id: turn_id.clone(),
            prompt: request.message.clone(),
        })?;
        if receipt.turn_id != turn_id {
            bail!("issue steering returned an unexpected turn receipt");
        }
        Ok(IssueSteerDelivery::Sent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::parallel_mode::{
        ParallelModeAgentSessionDetailSnapshot, ParallelModeLiveSessionDetailDefaults,
    };

    fn fixture() -> PlanningAuthorityRuntimeProjectionSnapshot {
        let lease = ParallelModeSlotLeaseSnapshot::new(
            "slot-a",
            "task-a",
            "title",
            "agent-a",
            "branch-a",
            "/work/a",
            ParallelModeSlotLeaseState::Running,
            "2026-11-21T00:00:00Z",
            None,
        )
        .with_lease_generation("0123456789abcdef0123456789abcdef");
        let mut detail = ParallelModeAgentSessionDetailSnapshot::assigned_for_lease(
            &lease,
            ParallelModeLiveSessionDetailDefaults {
                validation_summary: "pending",
                authority_refresh_outcome: "pending",
            },
        );
        detail.thread_id = Some("thread-a".to_owned());
        let mut projection = PlanningAuthorityRuntimeProjectionSnapshot::default();
        projection.slot_leases.insert(lease.slot_id.clone(), lease);
        projection.session_details.push(detail);
        projection
    }

    #[test]
    fn issue_steer_selects_only_the_current_lineage_session() {
        let projection = fixture();
        let target = live_work_target(&projection, &["task-a".to_owned()]).unwrap();
        assert_eq!(target.thread_id, "thread-a");
        assert!(live_work_target(&projection, &["other-task".to_owned()]).is_err());
    }

    #[test]
    fn issue_steer_rejects_reused_slot_and_stale_detail() {
        let mut projection = fixture();
        projection
            .slot_leases
            .get_mut("slot-a")
            .unwrap()
            .lease_generation = Some("abcdef0123456789abcdef0123456789".to_owned());
        assert!(live_work_target(&projection, &["task-a".to_owned()]).is_err());
        let mut projection = fixture();
        projection.session_details[0].worktree_path = "/other-work".to_owned();
        assert!(live_work_target(&projection, &["task-a".to_owned()]).is_err());
    }

    #[test]
    fn issue_steer_rejects_missing_duplicate_and_invalid_ownership() {
        let mut projection = fixture();
        projection.session_details[0].thread_id = None;
        assert!(live_work_target(&projection, &["task-a".to_owned()]).is_err());
        let mut projection = fixture();
        projection
            .session_details
            .push(projection.session_details[0].clone());
        assert!(live_work_target(&projection, &["task-a".to_owned()]).is_err());
        let mut projection = fixture();
        projection.invalid_slot_leases.insert("slot-a".to_owned());
        assert!(live_work_target(&projection, &["task-a".to_owned()]).is_err());
    }

    #[test]
    fn issue_steer_never_picks_arbitrarily_between_concurrent_lineage_workers() {
        let mut projection = fixture();
        let mut lease = projection.slot_leases["slot-a"].clone();
        lease.slot_id = "slot-b".to_owned();
        lease.task_id = "task-b".to_owned();
        let mut detail = ParallelModeAgentSessionDetailSnapshot::assigned_for_lease(
            &lease,
            ParallelModeLiveSessionDetailDefaults {
                validation_summary: "pending",
                authority_refresh_outcome: "pending",
            },
        );
        detail.thread_id = Some("thread-b".to_owned());
        projection.session_details.push(detail);
        projection.slot_leases.insert(lease.slot_id.clone(), lease);
        assert!(
            live_work_target(&projection, &["task-a".to_owned(), "task-b".to_owned()]).is_err()
        );
    }
}
