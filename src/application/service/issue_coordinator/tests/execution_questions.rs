use super::*;
use crate::application::port::inbound::stored_command_port::{
    StoredCommandContext, issue_confirmation::VerifiedHumanConfirmation,
};
use crate::application::port::outbound::issue_automation_store_port::{
    IssueDecisionFence, IssueTriageSaveResult,
};
use crate::application::port::outbound::issue_work_execution_port::{
    IssueWorkAnswerDelivery, IssueWorkAnswerReceipt, IssueWorkAnswerRequest,
    IssueWorkExecutionSnapshot,
};
use crate::domain::issue_automation::decision::{
    IssueDecisionAgentAnswer, IssueDecisionAnswerBy, IssueDecisionOutcome, IssueDecisionSharedText,
    IssueDecisionState,
};
use crate::domain::issue_automation::execution::*;
use crate::domain::issue_automation::execution_answer::{
    IssueExecutionAnswerRecord, IssueExecutionAnswerStatus,
};
use crate::domain::issue_automation::policy::IssueAutomationGateAction;
use crate::domain::issue_automation::work_control::{IssueWorkControl, IssueWorkControlAction};
use std::sync::atomic::AtomicUsize;

const REPLY: &str = "기존 작업의 계약을 유지하고 검증 결과를 확인해 주세요.";

struct AnswerExecution {
    base: Arc<FakeExecution>,
    calls: Mutex<Vec<IssueWorkAnswerRequest>>,
    receipt: Mutex<Option<IssueWorkAnswerReceipt>>,
    polls: AtomicUsize,
    lose_first_ack: AtomicBool,
}

impl AnswerExecution {
    fn new(base: Arc<FakeExecution>) -> Arc<Self> {
        Arc::new(Self {
            base,
            calls: Mutex::new(Vec::new()),
            receipt: Mutex::new(None),
            polls: AtomicUsize::new(0),
            lose_first_ack: AtomicBool::new(false),
        })
    }
}

impl IssueWorkExecutionPort for AnswerExecution {
    fn frozen_delivery_source(&self, workspace: &str, work: &str) -> Result<Option<String>> {
        self.base.frozen_delivery_source(workspace, work)
    }
    fn delivery_diff(
        &self,
        workspace: &str,
        work: &str,
        source: &str,
    ) -> Result<crate::domain::issue_automation::delivery::IssueDeliveryDiff> {
        self.base.delivery_diff(workspace, work, source)
    }
    fn control(&self, workspace: &str, work: &str) -> Result<IssueWorkControl> {
        self.base.control(workspace, work)
    }
    fn apply_control(
        &self,
        workspace: &str,
        work: &str,
        command: &str,
        action: IssueWorkControlAction,
    ) -> Result<IssueWorkControl> {
        self.base.apply_control(workspace, work, command, action)
    }
    fn execution_limit(&self, workspace: &str, work: &str, limit: u32) -> Result<()> {
        self.base.execution_limit(workspace, work, limit)
    }
    fn permit_gate_revision(&self, workspace: &str, work: &str) -> Result<u64> {
        self.base.permit_gate_revision(workspace, work)
    }
    fn observe_execution(
        &self,
        workspace: &str,
        work: &str,
    ) -> Result<Option<IssueWorkExecutionObservation>> {
        self.base.observe_execution(workspace, work)
    }
    fn snapshot(&self, workspace: &str, work: &str) -> Result<IssueWorkExecutionSnapshot> {
        self.base.snapshot(workspace, work)
    }
    fn task_ids(&self, workspace: &str, work: &str) -> Result<Vec<String>> {
        self.base.task_ids(workspace, work)
    }
    fn submit(&self, workspace: &str, order: &IssueWorkOrder) -> Result<Vec<String>> {
        self.base.submit(workspace, order)
    }
    fn grant(&self, workspace: &str, request: &IssueWorkPermitRequest) -> Result<()> {
        self.base.grant(workspace, request)
    }
    fn revoke(&self, workspace: &str, work: &str) -> Result<()> {
        self.base.revoke(workspace, work)
    }
    fn answer_question(
        &self,
        _: &str,
        request: &IssueWorkAnswerRequest,
    ) -> Result<IssueWorkAnswerReceipt> {
        self.calls.lock().unwrap().push(request.clone());
        let receipt = IssueWorkAnswerReceipt {
            command_id: request.command_id.clone(),
            question_id: request.question.question_id.clone(),
            delivery: IssueWorkAnswerDelivery::Queued,
        };
        *self.receipt.lock().unwrap() = Some(receipt.clone());
        if self.lose_first_ack.swap(false, Ordering::SeqCst) {
            bail!("simulated lost reply ACK");
        }
        Ok(receipt)
    }
    fn question_answer_receipt(
        &self,
        _: &str,
        _: &str,
        command: &str,
    ) -> Result<Option<IssueWorkAnswerReceipt>> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        let receipt = self.receipt.lock().unwrap().clone();
        if let Some(receipt) = &receipt {
            assert_eq!(receipt.command_id, command);
        }
        Ok(receipt)
    }
}

fn started() -> (Harness, Arc<AnswerExecution>) {
    let h = Harness::new();
    h.tick(true).unwrap();
    h.answer("start");
    h.tick(true).unwrap();
    let execution = AnswerExecution::new(h.execution.clone());
    (h, execution)
}

fn question(h: &Harness) -> IssueExecutionQuestion {
    IssueExecutionQuestion {
        question_id: "question-1".into(),
        task_id: "task-1".into(),
        session_key: "isolated-task-1".into(),
        thread_id: "original-thread".into(),
        slot_id: "slot-1".into(),
        worktree_path: h
            .fixture
            .root
            .join("isolated")
            .to_string_lossy()
            .into_owned(),
        branch_name: "akra/task-1".into(),
        lease_generation: "old-generation".into(),
        awaiting_task_updated_at: "2026-11-21T00:00:00Z".into(),
    }
}

fn observe_question(h: &Harness) {
    h.execution.state.lock().unwrap().observation = Some(IssueWorkExecutionObservation {
        phase: IssueExecutionPhase::Running,
        task_ids: vec!["task-1".into()],
        active_task_id: Some("task-1".into()),
        execution_ids: vec!["task-1".into()],
        question: Some(question(h)),
        ..Default::default()
    });
}

fn service(h: &Harness, execution: Arc<AnswerExecution>) -> IssueCoordinatorService {
    IssueCoordinatorService::new(
        h.fixture.workspace.clone(),
        h.fixture.store(),
        h.source.clone(),
        h.triage.clone(),
        h.decisions.clone(),
        IssuePolicyService::new(Arc::new(ValidationPolicy)),
        execution,
    )
}

fn track(
    h: &Harness,
    execution: &Arc<AnswerExecution>,
    enabled: bool,
) -> Result<Vec<IssueCoordinatorFact>> {
    let facts = Mutex::new(Vec::new());
    // Reconstruct the service and SQLite adapter on every tick to exercise restart behavior.
    service(h, execution.clone()).track_execution(
        &IssueCollectionCancellation::new(),
        123,
        enabled,
        &|values| facts.lock().unwrap().extend(values),
    )?;
    Ok(facts.into_inner().unwrap())
}

fn opened_question(h: &Harness) -> StoredIssueDecision {
    page(&h.triage).items[0]
        .open_decisions
        .iter()
        .find(|decision| decision.request.definition().kind == IssueDecisionKind::AgentQuestion)
        .unwrap()
        .clone()
}

fn context(h: &Harness, decision: &StoredIssueDecision, option: &str) -> StoredCommandContext {
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
        at: "2026-11-21T00:00:00Z".into(),
    }
}

fn reply(h: &Harness) -> StoredIssueDecision {
    let decision = opened_question(h);
    let result = h
        .decisions
        .answer(
            decision.request.id().as_str(),
            "reply",
            Some(REPLY),
            &context(h, &decision, "reply"),
        )
        .unwrap();
    assert_eq!(result.outcome, IssueDecisionOutcome::Answered);
    result.commit.record
}

fn pending(h: &Harness) -> Vec<IssueExecutionAnswerRecord> {
    h.fixture
        .store()
        .pending_issue_execution_answers(
            &h.fixture.workspace,
            &h.work().unwrap().order.work_order_id,
        )
        .unwrap()
}

fn pause_collection(h: &Harness) {
    let store = h.fixture.store();
    let collection = store.get_collection(&h.fixture.workspace).unwrap().unwrap();
    store
        .set_collection_mode(
            &h.fixture.workspace,
            Some(collection.epoch),
            IssueCollectionMode::Off,
        )
        .unwrap();
}

#[test]
fn the_same_question_opens_one_fixed_decision_without_starting_any_work() {
    let (h, execution) = started();
    let before_grants = h.execution.state.lock().unwrap().grants.len();
    let before_reads = h.source.calls.load(Ordering::SeqCst);
    observe_question(&h);
    track(&h, &execution, false).unwrap();
    let first = opened_question(&h);
    track(&h, &execution, true).unwrap();
    assert_eq!(opened_question(&h), first);
    let definition = first.request.definition();
    assert_eq!(definition.gate, IssueAutomationGate::Spec);
    assert_eq!(definition.evidence, ["question:question-1"]);
    assert_eq!(definition.on_timeout, IssueDecisionTimeout::Wait);
    assert!(definition.due_at.is_some());
    assert_eq!(
        definition.basis.work_order_id,
        Some(h.work().unwrap().order.work_order_id)
    );
    assert!(execution.calls.lock().unwrap().is_empty());
    assert!(pending(&h).is_empty());
    let state = h.execution.state.lock().unwrap();
    assert_eq!(state.registrations, 1);
    assert_eq!(state.grants.len(), before_grants);
    assert_eq!(h.source.calls.load(Ordering::SeqCst), before_reads);
}

#[test]
fn a_worker_reply_requires_both_a_verified_human_and_a_shared_note() {
    let (h, execution) = started();
    observe_question(&h);
    track(&h, &execution, true).unwrap();
    let open = opened_question(&h);
    let mut unverified = context(&h, &open, "reply");
    unverified.human_confirmation = None;
    assert!(
        h.decisions
            .answer(
                open.request.id().as_str(),
                "reply",
                Some(REPLY),
                &unverified
            )
            .is_err()
    );
    assert!(
        h.decisions
            .answer(
                open.request.id().as_str(),
                "reply",
                None,
                &context(&h, &open, "reply")
            )
            .is_err()
    );
    assert_eq!(opened_question(&h), open);
    track(&h, &execution, true).unwrap();
    assert!(execution.calls.lock().unwrap().is_empty());
    let answered = reply(&h);
    assert_eq!(
        answered.request.answer().unwrap().by,
        IssueDecisionAnswerBy::Human
    );
    track(&h, &execution, true).unwrap();
    let calls = execution.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].question, question(&h));
    assert_eq!(calls[0].message, REPLY);
    assert_eq!(
        calls[0].authorization_epoch,
        h.work()
            .unwrap()
            .current_authorization_basis()
            .authorization_epoch
    );
}

#[test]
fn a_lost_answer_ack_is_polled_after_restart_without_a_second_send() {
    let (h, execution) = started();
    observe_question(&h);
    track(&h, &execution, true).unwrap();
    reply(&h);
    execution.lose_first_ack.store(true, Ordering::SeqCst);
    track(&h, &execution, true).unwrap();
    assert_eq!(pending(&h)[0].status, IssueExecutionAnswerStatus::Unknown);
    let intent = pending(&h)[0].intent.clone();
    for _ in 0..3 {
        track(&h, &execution, true).unwrap();
    }
    assert_eq!(execution.calls.lock().unwrap().len(), 1);
    assert_eq!(pending(&h)[0].status, IssueExecutionAnswerStatus::Queued);
    assert_eq!(execution.polls.load(Ordering::SeqCst), 3);

    execution.receipt.lock().unwrap().as_mut().unwrap().delivery = IssueWorkAnswerDelivery::Unknown;
    track(&h, &execution, true).unwrap();
    assert_eq!(pending(&h)[0].status, IssueExecutionAnswerStatus::Unknown);
    execution.receipt.lock().unwrap().as_mut().unwrap().delivery =
        IssueWorkAnswerDelivery::Accepted {
            thread_id: "original-thread".into(),
            turn_id: "answer-turn".into(),
            lease_generation: "new-generation".into(),
        };
    h.execution
        .state
        .lock()
        .unwrap()
        .observation
        .as_mut()
        .unwrap()
        .question = None;
    track(&h, &execution, false).unwrap();
    assert!(pending(&h).is_empty());
    let stored = h
        .fixture
        .store()
        .claim_issue_execution_answer(&h.fixture.workspace, &intent)
        .unwrap();
    assert!(!stored.created);
    assert!(matches!(
        stored.record.status,
        IssueExecutionAnswerStatus::Accepted { .. }
    ));
    assert_eq!(execution.calls.lock().unwrap().len(), 1);
    assert_eq!(
        h.fixture
            .store()
            .get_issue_execution(&h.fixture.workspace, &h.fixture.reference("I_001"))
            .unwrap()
            .unwrap()
            .progress
            .attempts
            .execution,
        1,
    );
}

#[test]
fn a_receipt_for_the_wrong_question_or_generation_cannot_settle_the_claim() {
    for invalid_question in [true, false] {
        let (h, execution) = started();
        observe_question(&h);
        track(&h, &execution, true).unwrap();
        reply(&h);
        execution.lose_first_ack.store(true, Ordering::SeqCst);
        track(&h, &execution, true).unwrap();
        {
            let mut receipt = execution.receipt.lock().unwrap();
            let receipt = receipt.as_mut().unwrap();
            if invalid_question {
                receipt.question_id = "different-question".into();
            } else {
                receipt.delivery = IssueWorkAnswerDelivery::Accepted {
                    thread_id: "original-thread".into(),
                    turn_id: "answer-turn".into(),
                    lease_generation: "old-generation".into(),
                };
            }
        }
        assert!(track(&h, &execution, true).is_err());
        assert_eq!(pending(&h)[0].status, IssueExecutionAnswerStatus::Unknown);
        assert_eq!(execution.calls.lock().unwrap().len(), 1);
    }
}

#[test]
fn collection_off_allows_registered_questions_and_explicit_answers_without_new_permits() {
    let (h, execution) = started();
    pause_collection(&h);
    let before_reads = h.source.calls.load(Ordering::SeqCst);
    let before_grants = h.execution.state.lock().unwrap().grants.len();
    observe_question(&h);
    track(&h, &execution, true).unwrap();
    reply(&h);
    track(&h, &execution, true).unwrap();
    assert_eq!(execution.calls.lock().unwrap().len(), 1);
    assert_eq!(h.source.calls.load(Ordering::SeqCst), before_reads);
    let state = h.execution.state.lock().unwrap();
    assert_eq!(state.registrations, 1);
    assert_eq!(state.grants.len(), before_grants);
}

#[test]
fn disabled_execution_and_cancelled_tracking_never_claim_or_send_a_reply() {
    let (h, execution) = started();
    observe_question(&h);
    track(&h, &execution, false).unwrap();
    reply(&h);
    track(&h, &execution, false).unwrap();
    assert!(pending(&h).is_empty());
    assert!(execution.calls.lock().unwrap().is_empty());
    let cancel = IssueCollectionCancellation::new();
    cancel.cancel();
    service(&h, execution.clone())
        .track_execution(&cancel, 123, true, &|_| {})
        .unwrap();
    assert!(pending(&h).is_empty());
    assert!(execution.calls.lock().unwrap().is_empty());
}

#[test]
fn blocking_work_controls_cannot_use_an_old_human_reply() {
    for block in ["held", "taken_over", "cancelled", "untracked", "epoch"] {
        let (h, execution) = started();
        observe_question(&h);
        track(&h, &execution, true).unwrap();
        reply(&h);
        {
            let mut state = h.execution.state.lock().unwrap();
            match block {
                "held" => state.control.held = true,
                "taken_over" => state.control.taken_over = true,
                "cancelled" => state.control.cancelled = true,
                "untracked" => state.control.tracked = false,
                "epoch" => state.control.authorization_epoch += 1,
                _ => unreachable!(),
            }
        }
        track(&h, &execution, true).unwrap();
        assert!(pending(&h).is_empty());
        assert!(execution.calls.lock().unwrap().is_empty());
    }
}

#[test]
fn changing_full_question_coordinates_cannot_reuse_an_answer_with_the_same_question_id() {
    let (h, execution) = started();
    observe_question(&h);
    track(&h, &execution, true).unwrap();
    reply(&h);
    h.execution
        .state
        .lock()
        .unwrap()
        .observation
        .as_mut()
        .unwrap()
        .question
        .as_mut()
        .unwrap()
        .thread_id = "another-thread".into();
    assert!(track(&h, &execution, true).is_err());
    assert!(execution.calls.lock().unwrap().is_empty());
    assert!(pending(&h).is_empty());
}

#[test]
fn collection_off_triage_requires_a_registered_task_and_the_current_epoch() {
    for registered in [false, true] {
        let h = Harness::new();
        h.tick(true).unwrap();
        h.answer("start");
        if !registered {
            h.execution.state.lock().unwrap().fail_before_register = true;
            assert!(h.tick(true).is_err());
            assert!(h.work().unwrap().task_ids.is_empty());
        } else {
            h.tick(true).unwrap();
        }
        let store = h.fixture.store();
        let before = store
            .read_triage_batch(&h.fixture.workspace, None, 100)
            .unwrap();
        let old_epoch = before.collection.unwrap().epoch;
        pause_collection(&h);
        let batch = store
            .read_triage_batch(&h.fixture.workspace, None, 100)
            .unwrap();
        let item = batch
            .items
            .into_iter()
            .find(|item| item.issue == h.fixture.reference("I_001"))
            .unwrap();
        let assessment = h.triage.assess_current(&item).unwrap();
        let mut fence = IssueTriageFence {
            collection_epoch: old_epoch,
            observation_revision: item.observation_revision,
            tracked_revision: item.tracked_revision,
            assessment_revision: item.triage.as_ref().map(|triage| triage.revision),
        };
        assert_eq!(
            store
                .save_triage(&h.fixture.workspace, fence, &assessment)
                .unwrap(),
            IssueTriageSaveResult::Conflict
        );
        fence.collection_epoch = batch.collection.unwrap().epoch;
        let saved = store
            .save_triage(&h.fixture.workspace, fence, &assessment)
            .unwrap();
        assert_eq!(
            matches!(saved, IssueTriageSaveResult::Saved { .. }),
            registered
        );
    }
    let h = Harness::new();
    pause_collection(&h);
    let store = h.fixture.store();
    let batch = store
        .read_triage_batch(&h.fixture.workspace, None, 100)
        .unwrap();
    let item = &batch.items[0];
    let assessment = h.triage.assess_current(item).unwrap();
    assert_eq!(
        store
            .save_triage(
                &h.fixture.workspace,
                IssueTriageFence {
                    collection_epoch: batch.collection.unwrap().epoch,
                    observation_revision: item.observation_revision,
                    tracked_revision: item.tracked_revision,
                    assessment_revision: item.triage.as_ref().map(|triage| triage.revision),
                },
                &assessment
            )
            .unwrap(),
        IssueTriageSaveResult::Conflict
    );
}

#[test]
fn work_failure_recovery_requires_a_human_at_both_service_and_store_boundaries() {
    let h = Harness::new();
    *h.policy.text.lock().unwrap() = super::gated::gated_policy();
    super::gated::enable_gated(&h);
    h.tick(true).unwrap();
    let work = h.work().unwrap();
    h.execution.state.lock().unwrap().observation = Some(IssueWorkExecutionObservation {
        phase: IssueExecutionPhase::Terminal,
        task_ids: work.task_ids.clone(),
        active_task_id: Some("task-1".into()),
        execution_ids: vec!["task-1".into()],
        terminals: vec![IssueExecutionTerminalRecord {
            record_id: "terminal-recovery".into(),
            classification: IssueExecutionTerminalClass::ContractRecovery,
        }],
        current_terminal_id: Some("terminal-recovery".into()),
        ..Default::default()
    });
    h.coordinator()
        .track_execution(&IssueCollectionCancellation::new(), 123, false, &|_| {})
        .unwrap();
    run(&h.triage);
    let store = h.fixture.store();
    let batch = store
        .read_triage_batch(&h.fixture.workspace, None, 100)
        .unwrap();
    let item = &batch.items[0];
    let assessment = &item.triage.as_ref().unwrap().assessment;
    assert_eq!(
        assessment
            .gates()
            .iter()
            .find(|gate| gate.gate == IssueAutomationGate::Start)
            .unwrap()
            .action,
        IssueAutomationGateAction::Auto
    );
    let text = |value: &str| IssueDecisionSharedText::new_shared(value).unwrap();
    let definition = IssueDecisionDefinition {
        delivery_review: None,
        subject: IssueDecisionSubject::Issue(work.order.issue.clone()),
        gate: IssueAutomationGate::Start,
        kind: IssueDecisionKind::FailureRecovery,
        question: text("기존 작업의 복구 방향을 결정해 주세요."),
        options: vec![IssueDecisionOption {
            id: "repair".into(),
            label: text("복구 방향 기록"),
            effect: text("설정 복구 뒤 기존 검증을 확인한다."),
        }],
        recommended: None,
        basis: IssueDecisionBasis::from_assessment(
            assessment,
            IssueAutomationGate::Start,
            Some(work.order.work_order_id),
        )
        .unwrap(),
        on_timeout: IssueDecisionTimeout::Wait,
        due_at: None,
        asked_by: "recovery-fixture".into(),
        evidence: vec!["terminal:terminal-recovery".into()],
    };
    let open = h
        .decisions
        .ensure_request(
            IssueDecisionId::new("forced-human-recovery").unwrap(),
            definition,
        )
        .unwrap()
        .record;
    let mut unverified = context(&h, &open, "repair");
    unverified.human_confirmation = None;
    assert!(
        h.decisions
            .answer(open.request.id().as_str(), "repair", None, &unverified)
            .is_err()
    );
    let agent = open
        .request
        .answer_agent(
            &open.request.definition().basis,
            IssueDecisionAgentAnswer {
                option: "repair".into(),
                note: None,
                caller: Some("Desktop".into()),
                channel: "gateway".into(),
                at: "2026-11-21T00:00:00Z".into(),
            },
        )
        .unwrap()
        .request;
    let fence = IssueDecisionFence {
        decision_revision: open.revision,
        triage: IssueTriageFence {
            collection_epoch: batch.collection.unwrap().epoch,
            observation_revision: item.observation_revision,
            tracked_revision: item.tracked_revision,
            assessment_revision: item.triage.as_ref().map(|triage| triage.revision),
        },
    };
    assert!(
        store
            .save_issue_decision(&h.fixture.workspace, fence, assessment, &agent)
            .is_err()
    );
    assert_eq!(
        store
            .get_issue_decision_commit(&h.fixture.workspace, open.request.id())
            .unwrap()
            .unwrap()
            .record
            .request
            .state(),
        IssueDecisionState::Open
    );
    let accepted = h
        .decisions
        .answer(
            open.request.id().as_str(),
            "repair",
            None,
            &context(&h, &open, "repair"),
        )
        .unwrap();
    assert_eq!(
        accepted.commit.record.request.answer().unwrap().by,
        IssueDecisionAnswerBy::Human
    );
    assert!(
        accepted
            .commit
            .record
            .request
            .answer()
            .unwrap()
            .note
            .is_none()
    );
}

#[test]
fn collection_off_refreshes_held_work_but_rejects_the_previous_tracked_revision() {
    let (h, _) = started();
    pause_collection(&h);
    let store = h.fixture.store();
    let before = store
        .read_triage_batch(&h.fixture.workspace, None, 100)
        .unwrap();
    let previous = &before.items[0];
    let old_fence = IssueTriageFence {
        collection_epoch: before.collection.unwrap().epoch,
        observation_revision: previous.observation_revision,
        tracked_revision: previous.tracked_revision,
        assessment_revision: previous.triage.as_ref().map(|triage| triage.revision),
    };
    let mut held = store
        .get_issue(&h.fixture.workspace, &h.fixture.reference("I_001"))
        .unwrap()
        .unwrap();
    held.set_flag(
        crate::domain::issue_automation::IssueAutomationFlag::Held,
        true,
    );
    store
        .save_issue(&h.fixture.workspace, held.revision(), &held)
        .unwrap();
    let batch = store
        .read_triage_batch(&h.fixture.workspace, None, 100)
        .unwrap();
    let current = &batch.items[0];
    let assessment = h.triage.assess_current(current).unwrap();
    assert_ne!(assessment, previous.triage.as_ref().unwrap().assessment);
    assert_eq!(
        store
            .save_triage(&h.fixture.workspace, old_fence, &assessment)
            .unwrap(),
        IssueTriageSaveResult::Conflict
    );
    let saved = store
        .save_triage(
            &h.fixture.workspace,
            IssueTriageFence {
                collection_epoch: batch.collection.unwrap().epoch,
                observation_revision: current.observation_revision,
                tracked_revision: current.tracked_revision,
                assessment_revision: current.triage.as_ref().map(|triage| triage.revision),
            },
            &assessment,
        )
        .unwrap();
    assert!(matches!(
        saved,
        IssueTriageSaveResult::Saved { changed: true, .. }
    ));
}

struct UnavailableDisclosurePolicy;

impl IssuePolicySourcePort for UnavailableDisclosurePolicy {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        bail!("unavailable disclosure fixture");
    }
    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

#[test]
fn disclosure_policy_is_rechecked_before_claiming_a_confirmed_reply() {
    let (h, execution) = started();
    observe_question(&h);
    track(&h, &execution, true).unwrap();
    reply(&h);
    let service = IssueCoordinatorService::new(
        h.fixture.workspace.clone(),
        h.fixture.store(),
        h.source.clone(),
        h.triage.clone(),
        h.decisions.clone(),
        IssuePolicyService::new(Arc::new(UnavailableDisclosurePolicy)),
        execution.clone(),
    );
    assert!(
        service
            .track_execution(&IssueCollectionCancellation::new(), 123, true, &|_| {})
            .is_err()
    );
    assert!(execution.calls.lock().unwrap().is_empty());
    assert!(pending(&h).is_empty());
}

mod reapproval;

#[test]
fn a_human_recovery_terminal_cannot_send_or_claim_an_existing_question_reply() {
    use crate::domain::issue_automation::execution_answer::{
        IssueExecutionAnswerIntent, answer_command_id, answer_hash,
    };

    for classification in [
        IssueExecutionTerminalClass::ContractRecovery,
        IssueExecutionTerminalClass::PullRequestClosed,
        IssueExecutionTerminalClass::IdentityConflict,
    ] {
        let (h, execution) = started();
        observe_question(&h);
        track(&h, &execution, true).unwrap();
        let answered = reply(&h);
        let work = h.work().unwrap();
        let original_question = question(&h);
        let intent = IssueExecutionAnswerIntent {
            command_id: answer_command_id(&work.order.work_order_id, answered.request.id()),
            issue: work.order.issue.clone(),
            work_order_id: work.order.work_order_id.clone(),
            decision_id: answered.request.id().clone(),
            question: original_question.clone(),
            authorization_epoch: work.current_authorization_basis().authorization_epoch,
            answer_sha256: answer_hash(REPLY),
        };
        {
            let mut state = h.execution.state.lock().unwrap();
            let observation = state.observation.as_mut().unwrap();
            let terminal_id = format!("question-human-recovery-{classification:?}");
            observation.phase = IssueExecutionPhase::Terminal;
            observation.terminals = vec![IssueExecutionTerminalRecord {
                record_id: terminal_id.clone(),
                classification,
            }];
            observation.current_terminal_id = Some(terminal_id);
            observation.validate().unwrap();
        }

        track(&h, &execution, true).unwrap();
        let record = h
            .fixture
            .store()
            .get_issue_execution(&h.fixture.workspace, &work.order.issue)
            .unwrap()
            .unwrap();
        assert_eq!(record.question.as_ref(), Some(&original_question));
        assert!(record.progress.requires_human_recovery());
        assert!(
            execution.calls.lock().unwrap().is_empty(),
            "{classification:?}"
        );
        assert!(pending(&h).is_empty(), "{classification:?}");

        // A stale helper that read the non-terminal question before this observation must
        // also be rejected by the current-record check inside the durable claim transaction.
        assert!(
            h.fixture
                .store()
                .claim_issue_execution_answer(&h.fixture.workspace, &intent)
                .is_err(),
            "{classification:?}",
        );
        assert!(pending(&h).is_empty());
        track(&h, &execution, true).unwrap();
        assert!(execution.calls.lock().unwrap().is_empty());
        assert!(pending(&h).is_empty());
    }
}
