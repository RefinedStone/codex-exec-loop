use anyhow::Result;

/// Durable decision returned when a Telegram update is reserved for execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramUpdateClaimDecision {
    Execute,
    AlreadyCompleted,
    InFlight,
}

/// Result of trying to become the only Telegram runner for one workspace stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramRunnerLeaseClaimDecision {
    Acquired,
    ActiveRunner,
}

/// Bot-id-scoped idempotency boundary for the Telegram update stream.
///
/// Production uses the stable global Telegram store. The runner-lease methods remain only for
/// reading and migrating pre-v2 repository-local ledgers.
pub trait TelegramUpdateLedgerPort: Send + Sync {
    /// Atomically acquires an expired or absent runner lease.
    fn try_acquire_runner_lease(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        lease_ttl_seconds: u64,
    ) -> Result<TelegramRunnerLeaseClaimDecision>;

    /// Extends a live lease only when the caller still owns it.
    fn renew_runner_lease(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        lease_ttl_seconds: u64,
    ) -> Result<()>;

    /// Releases a lease with an owner compare-and-delete operation.
    fn release_runner_lease(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
    ) -> Result<bool>;

    /// Recovers an interrupted execution as completed and returns the durable poll offset.
    ///
    /// An interrupted command is not replayed because its external side effects cannot be
    /// atomically committed with SQLite. This deliberately chooses at-most-once execution.
    fn load_cursor_and_recover_inflight(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
    ) -> Result<Option<i64>>;

    /// Monotonically persists a cursor floor before polling.
    fn advance_cursor(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        next_offset: Option<i64>,
    ) -> Result<Option<i64>>;

    /// Reserves one update before any command or reply side effect is attempted.
    fn claim_update(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        update_id: i64,
    ) -> Result<TelegramUpdateClaimDecision>;

    /// Atomically completes one update and advances the durable poll offset.
    fn complete_update(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        update_id: i64,
    ) -> Result<Option<i64>>;
}
