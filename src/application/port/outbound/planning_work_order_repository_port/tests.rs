use super::*;

fn record() -> PlanningWorkOrderRecord {
    PlanningWorkOrderRecord {
        work_order_id: "source:item:hash:attempt-1".to_string(),
        origin: "example_module".to_string(),
        task_ids: vec!["task-first".to_string(), "task-followup".to_string()],
        permits: Vec::new(),
    }
}

#[test]
fn opaque_order_identity_preserves_exact_bytes_and_rejects_invalid_boundaries() {
    for (work_order_id, origin) in [
        ("source:item/hash:1", "module.name"),
        ("한글 지시", "중립 모듈"),
        (" padded identity ", " padded origin "),
    ] {
        validate_work_order_identity(work_order_id, origin).unwrap();
    }
    for invalid in ["", "   ", "line\nbreak", "nul\0byte", "delete\u{7f}"] {
        assert!(validate_work_order_identity(invalid, "module").is_err());
        assert!(validate_work_order_identity("order", invalid).is_err());
    }
    assert!(
        validate_work_order_identity(&"a".repeat(MAX_WORK_ORDER_ID_BYTES + 1), "module").is_err()
    );
    assert!(
        validate_work_order_identity("order", &"a".repeat(MAX_WORK_ORDER_ORIGIN_BYTES + 1))
            .is_err()
    );
    validate_work_order_identity(
        &"a".repeat(MAX_WORK_ORDER_ID_BYTES),
        &"b".repeat(MAX_WORK_ORDER_ORIGIN_BYTES),
    )
    .unwrap();
}

#[test]
fn task_identity_must_match_the_canonical_planning_row_key() {
    for invalid in ["", " ", " task", "task ", "task\n", "task\0"] {
        assert!(validate_work_order_task_id(invalid).is_err());
    }
    assert!(validate_work_order_task_id(&"t".repeat(MAX_WORK_ORDER_TASK_ID_BYTES + 1)).is_err());
    validate_work_order_task_id("task-20261120-example").unwrap();
}

#[test]
fn durable_record_requires_an_original_task_and_unique_lineage() {
    let valid = record();
    valid.validate().unwrap();
    assert_eq!(valid.initial_task_id().unwrap(), "task-first");
    let mut missing = valid.clone();
    missing.task_ids.clear();
    assert!(missing.validate().is_err());
    let mut duplicate = valid.clone();
    duplicate.task_ids.push(valid.task_ids[0].clone());
    assert!(duplicate.validate().is_err());
    let mut malformed = valid;
    malformed.task_ids[1] = " padded-task ".to_string();
    assert!(malformed.validate().is_err());
}

#[test]
fn origin_collision_is_typed_and_does_not_change_the_record() {
    let original = record();
    original.ensure_origin("example_module").unwrap();
    let error = original.ensure_origin("other_module").unwrap_err();
    assert_eq!(
        error.downcast_ref::<PlanningWorkOrderError>(),
        Some(&PlanningWorkOrderError::OriginConflict {
            work_order_id: original.work_order_id.clone(),
        })
    );
    assert_eq!(original, record());
}

#[test]
fn lineage_round_trip_preserves_order_and_empty_initial_permits() {
    let original = record();
    let json = serde_json::to_value(&original).unwrap();
    assert_eq!(json["permits"], serde_json::json!([]));
    assert_eq!(
        serde_json::from_value::<PlanningWorkOrderRecord>(json).unwrap(),
        original
    );
}
