//! 사용자가 합의한 완료 기준의 저장 계약과 의미 비교다. 실행 또는 통과 판정을 포함하지 않는다.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskAcceptanceCriterion {
    pub id: String,
    pub scenario: String,
    pub channel: TaskAcceptanceChannel,
    pub essential: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskAcceptanceChannel {
    Command,
    Data,
    Http,
    Screen,
}

impl TaskAcceptanceCriterion {
    pub fn normalized(&self) -> Self {
        Self {
            id: self.id.trim().to_string(),
            scenario: self.scenario.trim().to_string(),
            channel: self.channel,
            essential: self.essential,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskAcceptanceError {
    BlankId,
    BlankScenario,
    DuplicateId(String),
    MissingRevision,
}

impl Display for TaskAcceptanceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BlankId => formatter.write_str("task acceptance id must not be blank"),
            Self::BlankScenario => {
                formatter.write_str("task acceptance scenario must not be blank")
            }
            Self::DuplicateId(id) => write!(formatter, "duplicate task acceptance id: {id}"),
            Self::MissingRevision => {
                formatter.write_str("non-empty task acceptance requires a positive host revision")
            }
        }
    }
}
impl std::error::Error for TaskAcceptanceError {}

pub fn normalize_task_acceptance(
    criteria: &[TaskAcceptanceCriterion],
) -> Result<Vec<TaskAcceptanceCriterion>, TaskAcceptanceError> {
    let mut normalized = Vec::with_capacity(criteria.len());
    let mut ids = BTreeSet::new();
    for criterion in criteria {
        let criterion = criterion.normalized();
        if criterion.id.is_empty() {
            return Err(TaskAcceptanceError::BlankId);
        }
        if criterion.scenario.is_empty() {
            return Err(TaskAcceptanceError::BlankScenario);
        }
        if !ids.insert(criterion.id.clone()) {
            return Err(TaskAcceptanceError::DuplicateId(criterion.id));
        }
        normalized.push(criterion);
    }
    normalized.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(normalized)
}

pub fn validate_task_acceptance(
    criteria: &[TaskAcceptanceCriterion],
    revision: u64,
) -> Result<(), TaskAcceptanceError> {
    normalize_task_acceptance(criteria)?;
    if !criteria.is_empty() && revision == 0 {
        return Err(TaskAcceptanceError::MissingRevision);
    }
    // 삭제된 기준은 비어 있어도 이미 부여된 revision을 유지한다.
    Ok(())
}

#[cfg(test)]
mod tests;
