/*
Akra configuration is deliberately a narrow, reviewed surface.  It is not a
pass-through for Codex configuration: credentials, approval/sandbox policy,
process environment, and delivery opt-ins remain process/private contracts.

The resolver keeps all precedence in one place:

    command line > environment > project TOML > legacy Git config > global TOML > built-ins

Values are merged leaf-by-leaf.  This makes a project override for one TUI
setting inherit every unrelated global setting rather than replacing a table.
*/
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
#[cfg(not(unix))]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};
#[cfg(any(not(unix), test))]
use std::time::{SystemTime, UNIX_EPOCH};
use toml_edit::{DocumentMut, Item, value};
use tracing_subscriber::EnvFilter;

pub const CONFIG_SCHEMA_VERSION: u32 = 1;
const CONFIG_FILE_NAME: &str = "config.toml";
const PROJECT_CONFIG_DIRECTORY: &str = ".akra";
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_TEXT_VALUE_CHARS: usize = 256;
const CONFIG_LOCK_WAIT: Duration = Duration::from_secs(5);
const CONFIG_LOCK_RETRY: Duration = Duration::from_millis(20);

static INSTALLED_CONFIG: OnceLock<ResolvedAkraConfig> = OnceLock::new();

/// The source that supplied an effective leaf setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingOrigin {
    Builtin,
    GlobalToml,
    LegacyGitConfig { key: String },
    ProjectToml,
    Environment { variable: String },
    CommandLine,
}

impl fmt::Display for SettingOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Builtin => formatter.write_str("builtin"),
            Self::GlobalToml => formatter.write_str("global"),
            Self::LegacyGitConfig { key } => write!(formatter, "legacy git config ({key})"),
            Self::ProjectToml => formatter.write_str("project"),
            Self::Environment { variable } => write!(formatter, "environment ({variable})"),
            Self::CommandLine => formatter.write_str("command line"),
        }
    }
}

/// A resolved value paired with its provenance.  The public configuration
/// snapshot exposes values directly and `origin_for` provides this provenance
/// without making every consumer carry an extra wrapper type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSetting<T> {
    pub value: T,
    pub origin: SettingOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationConfig {
    /// `None` explicitly requests the app-server default (`"default"` in TOML).
    pub model: Option<String>,
    /// `None` explicitly requests the app-server default (`"default"` in TOML).
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiConfig {
    pub show_startup_visual: bool,
    pub planning_worker_visibility: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubConfig {
    pub auto_discover_pull_request: bool,
    pub review_poll_interval_secs: u64,
    pub push_remote: String,
    pub pull_request_mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelConfig {
    pub integration_branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppServerConfig {
    pub response_timeout_secs: u64,
    pub prompt_log: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubprocessConfig {
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsConfig {
    pub trace: String,
    pub spans: String,
    pub max_files: u64,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub tokio_console: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminConfig {
    pub graphic_enabled: bool,
    pub graphic_poll_interval_ms: u64,
}

/// Fully-resolved, secret-free runtime settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AkraConfig {
    pub conversation: ConversationConfig,
    pub tui: TuiConfig,
    pub github: GithubConfig,
    pub parallel: ParallelConfig,
    pub app_server: AppServerConfig,
    pub subprocess: SubprocessConfig,
    pub diagnostics: DiagnosticsConfig,
    pub admin: AdminConfig,
}

impl AkraConfig {
    pub fn builtin() -> Self {
        Self {
            conversation: ConversationConfig {
                model: Some("gpt-5.6-sol".to_string()),
                reasoning_effort: Some("medium".to_string()),
            },
            tui: TuiConfig {
                show_startup_visual: true,
                planning_worker_visibility: "normal".to_string(),
            },
            github: GithubConfig {
                auto_discover_pull_request: true,
                review_poll_interval_secs: 60,
                push_remote: "origin".to_string(),
                pull_request_mode: "required".to_string(),
            },
            parallel: ParallelConfig {
                integration_branch: "prerelease".to_string(),
            },
            app_server: AppServerConfig {
                response_timeout_secs: 15,
                prompt_log: false,
            },
            subprocess: SubprocessConfig { timeout_secs: 30 },
            diagnostics: DiagnosticsConfig {
                trace: "off".to_string(),
                spans: "none".to_string(),
                max_files: 7,
                max_file_bytes: 16 * 1024 * 1024,
                max_total_bytes: 64 * 1024 * 1024,
                tokio_console: false,
            },
            admin: AdminConfig {
                graphic_enabled: true,
                graphic_poll_interval_ms: 10_000,
            },
        }
    }
}

impl Default for AkraConfig {
    fn default() -> Self {
        Self::builtin()
    }
}

/// Sparse TOML layer.  It is public so adapters/tests can construct a layer,
/// but all mutation and validation should go through `ConfigurationService`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AkraConfigLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation: Option<ConversationConfigLayer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tui: Option<TuiConfigLayer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github: Option<GithubConfigLayer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel: Option<ParallelConfigLayer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_server: Option<AppServerConfigLayer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subprocess: Option<SubprocessConfigLayer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<DiagnosticsConfigLayer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin: Option<AdminConfigLayer>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationConfigLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TuiConfigLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_startup_visual: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planning_worker_visibility: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GithubConfigLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_discover_pull_request: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_poll_interval_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_remote: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pull_request_mode: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParallelConfigLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_branch: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppServerConfigLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_timeout_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_log: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubprocessConfigLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticsConfigLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spans: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_files: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_file_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokio_console: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdminConfigLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graphic_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graphic_poll_interval_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigScope {
    Global,
    Project,
}

impl ConfigScope {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPaths {
    pub global: PathBuf,
    pub project: Option<PathBuf>,
    pub workspace_root: Option<PathBuf>,
}

impl ConfigPaths {
    pub fn discover(cwd: &Path) -> Result<Self> {
        Self::discover_with_global(cwd, global_config_path()?)
    }

    fn discover_with_global(cwd: &Path, global: PathBuf) -> Result<Self> {
        let workspace_root = find_git_worktree_root(cwd)?;
        let project = workspace_root
            .as_ref()
            .map(|root| root.join(PROJECT_CONFIG_DIRECTORY).join(CONFIG_FILE_NAME));
        Ok(Self {
            global,
            project,
            workspace_root,
        })
    }

    pub fn path_for_scope(&self, scope: ConfigScope) -> Result<&Path> {
        match scope {
            ConfigScope::Global => Ok(&self.global),
            ConfigScope::Project => self.project.as_deref().ok_or_else(|| {
                anyhow!("project configuration is unavailable because the current directory is not a Git worktree")
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SettingKey {
    ConversationModel,
    ConversationReasoningEffort,
    TuiShowStartupVisual,
    TuiPlanningWorkerVisibility,
    GithubAutoDiscoverPullRequest,
    GithubReviewPollIntervalSecs,
    GithubPushRemote,
    GithubPullRequestMode,
    ParallelIntegrationBranch,
    AppServerResponseTimeoutSecs,
    AppServerPromptLog,
    SubprocessTimeoutSecs,
    DiagnosticsTrace,
    DiagnosticsSpans,
    DiagnosticsMaxFiles,
    DiagnosticsMaxFileBytes,
    DiagnosticsMaxTotalBytes,
    DiagnosticsTokioConsole,
    AdminGraphicEnabled,
    AdminGraphicPollIntervalMs,
}

impl SettingKey {
    pub const ALL: [Self; 20] = [
        Self::ConversationModel,
        Self::ConversationReasoningEffort,
        Self::TuiShowStartupVisual,
        Self::TuiPlanningWorkerVisibility,
        Self::GithubAutoDiscoverPullRequest,
        Self::GithubReviewPollIntervalSecs,
        Self::GithubPushRemote,
        Self::GithubPullRequestMode,
        Self::ParallelIntegrationBranch,
        Self::AppServerResponseTimeoutSecs,
        Self::AppServerPromptLog,
        Self::SubprocessTimeoutSecs,
        Self::DiagnosticsTrace,
        Self::DiagnosticsSpans,
        Self::DiagnosticsMaxFiles,
        Self::DiagnosticsMaxFileBytes,
        Self::DiagnosticsMaxTotalBytes,
        Self::DiagnosticsTokioConsole,
        Self::AdminGraphicEnabled,
        Self::AdminGraphicPollIntervalMs,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConversationModel => "conversation.model",
            Self::ConversationReasoningEffort => "conversation.reasoning_effort",
            Self::TuiShowStartupVisual => "tui.show_startup_visual",
            Self::TuiPlanningWorkerVisibility => "tui.planning_worker_visibility",
            Self::GithubAutoDiscoverPullRequest => "github.auto_discover_pull_request",
            Self::GithubReviewPollIntervalSecs => "github.review_poll_interval_secs",
            Self::GithubPushRemote => "github.push_remote",
            Self::GithubPullRequestMode => "github.pull_request_mode",
            Self::ParallelIntegrationBranch => "parallel.integration_branch",
            Self::AppServerResponseTimeoutSecs => "app_server.response_timeout_secs",
            Self::AppServerPromptLog => "app_server.prompt_log",
            Self::SubprocessTimeoutSecs => "subprocess.timeout_secs",
            Self::DiagnosticsTrace => "diagnostics.trace",
            Self::DiagnosticsSpans => "diagnostics.spans",
            Self::DiagnosticsMaxFiles => "diagnostics.max_files",
            Self::DiagnosticsMaxFileBytes => "diagnostics.max_file_bytes",
            Self::DiagnosticsMaxTotalBytes => "diagnostics.max_total_bytes",
            Self::DiagnosticsTokioConsole => "diagnostics.tokio_console",
            Self::AdminGraphicEnabled => "admin.graphic_enabled",
            Self::AdminGraphicPollIntervalMs => "admin.graphic_poll_interval_ms",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        let normalized = value.trim();
        Self::ALL
            .into_iter()
            .find(|key| key.as_str() == normalized)
            .ok_or_else(|| anyhow!("unknown configuration key `{normalized}`"))
    }

    pub const fn project_allowed(self) -> bool {
        !matches!(
            self,
            Self::DiagnosticsTrace
                | Self::DiagnosticsSpans
                | Self::DiagnosticsMaxFiles
                | Self::DiagnosticsMaxFileBytes
                | Self::DiagnosticsMaxTotalBytes
                | Self::DiagnosticsTokioConsole
                | Self::AppServerPromptLog
        )
    }
}

impl fmt::Display for SettingKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SettingValue {
    Text(String),
    Bool(bool),
    Unsigned(u64),
}

impl fmt::Display for SettingValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(value) => formatter.write_str(value),
            Self::Bool(value) => write!(formatter, "{value}"),
            Self::Unsigned(value) => write!(formatter, "{value}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveSetting {
    pub key: SettingKey,
    pub value: String,
    pub origin: SettingOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAkraConfig {
    pub config: AkraConfig,
    pub paths: ConfigPaths,
    origins: BTreeMap<SettingKey, SettingOrigin>,
    source_snapshot: ConfigurationSourceSnapshot,
}

impl ResolvedAkraConfig {
    pub fn origin_for(&self, key: SettingKey) -> &SettingOrigin {
        self.origins
            .get(&key)
            .expect("all known configuration keys have an origin")
    }

    pub fn effective_settings(&self) -> Vec<EffectiveSetting> {
        SettingKey::ALL
            .into_iter()
            .map(|key| EffectiveSetting {
                key,
                value: self.value_for(key).to_string(),
                origin: self.origin_for(key).clone(),
            })
            .collect()
    }

    fn value_for(&self, key: SettingKey) -> SettingValue {
        value_from_config(&self.config, key)
    }

    /// Resolve this immutable process snapshot for an explicitly selected
    /// workspace. Global, environment, and command-line layers stay pinned to
    /// process startup; only the workspace-specific Git and project-TOML
    /// layers are rebound.
    pub(crate) fn resolve_for_workspace(&self, cwd: &Path) -> Result<Self> {
        let paths = ConfigPaths::discover_with_global(cwd, self.paths.global.clone())?;
        if paths.workspace_root == self.paths.workspace_root {
            return Ok(self.clone());
        }
        ConfigurationService::resolve_with_source_snapshot(paths, self.source_snapshot.clone())
    }
}

/// Install the one immutable process snapshot used by production adapters.
/// Tests normally exercise `ConfigurationService` directly and therefore do
/// not need to touch this global.
pub fn install_process_config(config: ResolvedAkraConfig) -> Result<()> {
    INSTALLED_CONFIG
        .set(config)
        .map_err(|_| anyhow!("Akra configuration was installed more than once in one process"))
}

pub fn current_process_config() -> Option<&'static ResolvedAkraConfig> {
    INSTALLED_CONFIG.get()
}

/// Return the installed snapshot rebound to an explicitly selected workspace.
/// A missing process snapshot keeps test and standalone fallback paths intact.
pub(crate) fn current_process_config_for_workspace(
    cwd: &Path,
) -> Result<Option<ResolvedAkraConfig>> {
    current_process_config()
        .map(|config| config.resolve_for_workspace(cwd))
        .transpose()
}

/// `-c key=value` command-line override.  Parsing keeps this separate from
/// process environment, so it cannot accidentally become an inherited secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigOverride {
    pub key: SettingKey,
    value: SettingValue,
}

impl ConfigOverride {
    pub fn parse(value: &str) -> Result<Self> {
        let (raw_key, raw_value) = value
            .split_once('=')
            .ok_or_else(|| anyhow!("configuration override must use key=value: `{value}`"))?;
        let key = SettingKey::parse(raw_key)?;
        Ok(Self {
            key,
            value: parse_cli_value(key, raw_value)?,
        })
    }
}

/// Inputs captured once at process startup. Rebinding a target workspace must
/// never make a later file or environment mutation silently redefine the
/// process-level configuration contract.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigurationSourceSnapshot {
    global_layer: Option<AkraConfigLayer>,
    environment_values: BTreeMap<SettingKey, (SettingValue, String)>,
    overrides: Vec<ConfigOverride>,
}

impl ConfigurationSourceSnapshot {
    fn capture(global_path: &Path, overrides: &[ConfigOverride]) -> Result<Self> {
        Ok(Self {
            global_layer: read_layer_if_present(global_path, ConfigScope::Global)?,
            environment_values: EnvironmentConfigAdapter::from_process()?.values,
            overrides: overrides.to_vec(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedInvocation {
    pub command_args: Vec<OsString>,
    pub overrides: Vec<ConfigOverride>,
}

impl ParsedInvocation {
    pub fn parse<I, T>(args: I) -> Result<Self>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString>,
    {
        let input = args.into_iter().map(Into::into).collect::<Vec<_>>();
        let mut command_args = Vec::new();
        let mut overrides = Vec::new();
        let mut index = 0;
        while index < input.len() {
            let argument = &input[index];
            if argument == OsStr::new("-c") || argument == OsStr::new("--config") {
                let value = input
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("{} requires key=value", argument.to_string_lossy()))?;
                let value = value
                    .to_str()
                    .ok_or_else(|| anyhow!("configuration override must be valid UTF-8"))?;
                overrides.push(ConfigOverride::parse(value)?);
                index += 2;
                continue;
            }
            if let Some(value) = argument
                .to_str()
                .and_then(|value| value.strip_prefix("--config="))
            {
                overrides.push(ConfigOverride::parse(value)?);
                index += 1;
                continue;
            }
            command_args.push(argument.clone());
            index += 1;
        }
        Ok(Self {
            command_args,
            overrides,
        })
    }

    pub fn is_configuration_command(&self) -> bool {
        self.command_args.first().is_some_and(|arg| arg == "config")
    }

    pub fn is_help_only(&self) -> bool {
        matches!(
            self.command_args.as_slice(),
            [argument] if argument == "--help" || argument == "-h"
        )
    }
}

/// Configuration resolution, TOML persistence, and non-mutating diagnostics.
#[derive(Debug, Default)]
pub struct ConfigurationService;

impl ConfigurationService {
    pub fn resolve_for_startup(
        cwd: &Path,
        overrides: &[ConfigOverride],
    ) -> Result<ResolvedAkraConfig> {
        let paths = ConfigPaths::discover(cwd)?;
        ensure_global_default_file(&paths.global)?;
        Self::resolve_with_paths(paths, overrides)
    }

    /// Resolve without creating a missing global configuration file.  This is
    /// used by `config path/list/get/doctor` and tests.
    pub fn resolve_read_only(
        cwd: &Path,
        overrides: &[ConfigOverride],
    ) -> Result<ResolvedAkraConfig> {
        Self::resolve_with_paths(ConfigPaths::discover(cwd)?, overrides)
    }

    fn resolve_with_paths(
        paths: ConfigPaths,
        overrides: &[ConfigOverride],
    ) -> Result<ResolvedAkraConfig> {
        let source_snapshot = ConfigurationSourceSnapshot::capture(&paths.global, overrides)?;
        Self::resolve_with_source_snapshot(paths, source_snapshot)
    }

    fn resolve_with_source_snapshot(
        paths: ConfigPaths,
        source_snapshot: ConfigurationSourceSnapshot,
    ) -> Result<ResolvedAkraConfig> {
        let project_layer = match &paths.project {
            Some(path) => read_layer_if_present(path, ConfigScope::Project)?,
            None => None,
        };
        let legacy_values = paths
            .workspace_root
            .as_deref()
            .map(read_legacy_git_values)
            .transpose()?
            .unwrap_or_default();

        let mut config = AkraConfig::builtin();
        let mut origins = SettingKey::ALL
            .into_iter()
            .map(|key| (key, SettingOrigin::Builtin))
            .collect::<BTreeMap<_, _>>();

        if let Some(layer) = source_snapshot.global_layer.as_ref() {
            apply_layer(&mut config, &mut origins, layer, SettingOrigin::GlobalToml);
        }
        for (key, value) in legacy_values {
            apply_value(
                &mut config,
                &mut origins,
                key,
                value,
                SettingOrigin::LegacyGitConfig {
                    key: legacy_git_key(key).to_string(),
                },
            );
        }
        if let Some(layer) = project_layer.as_ref() {
            apply_layer(&mut config, &mut origins, layer, SettingOrigin::ProjectToml);
        }
        for (key, (value, variable)) in &source_snapshot.environment_values {
            apply_value(
                &mut config,
                &mut origins,
                *key,
                value.clone(),
                SettingOrigin::Environment {
                    variable: variable.clone(),
                },
            );
        }
        for override_value in &source_snapshot.overrides {
            apply_value(
                &mut config,
                &mut origins,
                override_value.key,
                override_value.value.clone(),
                SettingOrigin::CommandLine,
            );
        }

        Ok(ResolvedAkraConfig {
            config,
            paths,
            origins,
            source_snapshot,
        })
    }

    pub fn path_for_scope(cwd: &Path, scope: ConfigScope) -> Result<PathBuf> {
        Ok(ConfigPaths::discover(cwd)?
            .path_for_scope(scope)?
            .to_path_buf())
    }

    pub fn layer_for_scope(cwd: &Path, scope: ConfigScope) -> Result<Option<AkraConfigLayer>> {
        let paths = ConfigPaths::discover(cwd)?;
        read_layer_if_present(paths.path_for_scope(scope)?, scope)
    }

    pub fn set(
        cwd: &Path,
        scope: ConfigScope,
        key: SettingKey,
        raw_value: &str,
    ) -> Result<PathBuf> {
        validate_scope_for_key(scope, key)?;
        let value = parse_cli_value(key, raw_value)?;
        let paths = ConfigPaths::discover(cwd)?;
        let path = paths.path_for_scope(scope)?.to_path_buf();
        mutate_layer_file(&path, scope, |layer| {
            layer.set_value(key, value.clone());
            Ok(())
        })?;
        Ok(path)
    }

    pub fn unset(cwd: &Path, scope: ConfigScope, key: SettingKey) -> Result<PathBuf> {
        validate_scope_for_key(scope, key)?;
        let paths = ConfigPaths::discover(cwd)?;
        let path = paths.path_for_scope(scope)?.to_path_buf();
        mutate_layer_file(&path, scope, |layer| {
            layer.unset_value(key);
            Ok(())
        })?;
        Ok(path)
    }

    /// Save the two interactive defaults as one comment-preserving, locked
    /// mutation.  The TUI calls this after changing its in-memory choice, so a
    /// failed disk write never rolls the live selection back.
    pub fn set_conversation_defaults(
        cwd: &Path,
        model: Option<&str>,
        reasoning_effort: Option<&str>,
    ) -> Result<PathBuf> {
        let model = parse_cli_value(SettingKey::ConversationModel, model.unwrap_or("default"))?;
        let reasoning_effort = parse_cli_value(
            SettingKey::ConversationReasoningEffort,
            reasoning_effort.unwrap_or("default"),
        )?;
        let paths = ConfigPaths::discover(cwd)?;
        let path = paths.global;
        mutate_layer_file(&path, ConfigScope::Global, |layer| {
            layer.set_value(SettingKey::ConversationModel, model.clone());
            layer.set_value(
                SettingKey::ConversationReasoningEffort,
                reasoning_effort.clone(),
            );
            Ok(())
        })?;
        Ok(path)
    }

    pub fn doctor(cwd: &Path, overrides: &[ConfigOverride]) -> ConfigDoctorReport {
        let paths = match ConfigPaths::discover(cwd) {
            Ok(paths) => paths,
            Err(error) => return ConfigDoctorReport::path_error(error.to_string()),
        };
        let global = inspect_layer_file(&paths.global, ConfigScope::Global);
        let project = paths
            .project
            .as_ref()
            .map(|path| inspect_layer_file(path, ConfigScope::Project));
        let environment = EnvironmentConfigAdapter::from_process();
        let legacy = paths
            .workspace_root
            .as_deref()
            .map(read_legacy_git_values)
            .transpose();
        let resolution_error = Self::resolve_with_paths(paths.clone(), overrides)
            .err()
            .map(|error| error.to_string());
        ConfigDoctorReport {
            paths: Some(paths),
            global,
            project,
            environment: environment
                .map(|adapter| adapter.shadow_descriptions)
                .unwrap_or_else(|error| vec![format!("environment error: {error}")]),
            legacy: legacy
                .map(|values| {
                    values
                        .unwrap_or_default()
                        .into_keys()
                        .map(|key| format!("{} -> {}", legacy_git_key(key), key))
                        .collect()
                })
                .unwrap_or_else(|error| vec![format!("legacy Git config error: {error}")]),
            resolution_error,
        }
    }
}

impl AkraConfigLayer {
    pub fn configured_value(&self, key: SettingKey) -> Option<String> {
        self.value_for(key).map(|value| value.to_string())
    }

    fn canonical_defaults() -> Self {
        let config = AkraConfig::builtin();
        let mut layer = Self {
            schema_version: Some(CONFIG_SCHEMA_VERSION),
            ..Self::default()
        };
        for key in SettingKey::ALL {
            layer.set_value(key, value_from_config(&config, key));
        }
        layer
    }

    fn value_for(&self, key: SettingKey) -> Option<SettingValue> {
        match key {
            SettingKey::ConversationModel => self
                .conversation
                .as_ref()?
                .model
                .clone()
                .map(SettingValue::Text),
            SettingKey::ConversationReasoningEffort => self
                .conversation
                .as_ref()?
                .reasoning_effort
                .clone()
                .map(SettingValue::Text),
            SettingKey::TuiShowStartupVisual => self
                .tui
                .as_ref()?
                .show_startup_visual
                .map(SettingValue::Bool),
            SettingKey::TuiPlanningWorkerVisibility => self
                .tui
                .as_ref()?
                .planning_worker_visibility
                .clone()
                .map(SettingValue::Text),
            SettingKey::GithubAutoDiscoverPullRequest => self
                .github
                .as_ref()?
                .auto_discover_pull_request
                .map(SettingValue::Bool),
            SettingKey::GithubReviewPollIntervalSecs => self
                .github
                .as_ref()?
                .review_poll_interval_secs
                .map(SettingValue::Unsigned),
            SettingKey::GithubPushRemote => self
                .github
                .as_ref()?
                .push_remote
                .clone()
                .map(SettingValue::Text),
            SettingKey::GithubPullRequestMode => self
                .github
                .as_ref()?
                .pull_request_mode
                .clone()
                .map(SettingValue::Text),
            SettingKey::ParallelIntegrationBranch => self
                .parallel
                .as_ref()?
                .integration_branch
                .clone()
                .map(SettingValue::Text),
            SettingKey::AppServerResponseTimeoutSecs => self
                .app_server
                .as_ref()?
                .response_timeout_secs
                .map(SettingValue::Unsigned),
            SettingKey::AppServerPromptLog => {
                self.app_server.as_ref()?.prompt_log.map(SettingValue::Bool)
            }
            SettingKey::SubprocessTimeoutSecs => self
                .subprocess
                .as_ref()?
                .timeout_secs
                .map(SettingValue::Unsigned),
            SettingKey::DiagnosticsTrace => self
                .diagnostics
                .as_ref()?
                .trace
                .clone()
                .map(SettingValue::Text),
            SettingKey::DiagnosticsSpans => self
                .diagnostics
                .as_ref()?
                .spans
                .clone()
                .map(SettingValue::Text),
            SettingKey::DiagnosticsMaxFiles => self
                .diagnostics
                .as_ref()?
                .max_files
                .map(SettingValue::Unsigned),
            SettingKey::DiagnosticsMaxFileBytes => self
                .diagnostics
                .as_ref()?
                .max_file_bytes
                .map(SettingValue::Unsigned),
            SettingKey::DiagnosticsMaxTotalBytes => self
                .diagnostics
                .as_ref()?
                .max_total_bytes
                .map(SettingValue::Unsigned),
            SettingKey::DiagnosticsTokioConsole => self
                .diagnostics
                .as_ref()?
                .tokio_console
                .map(SettingValue::Bool),
            SettingKey::AdminGraphicEnabled => {
                self.admin.as_ref()?.graphic_enabled.map(SettingValue::Bool)
            }
            SettingKey::AdminGraphicPollIntervalMs => self
                .admin
                .as_ref()?
                .graphic_poll_interval_ms
                .map(SettingValue::Unsigned),
        }
    }

    fn set_value(&mut self, key: SettingKey, value: SettingValue) {
        match (key, value) {
            (SettingKey::ConversationModel, SettingValue::Text(value)) => {
                self.conversation.get_or_insert_default().model = Some(value)
            }
            (SettingKey::ConversationReasoningEffort, SettingValue::Text(value)) => {
                self.conversation.get_or_insert_default().reasoning_effort = Some(value)
            }
            (SettingKey::TuiShowStartupVisual, SettingValue::Bool(value)) => {
                self.tui.get_or_insert_default().show_startup_visual = Some(value)
            }
            (SettingKey::TuiPlanningWorkerVisibility, SettingValue::Text(value)) => {
                self.tui.get_or_insert_default().planning_worker_visibility = Some(value)
            }
            (SettingKey::GithubAutoDiscoverPullRequest, SettingValue::Bool(value)) => {
                self.github
                    .get_or_insert_default()
                    .auto_discover_pull_request = Some(value)
            }
            (SettingKey::GithubReviewPollIntervalSecs, SettingValue::Unsigned(value)) => {
                self.github
                    .get_or_insert_default()
                    .review_poll_interval_secs = Some(value)
            }
            (SettingKey::GithubPushRemote, SettingValue::Text(value)) => {
                self.github.get_or_insert_default().push_remote = Some(value)
            }
            (SettingKey::GithubPullRequestMode, SettingValue::Text(value)) => {
                self.github.get_or_insert_default().pull_request_mode = Some(value)
            }
            (SettingKey::ParallelIntegrationBranch, SettingValue::Text(value)) => {
                self.parallel.get_or_insert_default().integration_branch = Some(value)
            }
            (SettingKey::AppServerResponseTimeoutSecs, SettingValue::Unsigned(value)) => {
                self.app_server
                    .get_or_insert_default()
                    .response_timeout_secs = Some(value)
            }
            (SettingKey::AppServerPromptLog, SettingValue::Bool(value)) => {
                self.app_server.get_or_insert_default().prompt_log = Some(value)
            }
            (SettingKey::SubprocessTimeoutSecs, SettingValue::Unsigned(value)) => {
                self.subprocess.get_or_insert_default().timeout_secs = Some(value)
            }
            (SettingKey::DiagnosticsTrace, SettingValue::Text(value)) => {
                self.diagnostics.get_or_insert_default().trace = Some(value)
            }
            (SettingKey::DiagnosticsSpans, SettingValue::Text(value)) => {
                self.diagnostics.get_or_insert_default().spans = Some(value)
            }
            (SettingKey::DiagnosticsMaxFiles, SettingValue::Unsigned(value)) => {
                self.diagnostics.get_or_insert_default().max_files = Some(value)
            }
            (SettingKey::DiagnosticsMaxFileBytes, SettingValue::Unsigned(value)) => {
                self.diagnostics.get_or_insert_default().max_file_bytes = Some(value)
            }
            (SettingKey::DiagnosticsMaxTotalBytes, SettingValue::Unsigned(value)) => {
                self.diagnostics.get_or_insert_default().max_total_bytes = Some(value)
            }
            (SettingKey::DiagnosticsTokioConsole, SettingValue::Bool(value)) => {
                self.diagnostics.get_or_insert_default().tokio_console = Some(value)
            }
            (SettingKey::AdminGraphicEnabled, SettingValue::Bool(value)) => {
                self.admin.get_or_insert_default().graphic_enabled = Some(value)
            }
            (SettingKey::AdminGraphicPollIntervalMs, SettingValue::Unsigned(value)) => {
                self.admin.get_or_insert_default().graphic_poll_interval_ms = Some(value)
            }
            _ => unreachable!("setting key and parsed setting value must agree"),
        }
    }

    fn unset_value(&mut self, key: SettingKey) {
        match key {
            SettingKey::ConversationModel => {
                set_option_field(&mut self.conversation, |value| &mut value.model)
            }
            SettingKey::ConversationReasoningEffort => {
                set_option_field(&mut self.conversation, |value| &mut value.reasoning_effort)
            }
            SettingKey::TuiShowStartupVisual => {
                set_option_field(&mut self.tui, |value| &mut value.show_startup_visual)
            }
            SettingKey::TuiPlanningWorkerVisibility => {
                set_option_field(&mut self.tui, |value| &mut value.planning_worker_visibility)
            }
            SettingKey::GithubAutoDiscoverPullRequest => {
                set_option_field(&mut self.github, |value| {
                    &mut value.auto_discover_pull_request
                })
            }
            SettingKey::GithubReviewPollIntervalSecs => {
                set_option_field(&mut self.github, |value| {
                    &mut value.review_poll_interval_secs
                })
            }
            SettingKey::GithubPushRemote => {
                set_option_field(&mut self.github, |value| &mut value.push_remote)
            }
            SettingKey::GithubPullRequestMode => {
                set_option_field(&mut self.github, |value| &mut value.pull_request_mode)
            }
            SettingKey::ParallelIntegrationBranch => {
                set_option_field(&mut self.parallel, |value| &mut value.integration_branch)
            }
            SettingKey::AppServerResponseTimeoutSecs => {
                set_option_field(&mut self.app_server, |value| {
                    &mut value.response_timeout_secs
                })
            }
            SettingKey::AppServerPromptLog => {
                set_option_field(&mut self.app_server, |value| &mut value.prompt_log)
            }
            SettingKey::SubprocessTimeoutSecs => {
                set_option_field(&mut self.subprocess, |value| &mut value.timeout_secs)
            }
            SettingKey::DiagnosticsTrace => {
                set_option_field(&mut self.diagnostics, |value| &mut value.trace)
            }
            SettingKey::DiagnosticsSpans => {
                set_option_field(&mut self.diagnostics, |value| &mut value.spans)
            }
            SettingKey::DiagnosticsMaxFiles => {
                set_option_field(&mut self.diagnostics, |value| &mut value.max_files)
            }
            SettingKey::DiagnosticsMaxFileBytes => {
                set_option_field(&mut self.diagnostics, |value| &mut value.max_file_bytes)
            }
            SettingKey::DiagnosticsMaxTotalBytes => {
                set_option_field(&mut self.diagnostics, |value| &mut value.max_total_bytes)
            }
            SettingKey::DiagnosticsTokioConsole => {
                set_option_field(&mut self.diagnostics, |value| &mut value.tokio_console)
            }
            SettingKey::AdminGraphicEnabled => {
                set_option_field(&mut self.admin, |value| &mut value.graphic_enabled)
            }
            SettingKey::AdminGraphicPollIntervalMs => {
                set_option_field(&mut self.admin, |value| &mut value.graphic_poll_interval_ms)
            }
        }
    }
}

fn set_option_field<T, F, U>(section: &mut Option<T>, field: F)
where
    F: FnOnce(&mut T) -> &mut Option<U>,
{
    if let Some(section) = section.as_mut() {
        *field(section) = None;
    }
}

fn apply_layer(
    config: &mut AkraConfig,
    origins: &mut BTreeMap<SettingKey, SettingOrigin>,
    layer: &AkraConfigLayer,
    origin: SettingOrigin,
) {
    for key in SettingKey::ALL {
        if let Some(value) = layer.value_for(key) {
            apply_value(config, origins, key, value, origin.clone());
        }
    }
}

fn apply_value(
    config: &mut AkraConfig,
    origins: &mut BTreeMap<SettingKey, SettingOrigin>,
    key: SettingKey,
    value: SettingValue,
    origin: SettingOrigin,
) {
    match (key, value) {
        (SettingKey::ConversationModel, SettingValue::Text(value)) => {
            config.conversation.model = (value != "default").then_some(value)
        }
        (SettingKey::ConversationReasoningEffort, SettingValue::Text(value)) => {
            config.conversation.reasoning_effort = (value != "default").then_some(value)
        }
        (SettingKey::TuiShowStartupVisual, SettingValue::Bool(value)) => {
            config.tui.show_startup_visual = value
        }
        (SettingKey::TuiPlanningWorkerVisibility, SettingValue::Text(value)) => {
            config.tui.planning_worker_visibility = value
        }
        (SettingKey::GithubAutoDiscoverPullRequest, SettingValue::Bool(value)) => {
            config.github.auto_discover_pull_request = value
        }
        (SettingKey::GithubReviewPollIntervalSecs, SettingValue::Unsigned(value)) => {
            config.github.review_poll_interval_secs = value
        }
        (SettingKey::GithubPushRemote, SettingValue::Text(value)) => {
            config.github.push_remote = value
        }
        (SettingKey::GithubPullRequestMode, SettingValue::Text(value)) => {
            config.github.pull_request_mode = value
        }
        (SettingKey::ParallelIntegrationBranch, SettingValue::Text(value)) => {
            config.parallel.integration_branch = value
        }
        (SettingKey::AppServerResponseTimeoutSecs, SettingValue::Unsigned(value)) => {
            config.app_server.response_timeout_secs = value
        }
        (SettingKey::AppServerPromptLog, SettingValue::Bool(value)) => {
            config.app_server.prompt_log = value
        }
        (SettingKey::SubprocessTimeoutSecs, SettingValue::Unsigned(value)) => {
            config.subprocess.timeout_secs = value
        }
        (SettingKey::DiagnosticsTrace, SettingValue::Text(value)) => {
            config.diagnostics.trace = value
        }
        (SettingKey::DiagnosticsSpans, SettingValue::Text(value)) => {
            config.diagnostics.spans = value
        }
        (SettingKey::DiagnosticsMaxFiles, SettingValue::Unsigned(value)) => {
            config.diagnostics.max_files = value
        }
        (SettingKey::DiagnosticsMaxFileBytes, SettingValue::Unsigned(value)) => {
            config.diagnostics.max_file_bytes = value
        }
        (SettingKey::DiagnosticsMaxTotalBytes, SettingValue::Unsigned(value)) => {
            config.diagnostics.max_total_bytes = value
        }
        (SettingKey::DiagnosticsTokioConsole, SettingValue::Bool(value)) => {
            config.diagnostics.tokio_console = value
        }
        (SettingKey::AdminGraphicEnabled, SettingValue::Bool(value)) => {
            config.admin.graphic_enabled = value
        }
        (SettingKey::AdminGraphicPollIntervalMs, SettingValue::Unsigned(value)) => {
            config.admin.graphic_poll_interval_ms = value
        }
        _ => unreachable!("setting key and validated setting value must agree"),
    }
    origins.insert(key, origin);
}

fn value_from_config(config: &AkraConfig, key: SettingKey) -> SettingValue {
    match key {
        SettingKey::ConversationModel => SettingValue::Text(
            config
                .conversation
                .model
                .clone()
                .unwrap_or_else(|| "default".to_string()),
        ),
        SettingKey::ConversationReasoningEffort => SettingValue::Text(
            config
                .conversation
                .reasoning_effort
                .clone()
                .unwrap_or_else(|| "default".to_string()),
        ),
        SettingKey::TuiShowStartupVisual => SettingValue::Bool(config.tui.show_startup_visual),
        SettingKey::TuiPlanningWorkerVisibility => {
            SettingValue::Text(config.tui.planning_worker_visibility.clone())
        }
        SettingKey::GithubAutoDiscoverPullRequest => {
            SettingValue::Bool(config.github.auto_discover_pull_request)
        }
        SettingKey::GithubReviewPollIntervalSecs => {
            SettingValue::Unsigned(config.github.review_poll_interval_secs)
        }
        SettingKey::GithubPushRemote => SettingValue::Text(config.github.push_remote.clone()),
        SettingKey::GithubPullRequestMode => {
            SettingValue::Text(config.github.pull_request_mode.clone())
        }
        SettingKey::ParallelIntegrationBranch => {
            SettingValue::Text(config.parallel.integration_branch.clone())
        }
        SettingKey::AppServerResponseTimeoutSecs => {
            SettingValue::Unsigned(config.app_server.response_timeout_secs)
        }
        SettingKey::AppServerPromptLog => SettingValue::Bool(config.app_server.prompt_log),
        SettingKey::SubprocessTimeoutSecs => SettingValue::Unsigned(config.subprocess.timeout_secs),
        SettingKey::DiagnosticsTrace => SettingValue::Text(config.diagnostics.trace.clone()),
        SettingKey::DiagnosticsSpans => SettingValue::Text(config.diagnostics.spans.clone()),
        SettingKey::DiagnosticsMaxFiles => SettingValue::Unsigned(config.diagnostics.max_files),
        SettingKey::DiagnosticsMaxFileBytes => {
            SettingValue::Unsigned(config.diagnostics.max_file_bytes)
        }
        SettingKey::DiagnosticsMaxTotalBytes => {
            SettingValue::Unsigned(config.diagnostics.max_total_bytes)
        }
        SettingKey::DiagnosticsTokioConsole => SettingValue::Bool(config.diagnostics.tokio_console),
        SettingKey::AdminGraphicEnabled => SettingValue::Bool(config.admin.graphic_enabled),
        SettingKey::AdminGraphicPollIntervalMs => {
            SettingValue::Unsigned(config.admin.graphic_poll_interval_ms)
        }
    }
}

fn validate_scope_for_key(scope: ConfigScope, key: SettingKey) -> Result<()> {
    if scope == ConfigScope::Project && !key.project_allowed() {
        bail!(
            "{} is not permitted in project configuration; keep this machine-level setting in the global config",
            key
        );
    }
    Ok(())
}

fn validate_layer(layer: &AkraConfigLayer, scope: ConfigScope) -> Result<()> {
    if let Some(version) = layer.schema_version
        && version != CONFIG_SCHEMA_VERSION
    {
        bail!("unsupported config schema_version {version}; expected {CONFIG_SCHEMA_VERSION}");
    }
    for key in SettingKey::ALL {
        if let Some(value) = layer.value_for(key) {
            validate_scope_for_key(scope, key)?;
            validate_setting_value(key, value)?;
        }
    }
    Ok(())
}

fn parse_cli_value(key: SettingKey, raw_value: &str) -> Result<SettingValue> {
    let value = match key {
        SettingKey::TuiShowStartupVisual
        | SettingKey::GithubAutoDiscoverPullRequest
        | SettingKey::AppServerPromptLog
        | SettingKey::DiagnosticsTokioConsole
        | SettingKey::AdminGraphicEnabled => SettingValue::Bool(parse_bool(raw_value)?),
        SettingKey::GithubReviewPollIntervalSecs
        | SettingKey::AppServerResponseTimeoutSecs
        | SettingKey::SubprocessTimeoutSecs
        | SettingKey::DiagnosticsMaxFiles
        | SettingKey::DiagnosticsMaxFileBytes
        | SettingKey::DiagnosticsMaxTotalBytes
        | SettingKey::AdminGraphicPollIntervalMs => {
            SettingValue::Unsigned(parse_unsigned(raw_value, key)?)
        }
        _ => SettingValue::Text(raw_value.trim().to_string()),
    };
    validate_setting_value(key, value.clone())?;
    Ok(value)
}

fn validate_setting_value(key: SettingKey, value: SettingValue) -> Result<()> {
    match (key, value) {
        (SettingKey::ConversationModel, SettingValue::Text(value)) => validate_model_value(&value),
        (SettingKey::ConversationReasoningEffort, SettingValue::Text(value)) => {
            validate_enum_value(
                key,
                &value,
                &[
                    "default", "none", "minimal", "low", "medium", "high", "xhigh", "max",
                ],
            )
        }
        (SettingKey::TuiPlanningWorkerVisibility, SettingValue::Text(value)) => {
            validate_enum_value(key, &value, &["normal", "debug"])
        }
        (SettingKey::GithubPushRemote, SettingValue::Text(value)) => validate_remote_name(&value),
        (SettingKey::GithubPullRequestMode, SettingValue::Text(value)) => {
            validate_enum_value(key, &value, &["required", "auto", "disabled"])
        }
        (SettingKey::ParallelIntegrationBranch, SettingValue::Text(value)) => {
            validate_branch_name(&value)
        }
        (SettingKey::DiagnosticsTrace, SettingValue::Text(value)) => validate_trace_value(&value),
        (SettingKey::DiagnosticsSpans, SettingValue::Text(value)) => {
            validate_enum_value(key, &value, &["none", "close", "full", "0", "off"])
        }
        (SettingKey::GithubReviewPollIntervalSecs, SettingValue::Unsigned(value))
            if (1..=86_400).contains(&value) =>
        {
            Ok(())
        }
        (SettingKey::AppServerResponseTimeoutSecs, SettingValue::Unsigned(value))
            if (1..=300).contains(&value) =>
        {
            Ok(())
        }
        (SettingKey::SubprocessTimeoutSecs, SettingValue::Unsigned(value))
            if (1..=86_400).contains(&value) =>
        {
            Ok(())
        }
        (SettingKey::DiagnosticsMaxFiles, SettingValue::Unsigned(value))
            if (1..=365).contains(&value) =>
        {
            Ok(())
        }
        (SettingKey::DiagnosticsMaxFileBytes, SettingValue::Unsigned(value))
            if (64 * 1024..=1024 * 1024 * 1024).contains(&value) =>
        {
            Ok(())
        }
        (SettingKey::DiagnosticsMaxTotalBytes, SettingValue::Unsigned(value))
            if (64 * 1024..=4 * 1024 * 1024 * 1024).contains(&value) =>
        {
            Ok(())
        }
        (SettingKey::AdminGraphicPollIntervalMs, SettingValue::Unsigned(value))
            if value >= 5_000 =>
        {
            Ok(())
        }
        (
            SettingKey::TuiShowStartupVisual
            | SettingKey::GithubAutoDiscoverPullRequest
            | SettingKey::AppServerPromptLog
            | SettingKey::DiagnosticsTokioConsole
            | SettingKey::AdminGraphicEnabled,
            SettingValue::Bool(_),
        ) => Ok(()),
        (key, SettingValue::Unsigned(value)) => {
            bail!("{} has an out-of-range value `{value}`", key)
        }
        (key, SettingValue::Text(value)) => {
            bail!("{} has an invalid value `{value}`", key)
        }
        (key, value) => bail!("{} has the wrong value type `{value}`", key),
    }
}

fn validate_model_value(value: &str) -> Result<()> {
    if value == "default" {
        return Ok(());
    }
    validate_bounded_text("conversation.model", value)?;
    if value.chars().any(|character| character.is_whitespace()) {
        bail!("conversation.model must not contain whitespace")
    }
    Ok(())
}

fn validate_trace_value(value: &str) -> Result<()> {
    if value.trim() != value {
        bail!("diagnostics.trace must not have leading or trailing whitespace")
    }
    validate_bounded_text("diagnostics.trace", value)?;
    if matches!(
        value.to_ascii_lowercase().as_str(),
        "off" | "0" | "false" | "no" | "on" | "1" | "true" | "yes" | "planning" | "full"
    ) {
        return Ok(());
    }
    if EnvFilter::try_new(value).is_ok() {
        return Ok(());
    }
    bail!("diagnostics.trace must be off, on, planning, full, or a valid tracing filter")
}

fn validate_remote_name(value: &str) -> Result<()> {
    validate_bounded_text("github.push_remote", value)?;
    if value.starts_with('-')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        bail!("github.push_remote must be a Git remote name")
    }
    Ok(())
}

fn validate_branch_name(value: &str) -> Result<()> {
    validate_bounded_text("parallel.integration_branch", value)?;
    if value.starts_with('-')
        || value.contains("..")
        || value.ends_with('.')
        || value.contains("@{")
        || value.chars().any(|character| {
            character.is_whitespace()
                || character.is_control()
                || matches!(character, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
        })
    {
        bail!("parallel.integration_branch must be a safe Git branch name")
    }
    Ok(())
}

fn validate_enum_value(key: SettingKey, value: &str, allowed: &[&str]) -> Result<()> {
    if value.trim() != value {
        bail!("{} must not have leading or trailing whitespace", key)
    }
    if allowed.contains(&value) {
        Ok(())
    } else {
        bail!("{} is invalid; expected one of {}", key, allowed.join(", "))
    }
}

fn validate_bounded_text(name: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.chars().count() > MAX_TEXT_VALUE_CHARS
        || value.chars().any(char::is_control)
    {
        bail!("{name} must be a non-empty bounded text value")
    }
    Ok(())
}

fn parse_bool(value: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => bail!("expected a boolean value, got `{}`", value.trim()),
    }
}

fn parse_unsigned(value: &str, key: SettingKey) -> Result<u64> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|_| anyhow!("{} must be a positive integer", key))
}

fn global_config_path() -> Result<PathBuf> {
    let home = match std::env::var_os("AKRA_HOME") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => default_akra_home()?,
    };
    if !home.is_absolute() {
        bail!("AKRA_HOME must be an absolute path")
    }
    Ok(home.join(CONFIG_FILE_NAME))
}

fn default_akra_home() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        if let Some(home) = std::env::var_os("USERPROFILE").filter(|home| !home.is_empty()) {
            return Ok(PathBuf::from(home).join(".akra"));
        }
    }
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".akra"))
        .ok_or_else(|| anyhow!("AKRA_HOME is unset and the user home directory is unavailable"))
}

fn find_git_worktree_root(cwd: &Path) -> Result<Option<PathBuf>> {
    let mut candidate = cwd
        .canonicalize()
        .with_context(|| format!("failed to resolve working directory {}", cwd.display()))?;
    if !candidate.is_dir() {
        bail!(
            "working directory is not a directory: {}",
            candidate.display()
        )
    }
    loop {
        let marker = candidate.join(".git");
        if let Ok(metadata) = fs::symlink_metadata(&marker)
            && (metadata.file_type().is_dir() || metadata.file_type().is_file())
        {
            return Ok(Some(candidate));
        }
        let Some(parent) = candidate.parent().map(Path::to_path_buf) else {
            return Ok(None);
        };
        if parent == candidate {
            return Ok(None);
        }
        candidate = parent;
    }
}

fn read_layer_if_present(path: &Path, scope: ConfigScope) -> Result<Option<AkraConfigLayer>> {
    let Some(contents) = read_config_file_if_present(path, scope)? else {
        return Ok(None);
    };
    let layer = toml::from_str::<AkraConfigLayer>(&contents).map_err(|error| {
        anyhow!(
            "invalid {} configuration TOML at {}: {error}",
            scope.label(),
            path.display()
        )
    })?;
    validate_layer(&layer, scope).map_err(|error| {
        anyhow!(
            "invalid {} configuration at {}: {error}",
            scope.label(),
            path.display()
        )
    })?;
    Ok(Some(layer))
}

fn read_config_file_if_present(path: &Path, scope: ConfigScope) -> Result<Option<String>> {
    // Reject a link/reparse-point parent before querying the leaf.  O_NOFOLLOW
    // protects the final component on Unix, but it cannot protect `.akra` (or
    // an operator-provided AKRA_HOME) when that directory itself is a link.
    validate_config_parent_for_read(path, scope)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect configuration {}", path.display()));
        }
    };
    validate_config_metadata(path, &metadata, scope)?;
    let mut file = open_config_file_for_read(path)?;
    validate_open_config_identity(path, &metadata, &file, scope)?;
    let mut contents = String::new();
    Read::by_ref(&mut file)
        .take(MAX_CONFIG_BYTES + 1)
        .read_to_string(&mut contents)
        .with_context(|| format!("failed to read configuration {}", path.display()))?;
    if contents.len() as u64 > MAX_CONFIG_BYTES {
        bail!(
            "configuration exceeds {} byte limit: {}",
            MAX_CONFIG_BYTES,
            path.display()
        )
    }
    let final_metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to recheck configuration {}", path.display()))?;
    validate_open_config_identity(path, &final_metadata, &file, scope)?;
    Ok(Some(contents))
}

fn validate_config_parent_for_read(path: &Path, scope: ConfigScope) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("configuration path has no parent: {}", path.display()))?;
    let metadata = match fs::symlink_metadata(parent) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to inspect configuration directory {}",
                    parent.display()
                )
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "configuration parent must be a real directory: {}",
            parent.display()
        )
    }
    #[cfg(unix)]
    if scope == ConfigScope::Global {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } {
            bail!(
                "global configuration directory must be owned by the current user: {}",
                parent.display()
            )
        }
        if metadata.mode() & 0o022 != 0 {
            bail!(
                "global configuration directory must not be group- or world-writable: {}",
                parent.display()
            )
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let directory = OpenOptions::new()
            .read(true)
            .access_mode(
                crate::private_fs::WINDOWS_GENERIC_READ | crate::private_fs::WINDOWS_READ_CONTROL,
            )
            .share_mode(crate::private_fs::WINDOWS_FILE_SHARE_ALL)
            .custom_flags(
                crate::private_fs::WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT
                    | crate::private_fs::WINDOWS_FILE_FLAG_BACKUP_SEMANTICS,
            )
            .open(parent)
            .with_context(|| {
                format!(
                    "failed to open configuration directory {}",
                    parent.display()
                )
            })?;
        crate::private_fs::validate_windows_path_identity_only(parent, &directory, true)?;
        if scope == ConfigScope::Global {
            crate::private_fs::validate_windows_private_owner_and_acl(parent, &directory)?;
        }
    }
    Ok(())
}

fn open_config_file_for_read(path: &Path) -> Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .with_context(|| format!("failed to safely open configuration {}", path.display()))
    }
    #[cfg(windows)]
    {
        OpenOptions::new()
            .read(true)
            .open(path)
            .with_context(|| format!("failed to safely open configuration {}", path.display()))
    }
    #[cfg(not(any(unix, windows)))]
    {
        OpenOptions::new()
            .read(true)
            .open(path)
            .with_context(|| format!("failed to safely open configuration {}", path.display()))
    }
}

fn validate_config_metadata(
    path: &Path,
    metadata: &fs::Metadata,
    scope: ConfigScope,
) -> Result<()> {
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "configuration must be a regular non-symlink file: {}",
            path.display()
        )
    }
    if metadata.len() > MAX_CONFIG_BYTES {
        bail!(
            "configuration exceeds {} byte limit: {}",
            MAX_CONFIG_BYTES,
            path.display()
        )
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            bail!("configuration must not be hard-linked: {}", path.display())
        }
        if scope == ConfigScope::Global {
            if metadata.uid() != unsafe { libc::geteuid() } {
                bail!(
                    "global configuration must be owned by the current user: {}",
                    path.display()
                )
            }
            if metadata.mode() & 0o077 != 0 {
                bail!(
                    "global configuration must be owner-only (0600): {}",
                    path.display()
                )
            }
        }
    }
    #[cfg(windows)]
    {
        let file = open_config_file_for_read(path)?;
        crate::private_fs::validate_windows_path_identity_only(path, &file, false)?;
        if scope == ConfigScope::Global {
            crate::private_fs::validate_windows_private_owner_and_acl(path, &file)?;
        }
    }
    Ok(())
}

fn validate_open_config_identity(
    path: &Path,
    _expected: &fs::Metadata,
    file: &File,
    scope: ConfigScope,
) -> Result<()> {
    let opened = file
        .metadata()
        .with_context(|| format!("failed to inspect opened configuration {}", path.display()))?;
    validate_config_metadata(path, &opened, scope)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if _expected.dev() != opened.dev() || _expected.ino() != opened.ino() {
            bail!(
                "configuration changed while being opened: {}",
                path.display()
            )
        }
    }
    #[cfg(windows)]
    {
        crate::private_fs::validate_windows_path_identity_only(path, file, false)?;
    }
    Ok(())
}

fn ensure_global_default_file(path: &Path) -> Result<()> {
    ensure_config_parent(path, ConfigScope::Global)?;
    let _lock = ConfigWriteLock::acquire(path)?;
    if read_config_file_if_present(path, ConfigScope::Global)?.is_none() {
        let layer = AkraConfigLayer::canonical_defaults();
        let contents = toml::to_string_pretty(&layer)
            .context("failed to serialize canonical global configuration")?;
        atomic_write_config(path, ConfigScope::Global, &contents)?;
    }
    Ok(())
}

fn mutate_layer_file(
    path: &Path,
    scope: ConfigScope,
    mutate: impl FnOnce(&mut AkraConfigLayer) -> Result<()>,
) -> Result<()> {
    ensure_config_parent(path, scope)?;
    let _lock = ConfigWriteLock::acquire(path)?;
    let before = read_config_file_if_present(path, scope)?;
    let mut layer = match before.as_deref() {
        Some(contents) => toml::from_str::<AkraConfigLayer>(contents).map_err(|error| {
            anyhow!(
                "invalid {} configuration TOML at {}: {error}",
                scope.label(),
                path.display()
            )
        })?,
        None => AkraConfigLayer::default(),
    };
    validate_layer(&layer, scope)?;
    mutate(&mut layer)?;
    layer.schema_version = Some(CONFIG_SCHEMA_VERSION);
    validate_layer(&layer, scope)?;

    let mut document = match before {
        Some(contents) => contents.parse::<DocumentMut>().with_context(|| {
            format!(
                "invalid {} configuration TOML at {}",
                scope.label(),
                path.display()
            )
        })?,
        None => DocumentMut::new(),
    };
    document["schema_version"] = value(CONFIG_SCHEMA_VERSION as i64);
    sync_document_layer(&mut document, &layer);
    let contents = document.to_string();
    // Parse the exact post-edit text once before replacing the target.  This
    // catches an unexpected toml_edit shape change without damaging the old file.
    let parsed = toml::from_str::<AkraConfigLayer>(&contents)
        .context("failed to validate rewritten configuration TOML")?;
    validate_layer(&parsed, scope)?;
    atomic_write_config(path, scope, &contents)
}

fn sync_document_layer(document: &mut DocumentMut, layer: &AkraConfigLayer) {
    for key in SettingKey::ALL {
        match layer.value_for(key) {
            Some(value) => set_document_value(document, key, value),
            None => remove_document_value(document, key),
        }
    }
}

fn set_document_value(document: &mut DocumentMut, key: SettingKey, setting_value: SettingValue) {
    let (section, field) = key
        .as_str()
        .split_once('.')
        .expect("known setting keys contain a section separator");
    document[section][field] = match setting_value {
        SettingValue::Text(text) => toml_edit::value(text),
        SettingValue::Bool(enabled) => toml_edit::value(enabled),
        SettingValue::Unsigned(number) => toml_edit::value(number as i64),
    };
}

fn remove_document_value(document: &mut DocumentMut, key: SettingKey) {
    let (section, field) = key
        .as_str()
        .split_once('.')
        .expect("known setting keys contain a section separator");
    let Some(section_item) = document.get_mut(section) else {
        return;
    };
    if let Some(table) = section_item.as_table_mut() {
        table.remove(field);
    }
    let remove_section = document
        .get(section)
        .and_then(Item::as_table)
        .is_some_and(|table| table.is_empty());
    if remove_section {
        document.remove(section);
    }
}

fn ensure_config_parent(path: &Path, scope: ConfigScope) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("configuration path has no parent: {}", path.display()))?;
    let parent_existed = parent.exists();
    if !parent_existed {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create configuration directory {}",
                parent.display()
            )
        })?;
    }
    let metadata = fs::symlink_metadata(parent).with_context(|| {
        format!(
            "failed to inspect configuration directory {}",
            parent.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "configuration parent must be a real directory: {}",
            parent.display()
        )
    }
    #[cfg(unix)]
    if scope == ConfigScope::Global {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() } {
            bail!(
                "global configuration directory must be owned by the current user: {}",
                parent.display()
            )
        }
        if metadata.mode() & 0o022 != 0 {
            bail!(
                "global configuration directory must not be group- or world-writable: {}",
                parent.display()
            )
        }
        if !parent_existed && metadata.mode() & 0o077 != 0 {
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).with_context(|| {
                format!(
                    "failed to restrict global configuration directory {}",
                    parent.display()
                )
            })?;
        }
    }
    #[cfg(windows)]
    if scope == ConfigScope::Global {
        use std::os::windows::fs::OpenOptionsExt;
        let directory = OpenOptions::new()
            .read(true)
            .write(true)
            .access_mode(
                crate::private_fs::WINDOWS_GENERIC_READ
                    | crate::private_fs::WINDOWS_GENERIC_WRITE
                    | crate::private_fs::WINDOWS_READ_CONTROL
                    | crate::private_fs::WINDOWS_WRITE_DAC,
            )
            .share_mode(crate::private_fs::WINDOWS_FILE_SHARE_ALL)
            .custom_flags(
                crate::private_fs::WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT
                    | crate::private_fs::WINDOWS_FILE_FLAG_BACKUP_SEMANTICS,
            )
            .open(parent)
            .with_context(|| {
                format!(
                    "failed to open global configuration directory {}",
                    parent.display()
                )
            })?;
        crate::private_fs::validate_windows_path_identity_only(parent, &directory, true)?;
        crate::private_fs::set_windows_private_acl(&directory, true)?;
    }
    Ok(())
}

fn atomic_write_config(path: &Path, scope: ConfigScope, contents: &str) -> Result<()> {
    #[cfg(unix)]
    {
        atomic_write_config_unix(path, scope, contents)
    }
    #[cfg(not(unix))]
    {
        atomic_write_config_non_unix(path, scope, contents)
    }
}

/// Unix writes delegate to the descriptor-anchored filesystem primitive. It
/// pins the configuration directory, rechecks the destination just before
/// `renameat`, and verifies reachability after installation, so an ancestor
/// replacement cannot redirect a configuration mutation through a symlink.
#[cfg(unix)]
fn atomic_write_config_unix(path: &Path, scope: ConfigScope, contents: &str) -> Result<()> {
    if contents.len() as u64 > MAX_CONFIG_BYTES {
        bail!(
            "refusing to write oversized configuration: {}",
            path.display()
        )
    }
    let parent = path
        .parent()
        .expect("validated configuration path has a parent");
    let unix_mode = match scope {
        ConfigScope::Global => 0o600,
        // Project configuration is intended for ordinary repository review.
        // Its parent is still descriptor-pinned and the target still rejects
        // links and races; only the installed file's visibility differs.
        ConfigScope::Project => 0o644,
    };
    crate::adapter::outbound::filesystem::secure_fs::write_file_atomic_unlocked_with_limit_and_unix_mode(
        parent,
        Path::new(CONFIG_FILE_NAME),
        contents.as_bytes(),
        MAX_CONFIG_BYTES as usize,
        unix_mode,
    )
    .with_context(|| {
        format!(
            "failed to atomically replace configuration {}",
            path.display()
        )
    })?;
    // Re-open after replacement so global mode and link identity are verified
    // on the exact object now visible at the configured path.
    let _ = read_config_file_if_present(path, scope)?;
    Ok(())
}

#[cfg(not(unix))]
fn atomic_write_config_non_unix(path: &Path, scope: ConfigScope, contents: &str) -> Result<()> {
    if contents.len() as u64 > MAX_CONFIG_BYTES {
        bail!(
            "refusing to write oversized configuration: {}",
            path.display()
        )
    }
    let parent = path
        .parent()
        .expect("validated configuration path has a parent");
    let mut temporary = None;
    for attempt in 0..64u32 {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let candidate = parent.join(format!(
            ".{CONFIG_FILE_NAME}.{}.{}.{}.tmp",
            std::process::id(),
            nonce,
            attempt
        ));
        let open = create_private_temporary_file(&candidate, scope);
        match open {
            Ok(file) => {
                temporary = Some((candidate, file));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to create temporary configuration {}",
                        candidate.display()
                    )
                });
            }
        }
    }
    let Some((temporary_path, mut file)) = temporary else {
        bail!("failed to allocate a unique temporary configuration file")
    };
    let write_result = (|| -> Result<()> {
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        #[cfg(windows)]
        if scope == ConfigScope::Global {
            crate::private_fs::set_windows_private_acl(&file, false)?;
        }
        Ok(())
    })();
    drop(file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary_path);
        return Err(error).with_context(|| {
            format!(
                "failed to write temporary configuration {}",
                temporary_path.display()
            )
        });
    }
    if let Err(error) = replace_configuration_file(&temporary_path, path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(error).with_context(|| {
            format!(
                "failed to atomically replace configuration {}",
                path.display()
            )
        });
    }
    // Re-open after replacement so Unix mode / Windows ACL and link identity
    // are verified on the exact object now visible at the configured path.
    let _ = read_config_file_if_present(path, scope)?;
    Ok(())
}

/// Install a same-directory temporary file without exposing a delete-and-copy
/// window. `std::fs::rename` maps to an atomic replacement on Unix, but its
/// Windows replacement behavior is not uniform across supported filesystems.
/// `ReplaceFileW` preserves the existing destination atomically; a first
/// creation deliberately uses non-replacing `MoveFileExW` so a concurrent new
/// object fails closed instead of being overwritten.
#[cfg(not(unix))]
fn replace_configuration_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(not(windows))]
    {
        fs::rename(source, destination)
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_WRITE_THROUGH, MoveFileExW, REPLACEFILE_WRITE_THROUGH, ReplaceFileW,
        };

        let source = source
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let destination = destination
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();

        // SAFETY: both paths are NUL-terminated and remain live throughout the call.
        if unsafe {
            ReplaceFileW(
                destination.as_ptr(),
                source.as_ptr(),
                std::ptr::null(),
                REPLACEFILE_WRITE_THROUGH,
                std::ptr::null(),
                std::ptr::null(),
            )
        } != 0
        {
            return Ok(());
        }
        let replacement_error = std::io::Error::last_os_error();
        if replacement_error.kind() != std::io::ErrorKind::NotFound {
            return Err(replacement_error);
        }

        // SAFETY: both paths are NUL-terminated and remain live throughout the call. Omitting
        // MOVEFILE_REPLACE_EXISTING turns a race that creates destination into a safe failure.
        if unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        } != 0
        {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
}

#[cfg(not(unix))]
fn create_private_temporary_file(path: &Path, _scope: ConfigScope) -> std::io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

struct ConfigWriteLock {
    // Keeping the descriptor alive owns the OS-level exclusion. The lock file
    // intentionally remains after release, so an abnormal process exit cannot
    // leave a directory-shaped stale lock behind.
    _file: File,
}

impl ConfigWriteLock {
    fn acquire(config_path: &Path) -> Result<Self> {
        // Global config and project config mutations use the same lock root.
        // For a project config this path is still deterministic under AKRA_HOME.
        let path = config_write_lock_path(config_path)?;
        let deadline = Instant::now() + CONFIG_LOCK_WAIT;
        loop {
            match try_acquire_config_write_lock(&path)? {
                Some(file) => return Ok(Self { _file: file }),
                None if Instant::now() >= deadline => {
                    bail!(
                        "timed out waiting for configuration writer lock: {}",
                        path.display()
                    )
                }
                None => thread::sleep(CONFIG_LOCK_RETRY),
            }
        }
    }
}

fn config_write_lock_path(config_path: &Path) -> Result<PathBuf> {
    let global_config = global_config_path()?;
    ensure_config_parent(&global_config, ConfigScope::Global)?;
    let lock_root = global_config
        .parent()
        .ok_or_else(|| anyhow!("global configuration path has no parent"))?
        .join("config-locks");
    // Reuse the global-directory validation for creation and ACL handling; the
    // lock root additionally remains owner-only because it names every active
    // configuration mutation. The probe is never created.
    ensure_config_parent(
        &lock_root.join(".config-write-lock-root-probe"),
        ConfigScope::Global,
    )?;
    #[cfg(unix)]
    restrict_config_lock_root_permissions(&lock_root)?;
    let mut digest = Sha256::new();
    digest.update(config_path.as_os_str().to_string_lossy().as_bytes());
    Ok(lock_root.join(format!("{:x}.lock", digest.finalize())))
}

#[cfg(unix)]
fn restrict_config_lock_root_permissions(lock_root: &Path) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let metadata = fs::symlink_metadata(lock_root).with_context(|| {
        format!(
            "failed to inspect configuration lock directory {}",
            lock_root.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "configuration lock directory must be a real directory: {}",
            lock_root.display()
        )
    }
    if metadata.uid() != unsafe { libc::geteuid() } {
        bail!(
            "configuration lock directory must be owned by the current user: {}",
            lock_root.display()
        )
    }
    if metadata.mode() & 0o077 != 0 {
        fs::set_permissions(lock_root, fs::Permissions::from_mode(0o700)).with_context(|| {
            format!(
                "failed to restrict configuration lock directory {}",
                lock_root.display()
            )
        })?;
    }
    Ok(())
}

fn reject_legacy_config_lock_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => bail!(
            "legacy configuration lock directory found at {}; confirm no older Akra process is running, remove that directory, and retry",
            path.display()
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to inspect configuration writer lock {}",
                path.display()
            )
        }),
    }
}

#[cfg(unix)]
fn try_acquire_config_write_lock(path: &Path) -> Result<Option<File>> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    reject_legacy_config_lock_directory(path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| {
            format!(
                "failed to open configuration writer lock {}",
                path.display()
            )
        })?;
    validate_config_write_lock_file(path, &file)?;
    // SAFETY: flock observes only the owned descriptor and does not retain a
    // Rust pointer. The OS releases this lock if the process exits abruptly.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = std::io::Error::last_os_error();
        let error_code = error.raw_os_error();
        if error_code == Some(libc::EWOULDBLOCK) || error_code == Some(libc::EAGAIN) {
            return Ok(None);
        }
        return Err(error).with_context(|| {
            format!(
                "failed to acquire configuration writer lock {}",
                path.display()
            )
        });
    }
    validate_config_write_lock_file(path, &file)?;
    Ok(Some(file))
}

#[cfg(unix)]
fn validate_config_write_lock_file(path: &Path, file: &File) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let opened = file.metadata().with_context(|| {
        format!(
            "failed to inspect configuration writer lock {}",
            path.display()
        )
    })?;
    let visible = fs::symlink_metadata(path).with_context(|| {
        format!(
            "failed to inspect configuration writer lock {}",
            path.display()
        )
    })?;
    if !opened.is_file()
        || opened.uid() != unsafe { libc::geteuid() }
        || opened.nlink() != 1
        || opened.mode() & 0o077 != 0
    {
        bail!(
            "configuration writer lock must be an owner-private single-link regular file: {}",
            path.display()
        )
    }
    if visible.file_type().is_symlink()
        || !visible.is_file()
        || visible.dev() != opened.dev()
        || visible.ino() != opened.ino()
    {
        bail!(
            "configuration writer lock changed while being opened: {}",
            path.display()
        )
    }
    Ok(())
}

#[cfg(windows)]
fn try_acquire_config_write_lock(path: &Path) -> Result<Option<File>> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, HANDLE};
    use windows_sys::Win32::Storage::FileSystem::LockFile;

    reject_legacy_config_lock_directory(path)?;
    let mut create_options = OpenOptions::new();
    create_options
        .read(true)
        .write(true)
        .create_new(true)
        .access_mode(
            crate::private_fs::WINDOWS_GENERIC_READ
                | crate::private_fs::WINDOWS_GENERIC_WRITE
                | crate::private_fs::WINDOWS_READ_CONTROL
                | crate::private_fs::WINDOWS_WRITE_DAC,
        )
        .share_mode(crate::private_fs::WINDOWS_FILE_SHARE_ALL)
        .custom_flags(crate::private_fs::WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT);
    let (file, created) = match create_options.open(path) {
        Ok(file) => (file, true),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (
            OpenOptions::new()
                .read(true)
                .write(true)
                .access_mode(
                    crate::private_fs::WINDOWS_GENERIC_READ
                        | crate::private_fs::WINDOWS_GENERIC_WRITE
                        | crate::private_fs::WINDOWS_READ_CONTROL,
                )
                .share_mode(crate::private_fs::WINDOWS_FILE_SHARE_ALL)
                .custom_flags(crate::private_fs::WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
                .open(path)
                .with_context(|| {
                    format!(
                        "failed to open configuration writer lock {}",
                        path.display()
                    )
                })?,
            false,
        ),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to create configuration writer lock {}",
                    path.display()
                )
            });
        }
    };
    crate::private_fs::validate_windows_path_identity_only(path, &file, false)?;
    if created {
        crate::private_fs::set_windows_private_acl(&file, false)?;
    }
    crate::private_fs::validate_windows_private_owner_and_acl(path, &file)?;
    crate::private_fs::validate_windows_path_identity_only(path, &file, false)?;
    // SAFETY: LockFile receives the valid owned handle and locks one byte at
    // offset zero. Windows releases the byte-range lock on process exit.
    if unsafe { LockFile(file.as_raw_handle() as HANDLE, 0, 0, 1, 0) } == 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
            return Ok(None);
        }
        return Err(error).with_context(|| {
            format!(
                "failed to acquire configuration writer lock {}",
                path.display()
            )
        });
    }
    crate::private_fs::validate_windows_path_identity_only(path, &file, false)?;
    crate::private_fs::validate_windows_private_owner_and_acl(path, &file)?;
    Ok(Some(file))
}

#[cfg(not(any(unix, windows)))]
fn try_acquire_config_write_lock(_path: &Path) -> Result<Option<File>> {
    bail!("configuration writer locking is unsupported on this platform")
}

#[cfg(windows)]
impl Drop for ConfigWriteLock {
    fn drop(&mut self) {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::HANDLE;
        use windows_sys::Win32::Storage::FileSystem::UnlockFile;

        // SAFETY: the file handle remains owned by this guard and exactly the
        // byte range acquired above is released.
        unsafe {
            UnlockFile(self._file.as_raw_handle() as HANDLE, 0, 0, 1, 0);
        }
    }
}

#[derive(Default)]
struct EnvironmentConfigAdapter {
    values: BTreeMap<SettingKey, (SettingValue, String)>,
    shadow_descriptions: Vec<String>,
}

impl EnvironmentConfigAdapter {
    fn from_process() -> Result<Self> {
        let mut adapter = Self::default();
        adapter.add_startup_visual(&[
            "CODEX_EXEC_LOOP_SHOW_STARTUP_VISUAL",
            "CODEX_EXEC_LOOP_SHOW_STARTUP_ASCII_ART",
        ])?;
        adapter.add_planning_worker_visibility(&[
            "CODEX_EXEC_LOOP_PLANNING_WORKER_VISIBILITY",
            "CODEX_EXEC_LOOP_PLANNER_VISIBILITY",
        ])?;
        adapter.add_unsigned(
            SettingKey::GithubReviewPollIntervalSecs,
            &["CODEX_EXEC_LOOP_GITHUB_POLL_INTERVAL_SECS"],
        )?;
        adapter.add_text(SettingKey::GithubPushRemote, &["AKRA_GITHUB_PUSH_REMOTE"])?;
        adapter.add_text(SettingKey::GithubPullRequestMode, &["AKRA_GITHUB_PR_MODE"])?;
        adapter.add_text(
            SettingKey::ParallelIntegrationBranch,
            &["AKRA_PARALLEL_INTEGRATION_BRANCH"],
        )?;
        adapter.add_unsigned(
            SettingKey::AppServerResponseTimeoutSecs,
            &["CODEX_EXEC_LOOP_APP_SERVER_RESPONSE_TIMEOUT_SECS"],
        )?;
        adapter.add_bool(
            SettingKey::AppServerPromptLog,
            &["AKRA_APP_SERVER_PROMPT_LOG"],
        )?;
        adapter.add_unsigned(
            SettingKey::SubprocessTimeoutSecs,
            &["CODEX_EXEC_LOOP_SUBPROCESS_TIMEOUT_SECS"],
        )?;
        adapter.add_diagnostics_trace(&["AKRA_TRACE"])?;
        adapter.add_diagnostics_spans(&["AKRA_TRACE_SPANS"])?;
        adapter.add_unsigned(SettingKey::DiagnosticsMaxFiles, &["AKRA_TRACE_MAX_FILES"])?;
        adapter.add_unsigned(
            SettingKey::DiagnosticsMaxFileBytes,
            &["AKRA_TRACE_MAX_FILE_BYTES"],
        )?;
        adapter.add_unsigned(
            SettingKey::DiagnosticsMaxTotalBytes,
            &["AKRA_TRACE_MAX_TOTAL_BYTES"],
        )?;
        adapter.add_bool(SettingKey::DiagnosticsTokioConsole, &["AKRA_TOKIO_CONSOLE"])?;
        adapter.add_bool(
            SettingKey::AdminGraphicEnabled,
            &["AKRA_ADMIN_GRAPHIC_ENABLED"],
        )?;
        adapter.add_unsigned(
            SettingKey::AdminGraphicPollIntervalMs,
            &["AKRA_ADMIN_GRAPHIC_POLL_MS"],
        )?;
        for variable in ["RUST_LOG", "AKRA_TRACE_FILE", "CODEX_EXEC_LOOP_GITHUB_PR"] {
            if std::env::var_os(variable).is_some() {
                adapter
                    .shadow_descriptions
                    .push(format!("{variable} is active outside TOML"));
            }
        }
        Ok(adapter)
    }

    fn add_bool(&mut self, key: SettingKey, variables: &[&str]) -> Result<()> {
        if let Some((raw, variable)) = first_environment_value(variables)? {
            let value = parse_bool(&raw)
                .with_context(|| format!("invalid environment setting {variable}"))?;
            self.record(key, SettingValue::Bool(value), variable);
        }
        Ok(())
    }

    fn add_startup_visual(&mut self, variables: &[&str]) -> Result<()> {
        let Some((raw, variable)) = first_environment_value(variables)? else {
            return Ok(());
        };
        // The pre-layer startup visual accepted every non-falsey value as
        // enabled. Preserve that permissive capture/CI contract while the
        // TOML surface itself remains a typed boolean.
        let disabled = matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        );
        self.record(
            SettingKey::TuiShowStartupVisual,
            SettingValue::Bool(!disabled),
            variable,
        );
        Ok(())
    }

    fn add_planning_worker_visibility(&mut self, variables: &[&str]) -> Result<()> {
        let Some((raw, variable)) = first_environment_value(variables)? else {
            return Ok(());
        };
        // Existing launchers used boolean and descriptive aliases in addition
        // to the canonical normal/debug TOML values.
        let value = match raw.trim().to_ascii_lowercase().as_str() {
            "debug" | "verbose" | "detailed" | "1" | "true" => "debug",
            _ => "normal",
        };
        self.record(
            SettingKey::TuiPlanningWorkerVisibility,
            SettingValue::Text(value.to_string()),
            variable,
        );
        Ok(())
    }

    fn add_diagnostics_trace(&mut self, variables: &[&str]) -> Result<()> {
        let Some((raw, variable)) = first_environment_value(variables)? else {
            return Ok(());
        };
        let value = SettingValue::Text(raw.trim().to_string());
        validate_setting_value(SettingKey::DiagnosticsTrace, value.clone())
            .with_context(|| format!("invalid environment setting {variable}"))?;
        self.record(SettingKey::DiagnosticsTrace, value, variable);
        Ok(())
    }

    fn add_diagnostics_spans(&mut self, variables: &[&str]) -> Result<()> {
        let Some((raw, variable)) = first_environment_value(variables)? else {
            return Ok(());
        };
        // `0` and `off` have always meant no span events. Persist their
        // canonical spelling in the resolved snapshot and global writer.
        let value = match raw.trim().to_ascii_lowercase().as_str() {
            "0" | "off" => "none".to_string(),
            value => value.to_string(),
        };
        let value = SettingValue::Text(value);
        validate_setting_value(SettingKey::DiagnosticsSpans, value.clone())
            .with_context(|| format!("invalid environment setting {variable}"))?;
        self.record(SettingKey::DiagnosticsSpans, value, variable);
        Ok(())
    }

    fn add_unsigned(&mut self, key: SettingKey, variables: &[&str]) -> Result<()> {
        if let Some((raw, variable)) = first_environment_value(variables)? {
            let value = parse_unsigned(&raw, key)
                .with_context(|| format!("invalid environment setting {variable}"))?;
            let value = SettingValue::Unsigned(value);
            validate_setting_value(key, value.clone())
                .with_context(|| format!("invalid environment setting {variable}"))?;
            self.record(key, value, variable);
        }
        Ok(())
    }

    fn add_text(&mut self, key: SettingKey, variables: &[&str]) -> Result<()> {
        if let Some((raw, variable)) = first_environment_value(variables)? {
            let normalized = match key {
                SettingKey::TuiPlanningWorkerVisibility
                | SettingKey::GithubPullRequestMode
                | SettingKey::DiagnosticsTrace
                | SettingKey::DiagnosticsSpans => raw.trim().to_ascii_lowercase(),
                _ => raw.trim().to_string(),
            };
            let value = SettingValue::Text(normalized);
            validate_setting_value(key, value.clone())
                .with_context(|| format!("invalid environment setting {variable}"))?;
            self.record(key, value, variable);
        }
        Ok(())
    }

    fn record(&mut self, key: SettingKey, value: SettingValue, variable: String) {
        self.shadow_descriptions
            .push(format!("{variable} -> {key}"));
        self.values.insert(key, (value, variable));
    }
}

fn first_environment_value(variables: &[&str]) -> Result<Option<(String, String)>> {
    for variable in variables {
        match std::env::var(variable) {
            Ok(value) => return Ok(Some((value, (*variable).to_string()))),
            Err(std::env::VarError::NotPresent) => {}
            Err(std::env::VarError::NotUnicode(_)) => {
                bail!("{variable} must contain valid UTF-8")
            }
        }
    }
    Ok(None)
}

fn legacy_git_key(key: SettingKey) -> &'static str {
    match key {
        SettingKey::GithubPushRemote => "akra.githubPushRemote",
        SettingKey::GithubPullRequestMode => "akra.githubPrMode",
        SettingKey::ParallelIntegrationBranch => "akra.parallelIntegrationBranch",
        _ => unreachable!("only legacy Git-compatible keys ask for a Git key name"),
    }
}

fn read_legacy_git_values(repo_root: &Path) -> Result<BTreeMap<SettingKey, SettingValue>> {
    let mut values = BTreeMap::new();
    for key in [
        SettingKey::GithubPushRemote,
        SettingKey::GithubPullRequestMode,
        SettingKey::ParallelIntegrationBranch,
    ] {
        let mut command = crate::git_subprocess::command([
            "-C",
            repo_root.to_string_lossy().as_ref(),
            "config",
            "--get",
            legacy_git_key(key),
        ]);
        let output = command.output().with_context(|| {
            format!(
                "failed to read legacy Git configuration {}",
                legacy_git_key(key)
            )
        })?;
        if output.status.success() {
            let raw = String::from_utf8(output.stdout)
                .context("legacy Git configuration must be valid UTF-8")?;
            let value = parse_cli_value(key, raw.trim()).with_context(|| {
                format!("invalid legacy Git configuration {}", legacy_git_key(key))
            })?;
            values.insert(key, value);
        }
    }
    Ok(values)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigFileInspection {
    Missing,
    Valid,
    Error(String),
}

impl fmt::Display for ConfigFileInspection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => formatter.write_str("missing"),
            Self::Valid => formatter.write_str("ok"),
            Self::Error(error) => write!(formatter, "error: {error}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDoctorReport {
    pub paths: Option<ConfigPaths>,
    pub global: ConfigFileInspection,
    pub project: Option<ConfigFileInspection>,
    pub environment: Vec<String>,
    pub legacy: Vec<String>,
    pub resolution_error: Option<String>,
}

impl ConfigDoctorReport {
    fn path_error(error: String) -> Self {
        Self {
            paths: None,
            global: ConfigFileInspection::Error(error),
            project: None,
            environment: Vec::new(),
            legacy: Vec::new(),
            resolution_error: None,
        }
    }

    pub fn is_healthy(&self) -> bool {
        matches!(
            self.global,
            ConfigFileInspection::Missing | ConfigFileInspection::Valid
        ) && self.project.as_ref().is_none_or(|inspection| {
            matches!(
                inspection,
                ConfigFileInspection::Missing | ConfigFileInspection::Valid
            )
        }) && self.resolution_error.is_none()
    }
}

fn inspect_layer_file(path: &Path, scope: ConfigScope) -> ConfigFileInspection {
    match read_layer_if_present(path, scope) {
        Ok(None) => ConfigFileInspection::Missing,
        Ok(Some(_)) => ConfigFileInspection::Valid,
        Err(error) => ConfigFileInspection::Error(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EnvGuard {
        values: Vec<(String, Option<OsString>)>,
    }

    impl EnvGuard {
        fn set(values: &[(&str, Option<&str>)]) -> Self {
            let mut previous = Vec::new();
            for (name, value) in values {
                previous.push(((*name).to_string(), std::env::var_os(name)));
                match value {
                    Some(value) => unsafe { std::env::set_var(name, value) },
                    None => unsafe { std::env::remove_var(name) },
                }
            }
            Self { values: previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (name, value) in self.values.drain(..) {
                match value {
                    Some(value) => unsafe { std::env::set_var(name, value) },
                    None => unsafe { std::env::remove_var(name) },
                }
            }
        }
    }

    fn temporary_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "akra-config-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("temporary directory should create");
        path
    }

    fn write_global_fixture(root: &Path, body: &str) {
        fs::create_dir_all(root).expect("global fixture root should create");
        let path = root.join(CONFIG_FILE_NAME);
        fs::write(&path, body).expect("global config fixture should write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .expect("global config fixture should be owner-only");
        }
    }

    fn fake_git_worktree(root: &Path) {
        fs::create_dir_all(root.join("nested")).expect("Git worktree fixture should create");
        // A linked worktree has a `.git` file rather than a directory.  Config
        // discovery needs only that stable Git marker; the test deliberately
        // avoids a repository config side effect.
        fs::write(root.join(".git"), "gitdir: /nonexistent/test-worktree\n")
            .expect("linked worktree Git marker should write");
    }

    #[test]
    fn defaults_use_sol_and_medium() {
        let config = AkraConfig::builtin();

        assert_eq!(config.conversation.model.as_deref(), Some("gpt-5.6-sol"));
        assert_eq!(
            config.conversation.reasoning_effort.as_deref(),
            Some("medium")
        );
        assert_eq!(config.github.pull_request_mode, "required");
    }

    #[test]
    fn leaf_layers_inherit_unrelated_global_values() {
        let global = AkraConfigLayer {
            tui: Some(TuiConfigLayer {
                show_startup_visual: Some(false),
                planning_worker_visibility: None,
            }),
            ..AkraConfigLayer::default()
        };
        let project = AkraConfigLayer {
            tui: Some(TuiConfigLayer {
                show_startup_visual: None,
                planning_worker_visibility: Some("debug".to_string()),
            }),
            ..AkraConfigLayer::default()
        };
        validate_layer(&global, ConfigScope::Global).expect("global layer should validate");
        validate_layer(&project, ConfigScope::Project).expect("project layer should validate");
        let mut config = AkraConfig::builtin();
        let mut origins = BTreeMap::new();
        apply_layer(
            &mut config,
            &mut origins,
            &global,
            SettingOrigin::GlobalToml,
        );
        apply_layer(
            &mut config,
            &mut origins,
            &project,
            SettingOrigin::ProjectToml,
        );

        assert!(!config.tui.show_startup_visual);
        assert_eq!(config.tui.planning_worker_visibility, "debug");
    }

    #[test]
    fn default_values_are_explicit_app_server_requests() {
        let override_value = ConfigOverride::parse("conversation.model=default")
            .expect("default model override should parse");
        let mut config = AkraConfig::builtin();
        let mut origins = BTreeMap::new();
        apply_value(
            &mut config,
            &mut origins,
            override_value.key,
            override_value.value,
            SettingOrigin::CommandLine,
        );

        assert_eq!(config.conversation.model, None);
    }

    #[test]
    fn project_rejects_machine_only_settings() {
        let layer = AkraConfigLayer {
            diagnostics: Some(DiagnosticsConfigLayer {
                trace: Some("full".to_string()),
                ..DiagnosticsConfigLayer::default()
            }),
            ..AkraConfigLayer::default()
        };
        let error = validate_layer(&layer, ConfigScope::Project)
            .expect_err("diagnostics should be global-only")
            .to_string();

        assert!(error.contains("not permitted in project"));
    }

    #[test]
    fn invocation_accepts_repeated_config_overrides() {
        let invocation = ParsedInvocation::parse([
            "-c",
            "conversation.model=gpt-5.6-terra",
            "--config=conversation.reasoning_effort=max",
            "status",
        ])
        .expect("invocation should parse");

        assert_eq!(invocation.command_args, vec![OsString::from("status")]);
        assert_eq!(invocation.overrides.len(), 2);
    }

    #[test]
    fn read_only_resolution_does_not_create_global_file() {
        let _lock = crate::test_utils::process_environment_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = temporary_directory("readonly");
        let _environment = EnvGuard::set(&[("AKRA_HOME", Some(root.to_string_lossy().as_ref()))]);

        let resolved = ConfigurationService::resolve_read_only(&root, &[])
            .expect("read-only resolution should use builtins");

        assert_eq!(
            resolved.config.conversation.model.as_deref(),
            Some("gpt-5.6-sol")
        );
        assert!(!root.join(CONFIG_FILE_NAME).exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn normal_resolution_creates_canonical_global_file() {
        let _lock = crate::test_utils::process_environment_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = temporary_directory("startup");
        let _environment = EnvGuard::set(&[("AKRA_HOME", Some(root.to_string_lossy().as_ref()))]);

        ConfigurationService::resolve_for_startup(&root, &[])
            .expect("startup resolution should create global file");
        let contents = fs::read_to_string(root.join(CONFIG_FILE_NAME))
            .expect("global config should be written");

        assert!(contents.contains("schema_version = 1"));
        assert!(contents.contains("gpt-5.6-sol"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn environment_overrides_global_layer() {
        let _lock = crate::test_utils::process_environment_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = temporary_directory("environment");
        let _environment = EnvGuard::set(&[
            ("AKRA_HOME", Some(root.to_string_lossy().as_ref())),
            (
                "CODEX_EXEC_LOOP_APP_SERVER_RESPONSE_TIMEOUT_SECS",
                Some("42"),
            ),
        ]);
        fs::write(
            root.join(CONFIG_FILE_NAME),
            "schema_version = 1\n[app_server]\nresponse_timeout_secs = 20\n",
        )
        .expect("global config fixture should write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                root.join(CONFIG_FILE_NAME),
                fs::Permissions::from_mode(0o600),
            )
            .expect("global config fixture should be owner-only");
        }

        let resolved = ConfigurationService::resolve_read_only(&root, &[])
            .expect("configuration should resolve");

        assert_eq!(resolved.config.app_server.response_timeout_secs, 42);
        assert!(matches!(
            resolved.origin_for(SettingKey::AppServerResponseTimeoutSecs),
            SettingOrigin::Environment { .. }
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn environment_adapter_preserves_legacy_tui_and_trace_spellings() {
        let _lock = crate::test_utils::process_environment_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = temporary_directory("environment-legacy-aliases");
        let home_text = root.to_string_lossy().into_owned();
        let _environment = EnvGuard::set(&[
            ("AKRA_HOME", Some(&home_text)),
            ("CODEX_EXEC_LOOP_SHOW_STARTUP_ASCII_ART", Some("capture")),
            ("CODEX_EXEC_LOOP_PLANNER_VISIBILITY", Some("verbose")),
            ("AKRA_TRACE", Some("1")),
            ("AKRA_TRACE_SPANS", Some("off")),
        ]);

        let resolved = ConfigurationService::resolve_read_only(&root, &[])
            .expect("legacy environment values should resolve");

        assert!(resolved.config.tui.show_startup_visual);
        assert_eq!(resolved.config.tui.planning_worker_visibility, "debug");
        assert_eq!(resolved.config.diagnostics.trace, "1");
        assert_eq!(resolved.config.diagnostics.spans, "none");
        assert!(matches!(
            resolved.origin_for(SettingKey::TuiPlanningWorkerVisibility),
            SettingOrigin::Environment { variable }
                if variable == "CODEX_EXEC_LOOP_PLANNER_VISIBILITY"
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn resolver_applies_legacy_project_environment_and_command_precedence_in_order() {
        let _lock = crate::test_utils::process_environment_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = temporary_directory("full-precedence");
        let workspace = root.join("workspace");
        let initialized = std::process::Command::new("git")
            .arg("init")
            .arg("--quiet")
            .arg(&workspace)
            .status()
            .expect("Git should initialize a precedence fixture");
        assert!(initialized.success());
        let configured = std::process::Command::new("git")
            .arg("-C")
            .arg(&workspace)
            .arg("config")
            .arg("akra.githubPushRemote")
            .arg("legacy")
            .status()
            .expect("Git should write the legacy compatibility value");
        assert!(configured.success());

        let home = root.join("home");
        let home_text = home.to_string_lossy().into_owned();
        let _environment = EnvGuard::set(&[
            ("AKRA_HOME", Some(&home_text)),
            ("AKRA_GITHUB_PUSH_REMOTE", None),
        ]);
        write_global_fixture(&home, "[github]\npush_remote = \"global\"\n");

        let legacy = ConfigurationService::resolve_read_only(&workspace, &[])
            .expect("legacy compatibility layer should resolve");
        assert_eq!(legacy.config.github.push_remote, "legacy");
        assert!(matches!(
            legacy.origin_for(SettingKey::GithubPushRemote),
            SettingOrigin::LegacyGitConfig { key } if key == "akra.githubPushRemote"
        ));

        let project_path = workspace
            .join(PROJECT_CONFIG_DIRECTORY)
            .join(CONFIG_FILE_NAME);
        fs::create_dir_all(
            project_path
                .parent()
                .expect("project configuration has a parent"),
        )
        .expect("project configuration directory should create");
        fs::write(&project_path, "[github]\npush_remote = \"project\"\n")
            .expect("project configuration should write");
        let project = ConfigurationService::resolve_read_only(&workspace, &[])
            .expect("project layer should resolve");
        assert_eq!(project.config.github.push_remote, "project");
        assert!(matches!(
            project.origin_for(SettingKey::GithubPushRemote),
            SettingOrigin::ProjectToml
        ));

        {
            let _environment = EnvGuard::set(&[("AKRA_GITHUB_PUSH_REMOTE", Some("environment"))]);
            let environment = ConfigurationService::resolve_read_only(&workspace, &[])
                .expect("environment layer should resolve");
            assert_eq!(environment.config.github.push_remote, "environment");
            assert!(matches!(
                environment.origin_for(SettingKey::GithubPushRemote),
                SettingOrigin::Environment { variable } if variable == "AKRA_GITHUB_PUSH_REMOTE"
            ));

            let override_value = ConfigOverride::parse("github.push_remote=command")
                .expect("command-line override should parse");
            let command = ConfigurationService::resolve_read_only(&workspace, &[override_value])
                .expect("command layer should resolve");
            assert_eq!(command.config.github.push_remote, "command");
            assert!(matches!(
                command.origin_for(SettingKey::GithubPushRemote),
                SettingOrigin::CommandLine
            ));
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn explicit_workspace_rebinds_project_layers_without_reloading_process_sources() {
        let _lock = crate::test_utils::process_environment_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = temporary_directory("workspace-rebind");
        let home = root.join("home");
        let workspace_a = root.join("workspace-a");
        let workspace_b = root.join("workspace-b");
        fake_git_worktree(&workspace_a);
        fake_git_worktree(&workspace_b);
        let home_text = home.to_string_lossy().into_owned();
        let _environment = EnvGuard::set(&[
            ("AKRA_HOME", Some(&home_text)),
            ("AKRA_GITHUB_PUSH_REMOTE", None),
            ("AKRA_GITHUB_PR_MODE", None),
            ("AKRA_PARALLEL_INTEGRATION_BRANCH", None),
        ]);
        write_global_fixture(
            &home,
            "schema_version = 1\n[github]\nreview_poll_interval_secs = 13\n",
        );
        for (workspace, remote, mode, branch) in [
            (&workspace_a, "remote-a", "required", "integration-a"),
            (&workspace_b, "remote-b", "disabled", "integration-b"),
        ] {
            let project = workspace
                .join(PROJECT_CONFIG_DIRECTORY)
                .join(CONFIG_FILE_NAME);
            fs::create_dir_all(project.parent().expect("project config has parent"))
                .expect("project config parent should create");
            fs::write(
                project,
                format!(
                    "schema_version = 1\n[github]\npush_remote = \"{remote}\"\npull_request_mode = \"{mode}\"\n[parallel]\nintegration_branch = \"{branch}\"\n"
                ),
            )
            .expect("project config should write");
        }
        let overrides = [ConfigOverride::parse("github.pull_request_mode=auto")
            .expect("command override should parse")];
        let source = ConfigurationService::resolve_read_only(&workspace_a, &overrides)
            .expect("source workspace should resolve");
        write_global_fixture(
            &home,
            "schema_version = 1\n[github]\nreview_poll_interval_secs = 99\n",
        );

        let target = source
            .resolve_for_workspace(&workspace_b)
            .expect("target workspace should resolve from the process snapshot");

        assert_eq!(target.config.github.push_remote, "remote-b");
        assert_eq!(target.config.github.pull_request_mode, "auto");
        assert_eq!(target.config.parallel.integration_branch, "integration-b");
        assert_eq!(target.config.github.review_poll_interval_secs, 13);
        assert!(matches!(
            target.origin_for(SettingKey::GithubPushRemote),
            SettingOrigin::ProjectToml
        ));
        assert!(matches!(
            target.origin_for(SettingKey::GithubPullRequestMode),
            SettingOrigin::CommandLine
        ));
        assert!(matches!(
            target.origin_for(SettingKey::ParallelIntegrationBranch),
            SettingOrigin::ProjectToml
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mutation_preserves_comments_and_unset_restores_the_higher_leaf() {
        let _lock = crate::test_utils::process_environment_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = temporary_directory("comments-and-unset");
        let workspace = root.join("workspace");
        fake_git_worktree(&workspace);
        let home = root.join("home");
        let home_text = home.to_string_lossy().into_owned();
        let _environment = EnvGuard::set(&[("AKRA_HOME", Some(&home_text))]);
        write_global_fixture(
            &home,
            "# keep this header\nschema_version = 1\n\n[tui]\n# keep this field comment\nshow_startup_visual = false\nplanning_worker_visibility = \"normal\"\n",
        );
        let project_path = workspace
            .join(PROJECT_CONFIG_DIRECTORY)
            .join(CONFIG_FILE_NAME);
        fs::create_dir_all(project_path.parent().expect("project config has parent"))
            .expect("project config parent should create");
        fs::write(
            &project_path,
            "[tui]\nshow_startup_visual = true\nplanning_worker_visibility = \"debug\"\n",
        )
        .expect("project config fixture should write");

        ConfigurationService::set(
            &workspace,
            ConfigScope::Global,
            SettingKey::TuiPlanningWorkerVisibility,
            "debug",
        )
        .expect("global mutation should succeed");
        let updated =
            fs::read_to_string(home.join(CONFIG_FILE_NAME)).expect("updated config should read");
        assert!(updated.contains("# keep this header"));
        assert!(updated.contains("# keep this field comment"));
        assert!(updated.contains("show_startup_visual = false"));
        assert!(updated.contains("planning_worker_visibility = \"debug\""));

        let before_unset = ConfigurationService::resolve_read_only(&workspace, &[])
            .expect("project config should resolve before unset");
        assert!(before_unset.config.tui.show_startup_visual);
        assert_eq!(before_unset.config.tui.planning_worker_visibility, "debug");
        assert!(matches!(
            before_unset.origin_for(SettingKey::TuiPlanningWorkerVisibility),
            SettingOrigin::ProjectToml
        ));

        ConfigurationService::unset(
            &workspace,
            ConfigScope::Project,
            SettingKey::TuiPlanningWorkerVisibility,
        )
        .expect("project leaf unset should succeed");
        let after_unset = ConfigurationService::resolve_read_only(&workspace, &[])
            .expect("project config should resolve after unset");
        assert_eq!(after_unset.config.tui.planning_worker_visibility, "debug");
        assert!(matches!(
            after_unset.origin_for(SettingKey::TuiPlanningWorkerVisibility),
            SettingOrigin::GlobalToml
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn linked_worktree_uses_its_own_project_config_path() {
        let _lock = crate::test_utils::process_environment_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = temporary_directory("linked-worktree");
        let worktree = root.join("linked");
        fake_git_worktree(&worktree);
        let home = root.join("home");
        let home_text = home.to_string_lossy().into_owned();
        let _environment = EnvGuard::set(&[("AKRA_HOME", Some(&home_text))]);

        let discovered = ConfigPaths::discover(&worktree.join("nested"))
            .expect("linked worktree paths should discover");
        let canonical_worktree = worktree
            .canonicalize()
            .expect("linked worktree should canonicalize");
        assert_eq!(
            discovered.workspace_root.as_deref(),
            Some(canonical_worktree.as_path())
        );
        assert_eq!(
            discovered.project.as_deref(),
            Some(canonical_worktree.join(".akra/config.toml").as_path())
        );
        assert!(ConfigurationService::path_for_scope(&root, ConfigScope::Project).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn project_mutation_creates_a_reviewable_file_while_global_remains_private() {
        use std::os::unix::fs::PermissionsExt;

        let _lock = crate::test_utils::process_environment_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = temporary_directory("project-permissions");
        let workspace = root.join("workspace");
        fake_git_worktree(&workspace);
        let home = root.join("home");
        let home_text = home.to_string_lossy().into_owned();
        let _environment = EnvGuard::set(&[("AKRA_HOME", Some(&home_text))]);

        ConfigurationService::set(
            &workspace,
            ConfigScope::Project,
            SettingKey::TuiShowStartupVisual,
            "false",
        )
        .expect("project setting should create its configuration");
        let project_mode = fs::metadata(workspace.join(".akra/config.toml"))
            .expect("project configuration should exist")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(project_mode, 0o644);
        assert!(
            !workspace.join(".akra/.codex-exec-loop").exists(),
            "configuration writes must not create planning runtime state inside .akra"
        );

        ConfigurationService::set(
            &workspace,
            ConfigScope::Global,
            SettingKey::TuiShowStartupVisual,
            "true",
        )
        .expect("global setting should create its configuration");
        let global_mode = fs::metadata(home.join(CONFIG_FILE_NAME))
            .expect("global configuration should exist")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(global_mode, 0o600);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_unknown_and_wrong_scope_files_fail_before_startup() {
        let _lock = crate::test_utils::process_environment_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = temporary_directory("validation-failures");
        let home = root.join("home");
        let home_text = home.to_string_lossy().into_owned();
        let _environment = EnvGuard::set(&[("AKRA_HOME", Some(&home_text))]);

        write_global_fixture(&home, "[unknown]\nvalue = true\n");
        let unknown = ConfigurationService::resolve_read_only(&root, &[])
            .expect_err("unknown configuration keys must fail")
            .to_string();
        assert!(unknown.contains("invalid global configuration TOML"));

        write_global_fixture(&home, "[tui]\nshow_startup_visual = \"yes\"\n");
        let wrong_type = ConfigurationService::resolve_read_only(&root, &[])
            .expect_err("wrong configuration types must fail")
            .to_string();
        assert!(wrong_type.contains("invalid global configuration TOML"));

        write_global_fixture(&home, "[conversation]\nmodel = \" custom-model \"\n");
        let noncanonical_text = ConfigurationService::resolve_read_only(&root, &[])
            .expect_err("TOML model values must not retain whitespace aliases")
            .to_string();
        assert!(noncanonical_text.contains("must not contain whitespace"));

        let workspace = root.join("workspace");
        fake_git_worktree(&workspace);
        write_global_fixture(&home, "schema_version = 1\n");
        let project = workspace.join(".akra/config.toml");
        fs::create_dir_all(project.parent().expect("project config has parent"))
            .expect("project config parent should create");
        fs::write(&project, "[diagnostics]\ntrace = \"full\"\n")
            .expect("wrong-scope project fixture should write");
        let wrong_scope = ConfigurationService::resolve_read_only(&workspace, &[])
            .expect_err("machine-only project setting must fail")
            .to_string();
        assert!(
            wrong_scope.contains("not permitted in project"),
            "{wrong_scope}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn global_symlink_and_hardlink_are_rejected_without_reading_the_target() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let _lock = crate::test_utils::process_environment_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = temporary_directory("unsafe-links");
        let home = root.join("home");
        let home_text = home.to_string_lossy().into_owned();
        let _environment = EnvGuard::set(&[("AKRA_HOME", Some(&home_text))]);
        fs::create_dir_all(&home).expect("global home should create");
        let target = root.join("target.toml");
        fs::write(&target, "schema_version = 1\n").expect("target fixture should write");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600))
            .expect("target fixture should be private");
        symlink(&target, home.join(CONFIG_FILE_NAME)).expect("global symlink should create");
        let symlink_error = ConfigurationService::resolve_read_only(&root, &[])
            .expect_err("global symlink must fail")
            .to_string();
        assert!(symlink_error.contains("regular non-symlink"));

        fs::remove_file(home.join(CONFIG_FILE_NAME)).expect("symlink fixture should remove");
        fs::hard_link(&target, home.join(CONFIG_FILE_NAME)).expect("global hardlink should create");
        let hardlink_error = ConfigurationService::resolve_read_only(&root, &[])
            .expect_err("global hardlink must fail")
            .to_string();
        assert!(hardlink_error.contains("hard-linked"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn concurrent_global_writers_keep_independent_leaf_updates() {
        let _lock = crate::test_utils::process_environment_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = temporary_directory("concurrent-writers");
        let home = root.join("home");
        let home_text = home.to_string_lossy().into_owned();
        let _environment = EnvGuard::set(&[("AKRA_HOME", Some(&home_text))]);
        let first_root = root.clone();
        let second_root = root.clone();
        let first = std::thread::spawn(move || {
            ConfigurationService::set(
                &first_root,
                ConfigScope::Global,
                SettingKey::TuiShowStartupVisual,
                "false",
            )
        });
        let second = std::thread::spawn(move || {
            ConfigurationService::set(
                &second_root,
                ConfigScope::Global,
                SettingKey::GithubReviewPollIntervalSecs,
                "120",
            )
        });
        first
            .join()
            .expect("first writer should not panic")
            .expect("first writer should succeed");
        second
            .join()
            .expect("second writer should not panic")
            .expect("second writer should succeed");

        let layer = ConfigurationService::layer_for_scope(&root, ConfigScope::Global)
            .expect("global layer should read")
            .expect("writer should create global layer");
        assert_eq!(
            layer
                .configured_value(SettingKey::TuiShowStartupVisual)
                .as_deref(),
            Some("false")
        );
        assert_eq!(
            layer
                .configured_value(SettingKey::GithubReviewPollIntervalSecs)
                .as_deref(),
            Some("120")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn persistent_config_lock_file_is_reacquired_after_the_holder_exits() {
        let _lock = crate::test_utils::process_environment_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = temporary_directory("persistent-lock-file");
        let home = root.join("home");
        let home_text = home.to_string_lossy().into_owned();
        let _environment = EnvGuard::set(&[("AKRA_HOME", Some(&home_text))]);
        let config_path = home.join(CONFIG_FILE_NAME);

        let first = ConfigWriteLock::acquire(&config_path).expect("first lock should acquire");
        let lock_path = config_write_lock_path(&config_path).expect("lock path should resolve");
        assert!(
            lock_path.is_file(),
            "the durable lock file should remain visible"
        );
        drop(first);

        let second = ConfigWriteLock::acquire(&config_path)
            .expect("an unlocked persistent lock file should be reacquired");
        drop(second);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn legacy_config_lock_directory_fails_closed_during_migration() {
        let _lock = crate::test_utils::process_environment_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = temporary_directory("legacy-lock-directory");
        let home = root.join("home");
        let home_text = home.to_string_lossy().into_owned();
        let _environment = EnvGuard::set(&[("AKRA_HOME", Some(&home_text))]);
        let config_path = home.join(CONFIG_FILE_NAME);
        let lock_path = config_write_lock_path(&config_path).expect("lock path should resolve");
        fs::create_dir(&lock_path).expect("legacy directory lock should create");

        let error = match ConfigWriteLock::acquire(&config_path) {
            Ok(_) => panic!("a legacy directory lock must not be reclaimed blindly"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("legacy configuration lock directory found"));
        assert!(error.contains("confirm no older Akra process is running"));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn project_ancestor_replacement_race_cannot_redirect_a_configuration_write() {
        use std::os::unix::fs::symlink;

        let _lock = crate::test_utils::process_environment_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = temporary_directory("project-ancestor-race");
        let workspace = root.join("workspace");
        fake_git_worktree(&workspace);
        let home = root.join("home");
        let home_text = home.to_string_lossy().into_owned();
        let _environment = EnvGuard::set(&[("AKRA_HOME", Some(&home_text))]);
        let project_directory = workspace.join(PROJECT_CONFIG_DIRECTORY);
        fs::create_dir_all(&project_directory).expect("project config directory should create");
        let parked_project_directory = root.join("parked-project-config");
        let attacker_directory = root.join("attacker-directory");
        fs::create_dir_all(&attacker_directory).expect("attacker fixture directory should create");
        let hook_project_directory = project_directory.clone();
        let hook_parked_project_directory = parked_project_directory.clone();
        let hook_attacker_directory = attacker_directory.clone();

        crate::adapter::outbound::filesystem::secure_fs::install_before_atomic_replace_hook(
            move || {
                fs::rename(&hook_project_directory, &hook_parked_project_directory)
                    .expect("test hook should park the original project directory");
                symlink(&hook_attacker_directory, &hook_project_directory)
                    .expect("test hook should replace the project directory with a symlink");
            },
        );

        let error = ConfigurationService::set(
            &workspace,
            ConfigScope::Project,
            SettingKey::TuiShowStartupVisual,
            "false",
        )
        .expect_err("a replaced project ancestor must fail closed")
        .to_string();

        assert!(
            error.contains("failed to atomically replace configuration"),
            "unexpected race error: {error}"
        );
        assert!(
            !attacker_directory.join(CONFIG_FILE_NAME).exists(),
            "the configuration write must never follow the replacement symlink"
        );
        let _ = fs::remove_dir_all(root);
    }
}
