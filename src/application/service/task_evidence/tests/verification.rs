use super::*;
use crate::application::port::inbound::task_evidence_query_port::{
    TaskEvidencePlanningSnapshot, TaskVerificationProjection, TaskVerificationReceiptProjection,
};
use crate::application::port::outbound::task_verification_repository_port::{
    StoredTaskVerification, TaskVerificationRepositoryPort,
};
use crate::application::port::outbound::task_verification_runner_port::TaskVerificationRunnerPort;
use crate::domain::planning::verification::{
    TaskVerificationCurrentState, TaskVerificationExecutionManifest,
    TaskVerificationExecutionOutcome, TaskVerificationNotRunReason, TaskVerificationPhase,
    TaskVerificationRecipe, TaskVerificationResult,
};

mod fixtures;
use fixtures::*;
mod running;

fn host(snapshot: &TaskEvidencePlanningSnapshot) -> &TaskVerificationProjection {
    snapshot.tasks[0].criteria[0]
        .host_verification
        .as_ref()
        .unwrap()
}

fn expected_receipt(fixture: &HostFixture) -> Option<TaskVerificationReceiptProjection> {
    fixture.base.repository.snapshot.lock().unwrap().tasks[0].host_verifications[0]
        .receipt
        .as_ref()
        .map(Into::into)
}

fn fail_execution(stored: &mut StoredTaskVerification) {
    let TaskVerificationResult::Executed { observation } =
        &mut stored.receipt.as_mut().unwrap().result
    else {
        unreachable!()
    };
    observation.outcome = TaskVerificationExecutionOutcome::Exited { exit_code: 1 };
}

#[test]
fn a_worker_reported_pass_is_not_host_execution_and_an_unrun_recipe_is_pending() {
    let reported = Fixture::new();
    reported.install_record(TaskEvidenceStatus::Pass);
    let snapshot = reported
        .service
        .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
        .unwrap();
    assert_eq!(
        snapshot.tasks[0].criteria[0].state,
        Some(TaskCriterionEvidenceState::Pass)
    );
    assert!(snapshot.tasks[0].criteria[0].host_verification.is_none());

    let empty = HostFixture::empty();
    let snapshot = empty.query();
    assert_eq!(
        snapshot.tasks[0].criteria[0].state,
        Some(TaskCriterionEvidenceState::Pending)
    );
    assert_eq!(host(&snapshot).state, TaskVerificationCurrentState::Pending);
    assert_eq!(host(&snapshot).phase, TaskVerificationPhase::Pending);
    assert!(host(&snapshot).attempt.is_none());
    assert!(host(&snapshot).receipt.is_none());
    assert_eq!(empty.base.git.reads.load(Ordering::SeqCst), 0);
    assert_eq!(empty.artifacts.reads.load(Ordering::SeqCst), 0);
    assert_eq!(empty.runner.reads.load(Ordering::SeqCst), 0);

    let reserved = HostFixture::new();
    reserved.install_host(TaskVerificationPhase::Pending);
    let snapshot = reserved.query();
    assert_eq!(
        snapshot.tasks[0].criteria[0].state,
        Some(TaskCriterionEvidenceState::Pass)
    );
    assert_eq!(host(&snapshot).state, TaskVerificationCurrentState::Pending);
    assert!(host(&snapshot).attempt.is_some());
    assert!(host(&snapshot).receipt.is_none());
}

#[test]
fn sealed_host_execution_is_current_with_the_same_tree_after_head_changes() {
    let fixture = HostFixture::new();
    fixture.base.install_record(TaskEvidenceStatus::Fail);
    let stored = fixture.install_host(TaskVerificationPhase::Finished);
    let snapshot = fixture.query();
    assert_eq!(
        snapshot.tasks[0].criteria[0].state,
        Some(TaskCriterionEvidenceState::Fail)
    );
    assert_eq!(host(&snapshot).state, TaskVerificationCurrentState::Passed);
    assert_eq!(host(&snapshot).receipt, expected_receipt(&fixture));
    assert_eq!(
        snapshot.tasks[0]
            .source
            .as_ref()
            .unwrap()
            .tree
            .head_commit_sha,
        "b".repeat(40)
    );
    assert_eq!(
        stored.manifest.unwrap().source.head_commit_sha,
        "a".repeat(40)
    );
    assert_eq!(fixture.artifacts.reads.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.runner.reads.load(Ordering::SeqCst), 2);

    fixture.edit_host(fail_execution);
    let snapshot = fixture.query();
    assert_eq!(host(&snapshot).state, TaskVerificationCurrentState::Failed);
    assert_eq!(host(&snapshot).receipt, expected_receipt(&fixture));
}

#[test]
fn incomplete_output_is_a_failed_execution_and_missing_capture_is_unavailable() {
    let partial = HostFixture::new();
    partial.install_host(TaskVerificationPhase::Finished);
    partial.edit_host(|stored| {
        let TaskVerificationResult::Executed { observation } =
            &mut stored.receipt.as_mut().unwrap().result
        else {
            unreachable!()
        };
        observation.stdout.complete = false;
    });
    assert_eq!(
        host(&partial.query()).state,
        TaskVerificationCurrentState::Failed
    );

    for missing_metadata in [false, true] {
        let fixture = HostFixture::new();
        fixture.install_host(TaskVerificationPhase::Finished);
        if missing_metadata {
            fixture.edit_host(|stored| stored.receipt.as_mut().unwrap().artifact = None);
        } else {
            fixture.artifacts.available.store(false, Ordering::SeqCst);
        }
        let snapshot = fixture.query();
        assert_eq!(
            host(&snapshot).state,
            TaskVerificationCurrentState::Unavailable
        );
        assert_eq!(
            host(&snapshot).unavailable,
            Some(if missing_metadata {
                TaskEvidenceError::Unavailable
            } else {
                TaskEvidenceError::ArtifactMissing
            })
        );
        assert_eq!(host(&snapshot).receipt, expected_receipt(&fixture));
    }
}

#[test]
fn not_run_and_restart_interruption_preserve_facts_without_manufacturing_success() {
    for result in [
        TaskVerificationResult::NotRun {
            reason: TaskVerificationNotRunReason::InvalidInput,
        },
        TaskVerificationResult::Interrupted {},
    ] {
        let fixture = HostFixture::new();
        fixture.install_host(TaskVerificationPhase::Finished);
        fixture.edit_host(|stored| {
            stored.manifest = None;
            let receipt = stored.receipt.as_mut().unwrap();
            receipt.manifest = None;
            receipt.artifact = None;
            receipt.result = result.clone();
            receipt.validate().unwrap();
        });
        let snapshot = fixture.query();
        assert_eq!(host(&snapshot).state, TaskVerificationCurrentState::NotRun);
        assert_eq!(host(&snapshot).receipt.as_ref().unwrap().result, result);
        assert_eq!(fixture.runner.reads.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.artifacts.reads.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn changed_scope_recipe_base_session_tree_or_environment_keeps_the_original_fact_stale() {
    for changed in ["scope", "recipe", "base", "session", "tree", "environment"] {
        let fixture = HostFixture::new();
        fixture.install_host(TaskVerificationPhase::Finished);
        let original = expected_receipt(&fixture);
        match changed {
            "scope" => {
                fixture.base.repository.snapshot.lock().unwrap().tasks[0]
                    .scope
                    .scope_id = "9".repeat(64);
            }
            "recipe" => {
                let mut snapshot = fixture.base.repository.snapshot.lock().unwrap();
                let task = &mut snapshot.tasks[0];
                task.task.acceptance[0].verification_recipe = None;
                task.task.acceptance_revision += 1;
                task.scope.scope_id = "9".repeat(64);
                task.scope.acceptance_revision = task.task.acceptance_revision;
                task.scope.criteria_sha256 = criteria_sha256(&task.task.acceptance).unwrap();
            }
            "base" => {
                fixture.base.repository.snapshot.lock().unwrap().tasks[0].queue_records[0]
                    .source_base_commit_sha = "f".repeat(40);
            }
            "session" => {
                fixture.base.repository.snapshot.lock().unwrap().tasks[0].queue_records[0]
                    .session_key = "replacement-session".into();
            }
            "tree" => *fixture.base.git.tree.lock().unwrap() = "d".repeat(40),
            "environment" => fixture.runner.set(Ok(ENVIRONMENT_B.into())),
            _ => unreachable!(),
        }
        let snapshot = fixture.query();
        assert_eq!(
            host(&snapshot).state,
            TaskVerificationCurrentState::Stale,
            "{changed}"
        );
        assert_eq!(host(&snapshot).unavailable, None, "{changed}");
        assert_eq!(host(&snapshot).receipt, original, "{changed}");
    }
}

#[test]
fn one_unavailable_task_observation_clears_passed_failed_and_running_host_states() {
    for initial in [
        TaskVerificationCurrentState::Passed,
        TaskVerificationCurrentState::Failed,
        TaskVerificationCurrentState::Running,
    ] {
        let fixture = HostFixture::new();
        let phase = if initial == TaskVerificationCurrentState::Running {
            TaskVerificationPhase::Running
        } else {
            TaskVerificationPhase::Finished
        };
        fixture.install_host(phase);
        if initial == TaskVerificationCurrentState::Failed {
            fixture.edit_host(fail_execution);
        }
        fixture
            .base
            .artifacts
            .available
            .store(false, Ordering::SeqCst);
        let snapshot = fixture.query();
        assert_eq!(
            snapshot.tasks[0].unavailable,
            Some(TaskEvidenceError::ArtifactMissing)
        );
        assert_eq!(snapshot.tasks[0].criteria[0].state, None);
        assert_eq!(
            host(&snapshot).state,
            TaskVerificationCurrentState::Unavailable,
            "{initial:?}"
        );
        assert_eq!(
            host(&snapshot).unavailable,
            Some(TaskEvidenceError::ArtifactMissing)
        );
        assert_eq!(host(&snapshot).receipt, expected_receipt(&fixture));
        assert!(host(&snapshot).attempt.is_some());
    }
}

#[test]
fn one_page_cannot_mix_passed_receipts_from_two_environments_of_the_same_recipe() {
    for changed_before_return in [false, true] {
        let fixture = HostFixture::new();
        fixture.add_second_criterion();
        fixture.install_host(TaskVerificationPhase::Finished);
        fixture.add_second_host(ENVIRONMENT_B);
        if changed_before_return {
            // A naive per-criterion read could accept A then B as two simultaneous successes.
            fixture
                .runner
                .sequence(vec![Ok(ENVIRONMENT_A.into()), Ok(ENVIRONMENT_B.into())]);
        }
        let snapshot = fixture.query();
        let states: Vec<_> = snapshot.tasks[0]
            .criteria
            .iter()
            .map(|criterion| criterion.host_verification.as_ref().unwrap().state)
            .collect();
        assert_eq!(
            states,
            vec![
                if changed_before_return {
                    TaskVerificationCurrentState::Stale
                } else {
                    TaskVerificationCurrentState::Passed
                },
                TaskVerificationCurrentState::Stale,
            ]
        );
        assert_eq!(
            fixture.runner.reads.load(Ordering::SeqCst),
            2,
            "one initial and one final observation per recipe/object format"
        );
        assert_eq!(
            snapshot.tasks[0].criteria[1]
                .host_verification
                .as_ref()
                .unwrap()
                .receipt
                .as_ref()
                .unwrap()
                .manifest
                .as_ref()
                .unwrap()
                .environment_sha256,
            ENVIRONMENT_B
        );
    }
}

#[test]
fn the_final_environment_fence_runs_after_the_second_database_read_for_all_current_states() {
    for initial in [
        TaskVerificationCurrentState::Passed,
        TaskVerificationCurrentState::Failed,
        TaskVerificationCurrentState::Running,
    ] {
        for unavailable in [false, true] {
            let fixture = HostFixture::new();
            fixture.install_host(if initial == TaskVerificationCurrentState::Running {
                TaskVerificationPhase::Running
            } else {
                TaskVerificationPhase::Finished
            });
            if initial == TaskVerificationCurrentState::Failed {
                fixture.edit_host(fail_execution);
            }
            let original = expected_receipt(&fixture);
            let runner = fixture.runner.clone();
            fixture.repository.after_second_read(move || {
                runner.set(if unavailable {
                    Err(TaskVerificationNotRunReason::Unavailable)
                } else {
                    Ok(ENVIRONMENT_B.into())
                })
            });
            let snapshot = fixture.query();
            assert_eq!(snapshot.unavailable, None);
            assert_eq!(snapshot.evidence_revision, Some(2));
            assert_eq!(
                host(&snapshot).state,
                if unavailable {
                    TaskVerificationCurrentState::Unavailable
                } else {
                    TaskVerificationCurrentState::Stale
                },
                "{initial:?}, unavailable={unavailable}"
            );
            assert_eq!(
                host(&snapshot).unavailable,
                unavailable.then_some(TaskEvidenceError::Unavailable)
            );
            assert_eq!(host(&snapshot).receipt, original);
            assert_eq!(fixture.repository.reads.load(Ordering::SeqCst), 2);
            assert_eq!(fixture.runner.reads.load(Ordering::SeqCst), 2);
        }
    }
}

#[test]
fn a_changed_second_authority_snapshot_does_not_return_an_earlier_host_success() {
    let fixture = HostFixture::new();
    fixture.install_host(TaskVerificationPhase::Finished);
    fixture
        .base
        .repository
        .change_on_second_read
        .store(true, Ordering::SeqCst);
    let snapshot = fixture.query();
    assert_eq!(snapshot.unavailable, Some(TaskEvidenceError::Stale));
    assert!(snapshot.tasks.is_empty());
    assert_eq!(fixture.artifacts.reads.load(Ordering::SeqCst), 1);
}
