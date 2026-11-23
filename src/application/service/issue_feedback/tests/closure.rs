use super::*;
use crate::application::service::issue_automation_policy::resolve_issue_automation_policy;
use crate::domain::issue_automation::closure::{close_decision_evidence, close_decision_id};
use crate::domain::issue_automation::collection::IssueTriageProjection;
use crate::domain::issue_automation::decision::*;
use crate::domain::issue_automation::execution::IssueExecutionIntegration;
use crate::domain::issue_automation::policy::{IssueAutomationGate, IssueAutomationGateAction};
use crate::domain::issue_automation::triage::{IssueTriageInput, triage_issue};
use crate::domain::issue_validation::IssueRequirements;

fn integrated() -> Fixture {
    let fixture = Fixture::new(false, false);
    *fixture.policy.user.lock().unwrap() =
        "[feedback]\ntransition = true\n[close]\nwhen = \"integration\"\n".into();
    let file_policy =
        resolve_issue_automation_policy(Some(fixture.policy.user.lock().unwrap().as_str()), None)
            .unwrap();
    let mut data = fixture.store.data.lock().unwrap();
    let snapshot = &mut data.snapshot;
    let policy = crate::application::service::issue_autonomy::effective_policy(
        file_policy,
        &snapshot.autonomy,
    );
    snapshot.item.tracked.as_mut().unwrap().state = IssueAutomationState::Done;
    let work = snapshot.work.as_mut().unwrap();
    work.order.work_order_id =
        crate::domain::issue_automation::coordinator::issue_work_order_id(&work.order.issue);
    work.order.scope = work.order.issue.project_scope.workspace_dir.clone();
    work.validate().unwrap();
    let execution = snapshot.execution.as_mut().unwrap();
    execution.work_order_id = work.order.work_order_id.clone();
    execution.progress.state = IssueExecutionState::Done;
    execution.progress.integration = Some(IssueExecutionIntegration {
        record_id: "integration-close".into(),
        source_sha: "1".repeat(40),
        integration_sha: "2".repeat(40),
        pull_request_url: Some("https://github.com/owner/project/pull/1".into()),
    });
    let assessment = triage_issue(IssueTriageInput {
        issue: &snapshot.item.issue,
        current: snapshot.item.current.as_ref(),
        policy: &policy.policy,
        automation_policy_version: &policy.version,
        runtime_selection_revision: snapshot.autonomy.revision,
        requirements: &IssueRequirements::default(),
        validation_policy_version: &issue_policy_version(&IssuePolicy::builtin()),
        held: false,
        pinned_source_commit: None,
        previous: None,
    })
    .unwrap();
    snapshot.item.triage = Some(IssueTriageProjection {
        assessment,
        revision: 1,
        observation_revision: snapshot.item.observation_revision,
        tracked_revision: snapshot.item.tracked_revision,
        stale: false,
    });
    drop(data);
    fixture
}

fn approve(fixture: &Fixture) {
    let mut data = fixture.store.data.lock().unwrap();
    let snapshot = &mut data.snapshot;
    let work = snapshot.work.as_ref().unwrap();
    let execution = snapshot.execution.as_ref().unwrap();
    let mut basis = IssueDecisionBasis::from_assessment(
        &snapshot.item.triage.as_ref().unwrap().assessment,
        IssueAutomationGate::Close,
        Some(work.order.work_order_id.clone()),
    )
    .unwrap();
    basis.authorization_epoch = snapshot.authorization_epoch;
    let request = IssueDecisionRequest::new(
        close_decision_id(work, execution, &basis).unwrap(),
        IssueDecisionDefinition {
            delivery_review: None,
            subject: IssueDecisionSubject::Issue(issue()),
            gate: IssueAutomationGate::Close,
            kind: IssueDecisionKind::CloseApproval,
            question: IssueDecisionSharedText::new_shared("통합한 이슈를 닫을까요?").unwrap(),
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
    .unwrap()
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
    snapshot.close_approval = Some(StoredIssueDecision {
        request,
        revision: 2,
    });
    assert!(snapshot.authorized_close().is_some());
}

// Service tests provide the exact approval whose source recheck the host has completed.
fn sync_after_recheck(fixture: &Fixture) -> Result<()> {
    let approval = fixture.store.snapshot_value().close_approval;
    if let Some(approval) = approval {
        fixture.service().sync_issue_after_close_recheck(
            &issue(),
            123,
            &IssueCollectionCancellation::new(),
            approval.request.id(),
        )
    } else {
        fixture.sync()
    }
}

#[test]
fn integration_does_not_close_without_explicit_human_close_approval() {
    let fixture = integrated();
    sync_after_recheck(&fixture).unwrap();
    assert!(fixture.writes().is_empty());
    assert_eq!(
        fixture
            .store
            .snapshot_value()
            .execution
            .unwrap()
            .progress
            .state,
        IssueExecutionState::Done
    );
}

#[test]
fn a_confirmed_close_is_one_write_even_after_a_human_reopens() {
    let fixture = integrated();
    approve(&fixture);
    sync_after_recheck(&fixture).unwrap();
    assert_eq!(fixture.writes().len(), 1);
    assert!(matches!(
        fixture.writes()[0].operation,
        IssueFeedbackOperation::CloseIssue { .. }
    ));
    assert!(fixture.store.snapshot_value().state.close_proof.is_some());
    fixture.provider.closure.lock().unwrap().closed = false;
    sync_after_recheck(&fixture).unwrap();
    assert_eq!(fixture.writes().len(), 1);
}

#[test]
fn lost_close_response_and_failed_readback_never_resend_after_restart() {
    let fixture = integrated();
    approve(&fixture);
    *fixture.provider.write_behavior.lock().unwrap() = WriteBehavior::Lost;
    *fixture.provider.read_behavior.lock().unwrap() = ReadBehavior::Unavailable;
    sync_after_recheck(&fixture).unwrap();
    assert_eq!(
        fixture.store.record().state,
        IssueFeedbackIntentState::NeedsConfirmation
    );
    sync_after_recheck(&fixture).unwrap();
    fixture
        .service()
        .reconcile_pending(&IssueCollectionCancellation::new())
        .unwrap();
    assert_eq!(fixture.writes().len(), 1);
    *fixture.provider.read_behavior.lock().unwrap() = ReadBehavior::Confirm;
    fixture
        .service()
        .reconcile_pending(&IssueCollectionCancellation::new())
        .unwrap();
    assert_eq!(
        fixture.store.record().state,
        IssueFeedbackIntentState::NeedsConfirmation
    );
    assert!(!fixture.store.record().closure_acknowledged);
    assert!(fixture.store.snapshot_value().state.close_proof.is_none());
    sync_after_recheck(&fixture).unwrap();
    assert_eq!(fixture.writes().len(), 1);
}

#[test]
fn close_requires_current_policy_epoch_integration_and_source() {
    for condition in 0..7 {
        let fixture = integrated();
        approve(&fixture);
        {
            let mut data = fixture.store.data.lock().unwrap();
            match condition {
                0 => data.snapshot.authorization_epoch += 1,
                1 => {
                    data.snapshot
                        .execution
                        .as_mut()
                        .unwrap()
                        .progress
                        .integration
                        .as_mut()
                        .unwrap()
                        .integration_sha = "3".repeat(40)
                }
                2 => data.snapshot.pending_intervention = true,
                3 => data.snapshot.item.tracked.as_mut().unwrap().flags.held = true,
                4 => data.snapshot.item.triage.as_mut().unwrap().stale = true,
                5 => {
                    data.snapshot
                        .execution
                        .as_mut()
                        .unwrap()
                        .progress
                        .integration = None
                }
                _ => {
                    *fixture.policy.user.lock().unwrap() =
                        "[feedback]\ntransition = false\n[close]\nwhen = \"never\"\n".into()
                }
            }
        }
        sync_after_recheck(&fixture).unwrap();
        assert!(fixture.writes().is_empty(), "condition {condition}");
    }
}

#[test]
fn concurrent_authority_change_after_recording_abandons_without_sending() {
    let fixture = integrated();
    approve(&fixture);
    let store = fixture.store.clone();
    *fixture.store.after_record.lock().unwrap() = Some(Box::new(move || {
        store.data.lock().unwrap().snapshot.authorization_epoch += 1;
    }));
    sync_after_recheck(&fixture).unwrap();
    assert!(fixture.writes().is_empty());
    assert_eq!(
        fixture.store.record().state,
        IssueFeedbackIntentState::Abandoned
    );
}

#[test]
fn already_closed_or_incomplete_remote_state_does_not_send() {
    for incomplete in [false, true] {
        let fixture = integrated();
        approve(&fixture);
        {
            let mut closure = fixture.provider.closure.lock().unwrap();
            closure.closed = !incomplete;
            closure.complete = !incomplete;
        }
        sync_after_recheck(&fixture).unwrap();
        assert!(fixture.writes().is_empty());
    }
}

#[test]
fn project_policy_cannot_enable_transition() {
    let fixture = integrated();
    approve(&fixture);
    *fixture.policy.user.lock().unwrap() = "[feedback]\ntransition = false\n".into();
    *fixture.policy.project.lock().unwrap() =
        Some("[feedback]\ntransition = true\n[close]\nwhen = \"integration\"\n".into());
    let _ = sync_after_recheck(&fixture);
    assert!(fixture.writes().is_empty());
}

#[test]
fn a_crash_before_http_does_not_claim_a_same_account_human_close() {
    let fixture = integrated();
    approve(&fixture);
    let snapshot = fixture.store.snapshot_value();
    let mut record = pending_record(issue());
    record.state = IssueFeedbackIntentState::Sent;
    record.intent.operation = IssueFeedbackOperation::CloseIssue {
        integration_record_id: "integration-close".into(),
        integration_sha: "2".repeat(40),
        decision_id: snapshot
            .close_approval
            .unwrap()
            .request
            .id()
            .as_str()
            .into(),
        jira_transition: None,
        previous_event_id: None,
    };
    record.intent.desired_revision = 0;
    record.validate().unwrap();
    fixture.store.data.lock().unwrap().records.push(record);
    {
        let mut closure = fixture.provider.closure.lock().unwrap();
        closure.closed = true;
        closure.latest_event = Some(
            crate::application::port::outbound::issue_feedback_port::IssueFeedbackClosureEvent {
                event_id: "CE_human_same_account".into(),
                actor_id: Some(ACTOR.into()),
                closed: true,
            },
        );
    }
    fixture
        .service()
        .reconcile_pending(&IssueCollectionCancellation::new())
        .unwrap();
    assert!(fixture.writes().is_empty());
    assert_eq!(
        fixture.store.record().state,
        IssueFeedbackIntentState::NeedsConfirmation
    );
    assert!(fixture.store.snapshot_value().state.close_proof.is_none());
}

#[test]
fn close_requires_the_exact_approval_captured_before_source_recheck() {
    let fixture = integrated();
    approve(&fixture);
    fixture.sync().unwrap();
    assert!(
        fixture.writes().is_empty(),
        "ordinary feedback sync cannot consume a later approval"
    );
    let another = IssueDecisionId::new("another-close-approval").unwrap();
    fixture
        .service()
        .sync_issue_after_close_recheck(
            &issue(),
            123,
            &IssueCollectionCancellation::new(),
            &another,
        )
        .unwrap();
    assert!(
        fixture.writes().is_empty(),
        "the rechecked approval must be the current exact request"
    );
    sync_after_recheck(&fixture).unwrap();
    assert_eq!(fixture.writes().len(), 1);
}
