use super::*;
use crate::domain::issue_automation::policy::IssueAutomationPolicy;
use crate::domain::issue_automation::triage::{IssueTriageInput, triage_issue};
use crate::domain::issue_validation::IssueRequirements;
use crate::domain::issue_work::{IssueProjectScope, IssueProvider};

fn shared(value: &str) -> IssueDecisionSharedText {
    IssueDecisionSharedText::new_shared(value).unwrap()
}

fn issue() -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".into(),
        repository_id: "R_decisions".into(),
        issue_id: "I_decision".into(),
        project_scope: IssueProjectScope {
            workspace_dir: "/workspace/decisions".into(),
        },
    }
}

fn basis() -> IssueDecisionBasis {
    IssueDecisionBasis {
        policy_version: "a".repeat(64),
        field_hash: Some(IssueFieldHash::of_fields("shared title", "shared body")),
        work_order_id: None,
        gate_revision: 1,
    }
}

fn definition() -> IssueDecisionDefinition {
    IssueDecisionDefinition {
        subject: IssueDecisionSubject::Issue(issue()),
        gate: IssueAutomationGate::Start,
        kind: IssueDecisionKind::StartApproval,
        question: shared("공유된 질문"),
        options: vec![
            IssueDecisionOption {
                id: "proceed".into(),
                label: shared("진행"),
                effect: shared("진행 의사를 기록한다"),
            },
            IssueDecisionOption {
                id: "defer".into(),
                label: shared("보류"),
                effect: shared("보류 의사를 기록한다"),
            },
        ],
        recommended: Some("defer".into()),
        basis: basis(),
        on_timeout: IssueDecisionTimeout::Wait,
        due_at: Some("2026-11-21T12:00:00Z".into()),
        asked_by: "agent:collector".into(),
        evidence: vec!["triage:1".into()],
    }
}

fn request() -> IssueDecisionRequest {
    IssueDecisionRequest::new(IssueDecisionId::new("decision-1").unwrap(), definition()).unwrap()
}

fn input(option: &str) -> IssueDecisionAgentAnswer {
    IssueDecisionAgentAnswer {
        option: option.into(),
        note: Some(shared("공유된 답변")),
        caller: Some("Desktop".into()),
        channel: "gateway".into(),
        at: "2026-11-20T12:00:00Z".into(),
    }
}

#[test]
fn a_new_request_round_trips_as_an_open_validated_object() {
    let request = request();
    assert_eq!(request.state(), IssueDecisionState::Open);
    assert_eq!(request.answer(), None);
    assert_eq!(request.issue(), &issue());
    let encoded = serde_json::to_string(&request).unwrap();
    let decoded: IssueDecisionRequest = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, request);
}

#[test]
fn desktop_caller_still_records_an_agent_answer_without_confirmation() {
    let request = request();
    let resolution = request.answer_agent(&basis(), input("proceed")).unwrap();
    assert!(resolution.changed);
    assert_eq!(resolution.outcome, IssueDecisionOutcome::Answered);
    assert_eq!(resolution.request.state(), IssueDecisionState::Answered);
    let answer = resolution.request.answer().unwrap();
    assert_eq!(answer.by, IssueDecisionAnswerBy::Agent);
    assert_eq!(answer.confirmation, None);
    assert_eq!(answer.caller.as_deref(), Some("Desktop"));
    assert_eq!(answer.channel, "gateway");
    assert_eq!(answer.option, "proceed");
    resolution.request.validate_successor(&request).unwrap();
}

#[test]
fn the_first_answer_wins_even_when_the_second_answer_and_basis_differ() {
    let first = request()
        .answer_agent(&basis(), input("proceed"))
        .unwrap()
        .request;
    let mut changed = basis();
    changed.gate_revision += 1;
    let second = first.answer_agent(&changed, input("defer")).unwrap();
    assert_eq!(second.outcome, IssueDecisionOutcome::AlreadyResolved);
    assert!(!second.changed);
    assert_eq!(second.request, first);
    assert_eq!(second.request.answer().unwrap().option, "proceed");
}

#[test]
fn every_basis_dimension_can_supersede_without_retaining_the_attempted_answer() {
    let mut changes = Vec::new();
    let mut value = basis();
    value.policy_version = "b".repeat(64);
    changes.push(value);
    let mut value = basis();
    value.field_hash = Some(IssueFieldHash::of_fields("changed", "body"));
    changes.push(value);
    let mut value = basis();
    value.field_hash = None;
    changes.push(value);
    let mut value = basis();
    value.work_order_id = Some("work-order-1".into());
    changes.push(value);
    let mut value = basis();
    value.gate_revision = 2;
    changes.push(value);
    for current in changes {
        let request = request();
        let resolution = request.answer_agent(&current, input("proceed")).unwrap();
        assert!(resolution.changed);
        assert_eq!(resolution.outcome, IssueDecisionOutcome::Superseded);
        assert_eq!(resolution.request.state(), IssueDecisionState::Superseded);
        assert_eq!(resolution.request.answer(), None);
        assert_eq!(resolution.request.definition(), request.definition());
    }
}

#[test]
fn invalid_current_basis_is_an_error_and_does_not_invent_a_superseded_state() {
    let request = request();
    let mut invalid = basis();
    invalid.policy_version.clear();
    assert_eq!(
        request.answer_agent(&invalid, input("proceed")),
        Err(IssueDecisionError::InvalidBasis)
    );
    assert_eq!(request.state(), IssueDecisionState::Open);
}

#[test]
fn an_unknown_option_is_rejected_without_echoing_the_input() {
    let secret = "sensitive-unrecognized-option";
    let result = request().answer_agent(&basis(), input(secret));
    assert_eq!(result, Err(IssueDecisionError::InvalidOption));
    assert!(!result.unwrap_err().to_string().contains(secret));
}

#[test]
fn hydration_rejects_an_answer_on_any_state_except_answered() {
    let answered = request()
        .answer_agent(&basis(), input("proceed"))
        .unwrap()
        .request;
    for state in [
        IssueDecisionState::Open,
        IssueDecisionState::Expired,
        IssueDecisionState::Withdrawn,
        IssueDecisionState::Superseded,
    ] {
        let mut record = answered.to_record();
        record.state = state;
        assert_eq!(
            IssueDecisionRequest::hydrate(record),
            Err(IssueDecisionError::InvalidState)
        );
    }
    let mut record = answered.to_record();
    record.answer = None;
    assert_eq!(
        IssueDecisionRequest::hydrate(record),
        Err(IssueDecisionError::InvalidState)
    );
}

#[test]
fn hydration_cannot_forge_human_default_or_arbiter_answers() {
    let answered = request()
        .answer_agent(&basis(), input("proceed"))
        .unwrap()
        .request;
    for by in [
        IssueDecisionAnswerBy::Human,
        IssueDecisionAnswerBy::Default,
        IssueDecisionAnswerBy::Arbiter,
    ] {
        let mut record = answered.to_record();
        record.answer.as_mut().unwrap().by = by;
        assert_eq!(
            IssueDecisionRequest::hydrate(record),
            Err(IssueDecisionError::UnsupportedAnswerOrigin)
        );
    }
    let mut record = answered.to_record();
    record.answer.as_mut().unwrap().confirmation = Some("forged-confirmation".into());
    assert_eq!(
        IssueDecisionRequest::hydrate(record),
        Err(IssueDecisionError::UnsupportedAnswerOrigin)
    );
}

#[test]
fn closed_requests_cannot_reopen_or_replace_the_first_answer() {
    let first = request()
        .answer_agent(&basis(), input("proceed"))
        .unwrap()
        .request;
    let replacement = request()
        .answer_agent(&basis(), input("defer"))
        .unwrap()
        .request;
    assert_eq!(
        replacement.validate_successor(&first),
        Err(IssueDecisionError::AlreadyClosed)
    );
    assert_eq!(
        request().validate_successor(&first),
        Err(IssueDecisionError::AlreadyClosed)
    );
    for state in [
        IssueDecisionState::Expired,
        IssueDecisionState::Withdrawn,
        IssueDecisionState::Superseded,
    ] {
        let mut record = request().to_record();
        record.state = state;
        let closed = IssueDecisionRequest::hydrate(record).unwrap();
        let second = closed.answer_agent(&basis(), input("proceed")).unwrap();
        assert_eq!(second.request, closed);
        assert!(!second.changed);
        assert_eq!(
            request().validate_successor(&closed),
            Err(IssueDecisionError::AlreadyClosed)
        );
    }
}

#[test]
fn a_request_definition_and_identity_are_immutable_across_storage_updates() {
    let first = request();
    let mut changed = definition();
    changed.question = shared("다른 질문");
    let replacement = IssueDecisionRequest::new(first.id().clone(), changed).unwrap();
    assert_eq!(
        replacement.validate_successor(&first),
        Err(IssueDecisionError::DefinitionChanged)
    );
    let replacement =
        IssueDecisionRequest::new(IssueDecisionId::new("decision-2").unwrap(), definition())
            .unwrap();
    assert_eq!(
        replacement.validate_successor(&first),
        Err(IssueDecisionError::DefinitionChanged)
    );
}

#[test]
fn repeated_requests_share_a_key_even_when_the_caller_generates_another_id() {
    let first = request();
    let second =
        IssueDecisionRequest::new(IssueDecisionId::new("decision-2").unwrap(), definition())
            .unwrap();
    assert_eq!(first.uniqueness_key(), second.uniqueness_key());
    assert_eq!(first.uniqueness_key().len(), 64);
    let mut altered = definition();
    altered.question = shared("같은 판의 질문만 바꾸었다");
    let altered = IssueDecisionRequest::new(first.id().clone(), altered).unwrap();
    assert_ne!(first.uniqueness_key(), altered.uniqueness_key());
    assert_eq!(
        altered.validate_successor(&first),
        Err(IssueDecisionError::DefinitionChanged)
    );
}

#[test]
fn the_deduplication_key_keeps_subject_scope_kind_gate_and_basis_separate() {
    let first = request();
    let mut variants = Vec::new();
    let mut value = definition();
    let IssueDecisionSubject::Issue(issue) = &mut value.subject;
    issue.project_scope.workspace_dir = "/workspace/another".into();
    variants.push(value);
    let mut value = definition();
    let IssueDecisionSubject::Issue(issue) = &mut value.subject;
    issue.issue_id = "I_another".into();
    variants.push(value);
    let mut value = definition();
    value.kind = IssueDecisionKind::RiskException;
    variants.push(value);
    let mut value = definition();
    value.kind = IssueDecisionKind::PlanApproval;
    value.gate = IssueAutomationGate::Plan;
    variants.push(value);
    let mut value = definition();
    value.basis.gate_revision += 1;
    variants.push(value);
    for variant in variants {
        let request = IssueDecisionRequest::new(first.id().clone(), variant).unwrap();
        assert_ne!(first.uniqueness_key(), request.uniqueness_key());
    }
}

#[test]
fn facts_contain_no_question_note_caller_or_evidence_text() {
    let mut definition = definition();
    definition.question = shared("QUESTION-CANARY");
    definition.asked_by = "ASKED-BY-CANARY".into();
    definition.evidence = vec!["EVIDENCE-CANARY".into()];
    definition.options[0].label = shared("LABEL-CANARY");
    definition.options[0].effect = shared("EFFECT-CANARY");
    let request =
        IssueDecisionRequest::new(IssueDecisionId::new("decision-1").unwrap(), definition).unwrap();
    let mut answer = input("proceed");
    answer.note = Some(shared("NOTE-CANARY"));
    answer.caller = Some("CALLER-CANARY".into());
    let answered = request.answer_agent(&basis(), answer).unwrap().request;
    let encoded = serde_json::to_string(&answered.fact()).unwrap();
    for canary in [
        "QUESTION-CANARY",
        "ASKED-BY-CANARY",
        "EVIDENCE-CANARY",
        "LABEL-CANARY",
        "EFFECT-CANARY",
        "NOTE-CANARY",
        "CALLER-CANARY",
    ] {
        assert!(!encoded.contains(canary));
    }
    assert_eq!(
        answered.fact().answer_by,
        Some(IssueDecisionAnswerBy::Agent)
    );
    assert_eq!(answered.fact().option.as_deref(), Some("proceed"));
}

#[test]
fn definitions_reject_duplicate_options_and_missing_recommendations() {
    let mut duplicate = definition();
    duplicate.options.push(duplicate.options[0].clone());
    assert!(
        IssueDecisionRequest::new(IssueDecisionId::new("decision-1").unwrap(), duplicate).is_err()
    );
    let mut missing = definition();
    missing.recommended = Some("absent".into());
    assert_eq!(
        IssueDecisionRequest::new(IssueDecisionId::new("decision-1").unwrap(), missing),
        Err(IssueDecisionError::InvalidOption)
    );
}

#[test]
fn definitions_bound_text_identifiers_and_reject_invalid_deadlines() {
    assert!(IssueDecisionId::new("").is_err());
    assert!(IssueDecisionId::new("with whitespace").is_err());
    assert!(IssueDecisionId::new("x".repeat(MAX_ISSUE_DECISION_ID_BYTES + 1)).is_err());
    assert!(IssueDecisionSharedText::new_shared("   ").is_err());
    assert!(IssueDecisionSharedText::new_shared("control\0text").is_err());
    let mut invalid = definition();
    invalid.due_at = Some("not-a-date".into());
    assert!(
        IssueDecisionRequest::new(IssueDecisionId::new("decision-1").unwrap(), invalid).is_err()
    );
    let mut invalid = definition();
    invalid.asked_by = "free form text".into();
    assert!(
        IssueDecisionRequest::new(IssueDecisionId::new("decision-1").unwrap(), invalid).is_err()
    );
    let mut invalid = definition();
    invalid.evidence.push(invalid.evidence[0].clone());
    assert!(
        IssueDecisionRequest::new(IssueDecisionId::new("decision-1").unwrap(), invalid).is_err()
    );
}

#[test]
fn notes_and_answer_metadata_are_bounded_without_leaking_values_in_errors() {
    let mut answer = input("proceed");
    answer.note = Some(shared(&"x".repeat(MAX_ISSUE_DECISION_NOTE_BYTES + 1)));
    assert!(request().answer_agent(&basis(), answer).is_err());
    let mut answer = input("proceed");
    answer.at = "invalid-time-canary".into();
    let error = request().answer_agent(&basis(), answer).unwrap_err();
    assert!(!error.to_string().contains("invalid-time-canary"));
    let mut answer = input("proceed");
    answer.channel = "untrusted channel".into();
    assert!(request().answer_agent(&basis(), answer).is_err());
}

#[test]
fn kind_timeout_and_state_deserialization_use_closed_vocabularies() {
    assert_eq!(IssueDecisionKind::ALL.len(), 7);
    assert!(serde_json::from_str::<IssueDecisionKind>("\"execute_shell\"").is_err());
    assert!(serde_json::from_str::<IssueDecisionTimeout>("\"execute\"").is_err());
    assert!(serde_json::from_str::<IssueDecisionState>("\"running\"").is_err());
    assert!(serde_json::from_str::<IssueDecisionAnswerBy>("\"Desktop\"").is_err());
}

#[test]
fn restored_store_revisions_are_validated() {
    for revision in [0, MAX_ISSUE_AUTOMATION_REVISION + 1] {
        assert!(
            StoredIssueDecision {
                request: request(),
                revision
            }
            .validate()
            .is_err()
        );
    }
    StoredIssueDecision {
        request: request(),
        revision: 1,
    }
    .validate()
    .unwrap();
}

#[test]
fn a_fresh_assessment_gate_revision_is_part_of_the_decision_basis() {
    let issue = issue();
    let policy = IssueAutomationPolicy::builtin();
    let requirements = IssueRequirements::default();
    let version = "a".repeat(64);
    let validation_version = "b".repeat(64);
    let first = triage_issue(IssueTriageInput {
        issue: &issue,
        current: None,
        policy: &policy,
        automation_policy_version: &version,
        requirements: &requirements,
        validation_policy_version: &validation_version,
        held: false,
        pinned_source_commit: None,
        previous: None,
    })
    .unwrap();
    let after = triage_issue(IssueTriageInput {
        issue: &issue,
        current: None,
        policy: &policy,
        automation_policy_version: &version,
        requirements: &requirements,
        validation_policy_version: &validation_version,
        held: true,
        pinned_source_commit: None,
        previous: Some(&first),
    })
    .unwrap();
    let before =
        IssueDecisionBasis::from_assessment(&first, IssueAutomationGate::Start, None).unwrap();
    let after =
        IssueDecisionBasis::from_assessment(&after, IssueAutomationGate::Start, None).unwrap();
    assert_eq!(before.field_hash, None);
    assert_eq!(before.policy_version, after.policy_version);
    assert_ne!(before.gate_revision, after.gate_revision);
}

#[test]
fn different_agent_questions_on_the_same_basis_keep_distinct_identities() {
    let mut first = definition();
    first.kind = IssueDecisionKind::AgentQuestion;
    let mut second = first.clone();
    second.question = shared("같은 판에서 다른 선택을 묻는 질문");
    let first =
        IssueDecisionRequest::new(IssueDecisionId::new("question-1").unwrap(), first).unwrap();
    let second =
        IssueDecisionRequest::new(IssueDecisionId::new("question-2").unwrap(), second).unwrap();
    assert_eq!(first.definition().basis, second.definition().basis);
    assert_ne!(first.uniqueness_key(), second.uniqueness_key());
}

#[test]
fn a_human_gate_rejects_an_unverified_agent_without_closing_the_request() {
    let request = request();
    assert_eq!(
        request.answer_for_gate(
            &basis(),
            IssueAutomationGateAction::Human,
            input("proceed"),
            None
        ),
        Err(IssueDecisionError::HumanConfirmationRequired),
    );
    assert_eq!(request.state(), IssueDecisionState::Open);
    assert_eq!(request.answer(), None);
}

#[test]
fn a_verified_human_answer_stores_only_its_audit_reference_and_survives_hydration() {
    let reference = format!("desktop-confirmation:{}", "c".repeat(32));
    let request = request();
    let result = request
        .answer_for_gate(
            &basis(),
            IssueAutomationGateAction::Human,
            input("proceed"),
            Some(&reference),
        )
        .unwrap();
    let answer = result.request.answer().unwrap();
    assert_eq!(answer.by, IssueDecisionAnswerBy::Human);
    assert_eq!(answer.confirmation.as_deref(), Some(reference.as_str()));
    assert_eq!(
        IssueDecisionRequest::hydrate(result.request.to_record()).unwrap(),
        result.request
    );
    result.request.validate_successor(&request).unwrap();
    let repeated = result
        .request
        .answer_agent(&basis(), input("defer"))
        .unwrap();
    assert_eq!(repeated.request, result.request);
    assert!(!repeated.changed);
}

#[test]
fn stale_human_requests_are_superseded_without_a_confirmation_or_answer() {
    let mut current = basis();
    current.gate_revision += 1;
    let result = request()
        .answer_for_gate(
            &current,
            IssueAutomationGateAction::Human,
            input("proceed"),
            None,
        )
        .unwrap();
    assert_eq!(result.outcome, IssueDecisionOutcome::Superseded);
    assert!(result.request.answer().is_none());
}

#[test]
fn human_history_rejects_missing_or_malformed_audit_references() {
    let reference = format!("desktop-confirmation:{}", "d".repeat(32));
    let valid = request()
        .answer_for_gate(
            &basis(),
            IssueAutomationGateAction::Human,
            input("proceed"),
            Some(&reference),
        )
        .unwrap()
        .request;
    for value in [
        None,
        Some("raw-proof-canary".into()),
        Some("desktop-confirmation:wrong".into()),
    ] {
        let mut record = valid.to_record();
        record.answer.as_mut().unwrap().confirmation = value;
        assert_eq!(
            IssueDecisionRequest::hydrate(record),
            Err(IssueDecisionError::UnsupportedAnswerOrigin)
        );
    }
    let mut record = valid.to_record();
    record.answer.as_mut().unwrap().by = IssueDecisionAnswerBy::Agent;
    assert_eq!(
        IssueDecisionRequest::hydrate(record),
        Err(IssueDecisionError::UnsupportedAnswerOrigin)
    );
}
