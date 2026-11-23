//! 바인딩의 실행 세대와 일시 정지 영수증. 작업별 사람 개입 상태는 바꾸지 않는다.
use super::autonomy::validate_command_id;
use super::{IssueAutomationAutonomy, IssueAutomationBindingId, MAX_ISSUE_AUTOMATION_REVISION};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const ISSUE_BINDING_PAUSE_CONTROL_PREFIX: &str = "binding-pause-";
pub const ISSUE_BINDING_PAUSE_PAGE_LIMIT: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueBindingResumeBasis {
    /// 바인딩 메타데이터의 판이다. 실행 세대인 pause revision과 다르다.
    pub binding_revision: u64,
    pub autonomy_revision: u64,
    pub policy_version: String,
    pub effective_level: IssueAutomationAutonomy,
}

impl IssueBindingResumeBasis {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.binding_revision > MAX_ISSUE_AUTOMATION_REVISION
            || self.autonomy_revision > MAX_ISSUE_AUTOMATION_REVISION
            || self.policy_version.len() != 64
            || !self
                .policy_version
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("invalid issue binding resume basis");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueBindingPauseIntent {
    pub command_id: String,
    pub binding_id: IssueAutomationBindingId,
    pub paused: bool,
    pub expected_revision: u64,
    /// 정지는 정책을 읽을 수 없어도 가능하며, 재개만 현재 정책에 결속한다.
    pub resume_basis: Option<IssueBindingResumeBasis>,
}

impl IssueBindingPauseIntent {
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_command_id(&self.command_id)?;
        validate_binding_pause_id(&self.binding_id)?;
        if self.expected_revision >= MAX_ISSUE_AUTOMATION_REVISION {
            return Err("issue binding pause revision exhausted");
        }
        match (self.paused, &self.resume_basis) {
            (true, None) => Ok(()),
            (false, Some(basis)) => basis.validate(),
            _ => Err("issue binding pause intent has an invalid resume basis"),
        }
    }
}

pub fn validate_binding_pause_id(
    binding_id: &IssueAutomationBindingId,
) -> Result<(), &'static str> {
    let value = binding_id.as_str();
    if value.is_empty()
        || value.len() > 512
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err("invalid issue binding pause ID");
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueBindingPauseState {
    pub binding_id: IssueAutomationBindingId,
    /// 변경을 수락하는 transaction에서 증가하며 완료 때 다시 증가하지 않는다.
    pub revision: u64,
    pub paused: bool,
    pub pending: Option<IssueBindingPauseIntent>,
}

impl IssueBindingPauseState {
    pub fn allows_automation(&self) -> bool {
        !self.paused && self.pending.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueBindingPauseOutcome {
    Paused,
    Resumed,
    Unchanged,
    ResumeSuperseded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueBindingPauseReceipt {
    pub command_id: String,
    pub binding_id: IssueAutomationBindingId,
    pub revision: u64,
    pub paused: bool,
    pub outcome: IssueBindingPauseOutcome,
    /// Paused/Resumed만 true다. superseded도 예약한 새 실행 세대는 보존한다.
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredIssueBindingPauseCommand {
    pub intent: IssueBindingPauseIntent,
    pub accepted_revision: u64,
    /// None은 요청 수락이며, 정지 또는 재개 완료를 뜻하지 않는다.
    pub receipt: Option<IssueBindingPauseReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueBindingPauseEvent {
    pub event_id: String,
    pub receipt: IssueBindingPauseReceipt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueBindingPauseCompletion {
    Apply,
    ResumeSuperseded,
}
