use super::super::tests::{Fixture, transition_and_save};
use super::*;
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::domain::issue_automation::coordinator::IssueWorkStartAuthority;
use crate::domain::issue_automation::{IssueAutomationState, IssueTransition, IssueTriageResult};
use std::sync::{Arc, Barrier};

mod arbiter;
mod fences;
mod migration;
mod support;
mod timeout_default;
use support::*;

#[test]
fn simultaneous_human_and_policy_starts_share_the_last_running_or_daily_slot() {
    for (budget, limit) in [
        (budget(1, 10), IssueStartBudgetLimit::Running),
        (budget(10, 1), IssueStartBudgetLimit::StartedPerDay),
    ] {
        let fixture = Fixture::new("admission-race");
        let policy = prepare(
            &fixture,
            "I_policy",
            IssueWorkStartAuthority::Policy,
            &budget,
        );
        let human = prepare(&fixture, "I_human", IssueWorkStartAuthority::Human, &budget);
        let barrier = Arc::new(Barrier::new(2));
        let handles: Vec<_> = [policy, human]
            .into_iter()
            .map(|prepared| {
                let adapter = fixture.adapter();
                let workspace = fixture.cwd().to_owned();
                let barrier = barrier.clone();
                let budget = budget.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    adapter
                        .admit_issue_work(
                            &workspace,
                            prepared.fence,
                            &prepared.assessment,
                            &prepared.work,
                            &budget,
                            0,
                        )
                        .unwrap()
                })
            })
            .collect();
        let outcomes: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, IssueWorkAdmissionResult::Saved { .. }))
                .count(),
            1
        );
        assert_eq!(outcomes.iter().filter(|outcome| matches!(outcome,
            IssueWorkAdmissionResult::BudgetExhausted { wait, .. } if wait.exhausted == vec![limit]
        )).count(), 1);
        assert_eq!(count(&fixture, "issue_work_admissions"), 1);
        assert_eq!(count(&fixture, "issue_work_checkpoints"), 1);
        assert_eq!(count(&fixture, "issue_start_budget_waits"), 1);
    }
}

#[test]
fn registration_intent_queued_running_and_review_reserve_capacity_until_terminal() {
    let fixture = Fixture::new("admission-lifecycle");
    let budget = budget(1, 10);
    let active = prepare(
        &fixture,
        "I_active",
        IssueWorkStartAuthority::Policy,
        &budget,
    );
    let work = saved(run(&fixture, &active, &budget, 0));
    assert!(
        work.task_ids.is_empty(),
        "a registration intent already reserves capacity"
    );
    let waiting = prepare(
        &fixture,
        "I_waiting",
        IssueWorkStartAuthority::Human,
        &budget,
    );
    let fingerprint = waiting.assessment.input_fingerprint();
    for step in [
        None,
        Some(IssueTransition::Queue),
        Some(IssueTransition::Start),
        Some(IssueTransition::OpenReview),
    ] {
        if let Some(step) = step {
            transition(&fixture, &active, step);
        }
        assert!(matches!(run(&fixture, &waiting, &budget, 0),
            IssueWorkAdmissionResult::BudgetExhausted { wait, .. } if wait.exhausted == vec![IssueStartBudgetLimit::Running]
        ));
        let (_, item) = subject(&fixture, &waiting.work.order.issue);
        assert_eq!(item.tracked.unwrap().state, IssueAutomationState::Ready);
        assert_eq!(
            item.triage.unwrap().assessment.input_fingerprint(),
            fingerprint
        );
        assert!(item.work.is_none());
        assert!(item.start_budget_wait.is_some());
    }
    transition(&fixture, &active, IssueTransition::Complete);
    saved(run(&fixture, &waiting, &budget, 0));
    assert!(
        subject(&fixture, &waiting.work.order.issue)
            .1
            .start_budget_wait
            .is_none()
    );
    assert_eq!(count(&fixture, "issue_work_admissions"), 2);
    assert_eq!(count(&fixture, "issue_start_budget_waits"), 0);
}

#[test]
fn utc_rollover_opens_only_new_work_and_clock_rollback_does_not_reopen_old_days() {
    let fixture = Fixture::new("admission-midnight");
    let budget = budget(10, 1);
    let first = prepare(
        &fixture,
        "I_first",
        IssueWorkStartAuthority::Policy,
        &budget,
    );
    let next = prepare(&fixture, "I_next", IssueWorkStartAuthority::Human, &budget);
    let midnight = 22 * MILLIS_PER_UTC_DAY;
    let first_record = saved(run(&fixture, &first, &budget, midnight - 1));
    assert!(matches!(run(&fixture, &next, &budget, midnight - 1),
        IssueWorkAdmissionResult::BudgetExhausted { wait, .. } if wait.exhausted == vec![IssueStartBudgetLimit::StartedPerDay]
    ));
    saved(run(&fixture, &next, &budget, midnight));
    assert_eq!(
        saved(run(&fixture, &first, &budget, midnight)),
        first_record
    );
    assert_eq!(count(&fixture, "issue_work_admissions"), 2);
    let third = prepare(
        &fixture,
        "I_third",
        IssueWorkStartAuthority::Policy,
        &budget,
    );
    assert!(
        matches!(run(&fixture, &third, &budget, midnight - MILLIS_PER_UTC_DAY),
            IssueWorkAdmissionResult::BudgetExhausted { wait, .. }
                if wait.utc_day == 22 && wait.exhausted == vec![IssueStartBudgetLimit::StartedPerDay]
        )
    );
}

#[test]
fn restart_before_registration_and_after_lost_ack_reuses_the_reservation_under_lowered_limits() {
    let fixture = Fixture::new("admission-recovery");
    let original_budget = budget(2, 2);
    let prepared = prepare(
        &fixture,
        "I_existing",
        IssueWorkStartAuthority::Policy,
        &original_budget,
    );
    let admitted = saved(run(&fixture, &prepared, &original_budget, 0));
    assert_eq!(
        saved(run(&fixture, &prepared, &original_budget, 0)),
        admitted
    );
    let mut registered = admitted.clone();
    registered.task_ids = vec!["task-original".into()];
    let IssueWorkSaveResult::Saved { record, .. } = fixture
        .adapter()
        .save_issue_work(
            fixture.cwd(),
            prepared.fence,
            &prepared.assessment,
            &registered,
        )
        .unwrap()
    else {
        panic!("expected registration checkpoint")
    };
    let lowered = budget(1, 1);
    // Recovery reads the prior reservation; a lower quota input cannot charge it again.
    assert_eq!(
        saved(run(&fixture, &prepared, &lowered, MILLIS_PER_UTC_DAY)),
        *record
    );
    assert_eq!(record.task_ids, vec!["task-original"]);
    let next = prepare(&fixture, "I_new", IssueWorkStartAuthority::Policy, &lowered);
    assert!(matches!(run(&fixture, &next, &lowered, 0),
        IssueWorkAdmissionResult::BudgetExhausted { wait, .. }
            if wait.exhausted.contains(&IssueStartBudgetLimit::Running)
                && wait.exhausted.contains(&IssueStartBudgetLimit::StartedPerDay)
    ));
    assert_eq!(count(&fixture, "issue_work_admissions"), 1);
    assert_eq!(count(&fixture, "issue_work_checkpoints"), 1);
}

#[test]
fn configured_zero_attempts_never_creates_a_checkpoint_even_with_human_approval() {
    let fixture = Fixture::new("admission-zero-attempts");
    let mut budget = budget(10, 10);
    budget.max_attempts = 0;
    let prepared = prepare(&fixture, "I_zero", IssueWorkStartAuthority::Human, &budget);
    assert!(matches!(run(&fixture, &prepared, &budget, 0),
        IssueWorkAdmissionResult::BudgetExhausted { wait, .. } if wait.exhausted == vec![IssueStartBudgetLimit::Attempts]
    ));
    assert_eq!(count(&fixture, "issue_work_admissions"), 0);
    assert_eq!(count(&fixture, "issue_work_checkpoints"), 0);
    assert_eq!(
        subject(&fixture, &prepared.work.order.issue)
            .1
            .tracked
            .unwrap()
            .state,
        IssueAutomationState::Ready
    );
}
