use std::collections::BTreeSet;
use std::io::Read;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};

use super::copy::telegram_bot_usage_text;
use super::{DEFAULT_POLL_TIMEOUT_SECONDS, MAX_POLL_TIMEOUT_SECONDS};

const MAX_TELEGRAM_BOT_TOKEN_CHARS: usize = 256;
const MAX_TELEGRAM_CONFIG_BYTES: u64 = 1024 * 1024;
const TELEGRAM_ENV_PREFIX: &str = "AKRA_TELEGRAM_";
const TELEGRAM_ENV_KEYS: &[&str] = &[
    "AKRA_TELEGRAM_BOT_TOKEN",
    "AKRA_TELEGRAM_ALLOWED_CHAT_IDS",
    "AKRA_TELEGRAM_ALLOWED_USER_IDS",
];

/*
 * config.rs is the Telegram inbound adapter's bootstrap boundary. It resolves secrets and operator
 * safety controls before the runner is constructed: bot token, chat allowlist, long-poll timeout,
 * and whether old updates should be discarded. The rest of telegram_bot/mod.rs can then treat
 * TelegramBotArgs as a validated runtime contract instead of knowing about env files or CLI syntax.
 */
#[derive(Debug, Clone)]
pub(super) struct TelegramBotArgs {
    // Required secret used only by the outbound Telegram adapter; parsing keeps it out of runner logic.
    pub(super) token: String,
    // Empty set fails closed; every planning/review/parallel command requires an explicitly allowed chat.
    pub(super) allowed_chat_ids: BTreeSet<i64>,
    // Group and supergroup commands require both an allowed chat and an allowed sender account.
    pub(super) allowed_user_ids: BTreeSet<i64>,
    // Long polling needs a non-zero timeout because zero degenerates into a tight polling loop.
    pub(super) poll_timeout_seconds: u16,
    // Defaulting to drop protects a restarted local bot from replaying stale operator commands.
    pub(super) drop_pending_updates: bool,
    // Rebinding is explicit because one Telegram bot owns one machine-wide update stream.
    pub(super) rebind_workspace: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct TelegramBotEnvironment {
    /*
     * Environment is only an intermediate source merge result. token remains optional here so CLI
     * flags can override or supply it, and parse_args_with_environment can emit one final error that
     * names every supported source.
     */
    pub(super) token: Option<String>,
    pub(super) allowed_chat_ids: BTreeSet<i64>,
    pub(super) allowed_user_ids: BTreeSet<i64>,
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
    /*
     * Source precedence is deliberate: config file first and process environment next. Secrets are
     * never accepted on the command line because process arguments are observable. Chat ids are
     * additive across the merged environment and repeated --allow-chat-id
     * flags, which lets an operator keep a default allowlist and temporarily add one chat for a run.
     */
    let token = environment.token;
    let mut allowed_chat_ids = environment.allowed_chat_ids;
    let mut allowed_user_ids = environment.allowed_user_ids;
    let mut poll_timeout_seconds = DEFAULT_POLL_TIMEOUT_SECONDS;
    let mut drop_pending_updates = true;
    let mut rebind_workspace = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{}", telegram_bot_usage_text());
                std::process::exit(0);
            }
            "--token" => {
                bail!(
                    "--token is disabled because process arguments are observable; use AKRA_TELEGRAM_BOT_TOKEN or the private mode-0600 Telegram config file"
                );
            }
            token_argument if token_argument.starts_with("--token=") => {
                bail!(
                    "--token is disabled because process arguments are observable; use AKRA_TELEGRAM_BOT_TOKEN or the private mode-0600 Telegram config file"
                );
            }
            "--allow-chat-id" => {
                let raw = args
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --allow-chat-id"))?;
                allowed_chat_ids.insert(parse_chat_id(raw.as_str())?);
            }
            "--allow-user-id" => {
                let raw = args
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --allow-user-id"))?;
                allowed_user_ids.insert(parse_user_id(raw.as_str())?);
            }
            "--poll-timeout-seconds" => {
                let raw = args
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --poll-timeout-seconds"))?;
                poll_timeout_seconds = raw.parse::<u16>().with_context(|| {
                    format!("failed to parse poll timeout seconds from `{raw}`")
                })?;
                if poll_timeout_seconds == 0 {
                    // A zero timeout would still be accepted by Telegram syntax but is wrong for this runner.
                    bail!("--poll-timeout-seconds must be greater than zero");
                }
                if poll_timeout_seconds > MAX_POLL_TIMEOUT_SECONDS {
                    bail!("--poll-timeout-seconds must not exceed {MAX_POLL_TIMEOUT_SECONDS}");
                }
            }
            "--keep-pending" => {
                drop_pending_updates = false;
            }
            "--rebind-workspace" => {
                rebind_workspace = true;
            }
            unknown => {
                bail!(
                    "unsupported telegram-bot argument: {unknown}\n{}",
                    telegram_bot_usage_text()
                );
            }
        }
    }

    let token = token
        .filter(|token| {
            !token.is_empty()
                && token.trim() == token
                && token.chars().count() <= MAX_TELEGRAM_BOT_TOKEN_CHARS
                && !token.chars().any(char::is_control)
        })
        .ok_or_else(|| {
        anyhow!(
            "a non-empty telegram bot token is required via AKRA_TELEGRAM_BOT_TOKEN or the private mode-0600 Telegram config file"
        )
    })?;
    Ok(TelegramBotArgs {
        token,
        allowed_chat_ids,
        allowed_user_ids,
        poll_timeout_seconds,
        drop_pending_updates,
        rebind_workspace,
    })
}

fn load_environment() -> Result<TelegramBotEnvironment> {
    /*
     * The optional config file gives long-running local bots a stable place for secrets while still
     * allowing process env to override CI, shell, or service-manager values. Missing files are fine;
     * unreadable existing files are surfaced because they indicate an operator setup problem.
     */
    validate_telegram_process_environment_keys(std::env::vars_os().map(|(key, _)| key))?;
    let config_body = default_telegram_env_file_path()?
        .map(|path| read_telegram_environment_file(&path))
        .transpose()?;

    load_environment_from_sources(
        config_body.as_deref(),
        optional_utf8_environment_value("AKRA_TELEGRAM_BOT_TOKEN")?,
        optional_utf8_environment_value("AKRA_TELEGRAM_ALLOWED_CHAT_IDS")?,
        optional_utf8_environment_value("AKRA_TELEGRAM_ALLOWED_USER_IDS")?,
    )
}

pub(super) fn validate_telegram_process_environment_keys(
    keys: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<()> {
    for key in keys {
        let lossy = key.to_string_lossy();
        if !lossy.starts_with(TELEGRAM_ENV_PREFIX) {
            continue;
        }
        let key = key
            .to_str()
            .ok_or_else(|| anyhow!("Telegram environment key must be valid UTF-8"))?;
        if !TELEGRAM_ENV_KEYS.contains(&key) {
            bail!("unsupported Telegram environment key `{key}`");
        }
    }
    Ok(())
}

fn optional_utf8_environment_value(name: &str) -> Result<Option<String>> {
    utf8_environment_value(name, std::env::var_os(name))
}

pub(super) fn utf8_environment_value(
    name: &str,
    value: Option<std::ffi::OsString>,
) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some(value) => value
            .into_string()
            .map(Some)
            .map_err(|_| anyhow!("{name} must be valid UTF-8")),
    }
}

pub(super) fn read_telegram_environment_file(path: &std::path::Path) -> Result<String> {
    #[cfg(unix)]
    {
        read_private_unix_telegram_environment_file(path)
    }
    #[cfg(windows)]
    {
        read_private_windows_telegram_environment_file(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        bail!("secure Telegram config file reads are unsupported on this platform");
    }
}

fn read_bounded_utf8_config(file: &mut std::fs::File) -> Result<String> {
    let mut bytes = Vec::new();
    file.take(MAX_TELEGRAM_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("failed to read Telegram config file")?;
    if bytes.len() as u64 > MAX_TELEGRAM_CONFIG_BYTES {
        bail!(
            "Telegram config file exceeds the {} byte safety limit",
            MAX_TELEGRAM_CONFIG_BYTES
        );
    }
    String::from_utf8(bytes).context("Telegram config file must be valid UTF-8")
}

#[cfg(unix)]
fn read_private_unix_telegram_environment_file(path: &std::path::Path) -> Result<String> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

    let parent_path = path
        .parent()
        .ok_or_else(|| anyhow!("Telegram config path has no parent directory"))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow!("Telegram config path has no file name"))?;
    let parent = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(parent_path)
        .context("failed to securely open Telegram config parent directory")?;
    let parent_before = parent
        .metadata()
        .context("failed to inspect Telegram config parent directory")?;
    validate_unix_telegram_config_parent(parent_path, &parent_before)?;
    let parent_path_before = std::fs::symlink_metadata(parent_path)
        .context("failed to inspect Telegram config parent path")?;
    validate_unix_telegram_config_parent(parent_path, &parent_path_before)?;
    if parent_before.dev() != parent_path_before.dev()
        || parent_before.ino() != parent_path_before.ino()
    {
        bail!("Telegram config parent changed while it was being opened");
    }

    let before =
        std::fs::symlink_metadata(path).context("failed to inspect Telegram config file")?;
    validate_unix_telegram_config_metadata(path, &before)?;
    let file_name = std::ffi::CString::new(file_name.as_bytes())
        .context("Telegram config file name contains a NUL byte")?;
    let raw_file = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            file_name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if raw_file < 0 {
        return Err(std::io::Error::last_os_error())
            .context("failed to securely open Telegram config file from anchored parent");
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(raw_file) };
    let opened = file
        .metadata()
        .context("failed to inspect opened Telegram config file")?;
    validate_unix_telegram_config_metadata(path, &opened)?;
    if before.dev() != opened.dev() || before.ino() != opened.ino() {
        bail!("Telegram config file changed while it was being opened");
    }
    if opened.permissions().mode() & 0o077 != 0 {
        bail!("Telegram config file must not grant group or other permissions; set mode 0600");
    }

    let body = read_bounded_utf8_config(&mut file)?;
    let after = file
        .metadata()
        .context("failed to re-inspect opened Telegram config file")?;
    let path_after =
        std::fs::symlink_metadata(path).context("failed to re-inspect Telegram config path")?;
    let parent_after = parent
        .metadata()
        .context("failed to re-inspect Telegram config parent directory")?;
    let parent_path_after = std::fs::symlink_metadata(parent_path)
        .context("failed to re-inspect Telegram config parent path")?;
    validate_unix_telegram_config_parent(parent_path, &parent_after)?;
    validate_unix_telegram_config_parent(parent_path, &parent_path_after)?;
    validate_unix_telegram_config_metadata(path, &after)?;
    validate_unix_telegram_config_metadata(path, &path_after)?;
    if !same_unix_config_version(&opened, &after)
        || after.dev() != path_after.dev()
        || after.ino() != path_after.ino()
        || !same_unix_config_version(&after, &path_after)
        || parent_before.dev() != parent_after.dev()
        || parent_before.ino() != parent_after.ino()
        || parent_after.dev() != parent_path_after.dev()
        || parent_after.ino() != parent_path_after.ino()
    {
        bail!("Telegram config file changed while it was being read");
    }
    Ok(body)
}

#[cfg(unix)]
fn validate_unix_telegram_config_parent(
    path: &std::path::Path,
    metadata: &std::fs::Metadata,
) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o022 != 0
    {
        bail!(
            "Telegram config parent must be a current-user-owned non-writable real directory: {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(unix)]
fn validate_unix_telegram_config_metadata(
    path: &std::path::Path,
    metadata: &std::fs::Metadata,
) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
    {
        bail!(
            "Telegram config must be a current-user-owned regular file with one link: {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(unix)]
fn same_unix_config_version(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

#[cfg(windows)]
struct WindowsTelegramConfigParentAnchor {
    path: PathBuf,
    file: std::fs::File,
    identity: crate::private_fs::WindowsFileIdentitySnapshot,
}

#[cfg(windows)]
fn read_private_windows_telegram_environment_file(path: &std::path::Path) -> Result<String> {
    use std::os::windows::fs::OpenOptionsExt;

    use crate::private_fs::{
        WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT, WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ,
        WINDOWS_READ_CONTROL, validate_windows_identity_snapshot,
        validate_windows_path_identity_only, validate_windows_private_owner_and_acl,
        windows_file_identity_snapshot,
    };

    let parent_anchors = open_windows_telegram_config_parent_anchors(path)?;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .context("failed to securely open Windows Telegram config file")?;
    validate_windows_path_identity_only(path, &file, false)
        .context("Windows Telegram config must be a non-reparse single-link file")?;
    validate_windows_private_owner_and_acl(path, &file)
        .context("Windows Telegram config must have a protected current-user-only DACL")?;
    let file_identity = windows_file_identity_snapshot(&file)?;
    let body = read_bounded_utf8_config(&mut file)?;
    validate_windows_identity_snapshot(file_identity, &file, false)
        .context("Windows Telegram config changed while being read")?;
    validate_windows_path_identity_only(path, &file, false)
        .context("Windows Telegram config path changed while being read")?;
    validate_windows_private_owner_and_acl(path, &file)
        .context("Windows Telegram config security changed while being read")?;
    for anchor in &parent_anchors {
        validate_windows_identity_snapshot(anchor.identity, &anchor.file, true).with_context(
            || {
                format!(
                    "Windows Telegram config parent changed while reading: {}",
                    anchor.path.display()
                )
            },
        )?;
        validate_windows_path_identity_only(&anchor.path, &anchor.file, true).with_context(
            || {
                format!(
                    "Windows Telegram config parent path changed or became a reparse point: {}",
                    anchor.path.display()
                )
            },
        )?;
    }
    Ok(body)
}

#[cfg(windows)]
fn open_windows_telegram_config_parent_anchors(
    path: &std::path::Path,
) -> Result<Vec<WindowsTelegramConfigParentAnchor>> {
    use std::os::windows::fs::OpenOptionsExt;

    use crate::private_fs::{
        WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
        WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ, WINDOWS_READ_CONTROL,
        validate_windows_path_identity, validate_windows_path_identity_only,
        windows_file_identity_snapshot,
    };

    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("Windows Telegram config path has no parent"))?;
    let mut paths = parent.ancestors().collect::<Vec<_>>();
    paths.reverse();
    let mut anchors = Vec::with_capacity(paths.len());
    for parent_path in paths {
        if parent_path.as_os_str().is_empty() {
            continue;
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
            .share_mode(WINDOWS_FILE_SHARE_ALL)
            .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS)
            .open(parent_path)
            .with_context(|| {
                format!(
                    "failed to securely open Windows Telegram config parent {}",
                    parent_path.display()
                )
            })?;
        validate_windows_path_identity_only(parent_path, &file, true).with_context(|| {
            format!(
                "Windows Telegram config parent must not be a junction or reparse point: {}",
                parent_path.display()
            )
        })?;
        if parent_path == parent {
            validate_windows_path_identity(parent_path, &file, true).with_context(|| {
                format!(
                    "Windows Telegram config parent must be owned by the current user: {}",
                    parent_path.display()
                )
            })?;
        }
        let identity = windows_file_identity_snapshot(&file)?;
        anchors.push(WindowsTelegramConfigParentAnchor {
            path: parent_path.to_path_buf(),
            file,
            identity,
        });
    }
    Ok(anchors)
}

pub(super) fn load_environment_from_sources(
    config_body: Option<&str>,
    token: Option<String>,
    allowed_chat_ids: Option<String>,
    allowed_user_ids: Option<String>,
) -> Result<TelegramBotEnvironment> {
    // Tests call this directly to lock down source precedence without mutating the real process env.
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
    if allowed_user_ids.is_some() {
        environment.allowed_user_ids = parse_allowed_user_ids(allowed_user_ids)?;
    }
    Ok(environment)
}

fn default_telegram_env_file_path() -> Result<Option<PathBuf>> {
    /*
     * Follow XDG when available, then fall back to ~/.config. Returning None for missing files keeps
     * the default developer path zero-config while still making an existing malformed file visible
     * through load_environment's read error path.
     */
    let xdg_base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(validate_absolute_telegram_config_root)
        .transpose()?;
    #[cfg(unix)]
    let base_dir = match xdg_base {
        Some(path) => Some(path),
        None => std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .map(validate_absolute_telegram_config_root)
            .transpose()?
            .map(|home| home.join(".config")),
    };
    #[cfg(windows)]
    let base_dir = Some(match xdg_base {
        Some(path) => path,
        None => crate::private_fs::windows_local_app_data_path()?,
    });
    #[cfg(not(any(unix, windows)))]
    let base_dir = xdg_base;
    let Some(base_dir) = base_dir else {
        return Ok(None);
    };
    let path = base_dir.join("akra/telegram.env");
    telegram_env_file_path_if_present(path)
}

fn validate_absolute_telegram_config_root(path: PathBuf) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("Telegram config root must be an absolute path");
    }
    Ok(path)
}

pub(super) fn telegram_env_file_path_if_present(path: PathBuf) -> Result<Option<PathBuf>> {
    match std::fs::symlink_metadata(&path) {
        Ok(_) => Ok(Some(path)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("failed to inspect default Telegram config path"),
    }
}

pub(super) fn apply_environment_file(
    environment: &mut TelegramBotEnvironment,
    body: &str,
) -> Result<()> {
    /*
     * This parser intentionally accepts a small .env subset only: comments, blank lines, optional
     * `export`, KEY=VALUE, and simple surrounding quotes. Unrelated keys are ignored, while unknown
     * `AKRA_TELEGRAM_*` keys fail closed so allowlist or token typos cannot look configured.
     */
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
            "AKRA_TELEGRAM_ALLOWED_USER_IDS" => {
                environment.allowed_user_ids = parse_allowed_user_ids(Some(value.to_string()))?;
            }
            unknown if unknown.starts_with("AKRA_TELEGRAM_") => {
                bail!(
                    "unsupported Telegram config key `{unknown}` on line {}",
                    line_number + 1
                )
            }
            _ => {}
        }
    }
    Ok(())
}

fn trim_optional_quotes(value: &str) -> &str {
    // Do not unescape shell syntax; this is a small convenience layer, not a full shell parser.
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
    /*
     * Telegram chat ids may be negative for groups/supergroups, so i64 is the transport type.
     * BTreeSet gives deterministic ordering for tests and for later policy diagnostics.
     */
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
    let chat_id = raw
        .parse::<i64>()
        .map_err(|_| anyhow!("failed to parse telegram chat id; expected a non-zero integer"))?;
    if chat_id == 0 {
        bail!("telegram chat id must be non-zero");
    }
    Ok(chat_id)
}

fn parse_allowed_user_ids(raw: Option<String>) -> Result<BTreeSet<i64>> {
    let mut values = BTreeSet::new();
    let Some(raw) = raw else {
        return Ok(values);
    };
    for entry in raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        values.insert(parse_user_id(entry)?);
    }
    Ok(values)
}

fn parse_user_id(raw: &str) -> Result<i64> {
    let user_id = raw
        .parse::<i64>()
        .map_err(|_| anyhow!("failed to parse telegram user id; expected a positive integer"))?;
    if user_id <= 0 {
        bail!("telegram user id must be a positive integer");
    }
    Ok(user_id)
}
