//! 실행 중인 에이전트가 직접 연 질문의 좌표와 답 전달 글이다 (IA8-1, #2566, plan 28 §10.2).
//! awaiting_user 질문(IA4-1)과 같은 결정 종류와 전달 경로를 쓴다. 차이는 좌표를 질문할 때 고정하는 것이다.
use super::coordinator::IssueWorkCheckpoint;
use super::decision::{IssueDecisionAnswer, IssueDecisionDefinition, IssueDecisionKind};
use super::execution::{IssueExecutionQuestion, IssueExecutionRecord, IssueExecutionState};

/// 이슈 하나에 동시에 열 수 있는 에이전트 질문의 수이다. 질문 뒤에 턴이 끝나므로 하나면 된다.
pub const MAX_OPEN_AGENT_QUESTIONS_PER_ISSUE: usize = 1;
/// 질문 글의 상한이다. 선택지와 설명을 더한 답 글이 결정 글의 상한(8 KiB) 안에 들어간다.
pub const MAX_AGENT_QUESTION_BYTES: usize = 2048;
/// 에이전트 질문의 선택지마다 붙는 고정 효과 글이다. 에이전트가 효과를 정하지 못한다.
pub const AGENT_QUESTION_OPTION_EFFECT: &str = "같은 작업 대화에 이 선택을 한 번 전달합니다.";
/// IA4-1의 awaiting_user 질문이 받는 유일한 선택지이다.
pub const WORKER_REPLY_OPTION: &str = "reply";

const TASK: &str = "ask-task:";
const THREAD: &str = "ask-thread:";
const LEASE: &str = "ask-lease:";

/// 질문을 연 실행의 좌표이다. 답은 이 task·thread·lease generation 의 대기에만 간다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueAgentAsk {
    pub task_id: String,
    pub thread_id: String,
    pub lease_generation: String,
}

impl IssueAgentAsk {
    pub fn evidence(&self) -> Vec<String> {
        vec![
            format!("{TASK}{}", self.task_id),
            format!("{THREAD}{}", self.thread_id),
            format!("{LEASE}{}", self.lease_generation),
        ]
    }

    /// 에이전트가 연 질문이면 그 좌표를 돌려준다. awaiting_user 질문은 None 이다.
    pub fn from_definition(definition: &IssueDecisionDefinition) -> Option<Self> {
        if definition.kind != IssueDecisionKind::AgentQuestion {
            return None;
        }
        let [task, thread, lease] = definition.evidence.as_slice() else {
            return None;
        };
        Some(Self {
            task_id: task.strip_prefix(TASK)?.to_owned(),
            thread_id: thread.strip_prefix(THREAD)?.to_owned(),
            lease_generation: lease.strip_prefix(LEASE)?.to_owned(),
        })
    }

    pub fn matches(&self, question: &IssueExecutionQuestion) -> bool {
        self.task_id == question.task_id
            && self.thread_id == question.thread_id
            && self.lease_generation == question.lease_generation
    }
}

/// 이 결정이 지금의 대기 질문에 답을 보낼 수 있는가이다.
pub fn agent_question_targets(
    definition: &IssueDecisionDefinition,
    question: &IssueExecutionQuestion,
) -> bool {
    match IssueAgentAsk::from_definition(definition) {
        Some(ask) => ask.matches(question),
        None => definition.evidence == [format!("question:{}", question.question_id)],
    }
}

/// 질문 결정이 아직 지금의 작업과 실행에 속하는가이다. 서비스와 장부가 같은 규칙을 쓴다.
/// 에이전트 질문은 턴이 끝나기 전(대기 질문 없음)에도 같은 작업의 실행 중이면 현재이다.
pub fn agent_question_context_matches(
    definition: &IssueDecisionDefinition,
    work: &IssueWorkCheckpoint,
    record: &IssueExecutionRecord,
) -> bool {
    if work.current_authorization_basis().authorization_epoch
        != definition.basis.authorization_epoch
        || definition.basis.field_hash.as_ref() != Some(&work.order.field_hash)
    {
        return false;
    }
    match (&record.question, IssueAgentAsk::from_definition(definition)) {
        (Some(question), _) => agent_question_targets(definition, question),
        (None, Some(ask)) => {
            record.progress.state == IssueExecutionState::Running
                && work.task_ids.contains(&ask.task_id)
        }
        (None, None) => false,
    }
}

/// 사람의 답을 그 thread 에 보낼 글이다. 공유본만으로 만든다. 보낼 수 없는 답은 None 이다.
pub fn agent_question_reply(
    definition: &IssueDecisionDefinition,
    answer: &IssueDecisionAnswer,
) -> Option<String> {
    let note = answer.note.as_ref().map(|note| note.as_str());
    if IssueAgentAsk::from_definition(definition).is_none() {
        return note
            .filter(|_| answer.option == WORKER_REPLY_OPTION)
            .map(str::to_owned);
    }
    let option = definition
        .options
        .iter()
        .find(|option| option.id == answer.option)?;
    let mut message = format!(
        "질문: {}\n사람의 선택: {} (`{}`)",
        definition.question.as_str(),
        option.label.as_str(),
        option.id
    );
    if let Some(note) = note {
        message.push_str("\n설명: ");
        message.push_str(note);
    }
    Some(message)
}

#[cfg(test)]
mod tests;
