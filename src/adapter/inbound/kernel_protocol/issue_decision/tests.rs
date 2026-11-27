use super::*;
use crate::adapter::inbound::kernel_protocol::{
    CommandRequest, CommandResult, KernelCommand, KernelCommandInput, KernelEvent,
    KernelQueryAnswer, describe,
};
use crate::application::port::inbound::stored_command_port::{StoredCommand, StoredCommandResult};
use crate::domain::issue_automation::IssueAutomationBindingId;
use crate::domain::issue_automation::collection::{IssueCollectionDetail, IssueCollectionItem};
use crate::domain::issue_automation::decision::*;
use crate::domain::issue_automation::policy::IssueAutomationGate;
use crate::domain::issue_work::{IssueFieldHash, IssueProjectScope, IssueProvider, IssueReference};
use crate::domain::kernel_event::KernelEvent as Record;
use serde_json::{Value, json};

fn reference() -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".to_owned(),
        repository_id: "R_decision".to_owned(),
        issue_id: "I_decision".to_owned(),
        project_scope: IssueProjectScope {
            workspace_dir: "/workspace".to_owned(),
        },
    }
}

fn shared(value: &str) -> IssueDecisionSharedText {
    IssueDecisionSharedText::new_shared(value).unwrap()
}

fn request() -> IssueDecisionRequest {
    IssueDecisionRequest::new(
        IssueDecisionId::new("decision-1").unwrap(),
        IssueDecisionDefinition {
            subject: IssueDecisionSubject::Issue(reference()),
            gate: IssueAutomationGate::Start,
            kind: IssueDecisionKind::StartApproval,
            question: shared("공유된 질문 [removed]"),
            options: vec![IssueDecisionOption {
                id: "continue".to_owned(),
                label: shared("계속"),
                effect: shared("계속할 의사를 기록합니다."),
            }],
            recommended: Some("continue".to_owned()),
            basis: IssueDecisionBasis {
                authorization_epoch: 0,
                policy_version: "a".repeat(64),
                field_hash: Some(IssueFieldHash::of_fields("source title", "source body")),
                work_order_id: None,
                gate_revision: 7,
                delivery: None,
            },
            delivery_review: None,
            on_timeout: IssueDecisionTimeout::Wait,
            due_at: None,
            asked_by: "agent-1".to_owned(),
            evidence: vec!["evidence-1".to_owned()],
        },
    )
    .unwrap()
}

fn answer() -> IssueDecisionAgentAnswer {
    IssueDecisionAgentAnswer {
        option: "continue".to_owned(),
        note: Some(shared("answer-note-canary")),
        caller: Some("Desktop".to_owned()),
        channel: "kernel".to_owned(),
        at: "2026-11-20T20:00:00Z".to_owned(),
    }
}

fn answer_wire(note: Option<&str>) -> KernelCommand {
    KernelCommand::AnswerIssueDecision {
        criteria: None,
        decision_id: "decision-1".to_owned(),
        option: "continue".to_owned(),
        note: note.map(ToOwned::to_owned),
    }
}

fn assert_schema(section: &str, name: &str, property: &str, value: &Value) {
    let spec = serde_json::to_value(describe()).unwrap();
    let mut schema = spec[section]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == name)
        .unwrap()[property]
        .clone();
    schema
        .as_object_mut()
        .unwrap()
        .insert("$defs".to_owned(), spec["$defs"].clone());
    let validator = jsonschema::draft202012::options()
        .offline()
        .build(&schema)
        .unwrap();
    let errors: Vec<_> = validator
        .iter_errors(value)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn answer_wire_accepts_only_option_note_and_id_and_maps_to_storage() {
    let command = answer_wire(Some("shared after service policy"));
    assert!(!command.confirmed());
    assert!(matches!(command.into_input("/workspace").unwrap(),
        KernelCommandInput::Stored(StoredCommand::AnswerIssueDecision { decision_id, option, note, criteria: None })
        if decision_id == "decision-1" && option == "continue" && note.is_some()));
    for field in [
        "by",
        "confirmation",
        "caller",
        "channel",
        "basis",
        "gate_revision",
    ] {
        let mut wire = serde_json::to_value(answer_wire(None)).unwrap();
        wire[field] = json!("human");
        assert!(
            serde_json::from_value::<KernelCommand>(wire).is_err(),
            "{field}"
        );
    }
    assert_schema(
        "commands",
        "answer_issue_decision",
        "input",
        &serde_json::to_value(answer_wire(None)).unwrap(),
    );
}

#[test]
fn invalid_answer_inputs_are_bounded_without_echoing_the_original() {
    for (id, option, note) in [
        (" ".to_owned(), "continue".to_owned(), None),
        (
            "decision-1".to_owned(),
            "option with spaces".to_owned(),
            None,
        ),
        ("x".repeat(129), "continue".to_owned(), None),
        ("decision-1".to_owned(), "x".repeat(129), None),
        (
            "decision-1".to_owned(),
            "continue".to_owned(),
            Some("note-canary".repeat(410)),
        ),
        (
            "decision-1".to_owned(),
            "continue".to_owned(),
            Some("note-canary\0".to_owned()),
        ),
    ] {
        let error = KernelCommand::AnswerIssueDecision {
            criteria: None,
            decision_id: id,
            option,
            note,
        }
        .into_input("/workspace")
        .unwrap_err();
        assert_eq!(error.message, "invalid issue decision answer");
        assert!(!format!("{error:?}").contains("note-canary"));
    }
    assert!(validate_answer_input("decision-1", "continue", Some(&"x".repeat(4096))).is_ok());
    // 스키마의 문자열 길이는 글자 수다. 입구는 UTF-8 바이트 상한도 적용한다.
    assert!(validate_answer_input("decision-1", "continue", Some(&"가".repeat(1366))).is_err());
}

#[test]
fn note_changes_the_existing_idempotency_fingerprint_without_entering_the_descriptor() {
    let descriptor = |note| {
        CommandRequest {
            command: answer_wire(Some(note)),
            idempotency_key: Some("same-key".to_owned()),
        }
        .descriptor()
        .unwrap()
        .unwrap()
    };
    let original = descriptor("note-canary");
    assert_eq!(original, descriptor("note-canary"));
    assert_ne!(
        original.fingerprint,
        descriptor("changed-note-canary").fingerprint
    );
    assert!(!format!("{original:?}").contains("note-canary"));
    assert!(original.thread_id.is_empty());
}

#[test]
fn issue_detail_exposes_only_open_shared_requests_at_the_committed_catalog_revision() {
    let open = request();
    let mut closed_record = open.to_record();
    closed_record.id = IssueDecisionId::new("closed-decision").unwrap();
    closed_record.state = IssueDecisionState::Withdrawn;
    let closed = IssueDecisionRequest::hydrate(closed_record).unwrap();
    let detail = IssueCollectionDetail {
        comment_requests: Vec::new(),
        arbiter_waits: Vec::new(),
        timeline: None,
        revision: 41,
        report: None,
        interaction: None,
        execution: None,
        feedback: None,
        item: Some(IssueCollectionItem {
            binding_run_revision: 0,
            binding_run_blocked: false,
            start_budget_wait: None,
            issue: reference(),
            binding_id: IssueAutomationBindingId::new("binding").unwrap(),
            tracked: None,
            current: None,
            observation_revision: None,
            tracked_revision: None,
            triage: None,
            work: None,
            open_decisions: vec![
                StoredIssueDecision {
                    request: open,
                    revision: 3,
                },
                StoredIssueDecision {
                    request: closed,
                    revision: 4,
                },
            ],
        }),
    };
    let wire = serde_json::to_value(KernelQueryAnswer::Issue(
        crate::adapter::inbound::kernel_protocol::IssueState::from_domain(&detail),
    ))
    .unwrap();
    assert_eq!(wire["revision"], 41);
    assert_eq!(wire["open_decisions"].as_array().unwrap().len(), 1);
    assert_eq!(wire["open_decisions"][0]["revision"], 3);
    assert_eq!(
        wire["open_decisions"][0]["question"],
        "공유된 질문 [removed]"
    );
    assert_eq!(wire["open_decisions"][0]["basis"]["gate_revision"], 7);
    for omitted in [
        "answer",
        "note",
        "caller",
        "confirmation",
        "asked_by",
        "evidence",
    ] {
        assert!(wire["open_decisions"][0].get(omitted).is_none());
    }
    assert_schema("queries", "issue", "output", &wire);
}

#[test]
fn decision_events_and_records_never_copy_question_note_or_caller() {
    let open = request();
    let resolved = open
        .answer_agent(&open.definition().basis, answer())
        .unwrap()
        .request;
    let fact = resolved.fact();
    assert_eq!(fact.answer_by, Some(IssueDecisionAnswerBy::Agent));
    let requested = Record::issue_decision_requested(&open.fact(), 41);
    let resolved_record = Record::issue_decision_resolved(&fact, 42);
    let event = KernelEvent::IssueDecision(IssueDecisionChangedState::from_fact(&fact, 42));
    for value in [
        serde_json::to_value(&requested).unwrap(),
        serde_json::to_value(&resolved_record).unwrap(),
        serde_json::to_value(&event).unwrap(),
    ] {
        let text = value.to_string();
        for forbidden in [
            "공유된 질문",
            "answer-note-canary",
            "Desktop",
            "question",
            "note",
            "caller",
            "confirmation",
        ] {
            assert!(!text.contains(forbidden), "{forbidden}");
        }
        assert!(value["decision"].get("enabled").is_none());
    }
    assert_eq!(requested.thread_id(), "");
    assert_eq!(resolved_record.op_id(), None);
    assert_schema(
        "records",
        "issue_decision_requested",
        "payload",
        &serde_json::to_value(requested).unwrap(),
    );
    assert_schema(
        "records",
        "issue_decision_resolved",
        "payload",
        &serde_json::to_value(resolved_record).unwrap(),
    );
    assert_schema(
        "events",
        "issue_decision",
        "payload",
        &serde_json::to_value(event).unwrap(),
    );
}

#[test]
fn superseded_is_a_committed_result_with_a_reason_and_no_applied_answer() {
    let open = request();
    let mut changed = open.definition().basis.clone();
    changed.field_hash = Some(IssueFieldHash::of_fields("changed", "changed"));
    let resolution = open.answer_agent(&changed, answer()).unwrap();
    let fact = resolution.request.fact();
    let wire = CommandResult::from_stored(StoredCommandResult::IssueDecisionResolved {
        fact: Box::new(fact.clone()),
        outcome: resolution.outcome,
        changed: resolution.changed,
        catalog_revision: 42,
    });
    let wire = serde_json::to_value(wire).unwrap();
    assert_eq!(wire["outcome"], "superseded");
    assert_eq!(wire["changed"], true);
    assert_eq!(wire["reason"], "decision_superseded");
    assert_eq!(wire["decision"]["state"], "superseded");
    assert!(wire["decision"]["option"].is_null());
    assert!(wire["decision"]["answer_by"].is_null());
    assert_eq!(wire["catalog_revision"], 42);
    assert_schema("commands", "answer_issue_decision", "result", &wire);
    let event = IssueDecisionChangedState::from_fact(&fact, 42);
    assert_eq!(event.reason, Some(ReasonCode::DecisionSuperseded));
}

#[test]
fn decision_record_preserves_intervention_epoch_and_reads_legacy_zero_epoch() {
    use crate::domain::kernel_event::IssueDecisionBasisRecord;
    let basis = IssueDecisionBasis {
        policy_version: "policy-1".to_owned(),
        field_hash: None,
        work_order_id: Some("work-1".to_owned()),
        gate_revision: 3,
        authorization_epoch: 7,
        delivery: None,
    };
    let record = IssueDecisionBasisRecord::from(&basis);
    let mut wire = serde_json::to_value(record).unwrap();
    assert_eq!(wire["authorization_epoch"], 7);
    wire.as_object_mut().unwrap().remove("authorization_epoch");
    let legacy: IssueDecisionBasisRecord = serde_json::from_value(wire).unwrap();
    assert_eq!(legacy.authorization_epoch, 0);
}

#[test]
fn decision_history_omits_withheld_text_and_keeps_closed_requests_read_only() {
    use crate::adapter::inbound::kernel_protocol::issue_interaction::IssueDecisionHistoryState;
    use crate::domain::issue_automation::interaction::{
        IssueDecisionHistory, IssueDecisionResolutionCode,
    };

    let open = request();
    let answered = open
        .answer_agent(&open.definition().basis, answer())
        .unwrap()
        .request;
    for (resolution, state, expected_code) in [
        (
            IssueDecisionResolutionCode::Answered,
            IssueDecisionState::Answered,
            "answered",
        ),
        (
            IssueDecisionResolutionCode::Expired,
            IssueDecisionState::Expired,
            "expired",
        ),
        (
            IssueDecisionResolutionCode::Withdrawn,
            IssueDecisionState::Withdrawn,
            "withdrawn",
        ),
        (
            IssueDecisionResolutionCode::BasisChanged,
            IssueDecisionState::Superseded,
            "basis_changed",
        ),
    ] {
        let closed = if state == IssueDecisionState::Answered {
            answered.clone()
        } else {
            let mut record = open.to_record();
            record.state = state;
            IssueDecisionRequest::hydrate(record).unwrap()
        };
        let history = IssueDecisionHistory {
            decision: closed.fact(),
            request: Some(StoredIssueDecision {
                request: closed,
                revision: 5,
            }),
            resolution,
        };
        let visible = serde_json::to_value(IssueDecisionHistoryState::from(&history)).unwrap();
        assert_eq!(
            visible["request"]["state"],
            serde_json::to_value(state).unwrap()
        );
        assert_eq!(visible["request"]["question"], "공유된 질문 [removed]");
        assert_eq!(visible["resolution_reason"]["code"], expected_code);
        assert!(visible["resolution_reason"]["summary"].is_string());
        for forbidden in [
            "answer-note-canary",
            "Desktop",
            "asked_by",
            "evidence",
            "confirmation",
            "caller",
        ] {
            assert!(!visible.to_string().contains(forbidden), "{forbidden}");
        }
        let withheld = IssueDecisionHistory {
            request: None,
            ..history
        };
        let opaque = serde_json::to_value(IssueDecisionHistoryState::from(&withheld)).unwrap();
        assert!(opaque.get("request").is_none());
        assert!(opaque["decision"]["decision_id"].is_string());
        for forbidden in [
            "공유된 질문",
            "question",
            "options",
            "recommended",
            "answer-note-canary",
        ] {
            assert!(!opaque.to_string().contains(forbidden), "{forbidden}");
        }
    }
}

fn delivery_request(source: char) -> IssueDecisionRequest {
    let mut record = request().to_record();
    record.definition.kind = IssueDecisionKind::RiskException;
    record.definition.gate = IssueAutomationGate::Delivery;
    record.definition.basis.work_order_id = Some("work-1".to_owned());
    record.definition.basis.delivery = Some(IssueDecisionDeliveryContext {
        base_commit_sha: "a".repeat(40),
        source_commit_sha: source.to_string().repeat(40),
    });
    record.definition.delivery_review = Some(IssueDeliveryDiffVerdict {
        changed_files: 2,
        changed_lines: Some(14),
        reasons: vec![IssueDeliveryRiskReason::ProtectedPath],
    });
    IssueDecisionRequest::hydrate(record).unwrap()
}

#[test]
fn delivery_review_projects_the_frozen_decision_and_preserves_legacy_json() {
    let legacy = serde_json::to_value(IssueDecisionRequestState::from(&StoredIssueDecision {
        request: request(),
        revision: 1,
    }))
    .unwrap();
    assert!(legacy.get("delivery_review").is_none());
    assert!(legacy.get("arbiter_wait").is_none());
    assert!(legacy["basis"].get("delivery").is_none());

    let mut original = delivery_request('b').to_record();
    original.state = IssueDecisionState::Superseded;
    let original = IssueDecisionRequest::hydrate(original).unwrap();
    let current = IssueDecisionRequestState::from(&StoredIssueDecision {
        request: delivery_request('c'),
        revision: 4,
    });
    let history = crate::domain::issue_automation::interaction::IssueDecisionHistory {
        decision: original.fact(),
        request: Some(StoredIssueDecision {
            request: original,
            revision: 3,
        }),
        resolution:
            crate::domain::issue_automation::interaction::IssueDecisionResolutionCode::BasisChanged,
    };
    let history = serde_json::to_value(
        crate::adapter::inbound::kernel_protocol::issue_interaction::IssueDecisionHistoryState::from(&history),
    )
    .unwrap();
    let current = serde_json::to_value(current).unwrap();
    assert_eq!(
        current["basis"]["delivery"]["source_commit_sha"],
        "c".repeat(40)
    );
    assert_eq!(
        history["request"]["basis"]["delivery"]["source_commit_sha"],
        "b".repeat(40)
    );
    assert_eq!(
        history["request"]["basis"]["delivery"]["base_commit_sha"],
        "a".repeat(40)
    );
    assert_eq!(
        history["request"]["delivery_review"],
        json!({
            "changed_files": 2, "changed_lines": 14, "reasons": ["protected_path"]
        })
    );
    assert_eq!(
        history["decision"]["delivery_review"],
        history["request"]["delivery_review"]
    );
    assert_eq!(history["request"]["state"], "superseded");
    assert!(current.get("files").is_none());
}

#[test]
fn native_confirmation_target_roundtrip_keeps_delivery_basis_and_exact_commit_ids() {
    use crate::application::port::inbound::stored_command_port::issue_confirmation::IssueConfirmationTarget;
    let target = IssueConfirmationTarget::Decision {
        criteria_digest: None,
        workspace: "/workspace".to_owned(),
        decision_id: IssueDecisionId::new("delivery-1").unwrap(),
        option: "continue".to_owned(),
        basis: delivery_request('b').definition().basis.clone(),
        decision_revision: 3,
    };
    target.validate().unwrap();
    let mut wire = serde_json::to_value(&target).unwrap();
    assert_eq!(wire["basis"]["delivery"]["baseCommitSha"], "a".repeat(40));
    assert_eq!(
        serde_json::from_value::<IssueConfirmationTarget>(wire.clone()).unwrap(),
        target
    );
    wire["basis"]["delivery"]["sourceCommitSha"] = json!("c".repeat(40));
    let changed: IssueConfirmationTarget = serde_json::from_value(wire.clone()).unwrap();
    changed.validate().unwrap();
    assert_ne!(changed, target);
    wire["basis"]["delivery"]["sourceCommitSha"] = json!("main");
    assert!(
        serde_json::from_value::<IssueConfirmationTarget>(wire)
            .unwrap()
            .validate()
            .is_err()
    );
}

#[test]
fn arbiter_wait_is_projected_only_for_the_exact_open_decision_without_changing_its_definition() {
    use crate::adapter::inbound::kernel_protocol::IssueState;
    use crate::domain::issue_automation::arbiter::{IssueArbiterWaitEntry, IssueArbiterWaitReason};
    let mut record = request().to_record();
    record.definition.on_timeout = IssueDecisionTimeout::Arbiter;
    record.definition.due_at = Some("2026-11-23T00:00:00Z".to_owned());
    let stored = StoredIssueDecision {
        request: IssueDecisionRequest::hydrate(record).unwrap(),
        revision: 3,
    };
    let original = serde_json::to_value(IssueDecisionRequestState::from(&stored)).unwrap();
    assert!(original.get("arbiter_wait").is_none());
    let wait = IssueArbiterWaitEntry {
        decision_id: stored.request.id().clone(),
        decision_revision: 3,
        basis: stored.request.definition().basis.clone(),
        reason: IssueArbiterWaitReason::LowConfidence,
    };
    let detail = IssueCollectionDetail {
        comment_requests: Vec::new(),
        arbiter_waits: vec![wait],
        timeline: None,
        revision: 42,
        report: None,
        interaction: None,
        execution: None,
        feedback: None,
        item: Some(IssueCollectionItem {
            binding_run_revision: 0,
            binding_run_blocked: false,
            start_budget_wait: None,
            issue: reference(),
            binding_id: IssueAutomationBindingId::new("binding").unwrap(),
            tracked: None,
            current: None,
            observation_revision: None,
            tracked_revision: None,
            triage: None,
            work: None,
            open_decisions: vec![stored],
        }),
    };
    let wire =
        serde_json::to_value(KernelQueryAnswer::Issue(IssueState::from_domain(&detail))).unwrap();
    let shown = &wire["open_decisions"][0];
    assert_eq!(wire["revision"], 42);
    assert_eq!(shown["arbiter_wait"], json!({"reason":"low_confidence"}));
    for field in [
        "id",
        "revision",
        "state",
        "on_timeout",
        "basis",
        "due_at",
        "question",
        "options",
    ] {
        assert_eq!(shown[field], original[field], "{field}");
    }
    assert_schema("queries", "issue", "output", &wire);
    for change in 0..4 {
        let mut mismatched = detail.clone();
        match change {
            0 => {
                mismatched.arbiter_waits[0].decision_id =
                    IssueDecisionId::new("other-decision").unwrap()
            }
            1 => mismatched.arbiter_waits[0].decision_revision += 1,
            2 => mismatched.arbiter_waits[0].basis.gate_revision += 1,
            _ => {
                let stored = &mut mismatched.item.as_mut().unwrap().open_decisions[0];
                let mut record = stored.request.to_record();
                record.definition.on_timeout = IssueDecisionTimeout::Wait;
                stored.request = IssueDecisionRequest::hydrate(record).unwrap();
            }
        }
        let wire = serde_json::to_value(IssueState::from_domain(&mismatched)).unwrap();
        assert!(
            wire["open_decisions"][0].get("arbiter_wait").is_none(),
            "{change}"
        );
    }
    for invalid in [
        json!({"reason":"unknown"}),
        json!({"reason":"low_confidence","model_text":"private"}),
    ] {
        assert!(serde_json::from_value::<IssueArbiterWaitState>(invalid).is_err());
    }
}
