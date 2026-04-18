use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};

use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::adapter::outbound::telegram::CurlTelegramBotAdapter;
use crate::application::port::outbound::telegram_bot_port::{
    TelegramBotPort, TelegramInboundMessage, TelegramPollRequest, TelegramSendMessageRequest,
    TelegramUpdate,
};
use crate::application::service::planning::{
    PlanningAdminFacadeService, PlanningControlCommand, PlanningControlService, PlanningResetTarget,
};

const DEFAULT_POLL_TIMEOUT_SECONDS: u16 = 30;
const DEFAULT_POLL_LIMIT: u8 = 100;
const TELEGRAM_BOT_USAGE: &str = "Usage: akra telegram-bot [--token <token>] [--allow-chat-id <chat_id>]... [--poll-timeout-seconds <seconds>] [--keep-pending]\nEnv: AKRA_TELEGRAM_BOT_TOKEN, AKRA_TELEGRAM_ALLOWED_CHAT_IDS=123,456";

#[derive(Debug, Clone)]
struct TelegramBotArgs {
    token: String,
    allowed_chat_ids: BTreeSet<i64>,
    poll_timeout_seconds: u16,
    drop_pending_updates: bool,
}

pub fn run_from_env() -> Result<()> {
    run_with_args(std::env::args().skip(1))
}

pub fn run_with_args<I>(args: I) -> Result<()>
where
    I: IntoIterator<Item = String>,
{
    let args = parse_args(args)?;
    let workspace_dir = std::env::current_dir()
        .context("failed to resolve current directory for telegram bot")?
        .canonicalize()
        .context("failed to canonicalize current directory for telegram bot")?;
    let workspace_dir = workspace_dir.display().to_string();
    let control_service = PlanningControlService::new(Arc::new(PlanningAdminFacadeService::new(
        workspace_dir.clone(),
        Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
    )));
    let runner = TelegramBotRunner::new(
        Arc::new(CurlTelegramBotAdapter::new(args.token)),
        control_service,
        TelegramBotPolicy::new(args.allowed_chat_ids),
        args.poll_timeout_seconds,
        args.drop_pending_updates,
    );
    println!("telegram bot control listening for workspace {workspace_dir}");
    runner.run()
}

fn parse_args<I>(args: I) -> Result<TelegramBotArgs>
where
    I: IntoIterator<Item = String>,
{
    let mut token = std::env::var("AKRA_TELEGRAM_BOT_TOKEN").ok();
    let mut allowed_chat_ids = parse_allowed_chat_ids_env()?;
    let mut poll_timeout_seconds = DEFAULT_POLL_TIMEOUT_SECONDS;
    let mut drop_pending_updates = true;

    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{TELEGRAM_BOT_USAGE}");
                std::process::exit(0);
            }
            "--token" => {
                token = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("missing value for --token"))?,
                );
            }
            "--allow-chat-id" => {
                let raw = args
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --allow-chat-id"))?;
                allowed_chat_ids.insert(parse_chat_id(raw.as_str())?);
            }
            "--poll-timeout-seconds" => {
                let raw = args
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --poll-timeout-seconds"))?;
                poll_timeout_seconds = raw.parse::<u16>().with_context(|| {
                    format!("failed to parse poll timeout seconds from `{raw}`")
                })?;
                if poll_timeout_seconds == 0 {
                    bail!("--poll-timeout-seconds must be greater than zero");
                }
            }
            "--keep-pending" => {
                drop_pending_updates = false;
            }
            unknown => {
                bail!("unsupported telegram-bot argument: {unknown}\n{TELEGRAM_BOT_USAGE}");
            }
        }
    }

    let token = token.ok_or_else(|| {
        anyhow!("telegram bot token is required via --token or AKRA_TELEGRAM_BOT_TOKEN")
    })?;

    Ok(TelegramBotArgs {
        token,
        allowed_chat_ids,
        poll_timeout_seconds,
        drop_pending_updates,
    })
}

fn parse_allowed_chat_ids_env() -> Result<BTreeSet<i64>> {
    let Some(raw) = std::env::var("AKRA_TELEGRAM_ALLOWED_CHAT_IDS").ok() else {
        return Ok(BTreeSet::new());
    };

    let mut values = BTreeSet::new();
    for entry in raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        values.insert(parse_chat_id(entry)?);
    }
    Ok(values)
}

fn parse_chat_id(raw: &str) -> Result<i64> {
    raw.parse::<i64>()
        .with_context(|| format!("failed to parse telegram chat id from `{raw}`"))
}

#[derive(Debug, Clone)]
struct TelegramBotPolicy {
    allowed_chat_ids: BTreeSet<i64>,
}

impl TelegramBotPolicy {
    fn new(allowed_chat_ids: BTreeSet<i64>) -> Self {
        Self { allowed_chat_ids }
    }

    fn is_allowed(&self, chat_id: i64) -> bool {
        self.allowed_chat_ids.contains(&chat_id)
    }

    fn allowlist_is_empty(&self) -> bool {
        self.allowed_chat_ids.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TelegramInboundCommand {
    WhoAmI,
    Planning(PlanningControlCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TelegramParsedMessage {
    Ignore,
    Error(String),
    Command(TelegramInboundCommand),
}

struct TelegramBotRunner {
    gateway: Arc<dyn TelegramBotPort>,
    control_service: PlanningControlService,
    policy: TelegramBotPolicy,
    poll_timeout_seconds: u16,
    drop_pending_updates: bool,
}

impl TelegramBotRunner {
    fn new(
        gateway: Arc<dyn TelegramBotPort>,
        control_service: PlanningControlService,
        policy: TelegramBotPolicy,
        poll_timeout_seconds: u16,
        drop_pending_updates: bool,
    ) -> Self {
        Self {
            gateway,
            control_service,
            policy,
            poll_timeout_seconds,
            drop_pending_updates,
        }
    }

    fn run(&self) -> Result<()> {
        let mut next_offset = if self.drop_pending_updates {
            self.drop_pending_updates()?
        } else {
            None
        };

        loop {
            let updates = self.gateway.get_updates(&TelegramPollRequest::new(
                next_offset,
                self.poll_timeout_seconds,
                DEFAULT_POLL_LIMIT,
            ))?;
            if let Some(last_update) = updates.last() {
                next_offset = Some(last_update.update_id + 1);
            }
            self.process_updates(&updates)?;
        }
    }

    fn drop_pending_updates(&self) -> Result<Option<i64>> {
        let pending_updates =
            self.gateway
                .get_updates(&TelegramPollRequest::new(None, 1, DEFAULT_POLL_LIMIT))?;
        Ok(pending_updates.last().map(|update| update.update_id + 1))
    }

    fn process_updates(&self, updates: &[TelegramUpdate]) -> Result<()> {
        for update in updates {
            let Some(message) = update.message.as_ref() else {
                continue;
            };
            if let Some(reply) = self.handle_message(message)? {
                self.gateway
                    .send_message(&TelegramSendMessageRequest::new(message.chat_id, reply))?;
            }
        }
        Ok(())
    }

    fn handle_message(&self, message: &TelegramInboundMessage) -> Result<Option<String>> {
        let parsed = parse_message(message.text.as_deref());
        match parsed {
            TelegramParsedMessage::Ignore => Ok(None),
            TelegramParsedMessage::Error(error) => Ok(Some(error)),
            TelegramParsedMessage::Command(TelegramInboundCommand::WhoAmI) => {
                Ok(Some(self.render_whoami(message.chat_id)))
            }
            TelegramParsedMessage::Command(TelegramInboundCommand::Planning(command)) => {
                if matches!(command, PlanningControlCommand::Help) {
                    return Ok(Some(self.render_help()));
                }
                if !self.policy.is_allowed(message.chat_id) {
                    return Ok(Some(self.render_unauthorized(message.chat_id)));
                }
                let reply = self.control_service.execute(command)?;
                Ok(Some(reply.text))
            }
        }
    }

    fn render_whoami(&self, chat_id: i64) -> String {
        format!(
            "chat_id: {chat_id}\nallowed: {}\nallowlist_configured: {}",
            if self.policy.is_allowed(chat_id) {
                "yes"
            } else {
                "no"
            },
            if self.policy.allowlist_is_empty() {
                "no"
            } else {
                "yes"
            }
        )
    }

    fn render_unauthorized(&self, chat_id: i64) -> String {
        if self.policy.allowlist_is_empty() {
            format!(
                "허용된 chat_id가 설정되지 않았습니다.\n현재 chat_id: {chat_id}\nAKRA_TELEGRAM_ALLOWED_CHAT_IDS 또는 --allow-chat-id로 등록하세요."
            )
        } else {
            format!(
                "허용되지 않은 chat_id입니다.\n현재 chat_id: {chat_id}\n등록된 allowlist에 이 chat_id를 추가하세요."
            )
        }
    }

    fn render_help(&self) -> String {
        format!("{}\n/whoami", self.control_service.help_text())
    }
}

fn parse_message(text: Option<&str>) -> TelegramParsedMessage {
    let Some(text) = text.map(str::trim).filter(|text| !text.is_empty()) else {
        return TelegramParsedMessage::Ignore;
    };

    let mut parts = text.split_whitespace();
    let Some(raw_command) = parts.next() else {
        return TelegramParsedMessage::Ignore;
    };
    let command = normalize_command(raw_command);
    let arguments = parts.collect::<Vec<_>>();

    match command.as_str() {
        "/start" | "/help" | "help" => TelegramParsedMessage::Command(
            TelegramInboundCommand::Planning(PlanningControlCommand::Help),
        ),
        "/whoami" => TelegramParsedMessage::Command(TelegramInboundCommand::WhoAmI),
        "/status" | "status" => TelegramParsedMessage::Command(TelegramInboundCommand::Planning(
            PlanningControlCommand::Status,
        )),
        "/queue" | "queue" => TelegramParsedMessage::Command(TelegramInboundCommand::Planning(
            PlanningControlCommand::Queue,
        )),
        "/plan_on" => TelegramParsedMessage::Command(TelegramInboundCommand::Planning(
            PlanningControlCommand::EnablePlan,
        )),
        "/plan_off" => TelegramParsedMessage::Command(TelegramInboundCommand::Planning(
            PlanningControlCommand::DisablePlan,
        )),
        "/plan" => parse_plan_arguments(&arguments),
        "/reset_queue" => TelegramParsedMessage::Command(TelegramInboundCommand::Planning(
            PlanningControlCommand::Reset(PlanningResetTarget::Queue),
        )),
        "/reset_directions" => TelegramParsedMessage::Command(TelegramInboundCommand::Planning(
            PlanningControlCommand::Reset(PlanningResetTarget::Directions),
        )),
        "/reset_all" => TelegramParsedMessage::Command(TelegramInboundCommand::Planning(
            PlanningControlCommand::Reset(PlanningResetTarget::All),
        )),
        "/reset" => parse_reset_arguments(&arguments),
        token if token.starts_with('/') => TelegramParsedMessage::Error(format!(
            "지원하지 않는 명령어입니다: {token}\n{}",
            PlanningControlService::new(Arc::new(NoopPlanningControlSurface)).help_text()
        )),
        _ => TelegramParsedMessage::Ignore,
    }
}

fn parse_plan_arguments(arguments: &[&str]) -> TelegramParsedMessage {
    match arguments {
        ["on"] => TelegramParsedMessage::Command(TelegramInboundCommand::Planning(
            PlanningControlCommand::EnablePlan,
        )),
        ["off"] => TelegramParsedMessage::Command(TelegramInboundCommand::Planning(
            PlanningControlCommand::DisablePlan,
        )),
        _ => TelegramParsedMessage::Error("사용법: /plan on | /plan off".to_string()),
    }
}

fn parse_reset_arguments(arguments: &[&str]) -> TelegramParsedMessage {
    let Some(target) = arguments.first().copied() else {
        return TelegramParsedMessage::Error(
            "사용법: /reset queue | /reset directions | /reset all".to_string(),
        );
    };

    let command = match target {
        "queue" => PlanningControlCommand::Reset(PlanningResetTarget::Queue),
        "directions" => PlanningControlCommand::Reset(PlanningResetTarget::Directions),
        "all" => PlanningControlCommand::Reset(PlanningResetTarget::All),
        _ => {
            return TelegramParsedMessage::Error(
                "사용법: /reset queue | /reset directions | /reset all".to_string(),
            );
        }
    };
    TelegramParsedMessage::Command(TelegramInboundCommand::Planning(command))
}

fn normalize_command(raw_command: &str) -> String {
    let lowered = raw_command.to_ascii_lowercase();
    let mut parts = lowered.split('@');
    parts.next().unwrap_or_default().to_string()
}

struct NoopPlanningControlSurface;

impl crate::application::service::planning::control::PlanningControlSurface
    for NoopPlanningControlSurface
{
    fn load_status_snapshot(
        &self,
    ) -> Result<crate::application::service::planning::control::PlanningControlStatusSnapshot> {
        bail!("noop control surface should not execute");
    }

    fn set_plan_enabled(
        &self,
        _enabled: bool,
    ) -> Result<crate::application::service::planning::control::PlanningControlPlanToggleOutcome>
    {
        bail!("noop control surface should not execute");
    }

    fn reset_workspace(
        &self,
        _target: PlanningResetTarget,
    ) -> Result<crate::application::service::planning::control::PlanningControlResetOutcome> {
        bail!("noop control surface should not execute");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use anyhow::Result;

    use super::{
        parse_args, parse_message, TelegramBotPolicy, TelegramBotRunner, TelegramInboundCommand,
        TelegramParsedMessage,
    };
    use crate::application::port::outbound::telegram_bot_port::{
        TelegramBotPort, TelegramInboundMessage, TelegramPollRequest, TelegramSendMessageRequest,
        TelegramUpdate,
    };
    use crate::application::service::planning::control::{
        PlanningControlCommand, PlanningControlPlanToggleOutcome, PlanningControlQueueEntry,
        PlanningControlResetOutcome, PlanningControlService, PlanningControlStatusSnapshot,
        PlanningControlSurface,
    };
    use crate::application::service::planning::PlanningResetTarget;

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
                plan_enabled: true,
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

        fn set_plan_enabled(&self, enabled: bool) -> Result<PlanningControlPlanToggleOutcome> {
            Ok(PlanningControlPlanToggleOutcome {
                enabled,
                planning_state: if enabled {
                    "ready".to_string()
                } else {
                    "plan_disabled".to_string()
                },
                health: Some("planning workspace ready".to_string()),
                issue: None,
            })
        }

        fn reset_workspace(
            &self,
            target: PlanningResetTarget,
        ) -> Result<PlanningControlResetOutcome> {
            Ok(PlanningControlResetOutcome {
                target: target.label().to_string(),
                rewritten_paths: vec![".codex-exec-loop/planning/task-ledger.json".to_string()],
                removed_paths: Vec::new(),
                planning_state: "ready".to_string(),
                health: Some("queue reset complete".to_string()),
                issue: None,
            })
        }
    }

    #[derive(Default)]
    struct FakeTelegramBotPort {
        sent_messages: Mutex<Vec<TelegramSendMessageRequest>>,
    }

    impl TelegramBotPort for FakeTelegramBotPort {
        fn get_updates(&self, _request: &TelegramPollRequest) -> Result<Vec<TelegramUpdate>> {
            Ok(Vec::new())
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
        );
        (gateway, runner)
    }

    #[test]
    fn parse_message_accepts_plan_command_with_bot_mention() {
        let parsed = parse_message(Some("/plan@AkraBot off"));

        assert_eq!(
            parsed,
            TelegramParsedMessage::Command(TelegramInboundCommand::Planning(
                PlanningControlCommand::DisablePlan
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
    fn parse_args_reads_token_and_chat_ids_from_env_and_flags() {
        let _token_guard = EnvGuard::set("AKRA_TELEGRAM_BOT_TOKEN", Some("env-token"));
        let _chat_guard = EnvGuard::set("AKRA_TELEGRAM_ALLOWED_CHAT_IDS", Some("10,11"));

        let args = parse_args([
            "--allow-chat-id".to_string(),
            "12".to_string(),
            "--poll-timeout-seconds".to_string(),
            "45".to_string(),
        ])
        .expect("args should parse");

        assert_eq!(args.token, "env-token");
        assert_eq!(args.allowed_chat_ids.len(), 3);
        assert!(args.allowed_chat_ids.contains(&10));
        assert!(args.allowed_chat_ids.contains(&12));
        assert_eq!(args.poll_timeout_seconds, 45);
        assert!(args.drop_pending_updates);
    }

    struct EnvGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: Option<&str>) -> Self {
            let previous = std::env::var(key).ok();
            unsafe {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match self.previous.as_deref() {
                    Some(value) => std::env::set_var(self.key, value),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }
}
