use super::*;
use crate::application::port::outbound::issue_automation_store_port::{
    IssueArbiterClaimRequest, IssueArbiterClaimResult, IssueArbiterCurrentContext,
    IssueArbiterFinishCommit, IssueArbiterFinishRequest, IssueArbiterFinishResult,
    IssueArbiterWaitRequest, IssueDecisionFence, IssueDecisionSaveResult,
};
use crate::domain::issue_automation::arbiter::*;
use crate::domain::issue_automation::coordinator::timeout_arbiter_start;
use crate::domain::issue_automation::decision::{
    IssueDecisionAgentAnswer, IssueDecisionAnswerBy, IssueDecisionRequest, IssueDecisionState,
    StoredIssueDecision,
};
use crate::domain::issue_automation::policy::IssueAutomationGateAction;

mod budgets;
mod fences;

const DUE: u64 = 1000;

fn decision(fixture: &Fixture, prepared: &Prepared) -> StoredIssueDecision {
    fixture
        .adapter()
        .get_issue_decision(fixture.cwd(), prepared.work.decision_id.as_ref().unwrap())
        .unwrap()
        .unwrap()
}

fn claim_request(
    fixture: &Fixture,
    prepared: &Prepared,
    now_unix_ms: u64,
) -> IssueArbiterClaimRequest {
    let record = decision(fixture, prepared);
    IssueArbiterClaimRequest {
        fence: IssueDecisionFence {
            triage: prepared.fence,
            decision_revision: record.revision,
        },
        current: prepared.assessment.clone(),
        decision_id: record.request.id().clone(),
        input_basis: IssueArbiterInputBasis {
            decision_basis: record.request.definition().basis.clone(),
            decision_revision: record.revision,
            repository_commit_sha: "b".repeat(40),
            evidence_sha256: "c".repeat(64),
            model_route_revision: "route:1".into(),
            input_sha256: "d".repeat(64),
        },
        settings: IssueArbiterSettings::default(),
        now_unix_ms,
    }
}

fn claimed(result: IssueArbiterClaimResult) -> IssueArbiterAttempt {
    match result {
        IssueArbiterClaimResult::Claimed(attempt) => *attempt,
        other => panic!("expected one dispatch reservation, got {other:?}"),
    }
}

fn start(fixture: &Fixture, request: &IssueArbiterClaimRequest) -> IssueArbiterAttempt {
    claimed(
        fixture
            .adapter()
            .claim_issue_arbiter(fixture.cwd(), request)
            .unwrap(),
    )
}

fn proposal() -> IssueArbiterResult {
    IssueArbiterResult::Proposal(IssueArbiterProposal {
        decision: "start".into(),
        confidence: 9000,
        has_blockers: false,
        note: None,
    })
}

fn finish_request(
    attempt: &IssueArbiterAttempt,
    claim: &IssueArbiterClaimRequest,
) -> IssueArbiterFinishRequest {
    IssueArbiterFinishRequest {
        attempt_id: attempt.attempt_id.clone(),
        expected_revision: attempt.revision,
        current: Some(IssueArbiterCurrentContext {
            fence: claim.fence,
            current: claim.current.clone(),
            input_basis: claim.input_basis.clone(),
        }),
        result: proposal(),
        now_unix_ms: claim.now_unix_ms + 1,
    }
}

fn finished(fixture: &Fixture, request: &IssueArbiterFinishRequest) -> IssueArbiterFinishCommit {
    match fixture
        .adapter()
        .finish_issue_arbiter(fixture.cwd(), request)
        .unwrap()
    {
        IssueArbiterFinishResult::Saved(commit) => *commit,
        other => panic!("expected a durable arbiter result, got {other:?}"),
    }
}

fn save_human(fixture: &Fixture, prepared: &Prepared) -> StoredIssueDecision {
    let record = decision(fixture, prepared);
    let human = record
        .request
        .answer_for_gate(
            &record.request.definition().basis,
            IssueAutomationGateAction::TimeoutArbiter,
            IssueDecisionAgentAnswer {
                option: "start".into(),
                note: None,
                caller: Some("Desktop".into()),
                channel: "native".into(),
                at: "1970-01-01T00:00:01Z".into(),
            },
            Some("desktop-confirmation:0123456789abcdef0123456789abcdef"),
        )
        .unwrap()
        .request;
    match fixture
        .adapter()
        .save_issue_decision(
            fixture.cwd(),
            IssueDecisionFence {
                triage: prepared.fence,
                decision_revision: record.revision,
            },
            &prepared.assessment,
            &human,
        )
        .unwrap()
    {
        IssueDecisionSaveResult::Saved(commit) => commit.record,
        other => panic!("expected a confirmed human answer, got {other:?}"),
    }
}

#[test]
fn arbiter_claim_is_due_once_and_a_restarted_observer_cannot_dispatch_again() {
    let fixture = Fixture::new("arbiter-claim-once");
    let prepared = prepare_unanswered_arbiter(&fixture, "I_once", &budget(1, 5));
    let before = claim_request(&fixture, &prepared, DUE - 1);
    assert_eq!(
        fixture
            .adapter()
            .claim_issue_arbiter(fixture.cwd(), &before)
            .unwrap(),
        IssueArbiterClaimResult::Ineligible
    );
    assert_eq!(count(&fixture, "issue_arbiter_attempts"), 0);
    let claim = claim_request(&fixture, &prepared, DUE);
    let attempt = start(&fixture, &claim);
    assert_eq!(attempt.deadline_at_unix_ms, DUE + 180_000);
    assert_eq!(count(&fixture, "issue_arbiter_attempts"), 1);
    let restarted = fixture.adapter();
    assert_eq!(
        restarted
            .claim_issue_arbiter(fixture.cwd(), &claim)
            .unwrap(),
        IssueArbiterClaimResult::Existing(Box::new(attempt.clone()))
    );
    assert_eq!(
        restarted
            .unfinished_issue_arbiter_attempts(fixture.cwd(), None, 256)
            .unwrap(),
        vec![attempt]
    );
    assert_eq!(count(&fixture, "issue_arbiter_attempts"), 1);
    assert_eq!(count(&fixture, "issue_work_admissions"), 0);
}

#[test]
fn arbiter_applied_answer_has_one_receipt_and_uses_the_shared_start_budget() {
    let fixture = Fixture::new("arbiter-applied-admission");
    let budget = budget(1, 1);
    let prepared = prepare_unanswered_arbiter(&fixture, "I_apply", &budget);
    assert!(!timeout_arbiter_start(
        &prepared.assessment,
        &decision(&fixture, &prepared).request
    ));
    assert!(
        fixture
            .adapter()
            .admit_issue_work(
                fixture.cwd(),
                prepared.fence,
                &prepared.assessment,
                &prepared.work,
                &budget,
                DUE
            )
            .is_err()
    );
    let claim = claim_request(&fixture, &prepared, DUE);
    let attempt = start(&fixture, &claim);
    let mut finish = finish_request(&attempt, &claim);
    let applied = finished(&fixture, &finish);
    assert!(applied.changed);
    assert_eq!(
        applied.event.outcome,
        IssueArbiterFinishedOutcome::Applied {}
    );
    assert_eq!(applied.record.revision, 2);
    let answer = applied.record.request.answer().unwrap();
    assert_eq!(answer.by, IssueDecisionAnswerBy::Arbiter);
    assert_eq!(answer.channel, ISSUE_ARBITER_CHANNEL);
    assert!(answer.confirmation.is_none() && answer.caller.is_none());
    assert!(timeout_arbiter_start(
        &prepared.assessment,
        &applied.record.request
    ));
    finish.now_unix_ms += 10_000;
    finish.current = None;
    finish.result = IssueArbiterResult::Waiting(IssueArbiterWaitReason::ModelUnavailable);
    let repeated = finished(&fixture, &finish);
    assert!(!repeated.changed);
    assert_eq!(repeated.record, applied.record);
    assert_eq!(repeated.event, applied.event);
    assert_eq!(count(&fixture, "issue_arbiter_finished_events"), 1);
    assert!(
        fixture
            .adapter()
            .unfinished_issue_arbiter_attempts(fixture.cwd(), None, 256)
            .unwrap()
            .is_empty()
    );
    let work = saved(run(&fixture, &prepared, &budget, DUE + 1));
    assert_eq!(
        work.start_authority,
        IssueWorkStartAuthority::TimeoutArbiter
    );
    assert_eq!(saved(run(&fixture, &prepared, &budget, DUE + 2)), work);
    let other = prepare(
        &fixture,
        "I_policy",
        IssueWorkStartAuthority::Policy,
        &budget,
    );
    assert!(matches!(
        run(&fixture, &other, &budget, DUE + 3),
        IssueWorkAdmissionResult::BudgetExhausted { .. }
    ));
    assert_eq!(count(&fixture, "issue_work_admissions"), 1);
}

#[test]
fn arbiter_wait_survives_ack_restart_and_later_human_without_rewriting_its_receipt() {
    for (confidence, blockers, option, expected) in [
        (8999, false, "start", IssueArbiterWaitReason::LowConfidence),
        (
            10000,
            true,
            "start",
            IssueArbiterWaitReason::BlockersPresent,
        ),
        (
            10000,
            false,
            "unknown",
            IssueArbiterWaitReason::InvalidResponse,
        ),
    ] {
        let fixture = Fixture::new("arbiter-open-wait");
        let prepared = prepare_unanswered_arbiter(&fixture, "I_wait", &budget(1, 5));
        let original = decision(&fixture, &prepared);
        let claim = claim_request(&fixture, &prepared, DUE);
        let attempt = start(&fixture, &claim);
        let mut finish = finish_request(&attempt, &claim);
        finish.result = IssueArbiterResult::Proposal(IssueArbiterProposal {
            decision: option.into(),
            confidence,
            has_blockers: blockers,
            note: None,
        });
        let wait = finished(&fixture, &finish);
        assert_eq!(wait.record, original);
        assert_eq!(
            wait.event.outcome,
            IssueArbiterFinishedOutcome::Waiting { reason: expected }
        );
        let id = original.request.id().clone();
        let snapshot = fixture
            .adapter()
            .issue_decision_arbiter_waits(fixture.cwd(), std::slice::from_ref(&id))
            .unwrap();
        assert_eq!(snapshot.catalog_revision, wait.event.catalog_revision);
        assert_eq!(snapshot.waits.len(), 1);
        assert_eq!(snapshot.waits[0].reason, expected);
        assert_eq!(snapshot.waits[0].decision_revision, original.revision);
        fixture
            .adapter()
            .acknowledge_issue_arbiter_event(fixture.cwd(), &wait.event.event_id)
            .unwrap();
        let restarted = fixture.adapter();
        assert!(
            restarted
                .pending_issue_arbiter_events(fixture.cwd(), 256)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            restarted
                .issue_decision_arbiter_waits(fixture.cwd(), std::slice::from_ref(&id))
                .unwrap(),
            snapshot
        );
        assert!(matches!(
            restarted
                .claim_issue_arbiter(fixture.cwd(), &claim)
                .unwrap(),
            IssueArbiterClaimResult::Deferred(_)
        ));
        let human = save_human(&fixture, &prepared);
        assert!(
            restarted
                .issue_decision_arbiter_waits(fixture.cwd(), std::slice::from_ref(&id))
                .unwrap()
                .waits
                .is_empty()
        );
        let replay = finished(&fixture, &finish);
        assert!(!replay.changed);
        assert_eq!(replay.record, original);
        assert_eq!(replay.event, wait.event);
        assert_eq!(decision(&fixture, &prepared), human);
        assert_eq!(count(&fixture, "issue_arbiter_finished_events"), 1);
    }
}

#[test]
fn arbiter_no_call_wait_is_durable_and_does_not_reserve_a_model_turn() {
    let fixture = Fixture::new("arbiter-no-call");
    let prepared = prepare_unanswered_arbiter(&fixture, "I_no_evidence", &budget(1, 5));
    let claim = claim_request(&fixture, &prepared, DUE);
    let wait = IssueArbiterWaitRequest {
        fence: claim.fence,
        current: claim.current.clone(),
        decision_id: claim.decision_id.clone(),
        reason: IssueArbiterWaitReason::EvidenceUnavailable,
        now_unix_ms: DUE,
    };
    let first = fixture
        .adapter()
        .record_issue_arbiter_wait(fixture.cwd(), &wait)
        .unwrap();
    let IssueArbiterFinishResult::Saved(first) = first else {
        panic!("expected a no-call wait")
    };
    assert!(first.changed);
    let IssueArbiterFinishResult::Saved(repeated) = fixture
        .adapter()
        .record_issue_arbiter_wait(fixture.cwd(), &wait)
        .unwrap()
    else {
        panic!("expected the existing wait")
    };
    assert!(!repeated.changed);
    assert_eq!(first.event, repeated.event);
    assert_eq!(count(&fixture, "issue_arbiter_attempts"), 0);
    assert_eq!(count(&fixture, "issue_arbiter_budget_clock"), 0);
    assert!(matches!(
        fixture
            .adapter()
            .claim_issue_arbiter(fixture.cwd(), &claim)
            .unwrap(),
        IssueArbiterClaimResult::Deferred(_)
    ));
    assert_eq!(count(&fixture, "issue_arbiter_attempts"), 0);
}

#[test]
fn arbiter_generic_save_rejects_a_preconstructed_valid_arbiter_answer() {
    let fixture = Fixture::new("arbiter-save-fence");
    let budget = budget(1, 1);
    let prepared = prepare_unanswered_arbiter(&fixture, "I_forge", &budget);
    let original = decision(&fixture, &prepared);
    let IssueArbiterResult::Proposal(proposal) = proposal() else {
        unreachable!()
    };
    let answer = original
        .request
        .answer_arbiter(
            &original.request.definition().basis,
            &prepared.assessment,
            &IssueArbiterSettings::default(),
            &proposal,
            DUE,
        )
        .unwrap()
        .request;
    assert!(
        fixture
            .adapter()
            .save_issue_decision(
                fixture.cwd(),
                IssueDecisionFence {
                    triage: prepared.fence,
                    decision_revision: 1,
                },
                &prepared.assessment,
                &answer
            )
            .is_err()
    );
    assert_eq!(decision(&fixture, &prepared), original);
    let mut forged = prepared.work.clone();
    forged.start_authority = IssueWorkStartAuthority::Policy;
    forged.decision_id = None;
    assert!(
        fixture
            .adapter()
            .admit_issue_work(
                fixture.cwd(),
                prepared.fence,
                &prepared.assessment,
                &forged,
                &budget,
                DUE
            )
            .is_err()
    );
    assert_eq!(count(&fixture, "issue_arbiter_finished_events"), 0);
    assert_eq!(count(&fixture, "issue_work_admissions"), 0);
}
