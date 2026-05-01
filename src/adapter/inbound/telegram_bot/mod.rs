use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};

use crate::adapter::outbound::app_server::{AppServerPlanningWorkerAdapter, CodexAppServerAdapter};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::adapter::outbound::telegram::CurlTelegramBotAdapter;
use crate::application::port::outbound::telegram_bot_port::{
    TelegramBotPort, TelegramInboundMessage, TelegramPollRequest, TelegramSendMessageRequest,
    TelegramUpdate,
};
use crate::application::service::planning::{
    PlanningAdminFacadeService, PlanningControlCommand, PlanningControlService,
    PlanningResetTarget, PlanningServices,
};

const DEFAULT_POLL_TIMEOUT_SECONDS: u16 = 30;
const DEFAULT_POLL_LIMIT: u8 = 100;
const DEFAULT_FAILURE_BACKOFF: Duration = Duration::from_secs(2);
const TELEGRAM_BOT_USAGE: &str = "Usage: akra telegram [--token <token>] [--allow-chat-id <chat_id>]... [--poll-timeout-seconds <seconds>] [--keep-pending]\nAlias: akra telegram-bot [--token <token>] [--allow-chat-id <chat_id>]... [--poll-timeout-seconds <seconds>] [--keep-pending]\nEnv: AKRA_TELEGRAM_BOT_TOKEN, AKRA_TELEGRAM_ALLOWED_CHAT_IDS=123,456\nConfig: $XDG_CONFIG_HOME/akra/telegram.env or ~/.config/akra/telegram.env";

#[derive(Debug, Clone)]
struct TelegramBotArgs {
    token: String,
    allowed_chat_ids: BTreeSet<i64>,
    poll_timeout_seconds: u16,
    drop_pending_updates: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TelegramBotEnvironment {
    token: Option<String>,
    allowed_chat_ids: BTreeSet<i64>,
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

fn parse_args<I>(args: I) -> Result<TelegramBotArgs>
where
    I: IntoIterator<Item = String>,
{
    parse_args_with_environment(args, load_environment()?)
}

fn parse_args_with_environment<I>(
    args: I,
    environment: TelegramBotEnvironment,
) -> Result<TelegramBotArgs>
where
    I: IntoIterator<Item = String>,
{
    let mut token = environment.token;
    let mut allowed_chat_ids = environment.allowed_chat_ids;
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

fn load_environment() -> Result<TelegramBotEnvironment> {
    let config_body = default_telegram_env_file_path()
        .map(|path| {
            std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read Telegram config file {}", path.display()))
        })
        .transpose()?;

    load_environment_from_sources(
        config_body.as_deref(),
        std::env::var("AKRA_TELEGRAM_BOT_TOKEN").ok(),
        std::env::var("AKRA_TELEGRAM_ALLOWED_CHAT_IDS").ok(),
    )
}

fn load_environment_from_sources(
    config_body: Option<&str>,
    token: Option<String>,
    allowed_chat_ids: Option<String>,
) -> Result<TelegramBotEnvironment> {
    let mut environment = TelegramBotEnvironment::default();

    if let Some(config_body) = config_body {
        apply_environment_file(&mut environment, config_body)?;
    }
    if let Some(token) = token {
        environment.token = Some(token);
    }
    if allowed_chat_ids.is_some() {
        environment.allowed_chat_ids = parse_allowed_chat_ids(allowed_chat_ids)?;
    }

    Ok(environment)
}

fn default_telegram_env_file_path() -> Option<PathBuf> {
    let base_dir = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|home| home.join(".config"))
        })?;
    let path = base_dir.join("akra/telegram.env");
    path.is_file().then_some(path)
}

fn apply_environment_file(environment: &mut TelegramBotEnvironment, body: &str) -> Result<()> {
    for (line_number, raw_line) in body.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let line = line.strip_prefix("export ").unwrap_or(line);
        let (key, raw_value) = line.split_once('=').ok_or_else(|| {
            anyhow!(
                "invalid Telegram config entry on line {}: expected KEY=VALUE",
                line_number + 1
            )
        })?;
        let value = trim_optional_quotes(raw_value.trim());

        match key.trim() {
            "AKRA_TELEGRAM_BOT_TOKEN" => {
                environment.token = Some(value.to_string());
            }
            "AKRA_TELEGRAM_ALLOWED_CHAT_IDS" => {
                environment.allowed_chat_ids = parse_allowed_chat_ids(Some(value.to_string()))?;
            }
            _ => {}
        }
    }

    Ok(())
}

fn trim_optional_quotes(value: &str) -> &str {
    if value.len() >= 2 {
        if let Some(stripped) = value
            .strip_prefix('"')
            .and_then(|inner| inner.strip_suffix('"'))
        {
            return stripped;
        }
        if let Some(stripped) = value
            .strip_prefix('\'')
            .and_then(|inner| inner.strip_suffix('\''))
        {
            return stripped;
        }
    }
    value
}

fn parse_allowed_chat_ids(raw: Option<String>) -> Result<BTreeSet<i64>> {
    let mut values = BTreeSet::new();
    let Some(raw) = raw else {
        return Ok(values);
    };
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
        "/start" | "/help" | "help" => parse_command_without_arguments(
            &arguments,
            TelegramInboundCommand::Planning(PlanningControlCommand::Help),
            "/help",
        ),
        "/whoami" => {
            parse_command_without_arguments(&arguments, TelegramInboundCommand::WhoAmI, "/whoami")
        }
        "/status" | "status" => parse_command_without_arguments(
            &arguments,
            TelegramInboundCommand::Planning(PlanningControlCommand::Status),
            "/status",
        ),
        "/queue" | "queue" => parse_command_without_arguments(
            &arguments,
            TelegramInboundCommand::Planning(PlanningControlCommand::Queue),
            "/queue",
        ),
        "/plan" => parse_plan_arguments(&arguments),
        "/reset_queue" => parse_command_without_arguments(
            &arguments,
            TelegramInboundCommand::Planning(PlanningControlCommand::Reset(
                PlanningResetTarget::Queue,
            )),
            "/reset_queue",
        ),
        "/reset_directions" => parse_command_without_arguments(
            &arguments,
            TelegramInboundCommand::Planning(PlanningControlCommand::Reset(
                PlanningResetTarget::Directions,
            )),
            "/reset_directions",
        ),
        "/reset_all" => parse_command_without_arguments(
            &arguments,
            TelegramInboundCommand::Planning(PlanningControlCommand::Reset(
                PlanningResetTarget::All,
            )),
            "/reset_all",
        ),
        "/reset" => parse_reset_arguments(&arguments),
        token if token.starts_with('/') => TelegramParsedMessage::Error(format!(
            "지원하지 않는 명령어입니다: {token}\n{}",
            PlanningControlService::new(Arc::new(NoopPlanningControlSurface)).help_text()
        )),
        _ => TelegramParsedMessage::Ignore,
    }
}

fn parse_command_without_arguments(
    arguments: &[&str],
    command: TelegramInboundCommand,
    usage: &'static str,
) -> TelegramParsedMessage {
    if arguments.is_empty() {
        TelegramParsedMessage::Command(command)
    } else {
        TelegramParsedMessage::Error(format!("사용법: {usage}"))
    }
}

fn parse_plan_arguments(arguments: &[&str]) -> TelegramParsedMessage {
    match arguments {
        [] | ["status"] => TelegramParsedMessage::Command(TelegramInboundCommand::Planning(
            PlanningControlCommand::Status,
        )),
        _ => TelegramParsedMessage::Error("사용법: /plan [status]".to_string()),
    }
}

fn parse_reset_arguments(arguments: &[&str]) -> TelegramParsedMessage {
    let [target] = arguments else {
        return TelegramParsedMessage::Error(
            "사용법: /reset queue | /reset directions | /reset all".to_string(),
        );
    };

    let command = match *target {
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

    fn reset_workspace(
        &self,
        _target: PlanningResetTarget,
    ) -> Result<crate::application::service::planning::control::PlanningControlResetOutcome> {
        bail!("noop control surface should not execute");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use anyhow::{Result, anyhow, bail};

    use super::{
        TelegramBotEnvironment, TelegramBotPolicy, TelegramBotRunner, TelegramInboundCommand,
        TelegramParsedMessage, apply_environment_file, load_environment_from_sources,
        parse_args_with_environment, parse_message,
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

        fn reset_workspace(
            &self,
            target: PlanningResetTarget,
        ) -> Result<PlanningControlResetOutcome> {
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

        fn reset_workspace(
            &self,
            target: PlanningResetTarget,
        ) -> Result<PlanningControlResetOutcome> {
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
}
