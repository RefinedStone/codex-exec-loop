//! 파일 정책 상한 안에서 선택한 실행 수준과 서버가 계산한 선택 가능 여부다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::issue_automation::IssueAutomationAutonomy;
pub use crate::domain::issue_automation::autonomy::IssueAutonomyLevel;
use crate::domain::issue_automation::autonomy::{
    IssueAutonomyChoice, IssueAutonomyProjection, validate_command_id,
};

use super::KernelProtocolError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum IssueAutonomyState {
    Unavailable {},
    PolicyBlocked {},
    Ready(Box<IssueAutonomyReadyState>),
}

impl Default for IssueAutonomyState {
    fn default() -> Self {
        Self::Unavailable {}
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueAutonomyReadyState {
    pub requested_level: Option<IssueAutonomyLevel>,
    pub effective_level: IssueAutomationAutonomy,
    pub cap: IssueAutomationAutonomy,
    pub revision: u64,
    pub policy_version: String,
    pub pending: bool,
    pub choices: Vec<IssueAutonomyChoiceState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueAutonomyChoiceState {
    pub level: IssueAutonomyLevel,
    pub enabled: bool,
    pub requires_confirmation: bool,
    pub reason: Option<String>,
}

impl IssueAutonomyState {
    pub fn from_domain(projection: &IssueAutonomyProjection) -> Self {
        match projection {
            IssueAutonomyProjection::Unavailable => Self::Unavailable {},
            IssueAutonomyProjection::PolicyBlocked => Self::PolicyBlocked {},
            IssueAutonomyProjection::Ready(state) => {
                Self::Ready(Box::new(IssueAutonomyReadyState {
                    requested_level: state.requested_level,
                    effective_level: state.effective_level,
                    cap: state.cap,
                    revision: state.revision,
                    policy_version: state.policy_version.clone(),
                    pending: state.pending,
                    choices: state.choices.iter().map(Into::into).collect(),
                }))
            }
        }
    }
}

impl From<&IssueAutonomyChoice> for IssueAutonomyChoiceState {
    fn from(choice: &IssueAutonomyChoice) -> Self {
        Self {
            level: choice.level,
            enabled: choice.enabled,
            requires_confirmation: choice.requires_confirmation,
            reason: choice.reason.clone(),
        }
    }
}

pub(super) fn validate_input(command_id: &str) -> Result<(), KernelProtocolError> {
    validate_command_id(command_id)
        .map_err(|_| KernelProtocolError::invalid_params("invalid issue autonomy command"))
}

#[cfg(test)]
mod tests;
