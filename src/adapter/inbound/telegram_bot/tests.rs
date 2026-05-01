use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};

use super::config::{
    TelegramBotEnvironment, apply_environment_file, load_environment_from_sources,
    parse_args_with_environment,
};
use super::{
    TelegramBotPolicy, TelegramBotRunner, TelegramInboundCommand, TelegramParsedMessage,
    parse_message,
};
use crate::application::port::outbound::telegram_bot_port::{
    TelegramBotPort, TelegramInboundMessage, TelegramPollRequest, TelegramSendMessageRequest,
    TelegramUpdate,
};
use crate::application::service::planning::PlanningResetTarget;
use crate::application::service::planning::control::{
    PlanningControlCommand, PlanningControlQueueEntry, PlanningControlResetOutcome,
    PlanningControlService, PlanningControlStatusSnapshot, PlanningControlSurface,
};

struct FakePlanningControlSurface;

impl PlanningControlSurface for FakePlanningControlSurface {
    fn load_status_snapshot(&self) -> Result<PlanningControlStatusSnapshot> {
        Ok(PlanningControlStatusSnapshot {
            workspace_dir: "/tmp/repo".to_string(),
            planning_state: "ready".to_string(),
            queue_summary: Some("queue head ready".to_string()),
            proposal_summary: None,
            health: Some("planning workspace ready".to_string()),
            issue: None,
            note: None,
            preview_status_label: "queue ready".to_string(),
            preview_detail: None,
            queue_head: Some(PlanningControlQueueEntry {
                task_id: "task-1".to_string(),
                task_title: "Ship Telegram control".to_string(),
                direction_id: "general-workstream".to_string(),
                status: "ready".to_string(),
                combined_priority: 90,
            }),
            visible_tasks: Vec::new(),
            proposed_tasks: Vec::new(),
        })
    }

    fn reset_workspace(&self, target: PlanningResetTarget) -> Result<PlanningControlResetOutcome> {
        Ok(PlanningControlResetOutcome {
            target: target.label().to_string(),
            rewritten_paths: vec!["DB task authority".to_string()],
            removed_paths: Vec::new(),
            planning_state: "ready".to_string(),
            health: Some("queue reset complete".to_string()),
            issue: None,
        })
    }
}

struct FlakyPlanningControlSurface {
    load_calls: AtomicUsize,
}

impl PlanningControlSurface for FlakyPlanningControlSurface {
    fn load_status_snapshot(&self) -> Result<PlanningControlStatusSnapshot> {
        let call = self.load_calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            bail!("temporary planning failure");
        }
        FakePlanningControlSurface.load_status_snapshot()
    }

    fn reset_workspace(&self, target: PlanningResetTarget) -> Result<PlanningControlResetOutcome> {
        FakePlanningControlSurface.reset_workspace(target)
    }
}

#[derive(Default)]
struct FakeTelegramBotPort {
    poll_errors: Mutex<Vec<anyhow::Error>>,
    updates: Mutex<Vec<Vec<TelegramUpdate>>>,
    sent_messages: Mutex<Vec<TelegramSendMessageRequest>>,
}

impl TelegramBotPort for FakeTelegramBotPort {
    fn get_updates(&self, _request: &TelegramPollRequest) -> Result<Vec<TelegramUpdate>> {
        if let Some(error) = self
            .poll_errors
            .lock()
            .expect("poll error mutex should lock")
            .pop()
        {
            return Err(error);
        }

        Ok(self
            .updates
            .lock()
            .expect("updates mutex should lock")
            .pop()
            .unwrap_or_default())
    }

    fn send_message(&self, request: &TelegramSendMessageRequest) -> Result<()> {
        self.sent_messages
            .lock()
            .expect("sent messages mutex should lock")
            .push(request.clone());
        Ok(())
    }
}

fn build_runner(allowed_chat_ids: &[i64]) -> (Arc<FakeTelegramBotPort>, TelegramBotRunner) {
    let gateway = Arc::new(FakeTelegramBotPort::default());
    let runner = TelegramBotRunner::new(
        gateway.clone(),
        PlanningControlService::new(Arc::new(FakePlanningControlSurface)),
        TelegramBotPolicy::new(allowed_chat_ids.iter().copied().collect()),
        1,
        false,
        Duration::ZERO,
    );
    (gateway, runner)
}

#[test]
fn parse_message_accepts_plan_status_command_with_bot_mention() {
    let parsed = parse_message(Some("/plan@AkraBot status"));

    assert_eq!(
        parsed,
        TelegramParsedMessage::Command(TelegramInboundCommand::Planning(
            PlanningControlCommand::Status
        ))
    );
}

#[test]
fn parse_message_reports_usage_for_reset_without_target() {
    let parsed = parse_message(Some("/reset"));

    assert_eq!(
        parsed,
        TelegramParsedMessage::Error(
            "사용법: /reset queue | /reset directions | /reset all".to_string()
        )
    );
}

#[test]
fn parse_message_rejects_reset_with_extra_arguments() {
    let parsed = parse_message(Some("/reset queue now"));

    assert_eq!(
        parsed,
        TelegramParsedMessage::Error(
            "사용법: /reset queue | /reset directions | /reset all".to_string()
        )
    );
}

#[test]
fn parse_message_rejects_reset_alias_with_extra_arguments() {
    let parsed = parse_message(Some("/reset_queue now"));

    assert_eq!(
        parsed,
        TelegramParsedMessage::Error("사용법: /reset_queue".to_string())
    );
}

#[test]
fn runner_rejects_unauthorized_chat_with_current_chat_id() {
    let (_gateway, runner) = build_runner(&[]);
    let reply = runner
        .handle_message(&TelegramInboundMessage {
            message_id: 1,
            chat_id: 777,
            text: Some("/status".to_string()),
            sender_display_name: Some("operator".to_string()),
        })
        .expect("handler should succeed");

    let reply = reply.expect("reply should exist");
    assert!(reply.contains("현재 chat_id: 777"));
    assert!(reply.contains("AKRA_TELEGRAM_ALLOWED_CHAT_IDS"));
}

#[test]
fn runner_executes_planning_command_for_allowed_chat() {
    let (_gateway, runner) = build_runner(&[42]);
    let reply = runner
        .handle_message(&TelegramInboundMessage {
            message_id: 1,
            chat_id: 42,
            text: Some("/status".to_string()),
            sender_display_name: Some("operator".to_string()),
        })
        .expect("handler should succeed");

    let reply = reply.expect("reply should exist");
    assert!(reply.contains("상태 요약"));
    assert!(reply.contains("Ship Telegram control"));
}

#[test]
fn help_reply_mentions_whoami_without_allowlist() {
    let (_gateway, runner) = build_runner(&[]);
    let reply = runner
        .handle_message(&TelegramInboundMessage {
            message_id: 1,
            chat_id: 777,
            text: Some("/help".to_string()),
            sender_display_name: Some("operator".to_string()),
        })
        .expect("handler should succeed");

    let reply = reply.expect("reply should exist");
    assert!(reply.contains("/whoami"));
    assert!(reply.contains("/status"));
}

#[test]
fn parse_args_reads_token_and_chat_ids_from_environment_and_flags() {
    let args = parse_args_with_environment(
        [
            "--allow-chat-id".to_string(),
            "12".to_string(),
            "--poll-timeout-seconds".to_string(),
            "45".to_string(),
        ],
        TelegramBotEnvironment {
            token: Some("env-token".to_string()),
            allowed_chat_ids: [10, 11].into_iter().collect(),
        },
    )
    .expect("args should parse");

    assert_eq!(args.token, "env-token");
    assert_eq!(args.allowed_chat_ids.len(), 3);
    assert!(args.allowed_chat_ids.contains(&10));
    assert!(args.allowed_chat_ids.contains(&12));
    assert_eq!(args.poll_timeout_seconds, 45);
    assert!(args.drop_pending_updates);
}

#[test]
fn load_environment_from_sources_merges_config_file_and_process_env() {
    let environment = load_environment_from_sources(
        Some(
            r#"
            AKRA_TELEGRAM_BOT_TOKEN=config-token
            AKRA_TELEGRAM_ALLOWED_CHAT_IDS=10,11
            "#,
        ),
        Some("env-token".to_string()),
        Some("12,13".to_string()),
    )
    .expect("environment should load");

    assert_eq!(environment.token.as_deref(), Some("env-token"));
    assert_eq!(environment.allowed_chat_ids, [12, 13].into_iter().collect());
}

#[test]
fn apply_environment_file_reads_token_and_allowlist() {
    let mut environment = TelegramBotEnvironment::default();

    apply_environment_file(
        &mut environment,
        r#"
        # local bot config
        export AKRA_TELEGRAM_BOT_TOKEN="stored-token"
        AKRA_TELEGRAM_ALLOWED_CHAT_IDS='10,11'
        UNUSED_KEY=ignored
        "#,
    )
    .expect("config file should parse");

    assert_eq!(environment.token.as_deref(), Some("stored-token"));
    assert_eq!(environment.allowed_chat_ids, [10, 11].into_iter().collect());
}

#[test]
fn run_poll_cycle_keeps_loop_alive_after_poll_error() {
    let (gateway, runner) = build_runner(&[42]);
    gateway
        .poll_errors
        .lock()
        .expect("poll error mutex should lock")
        .push(anyhow!("network unavailable"));

    let next_offset = runner.run_poll_cycle(Some(99));

    assert_eq!(next_offset, Some(99));
}

#[test]
fn process_updates_continues_after_individual_message_failure() {
    let gateway = Arc::new(FakeTelegramBotPort::default());
    let runner = TelegramBotRunner::new(
        gateway.clone(),
        PlanningControlService::new(Arc::new(FlakyPlanningControlSurface {
            load_calls: AtomicUsize::new(0),
        })),
        TelegramBotPolicy::new([42].into_iter().collect()),
        1,
        false,
        Duration::ZERO,
    );

    runner.process_updates(&[
        TelegramUpdate {
            update_id: 1,
            message: Some(TelegramInboundMessage {
                message_id: 10,
                chat_id: 42,
                text: Some("/status".to_string()),
                sender_display_name: None,
            }),
        },
        TelegramUpdate {
            update_id: 2,
            message: Some(TelegramInboundMessage {
                message_id: 11,
                chat_id: 42,
                text: Some("/status".to_string()),
                sender_display_name: None,
            }),
        },
    ]);

    let sent_messages = gateway
        .sent_messages
        .lock()
        .expect("sent messages mutex should lock");
    assert_eq!(sent_messages.len(), 2);
    assert!(sent_messages[0].text.contains("명령 처리에 실패했습니다."));
    assert!(sent_messages[1].text.contains("상태 요약"));
}
