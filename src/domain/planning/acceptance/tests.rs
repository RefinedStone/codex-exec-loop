use super::*;
use crate::domain::planning::{
    DirectionCatalogDocument, PLANNING_FORMAT_VERSION, PlanningSemanticValidationService,
    PlanningValidationReport, TASK_AUTHORITY_FORMAT_VERSION, TaskAuthorityDocument,
    task_authority_format_version_supported,
};
use serde_json::json;

fn criterion(id: &str, scenario: &str) -> TaskAcceptanceCriterion {
    TaskAcceptanceCriterion {
        id: id.into(),
        scenario: scenario.into(),
        channel: TaskAcceptanceChannel::Command,
        essential: true,
        verification_recipe: None,
    }
}
fn legacy() -> TaskAuthorityDocument {
    serde_json::from_value(json!({
        "version":1,"tasks":[{
            "id":"task-1","direction_id":"direction-1","title":"Task","description":"Description",
            "status":"ready","base_priority":80,"created_by":"user","last_updated_by":"user",
            "updated_at":"2026-11-23T00:00:00Z"
        }]
    }))
    .unwrap()
}

#[test]
fn legacy_tasks_default_to_empty_criteria_and_revision_zero() {
    let document = legacy();
    assert_eq!(document.version, 1);
    assert!(document.tasks[0].acceptance.is_empty());
    assert_eq!(document.tasks[0].acceptance_revision, 0);
    let encoded = serde_json::to_value(&document).unwrap();
    let restored: TaskAuthorityDocument = serde_json::from_value(encoded).unwrap();
    assert_eq!(restored, document);
}

#[test]
fn current_task_round_trip_preserves_criteria_and_host_revision() {
    let mut document = legacy();
    document.version = TASK_AUTHORITY_FORMAT_VERSION;
    document.tasks[0].acceptance = vec![criterion("a", "verify the result")];
    document.tasks[0].acceptance_revision = 7;
    let encoded = serde_json::to_value(&document).unwrap();
    assert_eq!(encoded["tasks"][0]["acceptance_revision"], json!(7));
    assert_eq!(
        serde_json::from_value::<TaskAuthorityDocument>(encoded).unwrap(),
        document
    );
    let normalized = document.tasks[0].normalized();
    assert_eq!(normalized.acceptance_revision, 7);
}

#[test]
fn criteria_need_a_host_revision_but_cleared_criteria_keep_their_history() {
    assert_eq!(
        validate_task_acceptance(&[criterion("a", "scenario")], 0),
        Err(TaskAcceptanceError::MissingRevision)
    );
    assert!(validate_task_acceptance(&[criterion("a", "scenario")], 1).is_ok());
    assert!(validate_task_acceptance(&[], 0).is_ok());
    assert!(validate_task_acceptance(&[], 9).is_ok());
    let mut document = legacy();
    document.tasks[0].acceptance = vec![criterion("a", "scenario")];
    let mut report = PlanningValidationReport::new();
    PlanningSemanticValidationService::new().validate(None, Some(&document), &mut report);
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.code == "invalid_task_acceptance")
    );
}

#[test]
fn task_versions_accept_legacy_and_current_without_changing_directions() {
    assert_eq!(PLANNING_FORMAT_VERSION, 1);
    assert_eq!(TASK_AUTHORITY_FORMAT_VERSION, 2);
    for version in [1, 2] {
        assert!(task_authority_format_version_supported(version));
        let mut document = legacy();
        document.version = version;
        let mut report = PlanningValidationReport::new();
        PlanningSemanticValidationService::new().validate(None, Some(&document), &mut report);
        assert!(report.is_valid(), "{report:?}");
    }
    for version in [0, 3, u32::MAX] {
        assert!(!task_authority_format_version_supported(version));
        let mut document = legacy();
        document.version = version;
        let mut report = PlanningValidationReport::new();
        PlanningSemanticValidationService::new().validate(None, Some(&document), &mut report);
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.code == "unsupported_task_authority_version")
        );
    }
    let directions: DirectionCatalogDocument =
        serde_json::from_value(json!({"version":2,"directions":[]})).unwrap();
    let mut report = PlanningValidationReport::new();
    PlanningSemanticValidationService::new().validate(Some(&directions), None, &mut report);
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.code == "unsupported_directions_version")
    );
}
