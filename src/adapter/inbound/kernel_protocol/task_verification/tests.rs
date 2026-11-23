use super::*;
use crate::adapter::inbound::kernel_protocol::tests::{assert_matches_spec, schema_errors};
use crate::adapter::inbound::kernel_protocol::{
    CommandResult, KernelCommand, KernelCommandInput, TaskEvidenceState, describe,
};
use crate::application::port::inbound::stored_command_port::{StoredCommand, StoredCommandResult};
use crate::domain::kernel_event::{KernelEvent, sample_task_verification_event};
use crate::domain::kernel_spec::{
    CommandCompletion, CommandEffect, CommandIdempotency, CommandState,
};
use crate::domain::planning::evidence::{TaskCriterionEvidenceState, TaskEvidenceEvent};
use crate::domain::planning::verification::{TaskVerificationNotRunReason, TaskVerificationResult};
use serde_json::{Value, json};

fn request() -> Value {
    json!({
        "type":"request_task_verification", "request_id":"host-1", "task_id":"task-1",
        "criterion_id":"criterion with spaces", "scope_id":"b".repeat(64),
        "expected_acceptance_revision":3
    })
}

#[test]
fn host_request_is_keyed_runtime_reservation_with_exact_scope_and_no_execution_inputs() {
    let encoded = request();
    let spec = serde_json::to_value(describe()).unwrap();
    let command: KernelCommand = serde_json::from_value(encoded.clone()).unwrap();
    let contract = command.kind().spec();
    assert_eq!(contract.state, CommandState::Runtime);
    assert_eq!(contract.effect, CommandEffect::Write);
    assert_eq!(contract.idempotency, CommandIdempotency::Keyed);
    assert_eq!(contract.completion, CommandCompletion::Records);
    assert!(!command.confirmed());
    assert_matches_spec(&spec, "commands", "input", &encoded);
    let KernelCommandInput::Stored(stored) =
        command.clone().into_input("/bound-workspace").unwrap()
    else {
        panic!("reservation must use the workspace-bound service");
    };
    assert!(
        matches!(&stored, StoredCommand::RequestTaskVerification { request } if request.scope_id == "b".repeat(64))
    );
    assert_eq!(KernelCommand::from_stored(stored), command);
    let input_schema = &spec["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "request_task_verification")
        .unwrap()["input"];
    for key in [
        "workspace",
        "recipe",
        "status",
        "argv",
        "artifact_path",
        "tree_sha",
        "exit_code",
        "by",
        "owner_token",
        "run_token",
    ] {
        let mut forged = encoded.clone();
        forged[key] = json!("client supplied");
        assert!(
            serde_json::from_value::<KernelCommand>(forged.clone()).is_err(),
            "{key}"
        );
        assert!(
            !schema_errors(input_schema, &forged, &spec["$defs"], "host request").is_empty(),
            "{key}"
        );
    }
    for field in [
        "request_id",
        "task_id",
        "criterion_id",
        "scope_id",
        "expected_acceptance_revision",
    ] {
        let mut missing = encoded.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<KernelCommand>(missing).is_err(),
            "{field}"
        );
    }
}

#[test]
fn host_request_rejects_stale_input_shapes_and_nonexact_tokens_before_claim() {
    for (field, value) in [
        ("scope_id", json!(format!("{}\n", "a".repeat(63)))),
        ("scope_id", json!("A".repeat(64))),
        ("request_id", json!("x".repeat(129))),
        ("request_id", json!("request\n")),
        ("criterion_id", json!(" ")),
        ("expected_acceptance_revision", json!(0)),
        (
            "expected_acceptance_revision",
            json!(9_007_199_254_740_992u64),
        ),
    ] {
        let mut encoded = request();
        encoded[field] = value;
        let command: KernelCommand = serde_json::from_value(encoded).unwrap();
        assert!(command.into_input("/bound-workspace").is_err(), "{field}");
    }
    let result = CommandResult::from_stored(StoredCommandResult::TaskVerificationRequested {
        attempt_id: "a".repeat(64),
        changed: false,
    });
    assert_eq!(
        serde_json::to_value(result).unwrap(),
        json!({
            "type":"task_verification_requested", "attempt_id":"a".repeat(64), "changed":false
        })
    );
}

#[test]
fn host_projection_preserves_original_receipt_scope_and_keeps_reported_status_separate() {
    let TaskEvidenceEvent::VerificationFinished { receipt } = sample_task_verification_event()
    else {
        unreachable!()
    };
    let original_scope = receipt.attempt.scope.clone();
    let mut current_scope = original_scope.clone();
    current_scope.scope_id = "f".repeat(64);
    let snapshot = projection::TaskEvidencePlanningSnapshot {
        planning_revision: Some(7),
        evidence_revision: Some(6),
        unavailable: None,
        truncated: false,
        next_task_id: None,
        tasks: vec![projection::TaskEvidenceTaskProjection {
            scope: current_scope.clone(),
            source: None,
            unavailable: None,
            criteria: vec![projection::TaskCriterionEvidenceProjection {
                criterion: crate::domain::planning::TaskAcceptanceCriterion {
                    id: receipt.attempt.criterion_id.clone(),
                    scenario: "Check the fixed diff".into(),
                    channel: crate::domain::planning::TaskAcceptanceChannel::Command,
                    essential: true,
                    verification_recipe: Some(receipt.attempt.recipe),
                },
                state: Some(TaskCriterionEvidenceState::Pending),
                evidence: None,
                waiver: None,
                host_verification: Some(projection::TaskVerificationProjection {
                    attempt: Some((&receipt.attempt).into()),
                    phase: TaskVerificationPhase::Finished,
                    receipt: Some(receipt.as_ref().into()),
                    state: TaskVerificationCurrentState::Stale,
                    unavailable: None,
                }),
            }],
        }],
    };
    let encoded = serde_json::to_value(TaskEvidenceState::from(snapshot)).unwrap();
    let row = &encoded["tasks"][0]["criteria"][0];
    assert_eq!(
        encoded["tasks"][0]["scope"],
        serde_json::to_value(current_scope).unwrap()
    );
    assert_eq!(row["state"], "pending");
    assert!(row.get("evidence").is_none());
    assert_eq!(row["host_verification"]["state"], "stale");
    assert_eq!(
        row["host_verification"]["receipt"]["verification"]["attempt"]["scope"],
        serde_json::to_value(original_scope).unwrap()
    );
    for secret in [
        "request_id",
        "slot_id",
        "lease_generation",
        "session_key",
        "owner_token",
        "run_token",
        "artifact_path",
        "argv",
    ] {
        assert!(!encoded.to_string().contains(secret), "{secret}");
    }
}

#[test]
fn host_pending_and_unexecuted_facts_keep_optional_fields_and_match_public_schema() {
    let pending: TaskVerificationState = projection::TaskVerificationProjection {
        attempt: None,
        phase: TaskVerificationPhase::Pending,
        receipt: None,
        state: TaskVerificationCurrentState::Pending,
        unavailable: None,
    }
    .into();
    assert_eq!(
        serde_json::to_value(pending).unwrap(),
        json!({"phase":"pending", "state":"pending"})
    );
    let spec = serde_json::to_value(describe()).unwrap();
    let TaskEvidenceEvent::VerificationFinished { receipt } = sample_task_verification_event()
    else {
        unreachable!()
    };
    for result in [
        receipt.result.clone(),
        TaskVerificationResult::NotRun {
            reason: TaskVerificationNotRunReason::Stale,
        },
        TaskVerificationResult::Interrupted {},
    ] {
        let mut receipt = (*receipt).clone();
        if !matches!(&result, TaskVerificationResult::Executed { .. }) {
            receipt.manifest = None;
            receipt.artifact = None;
        }
        receipt.result = result;
        receipt.validate().unwrap();
        let event = KernelEvent::task_evidence(&TaskEvidenceEvent::VerificationFinished {
            receipt: Box::new(receipt),
        });
        assert_matches_spec(
            &spec,
            "records",
            "payload",
            &serde_json::to_value(event).unwrap(),
        );
    }
}

#[test]
fn public_schema_contains_only_the_projected_host_attempt_and_receipt() {
    let spec = serde_json::to_value(describe()).unwrap();
    let definitions = &spec["$defs"];
    for internal in [
        "TaskVerificationAttempt",
        "TaskVerificationReceipt",
        "TaskVerificationExecutionKey",
    ] {
        assert!(
            definitions.get(internal).is_none(),
            "private execution authority leaked as {internal}"
        );
    }
    for (name, expected) in [
        (
            "TaskVerificationAttemptRecord",
            vec![
                "attempt",
                "attempt_id",
                "criterion_id",
                "recipe",
                "requested_at",
                "scope",
            ],
        ),
        (
            "TaskVerificationRecord",
            vec!["artifact", "attempt", "finished_at", "manifest", "result"],
        ),
        (
            "TaskVerificationStream",
            vec!["complete", "sha256", "size_bytes"],
        ),
    ] {
        let definition = &definitions[name];
        let mut fields: Vec<_> = definition["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort_unstable();
        assert_eq!(fields, expected, "{name}");
        assert_eq!(definition["additionalProperties"], false, "{name}");
    }
}
