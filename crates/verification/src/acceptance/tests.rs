use super::*;
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

#[test]
fn criteria_trim_and_sort_by_id_without_changing_meaning() {
    let input = vec![criterion(" b ", " second "), criterion("a", " first\n")];
    let normalized = normalize_task_acceptance(&input).unwrap();
    assert_eq!(
        normalized,
        vec![criterion("a", "first"), criterion("b", "second")]
    );
    assert_eq!(input[0].id, " b ");
    assert_eq!(normalize_task_acceptance(&normalized).unwrap(), normalized);
}

#[test]
fn criteria_reject_blank_fields_and_duplicate_normalized_ids() {
    assert_eq!(
        normalize_task_acceptance(&[criterion(" \n", "scenario")]),
        Err(TaskAcceptanceError::BlankId)
    );
    assert_eq!(
        normalize_task_acceptance(&[criterion("id", " \n")]),
        Err(TaskAcceptanceError::BlankScenario)
    );
    assert_eq!(
        normalize_task_acceptance(&[criterion("id", "one"), criterion(" id ", "two")]),
        Err(TaskAcceptanceError::DuplicateId("id".into()))
    );
}

#[test]
fn criterion_wire_requires_every_field_and_rejects_unknown_fields_or_channels() {
    let valid = json!({"id":"a","scenario":"scenario","channel":"command","essential":true});
    for field in ["id", "scenario", "channel", "essential"] {
        let mut invalid = valid.clone();
        invalid.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<TaskAcceptanceCriterion>(invalid).is_err());
    }
    for field in ["acceptance_revision", "approved"] {
        let mut invalid = valid.clone();
        invalid[field] = json!(1);
        assert!(serde_json::from_value::<TaskAcceptanceCriterion>(invalid).is_err());
    }
    for value in ["unknown", "Command", "shell"] {
        let mut invalid = valid.clone();
        invalid["channel"] = json!(value);
        assert!(serde_json::from_value::<TaskAcceptanceCriterion>(invalid).is_err());
    }
    for channel in ["command", "data", "http", "screen"] {
        let mut value = valid.clone();
        value["channel"] = json!(channel);
        let parsed: TaskAcceptanceCriterion = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), value);
    }
}

#[test]
fn criteria_require_a_revision_and_preserve_empty_history() {
    assert_eq!(
        validate_task_acceptance(&[criterion("a", "scenario")], 0),
        Err(TaskAcceptanceError::MissingRevision)
    );
    assert!(validate_task_acceptance(&[criterion("a", "scenario")], 1).is_ok());
    assert!(validate_task_acceptance(&[], 0).is_ok());
    assert!(validate_task_acceptance(&[], 9).is_ok());
}
