use serde::{Deserialize, Serialize};

/// 정책이 정한 보드 필드 하나에서 이슈의 항목이 가진 값이다 (#2565).
/// 정책에 보드가 없으면 관찰에 넣지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueSourceBoardColumn {
    pub project_id: String,
    pub field_id: String,
    pub read: IssueSourceBoardRead,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum IssueSourceBoardRead {
    /// 이슈가 이 보드에 없다.
    NotOnBoard,
    /// `option_id` 가 없으면 필드 값이 비어 있다.
    Item {
        item_id: String,
        option_id: Option<String>,
    },
    /// 보드를 읽지 못했다 (`read:project` 권한 없음, 쪽 제한 초과, 응답 실패).
    /// 열 불일치로도, 보드에 없음으로도 보지 않는다.
    Unavailable,
}
