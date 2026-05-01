use std::collections::BTreeSet;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::adapter::outbound::app_server::{AppServerPlanningWorkerAdapter, CodexAppServerAdapter};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::adapter::outbound::telegram::CurlTelegramBotAdapter;
use crate::application::port::outbound::telegram_bot_port::{
    TelegramBotPort, TelegramInboundMessage, TelegramPollRequest, TelegramSendMessageRequest,
    TelegramUpdate,
};
use crate::application::service::planning::{
    PlanningAdminFacadeService, PlanningControlCommand, PlanningControlService, PlanningServices,
};

const DEFAULT_POLL_TIMEOUT_SECONDS: u16 = 30;
const DEFAULT_POLL_LIMIT: u8 = 100;
const DEFAULT_FAILURE_BACKOFF: Duration = Duration::from_secs(2);
const TELEGRAM_BOT_USAGE: &str = "Usage: akra telegram [--token <token>] [--allow-chat-id <chat_id>]... [--poll-timeout-seconds <seconds>] [--keep-pending]\nAlias: akra telegram-bot [--token <token>] [--allow-chat-id <chat_id>]... [--poll-timeout-seconds <seconds>] [--keep-pending]\nEnv: AKRA_TELEGRAM_BOT_TOKEN, AKRA_TELEGRAM_ALLOWED_CHAT_IDS=123,456\nConfig: $XDG_CONFIG_HOME/akra/telegram.env or ~/.config/akra/telegram.env";

mod config;
mod message;

use self::config::parse_args;
use self::message::{TelegramInboundCommand, TelegramParsedMessage, parse_message};

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
    let control_service =
        PlanningControlService::new(Arc::new(build_planning_admin_facade(workspace_dir.clone())));
    let runner = TelegramBotRunner::new(
        Arc::new(CurlTelegramBotAdapter::new(args.token)),
        control_service,
        TelegramBotPolicy::new(args.allowed_chat_ids),
        args.poll_timeout_seconds,
        args.drop_pending_updates,
        DEFAULT_FAILURE_BACKOFF,
    );
    println!("telegram bot control listening for local workspace {workspace_dir}");
    runner.run()
}

fn build_planning_admin_facade(workspace_dir: String) -> PlanningAdminFacadeService {
    let app_server_adapter = Arc::new(CodexAppServerAdapter::new(
        "codex-exec-loop-native",
        env!("CARGO_PKG_VERSION"),
    ));
    let planning_authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning_workspace_port = Arc::new(
        FilesystemPlanningWorkspaceAdapter::with_repo_scoped_store(planning_authority.clone()),
    );
    let planning = PlanningServices::from_ports(
        planning_workspace_port.clone(),
        planning_authority.clone(),
        planning_authority.clone(),
        Arc::new(AppServerPlanningWorkerAdapter::new(app_server_adapter)),
    );
    PlanningAdminFacadeService::from_planning_with_authority(
        workspace_dir,
        planning,
        planning_workspace_port,
        planning_authority.clone(),
        planning_authority,
    )
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

struct TelegramBotRunner {
    gateway: Arc<dyn TelegramBotPort>,
    control_service: PlanningControlService,
    policy: TelegramBotPolicy,
    poll_timeout_seconds: u16,
    drop_pending_updates: bool,
    failure_backoff: Duration,
}

impl TelegramBotRunner {
    fn new(
        gateway: Arc<dyn TelegramBotPort>,
        control_service: PlanningControlService,
        policy: TelegramBotPolicy,
        poll_timeout_seconds: u16,
        drop_pending_updates: bool,
        failure_backoff: Duration,
    ) -> Self {
        Self {
            gateway,
            control_service,
            policy,
            poll_timeout_seconds,
            drop_pending_updates,
            failure_backoff,
        }
    }

    fn run(&self) -> Result<()> {
        let mut next_offset = self.bootstrap_offset();

        loop {
            next_offset = self.run_poll_cycle(next_offset);
        }
    }

    fn bootstrap_offset(&self) -> Option<i64> {
        if !self.drop_pending_updates {
            return None;
        }

        match self.drop_pending_updates() {
            Ok(next_offset) => next_offset,
            Err(error) => {
                eprintln!("telegram bot failed to drop pending updates: {error:#}");
                self.sleep_backoff();
                None
            }
        }
    }

    fn run_poll_cycle(&self, next_offset: Option<i64>) -> Option<i64> {
        let updates = match self.gateway.get_updates(&TelegramPollRequest::new(
            next_offset,
            self.poll_timeout_seconds,
            DEFAULT_POLL_LIMIT,
        )) {
            Ok(updates) => updates,
            Err(error) => {
                eprintln!("telegram bot failed to poll updates: {error:#}");
                self.sleep_backoff();
                return next_offset;
            }
        };

        let next_offset = updates
            .last()
            .map(|update| update.update_id + 1)
            .or(next_offset);
        self.process_updates(&updates);
        next_offset
    }

    fn drop_pending_updates(&self) -> Result<Option<i64>> {
        let pending_updates =
            self.gateway
                .get_updates(&TelegramPollRequest::new(None, 1, DEFAULT_POLL_LIMIT))?;
        Ok(pending_updates.last().map(|update| update.update_id + 1))
    }

    fn process_updates(&self, updates: &[TelegramUpdate]) {
        for update in updates {
            let Some(message) = update.message.as_ref() else {
                continue;
            };
            let reply = match self.handle_message(message) {
                Ok(reply) => reply,
                Err(error) => {
                    eprintln!(
                        "telegram bot failed to handle message {} from chat {}: {error:#}",
                        message.message_id, message.chat_id
                    );
                    Some(self.render_command_failure(message, &error))
                }
            };
            if let Some(reply) = reply
                && let Err(error) = self
                    .gateway
                    .send_message(&TelegramSendMessageRequest::new(message.chat_id, reply))
            {
                eprintln!(
                    "telegram bot failed to send reply for message {} to chat {}: {error:#}",
                    message.message_id, message.chat_id
                );
            }
        }
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

    fn render_command_failure(
        &self,
        message: &TelegramInboundMessage,
        error: &anyhow::Error,
    ) -> String {
        format!(
            "명령 처리에 실패했습니다.\nchat_id: {}\nmessage_id: {}\nerror: {}",
            message.chat_id, message.message_id, error
        )
    }

    fn sleep_backoff(&self) {
        if !self.failure_backoff.is_zero() {
            thread::sleep(self.failure_backoff);
        }
    }
}

#[cfg(test)]
mod tests;
