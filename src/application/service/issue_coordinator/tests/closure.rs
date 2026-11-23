use super::*;
use crate::application::port::inbound::stored_command_port::{
    StoredCommandContext, issue_confirmation::VerifiedHumanConfirmation,
};
use crate::application::port::outbound::issue_automation_store_port::IssueDecisionFence;
use crate::domain::issue_automation::closure::{close_approval_matches, close_decision_id};
use crate::domain::issue_automation::execution::*;
use crate::domain::issue_automation::policy::IssueAutomationGateAction;

const CLOSE_POLICY: &str = "\n[feedback]\ntransition = true\n[close]\nwhen = \"integration\"\n";

fn started(suffix: &str) -> Harness {
    let h = Harness::new();
    h.policy.text.lock().unwrap().push_str(suffix);
    h.tick(true).unwrap();
    h.answer("start");
    h.tick(true).unwrap();
    h
}

fn integration() -> IssueWorkExecutionObservation {
    IssueWorkExecutionObservation {
        phase: IssueExecutionPhase::Integrated,
        task_ids: vec!["task-1".into()],
        active_task_id: Some("task-1".into()),
        execution_ids: vec!["task-1".into()],
        integration: Some(IssueExecutionIntegration {
            record_id: "close-integration-1".into(),
            source_sha: "a".repeat(40),
            integration_sha: "b".repeat(40),
            pull_request_url: Some("https://github.com/acme/repo/pull/1".into()),
        }),
        ..Default::default()
    }
}

fn track(h: &Harness) -> Vec<IssueCoordinatorFact> {
    let facts = Mutex::new(Vec::new());
    h.coordinator()
        .track_execution(&IssueCollectionCancellation::new(), 123, false, &|values| {
            facts.lock().unwrap().extend(values)
        })
        .unwrap();
    facts.into_inner().unwrap()
}

fn close_requests(h: &Harness) -> Vec<StoredIssueDecision> {
    page(&h.triage).items[0]
        .open_decisions
        .iter()
        .filter(|decision| decision.request.definition().kind == IssueDecisionKind::CloseApproval)
        .cloned()
        .collect()
}

fn execution(h: &Harness) -> IssueExecutionRecord {
    h.fixture
        .store()
        .get_issue_execution(&h.fixture.workspace, &h.fixture.reference("I_001"))
        .unwrap()
        .unwrap()
}

fn answer_context(
    h: &Harness,
    decision: &StoredIssueDecision,
    option: &str,
) -> StoredCommandContext {
    let preview = h
        .decisions
        .prepare_confirmation(decision.request.id().as_str(), option)
        .unwrap();
    StoredCommandContext {
        human_confirmation: Some(
            VerifiedHumanConfirmation::from_verified(
                preview.target,
                format!("desktop-confirmation:{}", "c".repeat(32)),
            )
            .unwrap(),
        ),
        caller: Some("Desktop".into()),
        channel: "gateway".into(),
        at: "2026-11-22T00:00:00Z".into(),
    }
}

#[test]
fn close_approval_is_separate_from_integration_and_reuses_one_persisted_request() {
    let h = started(CLOSE_POLICY);
    assert!(
        close_requests(&h).is_empty(),
        "starting work never approves closing"
    );
    let original = h.work().unwrap();
    h.execution.state.lock().unwrap().observation = Some(integration());
    let facts = track(&h);
    let request = close_requests(&h).pop().unwrap();
    assert_eq!(close_requests(&h).len(), 1);
    assert_eq!(
        request.request.definition().gate,
        IssueAutomationGate::Close
    );
    assert!(request.request.definition().due_at.is_some());
    assert_eq!(
        request.request.definition().on_timeout,
        IssueDecisionTimeout::Wait
    );
    assert_eq!(execution(&h).progress.state, IssueExecutionState::Done);
    assert!(facts.iter().any(
        |fact| matches!(fact, IssueCoordinatorFact::Decision { fact, .. }
        if fact.kind == IssueDecisionKind::CloseApproval)
    ));
    assert!(!close_approval_matches(
        &request.request,
        &h.work().unwrap(),
        &execution(&h),
        &request.request.definition().basis,
    ));
    track(&h);
    assert_eq!(close_requests(&h), vec![request]);
    assert_eq!(
        h.fixture
            .store()
            .get_issue(&h.fixture.workspace, &original.order.issue,)
            .unwrap()
            .unwrap()
            .state(),
        IssueAutomationState::Done
    );
    let state = h.execution.state.lock().unwrap();
    assert_eq!(state.registrations, 1);
    assert_eq!(state.grants.len(), 1);
    assert!(state.recoveries.is_empty());
}

#[test]
fn close_approval_requires_policy_opt_in_and_has_no_request_for_blocked_completion() {
    for suffix in [
        "",
        "\n[feedback]\ntransition = false\n",
        "\n[feedback]\ntransition = true\n[close]\nwhen = \"never\"\n",
    ] {
        let h = started(suffix);
        h.execution.state.lock().unwrap().observation = Some(integration());
        track(&h);
        assert_eq!(execution(&h).progress.state, IssueExecutionState::Done);
        assert!(close_requests(&h).is_empty());
    }
    let h = started(CLOSE_POLICY);
    let mut observed = integration();
    observed.terminals = vec![IssueExecutionTerminalRecord {
        record_id: "blocked-contract".into(),
        classification: IssueExecutionTerminalClass::ContractRecovery,
    }];
    observed.current_terminal_id = Some("blocked-contract".into());
    h.execution.state.lock().unwrap().observation = Some(observed);
    track(&h);
    assert_eq!(execution(&h).progress.state, IssueExecutionState::Done);
    assert!(
        close_requests(&h).is_empty(),
        "contract recovery precedes issue closure"
    );
    assert!(
        page(&h.triage).items[0]
            .open_decisions
            .iter()
            .any(
                |decision| decision.request.definition().kind == IssueDecisionKind::FailureRecovery
            )
    );
}

#[test]
fn close_approval_accepts_only_trusted_human_close_and_preserves_wait() {
    for option in ["close", "wait"] {
        let h = started(CLOSE_POLICY);
        h.execution.state.lock().unwrap().observation = Some(integration());
        track(&h);
        let decision = close_requests(&h).pop().unwrap();
        let unverified = StoredCommandContext {
            human_confirmation: None,
            caller: Some("Desktop".into()),
            channel: "gateway".into(),
            at: "2026-11-22T00:00:00Z".into(),
        };
        assert!(
            h.decisions
                .answer(decision.request.id().as_str(), option, None, &unverified,)
                .is_err()
        );
        let context = answer_context(&h, &decision, option);
        let answer = h
            .decisions
            .answer(decision.request.id().as_str(), option, None, &context)
            .unwrap();
        assert_eq!(answer.outcome, IssueDecisionOutcome::Answered);
        assert_eq!(
            close_approval_matches(
                &answer.commit.record.request,
                &h.work().unwrap(),
                &execution(&h),
                &decision.request.definition().basis,
            ),
            option == "close"
        );
        track(&h);
        assert!(
            close_requests(&h).is_empty(),
            "same answered request is not reopened"
        );
        assert_eq!(execution(&h).progress.state, IssueExecutionState::Done);
    }
}

#[test]
fn close_approval_binds_policy_fields_epoch_gate_work_and_exact_integration_evidence() {
    let h = started(CLOSE_POLICY);
    h.execution.state.lock().unwrap().observation = Some(integration());
    track(&h);
    let decision = close_requests(&h).pop().unwrap();
    let context = answer_context(&h, &decision, "close");
    let answer = h
        .decisions
        .answer(decision.request.id().as_str(), "close", None, &context)
        .unwrap()
        .commit
        .record
        .request;
    let work = h.work().unwrap();
    let record = execution(&h);
    let basis = answer.definition().basis.clone();
    assert!(close_approval_matches(&answer, &work, &record, &basis));
    for changed in [
        IssueDecisionBasis {
            authorization_epoch: basis.authorization_epoch + 1,
            ..basis.clone()
        },
        IssueDecisionBasis {
            policy_version: "c".repeat(64),
            ..basis.clone()
        },
        IssueDecisionBasis {
            field_hash: Some(IssueFieldHash::of_fields("changed", "copy")),
            ..basis.clone()
        },
        IssueDecisionBasis {
            gate_revision: basis.gate_revision + 1,
            ..basis.clone()
        },
        IssueDecisionBasis {
            work_order_id: Some("another-work".into()),
            ..basis.clone()
        },
    ] {
        assert!(!close_approval_matches(&answer, &work, &record, &changed));
    }
    for part in 0..3 {
        let mut changed = record.clone();
        let evidence = changed.progress.integration.as_mut().unwrap();
        match part {
            0 => evidence.record_id = "another-integration".into(),
            1 => evidence.source_sha = "c".repeat(40),
            _ => evidence.integration_sha = "d".repeat(40),
        }
        assert!(!close_approval_matches(&answer, &work, &changed, &basis));
        assert_ne!(
            close_decision_id(&work, &changed, &basis).unwrap(),
            *answer.id()
        );
    }
    let mut wrong_work = work.clone();
    wrong_work.order.work_order_id = "another-work".into();
    assert!(!close_approval_matches(
        &answer,
        &wrong_work,
        &record,
        &basis
    ));
}

#[test]
fn close_approval_policy_change_supersedes_answer_and_does_not_reimplement() {
    let h = started(CLOSE_POLICY);
    h.execution.state.lock().unwrap().observation = Some(integration());
    track(&h);
    let decision = close_requests(&h).pop().unwrap();
    let context = answer_context(&h, &decision, "close");
    *h.policy.text.lock().unwrap() = format!("{AUTO_POLICY}\n[feedback]\ntransition = false\n");
    let result = h
        .decisions
        .answer(decision.request.id().as_str(), "close", None, &context)
        .unwrap();
    assert_eq!(result.outcome, IssueDecisionOutcome::Superseded);
    track(&h);
    assert!(close_requests(&h).is_empty());
    assert_eq!(execution(&h).progress.state, IssueExecutionState::Done);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
}

#[test]
fn close_approval_store_rejects_unverified_answer_and_changed_integration() {
    let h = started(CLOSE_POLICY);
    h.execution.state.lock().unwrap().observation = Some(integration());
    track(&h);
    let decision = close_requests(&h).pop().unwrap();
    let item = page(&h.triage).items.pop().unwrap();
    let collection = h
        .fixture
        .store()
        .get_collection(&h.fixture.workspace)
        .unwrap()
        .unwrap();
    let assessment = h.triage.assess_current(&item).unwrap();
    let forged = decision
        .request
        .answer_agent(
            &decision.request.definition().basis,
            IssueDecisionAgentAnswer {
                option: "close".into(),
                note: None,
                caller: None,
                channel: "test".into(),
                at: "2026-11-22T00:00:00Z".into(),
            },
        )
        .unwrap();
    let fence = IssueDecisionFence {
        triage: IssueTriageFence {
            collection_epoch: collection.epoch,
            observation_revision: item.observation_revision,
            tracked_revision: item.tracked_revision,
            assessment_revision: item.triage.as_ref().map(|triage| triage.revision),
        },
        decision_revision: decision.revision,
    };
    assert!(
        h.fixture
            .store()
            .save_issue_decision(&h.fixture.workspace, fence, &assessment, &forged.request,)
            .is_err(),
        "SQLite boundary independently requires verified human evidence"
    );
    let mut wrong = decision.request.definition().clone();
    wrong.evidence[0] = "integration:other".into();
    assert!(
        h.decisions
            .ensure_request(IssueDecisionId::new("another-close").unwrap(), wrong,)
            .is_err()
    );
    assert_eq!(close_requests(&h), vec![decision]);
    assert_eq!(
        assessment
            .gates()
            .iter()
            .find(|gate| gate.gate == IssueAutomationGate::Close)
            .unwrap()
            .action,
        IssueAutomationGateAction::Human
    );
}

#[test]
fn close_approval_is_not_reopened_after_confirmed_close_even_when_policy_changes() {
    use crate::application::port::outbound::issue_feedback_store_port::IssueFeedbackStorePort;
    use crate::application::service::issue_feedback::tests::support::{
        FakeProvider, FakePublication, FakeStore,
    };
    use crate::domain::issue_automation::feedback::{IssueFeedbackEvidence, IssueFeedbackReceipt};

    let h = started(CLOSE_POLICY);
    h.execution.state.lock().unwrap().observation = Some(integration());
    track(&h);
    let decision = close_requests(&h).pop().unwrap();
    let context = answer_context(&h, &decision, "close");
    h.decisions
        .answer(decision.request.id().as_str(), "close", None, &context)
        .unwrap();
    let issue = h.fixture.reference("I_001");
    let snapshot = h.fixture.store().snapshot(&issue).unwrap().unwrap();
    let feedback_store = Arc::new(FakeStore::new(issue.clone()));
    {
        let mut data = feedback_store.data.lock().unwrap();
        data.snapshot = snapshot;
        data.snapshot.state.close_proof = Some(IssueFeedbackReceipt {
            issue: issue.clone(),
            intent_id: "previous-close".into(),
            actor_id: "U_feedback".into(),
            evidence: IssueFeedbackEvidence::Closed {
                event_id: "previous-close-event".into(),
            },
        });
    }
    let publication = Arc::new(FakePublication::default());
    let provider = Arc::new(FakeProvider::new(
        feedback_store.clone(),
        publication.clone(),
    ));
    let feedback = Arc::new(IssueFeedbackService::new(
        h.fixture.workspace.clone(),
        feedback_store.clone(),
        provider.clone(),
        publication,
        IssueAutomationPolicyService::new(h.policy.clone()),
        IssuePolicyService::new(Arc::new(ValidationPolicy)),
    ));
    let coordinator = h.coordinator().with_feedback(feedback);
    *h.policy.text.lock().unwrap() = format!(
        "{AUTO_POLICY}{}",
        CLOSE_POLICY.replace("transition = true", "comment = true\ntransition = true",)
    );
    let cancel = IssueCollectionCancellation::new();
    super::super::closure::reconcile(&coordinator, &issue, &cancel, 123, &|_| {}).unwrap();
    assert!(
        close_requests(&h).is_empty(),
        "a human-reopened issue receives no new close card"
    );
    assert!(provider.writes.lock().unwrap().is_empty());
    assert_eq!(
        h.fixture
            .store()
            .get_issue_decision(&h.fixture.workspace, decision.request.id(),)
            .unwrap()
            .unwrap()
            .request
            .state(),
        IssueDecisionState::Answered
    );

    feedback_store
        .data
        .lock()
        .unwrap()
        .snapshot
        .state
        .close_proof = None;
    super::super::closure::reconcile(&coordinator, &issue, &cancel, 123, &|_| {}).unwrap();
    assert_eq!(
        close_requests(&h).len(),
        1,
        "the new policy basis would otherwise ask again"
    );
}

#[test]
fn close_approval_arriving_during_feedback_sync_waits_for_a_fresh_source_recheck() {
    use crate::application::port::outbound::issue_feedback_store_port::IssueFeedbackStorePort;
    use crate::application::service::issue_feedback::tests::support::{
        FakeProvider, FakePublication, FakeStore,
    };

    let h = started(CLOSE_POLICY);
    h.execution.state.lock().unwrap().observation = Some(integration());
    track(&h);
    let decision = close_requests(&h).pop().unwrap();
    let context = answer_context(&h, &decision, "close");
    h.decisions
        .answer(decision.request.id().as_str(), "close", None, &context)
        .unwrap();
    let issue = h.fixture.reference("I_001");
    let mut snapshot = h.fixture.store().snapshot(&issue).unwrap().unwrap();
    let approval = snapshot.close_approval.take().unwrap();
    let feedback_store = Arc::new(FakeStore::new(issue.clone()));
    feedback_store.data.lock().unwrap().snapshot = snapshot;
    let completing = feedback_store.clone();
    *feedback_store.after_snapshot.lock().unwrap() = Some(Box::new(move || {
        completing.data.lock().unwrap().snapshot.close_approval = Some(approval);
    }));
    let publication = Arc::new(FakePublication::default());
    let provider = Arc::new(FakeProvider::new(
        feedback_store.clone(),
        publication.clone(),
    ));
    provider.closure.lock().unwrap().issue = issue.clone();
    let feedback = Arc::new(IssueFeedbackService::new(
        h.fixture.workspace.clone(),
        feedback_store.clone(),
        provider.clone(),
        publication,
        IssueAutomationPolicyService::new(h.policy.clone()),
        IssuePolicyService::new(Arc::new(ValidationPolicy)),
    ));
    let coordinator = h.coordinator().with_feedback(feedback);
    let before = h.source.calls.load(Ordering::SeqCst);
    let cancel = IssueCollectionCancellation::new();
    coordinator
        .sync_issue_feedback(&issue, 123, &cancel, &|_| {})
        .unwrap();
    assert!(feedback_store.snapshot_value().authorized_close().is_some());
    assert_eq!(h.source.calls.load(Ordering::SeqCst), before);
    assert!(
        provider.writes.lock().unwrap().is_empty(),
        "the concurrently answered request has no source recheck yet"
    );

    coordinator
        .sync_issue_feedback(&issue, 124, &cancel, &|_| {})
        .unwrap();
    assert!(h.source.calls.load(Ordering::SeqCst) > before);
    assert_eq!(provider.writes.lock().unwrap().len(), 1);
    assert!(feedback_store.snapshot_value().state.close_proof.is_some());
}
