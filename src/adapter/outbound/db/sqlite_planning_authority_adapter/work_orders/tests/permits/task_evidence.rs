use super::*;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkControlAction, PlanningWorkControlRepositoryPort,
};
use crate::application::port::outbound::task_evidence_repository_port::*;
use crate::domain::planning::evidence::*;
use crate::domain::planning::{
    TASK_AUTHORITY_FORMAT_VERSION, TaskAcceptanceChannel, TaskAcceptanceCriterion,
};

#[test]
fn task_evidence_work_control_changes_close_new_records_in_the_same_planning_transaction() {
    for action in [
        PlanningWorkControlAction::Hold,
        PlanningWorkControlAction::TakeOver,
        PlanningWorkControlAction::Cancel,
        PlanningWorkControlAction::Untrack,
        PlanningWorkControlAction::Retriage,
    ] {
        let (workspace, adapter) = fixture(&format!("evidence-control-{action:?}"));
        let snapshot = adapter
            .load_task_authority_snapshot(&workspace)
            .unwrap()
            .unwrap();
        let mut document = snapshot.task_authority;
        document.version = TASK_AUTHORITY_FORMAT_VERSION;
        let task = document
            .tasks
            .iter_mut()
            .find(|task| task.id == "controlled-task")
            .unwrap();
        task.acceptance = vec![TaskAcceptanceCriterion {
            id: "C1".into(),
            scenario: "The verification result is observable".into(),
            channel: TaskAcceptanceChannel::Command,
            essential: true,
        }];
        task.acceptance_revision = 1;
        adapter
            .commit_task_authority_snapshot(
                &workspace,
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: Some(snapshot.planning_revision),
                    task_authority: &document,
                    queue_projection: &super::super::projection(&document),
                },
            )
            .unwrap();
        grant_all(&adapter, &workspace, 1);
        let mut running = lease("controlled-task", "slot-1");
        running.state = ParallelModeSlotLeaseState::Running;
        adapter
            .upsert_runtime_slot_lease(&workspace, &running)
            .unwrap();
        let snapshot = adapter
            .load_current_snapshot(
                &workspace,
                &TaskEvidenceSnapshotQuery {
                    task_ids: vec!["controlled-task".into()],
                    after_task_id: None,
                    limit: 1,
                },
            )
            .unwrap();
        let attempt = adapter
            .prepare_attempt(
                &workspace,
                &TaskEvidencePrepareCommit {
                    request_id: "before-control".into(),
                    scope: snapshot.tasks[0].scope.clone(),
                    criterion_id: "C1".into(),
                    lease: running.clone(),
                    prepared_tree: TaskEvidenceTree {
                        head_commit_sha: "a".repeat(40),
                        tree_sha: "b".repeat(40),
                        worktree_clean: false,
                    },
                    at: "2026-11-23T12:00:00Z".into(),
                },
            )
            .unwrap()
            .value;
        let control = adapter
            .apply_work_order_control(&workspace, "order-1", "fence-evidence", action)
            .unwrap();
        assert!(control.authorization_epoch > 0);
        let result = adapter.record_evidence(
            &workspace,
            &TaskEvidenceRecordCommit {
                attempt: attempt.clone(),
                lease: running,
                status: TaskEvidenceStatus::Pass,
                artifact: TaskEvidenceArtifact {
                    sha256: "c".repeat(64),
                    size_bytes: 1,
                },
                recorded_tree: attempt.prepared_tree.clone(),
                note: None,
                by: "worker".into(),
                at: "2026-11-23T12:00:01Z".into(),
            },
        );
        assert_eq!(
            result.unwrap_err().downcast_ref::<TaskEvidenceError>(),
            Some(&TaskEvidenceError::LeaseChanged),
            "{action:?}"
        );
        assert!(
            adapter
                .load_attempt(&workspace, &attempt.attempt_id)
                .unwrap()
                .unwrap()
                .receipt
                .is_none()
        );
        assert!(adapter.pending_events(&workspace, 10).unwrap().is_empty());
    }
}
