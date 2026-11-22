use std::sync::{Arc, Barrier};

use super::*;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkControlAction, PlanningWorkControlRepositoryPort,
    PlanningWorkOrderAdmissionSnapshot, PlanningWorkOrderSupplementalAdmission,
    PlanningWorkOrderSupplementalAdmissionResult, PlanningWorkPermit, PlanningWorkPermitBasis,
    PlanningWorkPermitRepositoryPort, PlanningWorkPermitStage,
};

fn basis(revision: u64) -> PlanningWorkPermitBasis {
    PlanningWorkPermitBasis {
        authorization_epoch: 0,
        policy_version: "policy".into(),
        field_hash: "field".into(),
        gate_revision: revision,
        source_sha: "a".repeat(40),
    }
}

fn order_snapshot(workspace: &str) -> PlanningWorkOrderAdmissionSnapshot {
    SqlitePlanningAuthorityAdapter::new()
        .load_work_order_admission_snapshot(workspace, "order-a")
        .unwrap()
        .unwrap()
}

fn admit(
    workspace: &str,
    key: &str,
    task_id: &str,
    expected: &PlanningWorkOrderAdmissionSnapshot,
    before: &PlanningTaskAuthoritySnapshot,
) -> Result<PlanningWorkOrderSupplementalAdmissionResult> {
    let mut document = candidate(before, task_id);
    let provenance = TaskMutationProvenance::new(OriginSessionKind::System)
        .with_thread_turn(None, Some(key.to_owned()));
    document.tasks.last_mut().unwrap().provenance = provenance.clone();
    let ids = [task_id.to_owned()];
    SqlitePlanningAuthorityAdapter::new().admit_supplemental_work_order_task(
        workspace,
        PlanningWorkOrderSupplementalAdmission {
            admission_key: key,
            expected,
            observed_planning_revision: before.planning_revision,
            task_id,
            task_authority: &document,
            queue_projection: &projection(&document),
            audit: PlanningTaskAuthorityMutationAudit {
                task_ids: &ids,
                legacy_source_turn_id: None,
                provenance: &provenance,
            },
        },
    )
}

fn setup(label: &str) -> (String, SqlitePlanningAuthorityAdapter) {
    let workspace = workspace(label);
    seed(&workspace, Vec::new());
    register_next(&workspace, "order-a", "original");
    (workspace, SqlitePlanningAuthorityAdapter::new())
}

fn grant(adapter: &SqlitePlanningAuthorityAdapter, workspace: &str, revision: u64) {
    adapter
        .set_work_order_permit_basis(workspace, "order-a", &basis(revision))
        .unwrap();
    for stage in PlanningWorkPermitStage::ALL {
        adapter
            .grant_work_order_permit(
                workspace,
                &PlanningWorkPermit {
                    work_order_id: "order-a".into(),
                    stage,
                    basis: basis(revision),
                    requires_human_review: stage == PlanningWorkPermitStage::Integrate,
                },
            )
            .unwrap();
    }
}

#[test]
fn supplemental_task_lineage_gate_and_all_revocations_commit_together() {
    let (workspace, adapter) = setup("supplemental-atomic");
    grant(&adapter, &workspace, 1);
    let before = snapshot(&workspace);
    let expected = order_snapshot(&workspace);
    let admitted = admit(&workspace, "finding-1", "remediation", &expected, &before).unwrap();
    let PlanningWorkOrderSupplementalAdmissionResult::Admitted {
        snapshot: current,
        created,
        ..
    } = admitted
    else {
        panic!("expected admission");
    };
    assert!(created);
    assert_eq!(current.work_order.task_ids, ["original", "remediation"]);
    assert!(current.work_order.permits.is_empty());
    assert_eq!(current.control, expected.control);
    assert_eq!(current.permit_basis.as_ref().unwrap().gate_revision, 2);
    assert_eq!(
        count(&workspace, "planning_work_order_permit_revocations"),
        4
    );
    for stage in PlanningWorkPermitStage::ALL {
        assert!(
            adapter
                .grant_work_order_permit(
                    &workspace,
                    &PlanningWorkPermit {
                        work_order_id: "order-a".into(),
                        stage,
                        basis: basis(1),
                        requires_human_review: stage == PlanningWorkPermitStage::Integrate,
                    }
                )
                .is_err()
        );
    }
    grant(&adapter, &workspace, 2);
    adapter
        .apply_work_order_control(
            &workspace,
            "order-a",
            "cancel",
            PlanningWorkControlAction::Cancel,
        )
        .unwrap();
    assert!(
        snapshot(&workspace)
            .task_authority
            .tasks
            .iter()
            .all(|task| task.status == TaskStatus::Cancelled)
    );

    let replay = admit(&workspace, "finding-1", "unused-new-id", &expected, &before).unwrap();
    let PlanningWorkOrderSupplementalAdmissionResult::Existing { task_id, .. } = replay else {
        panic!("expected immutable admission");
    };
    assert_eq!(task_id, "remediation");
    assert_eq!(snapshot(&workspace).task_authority.tasks.len(), 2);
}

#[test]
fn failed_admission_rolls_back_task_lineage_gate_and_receipt() {
    let (workspace, adapter) = setup("supplemental-rollback");
    grant(&adapter, &workspace, 1);
    let before = snapshot(&workspace);
    let expected = order_snapshot(&workspace);
    connection(&workspace).execute_batch(
        "CREATE TRIGGER fail_supplemental_admission BEFORE INSERT ON planning_work_order_task_admissions BEGIN SELECT RAISE(ABORT,'synthetic admission failure'); END;"
    ).unwrap();
    assert!(
        admit(
            &workspace,
            "finding-rollback",
            "remediation",
            &expected,
            &before
        )
        .is_err()
    );
    assert_eq!(snapshot(&workspace), before);
    assert_eq!(order_snapshot(&workspace), expected);
    assert_eq!(count(&workspace, "planning_work_order_task_admissions"), 0);
    assert_eq!(
        count(&workspace, "planning_work_order_permit_revocations"),
        0
    );
    connection(&workspace)
        .execute_batch("DROP TRIGGER fail_supplemental_admission;")
        .unwrap();
    assert!(matches!(
        admit(
            &workspace,
            "finding-rollback",
            "remediation",
            &expected,
            &before
        )
        .unwrap(),
        PlanningWorkOrderSupplementalAdmissionResult::Admitted { .. }
    ));
}

#[test]
fn competing_candidates_with_one_admission_key_return_one_durable_task() {
    let (workspace, adapter) = setup("supplemental-race");
    grant(&adapter, &workspace, 1);
    let before = snapshot(&workspace);
    let expected = order_snapshot(&workspace);
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = ["candidate-a", "candidate-b"]
        .into_iter()
        .map(|task_id| {
            let workspace = workspace.clone();
            let before = before.clone();
            let expected = expected.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                admit(&workspace, "shared-finding", task_id, &expected, &before).unwrap()
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(
                result,
                PlanningWorkOrderSupplementalAdmissionResult::Admitted { .. }
            ))
            .count(),
        1
    );
    let ids: Vec<_> = results
        .iter()
        .map(|result| match result {
            PlanningWorkOrderSupplementalAdmissionResult::Admitted { task_id, .. }
            | PlanningWorkOrderSupplementalAdmissionResult::Existing { task_id, .. } => task_id,
            other => panic!("unexpected admission result: {other:?}"),
        })
        .collect();
    assert_eq!(ids[0], ids[1]);
    assert_eq!(snapshot(&workspace).task_authority.tasks.len(), 2);
    assert_eq!(
        order_snapshot(&workspace)
            .permit_basis
            .unwrap()
            .gate_revision,
        2
    );
    assert_eq!(count(&workspace, "planning_work_order_task_admissions"), 1);
}

#[test]
fn stale_gate_control_planning_revision_and_attempt_cap_prevent_creation() {
    for scenario in 0..5 {
        let (workspace, adapter) = setup(&format!("supplemental-fence-{scenario}"));
        grant(&adapter, &workspace, 1);
        let before = snapshot(&workspace);
        let mut expected = order_snapshot(&workspace);
        match scenario {
            0 => adapter
                .set_work_order_permit_basis(&workspace, "order-a", &basis(2))
                .unwrap(),
            1 | 2 => {
                let action = if scenario == 1 {
                    PlanningWorkControlAction::Cancel
                } else {
                    PlanningWorkControlAction::TakeOver
                };
                adapter
                    .apply_work_order_control(&workspace, "order-a", "intervene", action)
                    .unwrap();
                expected = order_snapshot(&workspace);
            }
            3 => commit_document(
                &workspace,
                Some(before.planning_revision),
                &candidate(&before, "unrelated"),
            ),
            _ => adapter
                .set_work_order_execution_limit(&workspace, "order-a", 0)
                .unwrap(),
        }
        let result = admit(&workspace, "new-finding", "remediation", &expected, &before);
        match scenario {
            0 => assert!(matches!(
                result.unwrap(),
                PlanningWorkOrderSupplementalAdmissionResult::WorkOrderConflict { .. }
            )),
            3 => assert!(matches!(
                result.unwrap(),
                PlanningWorkOrderSupplementalAdmissionResult::RevisionConflict { .. }
            )),
            _ => assert!(result.is_err()),
        }
        assert!(
            !snapshot(&workspace)
                .task_authority
                .tasks
                .iter()
                .any(|task| task.id == "remediation")
        );
        assert_eq!(count(&workspace, "planning_work_order_task_admissions"), 0);
        assert_eq!(stored(&workspace, "order-a").task_ids, ["original"]);
    }
}

#[test]
fn supplemental_candidate_identity_must_match_its_immutable_admission_key_and_audit() {
    for case in ["origin", "key", "audit"] {
        let (workspace, adapter) = setup(case);
        grant(&adapter, &workspace, 1);
        let before = snapshot(&workspace);
        let expected = order_snapshot(&workspace);
        let key = "finding-identity";
        let provenance = TaskMutationProvenance::new(OriginSessionKind::System)
            .with_thread_turn(None, Some(key.to_owned()));
        let mut document = candidate(&before, "remediation");
        document.tasks.last_mut().unwrap().provenance = provenance.clone();
        let mut audit_provenance = provenance;
        match case {
            "origin" => {
                document.tasks.last_mut().unwrap().provenance = TaskMutationProvenance::default()
            }
            "key" => {
                document.tasks.last_mut().unwrap().provenance.turn_id = Some("another-key".into())
            }
            "audit" => audit_provenance.turn_id = Some("another-audit-key".into()),
            _ => unreachable!(),
        }
        let ids = ["remediation".to_owned()];
        let result = adapter.admit_supplemental_work_order_task(
            &workspace,
            PlanningWorkOrderSupplementalAdmission {
                admission_key: key,
                expected: &expected,
                observed_planning_revision: before.planning_revision,
                task_id: "remediation",
                task_authority: &document,
                queue_projection: &projection(&document),
                audit: PlanningTaskAuthorityMutationAudit {
                    task_ids: &ids,
                    legacy_source_turn_id: None,
                    provenance: &audit_provenance,
                },
            },
        );
        assert!(result.is_err(), "{case}");
        assert_eq!(snapshot(&workspace), before);
        assert_eq!(order_snapshot(&workspace), expected);
        assert_eq!(count(&workspace, "planning_work_order_task_admissions"), 0);
    }
}
