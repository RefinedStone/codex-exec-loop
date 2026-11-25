use crate::domain::issue_automation::feedback::{
    IssueFeedbackConnection, IssueFeedbackIntent, IssueFeedbackJiraTransition,
    IssueFeedbackLinearState, IssueFeedbackReceipt,
};
use crate::domain::issue_automation::policy::{
    IssueAutomationJiraCloseTransition, IssueAutomationLinearCloseState,
};
use crate::domain::issue_automation::source::IssueSourceLabel;
use crate::domain::issue_work::IssueReference;
use anyhow::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueFeedbackLabelEvent {
    pub event_id: String,
    pub actor_id: Option<String>,
    pub applied: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueFeedbackLabelSnapshot {
    pub issue: IssueReference,
    /// 저장소에 라벨이 없으면 None이다. 이 port는 라벨을 만들지 않는다.
    pub label: Option<IssueSourceLabel>,
    pub present: bool,
    pub latest_event: Option<IssueFeedbackLabelEvent>,
    pub complete: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueFeedbackClosureEvent {
    pub event_id: String,
    pub actor_id: Option<String>,
    pub closed: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueFeedbackClosureSnapshot {
    pub issue: IssueReference,
    pub closed: bool,
    pub latest_event: Option<IssueFeedbackClosureEvent>,
    pub complete: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueFeedbackWriteResult {
    Acknowledged {
        comment_id: Option<String>,
    },
    /// 오류 원문/비밀값은 전달하지 않는다. 실제 쓰기는 수행됐을 수도 있다.
    Unconfirmed,
    Contradictory,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueFeedbackReadResult {
    Confirmed(Box<IssueFeedbackReceipt>),
    NeedsConfirmation,
    Contradictory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueFeedbackActorContext {
    pub actor_id: String,
    pub connection: Option<IssueFeedbackConnection>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueFeedbackJiraTransitionSnapshot {
    pub issue: IssueReference,
    pub target: IssueFeedbackJiraTransition,
    pub previous_event_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueFeedbackLinearStateSnapshot {
    pub issue: IssueReference,
    pub target: IssueFeedbackLinearState,
    pub previous_event_id: Option<String>,
}

/// 원격 읽기/단일 쓰기 경계다. 재전송·정책·일지 전이는 service가 소유한다.
pub trait IssueFeedbackPort: Send + Sync {
    fn actor_id(&self, issue: &IssueReference) -> Result<String>;
    fn actor_context(&self, issue: &IssueReference) -> Result<IssueFeedbackActorContext> {
        Ok(IssueFeedbackActorContext {
            actor_id: self.actor_id(issue)?,
            connection: None,
        })
    }
    fn read_jira_close_transition(
        &self,
        _issue: &IssueReference,
        _actor: &IssueFeedbackActorContext,
        _mapping: &IssueAutomationJiraCloseTransition,
    ) -> Result<IssueFeedbackJiraTransitionSnapshot> {
        anyhow::bail!("Jira status transition is unsupported")
    }
    fn read_linear_close_state(
        &self,
        _issue: &IssueReference,
        _actor: &IssueFeedbackActorContext,
        _mapping: &IssueAutomationLinearCloseState,
    ) -> Result<IssueFeedbackLinearStateSnapshot> {
        anyhow::bail!("Linear state change is unsupported")
    }
    fn read_label(&self, issue: &IssueReference, name: &str) -> Result<IssueFeedbackLabelSnapshot>;
    fn read_closure(&self, _issue: &IssueReference) -> Result<IssueFeedbackClosureSnapshot> {
        anyhow::bail!("issue close observation is unsupported")
    }
    /// Sent CAS 성공자만 한 번 호출한다. 이 메서드 안에서는 mutation을 재시도하지 않는다.
    fn write(&self, intent: &IssueFeedbackIntent) -> Result<IssueFeedbackWriteResult>;
    /// 항상 읽기만 한다. create는 모든 댓글 페이지의 유일한 exact owner/intent/hash를 확인한다.
    /// update는 고정 comment ID, author/editor 및 원문 hash를 확인한다.
    /// label은 preflight event 뒤 최신 효과의 ID/actor/label을 확인하며 no-op을 자기 쓰기로 소유하지 않는다.
    fn reconcile(&self, intent: &IssueFeedbackIntent) -> Result<IssueFeedbackReadResult>;
}
