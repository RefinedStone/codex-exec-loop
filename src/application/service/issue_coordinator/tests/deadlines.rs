use super::autonomy::autonomy;
use super::gated::{enable_gated, gated_policy};
use super::*;
use crate::domain::issue_automation::autonomy::IssueAutonomyLevel;

const START: u64 = 123;
const HOUR: u64 = 3_600_000;

fn timeout_policy() -> String {
    gated_policy().replace("start = \"auto\"", "start = \"timeout_default\"")
}

fn setup() -> Harness {
    let h = Harness::new();
    *h.policy.text.lock().unwrap() = timeout_policy();
    enable_gated(&h);
    h
}

fn deadline(stored: &StoredIssueDecision) -> u64 {
    chrono::DateTime::parse_from_rfc3339(stored.request.definition().due_at.as_deref().unwrap())
        .unwrap()
        .timestamp_millis() as u64
}

fn stored(h: &Harness, id: &IssueDecisionId) -> StoredIssueDecision {
    h.fixture
        .store()
        .get_issue_decision(&h.fixture.workspace, id)
        .unwrap()
        .unwrap()
}

fn human_answer_at(h: &Harness, initial: &StoredIssueDecision, option: &str, at: u64) {
    use crate::application::port::inbound::stored_command_port::{
        StoredCommandContext, issue_confirmation::VerifiedHumanConfirmation,
    };
    let preview = h
        .decisions
        .prepare_confirmation(initial.request.id().as_str(), option)
        .unwrap();
    let context = StoredCommandContext {
        human_confirmation: Some(
            VerifiedHumanConfirmation::from_verified(
                preview.target,
                format!("desktop-confirmation:{}", "d".repeat(32)),
            )
            .unwrap(),
        ),
        caller: Some("Desktop".into()),
        channel: "gateway".into(),
        at: chrono::DateTime::from_timestamp_millis(at as i64)
            .unwrap()
            .to_rfc3339(),
    };
    h.decisions
        .answer(initial.request.id().as_str(), option, None, &context)
        .unwrap();
}

fn track_at(
    coordinator: &IssueCoordinatorService,
    now: u64,
    enabled: bool,
) -> Result<Vec<IssueCoordinatorFact>> {
    let facts = Mutex::new(Vec::new());
    coordinator.track_execution(
        &IssueCollectionCancellation::new(),
        now,
        enabled,
        &|committed| facts.lock().unwrap().extend(committed),
    )?;
    Ok(facts.into_inner().unwrap())
}

fn tick_at(h: &Harness, now: u64) -> Result<()> {
    run(&h.triage);
    h.coordinator()
        .tick(&IssueCollectionCancellation::new(), now, true, &|_| {})
}

fn reassemble(h: &Harness) -> IssueCoordinatorService {
    let triage = Arc::new(h.fixture.service(h.policy.clone()));
    let decisions = Arc::new(IssueDecisionService::new(
        h.fixture.workspace.clone(),
        h.fixture.store(),
        triage.clone(),
        IssuePolicyService::new(Arc::new(ValidationPolicy)),
    ));
    IssueCoordinatorService::new(
        h.fixture.workspace.clone(),
        h.fixture.store(),
        h.source.clone(),
        triage,
        decisions,
        IssuePolicyService::new(Arc::new(ValidationPolicy)),
        h.execution.clone(),
    )
}

fn defaults(facts: &[IssueCoordinatorFact]) -> usize {
    facts
        .iter()
        .filter(|fact| {
            matches!(fact, IssueCoordinatorFact::Decision { fact, .. }
            if fact.answer_by == Some(IssueDecisionAnswerBy::Default))
        })
        .count()
}

#[test]
fn tracking_reminds_once_then_starts_from_one_default_at_the_exact_deadline() {
    let h = setup();
    h.tick(true).unwrap();
    let initial = h.decision();
    let due = deadline(&initial);
    assert_eq!(due, START + 24 * HOUR);
    assert_eq!(
        initial.request.definition().on_timeout,
        IssueDecisionTimeout::Default
    );
    assert_eq!(
        initial.request.definition().recommended.as_deref(),
        Some("start")
    );
    assert!(h.work().is_none());
    let before_reads = h.source.calls.load(Ordering::SeqCst);
    let coordinator = h.coordinator();
    let before = track_at(&coordinator, due - 2 * HOUR - 1, true).unwrap();
    assert!(
        before
            .iter()
            .all(|fact| !matches!(fact, IssueCoordinatorFact::DecisionReminded(_)))
    );
    assert_eq!(defaults(&before), 0);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
    assert_eq!(h.source.calls.load(Ordering::SeqCst), before_reads);

    let facts = track_at(&coordinator, due - 2 * HOUR, true).unwrap();
    let reminders: Vec<_> = facts
        .iter()
        .filter_map(|fact| match fact {
            IssueCoordinatorFact::DecisionReminded(event) => Some(event),
            _ => None,
        })
        .collect();
    assert_eq!(reminders.len(), 1);
    assert_eq!(reminders[0].fact.decision_id, *initial.request.id());
    assert_eq!(stored(&h, initial.request.id()), initial);
    coordinator
        .acknowledge_event(&reminders[0].event_id)
        .unwrap();
    let before_due = track_at(&coordinator, due - 1, true).unwrap();
    assert_eq!(defaults(&before_due), 0);
    assert!(
        before_due
            .iter()
            .all(|fact| !matches!(fact, IssueCoordinatorFact::DecisionReminded(_)))
    );
    assert_eq!(h.execution.state.lock().unwrap().registrations, 0);

    let due_facts = track_at(&coordinator, due, true).unwrap();
    assert_eq!(defaults(&due_facts), 1);
    let answered = stored(&h, initial.request.id());
    assert_eq!(answered.revision, initial.revision + 1);
    assert_eq!(
        answered.request.answer().unwrap().by,
        IssueDecisionAnswerBy::Default
    );
    let work = h.work().unwrap();
    assert_eq!(
        work.start_authority,
        IssueWorkStartAuthority::TimeoutDefault
    );
    assert_eq!(work.decision_id.as_ref(), Some(initial.request.id()));
    assert_eq!(work.task_ids, ["task-1"]);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
    assert_eq!(h.execution.state.lock().unwrap().grants.len(), 1);

    let recovered = reassemble(&h);
    for now in [due, due + 5_000, due + 24 * HOUR] {
        assert_eq!(defaults(&track_at(&recovered, now, true).unwrap()), 0);
        assert_eq!(stored(&h, initial.request.id()), answered);
    }
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
    assert_eq!(h.execution.state.lock().unwrap().grants.len(), 1);
    assert!(
        h.fixture
            .store()
            .pending_issue_decision_reminders(&h.fixture.workspace, 256)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn either_human_choice_before_timeout_remains_the_only_answer() {
    for option in ["start", "reject"] {
        let h = setup();
        h.tick(true).unwrap();
        let initial = h.decision();
        human_answer_at(&h, &initial, option, START + 1_000);
        h.tick(true).unwrap();
        let answered = stored(&h, initial.request.id());
        assert_eq!(
            answered.request.answer().unwrap().by,
            IssueDecisionAnswerBy::Human
        );
        let facts = track_at(&reassemble(&h), deadline(&initial), true).unwrap();
        assert_eq!(defaults(&facts), 0);
        assert_eq!(stored(&h, initial.request.id()), answered);
        assert_eq!(
            h.execution.state.lock().unwrap().registrations,
            usize::from(option == "start")
        );
        if let Some(work) = h.work() {
            assert_eq!(work.start_authority, IssueWorkStartAuthority::Human);
        }
    }
}

#[test]
fn an_unconfirmed_agent_answer_cannot_preempt_the_timeout_default() {
    use crate::application::port::inbound::stored_command_port::StoredCommandContext;
    let h = setup();
    h.tick(true).unwrap();
    let initial = h.decision();
    let unconfirmed = StoredCommandContext {
        human_confirmation: None,
        caller: Some("Desktop".into()),
        channel: "gateway".into(),
        at: "1970-01-01T00:00:00.500Z".into(),
    };
    assert!(
        h.decisions
            .answer(initial.request.id().as_str(), "reject", None, &unconfirmed,)
            .is_err()
    );
    assert_eq!(stored(&h, initial.request.id()), initial);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
    let facts = track_at(&h.coordinator(), deadline(&initial), true).unwrap();
    assert_eq!(defaults(&facts), 1);
    let answered = stored(&h, initial.request.id());
    assert_eq!(
        answered.request.answer().unwrap().by,
        IssueDecisionAnswerBy::Default
    );
    assert_eq!(answered.request.answer().unwrap().option, "start");
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
}

#[test]
fn propose_explicit_human_and_high_risk_deadlines_keep_waiting() {
    for scenario in ["propose", "human", "high"] {
        let h = Harness::with_observation(|observation| {
            if scenario == "high" {
                observation.labels = complete(vec![IssueSourceLabel {
                    id: "L_high".into(),
                    name: "high-risk".into(),
                }]);
            }
        });
        if scenario != "propose" {
            let mut policy = timeout_policy();
            if scenario == "human" {
                policy = policy.replace("start = \"timeout_default\"", "start = \"human\"");
            } else {
                policy.push_str("\n[[classify.rule]]\nclass=\"high\"\nlabels_any=[\"high-risk\"]\n[gates.gated.high]\nstart=\"timeout_default\"\n");
            }
            *h.policy.text.lock().unwrap() = policy;
            enable_gated(&h);
        }
        h.tick(true).unwrap();
        let initial = h.decision();
        assert_eq!(
            initial.request.definition().on_timeout,
            IssueDecisionTimeout::Wait,
            "{scenario}"
        );
        for now in [deadline(&initial), deadline(&initial) + 24 * HOUR] {
            assert_eq!(
                defaults(&track_at(&reassemble(&h), now, true).unwrap()),
                0,
                "{scenario}"
            );
            assert_eq!(stored(&h, initial.request.id()), initial, "{scenario}");
        }
        assert!(h.work().is_none());
        assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
    }
}

#[test]
fn a_committed_default_survives_disabled_execution_and_resumes_after_reassembly() {
    let h = setup();
    h.tick(true).unwrap();
    let initial = h.decision();
    let due = deadline(&initial);
    let facts = track_at(&h.coordinator(), due, false).unwrap();
    assert_eq!(defaults(&facts), 1);
    assert!(h.work().is_none());
    assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
    let answer = stored(&h, initial.request.id());
    let facts = track_at(&reassemble(&h), due + 1, true).unwrap();
    assert_eq!(defaults(&facts), 0);
    assert_eq!(stored(&h, initial.request.id()), answer);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
    assert_eq!(
        h.work().unwrap().start_authority,
        IssueWorkStartAuthority::TimeoutDefault
    );
}

#[test]
fn default_registration_and_grant_crashes_reuse_the_original_answer_and_admission() {
    for failure in ["before_register", "after_register", "after_grant"] {
        let h = setup();
        *h.policy.text.lock().unwrap() = format!(
            "{}\n[budget]\nmax_running=1\nmax_started_per_day=1\n",
            timeout_policy()
        );
        h.tick(true).unwrap();
        let initial = h.decision();
        let due = deadline(&initial);
        {
            let mut state = h.execution.state.lock().unwrap();
            state.fail_before_register = failure == "before_register";
            state.fail_after_register = failure == "after_register";
            state.fail_after_grant = failure == "after_grant";
        }
        assert!(track_at(&h.coordinator(), due, true).is_err(), "{failure}");
        let answered = stored(&h, initial.request.id());
        assert_eq!(
            answered.request.answer().unwrap().by,
            IssueDecisionAnswerBy::Default
        );
        let original = h.work().unwrap();
        let recovered = reassemble(&h);
        for now in [due + 24 * HOUR, due + 24 * HOUR + 1] {
            assert_eq!(defaults(&track_at(&recovered, now, true).unwrap()), 0);
        }
        let work = h.work().unwrap();
        assert_eq!(work.order, original.order, "{failure}");
        assert_eq!(work.approved_basis, original.approved_basis, "{failure}");
        assert_eq!(
            work.start_authority,
            IssueWorkStartAuthority::TimeoutDefault
        );
        assert_eq!(stored(&h, initial.request.id()), answered);
        assert_eq!(
            h.execution.state.lock().unwrap().registrations,
            1,
            "{failure}"
        );
        assert_eq!(
            h.execution.state.lock().unwrap().grants.len(),
            1,
            "{failure}"
        );
        assert!(page(&h.triage).items[0].start_budget_wait.is_none());
    }
}

#[test]
fn a_default_waiting_for_running_budget_starts_when_the_other_work_completes() {
    let h = setup();
    *h.policy.text.lock().unwrap() = format!(
        "{}\n[timeouts]\nstart=\"10s\"\nremind_before=\"2s\"\n[budget]\nmax_running=1\nmax_started_per_day=10\n",
        timeout_policy(),
    );
    tick_at(&h, 0).unwrap();
    track_at(&h.coordinator(), 10_000, true).unwrap();
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);

    let next = h.fixture.observation("I_002");
    h.fixture.seed(vec![next.clone()], &disclosure_version());
    *h.source.read.lock().unwrap() = IssueSourceRead::Observed(Box::new(next));
    tick_at(&h, 10_001).unwrap();
    let second = page(&h.triage)
        .items
        .into_iter()
        .find(|item| item.issue.issue_id == "I_002")
        .unwrap()
        .open_decisions
        .remove(0);
    let due = deadline(&second);
    assert_eq!(due, 20_001);
    assert_eq!(defaults(&track_at(&h.coordinator(), due, true).unwrap()), 1);
    let answered = stored(&h, second.request.id());
    let waiting = page(&h.triage)
        .items
        .into_iter()
        .find(|item| item.issue.issue_id == "I_002")
        .unwrap();
    assert_eq!(
        waiting.start_budget_wait.unwrap().exhausted,
        [IssueStartBudgetLimit::Running]
    );
    assert!(waiting.work.is_none());
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);

    // The fixture records the first work's verified terminal sequence through the normal state CAS.
    let store = h.fixture.store();
    let mut first = store
        .get_issue(&h.fixture.workspace, &h.fixture.reference("I_001"))
        .unwrap()
        .unwrap();
    for transition in [
        IssueTransition::Start,
        IssueTransition::OpenReview,
        IssueTransition::Complete,
    ] {
        let revision = first.revision();
        first.transition(transition).unwrap();
        store
            .save_issue(&h.fixture.workspace, revision, &first)
            .unwrap();
    }
    // FakeExecution keeps a single active work cache; the completed slot is now available.
    {
        let mut state = h.execution.state.lock().unwrap();
        state.order = None;
        state.saved_source = None;
        state.saved_revision = 0;
        state.saved_authorization_epoch = 0;
        state.revoked = false;
    }
    assert_eq!(
        defaults(&track_at(&reassemble(&h), due + 1, true).unwrap()),
        0
    );
    let second_work = store
        .get_issue_work(&h.fixture.workspace, &h.fixture.reference("I_002"))
        .unwrap()
        .unwrap();
    assert_eq!(
        second_work.start_authority,
        IssueWorkStartAuthority::TimeoutDefault
    );
    assert_eq!(second_work.decision_id.as_ref(), Some(second.request.id()));
    assert_eq!(stored(&h, second.request.id()), answered);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 2);
    let current = page(&h.triage)
        .items
        .into_iter()
        .find(|item| item.issue.issue_id == "I_002")
        .unwrap();
    assert!(current.start_budget_wait.is_none());
}

#[test]
fn off_or_changed_source_before_deadline_supersedes_without_default_or_submission() {
    for scenario in ["off", "source", "policy"] {
        let h = setup();
        h.tick(true).unwrap();
        let initial = h.decision();
        match scenario {
            "off" => {
                autonomy(&h)
                    .set("deadline-off".into(), IssueAutonomyLevel::Off, None)
                    .unwrap();
            }
            "source" => {
                let mut changed = h.fixture.observation("I_001");
                changed.author.as_mut().unwrap().id = "U_other".into();
                h.fixture.seed(vec![changed], &disclosure_version());
            }
            _ => {
                *h.policy.text.lock().unwrap() =
                    timeout_policy().replace("start = \"timeout_default\"", "start = \"human\"");
            }
        }
        let facts = track_at(&reassemble(&h), deadline(&initial), true).unwrap();
        assert_eq!(defaults(&facts), 0, "{scenario}");
        let superseded = stored(&h, initial.request.id());
        assert_eq!(
            superseded.request.state(),
            IssueDecisionState::Superseded,
            "{scenario}"
        );
        assert_eq!(superseded.request.answer(), None, "{scenario}");
        assert!(h.work().is_none());
        assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
    }
}

#[test]
fn a_new_assessment_revokes_existing_default_authority_during_tracking() {
    for scenario in ["policy", "source"] {
        let h = setup();
        h.tick(true).unwrap();
        let initial = h.decision();
        let due = deadline(&initial);
        track_at(&h.coordinator(), due, true).unwrap();
        let original = h.work().unwrap();
        if scenario == "policy" {
            *h.policy.text.lock().unwrap() =
                timeout_policy().replace("start = \"timeout_default\"", "start = \"human\"");
        } else {
            let mut changed = h.fixture.observation("I_001");
            changed.author.as_mut().unwrap().id = "U_other".into();
            h.fixture.seed(vec![changed], &disclosure_version());
        }
        run(&h.triage);
        track_at(&reassemble(&h), due + 5_000, true).unwrap();
        let state = h.execution.state.lock().unwrap();
        assert!(
            state
                .revoked_work_orders
                .contains(&original.order.work_order_id),
            "{scenario}"
        );
        assert_eq!(state.registrations, 1);
        assert_eq!(state.grants.len(), 1);
    }
}
