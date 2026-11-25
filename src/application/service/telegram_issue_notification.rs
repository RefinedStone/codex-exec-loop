//! 이슈 결정 요청, 중재자의 결정, 실행 실패를 Telegram 으로 알린다 (IA8-3, plan 28 §10.2).
//! 입력은 kernel 기록의 사건이다. 알림에는 식별자와 상태만 넣는다. 이슈의 제목, 본문, 경로, 오류 글은 넣지 않는다.
use std::sync::Arc;

use anyhow::Result;
use serde::Serialize;

use crate::application::port::inbound::telegram_issue_port::TelegramIssueNotificationPort;
use crate::application::port::outbound::kernel_event_log_port::KernelEventLogPort;
use crate::application::port::outbound::telegram_bot_port::{
    TelegramBotPort, TelegramSendMessageRequest,
};
use crate::application::port::outbound::telegram_issue_notification_port::{
    TelegramIssueDecisionLookupPort, TelegramIssueNotificationEnd,
    TelegramIssueNotificationLedgerPort,
};
use crate::domain::issue_automation::arbiter::IssueArbiterFinishedOutcome;
use crate::domain::issue_automation::execution::IssueExecutionState;
use crate::domain::kernel_event::{IssueDecisionFactRecord, IssueEventReference, KernelEvent};

use super::issue_automation_policy::IssueAutomationPolicyService;

/// 한 주기에 읽는 kernel 기록의 쪽 크기와 쪽 수이다. 남은 기록은 다음 주기에 읽는다.
const EVENT_PAGE: usize = 256;
const MAX_PAGES_PER_CYCLE: usize = 8;

pub struct TelegramIssueNotificationService {
    workspace_dir: String,
    bot: Arc<dyn TelegramBotPort>,
    ledger: Arc<dyn TelegramIssueNotificationLedgerPort>,
    events: Arc<dyn KernelEventLogPort>,
    decisions: Arc<dyn TelegramIssueDecisionLookupPort>,
    policy: IssueAutomationPolicyService,
}

impl TelegramIssueNotificationService {
    pub fn new(
        workspace_dir: String,
        bot: Arc<dyn TelegramBotPort>,
        ledger: Arc<dyn TelegramIssueNotificationLedgerPort>,
        events: Arc<dyn KernelEventLogPort>,
        decisions: Arc<dyn TelegramIssueDecisionLookupPort>,
        policy: IssueAutomationPolicyService,
    ) -> Self {
        Self {
            workspace_dir,
            bot,
            ledger,
            events,
            decisions,
            policy,
        }
    }

    fn send_once(&self, stream_key: &str, chat_ids: &[i64], notice: &IssueNotice) -> Result<usize> {
        let mut sent = 0;
        for &chat_id in chat_ids {
            if !self.ledger.claim_issue_notification(
                &self.workspace_dir,
                stream_key,
                &notice.intent_key,
                chat_id,
            )? {
                continue;
            }
            // 실패한 전송은 Telegram 에 닿았는지 모른다. 다시 보내지 않고 unknown 으로 남긴다.
            let end = match self
                .bot
                .send_message(&TelegramSendMessageRequest::new(chat_id, &notice.text))
            {
                Ok(()) => {
                    sent += 1;
                    TelegramIssueNotificationEnd::Sent
                }
                Err(error) => {
                    eprintln!(
                        "telegram issue notification {} to chat {chat_id} has an unknown result: {error:#}",
                        notice.intent_key
                    );
                    TelegramIssueNotificationEnd::Unknown
                }
            };
            self.ledger.finish_issue_notification(
                &self.workspace_dir,
                stream_key,
                &notice.intent_key,
                chat_id,
                end,
            )?;
        }
        Ok(sent)
    }

    fn notice_for(&self, event: &KernelEvent) -> Result<Option<IssueNotice>> {
        let Some(notice) = issue_notice(event) else {
            return Ok(None);
        };
        let KernelEvent::IssueDecisionRequested { decision, .. } = event else {
            return Ok(Some(notice));
        };
        // 이미 닫힌 요청은 알리지 않는다. 선택지의 ID 만 싣는다.
        let Some(options) = self
            .decisions
            .open_decision_options(&self.workspace_dir, &decision.decision_id)?
        else {
            return Ok(None);
        };
        Ok(Some(IssueNotice {
            text: format!(
                "{}\n선택지: {}\n답: /decide {} <선택지>",
                notice.text,
                options.join(", "),
                decision.decision_id
            ),
            ..notice
        }))
    }
}

impl TelegramIssueNotificationPort for TelegramIssueNotificationService {
    fn deliver(&self, stream_key: &str, chat_ids: &[i64]) -> Result<usize> {
        let last = self
            .events
            .last_kernel_event_sequence(&self.workspace_dir)?;
        let enabled = match self.policy.resolve(&self.workspace_dir) {
            Ok(resolved) => resolved.policy.feedback.telegram,
            // 깨진 정책은 보내지 않는다. cursor 는 두어 고친 뒤에 남은 사건을 알린다.
            Err(_) => return Ok(0),
        };
        let cursor = self
            .ledger
            .recover_issue_notifications(&self.workspace_dir, stream_key)?;
        // 처음 켠 때와 꺼진 동안의 사건은 알리지 않는다. 기록이 초기화되면 끝에서 다시 시작한다.
        let mut after = match cursor {
            Some(after) if enabled && after <= last => after,
            _ => {
                self.ledger.save_issue_notification_cursor(
                    &self.workspace_dir,
                    stream_key,
                    last,
                )?;
                return Ok(0);
            }
        };
        let mut sent = 0;
        for _ in 0..MAX_PAGES_PER_CYCLE {
            let page =
                self.events
                    .load_kernel_events_after(&self.workspace_dir, after, EVENT_PAGE)?;
            for record in &page.records {
                if let Some(notice) = self.notice_for(&record.event)? {
                    sent += self.send_once(stream_key, chat_ids, &notice)?;
                }
            }
            if page.next_after <= after {
                break;
            }
            after = page.next_after;
            self.ledger
                .save_issue_notification_cursor(&self.workspace_dir, stream_key, after)?;
        }
        Ok(sent)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IssueNotice {
    /// 같은 사실의 알림은 같은 key 이다. 장부는 key 마다 한 번만 보낸다.
    intent_key: String,
    text: String,
}

fn issue_notice(event: &KernelEvent) -> Option<IssueNotice> {
    match event {
        KernelEvent::IssueDecisionRequested { decision, .. } => Some(IssueNotice {
            intent_key: format!("decision_requested:{}", decision.decision_id),
            text: format!("[Akra] 새 결정 요청\n{}", decision_lines(decision)),
        }),
        KernelEvent::IssueDecisionArbiterFinished {
            event_id,
            decision,
            outcome,
            ..
        } => {
            let result = match outcome {
                IssueArbiterFinishedOutcome::Applied {} => format!(
                    "중재자가 결정했습니다: {}",
                    decision.option.as_deref().unwrap_or("-")
                ),
                IssueArbiterFinishedOutcome::Waiting { reason } => {
                    format!(
                        "중재자가 결정하지 않았습니다 ({}). 사람의 답을 기다립니다.",
                        name(reason)
                    )
                }
                // 사람이 먼저 답했거나 판이 바뀌었다. 알릴 새 사실이 없다.
                IssueArbiterFinishedOutcome::Superseded {}
                | IssueArbiterFinishedOutcome::Discarded {} => return None,
            };
            Some(IssueNotice {
                intent_key: format!("arbiter_finished:{event_id}"),
                text: format!("[Akra] 중재자\n{}\n{result}", decision_lines(decision)),
            })
        }
        KernelEvent::IssueExecutionProgressed {
            issue,
            work_order_id,
            progress,
            ..
        } if progress.state == IssueExecutionState::Failed => {
            let terminal = progress.terminal.as_ref();
            Some(IssueNotice {
                intent_key: format!(
                    "execution_failed:{work_order_id}:{}",
                    terminal.map_or("-", |terminal| terminal.record_id.as_str())
                ),
                text: format!(
                    "[Akra] 이슈 작업 실패\n이슈: {}\n작업 지시: {work_order_id}\n원인: {}\n실행 횟수: {}",
                    issue_line(issue),
                    terminal.map_or_else(
                        || "attempts_exhausted".to_string(),
                        |terminal| name(&terminal.classification)
                    ),
                    progress.attempts.execution
                ),
            })
        }
        _ => None,
    }
}

fn decision_lines(decision: &IssueDecisionFactRecord) -> String {
    let crate::domain::kernel_event::IssueDecisionSubjectRecord::Issue { issue } =
        &decision.subject;
    format!(
        "결정: {}\n이슈: {}\n관문: {} · 종류: {}",
        decision.decision_id,
        issue_line(issue),
        name(&decision.gate),
        name(&decision.kind)
    )
}

fn issue_line(issue: &IssueEventReference) -> String {
    format!(
        "{} {} {}",
        name(&issue.provider),
        issue.repository_id,
        issue.issue_id
    )
}

/// 닫힌 enum 의 전송 이름이다. 글이 아니므로 알림에 넣어도 된다.
fn name(value: &impl Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "-".to_string())
}

#[cfg(test)]
mod tests;
