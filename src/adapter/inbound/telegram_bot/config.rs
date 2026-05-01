use std::collections::BTreeSet;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};

use super::{DEFAULT_POLL_TIMEOUT_SECONDS, TELEGRAM_BOT_USAGE};

#[derive(Debug, Clone)]
pub(super) struct TelegramBotArgs {
    pub(super) token: String,
    pub(super) allowed_chat_ids: BTreeSet<i64>,
    pub(super) poll_timeout_seconds: u16,
    pub(super) drop_pending_updates: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct TelegramBotEnvironment {
    pub(super) token: Option<String>,
    pub(super) allowed_chat_ids: BTreeSet<i64>,
}

pub(super) fn parse_args<I>(args: I) -> Result<TelegramBotArgs>
where
    I: IntoIterator<Item = String>,
{
    parse_args_with_environment(args, load_environment()?)
}

pub(super) fn parse_args_with_environment<I>(
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

pub(super) fn load_environment_from_sources(
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

pub(super) fn apply_environment_file(
    environment: &mut TelegramBotEnvironment,
    body: &str,
) -> Result<()> {
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
