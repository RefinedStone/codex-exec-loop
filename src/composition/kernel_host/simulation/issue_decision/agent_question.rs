//! IA8-1 (#2566): 실행 중인 에이전트의 질문을 커널 명령으로 연다. 결정 카드로 보이고, 자기 답과 한도 초과는
//! 거절되고, 사람의 답은 같은 thread 에 한 번 간다. 실행기는 가짜이고 모델 턴은 없다.
use super::*;

use crate::adapter::inbound::kernel_protocol::CommandResponse;
use crate::application::port::inbound::stored_command_port::{
    StoredCommandContext, StoredCommandRejection, issue_confirmation::VerifiedHumanConfirmation,
};
use crate::application::port::outbound::issue_work_execution_port::{
    IssueWorkAnswerDelivery, IssueWorkAnswerReceipt, IssueWorkAnswerRequest,
    IssueWorkExecutionPort, IssueWorkExecutionSnapshot, IssueWorkPermitRequest,
};
use crate::application::service::issue_coordinator::IssueCoordinatorService;
use crate::domain::issue_automation::agent_question::IssueAgentAsk;
use crate::domain::issue_automation::coordinator::IssueWorkOrder;
use crate::domain::issue_automation::execution::{
    IssueExecutionPhase, IssueExecutionQuestion, IssueWorkExecutionObservation,
};
use std::sync::atomic::AtomicBool;

const TASK: &str = "fake-task-1";
const BRANCH: &str = "akra-agent/slot-1/ask";
const THREAD: &str = "worker-thread";
const GENERATION: &str = "generation-1";
const POLICY: &str = r#"
[binding.github]
mode = "propose"
[budget]
max_attempts = 2
[select]
mode = "all_open"
trusted_actors = [{provider="github",server_id="github.com",actor_id="U_decision"}]
"#;

#[derive(Default)]
struct Execution {
    order: Mutex<Option<IssueWorkOrder>>,
    waiting: AtomicBool,
    awaited: AtomicUsize,
    answers: Mutex<Vec<IssueWorkAnswerRequest>>,
}

fn ask() -> IssueAgentAsk {
    IssueAgentAsk {
        task_id: TASK.into(),
        thread_id: THREAD.into(),
        lease_generation: GENERATION.into(),
    }
}

fn waiting_question() -> IssueExecutionQuestion {
    IssueExecutionQuestion {
        question_id: "question-after-ask".into(),
        task_id: TASK.into(),
        session_key: "session-1".into(),
        thread_id: THREAD.into(),
        slot_id: "slot-1".into(),
        worktree_path: "/pool/slot-1".into(),
        branch_name: BRANCH.into(),
        lease_generation: GENERATION.into(),
        awaiting_task_updated_at: "2026-11-25T00:00:00Z".into(),
    }
}

impl IssueWorkExecutionPort for Execution {
    fn execution_limit(&self, _: &str, _: &str, _: u32) -> anyhow::Result<()> {
        Ok(())
    }
    fn snapshot(&self, _: &str, _: &str) -> anyhow::Result<IssueWorkExecutionSnapshot> {
        Ok(IssueWorkExecutionSnapshot {
            dispatch_source: Some("a".repeat(40)),
            ..Default::default()
        })
    }
    fn submit(&self, _: &str, order: &IssueWorkOrder) -> anyhow::Result<Vec<String>> {
        *self.order.lock().unwrap() = Some(order.clone());
        Ok(vec![TASK.into()])
    }
    fn grant(&self, _: &str, _: &IssueWorkPermitRequest) -> anyhow::Result<()> {
        Ok(())
    }
    fn revoke(&self, _: &str, _: &str) -> anyhow::Result<()> {
        Ok(())
    }
    fn observe_execution(
        &self,
        _: &str,
        _: &str,
    ) -> anyhow::Result<Option<IssueWorkExecutionObservation>> {
        let registered = self.order.lock().unwrap().is_some();
        Ok(registered.then(|| IssueWorkExecutionObservation {
            phase: IssueExecutionPhase::Running,
            task_ids: vec![TASK.into()],
            active_task_id: Some(TASK.into()),
            execution_ids: vec![TASK.into()],
            question: self.waiting.load(Ordering::SeqCst).then(waiting_question),
            ..Default::default()
        }))
    }
    fn asking_worker(
        &self,
        _: &str,
        branch: &str,
    ) -> anyhow::Result<Option<(String, IssueAgentAsk)>> {
        let order = self.order.lock().unwrap();
        Ok(order
            .as_ref()
            .filter(|_| branch == BRANCH && !self.waiting.load(Ordering::SeqCst))
            .map(|order| (order.work_order_id.clone(), ask())))
    }
    fn await_question(&self, _: &str, _: &str, asked: &IssueAgentAsk) -> anyhow::Result<bool> {
        assert_eq!(asked, &ask());
        self.awaited.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }
    fn answer_question(
        &self,
        _: &str,
        request: &IssueWorkAnswerRequest,
    ) -> anyhow::Result<IssueWorkAnswerReceipt> {
        self.answers.lock().unwrap().push(request.clone());
        Ok(IssueWorkAnswerReceipt {
            command_id: request.command_id.clone(),
            question_id: request.question.question_id.clone(),
            delivery: IssueWorkAnswerDelivery::Queued,
        })
    }
}

fn until<T>(sim: &mut SimKernel, what: &str, mut found: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + WAIT;
    loop {
        sim.host
            .run_cycle(Instant::now(), Duration::from_millis(20));
        if let Some(value) = found() {
            return value;
        }
        assert!(Instant::now() < deadline, "host did not reach: {what}");
    }
}

fn send(
    sim: &mut SimKernel,
    workspace: &str,
    command: serde_json::Value,
    caller: &str,
    key: &str,
) -> StoredCommandOutcome {
    let request = CommandRequest {
        command: serde_json::from_value(command).unwrap(),
        idempotency_key: Some(key.into()),
    };
    let mut descriptor = request.descriptor().unwrap().unwrap();
    descriptor.caller = Some(caller.into());
    let KernelCommandInput::Stored(command) = request.command.into_input(workspace).unwrap() else {
        panic!("an issue question is a stored command");
    };
    let (reply, response) = mpsc::channel();
    sim.host.handle_input(KernelHostInput::Command {
        command: KernelClientCommand::Stored(Box::new(command)),
        descriptor: Some(descriptor),
        reply,
    });
    match response.try_recv().expect("stored commands reply at once") {
        KernelCommandOutcome::Handled {
            stored: Some(StoredCommandEnd::Finished(outcome)),
            ..
        } => outcome,
        other => panic!("expected a finished stored command: {other:?}"),
    }
}

fn ask_command(question: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "ask_issue_decision",
        "branch": BRANCH,
        "question": question,
        "options": [{"id": "a", "label": "A 유지"}, {"id": "b", "label": "B 로 옮김"}],
        "recommended": "a",
    })
}

fn requested(outcome: &StoredCommandOutcome) -> (&IssueDecisionFact, bool) {
    match outcome {
        StoredCommandOutcome::Done(StoredCommandResult::IssueDecisionRequested {
            fact,
            changed,
            ..
        }) => (fact, *changed),
        other => panic!("expected an opened question: {other:?}"),
    }
}

fn rejected(outcome: StoredCommandOutcome) -> StoredCommandRejection {
    match outcome {
        StoredCommandOutcome::Rejected(rejection) => rejection,
        other => panic!("expected a rejection: {other:?}"),
    }
}

fn human(
    sim: &mut SimKernel,
    services: &Services,
    decision_id: &str,
    option: &str,
    note: Option<&str>,
) {
    let preview = services
        .decisions
        .prepare_confirmation(decision_id, option)
        .unwrap();
    let answered = services
        .decisions
        .answer(
            decision_id,
            option,
            note,
            &StoredCommandContext {
                human_confirmation: Some(
                    VerifiedHumanConfirmation::from_verified(
                        preview.target,
                        format!("desktop-confirmation:{}", "a".repeat(32)),
                    )
                    .unwrap(),
                ),
                caller: Some("Desktop".into()),
                channel: "gateway".into(),
                at: "2026-11-25T00:00:00Z".into(),
            },
        )
        .unwrap();
    sim.host.runtime.record_issue_decision_for_test(
        &answered.commit.record.request.fact(),
        answered.commit.catalog_revision,
    );
}

#[test]
fn kernel_simulation_agent_question_opens_a_card_rejects_misuse_and_answers_the_thread_once() {
    let fixture = Fixture::new();
    let source = Source::new();
    let mut services = fixture.services_with_policy(source.clone(), POLICY);
    services.seed();
    let execution = Arc::new(Execution::default());
    services.decisions = Arc::new(
        IssueDecisionService::new(
            fixture.workspace.clone(),
            services.store.clone(),
            services.triage.clone(),
            IssuePolicyService::new(Arc::new(DisclosurePolicy)),
        )
        .with_execution(execution.clone()),
    );
    let coordinator = Arc::new(IssueCoordinatorService::new(
        fixture.workspace.clone(),
        services.store.clone(),
        source.clone(),
        services.triage.clone(),
        services.decisions.clone(),
        IssuePolicyService::new(Arc::new(DisclosurePolicy)),
        execution.clone(),
    ));
    services.issues = Arc::new(
        IssueCollectionService::new(
            fixture.workspace.clone(),
            services.store.clone(),
            source.clone(),
            Arc::new(Origin),
            IssuePolicyService::new(Arc::new(DisclosurePolicy)),
        )
        .with_triage(services.triage.clone())
        .with_coordinator(coordinator.clone()),
    );
    let state = services
        .store
        .get_collection(&fixture.workspace)
        .unwrap()
        .unwrap();
    services
        .store
        .set_collection_mode(
            &fixture.workspace,
            Some(state.epoch),
            IssueCollectionMode::Observe,
        )
        .unwrap();
    let mut sim = fixture.boot(&services, Arc::new(SimKernelLog::default()));
    let events = subscribe(&mut sim);

    // A person approves the start; the fake executor registers one running task.
    let start = until(&mut sim, "the start decision", || {
        services
            .issues
            .issues(&IssueCollectionQuery::default())
            .unwrap()
            .items[0]
            .open_decisions
            .first()
            .cloned()
    });
    let issue = start.request.issue().clone();
    human(
        &mut sim,
        &services,
        start.request.id().as_str(),
        "start",
        None,
    );
    until(&mut sim, "the registered work", || {
        services
            .store
            .get_issue_work(&fixture.workspace, &issue)
            .unwrap()
            .filter(|work| work.permit_source.is_some())
    });
    let track = |enabled: bool| {
        coordinator
            .track_execution(&IssueCollectionCancellation::new(), 1_000, enabled, &|_| {})
            .unwrap()
    };
    track(false);
    assert!(
        services
            .store
            .get_issue_execution(&fixture.workspace, &issue)
            .unwrap()
            .is_some(),
        "the running task is tracked"
    );

    // The worker asks through the kernel. The issue comes from its branch, not from the input.
    let opened = send(
        &mut sim,
        &fixture.workspace,
        ask_command("형식 A 를 유지합니까, B 로 옮깁니까?"),
        "akra-cli",
        "ask-1",
    );
    let (fact, changed) = requested(&opened);
    assert!(changed);
    assert_eq!(fact.kind, IssueDecisionKind::AgentQuestion);
    let decision_id = fact.decision_id.as_str().to_owned();
    assert_eq!(execution.awaited.load(Ordering::SeqCst), 1);
    let detail = query(&mut sim, &issue);
    let card = detail
        .item
        .as_ref()
        .unwrap()
        .open_decisions
        .iter()
        .find(|decision| decision.request.id().as_str() == decision_id)
        .expect("the question is a decision card")
        .request
        .definition()
        .clone();
    assert_eq!(card.options.len(), 2);
    assert_eq!(card.asked_by, "akra-cli");
    assert!(
        live_decisions(&events)
            .iter()
            .any(|event| event.to_string().contains(&decision_id)),
        "clients see the new card live"
    );
    let resent = send(
        &mut sim,
        &fixture.workspace,
        ask_command("형식 A 를 유지합니까, B 로 옮깁니까?"),
        "akra-cli",
        "ask-2",
    );
    assert_eq!(requested(&resent), (fact, false));

    // Another open question exceeds the per-issue limit; the rejection carries `next`.
    let limited = rejected(send(
        &mut sim,
        &fixture.workspace,
        ask_command("다른 질문입니까?"),
        "akra-cli",
        "ask-3",
    ));
    assert_eq!(limited.reason, ReasonCode::IssueQuestionLimit);
    let wire = serde_json::to_value(
        CommandResponse::from_core(Some(&[]))
            .with_stored(Some(StoredCommandOutcome::Rejected(limited))),
    )
    .unwrap();
    assert!(!wire["rejection"]["next"].as_array().unwrap().is_empty());

    // The asking connection cannot answer its own question.
    let own = rejected(send(
        &mut sim,
        &fixture.workspace,
        serde_json::json!({"type":"answer_issue_decision","decision_id":decision_id,"option":"b"}),
        "akra-cli",
        "answer-own",
    ));
    assert_eq!(own.reason, ReasonCode::OwnIssueQuestion);

    // A person answers; the turn ends and the same thread waits; the answer goes there once.
    human(
        &mut sim,
        &services,
        &decision_id,
        "b",
        Some("새 판은 B 입니다."),
    );
    execution.waiting.store(true, Ordering::SeqCst);
    for _ in 0..3 {
        track(true);
    }
    let answers = execution.answers.lock().unwrap();
    assert_eq!(answers.len(), 1, "the answer reaches the thread once");
    assert_eq!(answers[0].question, waiting_question());
    assert!(answers[0].message.contains("B 로 옮김 (`b`)"));
    assert!(answers[0].message.contains("새 판은 B 입니다."));
    assert!(
        query(&mut sim, &issue)
            .item
            .unwrap()
            .open_decisions
            .is_empty(),
        "the waiting turn opens no second awaiting_user card"
    );
    assert!(
        sim.effects.take().is_empty(),
        "the fake host never starts a model turn"
    );
}
