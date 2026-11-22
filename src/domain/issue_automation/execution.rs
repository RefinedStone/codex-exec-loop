//! 실행기의 관측을 이슈 수명과 서로 독립적인 시도 횟수로 옮기는 중립 모델이다 (#2551).
//! PR 검증 FSM은 읽기만 하며, 같은 영속 증거를 반복해서 세지 않는다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod detail;
pub use detail::{
    IssueExecutionDetail, IssueExecutionReadyDetail, IssueExecutionRecovery,
    IssueExecutionRecoveryReason, IssueExecutionUnavailableReason,
};

/// Execution facts, rather than provider-specific validation phases.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueExecutionPhase {
    #[default]
    Queued,
    Running,
    InReview,
    Integrated,
    Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueExecutionTerminalClass {
    ReworkNeeded,
    ObserveAgain,
    ContractRecovery,
    IdentityConflict,
    PullRequestClosed,
    QueueRecovery,
}

impl IssueExecutionTerminalClass {
    pub fn requires_human_recovery(self) -> bool {
        matches!(
            self,
            Self::ContractRecovery | Self::IdentityConflict | Self::PullRequestClosed
        )
    }

    pub fn needs_verification_attempt(self) -> bool {
        matches!(self, Self::ObserveAgain | Self::QueueRecovery)
    }

    pub fn recovery_summary(self) -> &'static str {
        match self {
            Self::ReworkNeeded => "기존 PR 검증이 연결한 보완 작업을 확인합니다.",
            Self::ObserveAgain => "검증 결과를 다시 조회합니다. 구현 작업을 새로 만들지 않습니다.",
            Self::ContractRecovery => "통합 증거 또는 필수 검증 설정을 복구해야 합니다.",
            Self::IdentityConflict => "검증 대상의 식별 정보를 확인할 때까지 작업을 보류합니다.",
            Self::PullRequestClosed => "PR을 다시 열거나 새 PR을 연결할지 결정해야 합니다.",
            Self::QueueRecovery => "기존 보완 작업의 큐를 복구한 뒤 검증을 다시 확인합니다.",
        }
    }
}

/// The executor owns this stable identity. A repeated record is the same fact, not a retry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueExecutionTerminalRecord {
    pub record_id: String,
    pub classification: IssueExecutionTerminalClass,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueExecutionIntegration {
    pub record_id: String,
    pub source_sha: String,
    pub integration_sha: String,
    pub pull_request_url: Option<String>,
}

/// The question has no raw worker output. The decision asks the human to inspect this exact thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueExecutionQuestion {
    pub question_id: String,
    pub task_id: String,
    pub session_key: String,
    pub thread_id: String,
    pub slot_id: String,
    pub worktree_path: String,
    pub branch_name: String,
    pub lease_generation: String,
    pub awaiting_task_updated_at: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueWorkExecutionObservation {
    pub phase: IssueExecutionPhase,
    pub task_ids: Vec<String>,
    pub active_task_id: Option<String>,
    /// One identity per started implementation task; human replies continue the same attempt.
    pub execution_ids: Vec<String>,
    pub terminals: Vec<IssueExecutionTerminalRecord>,
    pub current_terminal_id: Option<String>,
    /// 현재 작업 계보의 활성 검증이 확인한 정확한 PR URL이다. 번호로 추측하지 않는다.
    #[serde(default)]
    pub review_pull_request_url: Option<String>,
    pub integration: Option<IssueExecutionIntegration>,
    pub question: Option<IssueExecutionQuestion>,
}

impl IssueWorkExecutionObservation {
    pub fn validate(&self) -> Result<(), &'static str> {
        use std::collections::BTreeSet;
        if self.task_ids.is_empty()
            || self.task_ids.iter().any(|id| !valid_id(id))
            || self.task_ids.iter().collect::<BTreeSet<_>>().len() != self.task_ids.len()
            || self
                .active_task_id
                .as_ref()
                .is_some_and(|id| !self.task_ids.contains(id))
            || self
                .execution_ids
                .iter()
                .any(|id| !valid_id(id) || !self.task_ids.contains(id))
            || self.execution_ids.iter().collect::<BTreeSet<_>>().len() != self.execution_ids.len()
            || self
                .terminals
                .iter()
                .any(|record| !valid_id(&record.record_id))
            || self
                .terminals
                .iter()
                .map(|record| &record.record_id)
                .collect::<BTreeSet<_>>()
                .len()
                != self.terminals.len()
        {
            return Err("invalid issue execution observation");
        }
        if self
            .current_terminal_id
            .as_ref()
            .is_some_and(|id| !self.terminals.iter().any(|record| &record.record_id == id))
        {
            return Err("the current terminal record was not observed");
        }
        if let Some(integration) = &self.integration
            && (!valid_id(&integration.record_id)
                || !valid_sha(&integration.source_sha)
                || !valid_sha(&integration.integration_sha))
        {
            return Err("invalid issue integration evidence");
        }
        if self.phase == IssueExecutionPhase::Integrated && self.integration.is_none() {
            return Err("integrated execution requires trusted evidence");
        }
        if let Some(question) = &self.question
            && (!valid_id(&question.question_id)
                || !valid_id(&question.task_id)
                || !valid_id(&question.session_key)
                || !valid_id(&question.thread_id)
                || !valid_id(&question.slot_id)
                || !valid_id(&question.branch_name)
                || !valid_id(&question.lease_generation)
                || !valid_id(&question.awaiting_task_updated_at)
                || question.worktree_path.trim().is_empty()
                || question.worktree_path.len() > 32 * 1024
                || question.worktree_path.contains('\0')
                || !self.task_ids.contains(&question.task_id)
                || self.active_task_id.as_ref() != Some(&question.task_id)
                || self.integration.is_some())
        {
            return Err("invalid issue execution question");
        }
        Ok(())
    }
}

fn valid_id(id: &str) -> bool {
    !id.trim().is_empty() && id.len() <= 1024 && !id.contains('\0')
}

fn valid_sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueExecutionState {
    #[default]
    Queued,
    Running,
    InReview,
    Done,
    Failed,
}

/// These are separate counters. A validation or publication failure never consumes execution budget.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueExecutionAttempts {
    pub execution: u32,
    pub verification: u32,
    pub publication: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueExecutionProgress {
    pub state: IssueExecutionState,
    pub attempts: IssueExecutionAttempts,
    pub terminal: Option<IssueExecutionTerminalRecord>,
    #[serde(default)]
    pub review_pull_request_url: Option<String>,
    pub integration: Option<IssueExecutionIntegration>,
    pub attempts_exhausted: bool,
    pub revision: u64,
}

impl IssueExecutionProgress {
    pub fn requires_human_recovery(&self) -> bool {
        self.attempts_exhausted
            || self
                .terminal
                .as_ref()
                .is_some_and(|terminal| terminal.classification.requires_human_recovery())
    }

    /// The store supplies only newly inserted receipt counts from the same transaction.
    pub fn reconcile(
        &self,
        observation: &IssueWorkExecutionObservation,
        new_execution_receipts: u32,
        new_verification_receipts: u32,
        max_attempts: u32,
    ) -> Result<Self, &'static str> {
        self.reconcile_with_budget(
            observation,
            new_execution_receipts,
            new_verification_receipts,
            Some(max_attempts),
        )
    }

    /// 정책 조회 실패도 이미 관측한 사실을 버리지 않는다. None은 새 소진 판정을 하지 않는다.
    /// 실행 허가는 별도의 경계에서 차단하며 이 메서드는 허가를 만들지 않는다.
    pub fn reconcile_with_budget(
        &self,
        observation: &IssueWorkExecutionObservation,
        new_execution_receipts: u32,
        new_verification_receipts: u32,
        max_attempts: Option<u32>,
    ) -> Result<Self, &'static str> {
        observation.validate()?;
        let mut next = self.clone();
        next.review_pull_request_url = observation.review_pull_request_url.clone();
        next.attempts.execution = next
            .attempts
            .execution
            .checked_add(new_execution_receipts)
            .ok_or("execution attempt count overflow")?;
        next.attempts.verification = next
            .attempts
            .verification
            .checked_add(new_verification_receipts)
            .ok_or("verification attempt count overflow")?;
        if let Some(integration) = &observation.integration {
            if let Some(existing) = &self.integration
                && existing != integration
            {
                return Err("the integrated work evidence cannot change");
            }
            next.integration = Some(integration.clone());
        }
        next.terminal = observation.current_terminal_id.as_ref().and_then(|id| {
            observation
                .terminals
                .iter()
                .find(|record| &record.record_id == id)
                .cloned()
        });
        // Integration is an irreversible observed fact. Later CI failures do not reimplement it.
        if next.integration.is_some() {
            next.state = IssueExecutionState::Done;
            next.attempts_exhausted = false;
            return Ok(next);
        }
        let rework = next.terminal.as_ref().is_some_and(|terminal| {
            terminal.classification == IssueExecutionTerminalClass::ReworkNeeded
        });
        next.attempts_exhausted = self.attempts_exhausted
            || (rework && max_attempts.is_some_and(|limit| next.attempts.execution >= limit));
        next.state = if next.attempts_exhausted {
            IssueExecutionState::Failed
        } else {
            match observation.phase {
                IssueExecutionPhase::Queued
                    if next.attempts.execution > 0
                        || matches!(
                            self.state,
                            IssueExecutionState::Running | IssueExecutionState::InReview
                        ) =>
                {
                    // A queued supplement continues this work; it is not a new initial queue.
                    IssueExecutionState::Running
                }
                IssueExecutionPhase::Queued => IssueExecutionState::Queued,
                IssueExecutionPhase::Running => IssueExecutionState::Running,
                IssueExecutionPhase::InReview | IssueExecutionPhase::Terminal => {
                    IssueExecutionState::InReview
                }
                IssueExecutionPhase::Integrated => IssueExecutionState::Done,
            }
        };
        Ok(next)
    }

    pub fn allows_additional_execution(&self, max_attempts: u32) -> bool {
        self.integration.is_none()
            && !self.attempts_exhausted
            && self.attempts.execution < max_attempts
            && self.terminal.as_ref().is_none_or(|terminal| {
                terminal.classification == IssueExecutionTerminalClass::ReworkNeeded
            })
    }
}

/// 저장된 실행 관측이다. 질문의 좌표는 답변 시점까지 원래 값으로 고정한다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueExecutionRecord {
    pub work_order_id: String,
    pub progress: IssueExecutionProgress,
    pub question: Option<IssueExecutionQuestion>,
}

impl IssueExecutionRecord {
    pub fn new(work_order_id: String) -> Self {
        Self {
            work_order_id,
            progress: IssueExecutionProgress::default(),
            question: None,
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if !valid_id(&self.work_order_id)
            || self.progress.revision > super::MAX_ISSUE_AUTOMATION_REVISION
            || (self.progress.state == IssueExecutionState::Done
                && self.progress.integration.is_none())
            || (self.progress.attempts_exhausted
                && self.progress.state != IssueExecutionState::Failed)
        {
            return Err("invalid persisted issue execution progress");
        }
        if let Some(terminal) = &self.progress.terminal
            && !valid_id(&terminal.record_id)
        {
            return Err("invalid persisted terminal identity");
        }
        if let Some(integration) = &self.progress.integration
            && (!valid_id(&integration.record_id)
                || !valid_sha(&integration.source_sha)
                || !valid_sha(&integration.integration_sha)
                || self.progress.state != IssueExecutionState::Done)
        {
            return Err("invalid persisted integration evidence");
        }
        if let Some(question) = &self.question {
            IssueWorkExecutionObservation {
                phase: IssueExecutionPhase::Running,
                task_ids: vec![question.task_id.clone()],
                active_task_id: Some(question.task_id.clone()),
                question: Some(question.clone()),
                ..Default::default()
            }
            .validate()?;
            if self.progress.integration.is_some() {
                return Err("integrated work cannot retain a pending worker question");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
