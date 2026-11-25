use std::collections::BTreeMap;
use std::sync::Mutex;

use anyhow::bail;

use super::*;
use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
use crate::application::port::outbound::telegram_bot_port::{
    TelegramBotIdentity, TelegramPollRequest, TelegramUpdate,
};
use crate::domain::issue_automation::arbiter::IssueArbiterWaitReason;
use crate::domain::issue_automation::decision::{IssueDecisionKind, IssueDecisionState};
use crate::domain::issue_automation::execution::{
    IssueExecutionAttempts, IssueExecutionProgress, IssueExecutionTerminalClass,
    IssueExecutionTerminalRecord,
};
use crate::domain::kernel_event::{
    IssueDecisionBasisRecord, IssueDecisionGateRecord, IssueDecisionSubjectRecord,
    IssueEventProjectScope, IssueEventProvider, KernelEventPage, RecordedKernelEvent,
};

const WORKSPACE: &str = "/repo";
const STREAM: &str = "bot-id:42";
const ON: &str = "[feedback]\ntelegram = true\n";
// 이슈의 원문은 kernel 기록에 없다. 이 글이 알림에 나오면 안 된다.
const ISSUE_BODY: &str = "SECRET ISSUE BODY";

#[derive(Default)]
struct Bot {
    sent: Mutex<Vec<(i64, String)>>,
    fail: bool,
}

impl TelegramBotPort for Bot {
    fn get_me(&self) -> Result<TelegramBotIdentity> {
        unreachable!()
    }

    fn get_updates(&self, _request: &TelegramPollRequest) -> Result<Vec<TelegramUpdate>> {
        unreachable!()
    }

    fn send_message(&self, request: &TelegramSendMessageRequest) -> Result<()> {
        self.sent
            .lock()
            .unwrap()
            .push((request.chat_id, request.text.clone()));
        if self.fail {
            bail!("timeout after the request left");
        }
        Ok(())
    }
}

/// 재시작 뒤에도 남는 장부이다. 같은 값을 새 service 에 넘겨 재시작을 흉내 낸다.
#[derive(Default)]
struct Ledger {
    cursor: Mutex<Option<i64>>,
    intents: Mutex<BTreeMap<(String, i64), &'static str>>,
}

impl TelegramIssueNotificationLedgerPort for Ledger {
    fn recover_issue_notifications(&self, _workspace: &str, _stream: &str) -> Result<Option<i64>> {
        for state in self.intents.lock().unwrap().values_mut() {
            if *state == "sending" {
                *state = "unknown";
            }
        }
        Ok(*self.cursor.lock().unwrap())
    }

    fn save_issue_notification_cursor(
        &self,
        _workspace: &str,
        _stream: &str,
        after: i64,
    ) -> Result<()> {
        *self.cursor.lock().unwrap() = Some(after);
        Ok(())
    }

    fn claim_issue_notification(
        &self,
        _workspace: &str,
        _stream: &str,
        intent: &str,
        chat_id: i64,
    ) -> Result<bool> {
        let mut intents = self.intents.lock().unwrap();
        let key = (intent.to_string(), chat_id);
        if intents.contains_key(&key) {
            return Ok(false);
        }
        intents.insert(key, "sending");
        Ok(true)
    }

    fn finish_issue_notification(
        &self,
        _workspace: &str,
        _stream: &str,
        intent: &str,
        chat_id: i64,
        end: TelegramIssueNotificationEnd,
    ) -> Result<()> {
        let state = match end {
            TelegramIssueNotificationEnd::Sent => "sent",
            TelegramIssueNotificationEnd::Unknown => "unknown",
        };
        self.intents
            .lock()
            .unwrap()
            .insert((intent.to_string(), chat_id), state);
        Ok(())
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<KernelEvent>>);

impl Events {
    fn push(&self, event: KernelEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl KernelEventLogPort for Events {
    fn last_kernel_event_sequence(&self, _workspace: &str) -> Result<i64> {
        Ok(self.0.lock().unwrap().len() as i64)
    }

    fn load_kernel_events(&self, _workspace: &str, _kinds: &[&str]) -> Result<Vec<KernelEvent>> {
        unreachable!()
    }

    fn append_kernel_events(
        &self,
        _workspace: &str,
        _records: &[RecordedKernelEvent],
    ) -> Result<()> {
        unreachable!()
    }

    fn load_kernel_events_after(
        &self,
        _workspace: &str,
        after: i64,
        limit: usize,
    ) -> Result<KernelEventPage> {
        let records = self
            .0
            .lock()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(index, event)| RecordedKernelEvent {
                sequence: index as i64 + 1,
                recorded_at: "2026-11-25T00:00:00Z".into(),
                event: event.clone(),
            })
            .filter(|record| record.sequence > after)
            .take(limit)
            .collect::<Vec<_>>();
        let next_after = records.last().map_or(after, |record| record.sequence);
        Ok(KernelEventPage {
            records,
            next_after,
            recorder: None,
        })
    }
}

#[derive(Default)]
struct Decisions(Mutex<BTreeMap<String, Vec<String>>>);

impl TelegramIssueDecisionLookupPort for Decisions {
    fn open_decision_options(
        &self,
        _workspace: &str,
        decision_id: &str,
    ) -> Result<Option<Vec<String>>> {
        Ok(self.0.lock().unwrap().get(decision_id).cloned())
    }
}

struct Policy(Mutex<Option<&'static str>>);

impl IssueAutomationPolicySourcePort for Policy {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().map(str::to_string))
    }

    fn load_project_policy_toml(&self, _workspace: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

struct Harness {
    bot: Arc<Bot>,
    ledger: Arc<Ledger>,
    events: Arc<Events>,
    decisions: Arc<Decisions>,
    policy: Arc<Policy>,
}

impl Harness {
    fn new(policy: Option<&'static str>, fail: bool) -> Self {
        Self {
            bot: Arc::new(Bot {
                sent: Mutex::new(Vec::new()),
                fail,
            }),
            ledger: Arc::default(),
            events: Arc::default(),
            decisions: Arc::default(),
            policy: Arc::new(Policy(Mutex::new(policy))),
        }
    }

    /// 매번 새 service 이다. 장부와 기록은 같다. 그래서 각 호출이 재시작한 runner 와 같다.
    fn deliver(&self) -> usize {
        TelegramIssueNotificationService::new(
            WORKSPACE.into(),
            self.bot.clone(),
            self.ledger.clone(),
            self.events.clone(),
            self.decisions.clone(),
            IssueAutomationPolicyService::new(self.policy.clone()),
        )
        .deliver(STREAM, &[7])
        .unwrap()
    }

    fn texts(&self) -> Vec<String> {
        let sent = self.bot.sent.lock().unwrap();
        sent.iter().map(|(_, text)| text.clone()).collect()
    }

    fn request(&self, id: &str) {
        self.decisions
            .0
            .lock()
            .unwrap()
            .insert(id.into(), vec!["start".into(), "reject".into()]);
        self.events.push(KernelEvent::IssueDecisionRequested {
            decision: fact(id),
            revision: 1,
        });
    }
}

fn issue() -> IssueEventReference {
    IssueEventReference {
        provider: IssueEventProvider::GitHub,
        server_id: "github.com".into(),
        repository_id: "R_repo".into(),
        issue_id: "I_issue".into(),
        project_scope: IssueEventProjectScope {
            workspace_dir: WORKSPACE.into(),
        },
    }
}

fn fact(id: &str) -> IssueDecisionFactRecord {
    IssueDecisionFactRecord {
        decision_id: id.into(),
        subject: IssueDecisionSubjectRecord::Issue { issue: issue() },
        gate: IssueDecisionGateRecord::Start,
        kind: IssueDecisionKind::StartApproval,
        basis: IssueDecisionBasisRecord {
            policy_version: "a".repeat(64),
            field_hash: None,
            work_order_id: None,
            gate_revision: 1,
            authorization_epoch: 0,
            delivery: None,
        },
        state: IssueDecisionState::Open,
        answer_by: None,
        option: None,
        delivery_review: None,
    }
}

#[test]
fn a_new_decision_request_is_one_notification_and_a_restart_does_not_send_it_again() {
    let harness = Harness::new(Some(ON), false);
    // 처음 켠 때는 기록의 끝에서 시작한다. 켜기 전의 요청은 알리지 않는다.
    harness.request("before-start");
    assert_eq!(harness.deliver(), 0);

    harness.request("decision-1");
    assert_eq!(harness.deliver(), 1);
    // 재시작 뒤: cursor 와 intent 장부가 남아 있다.
    assert_eq!(harness.deliver(), 0);
    // cursor 를 잃어도 같은 intent 는 다시 보내지 않는다.
    *harness.ledger.cursor.lock().unwrap() = Some(1);
    assert_eq!(harness.deliver(), 0);

    let texts = harness.texts();
    assert_eq!(texts.len(), 1);
    assert!(texts[0].contains("결정: decision-1"), "{}", texts[0]);
    assert!(texts[0].contains("선택지: start, reject"), "{}", texts[0]);
    assert!(
        texts[0].contains("/decide decision-1 <선택지>"),
        "{}",
        texts[0]
    );
    assert!(!texts[0].contains(ISSUE_BODY));
}

#[test]
fn policy_off_sends_nothing_and_later_enabling_does_not_replay_old_requests() {
    let harness = Harness::new(None, false);
    harness.deliver();
    harness.request("while-off");
    assert_eq!(harness.deliver(), 0);

    *harness.policy.0.lock().unwrap() = Some(ON);
    assert_eq!(harness.deliver(), 0);
    harness.request("after-on");
    assert_eq!(harness.deliver(), 1);
    assert_eq!(harness.texts().len(), 1);
    assert!(harness.texts()[0].contains("after-on"));
}

#[test]
fn a_closed_request_is_not_announced_and_an_unknown_send_is_not_retried() {
    let harness = Harness::new(Some(ON), true);
    harness.deliver();
    harness.request("answered-first");
    harness.decisions.0.lock().unwrap().remove("answered-first");
    harness.request("decision-2");
    assert_eq!(harness.deliver(), 0);
    *harness.ledger.cursor.lock().unwrap() = Some(0);
    assert_eq!(harness.deliver(), 0);
    // 한 번 시도했다. 결과를 모르므로 다시 보내지 않는다.
    assert_eq!(harness.bot.sent.lock().unwrap().len(), 1);
    assert_eq!(
        harness.ledger.intents.lock().unwrap()[&("decision_requested:decision-2".to_string(), 7)],
        "unknown"
    );
}

#[test]
fn arbiter_decisions_and_failures_carry_identifiers_and_states_only() {
    let harness = Harness::new(Some(ON), false);
    harness.deliver();
    let mut decided = fact("decision-3");
    decided.state = IssueDecisionState::Answered;
    decided.option = Some("start".into());
    harness
        .events
        .push(KernelEvent::IssueDecisionArbiterFinished {
            event_id: "arbiter-1".into(),
            decision: decided.clone(),
            outcome: IssueArbiterFinishedOutcome::Applied {},
            at: "2026-11-25T00:00:00Z".into(),
            revision: 2,
        });
    harness
        .events
        .push(KernelEvent::IssueDecisionArbiterFinished {
            event_id: "arbiter-2".into(),
            decision: fact("decision-4"),
            outcome: IssueArbiterFinishedOutcome::Waiting {
                reason: IssueArbiterWaitReason::LowConfidence,
            },
            at: "2026-11-25T00:00:00Z".into(),
            revision: 3,
        });
    harness
        .events
        .push(KernelEvent::IssueDecisionArbiterFinished {
            event_id: "arbiter-3".into(),
            decision: decided,
            outcome: IssueArbiterFinishedOutcome::Discarded {},
            at: "2026-11-25T00:00:00Z".into(),
            revision: 4,
        });
    let failed = KernelEvent::IssueExecutionProgressed {
        event_id: "progress-1".into(),
        issue: issue(),
        work_order_id: "wo-1".into(),
        progress: IssueExecutionProgress {
            state: IssueExecutionState::Failed,
            attempts: IssueExecutionAttempts {
                execution: 3,
                verification: 0,
                publication: 0,
            },
            terminal: Some(IssueExecutionTerminalRecord {
                record_id: "terminal-1".into(),
                classification: IssueExecutionTerminalClass::ContractRecovery,
            }),
            review_pull_request_url: None,
            integration: None,
            attempts_exhausted: true,
            revision: 5,
        },
        revision: 5,
    };
    harness.events.push(failed.clone());
    // 같은 실패의 다음 진전은 새 알림이 아니다.
    harness.events.push(failed);
    assert_eq!(harness.deliver(), 3);

    let texts = harness.texts();
    assert!(
        texts[0].contains("중재자가 결정했습니다: start"),
        "{}",
        texts[0]
    );
    assert!(texts[1].contains("low_confidence"), "{}", texts[1]);
    assert!(texts[2].contains("이슈 작업 실패"), "{}", texts[2]);
    assert!(texts[2].contains("contract_recovery"), "{}", texts[2]);
    assert!(texts[2].contains("github R_repo I_issue"), "{}", texts[2]);
}
