use serde_json::Value;

/// 쓰기 도구 호출 하나의 승인 요청이다 (#2830).
#[derive(Debug, Clone, PartialEq)]
pub struct McpWriteApprovalRequest {
    pub server: String,
    pub tool: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpWriteApproval {
    Approved,
    Declined,
    TimedOut,
    /// 물어볼 사람이 없다 (Desktop 이 Gateway 에 붙어 있지 않다).
    NoApprover,
}

/*
 * MCP hub 의 쓰기 도구 호출을 사람에게 묻는 바깥 경계다. Gateway 는 붙어 있는 Desktop 에 묻고 답을 기다린다.
 * 답이 오기 전에는 돌아오지 않는다. 승인이 아닌 모든 결과에서 도구를 실행하지 않는다.
 */
pub trait McpWriteApprovalPort: Send + Sync {
    fn request_approval(&self, request: &McpWriteApprovalRequest) -> McpWriteApproval;
}
