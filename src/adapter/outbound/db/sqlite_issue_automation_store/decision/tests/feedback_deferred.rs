use super::*;
use crate::application::port::outbound::issue_automation_store_port::{
    IssueCollectionApplyResult, IssueTriageSaveResult,
};
use crate::domain::issue_automation::policy::IssueAutomationGateAction;
use crate::domain::issue_automation::source::{IssueSourceFailure, IssueSourceRead};

fn human_policy() -> String {
    AGENT_POLICY.replace("mode = \"auto\"", "mode = \"propose\"")
}

fn approved(fixture: &Fixture) -> StoredIssueDecision {
    observe(fixture, observation(fixture));
    let adapter = fixture.adapter();
    let (fence, item) = subject(fixture);
    let assessment = assess(&item, Some(&human_policy()));
    adapter
        .save_triage(fixture.cwd(), fence, &assessment)
        .unwrap();
    let (fence, item) = subject(fixture);
    let assessment = item.triage.unwrap().assessment;
    let request = request(&assessment, "feedback-deferred-human-start");
    let opened = commit(
        adapter
            .ensure_issue_decision(fixture.cwd(), fence, &assessment, &request)
            .unwrap(),
    );
    let confirmed = request
        .answer_for_gate(
            &request.definition().basis,
            IssueAutomationGateAction::Human,
            IssueDecisionAgentAnswer {
                option: "accept".into(),
                note: None,
                caller: Some("Desktop".into()),
                channel: "gateway".into(),
                at: "2026-11-21T00:00:00Z".into(),
            },
            Some("desktop-confirmation:cccccccccccccccccccccccccccccccc"),
        )
        .unwrap();
    commit(
        adapter
            .save_issue_decision(
                fixture.cwd(),
                IssueDecisionFence {
                    triage: fence,
                    decision_revision: opened.record.revision,
                },
                &assessment,
                &confirmed.request,
            )
            .unwrap(),
    )
    .record
}

fn item(
    fixture: &Fixture,
    issue: &crate::domain::issue_work::IssueReference,
) -> IssueCollectionItem {
    fixture
        .adapter()
        .read_triage_batch(fixture.cwd(), None, 100)
        .unwrap()
        .items
        .into_iter()
        .find(|item| item.issue == *issue)
        .unwrap()
}

fn deferred(issue: &crate::domain::issue_work::IssueReference) -> IssueSourceRead {
    IssueSourceRead::Unknown {
        issue: issue.clone(),
        reason: IssueSourceFailure::FeedbackUnconfirmed,
    }
}

#[test]
fn feedback_unconfirmed_resync_preserves_human_approval_and_gate_until_confirmed_readback() {
    let fixture = Fixture::new("feedback-deferred-approval");
    let adapter = fixture.adapter();
    let approval = approved(&fixture);
    let issue = approval.request.issue().clone();
    let before = item(&fixture, &issue);
    let original_assessment = before.triage.as_ref().unwrap().assessment.clone();
    let catalog = adapter
        .read_triage_batch(fixture.cwd(), None, 100)
        .unwrap()
        .revision;
    assert_eq!(
        approval.request.answer().unwrap().by,
        IssueDecisionAnswerBy::Human
    );

    for index in 0..3 {
        let control = adapter.get_collection(fixture.cwd()).unwrap().unwrap();
        assert!(matches!(
            adapter
                .apply_issue_resync(fixture.cwd(), control.fence(), &issue, &deferred(&issue))
                .unwrap(),
            IssueCollectionApplyResult::Applied(_)
        ));
        assert_eq!(item(&fixture, &issue), before);
        assert_eq!(
            adapter
                .get_issue_decision(fixture.cwd(), approval.request.id())
                .unwrap(),
            Some(approval.clone())
        );
        // Collection progress retains its own catalog event; semantic gates do not change.
        assert_eq!(
            adapter
                .read_triage_batch(fixture.cwd(), None, 100)
                .unwrap()
                .revision,
            catalog + index + 1
        );
    }

    // A read with the now-confirmed self effect removed has the same semantic
    // content. It may be processed normally without increasing any gate.
    let control = adapter.get_collection(fixture.cwd()).unwrap().unwrap();
    let mut readback = before.current.clone().unwrap();
    let IssueSourceRead::Observed(observation) = &mut readback else {
        panic!("missing observed fixture");
    };
    let IssueSourceContent::Shared(shared) = &mut observation.content else {
        panic!("missing shared fixture");
    };
    shared.basis.provider_updated_at = "2026-11-21T00:00:01Z".into();
    adapter
        .apply_issue_resync(fixture.cwd(), control.fence(), &issue, &readback)
        .unwrap();
    let after_read = item(&fixture, &issue);
    assert_eq!(after_read.current.as_ref(), Some(&readback));
    assert!(after_read.observation_revision > before.observation_revision);
    assert_eq!(
        after_read.tracked.as_ref().unwrap().flags,
        before.tracked.as_ref().unwrap().flags
    );
    let assessment = assess(&after_read, Some(&human_policy()));
    assert_eq!(assessment, original_assessment);
    let expected = IssueTriageFence {
        binding_run_revision: 0,
        collection_epoch: control.epoch,
        observation_revision: after_read.observation_revision,
        tracked_revision: after_read.tracked_revision,
        assessment_revision: after_read.triage.as_ref().map(|triage| triage.revision),
    };
    assert!(matches!(
        adapter
            .save_triage(fixture.cwd(), expected, &assessment)
            .unwrap(),
        IssueTriageSaveResult::Saved { .. }
    ));
    let refreshed = item(&fixture, &issue);
    assert_eq!(
        refreshed.triage.as_ref().unwrap().assessment,
        original_assessment
    );
    assert_eq!(
        IssueDecisionBasis::from_assessment(&assessment, IssueAutomationGate::Start, None).unwrap(),
        approval.request.definition().basis
    );
    assert_eq!(
        adapter
            .get_issue_decision(fixture.cwd(), approval.request.id())
            .unwrap(),
        Some(approval)
    );
}

#[test]
fn feedback_unconfirmed_batch_skips_only_its_issue_and_regular_unknown_still_invalidates_basis() {
    let fixture = Fixture::new("feedback-deferred-batch");
    let adapter = fixture.adapter();
    let approval = approved(&fixture);
    let issue = approval.request.issue().clone();
    let mut other = observation(&fixture);
    other.issue.issue_id = "I_other".into();
    observe(&fixture, other.clone());
    let before = item(&fixture, &issue);
    let old_gate = before
        .triage
        .as_ref()
        .unwrap()
        .assessment
        .gates()
        .iter()
        .find(|gate| gate.gate == IssueAutomationGate::Start)
        .unwrap()
        .gate_revision;
    let control = adapter.get_collection(fixture.cwd()).unwrap().unwrap();
    let batch = adapter
        .read_resync_batch(fixture.cwd(), control.fence(), 100)
        .unwrap()
        .unwrap();
    assert_eq!(batch.targets.len(), 2);
    other.remote_state = IssueRemoteState::Closed;
    let reads: Vec<_> = batch
        .targets
        .iter()
        .map(|target| {
            if target == &issue {
                deferred(target)
            } else {
                assert_eq!(target, &other.issue);
                IssueSourceRead::Observed(Box::new(other.clone()))
            }
        })
        .collect();
    assert!(matches!(
        adapter
            .apply_resync_batch(fixture.cwd(), &batch, &reads)
            .unwrap(),
        IssueCollectionApplyResult::Applied(_)
    ));
    assert_eq!(item(&fixture, &issue), before);
    assert_eq!(
        item(&fixture, &other.issue).current,
        Some(IssueSourceRead::Observed(Box::new(other)))
    );
    assert_eq!(
        adapter
            .get_issue_decision(fixture.cwd(), approval.request.id())
            .unwrap(),
        Some(approval)
    );

    // No other provider failure receives the feedback deferral exemption.
    let control = adapter.get_collection(fixture.cwd()).unwrap().unwrap();
    let unavailable = IssueSourceRead::Unknown {
        issue: issue.clone(),
        reason: IssueSourceFailure::ProviderUnavailable,
    };
    adapter
        .apply_issue_resync(fixture.cwd(), control.fence(), &issue, &unavailable)
        .unwrap();
    let unknown = item(&fixture, &issue);
    assert_eq!(unknown.current, Some(unavailable));
    assert!(unknown.tracked.as_ref().unwrap().flags.unknown);
    let assessment = assess(&unknown, Some(&human_policy()));
    assert!(assessment.basis().field_hash.is_none());
    assert_eq!(
        assessment
            .gates()
            .iter()
            .find(|gate| gate.gate == IssueAutomationGate::Start)
            .unwrap()
            .gate_revision,
        old_gate + 1
    );
}
