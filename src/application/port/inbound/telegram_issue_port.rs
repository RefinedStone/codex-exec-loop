//! Telegram 입구의 이슈 결정 알림과 답이다 (IA8-3, plan 28 §10.2).
use anyhow::Result;

use crate::domain::kernel_spec::ReasonCode;

/// 정책 (`feedback.telegram`) 이 켠 때만 새 알림을 보낸다. 같은 알림은 다시 보내지 않는다.
pub trait TelegramIssueNotificationPort: Send + Sync {
    /// 보낸 알림의 수를 준다. 정책이 끄면 0 이다.
    fn deliver(&self, stream_key: &str, chat_ids: &[i64]) -> Result<usize>;
}

/// 커널이 답을 처리한 결과이다. 답은 `by: agent` 이다. 사람 확인의 증거가 아니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramIssueAnswerOutcome {
    Answered,
    /// 판이 바뀐 요청이다. 답을 적용하지 않았다.
    Superseded,
    AlreadyResolved,
    Rejected(ReasonCode),
    /// 커널이 답을 저장하지 못했다. 원인의 글은 Telegram 에 보내지 않는다.
    Failed,
}

/// 같은 kernel 명령 (`answer_issue_decision`) 을 `caller: akra-telegram` 으로 보낸다.
pub trait TelegramIssueAnswerPort: Send + Sync {
    fn answer(&self, decision_id: &str, option: &str) -> Result<TelegramIssueAnswerOutcome>;
}
