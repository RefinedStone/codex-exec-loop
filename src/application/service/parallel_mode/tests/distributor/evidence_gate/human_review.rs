use super::*;
use crate::application::service::task_evidence::preview::{PreviewKind, PreviewTarget};
use crate::domain::planning::evidence::{HumanEvidenceDecision, TaskCriterionEvidenceState};

#[test]
fn manual_review_binds_the_displayed_artifact_and_tree_before_enforce_releases_push() {
    let f = Fixture::with_setup(
        "a01-reviewed-screen",
        TaskEvidenceDeliveryMode::Enforce,
        FakeGithubAutomationPort::ready(),
        |repo, authority| {
            let ws = repo.workspace_dir();
            seed_task_with_commit(&ws, authority, true, |doc, projection| {
                let mut doc = doc.clone();
                doc.tasks[0].acceptance[0].channel = TaskAcceptanceChannel::Screen;
                doc.tasks[0].acceptance[0].verification_recipe = None;
                authority
                    .commit_task_authority_snapshot(
                        &ws,
                        PlanningTaskAuthorityCommit {
                            observed_planning_revision: None,
                            task_authority: &doc,
                            queue_projection: projection,
                        },
                    )
                    .unwrap();
            })
        },
        |s| s,
    );
    f.record_worker(TaskEvidenceStatus::Pass);
    let ws = f.repo.workspace_dir();
    let receipt = f
        .authority
        .load_current_snapshot(&ws, &TaskEvidenceSnapshotQuery::default())
        .unwrap()
        .tasks
        .remove(0)
        .evidence
        .remove(0);
    let scope = f.scope();
    let target = PreviewTarget {
        task_id: TASK.into(),
        criterion_id: "C1".into(),
        scope_id: scope.scope_id,
        revision: scope.acceptance_revision,
        event_id: receipt.event_id,
        sha256: receipt.evidence.artifact.sha256,
        kind: PreviewKind::Worker,
    };
    let state = || {
        crate::composition::task_result::query(&ws, TASK)
            .unwrap()
            .evidence
            .tasks[0]
            .criteria[0]
            .state
    };
    assert!(
        f.evidence
            .review_evidence(&target, HumanEvidenceDecision::Pass, |_| false)
            .is_err()
    );
    assert_eq!(state(), Some(TaskCriterionEvidenceState::Pass));
    for (decision, expected) in [
        (
            HumanEvidenceDecision::Fail,
            TaskCriterionEvidenceState::HumanFailed,
        ),
        (
            HumanEvidenceDecision::Pass,
            TaskCriterionEvidenceState::HumanPassed,
        ),
    ] {
        // Synthetic callback, never a real user's approval or an OS automation action.
        f.evidence
            .review_evidence(&target, decision, |display| {
                display.contains("worker report: no host execution authority")
            })
            .unwrap();
        assert_eq!(state(), Some(expected));
    }
    let path = Path::new(&f.lease.worktree_path).join("feature.txt");
    let original = fs::read(&path).unwrap();
    fs::write(&path, "new source\n").unwrap();
    assert_eq!(state(), Some(TaskCriterionEvidenceState::Stale));
    fs::write(&path, original).unwrap();
    f.enqueue();
    f.process();
    let queue = f.queue();
    assert_eq!(queue.queue_state, ParallelModeQueueItemState::Done);
    assert!(
        queue
            .evidence_observations
            .iter()
            .all(|o| o.criteria[0].state == TaskDeliveryCriterionState::HumanPassed)
    );
    assert_eq!(
        f.remote_ref(POOL_BASELINE_BRANCH),
        queue.integration_commit_sha
    );
}
