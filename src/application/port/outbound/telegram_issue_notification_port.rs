use anyhow::Result;

/// 보낸 뒤의 상태이다. `Unknown` 은 다시 보내지 않는다. 전송이 Telegram 에 닿았는지 모른다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramIssueNotificationEnd {
    Sent,
    Unknown,
}

/// 이슈 알림의 kernel 기록 cursor 와 알림 intent 의 영속 장부이다 (IA8-3).
/// 한 intent 와 chat 은 한 번만 보낸다 (at-most-once). 재시작 뒤에도 같다.
pub trait TelegramIssueNotificationLedgerPort: Send + Sync {
    /// 끝나지 않은 전송 (`sending`) 을 `unknown` 으로 바꾸고 cursor 를 준다. cursor 가 없으면 None 이다.
    fn recover_issue_notifications(
        &self,
        workspace_dir: &str,
        stream_key: &str,
    ) -> Result<Option<i64>>;

    fn save_issue_notification_cursor(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        after: i64,
    ) -> Result<()>;

    /// 처음 잡은 intent 와 chat 일 때만 true 이다. 이미 있는 intent 는 상태와 관계없이 false 이다.
    fn claim_issue_notification(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        intent_key: &str,
        chat_id: i64,
    ) -> Result<bool>;

    fn finish_issue_notification(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        intent_key: &str,
        chat_id: i64,
        end: TelegramIssueNotificationEnd,
    ) -> Result<()>;
}

/// 열린 결정 요청의 선택지 ID 이다. 닫혔거나 없는 요청은 None 이다. 질문과 선택지의 글은 주지 않는다.
pub trait TelegramIssueDecisionLookupPort: Send + Sync {
    fn open_decision_options(
        &self,
        workspace_dir: &str,
        decision_id: &str,
    ) -> Result<Option<Vec<String>>>;
}
