use super::*;
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityPort,
};
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkControlAction, PlanningWorkControlRepositoryPort,
};
use crate::application::port::outbound::task_delivery_evidence_repository_port::*;
use crate::domain::planning::delivery_evidence::*;

#[test]
fn delivery_evidence_final_transaction_obeys_current_work_control_and_never_audits_a_denied_effect()
{
    for action in [
        PlanningWorkControlAction::Hold,
        PlanningWorkControlAction::TakeOver,
        PlanningWorkControlAction::Cancel,
        PlanningWorkControlAction::Untrack,
        PlanningWorkControlAction::Retriage,
    ] {
        let (workspace, adapter) = fixture(&format!("delivery-evidence-control-{action:?}"));
        grant_all(&adapter, &workspace, 1);
        let mut running = lease("controlled-task", "slot-1");
        running.state = ParallelModeSlotLeaseState::Running;
        adapter
            .upsert_runtime_slot_lease(&workspace, &running)
            .unwrap();
        let queue: PlanningAuthorityDistributorQueueRecord = serde_json::from_value(serde_json::json!({
            "queue_item_id":"controlled-queue", "session_key":running.session_key(), "slot_id":running.slot_id,
            "agent_id":running.agent_id, "task_id":running.task_id, "task_title":running.task_title,
            "source_branch":running.branch_name,"branch_name":running.branch_name,"worktree_path":running.worktree_path,
            "source_base_commit_sha":"a".repeat(40),"source_commit_sha":"a".repeat(40),"commit_sha":"a".repeat(40),
            "validation_summary":"","authority_refresh_outcome":"","queue_state":"queued","integration_note":"queued",
            "enqueued_at":"2026-11-23T12:00:00Z","updated_at":"2026-11-23T12:00:00Z"
        })).unwrap();
        adapter
            .upsert_runtime_distributor_queue_record(&workspace, &queue)
            .unwrap();
        let queue = adapter
            .load_runtime_projections(&workspace)
            .unwrap()
            .distributor_queue_records
            .remove(0);
        assert!(
            adapter
                .try_acquire_distributor_queue_claim(
                    &workspace,
                    &queue.queue_item_id,
                    "delivery-owner"
                )
                .unwrap()
        );
        let expected = adapter
            .load_delivery_evidence(&workspace, &queue, TaskEvidenceDeliveryStage::Push)
            .unwrap();
        let observation = TaskDeliveryEvidenceObservation {
            stage: TaskEvidenceDeliveryStage::Push,
            mode: TaskEvidenceDeliveryMode::Observe,
            scope: expected.task.as_ref().map(|task| task.scope.clone()),
            evidence_revision: expected.evidence_revision,
            source: None,
            criteria: Vec::new(),
            reasons: vec![TaskDeliveryEvidenceReason::CriteriaMissing],
            observed_at: "2026-11-23T12:00:00Z".into(),
        };
        adapter
            .apply_work_order_control(&workspace, "order-1", "fence-delivery", action)
            .unwrap();
        assert!(
            adapter
                .commit_delivery_evidence_observation(
                    &workspace,
                    &TaskDeliveryEvidenceObservationCommit {
                        expected,
                        operation: TaskDeliveryEvidenceOperation::Effect {
                            owner_token: "delivery-owner".into()
                        },
                        observation,
                    }
                )
                .is_err(),
            "{action:?}"
        );
        let audit_count: i64 = connection(&workspace).query_row(
            "SELECT count(*) FROM runtime_events WHERE event_kind='task_delivery_evidence_observed'", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(audit_count, 0, "{action:?}");
    }
}
