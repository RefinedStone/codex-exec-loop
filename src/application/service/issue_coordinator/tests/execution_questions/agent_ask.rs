//! IA8-1: a running worker asks once, a person answers once, and the same thread gets it once.
use super::*;
use crate::application::port::inbound::stored_command_port::{
    AskIssueDecisionOption, AskIssueDecisionRequest, StoredCommandRejection,
};
use crate::domain::issue_automation::agent_question::IssueAgentAsk;
use crate::domain::kernel_spec::ReasonCode;

pub(super) const BRANCH: &str = "akra-agent/slot-1/ask";
const QUESTION: &str = "저장 형식은 A 와 B 중 무엇입니까?";

fn running(h: &Harness) {
    h.execution.state.lock().unwrap().observation = Some(IssueWorkExecutionObservation {
        phase: IssueExecutionPhase::Running,
        task_ids: vec!["task-1".into()],
        active_task_id: Some("task-1".into()),
        execution_ids: vec!["task-1".into()],
        ..Default::default()
    });
}

fn asking(h: &Harness, execution: &Arc<AnswerExecution>) -> IssueDecisionService {
    let waiting = question(h);
    *execution.ask.lock().unwrap() = Some((
        h.work().unwrap().order.work_order_id,
        IssueAgentAsk {
            task_id: waiting.task_id,
            thread_id: waiting.thread_id,
            lease_generation: waiting.lease_generation,
        },
    ));
    IssueDecisionService::new(
        h.fixture.workspace.clone(),
        h.fixture.store(),
        h.triage.clone(),
        IssuePolicyService::new(Arc::new(ValidationPolicy)),
    )
    .with_execution(execution.clone())
}

fn request(question: &str) -> AskIssueDecisionRequest {
    AskIssueDecisionRequest {
        branch: BRANCH.into(),
        question: question.into(),
        options: [("a", "A 형식"), ("b", "B 형식")]
            .into_iter()
            .map(|(id, label)| AskIssueDecisionOption {
                id: id.into(),
                label: label.into(),
            })
            .collect(),
        recommended: Some("a".into()),
    }
}

fn cli() -> StoredCommandContext {
    StoredCommandContext {
        human_confirmation: None,
        caller: Some("akra-cli".into()),
        channel: "kernel".into(),
        at: "2026-11-25T00:00:00Z".into(),
    }
}

fn rejection(error: anyhow::Error) -> ReasonCode {
    error
        .downcast_ref::<StoredCommandRejection>()
        .unwrap_or_else(|| panic!("expected a rejection: {error:#}"))
        .reason
}

#[test]
fn an_agent_question_opens_one_card_and_its_answer_reaches_the_same_thread_once() {
    let (h, execution) = started();
    running(&h);
    track(&h, &execution, true).unwrap();
    let decisions = asking(&h, &execution);

    let opened = decisions.ask(request(QUESTION), &cli()).unwrap();
    assert!(opened.changed);
    let resent = decisions.ask(request(QUESTION), &cli()).unwrap();
    assert!(!resent.changed, "the same question returns the same card");
    assert_eq!(resent.record.request.id(), opened.record.request.id());
    assert_eq!(execution.awaited.lock().unwrap().len(), 2);

    let card = opened_question(&h);
    assert_eq!(card.request.id(), opened.record.request.id());
    let definition = card.request.definition();
    assert_eq!(definition.question.as_str(), QUESTION);
    assert_eq!(definition.options.len(), 2);
    assert_eq!(definition.asked_by, "akra-cli");
    assert_eq!(definition.recommended.as_deref(), Some("a"));

    let limited = decisions
        .ask(request("다른 질문입니까?"), &cli())
        .unwrap_err();
    assert_eq!(rejection(limited), ReasonCode::IssueQuestionLimit);

    let own = h
        .decisions
        .answer(card.request.id().as_str(), "a", None, &cli())
        .unwrap_err();
    assert_eq!(rejection(own), ReasonCode::OwnIssueQuestion);
    assert_eq!(
        opened_question(&h),
        card,
        "a rejected answer changes nothing"
    );

    let answered = h
        .decisions
        .answer(
            card.request.id().as_str(),
            "b",
            Some("A 는 이전 판과 맞지 않습니다."),
            &context(&h, &card, "b"),
        )
        .unwrap();
    assert_eq!(answered.outcome, IssueDecisionOutcome::Answered);
    track(&h, &execution, true).unwrap();
    assert!(
        execution.calls.lock().unwrap().is_empty(),
        "the answer waits until the asking turn has ended"
    );

    observe_question(&h);
    track(&h, &execution, true).unwrap();
    track(&h, &execution, true).unwrap();
    let calls = execution.calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "the answer reaches the thread once");
    assert_eq!(calls[0].question, question(&h));
    assert_eq!(
        calls[0].message,
        format!("질문: {QUESTION}\n사람의 선택: B 형식 (`b`)\n설명: A 는 이전 판과 맞지 않습니다.")
    );
    assert!(
        page(&h.triage).items[0].open_decisions.is_empty(),
        "the waiting turn opens no second awaiting_user card"
    );
}

#[test]
fn a_question_without_a_running_issue_conversation_opens_nothing() {
    let (h, execution) = started();
    running(&h);
    track(&h, &execution, true).unwrap();
    let decisions = asking(&h, &execution);
    *execution.ask.lock().unwrap() = None;
    assert!(decisions.ask(request(QUESTION), &cli()).is_err());
    let mut anonymous = cli();
    anonymous.caller = None;
    assert!(
        asking(&h, &execution)
            .ask(request(QUESTION), &anonymous)
            .is_err()
    );
    assert!(execution.awaited.lock().unwrap().is_empty());
    assert!(page(&h.triage).items[0].open_decisions.is_empty());
}
