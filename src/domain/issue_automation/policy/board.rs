use serde::{Deserialize, Serialize};

use super::IssueAutomationPolicyValueError;

const MAX_BOARD_SELECT_OPTIONS: usize = 32;

/// GitHub Projects (v2) 보드 하나의 단일 선택 필드(보통 Status)이다 (#2565).
/// 표시 이름이 아닌 안정 node ID 만 쓴다. 이름은 바뀔 수 있다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueAutomationGitHubBoard {
    pub project_id: String,
    pub field_id: String,
    /// 이 열(선택지)에 있는 이슈만 선택한다. 비면 열 조건이 없다.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub select_option_ids: Vec<String>,
    /// 진행 단계마다 쓸 선택지이다. 사용자 파일에서만 온다. `feedback.board_field` 가 켜져야 쓴다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<IssueAutomationBoardProgress>,
}

/// 진행 단계별 목표 선택지이다. 없는 단계는 쓰지 않는다.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueAutomationBoardProgress {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_review: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done: Option<String>,
}

impl IssueAutomationGitHubBoard {
    pub fn validate(&self) -> Result<(), IssueAutomationPolicyValueError> {
        node_id(&self.project_id, "binding.github.board.project_id")?;
        node_id(&self.field_id, "binding.github.board.field_id")?;
        if self.select_option_ids.len() > MAX_BOARD_SELECT_OPTIONS {
            return Err(IssueAutomationPolicyValueError::new(
                "binding.github.board.select_option_ids",
                "too_many_options",
            ));
        }
        for option in &self.select_option_ids {
            node_id(option, "binding.github.board.select_option_ids")?;
        }
        if let Some(progress) = &self.progress {
            for option in [&progress.running, &progress.in_review, &progress.done]
                .into_iter()
                .flatten()
            {
                node_id(option, "binding.github.board.progress")?;
            }
        }
        Ok(())
    }

    /// 같은 보드의 같은 필드인가이다. 선택지 목록과 진행 매핑은 보지 않는다.
    pub fn same_field(&self, other: &Self) -> bool {
        self.project_id == other.project_id && self.field_id == other.field_id
    }
}

// 선택지 ID 도 GitHub node ID 와 같은 문자 집합이다.
fn node_id(value: &str, field: &'static str) -> Result<(), IssueAutomationPolicyValueError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'='))
    {
        return Err(IssueAutomationPolicyValueError::new(
            field,
            "invalid_stable_node_id",
        ));
    }
    Ok(())
}
