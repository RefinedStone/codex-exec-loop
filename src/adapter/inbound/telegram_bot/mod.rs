use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use rand::RngCore;

use crate::application::port::inbound::planning_control_port::{
    PlanningControlCommand, PlanningControlPort, PlanningControlRequest,
};
use crate::application::port::outbound::parallel_mode_runtime_event_log_port::ParallelModeRuntimeEventLogRequest;
use crate::application::port::outbound::telegram_bot_port::{
    TELEGRAM_LONG_POLL_TRANSPORT_MARGIN_SECONDS, TelegramBotPort, TelegramInboundMessage,
    TelegramPollRequest, TelegramSendMessageRequest, TelegramUpdate,
};
use crate::application::port::outbound::telegram_global_runner_lease_port::{
    TelegramGlobalRunnerLeaseClaimDecision, TelegramGlobalRunnerLeasePort,
    TelegramWorkspaceBindingPolicy,
};
use crate::application::port::outbound::telegram_update_ledger_port::{
    TelegramUpdateClaimDecision, TelegramUpdateLedgerPort,
};
use crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneComposition;
use crate::application::service::review_center::ReviewCenterReadService;
use crate::composition::production;

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
// Lease renewal surrounds every blocking poll and update. Five minutes leaves ample headroom over
// Telegram's poll timeout plus curl's transport margin while still allowing crash recovery.
const MIN_RUNNER_LEASE_TTL_SECONDS: u64 = 300;
const RUNNER_LEASE_POLL_MULTIPLIER: u64 = 2;
const RUNNER_LEASE_TRANSPORT_MARGIN_SECONDS: u64 = 60;
const MAX_POLL_TIMEOUT_SECONDS: u16 = 50;

// config는 secrets/env/CLI parsing만 맡고, message는 Telegram text를 planning control command로 축소한다.
mod config;
mod copy;
mod message;

use self::config::parse_args;
pub(crate) use self::copy::{TELEGRAM_BOT_ALIAS_USAGE, TELEGRAM_BOT_COMMAND_USAGE};
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
    let shutdown = crate::shutdown::GracefulShutdown::install()?;
    if shutdown.is_requested() {
        return Ok(());
    }
    /*
    Telegram bot은 현재 작업 디렉터리의 planning workspace를 원격 채팅에서 조작한다.
    canonical path를 facade에 넘겨 CLI/TUI와 같은 repo-scoped authority와 draft store를 보게 한다.
    */
    let workspace_dir = std::env::current_dir()
        .context("failed to resolve current directory for telegram bot")?
        .canonicalize()
        .context("failed to canonicalize current directory for telegram bot")?;
    let workspace_dir = workspace_dir
        .into_os_string()
        .into_string()
        .map_err(|_| anyhow::anyhow!("canonical Telegram workspace path must be valid UTF-8"))?;
    let binding_workspace_dir = production::resolve_active_planning_workspace_root(&workspace_dir)
        .into_os_string()
        .into_string()
        .map_err(|_| anyhow::anyhow!("canonical Telegram repository path must be valid UTF-8"))?;
    let stream_key = telegram_stream_key(&args.token)?;
    let application = build_telegram_application(workspace_dir.clone());

    // Production wiring: Telegram HTTP adapter + planning control service + local allowlist policy.
    let runner = TelegramBotRunner::new(
        production::build_telegram_bot_port(args.token),
        application.telegram_global_runner_lease_port,
        application.telegram_update_ledger_port,
        application.control_service,
        application.parallel_control_surface,
        TelegramBotRuntimeConfig {
            workspace_dir: workspace_dir.clone(),
            binding_workspace_dir,
            stream_key,
            policy: TelegramBotPolicy::new(args.allowed_chat_ids, args.allowed_user_ids),
            poll_timeout_seconds: args.poll_timeout_seconds,
            drop_pending_updates: args.drop_pending_updates,
            binding_policy: if args.rebind_workspace {
                TelegramWorkspaceBindingPolicy::RebindInactive
            } else {
                TelegramWorkspaceBindingPolicy::Preserve
            },
            failure_backoff: DEFAULT_FAILURE_BACKOFF,
        },
    )
    .with_review_center_read_service(application.review_center_read_service);
    runner.run(&shutdown)
}

struct TelegramApplication {
    control_service: Arc<dyn PlanningControlPort>,
    parallel_control_surface: Arc<dyn TelegramParallelControlSurface>,
    review_center_read_service: ReviewCenterReadService,
    telegram_update_ledger_port: Arc<dyn TelegramUpdateLedgerPort>,
    telegram_global_runner_lease_port: Arc<dyn TelegramGlobalRunnerLeasePort>,
}

fn build_telegram_application(workspace_dir: String) -> TelegramApplication {
    /*
    Telegram commands use the same application control facades as the CLI/admin/TUI command surfaces.
    Planning status/reset goes through PlanningControlService, while read-only parallel status uses
    ParallelModeControlPlaneComposition instead of reaching around to ParallelModeService.
    */
    let application = production::build_telegram_application(workspace_dir.clone());
    TelegramApplication {
        control_service: application.control_service,
        parallel_control_surface: Arc::new(TelegramParallelControlPlaneSurface {
            workspace_dir,
            control_plane: application.parallel_mode_control_plane,
        }),
        review_center_read_service: application.review_center_read_service,
        telegram_update_ledger_port: application.telegram_update_ledger_port,
        telegram_global_runner_lease_port: application.telegram_global_runner_lease_port,
    }
}

#[derive(Debug, Clone)]
struct TelegramBotPolicy {
    // Empty means no operator has configured remote control yet; planning commands are denied in that state.
    allowed_chat_ids: BTreeSet<i64>,
    allowed_user_ids: BTreeSet<i64>,
}

impl TelegramBotPolicy {
    fn new(allowed_chat_ids: BTreeSet<i64>, allowed_user_ids: BTreeSet<i64>) -> Self {
        Self {
            allowed_chat_ids,
            allowed_user_ids,
        }
    }
    fn is_allowed(&self, message: &TelegramInboundMessage) -> bool {
        if !self.chat_is_allowed(message.chat_id) {
            return false;
        }
        // Telegram private chat ids are positive and retain the historical
        // chat-only allowlist contract. Group/supergroup ids are negative and need
        // a stable sender account as a second authorization factor.
        message.chat_id >= 0 || self.user_is_allowed(message.sender_user_id)
    }
    fn chat_is_allowed(&self, chat_id: i64) -> bool {
        self.allowed_chat_ids.contains(&chat_id)
    }
    fn user_is_allowed(&self, user_id: Option<i64>) -> bool {
        user_id.is_some_and(|user_id| self.allowed_user_ids.contains(&user_id))
    }
    fn allowlist_is_empty(&self) -> bool {
        self.allowed_chat_ids.is_empty()
    }
    fn user_allowlist_is_empty(&self) -> bool {
        self.allowed_user_ids.is_empty()
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
    global_runner_lease: Arc<dyn TelegramGlobalRunnerLeasePort>,
    update_ledger: Arc<dyn TelegramUpdateLedgerPort>,
    workspace_dir: String,
    binding_workspace_dir: String,
    stream_key: String,
    // Application boundary shared with non-Telegram control surfaces.
    control_service: Arc<dyn PlanningControlPort>,
    parallel_control_surface: Arc<dyn TelegramParallelControlSurface>,
    review_center_read_service: Option<ReviewCenterReadService>,
    policy: TelegramBotPolicy,
    // Long polling timeout is configurable because Telegram HTTP infrastructure decides practical latency.
    poll_timeout_seconds: u16,
    // Startup cursor policy. When true, old chat history is skipped before live control starts.
    drop_pending_updates: bool,
    failure_backoff: Duration,
    lease_owner_token: String,
    lease_generation: AtomicU64,
    lease_ttl_seconds: u64,
    binding_policy: TelegramWorkspaceBindingPolicy,
}

struct TelegramBotRuntimeConfig {
    workspace_dir: String,
    binding_workspace_dir: String,
    stream_key: String,
    policy: TelegramBotPolicy,
    poll_timeout_seconds: u16,
    drop_pending_updates: bool,
    binding_policy: TelegramWorkspaceBindingPolicy,
    failure_backoff: Duration,
}

impl TelegramBotRunner {
    fn new(
        gateway: Arc<dyn TelegramBotPort>,
        global_runner_lease: Arc<dyn TelegramGlobalRunnerLeasePort>,
        update_ledger: Arc<dyn TelegramUpdateLedgerPort>,
        control_service: impl PlanningControlPort + 'static,
        parallel_control_surface: Arc<dyn TelegramParallelControlSurface>,
        config: TelegramBotRuntimeConfig,
    ) -> Self {
        Self {
            gateway,
            global_runner_lease,
            update_ledger,
            workspace_dir: config.workspace_dir,
            binding_workspace_dir: config.binding_workspace_dir,
            stream_key: config.stream_key,
            control_service: Arc::new(control_service),
            parallel_control_surface,
            review_center_read_service: None,
            policy: config.policy,
            poll_timeout_seconds: config.poll_timeout_seconds,
            drop_pending_updates: config.drop_pending_updates,
            failure_backoff: config.failure_backoff,
            lease_owner_token: telegram_runner_owner_token(),
            lease_generation: AtomicU64::new(0),
            lease_ttl_seconds: telegram_runner_lease_ttl_seconds(config.poll_timeout_seconds),
            binding_policy: config.binding_policy,
        }
    }

    fn with_review_center_read_service(
        mut self,
        review_center_read_service: ReviewCenterReadService,
    ) -> Self {
        self.review_center_read_service = Some(review_center_read_service);
        self
    }

    fn run(&self, shutdown: &crate::shutdown::GracefulShutdown) -> Result<()> {
        if shutdown.is_requested() {
            return Ok(());
        }
        self.authenticate_bot_identity()?;
        self.acquire_runner_lease()?;
        println!(
            "telegram bot control listening for workspace {} (stream binding {})",
            self.workspace_dir, self.binding_workspace_dir
        );
        let result = self.run_while_lease_owned(shutdown);
        if let Err(error) = self.global_runner_lease.release_global_runner_lease(
            &self.binding_workspace_dir,
            &self.stream_key,
            &self.lease_owner_token,
            std::process::id(),
            self.lease_generation.load(Ordering::Acquire),
        ) {
            eprintln!("telegram bot failed to release its global runner lease: {error:#}");
        }
        result
    }

    fn acquire_runner_lease(&self) -> Result<()> {
        match self
            .global_runner_lease
            .try_acquire_global_runner_lease(
                &self.binding_workspace_dir,
                &self.stream_key,
                &self.lease_owner_token,
                std::process::id(),
                self.lease_ttl_seconds,
                self.binding_policy,
            )
            .context("failed to acquire the machine-wide Telegram runner lease")?
        {
            TelegramGlobalRunnerLeaseClaimDecision::Acquired { generation } => {
                self.lease_generation.store(generation, Ordering::Release);
            }
            TelegramGlobalRunnerLeaseClaimDecision::ActiveRunner {
                bound_workspace_dir,
            } => {
                anyhow::bail!(
                    "another Telegram bot runner already owns this bot stream for canonical workspace `{bound_workspace_dir}`"
                );
            }
            TelegramGlobalRunnerLeaseClaimDecision::WorkspaceBindingMismatch {
                bound_workspace_dir,
            } => {
                anyhow::bail!(
                    "this Telegram bot stream is bound to canonical workspace `{bound_workspace_dir}`; rerun from `{}` with --rebind-workspace only after confirming the old runner is stopped",
                    self.binding_workspace_dir
                );
            }
        }

        Ok(())
    }

    fn authenticate_bot_identity(&self) -> Result<()> {
        let expected = telegram_stream_bot_id(&self.stream_key)?;
        let actual = self
            .gateway
            .get_me()
            .context("failed to authenticate Telegram bot token before binding")?;
        if actual.bot_id != expected {
            anyhow::bail!(
                "Telegram getMe bot id did not match the configured token prefix; refusing to mutate the workspace binding"
            );
        }
        Ok(())
    }

    fn run_while_lease_owned(&self, shutdown: &crate::shutdown::GracefulShutdown) -> Result<()> {
        /*
        Telegram update offset is the only loop state. Keeping it outside the gateway makes retry
        behavior explicit: poll failures keep the old cursor, successful batches advance past the
        last update whether individual messages inside the batch succeed or fail.
        */
        let mut next_offset = self.bootstrap_offset()?;
        while !shutdown.is_requested() {
            next_offset = self.run_poll_cycle_with_shutdown(next_offset, Some(shutdown))?;
        }
        Ok(())
    }

    fn bootstrap_offset(&self) -> Result<Option<i64>> {
        self.renew_runner_lease("before cursor recovery")?;
        let durable_offset = self
            .update_ledger
            .load_cursor_and_recover_inflight(
                &self.binding_workspace_dir,
                &self.stream_key,
                &self.lease_owner_token,
            )
            .context("failed to recover durable Telegram update cursor")?;
        // Default startup behavior drops stale commands so enabling the bot cannot replay old chat history.
        if !self.drop_pending_updates {
            return Ok(durable_offset);
        }

        // Discard is a replay-safety boundary, not an optimization. Starting from
        // offset=None after a discard failure could execute stale destructive commands.
        self.drop_pending_updates(durable_offset)
            .context("telegram bot could not safely discard pending updates")
    }

    #[cfg(test)]
    fn run_poll_cycle(&self, next_offset: Option<i64>) -> Result<Option<i64>> {
        self.run_poll_cycle_with_shutdown(next_offset, None)
    }

    fn run_poll_cycle_with_shutdown(
        &self,
        next_offset: Option<i64>,
        shutdown: Option<&crate::shutdown::GracefulShutdown>,
    ) -> Result<Option<i64>> {
        self.renew_runner_lease("before polling")?;
        let durable_offset = self
            .update_ledger
            .advance_cursor(
                &self.binding_workspace_dir,
                &self.stream_key,
                &self.lease_owner_token,
                next_offset,
            )
            .context("failed to persist Telegram cursor before polling")?;
        // Cursor, timeout, and batch size are bundled into the outbound port request so HTTP mapping stays adapter-local.
        let poll_result = self.gateway.get_updates(&TelegramPollRequest::new(
            durable_offset,
            self.poll_timeout_seconds,
            DEFAULT_POLL_LIMIT,
        ));
        self.renew_runner_lease("after polling")?;
        let updates = match poll_result {
            Ok(updates) => updates,
            Err(error) => {
                eprintln!("telegram bot failed to poll updates: {error:#}");
                if shutdown.is_none_or(|shutdown| !shutdown.is_requested()) {
                    self.sleep_backoff();
                }
                return Ok(durable_offset);
            }
        };
        if shutdown.is_some_and(crate::shutdown::GracefulShutdown::is_requested) {
            return Ok(durable_offset);
        }

        /*
        Offset advances by Telegram update_id, not message_id. Advancing after the batch prevents
        poison messages from being redelivered forever; message-level failures are answered inline.
        */
        // Compute a durable cursor before executing any command. If an invalid
        // update id cannot be advanced, returning an error prevents replaying the
        // same already-executed batch forever.
        validate_update_batch(&updates)?;
        self.process_updates(&updates, durable_offset)
    }

    fn drop_pending_updates(&self, current_offset: Option<i64>) -> Result<Option<i64>> {
        self.renew_runner_lease("before discarding pending updates")?;
        // Telegram's negative offset contract returns the newest pending update
        // and forgets all earlier updates. A normal offset=None request only
        // returns the oldest page and can replay stale commands when backlog > 100.
        let pending_updates =
            self.gateway
                .get_updates(&TelegramPollRequest::new(Some(-1), 0, 1))?;
        self.renew_runner_lease("after discarding pending updates")?;
        validate_update_batch(&pending_updates)?;
        let candidate = next_update_offset(&pending_updates, current_offset)?;
        self.update_ledger
            .advance_cursor(
                &self.binding_workspace_dir,
                &self.stream_key,
                &self.lease_owner_token,
                candidate,
            )
            .context("failed to persist discarded Telegram update cursor")
    }

    fn process_updates(
        &self,
        updates: &[TelegramUpdate],
        mut next_offset: Option<i64>,
    ) -> Result<Option<i64>> {
        let mut ordered_updates = updates.iter().collect::<Vec<_>>();
        ordered_updates.sort_by_key(|update| update.update_id);
        for update in ordered_updates {
            self.renew_runner_lease("before processing an update")?;
            match self
                .update_ledger
                .claim_update(
                    &self.binding_workspace_dir,
                    &self.stream_key,
                    &self.lease_owner_token,
                    update.update_id,
                )
                .with_context(|| {
                    format!(
                        "failed to persist Telegram update {} before execution",
                        update.update_id
                    )
                })? {
                TelegramUpdateClaimDecision::Execute => {}
                TelegramUpdateClaimDecision::AlreadyCompleted => {
                    self.renew_runner_lease("after skipping a completed update")?;
                    continue;
                }
                TelegramUpdateClaimDecision::InFlight => {
                    anyhow::bail!(
                        "Telegram update {} is already executing; refusing duplicate side effects",
                        update.update_id
                    );
                }
            }

            self.process_update(update);
            self.renew_runner_lease("after executing an update")?;
            next_offset = self
                .update_ledger
                .complete_update(
                    &self.binding_workspace_dir,
                    &self.stream_key,
                    &self.lease_owner_token,
                    update.update_id,
                )
                .with_context(|| {
                    format!(
                        "failed to durably complete Telegram update {}",
                        update.update_id
                    )
                })?;
            self.renew_runner_lease("after completing an update")?;
        }
        Ok(next_offset)
    }

    fn process_update(&self, update: &TelegramUpdate) {
        // Non-message updates do not participate in the planning command surface.
        let Some(message) = update.message.as_ref() else {
            return;
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
                Ok(Some(self.render_whoami(message)))
            }
            TelegramParsedMessage::Command(TelegramInboundCommand::ParallelStatus) => {
                if !self.policy.is_allowed(message) {
                    return Ok(Some(self.render_unauthorized(message)));
                }
                Ok(Some(
                    self.parallel_control_surface.render_parallel_status()?,
                ))
            }
            TelegramParsedMessage::Command(TelegramInboundCommand::Reviews) => {
                if !self.policy.is_allowed(message) {
                    return Ok(Some(self.render_unauthorized(message)));
                }
                Ok(Some(self.render_reviews_summary()?))
            }
            TelegramParsedMessage::Command(TelegramInboundCommand::Planning(command)) => {
                // Help is safe without allowlist because it only describes commands and includes `/whoami`.
                if matches!(command, PlanningControlCommand::Help) {
                    return Ok(Some(self.render_help()));
                }
                // Every state-changing or state-reading planning command requires explicit chat authorization.
                if !self.policy.is_allowed(message) {
                    return Ok(Some(self.render_unauthorized(message)));
                }

                // From this point on, Telegram is just another adapter calling the planning control service.
                let response = self
                    .control_service
                    .execute_request(PlanningControlRequest::new(command))?;
                Ok(Some(response.reply.text))
            }
        }
    }

    fn render_whoami(&self, message: &TelegramInboundMessage) -> String {
        let user_id = message
            .sender_user_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unavailable".to_string());
        format!(
            "chat_id: {}\nuser_id: {user_id}\ncontrol_allowed: {}\nchat_allowed: {}\nuser_allowed: {}\nchat_allowlist_configured: {}\nuser_allowlist_configured: {}",
            message.chat_id,
            if self.policy.is_allowed(message) {
                "yes"
            } else {
                "no"
            },
            if self.policy.chat_is_allowed(message.chat_id) {
                "yes"
            } else {
                "no"
            },
            if self.policy.user_is_allowed(message.sender_user_id) {
                "yes"
            } else {
                "no"
            },
            if self.policy.allowlist_is_empty() {
                "no"
            } else {
                "yes"
            },
            if self.policy.user_allowlist_is_empty() {
                "no"
            } else {
                "yes"
            }
        )
    }

    fn render_unauthorized(&self, message: &TelegramInboundMessage) -> String {
        let chat_id = message.chat_id;
        let user_id = message
            .sender_user_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "확인 불가".to_string());
        // Empty allowlist is a distinct setup error; non-empty allowlist means this chat must be added.
        if self.policy.allowlist_is_empty() {
            format!(
                "허용된 chat_id가 설정되지 않았습니다.\n현재 chat_id: {chat_id}\n현재 user_id: {user_id}\nAKRA_TELEGRAM_ALLOWED_CHAT_IDS 또는 --allow-chat-id로 등록하세요."
            )
        } else if !self.policy.chat_is_allowed(chat_id) {
            format!(
                "허용되지 않은 chat_id입니다.\n현재 chat_id: {chat_id}\n현재 user_id: {user_id}\n등록된 chat allowlist에 이 chat_id를 추가하세요."
            )
        } else if chat_id < 0 && !self.policy.user_is_allowed(message.sender_user_id) {
            format!(
                "그룹 제어에는 허용된 발신자 user_id가 필요합니다.\n현재 chat_id: {chat_id}\n현재 user_id: {user_id}\nAKRA_TELEGRAM_ALLOWED_USER_IDS 또는 --allow-user-id로 등록하세요."
            )
        } else {
            "Telegram 제어 권한을 확인할 수 없습니다.".to_string()
        }
    }

    fn render_help(&self) -> String {
        // `/whoami` lives in this adapter, so append it to the shared planning control help text.
        format!(
            "{}\n/parallel\n/reviews\n/whoami",
            self.control_service.help_text()
        )
    }

    fn render_reviews_summary(&self) -> Result<String> {
        let review_center_read_service = self
            .review_center_read_service
            .as_ref()
            .context("telegram review center read service is not configured")?;
        let inbox = review_center_read_service.load_pending_inbox()?;
        let current_thread_id = inbox.first().map(|item| item.thread_id.clone());
        let current_thread_reviews = match current_thread_id.as_deref() {
            Some(thread_id) => review_center_read_service.load_thread_reviews(thread_id)?,
            None => Vec::new(),
        };
        let history = review_center_read_service.load_recent_history()?;

        let mut lines = vec!["리뷰 센터".to_string(), format!("inbox: {}", inbox.len())];

        if inbox.is_empty() {
            lines.push("- empty".to_string());
        } else {
            lines.extend(inbox.iter().take(5).map(|item| {
                let handoff = item
                    .handoff_target
                    .as_deref()
                    .map(|target| format!(" → {target}"))
                    .unwrap_or_default();
                format!(
                    "- {} [{}] {}{}",
                    item.thread_id,
                    item.inbox_state,
                    compact_review_text(&item.summary, 72),
                    handoff,
                )
            }));
        }

        if let Some(thread_id) = current_thread_id.as_deref() {
            lines.push(format!("top pending inbox thread: {thread_id}"));
            if current_thread_reviews.is_empty() {
                lines.push("- no review rows for the top pending inbox thread".to_string());
            } else {
                lines.extend(current_thread_reviews.iter().take(3).map(|review| {
                    let handoff = match (
                        review.handoff_target.as_deref(),
                        review.handoff_note.as_deref(),
                    ) {
                        (Some(target), Some(note))
                            if !target.trim().is_empty() && !note.trim().is_empty() =>
                        {
                            format!(" / handoff: {target}: {note}")
                        }
                        (Some(target), _) if !target.trim().is_empty() => {
                            format!(" / handoff: {target}")
                        }
                        _ => String::new(),
                    };
                    format!(
                        "- {} [{}] {}{}",
                        review.review_label,
                        review.review_state,
                        compact_review_text(&review.review_summary, 72),
                        handoff,
                    )
                }));
            }
        }

        lines.push(format!("history: {}", history.len()));
        if history.is_empty() {
            lines.push("- empty".to_string());
        } else {
            lines.extend(history.iter().take(5).map(|entry| {
                format!(
                    "- {} [{}] {}",
                    entry.thread_id,
                    entry.event_kind,
                    compact_review_text(&entry.summary, 72),
                )
            }));
        }

        Ok(lines.join("\n"))
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

    fn renew_runner_lease(&self, operation: &str) -> Result<()> {
        self.global_runner_lease
            .renew_global_runner_lease(
                &self.binding_workspace_dir,
                &self.stream_key,
                &self.lease_owner_token,
                std::process::id(),
                self.lease_generation.load(Ordering::Acquire),
                self.lease_ttl_seconds,
            )
            .with_context(|| format!("global Telegram runner lease was lost {operation}"))?;
        Ok(())
    }
}

fn next_update_offset(
    updates: &[TelegramUpdate],
    current_offset: Option<i64>,
) -> Result<Option<i64>> {
    let Some(latest_update_id) = updates.iter().map(|update| update.update_id).max() else {
        return Ok(current_offset);
    };
    if latest_update_id < 0 {
        anyhow::bail!("telegram update id must be non-negative");
    }
    let candidate = latest_update_id.checked_add(1).context(
        "telegram update cursor overflowed; refusing to execute an unacknowledgeable batch",
    )?;
    Ok(Some(
        current_offset.map_or(candidate, |current| current.max(candidate)),
    ))
}

fn validate_update_batch(updates: &[TelegramUpdate]) -> Result<()> {
    for update in updates {
        if update.update_id < 0 {
            anyhow::bail!("telegram update id must be non-negative");
        }
        update.update_id.checked_add(1).context(
            "telegram update cursor overflowed; refusing to execute an unacknowledgeable batch",
        )?;
    }
    Ok(())
}

fn telegram_stream_key(token: &str) -> Result<String> {
    let (bot_id, secret) = token
        .split_once(':')
        .context("Telegram bot token does not contain a stable numeric bot id")?;
    if bot_id.is_empty()
        || bot_id.starts_with('0')
        || !bot_id.bytes().all(|byte| byte.is_ascii_digit())
        || secret.is_empty()
        || secret.contains(':')
    {
        anyhow::bail!("Telegram bot token does not contain a valid stable numeric bot id");
    }
    let bot_id = bot_id
        .parse::<u64>()
        .context("Telegram bot token contains an out-of-range bot id")?;
    if bot_id == 0 {
        anyhow::bail!("Telegram bot token does not contain a positive bot id");
    }
    Ok(format!("bot-id:{bot_id}"))
}

fn telegram_stream_bot_id(stream_key: &str) -> Result<u64> {
    let raw = stream_key
        .strip_prefix("bot-id:")
        .context("Telegram stream key does not contain a numeric bot id")?;
    let bot_id = raw
        .parse::<u64>()
        .context("Telegram stream key contains an invalid bot id")?;
    if bot_id == 0 || raw.starts_with('0') {
        anyhow::bail!("Telegram stream key contains a non-canonical bot id");
    }
    Ok(bot_id)
}

fn telegram_runner_owner_token() -> String {
    let mut nonce = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let nonce = nonce
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("runner-{}-{nonce}", std::process::id())
}

fn telegram_runner_lease_ttl_seconds(poll_timeout_seconds: u16) -> u64 {
    let ttl = u64::from(poll_timeout_seconds)
        .saturating_mul(RUNNER_LEASE_POLL_MULTIPLIER)
        .saturating_add(RUNNER_LEASE_TRANSPORT_MARGIN_SECONDS)
        .max(MIN_RUNNER_LEASE_TTL_SECONDS);
    debug_assert!(ttl > telegram_max_curl_request_seconds(poll_timeout_seconds));
    ttl
}

fn telegram_max_curl_request_seconds(poll_timeout_seconds: u16) -> u64 {
    u64::from(poll_timeout_seconds).saturating_add(TELEGRAM_LONG_POLL_TRANSPORT_MARGIN_SECONDS)
}

fn compact_review_text(text: &str, limit: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= limit {
        return normalized;
    }
    normalized
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>()
        + "…"
}

#[cfg(test)]
mod tests;
