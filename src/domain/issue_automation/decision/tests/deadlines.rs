use super::*;

fn deadline_request() -> IssueDecisionRequest {
    let mut definition = definition();
    definition.on_timeout = IssueDecisionTimeout::Default;
    IssueDecisionRequest::new(IssueDecisionId::new("deadline-1").unwrap(), definition).unwrap()
}

fn due_millis() -> u64 {
    chrono::DateTime::parse_from_rfc3339("2026-11-21T12:00:00Z")
        .unwrap()
        .timestamp_millis() as u64
}

#[test]
fn the_deadline_chooses_only_the_recommendation_once_without_human_metadata() {
    let request = deadline_request();
    assert!(
        request
            .timeout_for_gate(
                &basis(),
                IssueAutomationGateAction::TimeoutDefault,
                due_millis() - 1,
            )
            .unwrap()
            .is_none()
    );
    let resolved = request
        .timeout_for_gate(
            &basis(),
            IssueAutomationGateAction::TimeoutDefault,
            due_millis(),
        )
        .unwrap()
        .unwrap();
    assert!(resolved.changed);
    assert_eq!(resolved.outcome, IssueDecisionOutcome::Answered);
    let answer = resolved.request.answer().unwrap();
    assert_eq!(answer.by, IssueDecisionAnswerBy::Default);
    assert_eq!(answer.option, "defer");
    assert_eq!(answer.channel, ISSUE_DECISION_TIMEOUT_CHANNEL);
    assert_eq!(answer.note, None);
    assert_eq!(answer.caller, None);
    assert_eq!(answer.confirmation, None);
    assert_eq!(
        serde_json::from_str::<IssueDecisionRequest>(
            &serde_json::to_string(&resolved.request).unwrap()
        )
        .unwrap(),
        resolved.request,
    );
    let repeated = resolved
        .request
        .timeout_for_gate(
            &basis(),
            IssueAutomationGateAction::TimeoutDefault,
            due_millis() + 1000,
        )
        .unwrap()
        .unwrap();
    assert!(!repeated.changed);
    assert_eq!(repeated.outcome, IssueDecisionOutcome::AlreadyResolved);
    assert_eq!(repeated.request, resolved.request);
}

#[test]
fn human_wait_arbiter_and_incomplete_templates_do_not_default() {
    for gate in [
        IssueAutomationGateAction::Human,
        IssueAutomationGateAction::Auto,
        IssueAutomationGateAction::TimeoutArbiter,
    ] {
        assert!(
            deadline_request()
                .timeout_for_gate(&basis(), gate, due_millis())
                .unwrap()
                .is_none()
        );
    }
    let mut definitions = Vec::new();
    for timeout in [IssueDecisionTimeout::Wait, IssueDecisionTimeout::Arbiter] {
        let mut definition = deadline_request().definition().clone();
        definition.on_timeout = timeout;
        definitions.push(definition);
    }
    let mut definition = deadline_request().definition().clone();
    definition.due_at = None;
    definitions.push(definition);
    let mut definition = deadline_request().definition().clone();
    definition.recommended = None;
    definitions.push(definition);
    for definition in definitions {
        let request = IssueDecisionRequest::new(
            IssueDecisionId::new("waiting-deadline").unwrap(),
            definition,
        )
        .unwrap();
        assert!(
            request
                .timeout_for_gate(
                    &basis(),
                    IssueAutomationGateAction::TimeoutDefault,
                    due_millis(),
                )
                .unwrap()
                .is_none()
        );
        assert_eq!(request.state(), IssueDecisionState::Open);
    }
}

#[test]
fn a_changed_basis_supersedes_before_human_or_deadline_wait() {
    let request = request();
    let mut current = basis();
    current.authorization_epoch += 1;
    let resolved = request
        .timeout_for_gate(&current, IssueAutomationGateAction::Human, due_millis() - 1)
        .unwrap()
        .unwrap();
    assert_eq!(resolved.outcome, IssueDecisionOutcome::Superseded);
    assert_eq!(resolved.request.state(), IssueDecisionState::Superseded);
    assert_eq!(resolved.request.answer(), None);
    assert_eq!(resolved.request.definition(), request.definition());
}

#[test]
fn a_human_answer_before_the_deadline_is_never_replaced_by_default() {
    let request = deadline_request();
    let human = request
        .answer_for_gate(
            &basis(),
            IssueAutomationGateAction::Human,
            input("proceed"),
            Some("desktop-confirmation:0123456789abcdef0123456789abcdef"),
        )
        .unwrap();
    let repeated = human
        .request
        .timeout_for_gate(
            &basis(),
            IssueAutomationGateAction::TimeoutDefault,
            due_millis(),
        )
        .unwrap()
        .unwrap();
    assert!(!repeated.changed);
    assert_eq!(repeated.request, human.request);
    assert_eq!(
        repeated.request.answer().unwrap().by,
        IssueDecisionAnswerBy::Human
    );
}

#[test]
fn timeout_default_requires_human_confirmation_for_an_early_answer() {
    let request = deadline_request();
    assert_eq!(
        request.answer_for_gate(
            &basis(),
            IssueAutomationGateAction::TimeoutDefault,
            input("proceed"),
            None,
        ),
        Err(IssueDecisionError::HumanConfirmationRequired),
    );
    assert_eq!(request.state(), IssueDecisionState::Open);
    assert_eq!(request.answer(), None);
}

#[test]
fn stored_defaults_reject_other_options_early_times_and_caller_metadata() {
    let resolved = deadline_request()
        .timeout_for_gate(
            &basis(),
            IssueAutomationGateAction::TimeoutDefault,
            due_millis(),
        )
        .unwrap()
        .unwrap();
    for case in 0..8 {
        let mut record = resolved.request.to_record();
        let answer = record.answer.as_mut().unwrap();
        match case {
            0 => answer.option = "proceed".into(),
            1 => answer.at = "2026-11-21T11:59:59Z".into(),
            2 => answer.note = Some(shared("invented answer")),
            3 => answer.caller = Some("Desktop".into()),
            4 => answer.channel = "gateway".into(),
            5 => {
                answer.confirmation =
                    Some("desktop-confirmation:0123456789abcdef0123456789abcdef".into())
            }
            6 => record.definition.on_timeout = IssueDecisionTimeout::Wait,
            7 => record.definition.due_at = None,
            _ => unreachable!(),
        }
        assert!(
            IssueDecisionRequest::hydrate(record).is_err(),
            "case {case}"
        );
    }
}

#[test]
fn work_questions_and_close_deadlines_keep_waiting_for_a_human() {
    for kind in [
        IssueDecisionKind::AgentQuestion,
        IssueDecisionKind::FailureRecovery,
    ] {
        let mut definition = deadline_request().definition().clone();
        definition.kind = kind;
        definition.basis.work_order_id = Some("work-1".into());
        let basis = definition.basis.clone();
        let request =
            IssueDecisionRequest::new(IssueDecisionId::new("human-work").unwrap(), definition)
                .unwrap();
        assert!(
            request
                .timeout_for_gate(
                    &basis,
                    IssueAutomationGateAction::TimeoutDefault,
                    due_millis()
                )
                .unwrap()
                .is_none()
        );
    }
    let mut definition = definition();
    definition.kind = IssueDecisionKind::CloseApproval;
    definition.gate = IssueAutomationGate::Close;
    definition.basis.work_order_id = Some("work-1".into());
    let basis = definition.basis.clone();
    let request = IssueDecisionRequest::new(
        IssueDecisionId::new("human-close").unwrap(),
        definition.clone(),
    )
    .unwrap();
    assert!(request.definition().due_at.is_some());
    assert!(
        request
            .timeout_for_gate(
                &basis,
                IssueAutomationGateAction::TimeoutDefault,
                due_millis()
            )
            .unwrap()
            .is_none()
    );
    definition.on_timeout = IssueDecisionTimeout::Default;
    assert!(
        IssueDecisionRequest::new(IssueDecisionId::new("invalid-close").unwrap(), definition)
            .is_err()
    );
}
