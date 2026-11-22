use super::*;
use crate::application::service::issue_intervention::recover_pending_controls;
use crate::domain::issue_automation::execution_answer::{
    IssueExecutionAnswerIntent, answer_command_id, answer_hash,
};
use crate::domain::issue_automation::intervention::{
    IssueInterventionAction, IssueInterventionRequest,
};
use std::sync::Barrier;

fn answer_intent(h: &Harness, decision: &StoredIssueDecision) -> IssueExecutionAnswerIntent {
    let work = h.work().unwrap();
    let question = h
        .execution
        .state
        .lock()
        .unwrap()
        .observation
        .as_ref()
        .unwrap()
        .question
        .as_ref()
        .unwrap()
        .clone();
    let intent = IssueExecutionAnswerIntent {
        command_id: answer_command_id(&work.order.work_order_id, decision.request.id()),
        issue: work.order.issue.clone(),
        work_order_id: work.order.work_order_id,
        decision_id: decision.request.id().clone(),
        question,
        authorization_epoch: decision.request.definition().basis.authorization_epoch,
        answer_sha256: answer_hash(REPLY),
    };
    intent.validate().unwrap();
    intent
}

fn claimed_question(
    status: IssueExecutionAnswerStatus,
) -> (Harness, Arc<AnswerExecution>, IssueExecutionAnswerRecord) {
    let (h, execution) = started();
    observe_question(&h);
    track(&h, &execution, true).unwrap();
    let decision = reply(&h);
    let intent = answer_intent(&h, &decision);
    let store = h.fixture.store();
    // Simulate a crash after the durable issue claim and before any execution-port call.
    let claim = store
        .claim_issue_execution_answer(&h.fixture.workspace, &intent)
        .unwrap();
    assert!(claim.created);
    let record = store
        .save_issue_execution_answer(
            &h.fixture.workspace,
            &intent.command_id,
            claim.record.revision,
            &status,
        )
        .unwrap()
        .unwrap();
    assert!(execution.calls.lock().unwrap().is_empty());
    (h, execution, record)
}

fn full_tick(h: &Harness, execution: &Arc<AnswerExecution>) {
    run(&h.triage);
    service(h, execution.clone())
        .tick(&IssueCollectionCancellation::new(), 123, true, &|_| {})
        .unwrap();
}

fn approve_fresh_start(h: &Harness, execution: &Arc<AnswerExecution>) {
    full_tick(h, execution);
    let start = page(&h.triage)
        .items
        .into_iter()
        .find(|item| item.issue == h.fixture.reference("I_001"))
        .unwrap()
        .open_decisions
        .into_iter()
        .find(|decision| decision.request.definition().kind == IssueDecisionKind::StartApproval)
        .expect("the changed authorization requires a new Start decision");
    let result = h
        .decisions
        .answer(
            start.request.id().as_str(),
            "start",
            None,
            &context(h, &start, "start"),
        )
        .unwrap();
    assert_eq!(result.outcome, IssueDecisionOutcome::Answered);
    full_tick(h, execution);
}

fn advance_epoch(h: &Harness, execution: &Arc<AnswerExecution>) {
    let work = h.work().unwrap();
    let next = work.current_authorization_basis().authorization_epoch + 1;
    let store = h.fixture.store();
    store
        .begin_issue_intervention(
            &h.fixture.workspace,
            &IssueInterventionRequest {
                command_id: format!("question-reauthorize-{next}"),
                issue: work.order.issue,
                action: IssueInterventionAction::Retriage,
            },
        )
        .unwrap();
    recover_pending_controls(&h.fixture.workspace, store.as_ref(), h.execution.as_ref()).unwrap();
    approve_fresh_start(h, execution);
    assert_eq!(
        h.work()
            .unwrap()
            .current_authorization_basis()
            .authorization_epoch,
        next,
    );
}

fn assert_original_receipt(h: &Harness, original: &IssueExecutionAnswerRecord) {
    let replay = h
        .fixture
        .store()
        .claim_issue_execution_answer(&h.fixture.workspace, &original.intent)
        .unwrap();
    assert!(!replay.created);
    assert_eq!(&replay.record, original);
}

fn rejected() -> IssueExecutionAnswerStatus {
    IssueExecutionAnswerStatus::RejectedBeforeStart {
        reason: "work_controlled".into(),
    }
}

#[test]
fn a_new_epoch_and_human_answer_cannot_resend_unknown_queued_or_accepted_questions() {
    for status in [
        IssueExecutionAnswerStatus::Unknown,
        IssueExecutionAnswerStatus::Queued,
        IssueExecutionAnswerStatus::Accepted {
            thread_id: "original-thread".into(),
            turn_id: "prior-answer-turn".into(),
            lease_generation: "prior-new-generation".into(),
        },
    ] {
        let (h, execution, original) = claimed_question(status);
        advance_epoch(&h, &execution);
        track(&h, &execution, true).unwrap();
        let renewed = reply(&h);
        let intent = answer_intent(&h, &renewed);
        assert_ne!(intent.decision_id, original.intent.decision_id);
        assert_ne!(intent.command_id, original.intent.command_id);
        assert!(intent.authorization_epoch > original.intent.authorization_epoch);

        let error = track(&h, &execution, true).unwrap_err();
        assert!(format!("{error:#}").contains("already has an unresolved or accepted answer"));
        assert!(execution.calls.lock().unwrap().is_empty());
        assert_eq!(pending(&h).len(), usize::from(original.status.pending()));
        // A failed duplicate does not leave a fresh Unknown row that could be polled forever.
        assert!(
            h.fixture
                .store()
                .claim_issue_execution_answer(&h.fixture.workspace, &intent)
                .is_err()
        );
        assert_original_receipt(&h, &original);
    }
}

#[test]
fn a_rejected_question_requires_a_higher_epoch_even_with_a_new_basis_and_human_answer() {
    let (h, execution, original) = claimed_question(rejected());
    *h.policy.text.lock().unwrap() = AUTO_POLICY.replace("merge = \"auto\"", "merge = \"human\"");
    approve_fresh_start(&h, &execution);
    track(&h, &execution, true).unwrap();
    let renewed = reply(&h);
    let intent = answer_intent(&h, &renewed);
    assert_ne!(intent.decision_id, original.intent.decision_id);
    assert_eq!(
        intent.authorization_epoch,
        original.intent.authorization_epoch
    );

    let error = track(&h, &execution, true).unwrap_err();
    assert!(format!("{error:#}").contains("requires a newer authorization epoch"));
    assert!(execution.calls.lock().unwrap().is_empty());
    assert!(pending(&h).is_empty());
    assert!(
        h.fixture
            .store()
            .claim_issue_execution_answer(&h.fixture.workspace, &intent)
            .is_err()
    );
    assert_original_receipt(&h, &original);
}

#[test]
fn a_settled_rejection_allows_one_new_human_approval_in_a_higher_epoch() {
    let (h, execution, original) = claimed_question(rejected());
    advance_epoch(&h, &execution);
    track(&h, &execution, true).unwrap();
    let open = opened_question(&h);
    let mut unconfirmed = context(&h, &open, "reply");
    unconfirmed.human_confirmation = None;
    assert!(
        h.decisions
            .answer(
                open.request.id().as_str(),
                "reply",
                Some(REPLY),
                &unconfirmed
            )
            .is_err()
    );
    track(&h, &execution, true).unwrap();
    assert!(execution.calls.lock().unwrap().is_empty());
    assert!(pending(&h).is_empty());

    let renewed = reply(&h);
    let intent = answer_intent(&h, &renewed);
    assert_ne!(intent.command_id, original.intent.command_id);
    assert!(intent.authorization_epoch > original.intent.authorization_epoch);
    track(&h, &execution, true).unwrap();
    assert_eq!(execution.calls.lock().unwrap().len(), 1);
    assert_eq!(
        execution.calls.lock().unwrap()[0].command_id,
        intent.command_id
    );
    let current = pending(&h);
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].intent, intent);
    assert_eq!(current[0].status, IssueExecutionAnswerStatus::Queued);
    assert_original_receipt(&h, &original);

    track(&h, &execution, true).unwrap();
    assert_eq!(execution.calls.lock().unwrap().len(), 1);
    assert_original_receipt(&h, &original);
}

#[test]
fn a_rejected_question_cannot_change_full_coordinates_when_reapproved() {
    let (h, execution, original) = claimed_question(rejected());
    advance_epoch(&h, &execution);
    h.execution
        .state
        .lock()
        .unwrap()
        .observation
        .as_mut()
        .unwrap()
        .question
        .as_mut()
        .unwrap()
        .thread_id = "another-thread".into();
    track(&h, &execution, true).unwrap();
    reply(&h);
    let error = track(&h, &execution, true).unwrap_err();
    assert!(format!("{error:#}").contains("question identity changed"));
    assert!(execution.calls.lock().unwrap().is_empty());
    assert!(pending(&h).is_empty());
    assert_original_receipt(&h, &original);
}

#[test]
fn different_questions_keep_independent_answer_claims() {
    let (h, execution, original) = claimed_question(IssueExecutionAnswerStatus::Unknown);
    h.execution
        .state
        .lock()
        .unwrap()
        .observation
        .as_mut()
        .unwrap()
        .question
        .as_mut()
        .unwrap()
        .question_id = "question-2".into();
    track(&h, &execution, true).unwrap();
    let renewed = reply(&h);
    let intent = answer_intent(&h, &renewed);
    let claimed = h
        .fixture
        .store()
        .claim_issue_execution_answer(&h.fixture.workspace, &intent)
        .unwrap();
    assert!(claimed.created);
    assert_eq!(pending(&h).len(), 2);
    assert_original_receipt(&h, &original);
}

#[test]
fn competing_new_human_approvals_after_rejection_claim_only_one_new_delivery() {
    let (h, execution, original) = claimed_question(rejected());
    advance_epoch(&h, &execution);
    track(&h, &execution, true).unwrap();
    let first = reply(&h);
    let mut definition = first.request.definition().clone();
    definition.question =
        IssueDecisionSharedText::new_shared("같은 작업 질문의 새 답변 승인을 확인해 주세요.")
            .unwrap();
    let alternative = h
        .decisions
        .ensure_request(
            IssueDecisionId::new("concurrent-question-approval").unwrap(),
            definition,
        )
        .unwrap()
        .record;
    let second = h
        .decisions
        .answer(
            alternative.request.id().as_str(),
            "reply",
            Some(REPLY),
            &context(&h, &alternative, "reply"),
        )
        .unwrap()
        .commit
        .record;
    let intents = [answer_intent(&h, &first), answer_intent(&h, &second)];
    assert_ne!(intents[0].command_id, intents[1].command_id);
    let barrier = Arc::new(Barrier::new(2));
    let outcomes = std::thread::scope(|scope| {
        let handles: Vec<_> = intents
            .iter()
            .map(|intent| {
                let barrier = barrier.clone();
                let store = h.fixture.store();
                let workspace = h.fixture.workspace.clone();
                scope.spawn(move || {
                    barrier.wait();
                    store.claim_issue_execution_answer(&workspace, intent)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    assert_eq!(
        outcomes.iter().filter(|outcome| outcome.is_err()).count(),
        1
    );
    assert!(
        outcomes
            .iter()
            .filter_map(|outcome| outcome.as_ref().ok())
            .all(|claim| claim.created)
    );
    assert_eq!(pending(&h).len(), 1);
    assert!(execution.calls.lock().unwrap().is_empty());
    assert_original_receipt(&h, &original);
}
