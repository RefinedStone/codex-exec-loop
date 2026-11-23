use super::*;
use crate::domain::planning::evidence::{TaskEvidenceStatus, criteria_sha256};
use crate::domain::planning::verification::{
    TaskVerificationExecutionOutcome, TaskVerificationNotRunReason, TaskVerificationPhase,
    TaskVerificationResult,
};
use crate::domain::planning::{TaskAcceptanceChannel, TaskStatus};
use std::sync::atomic::Ordering;

mod fences;
mod fixtures;
mod leases;
use fixtures::*;

#[test]
fn current_host_result_can_deliver_a_done_task_after_a_same_tree_host_commit() {
    let fixture = Fixture::new(TaskEvidenceDeliveryMode::Enforce);
    assert_eq!(
        fixture
            .repository
            .snapshot
            .lock()
            .unwrap()
            .task
            .as_ref()
            .unwrap()
            .task
            .status,
        TaskStatus::Done
    );
    assert!(fixture.effect().unwrap().allowed);
    let observed = fixture.last();
    assert!(observed.satisfied());
    assert_eq!(
        observed.criteria[0].state,
        TaskDeliveryCriterionState::HostPassed
    );
    assert_eq!(
        observed.source.as_ref().unwrap().source_commit_sha,
        "e".repeat(40)
    );
    let snapshot = fixture.repository.snapshot.lock().unwrap();
    assert_eq!(
        snapshot.task.as_ref().unwrap().host_verifications[0]
            .manifest
            .as_ref()
            .unwrap()
            .source
            .head_commit_sha,
        "c".repeat(40)
    );
    assert_eq!(fixture.repository.reads.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.runner.reads.load(Ordering::SeqCst), 2);
}

#[test]
fn observe_records_missing_criteria_and_legacy_source_without_adding_an_enforce_bypass() {
    for mode in [
        TaskEvidenceDeliveryMode::Observe,
        TaskEvidenceDeliveryMode::Enforce,
    ] {
        for absent_task in [false, true] {
            let fixture = Fixture::new(mode);
            fixture.change(|snapshot| {
                snapshot.queue.source_commit_sha.clear();
                snapshot.queue.evidence_binding = None;
                if absent_task {
                    snapshot.task = None;
                } else {
                    let task = snapshot.task.as_mut().unwrap();
                    task.task.acceptance.clear();
                    task.scope.criteria_sha256 = criteria_sha256(&[]).unwrap();
                }
            });
            assert_eq!(
                fixture.effect().unwrap().allowed,
                mode == TaskEvidenceDeliveryMode::Observe
            );
            let observation = fixture.last();
            assert!(
                observation
                    .reasons
                    .contains(&TaskDeliveryEvidenceReason::CriteriaMissing)
            );
            assert!(observation.source.is_none());
            assert!(!observation.satisfied());
        }
    }
}

#[test]
fn worker_pass_is_insufficient_and_current_fail_or_blocked_vetoes_host_success() {
    for status in [
        TaskEvidenceStatus::Pass,
        TaskEvidenceStatus::Fail,
        TaskEvidenceStatus::Blocked,
    ] {
        let fixture = Fixture::new(TaskEvidenceDeliveryMode::Enforce);
        fixture.install_report(status);
        if status == TaskEvidenceStatus::Pass {
            fixture.change(|snapshot| snapshot.task.as_mut().unwrap().host_verifications.clear());
        }
        assert!(!fixture.effect().unwrap().allowed);
        assert!(!fixture.last().satisfied());
    }
}

#[test]
fn only_the_current_human_waiver_can_exempt_a_manual_criterion_and_reported_failure() {
    for stale_waiver in [false, true] {
        let fixture = Fixture::new(TaskEvidenceDeliveryMode::Enforce);
        fixture.install_report(TaskEvidenceStatus::Fail);
        fixture.change(|snapshot| {
            let criterion = &mut snapshot.task.as_mut().unwrap().task.acceptance[0];
            criterion.channel = TaskAcceptanceChannel::Screen;
            criterion.verification_recipe = None;
        });
        fixture.synchronize_scope();
        fixture.waive();
        if stale_waiver {
            fixture.change(|snapshot| {
                snapshot.task.as_mut().unwrap().waivers[0]
                    .waiver
                    .scope
                    .scope_id = "9".repeat(64)
            });
        }
        assert_eq!(fixture.effect().unwrap().allowed, !stale_waiver);
        if !stale_waiver {
            assert_eq!(
                fixture.last().criteria[0].state,
                TaskDeliveryCriterionState::Waived
            );
        }
    }
}

#[test]
fn scope_generation_base_source_and_environment_changes_do_not_reuse_old_success() {
    for change in 0..6 {
        let fixture = Fixture::new(TaskEvidenceDeliveryMode::Enforce);
        match change {
            0 => fixture.change(|snapshot| {
                snapshot
                    .queue
                    .evidence_binding
                    .as_mut()
                    .unwrap()
                    .scope
                    .scope_id = "8".repeat(64)
            }),
            1 => fixture.change(|snapshot| {
                snapshot
                    .queue
                    .evidence_binding
                    .as_mut()
                    .unwrap()
                    .lease_generation = "8".repeat(64)
            }),
            2 => fixture.change(|snapshot| snapshot.queue.source_base_commit_sha = "e".repeat(40)),
            3 => {
                fixture
                    .runtime
                    .trees
                    .lock()
                    .unwrap()
                    .insert("e".repeat(40), "8".repeat(40));
            }
            4 => {
                *fixture.runner.current.lock().unwrap() = Ok("8".repeat(64));
            }
            _ => fixture.change(|snapshot| snapshot.queue.session_key = "another-session".into()),
        }
        assert!(!fixture.effect().unwrap().allowed, "case {change}");
        assert!(!fixture.last().satisfied());
    }
}

#[test]
fn unexecuted_failed_partial_and_unsealed_facts_cannot_satisfy_a_criterion() {
    for case in 0..7 {
        let fixture = Fixture::new(TaskEvidenceDeliveryMode::Enforce);
        fixture.change(|snapshot| {
            let host = &mut snapshot.task.as_mut().unwrap().host_verifications[0];
            match case {
                0 => {
                    host.phase = TaskVerificationPhase::Pending;
                    host.manifest = None;
                    host.receipt = None;
                }
                1 => {
                    host.phase = TaskVerificationPhase::Running;
                    host.receipt = None;
                }
                2 | 3 => {
                    let receipt = host.receipt.as_mut().unwrap();
                    receipt.result = if case == 2 {
                        TaskVerificationResult::Interrupted {}
                    } else {
                        TaskVerificationResult::NotRun {
                            reason: TaskVerificationNotRunReason::Cancelled,
                        }
                    };
                    receipt.artifact = None;
                }
                4 | 5 => {
                    let TaskVerificationResult::Executed { observation } =
                        &mut host.receipt.as_mut().unwrap().result
                    else {
                        unreachable!()
                    };
                    if case == 4 {
                        observation.outcome =
                            TaskVerificationExecutionOutcome::Exited { exit_code: 2 };
                    } else {
                        observation.stdout.complete = false;
                    }
                }
                _ => host.receipt.as_mut().unwrap().artifact = None,
            }
        });
        assert!(!fixture.effect().unwrap().allowed, "case {case}");
    }
    let fixture = Fixture::new(TaskEvidenceDeliveryMode::Enforce);
    fixture.artifacts.available.store(false, Ordering::SeqCst);
    assert!(!fixture.effect().unwrap().allowed);
    assert!(
        fixture
            .last()
            .reasons
            .contains(&TaskDeliveryEvidenceReason::EvidenceUnavailable)
    );
}

#[test]
fn every_essential_criterion_is_required_but_optional_criteria_do_not_block_delivery() {
    for essential in [false, true] {
        let fixture = Fixture::new(TaskEvidenceDeliveryMode::Enforce);
        fixture.change(|snapshot| {
            let task = &mut snapshot.task.as_mut().unwrap().task;
            let mut criterion = task.acceptance[0].clone();
            criterion.id = "second".into();
            criterion.essential = essential;
            criterion.verification_recipe = None;
            task.acceptance.push(criterion);
        });
        fixture.synchronize_scope();
        assert_eq!(fixture.effect().unwrap().allowed, !essential);
    }
}

#[test]
fn integration_checks_its_actual_result_tree_and_frozen_base() {
    for change in 0..3 {
        let fixture = Fixture::new(TaskEvidenceDeliveryMode::Enforce);
        let base = if change == 1 {
            "e".repeat(40)
        } else {
            "a".repeat(40)
        };
        fixture.change(|snapshot| {
            snapshot.queue.integration_base_commit_sha = Some(base.clone());
            snapshot.queue.integration_commit_sha = Some("f".repeat(40));
        });
        if change == 2 {
            fixture
                .runtime
                .trees
                .lock()
                .unwrap()
                .insert("f".repeat(40), "9".repeat(40));
        }
        let queue = fixture.repository.snapshot.lock().unwrap().queue.clone();
        let result = fixture
            .service
            .evaluate(
                "/repo",
                &queue,
                TaskEvidenceDeliveryStage::Integrate,
                Some(&TaskDeliveryEvidenceIntegration {
                    base_commit_sha: base,
                    result_commit_sha: "f".repeat(40),
                    preflight_requires_human_review: Some(false),
                }),
                &TaskDeliveryEvidenceOperation::Effect {
                    owner_token: "owner".into(),
                },
            )
            .unwrap();
        assert_eq!(result.allowed, change == 0);
        assert!(fixture.last().source.unwrap().integration.is_some());
    }
}
