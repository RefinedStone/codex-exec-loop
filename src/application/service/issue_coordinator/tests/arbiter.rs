use super::autonomy::autonomy;
use super::gated::{enable_gated, gated_policy};
use super::*;

mod dispatch;
use std::time::{Duration, Instant};

use crate::adapter::outbound::git::issue_assessment_evidence::fake::FakeIssueAssessmentEvidenceAdapter;
use crate::adapter::outbound::issue_assessment_model::fake::FakeIssueAssessmentModelAdapter;
use crate::application::port::outbound::issue_assessment_model_port::IssueAssessmentModelPort;
use crate::domain::issue_automation::arbiter::{
    IssueArbiterFinishedEvent, IssueArbiterFinishedOutcome, IssueArbiterWaitReason,
};
use crate::domain::issue_automation::autonomy::IssueAutonomyLevel;

mod recovery;

const START: u64 = 123;
const REPLY: &str = r#"{"decision":"start","confidence":9500,"reasons":["현재 공유 근거가 완료 기준을 충족합니다."],"blockers":[]}"#;

struct ArbiterHarness {
    h: Harness,
    model: Arc<FakeIssueAssessmentModelAdapter>,
    evidence: Arc<FakeIssueAssessmentEvidenceAdapter>,
    initial: StoredIssueDecision,
    due: u64,
    clock: Arc<Mutex<Instant>>,
}

impl ArbiterHarness {
    fn new() -> Self {
        Self::with_policy(|policy| policy)
    }

    fn with_policy(change: impl FnOnce(String) -> String) -> Self {
        let h = Harness::new();
        *h.policy.text.lock().unwrap() = change(format!(
            "{}\n[timeouts]\nstart = \"4s\"\nremind_before = \"2s\"\n",
            gated_policy().replace("start = \"auto\"", "start = \"timeout_arbiter\"")
        ));
        enable_gated(&h);
        h.tick(true).unwrap();
        let initial = h.decision();
        let due = chrono::DateTime::parse_from_rfc3339(
            initial.request.definition().due_at.as_deref().unwrap(),
        )
        .unwrap()
        .timestamp_millis() as u64;
        assert_eq!(due, START + 4_000);
        assert_eq!(
            initial.request.definition().on_timeout,
            IssueDecisionTimeout::Arbiter
        );
        Self {
            h,
            model: Arc::new(FakeIssueAssessmentModelAdapter::default()),
            evidence: Arc::new(FakeIssueAssessmentEvidenceAdapter::default()),
            initial,
            due,
            clock: Arc::new(Mutex::new(Instant::now())),
        }
    }

    fn coordinator(&self) -> IssueCoordinatorService {
        self.coordinator_with_model(self.model.clone())
    }

    fn coordinator_with_model(
        &self,
        model: Arc<FakeIssueAssessmentModelAdapter>,
    ) -> IssueCoordinatorService {
        let clock = self.clock.clone();
        self.h
            .coordinator()
            .with_issue_binding_admission(Arc::new(
                crate::application::service::issue_binding_admission::IssueBindingAdmission::new(),
            ))
            .with_arbiter(model, self.evidence.clone())
            .with_arbiter_clock(Arc::new(move || *clock.lock().unwrap()))
    }

    fn track(&self, at: u64) -> Vec<IssueCoordinatorFact> {
        track(&self.coordinator(), at, true)
    }

    fn stored(&self) -> StoredIssueDecision {
        self.h
            .fixture
            .store()
            .get_issue_decision(&self.h.fixture.workspace, self.initial.request.id())
            .unwrap()
            .unwrap()
    }

    fn pending_attempt(&self) -> String {
        let requests = self.model.requests();
        assert_eq!(requests.len(), 1);
        requests[0].attempt_id.clone()
    }

    fn assert_wait(&self, reason: IssueArbiterWaitReason) {
        assert_eq!(
            self.stored(),
            self.initial,
            "a Wait must preserve the original decision and revision"
        );
        let snapshot = self
            .h
            .fixture
            .store()
            .issue_decision_arbiter_waits(
                &self.h.fixture.workspace,
                &[self.initial.request.id().clone()],
            )
            .unwrap();
        assert_eq!(snapshot.waits.len(), 1);
        assert_eq!(snapshot.waits[0].reason, reason);
        assert_eq!(snapshot.waits[0].decision_revision, self.initial.revision);
        assert!(self.h.work().is_none());
        assert_eq!(self.h.execution.state.lock().unwrap().registrations, 0);
    }
}

fn track(
    coordinator: &IssueCoordinatorService,
    now: u64,
    enabled: bool,
) -> Vec<IssueCoordinatorFact> {
    let facts = Mutex::new(Vec::new());
    coordinator
        .track_execution(
            &IssueCollectionCancellation::new(),
            now,
            enabled,
            &|committed| facts.lock().unwrap().extend(committed),
        )
        .unwrap();
    facts.into_inner().unwrap()
}

fn events(facts: &[IssueCoordinatorFact]) -> Vec<IssueArbiterFinishedEvent> {
    facts
        .iter()
        .filter_map(|fact| match fact {
            IssueCoordinatorFact::DecisionArbiterFinished(event) => Some(event.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn arbiter_dispatches_once_at_due_and_uses_normal_start_admission_after_completion() {
    let a = ArbiterHarness::new();
    a.track(a.due - 1);
    assert!(a.model.requests().is_empty());
    assert_eq!(a.evidence.reads(), 0);
    assert_eq!(a.h.execution.state.lock().unwrap().registrations, 0);
    a.track(a.due);
    let id = a.pending_attempt();
    assert_eq!(a.stored(), a.initial);
    let source_reads = a.h.source.calls.load(Ordering::SeqCst);
    let evidence_reads = a.evidence.reads();
    for now in [a.due, a.due + 1, a.due + 2] {
        a.track(now);
    }
    assert_eq!(a.model.requests().len(), 1);
    assert_eq!(a.h.source.calls.load(Ordering::SeqCst), source_reads);
    assert_eq!(a.evidence.reads(), evidence_reads);
    assert_eq!(a.model.observations().len(), 3);
    assert!(
        !a.model.requests()[0]
            .input_json
            .contains(&a.h.fixture.workspace)
    );

    a.model.complete(&id, REPLY);
    let completed = a.track(a.due + 3);
    let answered = a.stored();
    assert_eq!(answered.revision, a.initial.revision + 1);
    assert_eq!(
        answered.request.answer().unwrap().by,
        IssueDecisionAnswerBy::Arbiter
    );
    let work = a.h.work().unwrap();
    assert_eq!(
        work.start_authority,
        IssueWorkStartAuthority::TimeoutArbiter
    );
    assert_eq!(work.decision_id.as_ref(), Some(a.initial.request.id()));
    assert_eq!(a.h.execution.state.lock().unwrap().registrations, 1);
    assert_eq!(a.h.execution.state.lock().unwrap().grants.len(), 1);
    assert_eq!(a.model.forgotten(), vec![id]);
    let completed_events = events(&completed);
    assert_eq!(completed_events.len(), 1);
    assert_eq!(
        completed_events[0].outcome,
        IssueArbiterFinishedOutcome::Applied {}
    );
    assert!(
        !completed.iter().any(
            |fact| matches!(fact, IssueCoordinatorFact::Decision { fact, .. }
        if fact.answer_by == Some(IssueDecisionAnswerBy::Arbiter))
        ),
        "the durable arbiter event is the only resolution writer"
    );
    a.coordinator()
        .acknowledge_event(&completed_events[0].event_id)
        .unwrap();
    a.track(a.due + 4);
    assert_eq!(a.stored(), answered);
    assert_eq!(a.model.requests().len(), 1);
    assert_eq!(a.h.execution.state.lock().unwrap().registrations, 1);
}

#[test]
fn arbiter_low_confidence_blockers_and_invalid_replies_remain_open_after_ack_and_restart() {
    for (reply, reason) in [
        (
            r#"{"decision":"start","confidence":8999,"reasons":[],"blockers":[]}"#,
            IssueArbiterWaitReason::LowConfidence,
        ),
        (
            r#"{"decision":"start","confidence":10000,"reasons":[],"blockers":["추가 확인이 필요합니다."]}"#,
            IssueArbiterWaitReason::BlockersPresent,
        ),
        (
            r#"{"decision":"start","confidence":10000,"reasons":[],"blockers":[],"by":"human"}"#,
            IssueArbiterWaitReason::InvalidResponse,
        ),
        (
            r#"{"decision":"unknown-option","confidence":10000,"reasons":[],"blockers":[]}"#,
            IssueArbiterWaitReason::InvalidResponse,
        ),
    ] {
        let a = ArbiterHarness::new();
        a.track(a.due);
        let id = a.pending_attempt();
        a.model.complete(&id, reply);
        let finished = events(&a.track(a.due + 1));
        assert_eq!(finished.len(), 1);
        assert_eq!(
            finished[0].outcome,
            IssueArbiterFinishedOutcome::Waiting { reason }
        );
        a.assert_wait(reason);
        a.coordinator()
            .acknowledge_event(&finished[0].event_id)
            .unwrap();
        let restarted_model = Arc::new(FakeIssueAssessmentModelAdapter::default());
        let recovered = a.coordinator_with_model(restarted_model.clone());
        assert!(events(&track(&recovered, a.due + 5_000, true)).is_empty());
        a.assert_wait(reason);
        assert!(restarted_model.requests().is_empty());
        assert!(restarted_model.observations().is_empty());
        assert_eq!(a.model.forgotten(), vec![id]);
    }
}

#[test]
fn arbiter_late_completion_preserves_the_first_verified_human_answer() {
    let a = ArbiterHarness::new();
    a.track(a.due);
    let id = a.pending_attempt();
    a.h.answer("reject");
    let human = a.stored();
    assert_eq!(
        human.request.answer().unwrap().by,
        IssueDecisionAnswerBy::Human
    );
    a.model.complete(&id, REPLY);
    let finished = events(&a.track(a.due + 1));
    assert_eq!(a.stored(), human);
    assert_eq!(finished.len(), 1);
    assert_eq!(
        finished[0].outcome,
        IssueArbiterFinishedOutcome::Discarded {}
    );
    assert_eq!(
        finished[0].fact.answer_by,
        Some(IssueDecisionAnswerBy::Human)
    );
    assert_eq!(a.model.cancelled(), vec![id]);
    assert_eq!(a.h.execution.state.lock().unwrap().registrations, 0);
}

#[test]
fn arbiter_restart_observes_the_existing_claim_and_never_dispatches_it_again() {
    let a = ArbiterHarness::new();
    a.track(a.due);
    let id = a.pending_attempt();
    let restarted_model = Arc::new(FakeIssueAssessmentModelAdapter::default());
    let recovered = a.coordinator_with_model(restarted_model.clone());
    let finished = events(&track(&recovered, a.due + 1, true));
    assert_eq!(finished.len(), 1);
    a.assert_wait(IssueArbiterWaitReason::OutcomeUnknown);
    assert_eq!(restarted_model.observations(), vec![id.clone()]);
    assert!(restarted_model.requests().is_empty());
    assert_eq!(restarted_model.forgotten(), vec![id]);
    track(&recovered, a.due + 2, true);
    assert_eq!(restarted_model.observations().len(), 1);
    assert!(restarted_model.requests().is_empty());
}

#[test]
fn arbiter_evidence_or_route_drift_discards_the_response_without_revising_the_decision() {
    for change in ["commit", "contents", "route"] {
        let a = ArbiterHarness::new();
        a.track(a.due);
        let id = a.pending_attempt();
        a.model.complete(&id, REPLY);
        match change {
            "commit" => a.evidence.set_commit_sha("b".repeat(40)),
            "contents" => a
                .evidence
                .set_file("README.md", "A different bounded snapshot.\n"),
            "route" => {
                let mut configuration = a.model.configuration().unwrap();
                configuration.route_revision = "b".repeat(64);
                a.model.set_configuration(configuration);
            }
            _ => unreachable!(),
        }
        a.track(a.due + 1);
        a.assert_wait(if change == "route" {
            IssueArbiterWaitReason::ContextUnavailable
        } else {
            IssueArbiterWaitReason::EvidenceChanged
        });
        assert_eq!(a.model.requests().len(), 1);
    }
}

#[test]
fn arbiter_fences_a_late_response_after_off_hold_or_execution_disable() {
    for intervention in ["off", "hold", "disabled"] {
        let a = ArbiterHarness::new();
        a.track(a.due);
        let id = a.pending_attempt();
        a.model.complete(&id, REPLY);
        match intervention {
            "off" => {
                autonomy(&a.h)
                    .set("stop-arbiter".into(), IssueAutonomyLevel::Off, None)
                    .unwrap();
            }
            "hold" => {
                a.h.execution.state.lock().unwrap().control.held = true;
            }
            "disabled" => {}
            _ => unreachable!(),
        }
        track(&a.coordinator(), a.due + 1, intervention != "disabled");
        assert!(a.stored().request.answer().is_none());
        assert_eq!(a.h.execution.state.lock().unwrap().registrations, 0);
        assert_eq!(a.model.requests().len(), 1);
        assert_eq!(a.model.cancelled(), vec![id]);
    }
}

#[test]
fn arbiter_unavailable_budget_and_private_evidence_wait_without_any_model_call() {
    for (failure, reason) in [
        ("model", IssueArbiterWaitReason::ModelUnavailable),
        ("budget", IssueArbiterWaitReason::BudgetExhausted),
        ("evidence", IssueArbiterWaitReason::EvidenceUnavailable),
        ("private", IssueArbiterWaitReason::EvidenceUnavailable),
    ] {
        let a = ArbiterHarness::new();
        match failure {
            "model" => a.model.make_unavailable(),
            "budget" => {
                let mut configuration = a.model.configuration().unwrap();
                configuration.max_started_per_day = 0;
                a.model.set_configuration(configuration);
            }
            "evidence" => a.evidence.make_unavailable(),
            "private" => a
                .evidence
                .set_file("README.md", "token=fixture-not-a-real-secret"),
            _ => unreachable!(),
        }
        let first = events(&a.track(a.due));
        assert_eq!(first.len(), 1);
        a.assert_wait(reason);
        a.coordinator()
            .acknowledge_event(&first[0].event_id)
            .unwrap();
        assert!(events(&a.track(a.due + 1)).is_empty());
        assert!(a.model.requests().is_empty());
        assert!(
            a.h.fixture
                .store()
                .unfinished_issue_arbiter_attempts(&a.h.fixture.workspace, None, 256)
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn arbiter_model_timeout_cancels_once_and_preserves_open_wait() {
    let a = ArbiterHarness::new();
    a.track(a.due);
    let id = a.pending_attempt();
    a.track(a.due + 179_999);
    assert!(a.model.cancelled().is_empty());
    a.track(a.due + 180_000);
    a.assert_wait(IssueArbiterWaitReason::OutcomeUnknown);
    assert_eq!(a.model.cancelled(), vec![id]);
    a.track(a.due + 180_001);
    assert_eq!(a.model.cancelled().len(), 1);
    assert_eq!(a.model.requests().len(), 1);
}

#[test]
fn arbiter_ordinary_agent_answers_cannot_preempt_a_timeout_arbiter_gate() {
    use crate::application::port::inbound::stored_command_port::StoredCommandContext;
    let a = ArbiterHarness::new();
    let context = StoredCommandContext {
        human_confirmation: None,
        caller: Some("fixture-agent".into()),
        channel: "gateway".into(),
        at: "1970-01-01T00:00:00.500Z".into(),
    };
    assert!(
        a.h.decisions
            .answer(a.initial.request.id().as_str(), "start", None, &context)
            .is_err()
    );
    assert_eq!(a.stored(), a.initial);
    a.track(a.due);
    assert_eq!(a.model.requests().len(), 1);
}

#[test]
fn arbiter_success_notes_are_privacy_checked_before_they_reach_the_ledger() {
    let a = ArbiterHarness::new();
    a.track(a.due);
    let id = a.pending_attempt();
    a.model.complete(&id, r#"{"decision":"start","confidence":9500,"reasons":["token=fixture-not-a-real-secret"],"blockers":[]}"#);
    a.track(a.due + 1);
    let stored = a.stored();
    let encoded = serde_json::to_string(&stored.request).unwrap();
    assert!(!encoded.contains("fixture-not-a-real-secret"));
    if let Some(answer) = stored.request.answer() {
        assert_eq!(answer.by, IssueDecisionAnswerBy::Arbiter);
    } else {
        a.assert_wait(IssueArbiterWaitReason::InvalidResponse);
    }
}

#[test]
fn arbiter_private_policy_strings_never_reach_the_model() {
    let a = ArbiterHarness::with_policy(|policy| {
        format!("{policy}\n[labels]\nhold = \"token=private-fixture-value\"\n")
    });
    let finished = events(&a.track(a.due));
    assert_eq!(finished.len(), 1);
    a.assert_wait(IssueArbiterWaitReason::EvidenceUnavailable);
    assert!(a.model.requests().is_empty());
    assert!(
        a.h.fixture
            .store()
            .unfinished_issue_arbiter_attempts(&a.h.fixture.workspace, None, 256,)
            .unwrap()
            .is_empty()
    );
    assert!(
        !serde_json::to_string(&finished[0])
            .unwrap()
            .contains("private-fixture-value")
    );
}

#[test]
fn arbiter_rechecks_the_route_when_it_changes_during_evidence_capture() {
    for completing in [false, true] {
        let a = ArbiterHarness::new();
        if completing {
            a.track(a.due);
            a.model.complete(&a.pending_attempt(), REPLY);
        }
        let model = a.model.clone();
        a.evidence.after_next_read(move || {
            let mut configuration = model.configuration().unwrap();
            configuration.route_revision = "b".repeat(64);
            model.set_configuration(configuration);
        });
        let finished = events(&a.track(a.due + u64::from(completing)));
        assert_eq!(finished.len(), 1);
        a.assert_wait(IssueArbiterWaitReason::EvidenceChanged);
        assert_eq!(a.model.requests().len(), usize::from(completing));
        assert_eq!(a.model.forgotten().len(), usize::from(completing));
    }
}

#[test]
fn arbiter_evidence_capture_cannot_apply_a_reply_after_its_hard_finish_deadline() {
    let a = ArbiterHarness::new();
    a.track(a.due);
    let id = a.pending_attempt();
    a.model.complete(&id, REPLY);
    let clock = a.clock.clone();
    a.evidence.after_next_read(move || {
        let mut now = clock.lock().unwrap();
        *now += Duration::from_secs(180);
    });
    let finished = events(&a.track(a.due + 1));
    assert_eq!(finished.len(), 1);
    a.assert_wait(IssueArbiterWaitReason::OutcomeUnknown);
    assert_eq!(a.model.requests().len(), 1);
    assert_eq!(a.model.forgotten(), vec![id]);
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(&finished[0].at)
            .unwrap()
            .timestamp_millis(),
        (a.due + 180_001) as i64,
        "the receipt uses the injected UTC plus elapsed monotonic time",
    );
}

#[test]
fn arbiter_claim_starts_its_call_deadline_after_the_input_capture_finishes() {
    let a = ArbiterHarness::new();
    let clock = a.clock.clone();
    a.evidence.after_next_read(move || {
        let mut now = clock.lock().unwrap();
        *now += Duration::from_secs(180);
    });
    a.track(a.due);
    assert_eq!(a.model.requests().len(), 1);
    let attempts =
        a.h.fixture
            .store()
            .unfinished_issue_arbiter_attempts(&a.h.fixture.workspace, None, 256)
            .unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].claimed_at_unix_ms, a.due + 180_000);
    assert_eq!(attempts[0].deadline_at_unix_ms, a.due + 360_000);
    assert_eq!(a.model.requests()[0].timeout, Duration::from_secs(180));
    assert_eq!(a.stored(), a.initial);
}
