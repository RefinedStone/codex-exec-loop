use super::*;
use crate::adapter::inbound::kernel_protocol::TaskResultReadStatus;
use crate::domain::planning::verification::TaskVerificationCurrentState as Host;

/// All four cases start from the same accepted task and the same local bare remote shape.
/// Worker reports, host observations, PR metadata and the integration ref are independent facts.
#[test]
fn a01_task_result_tracks_pre_push_holds_and_their_recovery() {
    for (expected, reason) in [
        (
            Host::Pending,
            Some(TaskDeliveryEvidenceReason::EvidenceMissing),
        ),
        (
            Host::Failed,
            Some(TaskDeliveryEvidenceReason::CriteriaNotMet),
        ),
        (Host::Stale, Some(TaskDeliveryEvidenceReason::EvidenceStale)),
        (Host::Passed, None),
    ] {
        let fixture = Fixture::new(
            "a01-integrated-evidence",
            TaskEvidenceDeliveryMode::Enforce,
            true,
        );
        let workspace = fixture.repo.workspace_dir();
        let task = fixture.scope().task_id;
        assert_eq!(fixture.lease.task_id, task);
        let path = Path::new(&fixture.lease.worktree_path).join("feature.txt");
        match expected {
            Host::Pending => {}
            Host::Failed => {
                fs::write(&path, "bad whitespace  \n").unwrap();
                fixture.verify_host(false);
            }
            Host::Stale => {
                fixture.verify_host(true);
                fs::write(&path, "changed after observation\n").unwrap();
            }
            Host::Passed => fixture.verify_host(true),
            _ => unreachable!("fixed cases"),
        }
        fixture.record_worker(TaskEvidenceStatus::Pass);
        let query = || crate::composition::task_result::query(&workspace, &task).unwrap();
        let before = query();
        assert_eq!(before.status, TaskResultReadStatus::Available);
        assert_eq!(before.task.as_ref().unwrap().task_id, task);
        let row = &before.evidence.tasks[0].criteria[0];
        assert_eq!(
            row.evidence.as_ref().unwrap().evidence.status,
            TaskEvidenceStatus::Pass
        );
        assert_eq!(row.host_verification.as_ref().unwrap().state, expected);
        fixture.enqueue();
        fixture.process();
        if let Some(reason) = reason {
            fixture.assert_no_source_or_pr();
            fixture.assert_blocked(reason, TaskEvidenceDeliveryStage::Push);
            let held = query();
            assert_eq!(
                held.deliveries[0].evidence_wait,
                Some(TaskEvidenceDeliveryStage::Push)
            );
            // A queued, frozen source has no Running lease on which to execute another check.
            // Use the supported, scoped synthetic waiver, not a new host success or a state edit.
            fixture.waive();
            fixture.process();
        }
        let result = query();
        assert_eq!(result.task_id, task);
        let delivery = &result.deliveries[0];
        assert_eq!(delivery.state, "done");
        assert!(delivery.pull_request_number.is_some());
        assert_eq!(
            fixture.remote_ref(POOL_BASELINE_BRANCH),
            delivery.integration_commit_sha
        );
        assert!(delivery.integration_commit_sha.is_some());
        assert_eq!(
            delivery
                .evidence_observations
                .iter()
                .find(|a| a.stage == TaskEvidenceDeliveryStage::Push)
                .unwrap()
                .criteria[0]
                .state,
            if reason.is_none() {
                TaskDeliveryCriterionState::HostPassed
            } else {
                TaskDeliveryCriterionState::Waived
            }
        );
        assert!(
            result.work_order.is_none(),
            "no external issue or close-on-merge order exists in this fixture"
        );
    }
}
