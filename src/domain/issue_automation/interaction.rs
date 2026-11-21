//! 상세 화면에 필요한 읽기 전용 개입/결정/대화 투영이다.
use super::decision::{IssueDecisionFact, StoredIssueDecision};
use crate::domain::issue_work::IssueSharedCopy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueActionCommand {
    HoldIssue,
    ReleaseIssue,
    SteerIssue,
    TakeOverIssue,
    RetriageIssue,
    CancelIssueWork,
    TrackIssue,
    UntrackIssue,
}
impl IssueActionCommand {
    pub const ALL: [Self; 8] = [
        Self::HoldIssue,
        Self::ReleaseIssue,
        Self::SteerIssue,
        Self::TakeOverIssue,
        Self::RetriageIssue,
        Self::CancelIssueWork,
        Self::TrackIssue,
        Self::UntrackIssue,
    ];
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueControlProjection {
    pub tracked: bool,
    pub held: bool,
    pub taken_over: bool,
    pub cancelled: bool,
    pub authorization_epoch: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueActionProjection {
    pub command: IssueActionCommand,
    pub enabled: bool,
    pub reason: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueDecisionResolutionCode {
    Answered,
    Expired,
    Withdrawn,
    BasisChanged,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueDecisionHistory {
    pub decision: IssueDecisionFact,
    /// Historical text is absent unless today's disclosure and basis still permit it.
    pub request: Option<StoredIssueDecision>,
    pub resolution: IssueDecisionResolutionCode,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueConversationTarget {
    Thread { thread_id: String },
    Draft { shared_copy: IssueSharedCopy },
    Unavailable,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueInteractionProjection {
    pub control: Option<IssueControlProjection>,
    pub actions: Vec<IssueActionProjection>,
    pub decision_history: Vec<IssueDecisionHistory>,
    pub conversation: IssueConversationTarget,
}
