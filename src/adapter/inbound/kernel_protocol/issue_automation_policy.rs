//! 자동화 정책의 효과값과 출처를 표시한다. 파일 원문이나 I/O 오류를 wire에 싣지 않는다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::application::port::inbound::issue_automation_policy_query_port::{
    IssueAutomationPolicyFile, IssueAutomationPolicyFileScope, IssueAutomationPolicyFileStatus,
    IssueAutomationPolicyInspection, IssueAutomationPolicyInspectionBlocked,
    IssueAutomationPolicyOriginScope, IssueAutomationPolicyResolution, IssueAutomationPolicyScope,
    IssueAutomationPolicySetting,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum IssueAutomationPolicyState {
    Unavailable {},
    Ready(Box<IssueAutomationPolicyReadyState>),
    PolicyBlocked(Box<IssueAutomationPolicyBlockedState>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueAutomationPolicyReadyState {
    /// 조회가 실제로 사용한 대표 workspace다. 클라이언트가 요청한 하위 폴더와 다를 수 있다.
    pub workspace: String,
    pub version: String,
    pub settings: Vec<IssueAutomationPolicySettingState>,
    pub files: Vec<IssueAutomationPolicyFileState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueAutomationPolicyBlockedState {
    pub workspace: String,
    pub blocked: IssueAutomationPolicyProblemState,
    pub files: Vec<IssueAutomationPolicyFileState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueAutomationPolicySettingState {
    pub key: String,
    pub label: String,
    pub value: String,
    pub origins: Vec<IssueAutomationPolicyOriginState>,
    pub resolution: IssueAutomationPolicyResolutionState,
    pub resolution_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueAutomationPolicyOriginState {
    pub scope: IssueAutomationPolicyOriginScopeState,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueAutomationPolicyOriginScopeState {
    Builtin,
    User,
    Project,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueAutomationPolicyResolutionState {
    Default,
    Override,
    Minimum,
    Maximum,
    And,
    Intersection,
    Union,
    Cap,
    Derived,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueAutomationPolicyFileState {
    pub scope: IssueAutomationPolicyFileScopeState,
    pub label: String,
    pub path: Option<String>,
    pub status: IssueAutomationPolicyFileStatusState,
    pub open_target: Option<IssueAutomationPolicyFileScopeState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueAutomationPolicyFileScopeState {
    User,
    Project,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueAutomationPolicyFileStatusState {
    Missing,
    Readable,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueAutomationPolicyProblemState {
    pub scope: IssueAutomationPolicyProblemScopeState,
    pub field: String,
    pub reason: String,
    pub summary: String,
    pub next_summary: String,
    pub file_scopes: Vec<IssueAutomationPolicyFileScopeState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueAutomationPolicyProblemScopeState {
    User,
    Project,
    Effective,
}

impl IssueAutomationPolicyState {
    pub fn unavailable() -> Self {
        Self::Unavailable {}
    }

    pub fn from_domain(value: &IssueAutomationPolicyInspection) -> Self {
        match value {
            IssueAutomationPolicyInspection::Ready {
                workspace,
                version,
                settings,
                files,
            } => Self::Ready(Box::new(IssueAutomationPolicyReadyState {
                workspace: workspace.clone(),
                version: version.clone(),
                settings: settings.iter().map(Into::into).collect(),
                files: files.iter().map(Into::into).collect(),
            })),
            IssueAutomationPolicyInspection::PolicyBlocked {
                workspace,
                blocked,
                files,
            } => Self::PolicyBlocked(Box::new(IssueAutomationPolicyBlockedState {
                workspace: workspace.clone(),
                blocked: blocked.into(),
                files: files.iter().map(Into::into).collect(),
            })),
        }
    }
}

impl From<&IssueAutomationPolicySetting> for IssueAutomationPolicySettingState {
    fn from(value: &IssueAutomationPolicySetting) -> Self {
        Self {
            key: value.key.clone(),
            label: value.label.clone(),
            value: value.value.clone(),
            origins: value
                .origins
                .iter()
                .map(|origin| IssueAutomationPolicyOriginState {
                    scope: match origin.scope {
                        IssueAutomationPolicyOriginScope::Builtin => {
                            IssueAutomationPolicyOriginScopeState::Builtin
                        }
                        IssueAutomationPolicyOriginScope::User => {
                            IssueAutomationPolicyOriginScopeState::User
                        }
                        IssueAutomationPolicyOriginScope::Project => {
                            IssueAutomationPolicyOriginScopeState::Project
                        }
                    },
                    label: origin.label.clone(),
                })
                .collect(),
            resolution: match value.resolution {
                IssueAutomationPolicyResolution::Default => {
                    IssueAutomationPolicyResolutionState::Default
                }
                IssueAutomationPolicyResolution::Override => {
                    IssueAutomationPolicyResolutionState::Override
                }
                IssueAutomationPolicyResolution::Minimum => {
                    IssueAutomationPolicyResolutionState::Minimum
                }
                IssueAutomationPolicyResolution::Maximum => {
                    IssueAutomationPolicyResolutionState::Maximum
                }
                IssueAutomationPolicyResolution::And => IssueAutomationPolicyResolutionState::And,
                IssueAutomationPolicyResolution::Intersection => {
                    IssueAutomationPolicyResolutionState::Intersection
                }
                IssueAutomationPolicyResolution::Union => {
                    IssueAutomationPolicyResolutionState::Union
                }
                IssueAutomationPolicyResolution::Cap => IssueAutomationPolicyResolutionState::Cap,
                IssueAutomationPolicyResolution::Derived => {
                    IssueAutomationPolicyResolutionState::Derived
                }
            },
            resolution_summary: value.resolution_summary.clone(),
        }
    }
}

impl From<&IssueAutomationPolicyFile> for IssueAutomationPolicyFileState {
    fn from(value: &IssueAutomationPolicyFile) -> Self {
        Self {
            scope: file_scope(&value.scope),
            label: value.label.clone(),
            path: value.path.clone(),
            status: match value.status {
                IssueAutomationPolicyFileStatus::Missing => {
                    IssueAutomationPolicyFileStatusState::Missing
                }
                IssueAutomationPolicyFileStatus::Readable => {
                    IssueAutomationPolicyFileStatusState::Readable
                }
                IssueAutomationPolicyFileStatus::Unavailable => {
                    IssueAutomationPolicyFileStatusState::Unavailable
                }
            },
            open_target: value.open_target.as_ref().map(file_scope),
        }
    }
}

impl From<&IssueAutomationPolicyInspectionBlocked> for IssueAutomationPolicyProblemState {
    fn from(value: &IssueAutomationPolicyInspectionBlocked) -> Self {
        Self {
            scope: match value.scope {
                IssueAutomationPolicyScope::User => IssueAutomationPolicyProblemScopeState::User,
                IssueAutomationPolicyScope::Project => {
                    IssueAutomationPolicyProblemScopeState::Project
                }
                IssueAutomationPolicyScope::Effective => {
                    IssueAutomationPolicyProblemScopeState::Effective
                }
            },
            field: value.field.to_string(),
            reason: value.reason.to_string(),
            summary: value.summary.clone(),
            next_summary: value.next_summary.clone(),
            file_scopes: value.file_scopes.iter().map(file_scope).collect(),
        }
    }
}

fn file_scope(value: &IssueAutomationPolicyFileScope) -> IssueAutomationPolicyFileScopeState {
    match value {
        IssueAutomationPolicyFileScope::User => IssueAutomationPolicyFileScopeState::User,
        IssueAutomationPolicyFileScope::Project => IssueAutomationPolicyFileScopeState::Project,
    }
}

#[cfg(test)]
mod tests;
