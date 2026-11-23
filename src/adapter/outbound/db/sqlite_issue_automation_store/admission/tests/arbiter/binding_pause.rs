use super::*;
use crate::domain::issue_automation::binding_pause::*;

fn pause_and_resume(fixture: &Fixture, prepared: &Prepared) {
    let adapter = fixture.adapter();
    let binding_id = subject(fixture, &prepared.work.order.issue).1.binding_id;
    let pause = IssueBindingPauseIntent {
        command_id: "pause".into(),
        binding_id: binding_id.clone(),
        paused: true,
        expected_revision: 0,
        resume_basis: None,
    };
    adapter
        .begin_issue_binding_pause(fixture.cwd(), &pause)
        .unwrap();
    adapter
        .finish_issue_binding_pause(fixture.cwd(), &pause, IssueBindingPauseCompletion::Apply)
        .unwrap();
    let resume = IssueBindingPauseIntent {
        command_id: "resume".into(),
        binding_id: binding_id.clone(),
        paused: false,
        expected_revision: 1,
        resume_basis: Some(IssueBindingResumeBasis {
            binding_revision: adapter
                .get_binding(fixture.cwd(), &binding_id)
                .unwrap()
                .unwrap()
                .revision,
            autonomy_revision: adapter
                .issue_autonomy_selection(fixture.cwd())
                .unwrap()
                .revision,
            policy_version: prepared
                .assessment
                .basis()
                .automation_policy_version
                .clone(),
            effective_level: prepared.assessment.effective_mode(),
        }),
    };
    adapter
        .begin_issue_binding_pause(fixture.cwd(), &resume)
        .unwrap();
    adapter
        .finish_issue_binding_pause(fixture.cwd(), &resume, IssueBindingPauseCompletion::Apply)
        .unwrap();
}

#[test]
fn arbiter_pause_blocks_new_claims_even_with_a_fresh_generation_fence() {
    let fixture = Fixture::new("arbiter-binding-pause");
    let prepared = prepare_unanswered_arbiter(&fixture, "I_pause", &budget(1, 5));
    let mut claim = claim_request(&fixture, &prepared, DUE);
    let binding_id = subject(&fixture, &prepared.work.order.issue).1.binding_id;
    let pause = IssueBindingPauseIntent {
        command_id: "pause".into(),
        binding_id,
        paused: true,
        expected_revision: 0,
        resume_basis: None,
    };
    let adapter = fixture.adapter();
    adapter
        .begin_issue_binding_pause(fixture.cwd(), &pause)
        .unwrap();
    claim.fence.triage.binding_run_revision = 1;
    claim.input_basis.binding_run_revision = 1;
    assert!(matches!(
        adapter.claim_issue_arbiter(fixture.cwd(), &claim).unwrap(),
        IssueArbiterClaimResult::Ineligible
    ));
    adapter
        .finish_issue_binding_pause(fixture.cwd(), &pause, IssueBindingPauseCompletion::Apply)
        .unwrap();
    assert!(matches!(
        adapter.claim_issue_arbiter(fixture.cwd(), &claim).unwrap(),
        IssueArbiterClaimResult::Ineligible
    ));
    assert_eq!(count(&fixture, "issue_arbiter_attempts"), 0);
}

#[test]
fn arbiter_old_generation_cannot_claim_or_apply_after_binding_pause_resume_aba() {
    for already_claimed in [false, true] {
        let fixture = Fixture::new("arbiter-binding-aba");
        let prepared = prepare_unanswered_arbiter(&fixture, "I_aba", &budget(1, 5));
        let original = decision(&fixture, &prepared);
        let claim = claim_request(&fixture, &prepared, DUE);
        let attempt = already_claimed.then(|| start(&fixture, &claim));
        pause_and_resume(&fixture, &prepared);
        if let Some(attempt) = attempt {
            let mut completion = finish_request(&attempt, &claim);
            let current = completion.current.as_mut().unwrap();
            current.fence.triage.binding_run_revision = 2;
            current.input_basis.binding_run_revision = 2;
            let finished = finished(&fixture, &completion);
            assert_eq!(
                finished.event.outcome,
                IssueArbiterFinishedOutcome::Waiting {
                    reason: IssueArbiterWaitReason::EvidenceChanged
                }
            );
            assert_eq!(finished.record, original);
        } else {
            assert!(matches!(
                fixture
                    .adapter()
                    .claim_issue_arbiter(fixture.cwd(), &claim)
                    .unwrap(),
                IssueArbiterClaimResult::Ineligible
            ));
            let mut mismatched = claim.clone();
            mismatched.fence.triage.binding_run_revision = 2;
            assert!(matches!(
                fixture
                    .adapter()
                    .claim_issue_arbiter(fixture.cwd(), &mismatched)
                    .unwrap(),
                IssueArbiterClaimResult::Conflict
            ));
            mismatched.input_basis.binding_run_revision = 2;
            start(&fixture, &mismatched);
        }
        assert_eq!(count(&fixture, "issue_work_admissions"), 0);
    }
}
