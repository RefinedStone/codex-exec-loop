use std::collections::BTreeSet;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::adapter::outbound::app_server::{AppServerPlanningWorkerAdapter, CodexAppServerAdapter};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::adapter::outbound::github::GithubAutomationAdapter;
use crate::adapter::outbound::telegram::CurlTelegramBotAdapter;
use crate::application::port::outbound::github_automation_port::GithubAutomationPort;
use crate::application::port::outbound::parallel_agent_worker_port::ParallelAgentWorkerPort;
use crate::application::port::outbound::parallel_mode_runtime_event_log_port::ParallelModeRuntimeEventLogRequest;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::telegram_bot_port::{
    TelegramBotPort, TelegramInboundMessage, TelegramPollRequest, TelegramSendMessageRequest,
    TelegramUpdate,
};
use crate::application::service::parallel_mode::{
    ParallelModeService, control_plane::ParallelModeControlPlaneComposition,
};
use crate::application::service::planning::{
    PlanningControlCommand, PlanningControlFacadeService, PlanningControlRequest,
    PlanningControlService, PlanningServices,
};

/*
이 module은 Telegram을 "또 하나의 shell"로 붙이는 inbound adapter다. Telegram Bot API
polling과 응답 전송은 `TelegramBotPort` 뒤에 숨기고, 실제 planning 조작은 CLI/TUI가 쓰는
`PlanningControlService`로 넘긴다. 그래서 여기의 핵심 책임은 bootstrapping, chat allowlist,
update cursor 관리, 메시지 단위 장애 격리다.
*/
const DEFAULT_POLL_TIMEOUT_SECONDS: u16 = 30;
// Telegram getUpdates의 limit 상한에 맞춘 batch 크기다. runner는 batch 전체를 처리한 뒤 offset을 전진시킨다.
const DEFAULT_POLL_LIMIT: u8 = 100;
// 네트워크 장애나 startup discard 실패 때 tight loop로 Telegram API를 때리지 않기 위한 최소 backoff다.
const DEFAULT_FAILURE_BACKOFF: Duration = Duration::from_secs(2);
const TELEGRAM_BOT_USAGE: &str = "Usage: akra telegram [--token <token>] [--allow-chat-id <chat_id>]... [--poll-timeout-seconds <seconds>] [--keep-pending]\nAlias: akra telegram-bot [--token <token>] [--allow-chat-id <chat_id>]... [--poll-timeout-seconds <seconds>] [--keep-pending]\nEnv: AKRA_TELEGRAM_BOT_TOKEN, AKRA_TELEGRAM_ALLOWED_CHAT_IDS=123,456\nConfig: $XDG_CONFIG_HOME/akra/telegram.env or ~/.config/akra/telegram.env";

// config는 secrets/env/CLI parsing만 맡고, message는 Telegram text를 planning control command로 축소한다.
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
    // Args parsing is deliberately outside the runner so tests can inject a fake gateway and service.
    let args = parse_args(args)?;
    /*
    Telegram bot은 현재 작업 디렉터리의 planning workspace를 원격 채팅에서 조작한다.
    canonical path를 facade에 넘겨 CLI/TUI와 같은 repo-scoped authority와 draft store를 보게 한다.
    */
    let workspace_dir = std::env::current_dir()
        .context("failed to resolve current directory for telegram bot")?
        .canonicalize()
        .context("failed to canonicalize current directory for telegram bot")?;
    let workspace_dir = workspace_dir.display().to_string();
    let application = build_telegram_application(workspace_dir.clone());

    // Production wiring: Telegram HTTP adapter + planning control service + local allowlist policy.
    let runner = TelegramBotRunner::new(
        Arc::new(CurlTelegramBotAdapter::new(args.token)),
        application.control_service,
        application.parallel_control_surface,
        TelegramBotPolicy::new(args.allowed_chat_ids),
        args.poll_timeout_seconds,
        args.drop_pending_updates,
        DEFAULT_FAILURE_BACKOFF,
    );
    println!("telegram bot control listening for local workspace {workspace_dir}");
    runner.run()
}

struct TelegramApplication {
    control_service: PlanningControlService,
    parallel_control_surface: Arc<dyn TelegramParallelControlSurface>,
}

fn build_telegram_application(workspace_dir: String) -> TelegramApplication {
    /*
    Telegram commands use the same application control facades as the CLI/admin/TUI command surfaces.
    Planning status/reset goes through PlanningControlService, while read-only parallel status uses
    ParallelModeControlPlaneComposition instead of reaching around to ParallelModeService.
    */
    let app_server_adapter = Arc::new(CodexAppServerAdapter::new(
        "codex-exec-loop-native",
        env!("CARGO_PKG_VERSION"),
    ));
    let planning_authority_adapter = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let planning_authority: Arc<dyn PlanningAuthorityPort> = planning_authority_adapter.clone();
    let planning_workspace_port =
        Arc::new(FilesystemPlanningWorkspaceAdapter::with_repo_scoped_store(
            planning_authority_adapter.clone(),
        ));
    let planning = PlanningServices::from_ports(
        planning_workspace_port.clone(),
        planning_authority.clone(),
        planning_authority_adapter,
        Arc::new(AppServerPlanningWorkerAdapter::new(
            app_server_adapter.clone(),
        )),
    );
    let parallel_agent_worker_port: Arc<dyn ParallelAgentWorkerPort> = app_server_adapter;
    let github_automation: Arc<dyn GithubAutomationPort> = Arc::new(GithubAutomationAdapter::new());
    let parallel_mode_service = ParallelModeService::new(
        planning_authority,
        github_automation,
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    );
    let parallel_control_plane = Arc::new(ParallelModeControlPlaneComposition::new(
        parallel_mode_service,
        planning.clone(),
        parallel_agent_worker_port,
    ));
    TelegramApplication {
        control_service: PlanningControlService::new(Arc::new(PlanningControlFacadeService::new(
            workspace_dir.clone(),
            planning,
        ))),
        parallel_control_surface: Arc::new(TelegramParallelControlPlaneSurface {
            workspace_dir,
            control_plane: parallel_control_plane,
        }),
    }
}

#[derive(Debug, Clone)]
struct TelegramBotPolicy {
    // Empty means no operator has configured remote control yet; planning commands are denied in that state.
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

trait TelegramParallelControlSurface: Send + Sync {
    fn render_parallel_status(&self) -> Result<String>;
}

struct TelegramParallelControlPlaneSurface {
    workspace_dir: String,
    control_plane: Arc<ParallelModeControlPlaneComposition>,
}

impl TelegramParallelControlSurface for TelegramParallelControlPlaneSurface {
    fn render_parallel_status(&self) -> Result<String> {
        let snapshot = self.control_plane.inspect_dashboard_snapshot(
            &self.workspace_dir,
            ParallelModeRuntimeEventLogRequest::recent(5),
        );
        Ok(format!(
            "병렬 상태\nreadiness: {}\npool: {}\nactive_agents: {}\nqueue_depth: {}\nevents: {}",
            snapshot.readiness.readiness_label(),
            snapshot.supervisor.pool.reconcile_status,
            snapshot.supervisor.roster.active_count(),
            snapshot.supervisor.distributor.queue_depth(),
            snapshot.events.visible_count(),
        ))
    }
}

/*
`TelegramBotRunner` is the long-running orchestration loop. It owns no planning domain
logic: it polls updates, checks Telegram-specific authorization, delegates command execution,
and always tries to keep polling after per-message failures.
*/
struct TelegramBotRunner {
    // Transport boundary: real HTTP in production, fake port in tests.
    gateway: Arc<dyn TelegramBotPort>,
    // Application boundary shared with non-Telegram control surfaces.
    control_service: PlanningControlService,
    parallel_control_surface: Arc<dyn TelegramParallelControlSurface>,
    policy: TelegramBotPolicy,
    // Long polling timeout is configurable because Telegram HTTP infrastructure decides practical latency.
    poll_timeout_seconds: u16,
    // Startup cursor policy. When true, old chat history is skipped before live control starts.
    drop_pending_updates: bool,
    failure_backoff: Duration,
}

impl TelegramBotRunner {
    fn new(
        gateway: Arc<dyn TelegramBotPort>,
        control_service: PlanningControlService,
        parallel_control_surface: Arc<dyn TelegramParallelControlSurface>,
        policy: TelegramBotPolicy,
        poll_timeout_seconds: u16,
        drop_pending_updates: bool,
        failure_backoff: Duration,
    ) -> Self {
        Self {
            gateway,
            control_service,
            parallel_control_surface,
            policy,
            poll_timeout_seconds,
            drop_pending_updates,
            failure_backoff,
        }
    }

    fn run(&self) -> Result<()> {
        /*
        Telegram update offset is the only loop state. Keeping it outside the gateway makes retry
        behavior explicit: poll failures keep the old cursor, successful batches advance past the
        last update whether individual messages inside the batch succeed or fail.
        */
        let mut next_offset = self.bootstrap_offset();
        loop {
            next_offset = self.run_poll_cycle(next_offset);
        }
    }

    fn bootstrap_offset(&self) -> Option<i64> {
        // Default startup behavior drops stale commands so enabling the bot cannot replay old chat history.
        if !self.drop_pending_updates {
            return None;
        }

        // A discard failure should not prevent the bot from coming up; it only loses the initial skip optimization.
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
        // Cursor, timeout, and batch size are bundled into the outbound port request so HTTP mapping stays adapter-local.
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

        /*
        Offset advances by Telegram update_id, not message_id. Advancing after the batch prevents
        poison messages from being redelivered forever; message-level failures are answered inline.
        */
        let next_offset = updates
            .last()
            .map(|update| update.update_id + 1)
            .or(next_offset);
        self.process_updates(&updates);
        next_offset
    }

    fn drop_pending_updates(&self) -> Result<Option<i64>> {
        // One short poll is enough to discover the latest update_id and start live polling after it.
        let pending_updates =
            self.gateway
                .get_updates(&TelegramPollRequest::new(None, 1, DEFAULT_POLL_LIMIT))?;
        Ok(pending_updates.last().map(|update| update.update_id + 1))
    }

    fn process_updates(&self, updates: &[TelegramUpdate]) {
        for update in updates {
            // Non-message updates do not participate in the planning command surface.
            let Some(message) = update.message.as_ref() else {
                continue;
            };

            /*
            Each Telegram message is isolated. A failed planning service call becomes a reply for
            that chat while later updates in the same batch still execute.
            */
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

            // Send failures are logged but do not rewind the update offset; otherwise one bad chat would stall polling.
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
        /*
        Parsing is pure and Telegram-specific. Authorization intentionally happens after parsing so
        `/help` and syntax errors remain reachable even before an operator has configured allowlist.
        */
        let parsed = parse_message(message.text.as_deref());
        match parsed {
            TelegramParsedMessage::Ignore => Ok(None),
            TelegramParsedMessage::Error(error) => Ok(Some(error)),
            TelegramParsedMessage::Command(TelegramInboundCommand::WhoAmI) => {
                Ok(Some(self.render_whoami(message.chat_id)))
            }
            TelegramParsedMessage::Command(TelegramInboundCommand::ParallelStatus) => {
                if !self.policy.is_allowed(message.chat_id) {
                    return Ok(Some(self.render_unauthorized(message.chat_id)));
                }
                Ok(Some(
                    self.parallel_control_surface.render_parallel_status()?,
                ))
            }
            TelegramParsedMessage::Command(TelegramInboundCommand::Planning(command)) => {
                // Help is safe without allowlist because it only describes commands and includes `/whoami`.
                if matches!(command, PlanningControlCommand::Help) {
                    return Ok(Some(self.render_help()));
                }
                // Every state-changing or state-reading planning command requires explicit chat authorization.
                if !self.policy.is_allowed(message.chat_id) {
                    return Ok(Some(self.render_unauthorized(message.chat_id)));
                }

                // From this point on, Telegram is just another adapter calling the planning control service.
                let response = self
                    .control_service
                    .execute_request(PlanningControlRequest::new(command))?;
                Ok(Some(response.reply.text))
            }
        }
    }

    fn render_whoami(&self, chat_id: i64) -> String {
        // Operators need both the current chat id and whether any allowlist was configured to repair access remotely.
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
        // Empty allowlist is a distinct setup error; non-empty allowlist means this chat must be added.
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
        // `/whoami` lives in this adapter, so append it to the shared planning control help text.
        format!("{}\n/parallel\n/whoami", self.control_service.help_text())
    }

    fn render_command_failure(
        &self,
        message: &TelegramInboundMessage,
        error: &anyhow::Error,
    ) -> String {
        // Include Telegram ids in the user-facing failure so logs and chat screenshots can be correlated.
        format!(
            "명령 처리에 실패했습니다.\nchat_id: {}\nmessage_id: {}\nerror: {}",
            message.chat_id, message.message_id, error
        )
    }

    fn sleep_backoff(&self) {
        // Tests pass Duration::ZERO to keep retry-path assertions fast and deterministic.
        if !self.failure_backoff.is_zero() {
            thread::sleep(self.failure_backoff);
        }
    }
}

#[cfg(test)]
mod tests;
