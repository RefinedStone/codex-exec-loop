//! IA8-3: Telegram 의 답과 알림을 실제 command-only 커널, SQLite 장부, kernel 기록으로 검증한다.
use std::sync::Mutex;

use super::issue_decision::seed_decision;
use super::*;
use crate::adapter::outbound::db::SqliteIssueAutomationStoreAdapter;
use crate::application::port::inbound::telegram_issue_port::{
    TelegramIssueAnswerOutcome, TelegramIssueNotificationPort,
};
use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::port::outbound::telegram_bot_port::{
    TelegramBotIdentity, TelegramBotPort, TelegramPollRequest, TelegramSendMessageRequest,
    TelegramUpdate,
};
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::application::service::telegram_issue_notification::TelegramIssueNotificationService;
use crate::composition::telegram_issue::{StoredIssueDecisionLookup, answer_through_kernel};
use crate::domain::issue_automation::decision::IssueDecisionId;

fn store() -> SqliteIssueAutomationStoreAdapter {
    SqliteIssueAutomationStoreAdapter::new(|workspace| {
        let location =
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?;
        Ok(PathBuf::from(location.runtime_dir))
    })
}

fn telegram(kernel: &CommandOnlyKernel) -> CommandOnlyKernel {
    CommandOnlyKernel {
        workspace: kernel.workspace.clone(),
        caller: CALLER_TELEGRAM,
    }
}

#[test]
fn a_telegram_answer_cannot_pass_a_human_gate_and_a_changed_basis_is_superseded() {
    let (_directory, kernel) = fixture();
    seed_decision(&kernel);
    let telegram = telegram(&kernel);
    assert_eq!(
        answer_through_kernel(&telegram, "offline-decision", "acknowledge").unwrap(),
        TelegramIssueAnswerOutcome::Rejected(ReasonCode::HumanConfirmationRequired)
    );

    // 저장소 정책이 바뀌면 열린 요청의 판이 낡는다. 그 뒤의 답은 적용되지 않는다.
    let project = PathBuf::from(&kernel.workspace).join(".akra");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("issue-automation.toml"),
        "[budget]\nmax_attempts = 1\n",
    )
    .unwrap();
    assert_eq!(
        answer_through_kernel(&telegram, "offline-decision", "acknowledge").unwrap(),
        TelegramIssueAnswerOutcome::Superseded
    );
    let stored = store()
        .get_issue_decision(
            &kernel.workspace,
            &IssueDecisionId::new("offline-decision").unwrap(),
        )
        .unwrap()
        .unwrap();
    assert!(stored.request.answer().is_none());
    let callers = telegram
        .records(crate::composition::production::kernel_event_log_port().as_ref())
        .unwrap()
        .into_iter()
        .filter_map(|record| match record.event {
            KernelEvent::CommandReceived { caller, .. } => caller,
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(callers, vec![CALLER_TELEGRAM, CALLER_TELEGRAM]);
}

#[derive(Default)]
struct Bot(Mutex<Vec<String>>);

impl TelegramBotPort for Bot {
    fn get_me(&self) -> Result<TelegramBotIdentity> {
        unreachable!()
    }

    fn get_updates(&self, _request: &TelegramPollRequest) -> Result<Vec<TelegramUpdate>> {
        unreachable!()
    }

    fn send_message(&self, request: &TelegramSendMessageRequest) -> Result<()> {
        self.0.lock().unwrap().push(request.text.clone());
        Ok(())
    }
}

struct TelegramOn;

impl IssueAutomationPolicySourcePort for TelegramOn {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        Ok(Some("[feedback]\ntelegram = true\n".into()))
    }

    fn load_project_policy_toml(&self, _workspace: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

#[test]
fn a_recorded_decision_request_is_announced_once_across_a_restart_without_issue_text() {
    let (_directory, kernel) = fixture();
    let bot = Arc::new(Bot::default());
    let deliver = || {
        TelegramIssueNotificationService::new(
            kernel.workspace.clone(),
            bot.clone(),
            Arc::new(SqlitePlanningAuthorityAdapter::new()),
            crate::composition::production::kernel_event_log_port(),
            Arc::new(StoredIssueDecisionLookup(store())),
            IssueAutomationPolicyService::new(Arc::new(TelegramOn)),
        )
        .deliver("bot-id:42", &[42])
        .unwrap()
    };
    assert_eq!(deliver(), 0);

    seed_decision(&kernel);
    let stored = store()
        .get_issue_decision(
            &kernel.workspace,
            &IssueDecisionId::new("offline-decision").unwrap(),
        )
        .unwrap()
        .unwrap();
    let log = crate::composition::production::kernel_event_log_port();
    let sequence = log.last_kernel_event_sequence(&kernel.workspace).unwrap() + 1;
    log.append_kernel_events(
        &kernel.workspace,
        &[RecordedKernelEvent {
            sequence,
            recorded_at: "2026-11-25T00:00:00Z".into(),
            event: KernelEvent::issue_decision_requested(&stored.request.fact(), stored.revision),
        }],
    )
    .unwrap();

    assert_eq!(deliver(), 1);
    // 새 service 는 재시작한 runner 이다. 같은 요청을 다시 보내지 않는다.
    assert_eq!(deliver(), 0);
    let sent = bot.0.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("결정: offline-decision"), "{}", sent[0]);
    assert!(sent[0].contains("선택지: acknowledge"), "{}", sent[0]);
    for private in ["private original body", "legacy title", "공유된 질문"] {
        assert!(!sent[0].contains(private), "{private}: {}", sent[0]);
    }
}
