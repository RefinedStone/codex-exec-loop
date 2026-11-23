use super::*;
use crate::adapter::inbound::kernel_protocol::tests::assert_matches_spec;
use crate::adapter::inbound::kernel_protocol::{
    CommandResult, KernelCommand, KernelCommandInput, KernelQuery, KernelQueryAnswer,
    PlanningState, describe,
};
use crate::application::port::inbound::stored_command_port::StoredCommandResult;
use crate::domain::kernel_event::sample_task_evidence_events;
use crate::domain::planning::RuntimeProjection;
use crate::domain::planning::evidence::{TaskEvidenceEvent, TaskEvidenceStatus};
use serde_json::{Value, json};

fn prepare_command() -> Value {
    json!({
        "type":"prepare_evidence", "request_id":"prepare-1", "task_id":"task-1",
        "criterion_id":"criterion with spaces", "scope_id":"a".repeat(64),
        "expected_acceptance_revision":3
    })
}

fn waiver_command() -> Value {
    let mut command = prepare_command();
    command["type"] = json!("waive_criterion");
    command["request_id"] = json!("waive-1");
    command["reason"] = json!("The agreed scope is exempted.");
    command
}

#[test]
fn evidence_commands_keep_the_stored_boundary_and_reject_authority_claims() {
    let spec = serde_json::to_value(describe()).unwrap();
    for encoded in [
        prepare_command(),
        json!({"type":"record_evidence","attempt_id":"b".repeat(64),"status":"pass","note":"Recorded output"}),
        waiver_command(),
    ] {
        let command: KernelCommand = serde_json::from_value(encoded.clone()).unwrap();
        assert!(!command.confirmed());
        assert_matches_spec(&spec, "commands", "input", &encoded);
        let KernelCommandInput::Stored(stored) = command.clone().into_input("/workspace").unwrap()
        else {
            panic!("evidence commands must use the dedicated stored service");
        };
        assert_eq!(KernelCommand::from_stored(stored), command);
        for field in [
            "artifact_path",
            "sha256",
            "tree_sha",
            "by",
            "human",
            "confirm",
            "recorded_at",
            "lease_generation",
        ] {
            let mut forged = encoded.clone();
            forged[field] = json!("forged");
            assert!(
                serde_json::from_value::<KernelCommand>(forged).is_err(),
                "{field}"
            );
        }
    }
}

#[test]
fn evidence_input_requires_a_bounded_current_scope_and_closed_status() {
    for mut encoded in [prepare_command(), waiver_command()] {
        for (field, invalid) in [
            ("request_id", json!("x".repeat(129))),
            ("task_id", json!("")),
            ("criterion_id", json!(" ")),
            ("scope_id", json!("A".repeat(64))),
            ("expected_acceptance_revision", json!(0)),
            (
                "expected_acceptance_revision",
                json!(9_007_199_254_740_992u64),
            ),
        ] {
            let old = encoded[field].clone();
            encoded[field] = invalid;
            let command: KernelCommand = serde_json::from_value(encoded.clone()).unwrap();
            assert!(command.into_input("/workspace").is_err(), "{field}");
            encoded[field] = old;
        }
        encoded.as_object_mut().unwrap().remove("scope_id");
        assert!(serde_json::from_value::<KernelCommand>(encoded).is_err());
    }
    assert!(
        serde_json::from_value::<KernelCommand>(json!({
            "type":"record_evidence", "attempt_id":"a".repeat(64), "status":"waived"
        }))
        .is_err()
    );
    let oversized: KernelCommand = serde_json::from_value(json!({
        "type":"record_evidence", "attempt_id":"a".repeat(64), "status":"blocked", "note":"가".repeat(1366)
    })).unwrap();
    assert!(
        oversized.into_input("/workspace").is_err(),
        "the text limit counts bytes"
    );
}

#[test]
fn public_receipts_preserve_original_facts_and_never_expose_private_contents() {
    for event in sample_task_evidence_events() {
        let result = match event.clone() {
            TaskEvidenceEvent::Recorded { receipt } => {
                StoredCommandResult::CriterionEvidenceRecorded {
                    receipt,
                    changed: false,
                }
            }
            TaskEvidenceEvent::Waived { receipt } => StoredCommandResult::CriterionWaived {
                receipt,
                changed: false,
            },
        };
        let encoded = serde_json::to_value(CommandResult::from_stored(result)).unwrap();
        assert_eq!(encoded["receipt"]["event_id"], event.event_id());
        assert_eq!(
            encoded["receipt"]["evidence_revision"],
            event.evidence_revision()
        );
        assert_eq!(encoded["changed"], false);
        let text = encoded.to_string();
        for private in [
            "private evidence note",
            "private waiver reason",
            "confirmation_ref",
            "artifact_path",
            "session_key",
            "lease_generation",
        ] {
            assert!(!text.contains(private), "{private}");
        }
    }
}

#[test]
fn only_preparation_exposes_the_fixed_writable_artifact_path() {
    let TaskEvidenceEvent::Recorded { receipt } = sample_task_evidence_events()[0].clone() else {
        unreachable!()
    };
    let prepared = TaskEvidencePrepared {
        attempt: receipt.evidence.attempt.clone(),
        artifact_path: "/private/incoming/fixed/evidence.bin".into(),
        changed: false,
    };
    let result = CommandResult::from_stored(StoredCommandResult::EvidencePrepared {
        preparation: Box::new(prepared),
    });
    let encoded = serde_json::to_value(result).unwrap();
    assert_eq!(
        encoded["preparation"]["attempt_id"],
        receipt.evidence.attempt.attempt_id
    );
    assert_eq!(
        encoded["preparation"]["artifact_path"],
        "/private/incoming/fixed/evidence.bin"
    );
    assert_eq!(encoded["preparation"]["scope_id"], "a".repeat(64));
    assert!(encoded["preparation"].get("session_key").is_none());
}

#[test]
fn task_evidence_pages_are_bounded_and_do_not_infer_an_empty_selection() {
    let default: KernelQuery = serde_json::from_value(json!({"type":"task_evidence"})).unwrap();
    let request = default.task_evidence_request().unwrap().unwrap();
    assert_eq!(request, TaskEvidenceSnapshotQuery::default());
    let explicit: KernelQuery = serde_json::from_value(json!({
        "type":"task_evidence", "task_ids":["task-1", "task with spaces"], "limit":2
    }))
    .unwrap();
    assert_eq!(
        explicit
            .task_evidence_request()
            .unwrap()
            .unwrap()
            .task_ids
            .len(),
        2
    );
    let page: KernelQuery = serde_json::from_value(json!({
        "type":"task_evidence", "after_task_id":"task-256", "limit":1
    }))
    .unwrap();
    assert_eq!(
        page.task_evidence_request()
            .unwrap()
            .unwrap()
            .after_task_id
            .as_deref(),
        Some("task-256")
    );
    for fields in [
        json!({"task_ids":[]}),
        json!({"task_ids":["task-1","task-1"]}),
        json!({"task_ids":["task-1"],"after_task_id":"task-1"}),
        json!({"task_ids":["task-1","task-2"],"limit":1}),
        json!({"limit":0}),
        json!({"limit":257}),
        json!({"after_task_id":" "}),
    ] {
        let mut value = json!({"type":"task_evidence"});
        value
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        let query: KernelQuery = serde_json::from_value(value.clone()).unwrap();
        assert!(query.task_evidence_request().is_err(), "{value}");
    }
}

#[test]
fn planning_evidence_is_optional_and_keeps_old_receipt_scope_when_stale() {
    let original = PlanningState::from_core(&RuntimeProjection::uninitialized());
    let encoded = serde_json::to_value(&original).unwrap();
    assert!(encoded.get("task_evidence").is_none());
    assert_eq!(
        serde_json::from_value::<PlanningState>(encoded).unwrap(),
        original
    );
    let TaskEvidenceEvent::Recorded { receipt } = sample_task_evidence_events()[0].clone() else {
        unreachable!()
    };
    let receipt = *receipt;
    let evidence = receipt.evidence;
    let original_scope = evidence.attempt.scope.clone();
    let mut current_scope = original_scope.clone();
    current_scope.scope_id = "f".repeat(64);
    let snapshot = projection::TaskEvidencePlanningSnapshot {
        planning_revision: Some(9),
        evidence_revision: Some(4),
        unavailable: None,
        tasks: vec![projection::TaskEvidenceTaskProjection {
            scope: current_scope.clone(),
            source: None,
            unavailable: None,
            criteria: vec![projection::TaskCriterionEvidenceProjection {
                criterion: crate::domain::planning::TaskAcceptanceCriterion {
                    id: evidence.attempt.criterion_id.clone(),
                    scenario: "Observable result".into(),
                    channel: crate::domain::planning::TaskAcceptanceChannel::Command,
                    essential: true,
                },
                state: Some(TaskCriterionEvidenceState::Stale),
                evidence: Some(projection::TaskEvidenceRecordProjection {
                    event_id: receipt.event_id,
                    evidence_revision: receipt.evidence_revision,
                    scope: original_scope.clone(),
                    criterion_id: evidence.attempt.criterion_id,
                    attempt_id: evidence.attempt.attempt_id,
                    attempt: evidence.attempt.attempt,
                    status: TaskEvidenceStatus::Pass,
                    artifact: evidence.artifact,
                    recorded_tree: evidence.recorded_tree,
                    recorded_at: evidence.recorded_at,
                    by: evidence.by,
                }),
                waiver: None,
            }],
        }],
        truncated: true,
        next_task_id: Some("task-1".into()),
    };
    let state = TaskEvidenceState::from(snapshot);
    assert_eq!(state.tasks[0].scope, current_scope);
    assert_eq!(
        state.tasks[0].criteria[0]
            .evidence
            .as_ref()
            .unwrap()
            .evidence
            .scope,
        original_scope
    );
    assert_eq!(
        state.tasks[0].criteria[0].state,
        Some(TaskCriterionEvidenceState::Stale)
    );
    let encoded = serde_json::to_value(KernelQueryAnswer::TaskEvidence(state)).unwrap();
    assert_matches_spec(
        &serde_json::to_value(describe()).unwrap(),
        "queries",
        "output",
        &encoded,
    );
    let unavailable = serde_json::to_value(TaskEvidenceState::unavailable()).unwrap();
    assert_eq!(unavailable["unavailable"], "unavailable");
    assert_eq!(unavailable["tasks"], json!([]));
}
