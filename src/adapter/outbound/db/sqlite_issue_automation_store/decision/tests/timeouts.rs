use super::*;
use crate::domain::issue_automation::autonomy::{IssueAutonomyIntent, IssueAutonomyLevel};
use crate::domain::issue_automation::policy::{IssueAutomationGateAction, IssueAutomationTimeouts};

const TTL_MS: u64 = 24 * 60 * 60 * 1000;

fn timeouts() -> IssueAutomationTimeouts {
    IssueAutomationTimeouts {
        start_seconds: 24 * 60 * 60,
        remind_before_seconds: 2 * 60 * 60,
    }
}

fn timeout_assessment(
    fixture: &Fixture,
    item: &IssueCollectionItem,
    action: &str,
) -> IssueTriageAssessment {
    let source = format!(
        "[binding.github]\nmode=\"gated\"\n[authority]\nautonomy_cap=\"gated\"\n[select]\nmode=\"all_open\"\ntrusted_actors=[{{provider=\"github\",server_id=\"github.com\",actor_id=\"U_author\"}}]\n[gates.gated.normal]\nstart=\"{action}\"\n"
    );
    let policy = resolve_issue_automation_policy(Some(&source), None).unwrap();
    triage_issue(IssueTriageInput {
        runtime_selection_revision: fixture
            .adapter()
            .issue_autonomy_selection(fixture.cwd())
            .unwrap()
            .revision,
        issue: &item.issue,
        current: item.current.as_ref(),
        policy: &policy.policy,
        automation_policy_version: &policy.version,
        requirements: &IssueRequirements::default(),
        validation_policy_version: &issue_policy_version(&IssuePolicy::builtin()),
        held: item
            .tracked
            .as_ref()
            .is_some_and(|tracked| tracked.flags.held),
        pinned_source_commit: None,
        previous: item.triage.as_ref().map(|triage| &triage.assessment),
    })
    .unwrap()
}

fn prepared_timeout(
    fixture: &Fixture,
    action: &str,
    select_gated: bool,
) -> (
    IssueTriageFence,
    IssueTriageAssessment,
    IssueDecisionRequest,
) {
    let adapter = fixture.adapter();
    adapter.ensure_collection(fixture.cwd()).unwrap();
    if select_gated {
        let intent = IssueAutonomyIntent {
            command_id: "deadline-native-selection".into(),
            level: IssueAutonomyLevel::Gated,
            expected_revision: 0,
            policy_version: "a".repeat(64),
        };
        adapter
            .begin_issue_autonomy(fixture.cwd(), &intent)
            .unwrap();
        adapter
            .finish_issue_autonomy(fixture.cwd(), &intent)
            .unwrap();
    }
    observe(fixture, observation(fixture));
    let (fence, item) = subject(fixture);
    let assessment = timeout_assessment(fixture, &item, action);
    adapter
        .save_triage(fixture.cwd(), fence, &assessment)
        .unwrap();
    let (fence, item) = subject(fixture);
    let assessment = item.triage.unwrap().assessment;
    let mut definition = request(&assessment, "deadline-default")
        .definition()
        .clone();
    definition.on_timeout = IssueDecisionTimeout::Default;
    let request = IssueDecisionRequest::new(
        IssueDecisionId::new("deadline-default").unwrap(),
        definition,
    )
    .unwrap();
    (fence, assessment, request)
}

fn start_timing(
    fixture: &Fixture,
    fence: IssueTriageFence,
    assessment: &IssueTriageAssessment,
    request: &IssueDecisionRequest,
) -> IssueDecisionCommit {
    commit(
        fixture
            .adapter()
            .ensure_issue_decision_timed(fixture.cwd(), fence, assessment, request, 0, &timeouts())
            .unwrap(),
    )
}

fn decision_fence(triage: IssueTriageFence) -> IssueDecisionFence {
    IssueDecisionFence {
        triage,
        decision_revision: 1,
    }
}

fn human_answer(request: &IssueDecisionRequest) -> IssueDecisionRequest {
    request
        .answer_for_gate(
            &request.definition().basis,
            IssueAutomationGateAction::TimeoutDefault,
            IssueDecisionAgentAnswer {
                option: "reject".into(),
                note: None,
                caller: None,
                channel: "native".into(),
                at: "1970-01-02T00:00:00Z".into(),
            },
            Some("desktop-confirmation:0123456789abcdef0123456789abcdef"),
        )
        .unwrap()
        .request
}

#[test]
fn simultaneous_first_creation_keeps_one_canonical_deadline() {
    let fixture = Fixture::new("decision-deadline-create-race");
    let (fence, assessment, request) = prepared_timeout(&fixture, "timeout_default", true);
    let barrier = Arc::new(Barrier::new(3));
    let threads: Vec<_> = [0, 50]
        .into_iter()
        .map(|now| {
            let adapter = fixture.adapter();
            let workspace = fixture.cwd().to_owned();
            let assessment = assessment.clone();
            let request = request.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                commit(
                    adapter
                        .ensure_issue_decision_timed(
                            &workspace,
                            fence,
                            &assessment,
                            &request,
                            now,
                            &timeouts(),
                        )
                        .unwrap(),
                )
            })
        })
        .collect();
    barrier.wait();
    let results: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.changed).count(), 1);
    assert_eq!(results[0].record, results[1].record);
    assert_eq!(results[0].catalog_revision, results[1].catalog_revision);
    assert_eq!(results[0].record.revision, 1);
    let due = chrono::DateTime::parse_from_rfc3339(
        results[0]
            .record
            .request
            .definition()
            .due_at
            .as_deref()
            .unwrap(),
    )
    .unwrap()
    .timestamp_millis() as u64;
    assert!([TTL_MS, TTL_MS + 50].contains(&due));
    let repeated = commit(
        fixture
            .adapter()
            .ensure_issue_decision_timed(
                fixture.cwd(),
                fence,
                &assessment,
                &request,
                TTL_MS,
                &timeouts(),
            )
            .unwrap(),
    );
    assert!(!repeated.changed);
    assert_eq!(repeated.record, results[0].record);
}

#[test]
fn exact_deadline_records_default_once_across_reopening() {
    let fixture = Fixture::new("decision-deadline-restart");
    let (fence, assessment, request) = prepared_timeout(&fixture, "timeout_default", true);
    let initial = start_timing(&fixture, fence, &assessment, &request);
    let before = commit(
        fixture
            .adapter()
            .timeout_issue_decision(
                fixture.cwd(),
                decision_fence(fence),
                &assessment,
                request.id(),
                TTL_MS - 1,
            )
            .unwrap(),
    );
    assert_eq!(before.record, initial.record);
    assert!(!before.changed);
    let due = commit(
        fixture
            .adapter()
            .timeout_issue_decision(
                fixture.cwd(),
                decision_fence(fence),
                &assessment,
                request.id(),
                TTL_MS,
            )
            .unwrap(),
    );
    assert!(due.changed);
    assert_eq!(due.record.revision, 2);
    assert_eq!(due.catalog_revision, initial.catalog_revision + 1);
    let answer = due.record.request.answer().unwrap();
    assert_eq!(answer.by, IssueDecisionAnswerBy::Default);
    assert_eq!(answer.option, "accept");
    assert_eq!(answer.confirmation, None);
    let recovered = commit(
        fixture
            .adapter()
            .timeout_issue_decision(
                fixture.cwd(),
                decision_fence(fence),
                &assessment,
                request.id(),
                TTL_MS + 1000,
            )
            .unwrap(),
    );
    assert!(!recovered.changed);
    assert_eq!(recovered.record, due.record);
    assert_eq!(recovered.catalog_revision, due.catalog_revision);
    assert!(
        fixture
            .adapter()
            .issue_decision_deadlines(fixture.cwd(), None, 256)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn human_and_timeout_race_share_the_first_terminal_record() {
    let fixture = Fixture::new("decision-human-timeout-race");
    let (fence, assessment, request) = prepared_timeout(&fixture, "timeout_default", true);
    let initial = start_timing(&fixture, fence, &assessment, &request);
    let human = human_answer(&initial.record.request);
    let barrier = Arc::new(Barrier::new(3));
    let threads: Vec<_> = [false, true]
        .into_iter()
        .map(|default| {
            let adapter = fixture.adapter();
            let workspace = fixture.cwd().to_owned();
            let assessment = assessment.clone();
            let human = human.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                commit(
                    if default {
                        adapter.timeout_issue_decision(
                            &workspace,
                            decision_fence(fence),
                            &assessment,
                            human.id(),
                            TTL_MS,
                        )
                    } else {
                        adapter.save_issue_decision(
                            &workspace,
                            decision_fence(fence),
                            &assessment,
                            &human,
                        )
                    }
                    .unwrap(),
                )
            })
        })
        .collect();
    barrier.wait();
    let results: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.changed).count(), 1);
    assert_eq!(results[0].record, results[1].record);
    assert_eq!(results[0].catalog_revision, initial.catalog_revision + 1);
    assert_eq!(results[0].catalog_revision, results[1].catalog_revision);
}

#[test]
fn human_first_prevents_any_default_commit() {
    let fixture = Fixture::new("decision-human-before-timeout");
    let (fence, assessment, request) = prepared_timeout(&fixture, "timeout_default", true);
    let initial = start_timing(&fixture, fence, &assessment, &request);
    let human = commit(
        fixture
            .adapter()
            .save_issue_decision(
                fixture.cwd(),
                decision_fence(fence),
                &assessment,
                &human_answer(&initial.record.request),
            )
            .unwrap(),
    );
    let overdue = commit(
        fixture
            .adapter()
            .timeout_issue_decision(
                fixture.cwd(),
                decision_fence(fence),
                &assessment,
                request.id(),
                TTL_MS,
            )
            .unwrap(),
    );
    assert!(!overdue.changed);
    assert_eq!(overdue.record, human.record);
    assert_eq!(overdue.catalog_revision, human.catalog_revision);
    assert_eq!(
        overdue.record.request.answer().unwrap().by,
        IssueDecisionAnswerBy::Human
    );
}

#[test]
fn changed_actor_rejects_the_old_fence_and_supersedes_with_a_fresh_assessment() {
    let fixture = Fixture::new("decision-timeout-stale-actor");
    let (fence, assessment, request) = prepared_timeout(&fixture, "timeout_default", true);
    start_timing(&fixture, fence, &assessment, &request);
    let mut changed = observation(&fixture);
    changed.author.as_mut().unwrap().id = "U_other".into();
    observe(&fixture, changed);
    assert_eq!(
        fixture
            .adapter()
            .timeout_issue_decision(
                fixture.cwd(),
                decision_fence(fence),
                &assessment,
                request.id(),
                TTL_MS,
            )
            .unwrap(),
        IssueDecisionSaveResult::Conflict
    );
    let (fresh_fence, item) = subject(&fixture);
    let current = timeout_assessment(&fixture, &item, "timeout_default");
    let superseded = commit(
        fixture
            .adapter()
            .timeout_issue_decision(
                fixture.cwd(),
                decision_fence(fresh_fence),
                &current,
                request.id(),
                TTL_MS,
            )
            .unwrap(),
    );
    assert!(superseded.changed);
    assert_eq!(
        superseded.record.request.state(),
        IssueDecisionState::Superseded
    );
    assert_eq!(superseded.record.request.answer(), None);
}

#[test]
fn a_human_gate_or_missing_native_selection_keeps_default_requests_open() {
    for (action, selected) in [("human", true), ("timeout_default", false)] {
        let fixture = Fixture::new("decision-timeout-human-fence");
        let (fence, assessment, request) = prepared_timeout(&fixture, action, selected);
        let initial = start_timing(&fixture, fence, &assessment, &request);
        let overdue = commit(
            fixture
                .adapter()
                .timeout_issue_decision(
                    fixture.cwd(),
                    decision_fence(fence),
                    &assessment,
                    request.id(),
                    TTL_MS,
                )
                .unwrap(),
        );
        assert!(!overdue.changed);
        assert_eq!(overdue.record, initial.record);
    }
}

#[test]
fn an_ordinary_save_cannot_insert_a_preconstructed_default_answer() {
    let fixture = Fixture::new("decision-default-save-fence");
    let (fence, assessment, request) = prepared_timeout(&fixture, "timeout_default", true);
    let initial = start_timing(&fixture, fence, &assessment, &request);
    let forged = initial
        .record
        .request
        .timeout_for_gate(
            &request.definition().basis,
            IssueAutomationGateAction::TimeoutDefault,
            TTL_MS,
        )
        .unwrap()
        .unwrap()
        .request;
    assert!(
        fixture
            .adapter()
            .save_issue_decision(fixture.cwd(), decision_fence(fence), &assessment, &forged,)
            .is_err()
    );
    assert_eq!(
        fixture
            .adapter()
            .get_issue_decision(fixture.cwd(), request.id())
            .unwrap(),
        Some(initial.record)
    );
}

#[test]
fn timed_creation_preserves_legacy_none_and_strict_deadline_identity() {
    let fixture = Fixture::new("decision-deadline-legacy");
    let (fence, assessment, request) = prepared(&fixture);
    let initial = commit(
        fixture
            .adapter()
            .ensure_issue_decision(fixture.cwd(), fence, &assessment, &request)
            .unwrap(),
    );
    let repeated = commit(
        fixture
            .adapter()
            .ensure_issue_decision_timed(
                fixture.cwd(),
                fence,
                &assessment,
                &request,
                TTL_MS,
                &timeouts(),
            )
            .unwrap(),
    );
    assert!(!repeated.changed);
    assert_eq!(repeated.record, initial.record);
    assert_eq!(repeated.record.request.definition().due_at, None);
    let mut explicit = request.definition().clone();
    explicit.due_at = Some("1970-01-02T00:00:00Z".into());
    let explicit = IssueDecisionRequest::new(request.id().clone(), explicit).unwrap();
    assert!(
        fixture
            .adapter()
            .ensure_issue_decision(fixture.cwd(), fence, &assessment, &explicit,)
            .is_err()
    );
    assert!(
        fixture
            .adapter()
            .ensure_issue_decision_timed(
                fixture.cwd(),
                fence,
                &assessment,
                &explicit,
                0,
                &timeouts(),
            )
            .is_err()
    );
}
