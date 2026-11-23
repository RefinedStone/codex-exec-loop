//! 파일 정책과 독립적으로 영속하는 사람의 실행 수준 선택이다.
use super::{IssueAutomationAutonomy, MAX_ISSUE_AUTOMATION_REVISION};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 외부 개입 명령은 이 prefix를 사용할 수 없다.
pub const ISSUE_AUTONOMY_CONTROL_PREFIX: &str = "autonomy-";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueAutonomyLevel {
    Off,
    Observe,
    Propose,
    Gated,
    Auto,
}
impl From<IssueAutonomyLevel> for IssueAutomationAutonomy {
    fn from(value: IssueAutonomyLevel) -> Self {
        match value {
            IssueAutonomyLevel::Off => Self::Off,
            IssueAutonomyLevel::Observe => Self::Observe,
            IssueAutonomyLevel::Propose => Self::Propose,
            IssueAutonomyLevel::Gated => Self::Gated,
            IssueAutonomyLevel::Auto => Self::Auto,
        }
    }
}
impl IssueAutonomyLevel {
    pub const ALL: [Self; 5] = [
        Self::Off,
        Self::Observe,
        Self::Propose,
        Self::Gated,
        Self::Auto,
    ];
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueAutonomyIntent {
    pub command_id: String,
    pub level: IssueAutonomyLevel,
    pub expected_revision: u64,
    pub policy_version: String,
}
impl IssueAutonomyIntent {
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_command_id(&self.command_id)?;
        if self.expected_revision >= MAX_ISSUE_AUTOMATION_REVISION
            || self.policy_version.len() != 64
            || !self.policy_version.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("invalid issue autonomy intent");
        }
        Ok(())
    }
}
pub fn validate_command_id(value: &str) -> Result<(), &'static str> {
    if value.trim().is_empty()
        || value.len() > 256
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err("invalid issue autonomy command ID");
    }
    Ok(())
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IssueAutonomySelection {
    pub requested_level: Option<IssueAutonomyLevel>,
    pub revision: u64,
    pub pending: Option<IssueAutonomyIntent>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredIssueAutonomyCommand {
    pub intent: IssueAutonomyIntent,
    pub completed_revision: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueAutonomyCommit {
    pub command: StoredIssueAutonomyCommand,
    pub changed: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueAutonomyProjection {
    Ready(IssueAutonomyReady),
    PolicyBlocked,
    Unavailable,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueAutonomyReady {
    pub requested_level: Option<IssueAutonomyLevel>,
    pub effective_level: IssueAutomationAutonomy,
    pub cap: IssueAutomationAutonomy,
    pub revision: u64,
    /// Runtime hash. File policy inspection keeps its original canonical hash.
    pub policy_version: String,
    pub pending: bool,
    pub choices: Vec<IssueAutonomyChoice>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueAutonomyChoice {
    pub level: IssueAutonomyLevel,
    pub enabled: bool,
    pub requires_confirmation: bool,
    pub reason: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueAutonomyChange {
    pub command_id: String,
    pub changed: bool,
    pub autonomy: IssueAutonomyProjection,
}
