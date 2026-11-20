//! Desktop 확인을 검증한 뒤 커널에 넘기는 계약이다. raw proof/token은 이 타입에 없다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::issue_automation::MAX_ISSUE_AUTOMATION_REVISION;
use crate::domain::issue_automation::collection::IssueCollectionMode;
use crate::domain::issue_automation::decision::{
    IssueDecisionBasis, IssueDecisionId, MAX_ISSUE_DECISION_OPTION_BYTES,
    validate_issue_decision_confirmation_reference,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IssueConfirmationTarget {
    Decision {
        workspace: String,
        decision_id: IssueDecisionId,
        option: String,
        basis: IssueDecisionBasis,
        decision_revision: u64,
    },
    Mode {
        workspace: String,
        from: Option<IssueCollectionMode>,
        epoch: Option<u64>,
        revision: Option<u64>,
        to: IssueCollectionMode,
    },
}

impl IssueConfirmationTarget {
    pub fn workspace(&self) -> &str {
        match self {
            Self::Decision { workspace, .. } | Self::Mode { workspace, .. } => workspace,
        }
    }

    pub fn validate(&self) -> Result<(), IssueConfirmationError> {
        let workspace = self.workspace();
        if workspace.is_empty()
            || workspace.len() > 4096
            || workspace.trim() != workspace
            || workspace.chars().any(char::is_control)
        {
            return Err(IssueConfirmationError::Invalid);
        }
        match self {
            Self::Decision {
                option,
                basis,
                decision_revision,
                ..
            } => {
                if !identifier(option, MAX_ISSUE_DECISION_OPTION_BYTES)
                    || !valid_revision(*decision_revision)
                    || basis.validate().is_err()
                {
                    return Err(IssueConfirmationError::Invalid);
                }
            }
            Self::Mode {
                from,
                epoch,
                revision,
                ..
            } => match (from, epoch, revision) {
                (Some(_), Some(epoch), Some(revision))
                    if valid_revision(*epoch) && valid_revision(*revision) && revision >= epoch => {
                }
                (None, None, None) => {}
                _ => return Err(IssueConfirmationError::Invalid),
            },
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueConfirmationBinding {
    pub registration_id: String,
    pub connection_id: String,
    pub attachment_id: String,
    pub client_id: String,
    pub workspace: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueConfirmationIntent {
    pub binding: IssueConfirmationBinding,
    pub target: IssueConfirmationTarget,
    pub key: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueConfirmationPreview {
    pub target: IssueConfirmationTarget,
    pub display: IssueConfirmationDisplay,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueConfirmationDisplay {
    pub message: String,
    pub detail: String,
    pub confirm_label: String,
}

/// Gateway가 main 전용 broker에서 검증한 행동이다.
/// 직렬화/역직렬화가 없고 필드가 비공개라 client payload로 만들 수 없다.
/// 같은 논리 명령의 내부 복제는 허용하며 CommandMemory와 저장 CAS가 중복 효과를 막는다.
#[derive(Clone, PartialEq, Eq)]
pub struct VerifiedHumanConfirmation {
    target: IssueConfirmationTarget,
    confirmation_ref: String,
}

impl VerifiedHumanConfirmation {
    /// 실제 broker 검증 성공 뒤에만 호출한다. 이 함수는 proof를 검증하지 않는다.
    pub(crate) fn from_verified(
        target: IssueConfirmationTarget,
        confirmation_ref: String,
    ) -> Result<Self, IssueConfirmationError> {
        target.validate()?;
        validate_issue_decision_confirmation_reference(&confirmation_ref)
            .map_err(|_| IssueConfirmationError::Invalid)?;
        Ok(Self {
            target,
            confirmation_ref,
        })
    }

    pub fn target(&self) -> &IssueConfirmationTarget {
        &self.target
    }

    pub fn reference(&self) -> &str {
        &self.confirmation_ref
    }

    pub fn require_target(
        &self,
        expected: &IssueConfirmationTarget,
    ) -> Result<(), IssueConfirmationError> {
        if self.target != *expected {
            return Err(IssueConfirmationError::ScopeMismatch);
        }
        Ok(())
    }
}

impl std::fmt::Debug for VerifiedHumanConfirmation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("VerifiedHumanConfirmation(<verified metadata>)")
    }
}

/// 신규 명령이 아무것도 바꾸지 않고 거절되는 이유다. 원문이나 비밀을 넣지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueConfirmationError {
    Required,
    ScopeMismatch,
    Invalid,
}

impl std::fmt::Display for IssueConfirmationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Required => "this action requires a Desktop human confirmation",
            Self::ScopeMismatch => "the human confirmation no longer matches this action",
            Self::Invalid => "the human confirmation metadata is invalid",
        })
    }
}

impl std::error::Error for IssueConfirmationError {}

fn identifier(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && !value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
}

fn valid_revision(revision: u64) -> bool {
    revision > 0 && revision <= MAX_ISSUE_AUTOMATION_REVISION
}

#[cfg(test)]
mod tests;
