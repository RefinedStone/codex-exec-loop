use super::*;
use crate::application::port::outbound::issue_automation_store_port::{
    IssueDecisionFence, IssueDecisionSaveResult,
};
use crate::domain::issue_automation::closure::{close_decision_evidence, close_decision_id};
use crate::domain::issue_automation::decision::*;
use crate::domain::issue_automation::policy::IssueAutomationGateAction;

fn decision_fence(snapshot: &IssueFeedbackSnapshot) -> IssueTriageFence {
    IssueTriageFence {
        binding_run_revision: snapshot.item.binding_run_revision,
        collection_epoch: snapshot.collection.epoch,
        observation_revision: snapshot.item.observation_revision,
        tracked_revision: snapshot.item.tracked_revision,
        assessment_revision: snapshot.item.triage.as_ref().map(|value| value.revision),
    }
}

fn approve_close(fixture: &Fixture, issue: &IssueReference) {
    let adapter = fixture.adapter();
    let snapshot = current(&adapter, issue);
    let work = snapshot.work.as_ref().unwrap();
    let execution = snapshot.execution.as_ref().unwrap();
    let assessment = &snapshot.item.triage.as_ref().unwrap().assessment;
    let mut basis = IssueDecisionBasis::from_assessment(
        assessment,
        IssueAutomationGate::Close,
        Some(work.order.work_order_id.clone()),
    )
    .unwrap();
    basis.authorization_epoch = snapshot.authorization_epoch;
    let request = IssueDecisionRequest::new(
        close_decision_id(work, execution, &basis).unwrap(),
        IssueDecisionDefinition {
            delivery_review: None,
            subject: IssueDecisionSubject::Issue(issue.clone()),
            gate: IssueAutomationGate::Close,
            kind: IssueDecisionKind::CloseApproval,
            question: IssueDecisionSharedText::new_shared("통합 결과를 확인하고 이슈를 닫을까요?")
                .unwrap(),
            options: ["close", "wait"]
                .map(|id| IssueDecisionOption {
                    id: id.into(),
                    label: IssueDecisionSharedText::new_shared(id).unwrap(),
                    effect: IssueDecisionSharedText::new_shared(id).unwrap(),
                })
                .to_vec(),
            recommended: Some("wait".into()),
            basis: basis.clone(),
            on_timeout: IssueDecisionTimeout::Wait,
            due_at: None,
            asked_by: "host".into(),
            evidence: close_decision_evidence(execution).unwrap(),
        },
    )
    .unwrap();
    let IssueDecisionSaveResult::Saved(created) = adapter
        .ensure_issue_decision(
            fixture.cwd(),
            decision_fence(&snapshot),
            assessment,
            &request,
        )
        .unwrap()
    else {
        panic!("close request was not recorded")
    };
    let answered = request
        .answer_for_gate(
            &basis,
            IssueAutomationGateAction::Human,
            IssueDecisionAgentAnswer {
                option: "close".into(),
                note: None,
                caller: None,
                channel: "desktop".into(),
                at: "2026-11-22T00:00:00Z".into(),
            },
            Some("desktop-confirmation:0123456789abcdef0123456789abcdef"),
        )
        .unwrap()
        .request;
    assert!(matches!(
        adapter
            .save_issue_decision(
                fixture.cwd(),
                IssueDecisionFence {
                    triage: decision_fence(&current(&adapter, issue)),
                    decision_revision: created.record.revision
                },
                assessment,
                &answered,
            )
            .unwrap(),
        IssueDecisionSaveResult::Saved(_)
    ));
    assert!(current(&adapter, issue).authorized_close().is_some());
}

fn close_intent(snapshot: &IssueFeedbackSnapshot, id: &str) -> IssueFeedbackIntent {
    let work = snapshot.work.as_ref().unwrap();
    let execution = snapshot.execution.as_ref().unwrap();
    let integration = execution.progress.integration.as_ref().unwrap();
    let mut basis = IssueDecisionBasis::from_assessment(
        &snapshot.item.triage.as_ref().unwrap().assessment,
        IssueAutomationGate::Close,
        Some(work.order.work_order_id.clone()),
    )
    .unwrap();
    basis.authorization_epoch = snapshot.authorization_epoch;
    make_intent(
        snapshot,
        id,
        IssueFeedbackOperation::CloseIssue {
            integration_record_id: integration.record_id.clone(),
            integration_sha: integration.integration_sha.clone(),
            decision_id: close_decision_id(work, execution, &basis)
                .unwrap()
                .as_str()
                .into(),
            jira_transition: None,
            previous_event_id: None,
        },
    )
}

#[test]
fn close_intent_requires_the_durable_human_answer_in_the_same_transaction() {
    let fixture = Fixture::new("feedback-close-approval");
    let issue = seed(&fixture, "close-approval");
    let work = existing_work(&fixture, &issue);
    integrate(&fixture, &work);
    refresh_assessment(&fixture, &issue);
    let adapter = fixture.adapter();
    let snapshot = current(&adapter, &issue);
    let intent = close_intent(&snapshot, "close-intent");
    assert_eq!(
        adapter.record(snapshot.fence(), &intent).unwrap(),
        IssueFeedbackRecordResult::Conflict
    );
    assert_eq!(count(&fixture, "issue_feedback_intents"), 0);
    approve_close(&fixture, &issue);
    let recorded = recorded(&adapter, &intent);
    let sent = sent(&adapter, &recorded);
    let receipt = IssueFeedbackReceipt {
        issue: issue.clone(),
        intent_id: intent.intent_id.clone(),
        actor_id: intent.actor_id.clone(),
        evidence: IssueFeedbackEvidence::Closed {
            event_id: "CE_confirmed".into(),
        },
    };
    assert!(
        adapter
            .complete(
                &issue,
                &intent.intent_id,
                sent.revision,
                &IssueFeedbackCompletion::Confirmed(Box::new(receipt.clone()))
            )
            .is_err()
    );
    let acknowledged = adapter
        .complete(
            &issue,
            &intent.intent_id,
            sent.revision,
            &IssueFeedbackCompletion::CloseAcknowledged,
        )
        .unwrap()
        .unwrap();
    assert!(acknowledged.closure_acknowledged);
    let completed = confirmed(&adapter, &acknowledged, receipt.clone());
    assert_eq!(completed.state, IssueFeedbackIntentState::Confirmed);
    drop(adapter);
    let reopened = fixture.adapter();
    assert_eq!(current(&reopened, &issue).state.close_proof, Some(receipt));
    let another = close_intent(&current(&reopened, &issue), "close-another");
    assert_eq!(
        reopened
            .record(current(&reopened, &issue).fence(), &another)
            .unwrap(),
        IssueFeedbackRecordResult::Conflict
    );
    assert_eq!(count(&fixture, "issue_feedback_outbox"), 1);
}

#[test]
fn remote_source_change_between_record_and_claim_prevents_the_close_send() {
    let fixture = Fixture::new("feedback-close-source");
    let issue = seed(&fixture, "close-source");
    let work = existing_work(&fixture, &issue);
    integrate(&fixture, &work);
    refresh_assessment(&fixture, &issue);
    approve_close(&fixture, &issue);
    let adapter = fixture.adapter();
    let snapshot = current(&adapter, &issue);
    let intent = close_intent(&snapshot, "close-source-intent");
    let recorded = recorded(&adapter, &intent);
    let mut observed = match snapshot.item.current.unwrap() {
        IssueSourceRead::Observed(observed) => *observed,
        _ => panic!("source missing"),
    };
    if let IssueSourceContent::Shared(shared) = &mut observed.content {
        shared
            .shared_copy
            .body
            .push_str("\n추가 확인이 필요합니다.");
        shared.basis.field_hash = crate::domain::issue_work::IssueFieldHash::of_fields(
            &shared.shared_copy.title,
            &shared.shared_copy.body,
        );
    }
    adapter
        .apply_issue_resync(
            fixture.cwd(),
            snapshot.collection.fence(),
            &issue,
            &IssueSourceRead::Observed(Box::new(observed)),
        )
        .unwrap();
    let changed = current(&adapter, &issue);
    assert!(changed.authorized_close().is_none());
    assert_eq!(
        adapter
            .claim_sent(
                &issue,
                &intent.intent_id,
                recorded.revision,
                changed.fence()
            )
            .unwrap(),
        IssueFeedbackClaimResult::Conflict
    );
    assert_eq!(count(&fixture, "issue_feedback_intents"), 1);
}
