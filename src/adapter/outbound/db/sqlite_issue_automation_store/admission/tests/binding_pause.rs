use super::*;
use crate::domain::issue_automation::binding_pause::*;
use crate::domain::issue_automation::intervention::{
    IssueInterventionAction, IssueInterventionReceipt, IssueInterventionRequest,
};

fn command(
    fixture: &Fixture,
    prepared: &Prepared,
    id: &str,
    paused: bool,
) -> IssueBindingPauseIntent {
    let adapter = fixture.adapter();
    let (_, item) = subject(fixture, &prepared.work.order.issue);
    let state = adapter
        .issue_binding_pause_state(fixture.cwd(), &item.binding_id)
        .unwrap();
    IssueBindingPauseIntent {
        command_id: id.into(),
        binding_id: item.binding_id.clone(),
        paused,
        expected_revision: state.revision,
        resume_basis: (!paused).then(|| IssueBindingResumeBasis {
            binding_revision: adapter
                .get_binding(fixture.cwd(), &item.binding_id)
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
    }
}

fn complete_fence(
    fixture: &Fixture,
    request: &IssueInterventionRequest,
    work: &IssueWorkCheckpoint,
    epoch: u64,
) -> IssueInterventionReceipt {
    let receipt = IssueInterventionReceipt {
        issue: request.issue.clone(),
        command_id: request.command_id.clone(),
        action: request.action.label().into(),
        work_order_id: Some(work.order.work_order_id.clone()),
        authorization_epoch: epoch,
        changed: true,
        delivery: None,
        handoff: None,
    };
    fixture
        .adapter()
        .finish_issue_intervention(fixture.cwd(), &receipt)
        .unwrap();
    receipt
}

#[test]
fn pending_and_completed_pause_close_admission_and_old_generation_stays_stale_after_resume() {
    let fixture = Fixture::new("binding-pause-admission");
    let budget = budget(10, 10);
    let prepared = prepare(&fixture, "I_new", IssueWorkStartAuthority::Policy, &budget);
    let adapter = fixture.adapter();
    let pause = command(&fixture, &prepared, "pause", true);
    adapter
        .begin_issue_binding_pause(fixture.cwd(), &pause)
        .unwrap();
    assert!(matches!(
        run(&fixture, &prepared, &budget, 0),
        IssueWorkAdmissionResult::Conflict
    ));
    let mut fresh = prepared.clone();
    fresh.fence = subject(&fixture, &prepared.work.order.issue).0;
    assert_eq!(fresh.fence.binding_run_revision, 1);
    assert!(matches!(
        run(&fixture, &fresh, &budget, 0),
        IssueWorkAdmissionResult::Conflict
    ));
    adapter
        .finish_issue_binding_pause(fixture.cwd(), &pause, IssueBindingPauseCompletion::Apply)
        .unwrap();
    assert!(matches!(
        run(&fixture, &fresh, &budget, 0),
        IssueWorkAdmissionResult::Conflict
    ));
    let resume = command(&fixture, &prepared, "resume", false);
    adapter
        .begin_issue_binding_pause(fixture.cwd(), &resume)
        .unwrap();
    adapter
        .finish_issue_binding_pause(fixture.cwd(), &resume, IssueBindingPauseCompletion::Apply)
        .unwrap();
    assert!(matches!(
        run(&fixture, &prepared, &budget, 0),
        IssueWorkAdmissionResult::Conflict
    ));
    fresh.fence = subject(&fixture, &prepared.work.order.issue).0;
    assert_eq!(fresh.fence.binding_run_revision, 2);
    saved(run(&fixture, &fresh, &budget, 0));
    assert_eq!(count(&fixture, "issue_work_admissions"), 1);
}

#[test]
fn owned_fences_resume_partial_progress_and_require_every_exact_receipt_before_completion() {
    let fixture = Fixture::new("binding-pause-owned-progress");
    let budget = budget(10, 10);
    let a = prepare(&fixture, "I_a", IssueWorkStartAuthority::Policy, &budget);
    let b = prepare(&fixture, "I_b", IssueWorkStartAuthority::Policy, &budget);
    let work_a = saved(run(&fixture, &a, &budget, 0));
    let work_b = saved(run(&fixture, &b, &budget, 0));
    let pause = command(&fixture, &a, "pause", true);
    let adapter = fixture.adapter();
    adapter
        .begin_issue_binding_pause(fixture.cwd(), &pause)
        .unwrap();
    let first_page = adapter
        .pending_issue_binding_pause_work(fixture.cwd(), &pause, None, 1)
        .unwrap();
    assert_eq!(first_page.len(), 1);
    let other_page = adapter
        .pending_issue_binding_pause_work(
            fixture.cwd(),
            &pause,
            Some(&first_page[0].order.work_order_id),
            1,
        )
        .unwrap();
    assert_eq!(other_page.len(), 1);
    let first = adapter
        .claim_issue_binding_pause_fence(fixture.cwd(), &pause, &work_a)
        .unwrap();
    assert!(first.receipt.is_none());
    assert_eq!(first.request.action, IssueInterventionAction::Retriage);
    assert!(
        adapter
            .finish_issue_binding_pause(fixture.cwd(), &pause, IssueBindingPauseCompletion::Apply)
            .is_err()
    );
    let receipt = complete_fence(&fixture, &first.request, &work_a, 1);
    assert!(
        adapter
            .finish_issue_binding_pause(fixture.cwd(), &pause, IssueBindingPauseCompletion::Apply)
            .is_err()
    );
    let reopened = fixture.adapter();
    let mut expected_work = vec![work_a.clone(), work_b.clone()];
    expected_work.sort_by(|a, b| a.order.work_order_id.cmp(&b.order.work_order_id));
    assert_eq!(
        reopened
            .pending_issue_binding_pause_work(fixture.cwd(), &pause, None, 64)
            .unwrap(),
        expected_work
    );
    let replay = reopened
        .claim_issue_binding_pause_fence(fixture.cwd(), &pause, &work_a)
        .unwrap();
    assert_eq!(replay.receipt, Some(receipt));
    let second = reopened
        .claim_issue_binding_pause_fence(fixture.cwd(), &pause, &work_b)
        .unwrap();
    complete_fence(&fixture, &second.request, &work_b, 1);
    let completed = reopened
        .finish_issue_binding_pause(fixture.cwd(), &pause, IssueBindingPauseCompletion::Apply)
        .unwrap();
    assert_eq!(
        completed.receipt.unwrap().outcome,
        IssueBindingPauseOutcome::Paused
    );
    assert_eq!(count(&fixture, "issue_binding_pause_work_fences"), 2);
}

#[test]
fn a_preexisting_reserved_id_is_never_adopted_as_the_new_owned_fence() {
    use sha2::{Digest, Sha256};
    let fixture = Fixture::new("binding-pause-fresh-fence");
    let budget = budget(10, 10);
    let prepared = prepare(&fixture, "I_work", IssueWorkStartAuthority::Policy, &budget);
    let work = saved(run(&fixture, &prepared, &budget, 0));
    let pause = command(&fixture, &prepared, "pause", true);
    let old_id = format!(
        "{}{:x}",
        ISSUE_BINDING_PAUSE_CONTROL_PREFIX,
        Sha256::digest(
            serde_json::to_vec(&(
                fixture.cwd(),
                &pause.binding_id,
                &pause.command_id,
                &work.order.work_order_id
            ))
            .unwrap()
        )
    );
    let old = IssueInterventionRequest {
        command_id: old_id.clone(),
        issue: work.order.issue.clone(),
        action: IssueInterventionAction::Retriage,
    };
    let adapter = fixture.adapter();
    adapter
        .begin_issue_intervention(fixture.cwd(), &old)
        .unwrap();
    let receipt = complete_fence(&fixture, &old, &work, 1);
    adapter
        .begin_issue_binding_pause(fixture.cwd(), &pause)
        .unwrap();
    let owned = adapter
        .claim_issue_binding_pause_fence(fixture.cwd(), &pause, &work)
        .unwrap();
    assert_ne!(owned.request.command_id, old_id);
    assert!(owned.receipt.is_none());
    assert!(
        adapter
            .finish_issue_binding_pause(fixture.cwd(), &pause, IssueBindingPauseCompletion::Apply)
            .is_err()
    );
    complete_fence(&fixture, &owned.request, &work, 2);
    adapter
        .finish_issue_binding_pause(fixture.cwd(), &pause, IssueBindingPauseCompletion::Apply)
        .unwrap();
    assert_eq!(
        adapter
            .get_issue_intervention(fixture.cwd(), &old_id)
            .unwrap()
            .unwrap()
            .receipt,
        Some(receipt)
    );
}

#[test]
fn owned_retriage_preserves_held_and_untracked_work_and_lineage_resolution_is_exact() {
    let fixture = Fixture::new("binding-pause-lineage");
    let budget = budget(10, 10);
    let prepared = prepare(&fixture, "I_work", IssueWorkStartAuthority::Policy, &budget);
    let work = saved(run(&fixture, &prepared, &budget, 0));
    let adapter = fixture.adapter();
    let hold = IssueInterventionRequest {
        command_id: "human-hold".into(),
        issue: work.order.issue.clone(),
        action: IssueInterventionAction::Hold,
    };
    adapter
        .begin_issue_intervention(fixture.cwd(), &hold)
        .unwrap();
    complete_fence(&fixture, &hold, &work, 1);
    let untrack = IssueInterventionRequest {
        command_id: "human-untrack".into(),
        issue: work.order.issue.clone(),
        action: IssueInterventionAction::Untrack,
    };
    adapter
        .begin_issue_intervention(fixture.cwd(), &untrack)
        .unwrap();
    complete_fence(&fixture, &untrack, &work, 2);
    let pause = command(&fixture, &prepared, "pause", true);
    assert_eq!(
        adapter
            .resolve_issue_work_binding(fixture.cwd(), &work.order.work_order_id)
            .unwrap(),
        Some(pause.binding_id.clone())
    );
    assert_eq!(
        adapter
            .resolve_issue_work_binding(fixture.cwd(), "ordinary-or-missing-work")
            .unwrap(),
        None
    );
    adapter
        .begin_issue_binding_pause(fixture.cwd(), &pause)
        .unwrap();
    assert_eq!(
        adapter
            .pending_issue_binding_pause_work(fixture.cwd(), &pause, None, 64)
            .unwrap(),
        vec![work.clone()]
    );
    let owned = adapter
        .claim_issue_binding_pause_fence(fixture.cwd(), &pause, &work)
        .unwrap();
    complete_fence(&fixture, &owned.request, &work, 3);
    adapter
        .finish_issue_binding_pause(fixture.cwd(), &pause, IssueBindingPauseCompletion::Apply)
        .unwrap();
    assert!(
        adapter
            .get_issue(fixture.cwd(), &work.order.issue)
            .unwrap()
            .unwrap()
            .flags()
            .held
    );
    assert!(subject(&fixture, &work.order.issue).1.tracked.is_none());
    Connection::open(fixture.database())
        .unwrap()
        .execute("DELETE FROM tracked_issues", [])
        .unwrap();
    Connection::open(fixture.database())
        .unwrap()
        .execute("DELETE FROM issue_source_observations", [])
        .unwrap();
    assert!(
        adapter
            .resolve_issue_work_binding(fixture.cwd(), &work.order.work_order_id)
            .is_err()
    );
}
