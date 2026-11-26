use serde_json::Value;

/// Gateway 가 Desktop 에 알리는 쓰기 승인 사건이다 (#2830).
#[derive(Debug, Clone, PartialEq)]
pub enum McpWriteApprovalEvent {
    Requested {
        id: String,
        server: String,
        tool: String,
        arguments: Value,
    },
    Resolved {
        id: String,
    },
}

/// 구독을 버리면 그 구독자에게 더 알리지 않는다.
pub type McpWriteApprovalSubscription = Box<dyn Send>;

/// Desktop 접속이 쓰기 승인 요청을 받고 답하는 입구이다.
pub trait McpWriteApprovalInboxPort: Send + Sync {
    fn subscribe(
        &self,
        listener: Box<dyn Fn(&McpWriteApprovalEvent) + Send + Sync>,
    ) -> McpWriteApprovalSubscription;

    /// 기다리는 요청이면 답을 넘기고 `true` 이다. 이미 끝났거나 모르는 요청이면 `false` 이다.
    fn answer(&self, id: &str, approve: bool) -> bool;
}
