use anyhow::Result;

/// Controls whether an inactive bot stream may move to another canonical workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramWorkspaceBindingPolicy {
    Preserve,
    RebindInactive,
}

/// Result of acquiring the machine-wide runner gate for one numeric Telegram bot id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TelegramGlobalRunnerLeaseClaimDecision {
    Acquired { generation: u64 },
    ActiveRunner { bound_workspace_dir: String },
    WorkspaceBindingMismatch { bound_workspace_dir: String },
}

/// Stable OS-user ownership boundary for Telegram's one global update stream per bot.
pub trait TelegramGlobalRunnerLeasePort: Send + Sync {
    /// Acquires the bot stream and returns its monotonic fencing generation.
    /// The adapter must bind `owner_pid` to the OS process-start identity observed at acquisition.
    fn try_acquire_global_runner_lease(
        &self,
        canonical_workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        owner_pid: u32,
        lease_ttl_seconds: u64,
        binding_policy: TelegramWorkspaceBindingPolicy,
    ) -> Result<TelegramGlobalRunnerLeaseClaimDecision>;

    /// Renews only the exact bot, workspace identity, PID/start identity, owner, and generation tuple.
    fn renew_global_runner_lease(
        &self,
        canonical_workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        owner_pid: u32,
        generation: u64,
        lease_ttl_seconds: u64,
    ) -> Result<()>;

    /// Releases ownership while preserving the bot-to-workspace binding and process fencing.
    fn release_global_runner_lease(
        &self,
        canonical_workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        owner_pid: u32,
        generation: u64,
    ) -> Result<bool>;
}
