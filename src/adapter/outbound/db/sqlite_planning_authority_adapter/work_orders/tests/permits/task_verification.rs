use super::*;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkControlAction, PlanningWorkControlRepositoryPort,
};
use crate::application::port::outbound::task_evidence_repository_port::*;
use crate::application::port::outbound::task_verification_repository_port::*;
use crate::domain::planning::evidence::*;
use crate::domain::planning::verification::*;
use crate::domain::planning::{
    TASK_AUTHORITY_FORMAT_VERSION, TaskAcceptanceChannel, TaskAcceptanceCriterion,
};

const NOW: &str = "2026-11-23T12:00:00Z";

fn prepared(
    label: &str,
) -> (
    String,
    SqlitePlanningAuthorityAdapter,
    TaskVerificationClaimCommit,
) {
    let (workspace, adapter) = fixture(label);
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
        scenario: "Check the frozen diff for whitespace errors".into(),
        channel: TaskAcceptanceChannel::Command,
        essential: true,
        verification_recipe: Some(TaskVerificationRecipe::GitDiffCheckV1),
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
    let scope = adapter
        .load_current_snapshot(
            &workspace,
            &TaskEvidenceSnapshotQuery {
                task_ids: vec!["controlled-task".into()],
                after_task_id: None,
                limit: 1,
            },
        )
        .unwrap()
        .tasks
        .remove(0)
        .scope;
    let owner = "e".repeat(64);
    adapter
        .recover_incomplete_host_verifications(&workspace, &owner, NOW)
        .unwrap();
    let request = TaskVerificationClaimCommit {
        request_id: "before-control".into(),
        task_id: "controlled-task".into(),
        criterion_id: "C1".into(),
        scope_id: scope.scope_id,
        expected_acceptance_revision: scope.acceptance_revision,
        owner_token: owner,
        at: NOW.into(),
    };
    (workspace, adapter, request)
}

fn manifest() -> TaskVerificationExecutionManifest {
    TaskVerificationExecutionManifest {
        recipe: TaskVerificationRecipe::GitDiffCheckV1,
        base_commit_sha: "a".repeat(40),
        base_tree_sha: "b".repeat(40),
        source: TaskEvidenceTree {
            head_commit_sha: "c".repeat(40),
            tree_sha: "d".repeat(40),
            worktree_clean: false,
        },
        git_binary_sha256: "a".repeat(64),
        command_sha256: "b".repeat(64),
        environment_sha256: "c".repeat(64),
        configuration_sha256: "d".repeat(64),
    }
}

fn actions() -> [PlanningWorkControlAction; 5] {
    [
        PlanningWorkControlAction::Hold,
        PlanningWorkControlAction::TakeOver,
        PlanningWorkControlAction::Cancel,
        PlanningWorkControlAction::Untrack,
        PlanningWorkControlAction::Retriage,
    ]
}

#[test]
fn host_verification_work_control_fences_claim_and_bind_but_preserves_not_run_facts() {
    for action in actions() {
        let (workspace, adapter, request) = prepared(&format!("host-control-pending-{action:?}"));
        let claim = adapter
            .claim_host_verification(&workspace, &request)
            .unwrap();
        let execution = claim.execution.unwrap();
        let control = adapter
            .apply_work_order_control(&workspace, "order-1", "fence-host", action)
            .unwrap();
        assert!(control.authorization_epoch > 0);
        let bind = TaskVerificationBindCommit {
            execution: execution.clone(),
            manifest: manifest(),
            at: NOW.into(),
        };
        assert_eq!(
            adapter
                .bind_host_verification(&workspace, &bind)
                .unwrap_err()
                .downcast_ref::<TaskEvidenceError>(),
            Some(&TaskEvidenceError::LeaseChanged),
            "{action:?}"
        );
        let finish = TaskVerificationFinishCommit {
            execution,
            result: TaskVerificationResult::NotRun {
                reason: TaskVerificationNotRunReason::Cancelled,
            },
            artifact: None,
            at: NOW.into(),
        };
        let saved = adapter
            .finish_host_verification(&workspace, &finish)
            .unwrap();
        assert!(!saved.value.passed());
        assert_eq!(saved.value.attempt, claim.stored.attempt);
        assert_eq!(
            adapter.pending_events(&workspace, 10).unwrap(),
            vec![TaskEvidenceEvent::VerificationFinished {
                receipt: Box::new(saved.value)
            }]
        );
        let fresh = TaskVerificationClaimCommit {
            request_id: "after-control".into(),
            ..request
        };
        assert_eq!(
            adapter
                .claim_host_verification(&workspace, &fresh)
                .unwrap_err()
                .downcast_ref::<TaskEvidenceError>(),
            Some(&TaskEvidenceError::LeaseChanged),
            "{action:?}"
        );
    }
}

#[test]
fn host_verification_late_execution_fact_does_not_restore_revoked_dispatch() {
    for action in actions() {
        let (workspace, adapter, request) = prepared(&format!("host-control-running-{action:?}"));
        let claim = adapter
            .claim_host_verification(&workspace, &request)
            .unwrap();
        let execution = claim.execution.unwrap();
        let bind = TaskVerificationBindCommit {
            execution: execution.clone(),
            manifest: manifest(),
            at: NOW.into(),
        };
        assert!(adapter.bind_host_verification(&workspace, &bind).unwrap());
        adapter
            .apply_work_order_control(&workspace, "order-1", "fence-running-host", action)
            .unwrap();
        let empty = || TaskVerificationStream {
            sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into(),
            size_bytes: 0,
            complete: true,
        };
        let finish = TaskVerificationFinishCommit {
            execution,
            result: TaskVerificationResult::Executed {
                observation: Box::new(TaskVerificationObservation {
                    outcome: TaskVerificationExecutionOutcome::Exited { exit_code: 0 },
                    started_at: NOW.into(),
                    finished_at: NOW.into(),
                    duration_ms: 1,
                    stdout: empty(),
                    stderr: empty(),
                }),
            },
            artifact: Some(TaskEvidenceArtifact {
                sha256: "f".repeat(64),
                size_bytes: 123,
            }),
            at: NOW.into(),
        };
        let saved = adapter
            .finish_host_verification(&workspace, &finish)
            .unwrap();
        assert_eq!(saved.value.attempt, claim.stored.attempt);
        assert!(matches!(
            saved.value.result,
            TaskVerificationResult::Executed { .. }
        ));
        assert!(matches!(
            check(
                &adapter,
                &workspace,
                PlanningWorkPermitStage::Dispatch,
                None
            ),
            PlanningWorkPermitDecision::Denied(_)
        ));
        let replay = adapter
            .finish_host_verification(&workspace, &finish)
            .unwrap();
        assert!(!replay.changed);
        assert_eq!(replay.value, saved.value);
        assert_eq!(adapter.pending_events(&workspace, 10).unwrap().len(), 1);
    }
}

#[test]
fn host_verification_claim_and_bind_require_dispatch_for_the_frozen_lease_base() {
    for claimed_before_source_change in [false, true] {
        let (workspace, adapter, request) = prepared(&format!(
            "host-permit-source-{claimed_before_source_change}"
        ));
        let claim = claimed_before_source_change.then(|| {
            adapter
                .claim_host_verification(&workspace, &request)
                .unwrap()
        });
        let mut changed_basis = basis(2);
        changed_basis.source_sha = "e".repeat(40);
        adapter
            .set_work_order_permit_basis(&workspace, "order-1", &changed_basis)
            .unwrap();
        adapter
            .grant_work_order_permit(
                &workspace,
                &PlanningWorkPermit {
                    basis: changed_basis,
                    ..permit(PlanningWorkPermitStage::Dispatch, 2)
                },
            )
            .unwrap();
        assert!(
            matches!(
                check(
                    &adapter,
                    &workspace,
                    PlanningWorkPermitStage::Dispatch,
                    None
                ),
                PlanningWorkPermitDecision::Granted { .. }
            ),
            "the grant exists; only its source differs from the frozen lease"
        );
        assert!(matches!(
            check(
                &adapter,
                &workspace,
                PlanningWorkPermitStage::Dispatch,
                Some(&"a".repeat(40))
            ),
            PlanningWorkPermitDecision::Denied(_)
        ));
        if let Some(claim) = claim {
            let bind = TaskVerificationBindCommit {
                execution: claim.execution.unwrap(),
                manifest: manifest(),
                at: NOW.into(),
            };
            assert_eq!(
                adapter
                    .bind_host_verification(&workspace, &bind)
                    .unwrap_err()
                    .downcast_ref::<TaskEvidenceError>(),
                Some(&TaskEvidenceError::LeaseChanged),
            );
            let stored = adapter
                .load_host_verification(&workspace, &claim.stored.attempt.attempt_id)
                .unwrap()
                .unwrap();
            assert_eq!(stored.phase, TaskVerificationPhase::Pending);
            assert!(stored.manifest.is_none());
            assert_eq!(count(&workspace, "task_verification_runs"), 1);
        } else {
            assert_eq!(
                adapter
                    .claim_host_verification(&workspace, &request)
                    .unwrap_err()
                    .downcast_ref::<TaskEvidenceError>(),
                Some(&TaskEvidenceError::LeaseChanged),
            );
            assert_eq!(count(&workspace, "task_verification_runs"), 0);
        }
        assert!(adapter.pending_events(&workspace, 10).unwrap().is_empty());
    }
}
