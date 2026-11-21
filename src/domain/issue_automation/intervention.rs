//! 사람의 개입은 작업별로 기록한다. 같은 command_id는 한 번만 적용한다.
use crate::domain::issue_work::{IssueReference, IssueSharedCopy};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum IssueInterventionAction {
    Hold,
    Release,
    Steer { message: String },
    TakeOver,
    Retriage,
    Cancel { confirm: bool },
    Track,
    Untrack,
}

impl IssueInterventionAction {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Hold => "hold_issue",
            Self::Release => "release_issue",
            Self::Steer { .. } => "steer_issue",
            Self::TakeOver => "take_over_issue",
            Self::Retriage => "retriage_issue",
            Self::Cancel { .. } => "cancel_issue_work",
            Self::Track => "track_issue",
            Self::Untrack => "untrack_issue",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueInterventionRequest {
    pub command_id: String,
    pub issue: IssueReference,
    pub action: IssueInterventionAction,
}

impl IssueInterventionRequest {
    pub fn validate(&self) -> Result<()> {
        if self.command_id.trim().is_empty()
            || self.command_id.len() > 256
            || self.command_id.chars().any(char::is_control)
        {
            bail!("an intervention requires a bounded nonempty command_id");
        }
        if let IssueInterventionAction::Cancel { confirm: false } = self.action {
            bail!("cancel_issue_work requires confirm");
        }
        if let IssueInterventionAction::Steer { message } = &self.action
            && (message.trim().is_empty() || message.len() > 64 * 1024)
        {
            bail!("steer_issue requires a nonempty message of at most 64 KiB");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSteerDelivery {
    Queued,
    Sent,
    /// 처리를 시작한 뒤 최종 전달 결과를 확인하지 못했다. 자동 재전송하지 않는다.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueInterventionReceipt {
    pub issue: IssueReference,
    pub command_id: String,
    pub action: String,
    pub work_order_id: Option<String>,
    pub authorization_epoch: u64,
    pub changed: bool,
    pub delivery: Option<IssueSteerDelivery>,
    pub handoff: Option<IssueSharedCopy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredIssueIntervention {
    pub request: IssueInterventionRequest,
    pub receipt: Option<IssueInterventionReceipt>,
}
