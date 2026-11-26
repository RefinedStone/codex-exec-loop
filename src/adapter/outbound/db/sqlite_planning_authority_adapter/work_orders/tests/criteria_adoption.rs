use super::*;
use crate::domain::planning::{TaskAcceptanceChannel, TaskAcceptanceCriterion};

fn register_confirmed(
    workspace: &str,
    receipt: &str,
) -> Result<PlanningWorkOrderRegistrationResult> {
    let before = snapshot(workspace);
    let mut document = candidate(&before, "task-confirmed");
    document.version = crate::domain::planning::TASK_AUTHORITY_FORMAT_VERSION;
    let task = document.tasks.last_mut().unwrap();
    task.acceptance = vec![TaskAcceptanceCriterion {
        id: "criterion-1".into(),
        scenario: "preserve approved outcome".into(),
        channel: TaskAcceptanceChannel::Data,
        essential: true,
        verification_recipe: None,
    }];
    task.acceptance_revision = 1;
    let provenance = TaskMutationProvenance::new(OriginSessionKind::System);
    SqlitePlanningAuthorityAdapter::new().register_work_order(
        workspace,
        PlanningWorkOrderRegistration {
            acceptance_receipt: Some(receipt),
            work_order_id: "order-confirmed",
            origin: "issue_automation",
            link_reference: None,
            initial_task_id: "task-confirmed",
            observed_planning_revision: before.planning_revision,
            task_authority: &document,
            queue_projection: &projection(&document),
            audit: PlanningTaskAuthorityMutationAudit {
                task_ids: &["task-confirmed".into()],
                legacy_source_turn_id: None,
                provenance: &provenance,
            },
        },
    )
}

#[test]
fn criteria_adoption_sqlite_rejects_changed_receipt_for_an_existing_work_order() {
    let workspace = workspace("criteria-receipt-conflict");
    seed(&workspace, Vec::new());
    register_confirmed(&workspace, &"a".repeat(64)).unwrap();
    let before = snapshot(&workspace);
    let lineage = stored(&workspace, "order-confirmed");
    assert!(register_confirmed(&workspace, &"b".repeat(64)).is_err());
    assert_eq!(snapshot(&workspace), before);
    assert_eq!(stored(&workspace, "order-confirmed"), lineage);
    let replay = register_confirmed(&workspace, &"a".repeat(64)).unwrap();
    assert!(matches!(
        replay,
        PlanningWorkOrderRegistrationResult::Existing { .. }
    ));
    assert_eq!(snapshot(&workspace), before);
    std::fs::remove_dir_all(workspace).unwrap();
}

#[test]
fn criteria_adoption_sqlite_receipt_failure_rolls_back_task_lineage_and_revision() {
    let workspace = workspace("criteria-receipt-atomic");
    let before = seed(&workspace, Vec::new());
    let db = connection(&workspace);
    ensure_work_order_tables(&db).unwrap();
    db.execute_batch("CREATE TRIGGER reject_criteria_receipt BEFORE INSERT ON planning_work_order_acceptance_receipts BEGIN SELECT RAISE(ABORT, 'injected receipt write failure'); END;").unwrap();
    assert!(register_confirmed(&workspace, &"a".repeat(64)).is_err());
    assert_eq!(snapshot(&workspace), before);
    assert!(
        SqlitePlanningAuthorityAdapter::new()
            .load_work_order(&workspace, "order-confirmed")
            .unwrap()
            .is_none()
    );
    drop(db);
    std::fs::remove_dir_all(workspace).unwrap();
}

#[test]
fn criteria_adoption_legacy_database_adds_the_receipt_table_without_changing_lineage() {
    let workspace = workspace("criteria-legacy-schema");
    seed(&workspace, Vec::new());
    let old = register_next(&workspace, "legacy-order", "legacy-task");
    let db = connection(&workspace);
    db.execute_batch("DROP TABLE planning_work_order_acceptance_receipts;")
        .unwrap();
    assert_eq!(stored(&workspace, "legacy-order"), old);
    assert!(table_exists(&db, "planning_work_order_acceptance_receipts").unwrap());
    assert_eq!(old.acceptance_receipt, None);
    drop(db);
    std::fs::remove_dir_all(workspace).unwrap();
}
