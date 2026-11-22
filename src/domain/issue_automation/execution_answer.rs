//! 사람의 한 답변을 원래 worker 대화에 한 번만 전달하는 영속 의도다.
use super::decision::IssueDecisionId;
use super::execution::{
    IssueExecutionPhase, IssueExecutionQuestion, IssueWorkExecutionObservation,
};
use crate::domain::issue_work::IssueReference;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueExecutionAnswerIntent {
    pub command_id: String,
    pub issue: IssueReference,
    pub work_order_id: String,
    pub decision_id: IssueDecisionId,
    pub question: IssueExecutionQuestion,
    pub authorization_epoch: u64,
    /// 공개 정책을 통과한 결정 답변의 지문이다. 자유 본문은 중복 저장하지 않는다.
    pub answer_sha256: String,
}

impl IssueExecutionAnswerIntent {
    pub fn validate(&self) -> Result<(), &'static str> {
        super::validate_reference(&self.issue).map_err(|_| "invalid answer issue")?;
        if self.work_order_id != super::coordinator::issue_work_order_id(&self.issue)
            || self.command_id != answer_command_id(&self.work_order_id, &self.decision_id)
            || self.authorization_epoch > super::MAX_ISSUE_AUTOMATION_REVISION
            || self.answer_sha256.len() != 64
            || !self
                .answer_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("invalid issue execution answer intent");
        }
        IssueWorkExecutionObservation {
            phase: IssueExecutionPhase::Running,
            task_ids: vec![self.question.task_id.clone()],
            active_task_id: Some(self.question.task_id.clone()),
            question: Some(self.question.clone()),
            ..Default::default()
        }
        .validate()
    }
}

pub fn answer_command_id(work_order_id: &str, decision_id: &IssueDecisionId) -> String {
    let bytes = serde_json::to_vec(&(work_order_id, decision_id))
        .expect("typed answer identity serializes");
    format!("issue-answer-{:x}", Sha256::digest(bytes))
}

pub fn answer_hash(message: &str) -> String {
    format!("{:x}", Sha256::digest(message.as_bytes()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum IssueExecutionAnswerStatus {
    /// 이 상태를 저장한 최초 호출만 실행 포트에 요청할 수 있다.
    Unknown,
    /// planning이 의도를 받았으나 turn/start ACK는 아직 없다.
    Queued,
    Accepted {
        thread_id: String,
        turn_id: String,
        lease_generation: String,
    },
    RejectedBeforeStart {
        reason: String,
    },
}

impl IssueExecutionAnswerStatus {
    pub fn pending(&self) -> bool {
        matches!(self, Self::Unknown | Self::Queued)
    }

    pub fn validate_for(&self, intent: &IssueExecutionAnswerIntent) -> Result<(), &'static str> {
        let identifier = |value: &str| {
            !value.trim().is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control)
        };
        match self {
            Self::Accepted {
                thread_id,
                turn_id,
                lease_generation,
            } => {
                if thread_id != &intent.question.thread_id
                    || !identifier(turn_id)
                    || !identifier(lease_generation)
                    || lease_generation == &intent.question.lease_generation
                {
                    return Err(
                        "the answer ACK does not match its original thread and new generation",
                    );
                }
            }
            Self::RejectedBeforeStart { reason } => {
                if reason.is_empty()
                    || reason.len() > 64
                    || !reason.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                    })
                {
                    return Err("the answer rejection must be a bounded public reason code");
                }
            }
            Self::Unknown | Self::Queued => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueExecutionAnswerRecord {
    pub intent: IssueExecutionAnswerIntent,
    pub status: IssueExecutionAnswerStatus,
    pub revision: u64,
}

impl IssueExecutionAnswerRecord {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.intent.validate()?;
        self.status.validate_for(&self.intent)?;
        if self.revision == 0 || self.revision > super::MAX_ISSUE_AUTOMATION_REVISION {
            return Err("invalid issue execution answer revision");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueExecutionAnswerClaim {
    pub record: IssueExecutionAnswerRecord,
    pub created: bool,
}
