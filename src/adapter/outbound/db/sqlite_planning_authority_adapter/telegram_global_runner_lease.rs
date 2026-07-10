use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};

use crate::application::port::outbound::telegram_global_runner_lease_port::{
    TelegramGlobalRunnerLeaseClaimDecision, TelegramGlobalRunnerLeasePort,
    TelegramWorkspaceBindingPolicy,
};
use crate::application::port::outbound::telegram_update_ledger_port::{
    TelegramRunnerLeaseClaimDecision, TelegramUpdateClaimDecision, TelegramUpdateLedgerPort,
};
use crate::process_liveness::{
    process_is_alive, process_start_identity, required_process_start_identity,
};

use super::{
    AUTHORITY_STORE_BUSY_TIMEOUT, SqlitePlanningAuthorityAdapter,
    configure_private_authority_connection, prepare_private_authority_directory_tree,
    prepare_private_authority_sidecar_files, prepare_private_authority_store_file,
    validate_authority_store_path_identity,
};

const GLOBAL_TELEGRAM_STORE_APPLICATION_ID: i64 = 0x414B_5447;
const GLOBAL_TELEGRAM_STORE_SCHEMA_VERSION: i64 = 3;
const PREVIOUS_GLOBAL_TELEGRAM_STORE_SCHEMA_VERSION: i64 = 2;
const LEGACY_GLOBAL_TELEGRAM_STORE_SCHEMA_VERSION: i64 = 1;
const GLOBAL_DIRECTORY: &str = "global";
const TELEGRAM_DIRECTORY: &str = "telegram";
const RUNTIME_DIRECTORY: &str = "runtime";
const STORE_FILE_NAME: &str = "runner-leases.db";
const MAX_STREAM_KEY_BYTES: usize = 128;
const MAX_OWNER_TOKEN_BYTES: usize = 128;
const MAX_PROCESS_START_IDENTITY_BYTES: usize = 256;
const MAX_WORKSPACE_BYTES: usize = 16 * 1024;
const COMPLETED_UPDATE_RETENTION_OFFSET: i64 = 511;

/// SQLite implementation of the machine-wide Telegram stream gate.
#[derive(Debug, Clone, Default)]
pub struct SqliteTelegramGlobalRunnerLeaseAdapter {
    store_path_override: Option<PathBuf>,
}

impl SqliteTelegramGlobalRunnerLeaseAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub(crate) fn with_store_path(store_path: PathBuf) -> Self {
        Self {
            store_path_override: Some(store_path),
        }
    }

    fn store_path(&self) -> Result<PathBuf> {
        let store_path = match self.store_path_override.clone() {
            Some(path) => path,
            None => stable_telegram_data_root()?
                .join(GLOBAL_DIRECTORY)
                .join(TELEGRAM_DIRECTORY)
                .join(RUNTIME_DIRECTORY)
                .join(STORE_FILE_NAME),
        };
        if !store_path.is_absolute() {
            bail!(
                "global Telegram runner store requires an absolute Akra data root: {}",
                store_path.display()
            );
        }
        Ok(store_path)
    }

    fn open_connection(&self) -> Result<Connection> {
        let store_path = self.store_path()?;
        let parent = store_path.parent().ok_or_else(|| {
            anyhow!(
                "global Telegram runner store path has no parent: {}",
                store_path.display()
            )
        })?;
        let _directory_anchors = prepare_private_authority_directory_tree(parent)
            .context("failed to prepare private global Telegram runner directories")?;
        let store_anchor = prepare_private_authority_store_file(&store_path)
            .context("failed to prepare private global Telegram runner store")?;
        let _sidecar_anchors = prepare_private_authority_sidecar_files(&store_path)
            .context("failed to prepare private global Telegram runner sidecars")?;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let mut connection = Connection::open_with_flags(&store_path, flags)
            .with_context(|| format!("failed to open {}", store_path.display()))?;
        connection
            .busy_timeout(AUTHORITY_STORE_BUSY_TIMEOUT)
            .context("failed to configure global Telegram runner store busy timeout")?;
        validate_authority_store_path_identity(&store_path, &store_anchor)
            .context("global Telegram runner store identity changed while opening")?;
        configure_private_authority_connection(&connection)
            .context("failed to configure private global Telegram runner store")?;
        let _current_sidecar_anchors = prepare_private_authority_sidecar_files(&store_path)
            .context("failed to validate global Telegram runner sidecars")?;
        ensure_schema(&mut connection)?;
        validate_authority_store_path_identity(&store_path, &store_anchor)
            .context("global Telegram runner store identity changed during schema setup")?;
        let _final_sidecar_anchors = prepare_private_authority_sidecar_files(&store_path)
            .context("failed to validate final global Telegram runner sidecars")?;
        Ok(connection)
    }
}

#[cfg(unix)]
fn stable_telegram_data_root() -> Result<PathBuf> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStringExt;

    let uid = unsafe { libc::geteuid() };
    let buffer_size = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let buffer_size = if buffer_size <= 0 {
        16 * 1024
    } else {
        usize::try_from(buffer_size)
            .unwrap_or(16 * 1024)
            .min(1024 * 1024)
    };
    let mut record = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut result = std::ptr::null_mut();
    let mut buffer = vec![0_u8; buffer_size];
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            record.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    if status != 0 || result.is_null() {
        bail!("failed to resolve the stable operating-system home for Telegram runtime data");
    }
    let record = unsafe { record.assume_init() };
    if record.pw_dir.is_null() {
        bail!("operating-system account has no home directory for Telegram runtime data");
    }
    let home = PathBuf::from(std::ffi::OsString::from_vec(
        unsafe { CStr::from_ptr(record.pw_dir) }.to_bytes().to_vec(),
    ));
    if !home.is_absolute() {
        bail!("operating-system account home must be absolute for Telegram runtime data");
    }
    Ok(home.join(".akra"))
}

#[cfg(windows)]
fn stable_telegram_data_root() -> Result<PathBuf> {
    Ok(crate::private_fs::windows_local_app_data_path()?.join("Akra"))
}

#[cfg(not(any(unix, windows)))]
fn stable_telegram_data_root() -> Result<PathBuf> {
    bail!("stable Telegram runtime storage is unsupported on this platform")
}

impl TelegramGlobalRunnerLeasePort for SqliteTelegramGlobalRunnerLeaseAdapter {
    fn try_acquire_global_runner_lease(
        &self,
        canonical_workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        owner_pid: u32,
        lease_ttl_seconds: u64,
        binding_policy: TelegramWorkspaceBindingPolicy,
    ) -> Result<TelegramGlobalRunnerLeaseClaimDecision> {
        validate_inputs(canonical_workspace_dir, stream_key, owner_token, owner_pid)?;
        let owner_start_identity = Some(
            required_process_start_identity(owner_pid)
                .context("global Telegram runner lease requires a process birth identity")?,
        );
        validate_process_start_identity(owner_start_identity.as_deref())?;
        let canonical_workspace_dir = validate_canonical_workspace(canonical_workspace_dir)?;
        let workspace_identity = workspace_identity(Path::new(&canonical_workspace_dir))?;
        let legacy_snapshot = load_legacy_workspace_ledger(&canonical_workspace_dir, stream_key)?;
        let (now_millis, expires_at_millis) = lease_deadline(lease_ttl_seconds)?;
        let mut connection = self.open_connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open global Telegram runner lease transaction")?;
        let existing = load_binding(&transaction, stream_key)?;
        let decision = match existing {
            None => {
                insert_binding(
                    &transaction,
                    stream_key,
                    &LeaseWrite::new(
                        &canonical_workspace_dir,
                        &workspace_identity,
                        owner_token,
                        owner_pid,
                        owner_start_identity.as_deref(),
                        expires_at_millis,
                        1,
                    ),
                )?;
                TelegramGlobalRunnerLeaseClaimDecision::Acquired { generation: 1 }
            }
            Some(binding)
                if binding.workspace_dir == canonical_workspace_dir
                    && binding.workspace_identity == workspace_identity
                    && binding.owner_token.as_deref() == Some(owner_token)
                    && binding.owner_pid == Some(owner_pid)
                    && (binding.owner_start_identity.is_none()
                        || binding.owner_start_identity.as_deref()
                            == owner_start_identity.as_deref()) =>
            {
                update_owned_lease(
                    &transaction,
                    stream_key,
                    &LeaseWrite::new(
                        &canonical_workspace_dir,
                        &workspace_identity,
                        owner_token,
                        owner_pid,
                        owner_start_identity.as_deref(),
                        expires_at_millis,
                        binding.generation,
                    ),
                )?;
                TelegramGlobalRunnerLeaseClaimDecision::Acquired {
                    generation: binding.generation,
                }
            }
            Some(binding) if binding.blocks_takeover(now_millis)? => {
                TelegramGlobalRunnerLeaseClaimDecision::ActiveRunner {
                    bound_workspace_dir: binding.workspace_dir,
                }
            }
            Some(binding)
                if binding.workspace_dir == canonical_workspace_dir
                    && binding.workspace_identity == workspace_identity =>
            {
                let generation = binding.next_generation()?;
                update_owned_lease(
                    &transaction,
                    stream_key,
                    &LeaseWrite::new(
                        &canonical_workspace_dir,
                        &workspace_identity,
                        owner_token,
                        owner_pid,
                        owner_start_identity.as_deref(),
                        expires_at_millis,
                        generation,
                    ),
                )?;
                TelegramGlobalRunnerLeaseClaimDecision::Acquired { generation }
            }
            Some(binding) if binding_policy == TelegramWorkspaceBindingPolicy::RebindInactive => {
                let generation = binding.next_generation()?;
                rebind_inactive_lease(
                    &transaction,
                    stream_key,
                    &LeaseWrite::new(
                        &canonical_workspace_dir,
                        &workspace_identity,
                        owner_token,
                        owner_pid,
                        owner_start_identity.as_deref(),
                        expires_at_millis,
                        generation,
                    ),
                    now_millis,
                )?;
                TelegramGlobalRunnerLeaseClaimDecision::Acquired { generation }
            }
            Some(binding) => TelegramGlobalRunnerLeaseClaimDecision::WorkspaceBindingMismatch {
                bound_workspace_dir: binding.workspace_dir,
            },
        };
        if matches!(
            decision,
            TelegramGlobalRunnerLeaseClaimDecision::Acquired { .. }
        ) {
            let binding = load_binding(&transaction, stream_key)?
                .context("acquired global Telegram binding disappeared")?;
            migrate_legacy_snapshot(
                &transaction,
                stream_key,
                &workspace_identity,
                &legacy_snapshot,
                binding.generation,
            )?;
        }
        transaction
            .commit()
            .context("failed to commit global Telegram runner lease transaction")?;
        Ok(decision)
    }

    fn renew_global_runner_lease(
        &self,
        canonical_workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        owner_pid: u32,
        generation: u64,
        lease_ttl_seconds: u64,
    ) -> Result<()> {
        validate_inputs(canonical_workspace_dir, stream_key, owner_token, owner_pid)?;
        let owner_start_identity =
            Some(required_process_start_identity(owner_pid).context(
                "global Telegram runner lease renewal requires a process birth identity",
            )?);
        validate_process_start_identity(owner_start_identity.as_deref())?;
        let canonical_workspace_dir = validate_canonical_workspace(canonical_workspace_dir)?;
        let workspace_identity = workspace_identity(Path::new(&canonical_workspace_dir))?;
        let (_, expires_at_millis) = lease_deadline(lease_ttl_seconds)?;
        let mut connection = self.open_connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open global Telegram runner lease renewal transaction")?;
        let changed = transaction
            .execute(
                "UPDATE telegram_global_runner_bindings
                 SET owner_process_start_identity =
                         COALESCE(owner_process_start_identity, ?6),
                     lease_expires_at_epoch_millis = ?8, updated_at = ?9
                 WHERE stream_key = ?1 AND canonical_workspace_dir = ?2
                   AND workspace_identity = ?3 AND owner_token = ?4
                   AND owner_pid = ?5
                   AND (owner_process_start_identity IS NULL
                        OR owner_process_start_identity = ?6)
                   AND lease_generation = ?7",
                params![
                    stream_key,
                    canonical_workspace_dir,
                    workspace_identity,
                    owner_token,
                    owner_pid,
                    owner_start_identity,
                    generation,
                    expires_at_millis,
                    Utc::now().to_rfc3339(),
                ],
            )
            .context("failed to renew global Telegram runner lease")?;
        if changed != 1 {
            bail!("global Telegram runner lease is no longer owned, bound, or active");
        }
        transaction
            .commit()
            .context("failed to commit global Telegram runner lease renewal")?;
        Ok(())
    }

    fn release_global_runner_lease(
        &self,
        canonical_workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        owner_pid: u32,
        generation: u64,
    ) -> Result<bool> {
        validate_inputs(canonical_workspace_dir, stream_key, owner_token, owner_pid)?;
        let owner_start_identity =
            Some(required_process_start_identity(owner_pid).context(
                "global Telegram runner lease release requires a process birth identity",
            )?);
        validate_process_start_identity(owner_start_identity.as_deref())?;
        let canonical_workspace_dir = validate_canonical_workspace(canonical_workspace_dir)?;
        let workspace_identity = workspace_identity(Path::new(&canonical_workspace_dir))?;
        let mut connection = self.open_connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open global Telegram runner lease release transaction")?;
        let changed = transaction
            .execute(
                "UPDATE telegram_global_runner_bindings
                 SET owner_token = NULL, owner_pid = NULL,
                     owner_process_start_identity = NULL,
                     lease_expires_at_epoch_millis = NULL, updated_at = ?8
                 WHERE stream_key = ?1 AND canonical_workspace_dir = ?2
                   AND workspace_identity = ?3 AND owner_token = ?4
                   AND owner_pid = ?5
                   AND (owner_process_start_identity IS NULL
                        OR owner_process_start_identity = ?6)
                   AND lease_generation = ?7",
                params![
                    stream_key,
                    canonical_workspace_dir,
                    workspace_identity,
                    owner_token,
                    owner_pid,
                    owner_start_identity,
                    generation,
                    Utc::now().to_rfc3339(),
                ],
            )
            .context("failed to release global Telegram runner lease")?;
        transaction
            .commit()
            .context("failed to commit global Telegram runner lease release")?;
        Ok(changed == 1)
    }
}

impl TelegramUpdateLedgerPort for SqliteTelegramGlobalRunnerLeaseAdapter {
    fn try_acquire_runner_lease(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        _lease_ttl_seconds: u64,
    ) -> Result<TelegramRunnerLeaseClaimDecision> {
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let owned =
            assert_owned_active_lease(&transaction, workspace_dir, stream_key, owner_token).is_ok();
        transaction.commit()?;
        Ok(if owned {
            TelegramRunnerLeaseClaimDecision::Acquired
        } else {
            TelegramRunnerLeaseClaimDecision::ActiveRunner
        })
    }

    fn renew_runner_lease(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        lease_ttl_seconds: u64,
    ) -> Result<()> {
        let connection = self.open_connection()?;
        let binding = load_binding_from_connection(&connection, stream_key)?
            .context("global Telegram stream binding is missing")?;
        let owner_pid = binding
            .owner_pid
            .context("global Telegram stream has no process owner")?;
        self.renew_global_runner_lease(
            workspace_dir,
            stream_key,
            owner_token,
            owner_pid,
            binding.generation,
            lease_ttl_seconds,
        )
    }

    fn release_runner_lease(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
    ) -> Result<bool> {
        let connection = self.open_connection()?;
        let Some(binding) = load_binding_from_connection(&connection, stream_key)? else {
            return Ok(false);
        };
        let Some(owner_pid) = binding.owner_pid else {
            return Ok(false);
        };
        self.release_global_runner_lease(
            workspace_dir,
            stream_key,
            owner_token,
            owner_pid,
            binding.generation,
        )
    }

    fn load_cursor_and_recover_inflight(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
    ) -> Result<Option<i64>> {
        self.ensure_legacy_workspace_migrated(workspace_dir, stream_key, owner_token)?;
        let mut connection = self.open_connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open global Telegram update recovery transaction")?;
        let binding =
            assert_owned_active_lease(&transaction, workspace_dir, stream_key, owner_token)?;
        let interrupted_update_id = transaction
            .query_row(
                "SELECT MAX(update_id) FROM telegram_global_update_inbox
                 WHERE stream_key = ?1 AND update_state = 'executing'",
                [stream_key],
                |row| row.get::<_, Option<i64>>(0),
            )
            .context("failed to inspect interrupted global Telegram updates")?;
        if let Some(update_id) = interrupted_update_id {
            let next_offset = checked_next_offset(update_id)?;
            let completed_at = Utc::now().to_rfc3339();
            transaction.execute(
                "UPDATE telegram_global_update_inbox
                 SET update_state = 'completed', completed_at = ?2,
                     claim_generation = MAX(claim_generation, ?3)
                 WHERE stream_key = ?1 AND update_state = 'executing'",
                params![stream_key, completed_at, binding.generation],
            )?;
            persist_global_cursor(&transaction, stream_key, next_offset, &completed_at)?;
            prune_global_completed_updates(&transaction, stream_key)?;
        }
        let cursor = load_global_cursor(&transaction, stream_key)?;
        transaction.commit()?;
        Ok(cursor)
    }

    fn advance_cursor(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        next_offset: Option<i64>,
    ) -> Result<Option<i64>> {
        self.ensure_legacy_workspace_migrated(workspace_dir, stream_key, owner_token)?;
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        assert_owned_active_lease(&transaction, workspace_dir, stream_key, owner_token)?;
        if let Some(next_offset) = next_offset {
            persist_global_cursor(
                &transaction,
                stream_key,
                next_offset,
                &Utc::now().to_rfc3339(),
            )?;
        }
        let cursor = load_global_cursor(&transaction, stream_key)?;
        transaction.commit()?;
        Ok(cursor)
    }

    fn claim_update(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        update_id: i64,
    ) -> Result<TelegramUpdateClaimDecision> {
        self.ensure_legacy_workspace_migrated(workspace_dir, stream_key, owner_token)?;
        let _ = checked_next_offset(update_id)?;
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let binding =
            assert_owned_active_lease(&transaction, workspace_dir, stream_key, owner_token)?;
        if load_global_cursor(&transaction, stream_key)?.is_some_and(|offset| update_id < offset) {
            transaction.commit()?;
            return Ok(TelegramUpdateClaimDecision::AlreadyCompleted);
        }
        let existing = transaction
            .query_row(
                "SELECT update_state FROM telegram_global_update_inbox
                 WHERE stream_key = ?1 AND update_id = ?2",
                params![stream_key, update_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(state) = existing {
            let decision = match state.as_str() {
                "completed" => TelegramUpdateClaimDecision::AlreadyCompleted,
                "executing" => TelegramUpdateClaimDecision::InFlight,
                _ => bail!("global Telegram update inbox contains unsupported state"),
            };
            transaction.commit()?;
            return Ok(decision);
        }
        transaction.execute(
            "INSERT INTO telegram_global_update_inbox
             (stream_key, update_id, update_state, received_at, completed_at, claim_generation)
             VALUES (?1, ?2, 'executing', ?3, NULL, ?4)",
            params![
                stream_key,
                update_id,
                Utc::now().to_rfc3339(),
                binding.generation
            ],
        )?;
        transaction.commit()?;
        Ok(TelegramUpdateClaimDecision::Execute)
    }

    fn complete_update(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        update_id: i64,
    ) -> Result<Option<i64>> {
        let next_offset = checked_next_offset(update_id)?;
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let binding =
            assert_owned_active_lease(&transaction, workspace_dir, stream_key, owner_token)?;
        let completed_at = Utc::now().to_rfc3339();
        let changed = transaction.execute(
            "UPDATE telegram_global_update_inbox
             SET update_state = 'completed', completed_at = ?3
             WHERE stream_key = ?1 AND update_id = ?2 AND update_state = 'executing'
               AND claim_generation = ?4",
            params![stream_key, update_id, completed_at, binding.generation],
        )?;
        if changed == 0 {
            let state = transaction
                .query_row(
                    "SELECT update_state FROM telegram_global_update_inbox
                     WHERE stream_key = ?1 AND update_id = ?2",
                    params![stream_key, update_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            match state.as_deref() {
                Some("completed") => {}
                Some(_) => bail!("Telegram update claim belongs to another lease generation"),
                None => bail!("Telegram update must be claimed before completion"),
            }
        }
        persist_global_cursor(&transaction, stream_key, next_offset, &completed_at)?;
        prune_global_completed_updates(&transaction, stream_key)?;
        let cursor = load_global_cursor(&transaction, stream_key)?;
        transaction.commit()?;
        Ok(cursor)
    }
}

impl SqliteTelegramGlobalRunnerLeaseAdapter {
    fn ensure_legacy_workspace_migrated(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
    ) -> Result<()> {
        let canonical_workspace = validate_canonical_workspace(workspace_dir)?;
        let source_identity = workspace_identity(Path::new(&canonical_workspace))?;
        {
            let mut connection = self.open_connection()?;
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            assert_owned_active_lease(&transaction, workspace_dir, stream_key, owner_token)?;
            if global_migration_exists(&transaction, stream_key, &source_identity)? {
                transaction.commit()?;
                return Ok(());
            }
            transaction.commit()?;
        }

        let snapshot = load_legacy_workspace_ledger(workspace_dir, stream_key)?;
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let binding =
            assert_owned_active_lease(&transaction, workspace_dir, stream_key, owner_token)?;
        if global_migration_exists(&transaction, stream_key, &source_identity)? {
            transaction.commit()?;
            return Ok(());
        }
        migrate_legacy_snapshot(
            &transaction,
            stream_key,
            &source_identity,
            &snapshot,
            binding.generation,
        )?;
        transaction.commit()?;
        Ok(())
    }
}

#[derive(Default)]
struct LegacyLedgerSnapshot {
    next_offset: Option<i64>,
    update_ids: Vec<i64>,
}

fn migrate_legacy_snapshot(
    transaction: &Transaction<'_>,
    stream_key: &str,
    source_identity: &str,
    snapshot: &LegacyLedgerSnapshot,
    generation: u64,
) -> Result<()> {
    if global_migration_exists(transaction, stream_key, source_identity)? {
        return Ok(());
    }
    let migrated_at = Utc::now().to_rfc3339();
    let mut cursor_floor = snapshot.next_offset;
    for &update_id in &snapshot.update_ids {
        let next = checked_next_offset(update_id)?;
        cursor_floor = Some(cursor_floor.map_or(next, |current| current.max(next)));
        transaction.execute(
            "INSERT INTO telegram_global_update_inbox
             (stream_key, update_id, update_state, received_at, completed_at, claim_generation)
             VALUES (?1, ?2, 'completed', ?3, ?3, ?4)
             ON CONFLICT(stream_key, update_id) DO UPDATE SET
                 update_state = 'completed',
                 completed_at = COALESCE(telegram_global_update_inbox.completed_at, excluded.completed_at),
                 claim_generation = MAX(telegram_global_update_inbox.claim_generation,
                                        excluded.claim_generation)",
            params![stream_key, update_id, migrated_at, generation],
        )?;
    }
    if let Some(cursor_floor) = cursor_floor {
        persist_global_cursor(transaction, stream_key, cursor_floor, &migrated_at)?;
    }
    transaction.execute(
        "INSERT INTO telegram_global_ledger_migrations
         (stream_key, source_workspace_identity, migrated_at) VALUES (?1, ?2, ?3)",
        params![stream_key, source_identity, migrated_at],
    )?;
    prune_global_completed_updates(transaction, stream_key)
}

fn load_legacy_workspace_ledger(
    workspace_dir: &str,
    stream_key: &str,
) -> Result<LegacyLedgerSnapshot> {
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace_dir)?;
    let store_path = Path::new(&location.authority_store_path);
    if !store_path.exists() {
        return Ok(LegacyLedgerSnapshot::default());
    }
    // Route legacy reads through the same private main-file/sidecar boundary as every
    // authority write. A read-only WAL connection may still create or update `-shm`,
    // so opening this path directly would bypass sidecar hardlink validation.
    let connection = super::open_authority_connection(&location)
        .context("failed to securely open legacy workspace Telegram ledger")?;
    let tables: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'
         AND name IN ('telegram_update_streams', 'telegram_update_inbox')",
        [],
        |row| row.get(0),
    )?;
    if tables != 2 {
        return Ok(LegacyLedgerSnapshot::default());
    }
    let next_offset = connection
        .query_row(
            "SELECT next_offset FROM telegram_update_streams WHERE stream_key = ?1",
            [stream_key],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    let mut statement = connection.prepare(
        "SELECT update_id FROM telegram_update_inbox WHERE stream_key = ?1 ORDER BY update_id",
    )?;
    let update_ids = statement
        .query_map([stream_key], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(LegacyLedgerSnapshot {
        next_offset,
        update_ids,
    })
}

fn global_migration_exists(
    transaction: &Transaction<'_>,
    stream_key: &str,
    source_identity: &str,
) -> Result<bool> {
    transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM telegram_global_ledger_migrations
             WHERE stream_key = ?1 AND source_workspace_identity = ?2)",
            params![stream_key, source_identity],
            |row| row.get(0),
        )
        .context("failed to inspect global Telegram ledger migration")
}

fn assert_owned_active_lease(
    transaction: &Transaction<'_>,
    workspace_dir: &str,
    stream_key: &str,
    owner_token: &str,
) -> Result<StoredBinding> {
    let canonical_workspace = validate_canonical_workspace(workspace_dir)?;
    let identity = workspace_identity(Path::new(&canonical_workspace))?;
    let binding = load_binding(transaction, stream_key)?
        .context("global Telegram stream binding is missing")?;
    let process_identity_matches =
        match (binding.owner_pid, binding.owner_start_identity.as_deref()) {
            (Some(pid), Some(expected)) => {
                process_start_identity(pid)
                    .context("failed to verify global Telegram runner process birth identity")?
                    .as_deref()
                    == Some(expected)
            }
            (_, None) => true,
            (None, Some(_)) => false,
        };
    if binding.workspace_dir != canonical_workspace
        || binding.workspace_identity != identity
        || binding.owner_token.as_deref() != Some(owner_token)
        || !process_identity_matches
        || binding
            .expires_at_millis
            .is_none_or(|expires_at| expires_at <= Utc::now().timestamp_millis())
    {
        bail!("global Telegram runner lease is no longer owned, bound, or active");
    }
    Ok(binding)
}

fn load_binding_from_connection(
    connection: &Connection,
    stream_key: &str,
) -> Result<Option<StoredBinding>> {
    connection
        .query_row(
            "SELECT canonical_workspace_dir, workspace_identity, owner_token, owner_pid,
                    lease_expires_at_epoch_millis, lease_generation,
                    owner_process_start_identity
             FROM telegram_global_runner_bindings WHERE stream_key = ?1",
            [stream_key],
            |row| {
                Ok(StoredBinding {
                    workspace_dir: row.get(0)?,
                    workspace_identity: row.get(1)?,
                    owner_token: row.get(2)?,
                    owner_pid: row.get(3)?,
                    expires_at_millis: row.get(4)?,
                    generation: row.get(5)?,
                    owner_start_identity: row.get(6)?,
                })
            },
        )
        .optional()
        .context("failed to load global Telegram runner binding")
}

fn checked_next_offset(update_id: i64) -> Result<i64> {
    if update_id < 0 {
        bail!("Telegram update id must be non-negative");
    }
    update_id
        .checked_add(1)
        .context("Telegram update cursor overflowed")
}

fn load_global_cursor(transaction: &Transaction<'_>, stream_key: &str) -> Result<Option<i64>> {
    transaction
        .query_row(
            "SELECT next_offset FROM telegram_global_update_streams WHERE stream_key = ?1",
            [stream_key],
            |row| row.get(0),
        )
        .optional()
        .context("failed to load global Telegram cursor")
}

fn persist_global_cursor(
    transaction: &Transaction<'_>,
    stream_key: &str,
    next_offset: i64,
    updated_at: &str,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO telegram_global_update_streams (stream_key, next_offset, updated_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(stream_key) DO UPDATE SET
             next_offset = MAX(telegram_global_update_streams.next_offset, excluded.next_offset),
             updated_at = CASE WHEN excluded.next_offset >= telegram_global_update_streams.next_offset
                               THEN excluded.updated_at ELSE telegram_global_update_streams.updated_at END",
        params![stream_key, next_offset, updated_at],
    )?;
    Ok(())
}

fn prune_global_completed_updates(transaction: &Transaction<'_>, stream_key: &str) -> Result<()> {
    transaction.execute(
        "DELETE FROM telegram_global_update_inbox
         WHERE stream_key = ?1 AND update_state = 'completed'
           AND update_id < COALESCE(
               (SELECT update_id FROM telegram_global_update_inbox
                WHERE stream_key = ?1 AND update_state = 'completed'
                ORDER BY update_id DESC LIMIT 1 OFFSET ?2), 0)",
        params![stream_key, COMPLETED_UPDATE_RETENTION_OFFSET],
    )?;
    Ok(())
}

#[derive(Debug)]
struct StoredBinding {
    workspace_dir: String,
    workspace_identity: String,
    owner_token: Option<String>,
    owner_pid: Option<u32>,
    owner_start_identity: Option<String>,
    expires_at_millis: Option<i64>,
    generation: u64,
}

impl StoredBinding {
    fn blocks_takeover(&self, now_millis: i64) -> Result<bool> {
        if self.owner_token.is_none() {
            return Ok(false);
        }
        if self
            .expires_at_millis
            .is_some_and(|expires_at| expires_at > now_millis)
        {
            return Ok(true);
        }
        let (Some(owner_pid), Some(expected_identity)) =
            (self.owner_pid, self.owner_start_identity.as_deref())
        else {
            // Nullable process identities are accepted only for legacy on-disk rows.
            return Ok(false);
        };
        if !process_is_alive(owner_pid)
            .context("failed to verify global Telegram runner process liveness")?
        {
            return Ok(false);
        }
        match process_start_identity(owner_pid) {
            Ok(Some(current_identity)) => Ok(current_identity == expected_identity),
            // A live PID with an ambiguous birth identity cannot safely be reclaimed. Nullable
            // identities are accepted only on legacy rows handled above.
            Ok(None) => Ok(true),
            Err(error) => {
                Err(error).context("failed to verify global Telegram runner process birth identity")
            }
        }
    }

    fn next_generation(&self) -> Result<u64> {
        self.generation
            .checked_add(1)
            .context("global Telegram lease generation overflowed")
    }
}

fn ensure_schema(connection: &mut Connection) -> Result<()> {
    connection
        .execute_batch("PRAGMA foreign_keys = ON; PRAGMA secure_delete = ON;")
        .context("failed to enable global Telegram runner store safety pragmas")?;
    let application_id = connection
        .pragma_query_value(None, "application_id", |row| row.get::<_, i64>(0))
        .context("failed to inspect global Telegram runner store application id")?;
    let schema_version = connection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .context("failed to inspect global Telegram runner store schema version")?;
    let object_count = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .context("failed to inspect global Telegram runner store objects")?;
    if application_id == 0 && schema_version == 0 && object_count == 0 {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open global Telegram runner schema transaction")?;
        create_global_telegram_schema(&transaction)?;
        transaction
            .pragma_update(None, "application_id", GLOBAL_TELEGRAM_STORE_APPLICATION_ID)
            .context("failed to mark global Telegram runner store application id")?;
        transaction
            .pragma_update(None, "user_version", GLOBAL_TELEGRAM_STORE_SCHEMA_VERSION)
            .context("failed to mark global Telegram runner store schema version")?;
        transaction
            .commit()
            .context("failed to commit global Telegram runner schema")?;
        validate_global_telegram_schema(connection)?;
        return Ok(());
    }
    if application_id != GLOBAL_TELEGRAM_STORE_APPLICATION_ID {
        bail!(
            "unsupported global Telegram runner store identity or schema version: application_id={application_id}, schema_version={schema_version}"
        );
    }
    if schema_version == LEGACY_GLOBAL_TELEGRAM_STORE_SCHEMA_VERSION {
        migrate_global_telegram_schema_v1(connection)?;
    } else if schema_version == PREVIOUS_GLOBAL_TELEGRAM_STORE_SCHEMA_VERSION {
        migrate_global_telegram_schema_v2(connection)?;
    } else if schema_version != GLOBAL_TELEGRAM_STORE_SCHEMA_VERSION {
        bail!(
            "unsupported global Telegram runner store identity or schema version: application_id={application_id}, schema_version={schema_version}"
        );
    }
    validate_global_telegram_schema(connection)
}

fn create_global_telegram_schema(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS telegram_global_runner_bindings (
                 stream_key TEXT PRIMARY KEY,
                 canonical_workspace_dir TEXT NOT NULL,
                 workspace_identity TEXT NOT NULL,
                 owner_token TEXT,
                 owner_pid INTEGER,
                 lease_generation INTEGER NOT NULL DEFAULT 0 CHECK (lease_generation >= 0),
                 lease_expires_at_epoch_millis INTEGER,
                 updated_at TEXT NOT NULL,
                 owner_process_start_identity TEXT,
                 CHECK (
                     (owner_token IS NULL AND owner_pid IS NULL
                         AND owner_process_start_identity IS NULL
                         AND lease_expires_at_epoch_millis IS NULL)
                     OR (owner_token IS NOT NULL AND lease_expires_at_epoch_millis IS NOT NULL)
                 ),
                 CHECK (owner_pid IS NULL OR owner_pid > 0),
                 CHECK (owner_process_start_identity IS NULL OR owner_pid IS NOT NULL),
                 CHECK (lease_expires_at_epoch_millis IS NULL OR lease_expires_at_epoch_millis >= 0)
             );
             CREATE TABLE IF NOT EXISTS telegram_global_update_streams (
                 stream_key TEXT PRIMARY KEY,
                 next_offset INTEGER NOT NULL CHECK (next_offset >= 0),
                 updated_at TEXT NOT NULL,
                 FOREIGN KEY (stream_key) REFERENCES telegram_global_runner_bindings(stream_key)
                     ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS telegram_global_update_inbox (
                 stream_key TEXT NOT NULL,
                 update_id INTEGER NOT NULL CHECK (update_id >= 0),
                 update_state TEXT NOT NULL CHECK (update_state IN ('executing', 'completed')),
                 received_at TEXT NOT NULL,
                 completed_at TEXT,
                 claim_generation INTEGER NOT NULL CHECK (claim_generation >= 0),
                 PRIMARY KEY (stream_key, update_id),
                 FOREIGN KEY (stream_key) REFERENCES telegram_global_runner_bindings(stream_key)
                     ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_telegram_global_update_inbox_stream_state_id
                 ON telegram_global_update_inbox(stream_key, update_state, update_id DESC);
             CREATE TABLE IF NOT EXISTS telegram_global_ledger_migrations (
                 stream_key TEXT NOT NULL,
                 source_workspace_identity TEXT NOT NULL,
                 migrated_at TEXT NOT NULL,
                 PRIMARY KEY (stream_key, source_workspace_identity),
                 FOREIGN KEY (stream_key) REFERENCES telegram_global_runner_bindings(stream_key)
                     ON DELETE CASCADE
             );",
        )
        .context("failed to create global Telegram stream schema")
}

fn migrate_global_telegram_schema_v2(connection: &mut Connection) -> Result<()> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("failed to open global Telegram v2 migration transaction")?;
    transaction
        .execute_batch(
            "ALTER TABLE telegram_global_runner_bindings
                 ADD COLUMN owner_process_start_identity TEXT;",
        )
        .context("failed to add Telegram process start identity to the v2 store")?;
    transaction
        .pragma_update(None, "user_version", GLOBAL_TELEGRAM_STORE_SCHEMA_VERSION)
        .context("failed to mark global Telegram v3 schema")?;
    transaction
        .commit()
        .context("failed to commit global Telegram v2 migration")
}

fn migrate_global_telegram_schema_v1(connection: &mut Connection) -> Result<()> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("failed to open global Telegram v1 migration transaction")?;
    transaction
        .execute_batch(
            "ALTER TABLE telegram_global_runner_bindings
                 RENAME TO telegram_global_runner_bindings_v1;",
        )
        .context("failed to preserve the global Telegram v1 binding table")?;
    create_global_telegram_schema(&transaction)?;
    transaction
        .execute_batch(
            "INSERT INTO telegram_global_runner_bindings
             (stream_key, canonical_workspace_dir, workspace_identity, owner_token, owner_pid,
              lease_generation, lease_expires_at_epoch_millis, updated_at)
             SELECT stream_key, canonical_workspace_dir, '', owner_token, NULL, 0,
                    lease_expires_at_epoch_millis, updated_at
             FROM telegram_global_runner_bindings_v1;",
        )
        .context("failed to copy global Telegram v1 bindings")?;
    let workspace_paths = {
        let mut statement = transaction
            .prepare(
                "SELECT stream_key, canonical_workspace_dir FROM telegram_global_runner_bindings",
            )
            .context("failed to inspect v1 Telegram workspace bindings")?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .context("failed to query v1 Telegram workspace bindings")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read v1 Telegram workspace bindings")?
    };
    for (stream_key, workspace_dir) in workspace_paths {
        let identity = workspace_identity_for_stored_path(&workspace_dir);
        transaction
            .execute(
                "UPDATE telegram_global_runner_bindings
                 SET workspace_identity = ?2 WHERE stream_key = ?1",
                params![stream_key, identity],
            )
            .context("failed to migrate Telegram workspace identity")?;
    }
    transaction
        .execute_batch("DROP TABLE telegram_global_runner_bindings_v1;")
        .context("failed to remove migrated global Telegram v1 binding table")?;
    transaction
        .pragma_update(None, "user_version", GLOBAL_TELEGRAM_STORE_SCHEMA_VERSION)
        .context("failed to mark global Telegram v3 schema")?;
    transaction
        .commit()
        .context("failed to commit global Telegram v1 migration")
}

fn validate_global_telegram_schema(connection: &Connection) -> Result<()> {
    for (table, expected_columns, expected_primary_keys) in [
        (
            "telegram_global_runner_bindings",
            &[
                "stream_key",
                "canonical_workspace_dir",
                "workspace_identity",
                "owner_token",
                "owner_pid",
                "lease_generation",
                "lease_expires_at_epoch_millis",
                "updated_at",
                "owner_process_start_identity",
            ][..],
            &[("stream_key", 1_i64)][..],
        ),
        (
            "telegram_global_update_streams",
            &["stream_key", "next_offset", "updated_at"][..],
            &[("stream_key", 1_i64)][..],
        ),
        (
            "telegram_global_update_inbox",
            &[
                "stream_key",
                "update_id",
                "update_state",
                "received_at",
                "completed_at",
                "claim_generation",
            ][..],
            &[("stream_key", 1_i64), ("update_id", 2_i64)][..],
        ),
        (
            "telegram_global_ledger_migrations",
            &["stream_key", "source_workspace_identity", "migrated_at"][..],
            &[("stream_key", 1_i64), ("source_workspace_identity", 2_i64)][..],
        ),
    ] {
        let mut statement = connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .with_context(|| format!("failed to inspect global Telegram table {table}"))?;
        let columns = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let names = columns
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>();
        if names != expected_columns {
            bail!("global Telegram table `{table}` has an unexpected column layout");
        }
        let primary_keys = columns
            .iter()
            .filter(|(_, position)| *position > 0)
            .map(|(name, position)| (name.as_str(), *position))
            .collect::<Vec<_>>();
        if primary_keys != expected_primary_keys {
            bail!("global Telegram table `{table}` has an unexpected primary key");
        }
    }
    Ok(())
}

fn load_binding(
    transaction: &rusqlite::Transaction<'_>,
    stream_key: &str,
) -> Result<Option<StoredBinding>> {
    transaction
        .query_row(
            "SELECT canonical_workspace_dir, workspace_identity, owner_token, owner_pid,
                    lease_expires_at_epoch_millis, lease_generation,
                    owner_process_start_identity
             FROM telegram_global_runner_bindings WHERE stream_key = ?1",
            [stream_key],
            |row| {
                Ok(StoredBinding {
                    workspace_dir: row.get(0)?,
                    workspace_identity: row.get(1)?,
                    owner_token: row.get(2)?,
                    owner_pid: row.get(3)?,
                    expires_at_millis: row.get(4)?,
                    generation: row.get(5)?,
                    owner_start_identity: row.get(6)?,
                })
            },
        )
        .optional()
        .context("failed to load global Telegram runner binding")
}

struct LeaseWrite<'a> {
    workspace_dir: &'a str,
    workspace_identity: &'a str,
    owner_token: &'a str,
    owner_pid: u32,
    owner_start_identity: Option<&'a str>,
    expires_at_millis: i64,
    generation: u64,
}

impl<'a> LeaseWrite<'a> {
    fn new(
        workspace_dir: &'a str,
        workspace_identity: &'a str,
        owner_token: &'a str,
        owner_pid: u32,
        owner_start_identity: Option<&'a str>,
        expires_at_millis: i64,
        generation: u64,
    ) -> Self {
        Self {
            workspace_dir,
            workspace_identity,
            owner_token,
            owner_pid,
            owner_start_identity,
            expires_at_millis,
            generation,
        }
    }
}

fn insert_binding(
    transaction: &rusqlite::Transaction<'_>,
    stream_key: &str,
    lease: &LeaseWrite<'_>,
) -> Result<()> {
    transaction
        .execute(
            "INSERT INTO telegram_global_runner_bindings
             (stream_key, canonical_workspace_dir, workspace_identity, owner_token, owner_pid,
              lease_generation, lease_expires_at_epoch_millis, updated_at,
              owner_process_start_identity)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                stream_key,
                lease.workspace_dir,
                lease.workspace_identity,
                lease.owner_token,
                lease.owner_pid,
                lease.generation,
                lease.expires_at_millis,
                Utc::now().to_rfc3339(),
                lease.owner_start_identity,
            ],
        )
        .context("failed to create global Telegram runner binding")?;
    Ok(())
}

fn update_owned_lease(
    transaction: &rusqlite::Transaction<'_>,
    stream_key: &str,
    lease: &LeaseWrite<'_>,
) -> Result<()> {
    let changed = transaction
        .execute(
            "UPDATE telegram_global_runner_bindings
             SET workspace_identity = ?3, owner_token = ?4, owner_pid = ?5,
                 lease_generation = ?6, lease_expires_at_epoch_millis = ?7, updated_at = ?8,
                 owner_process_start_identity = ?9
             WHERE stream_key = ?1 AND canonical_workspace_dir = ?2",
            params![
                stream_key,
                lease.workspace_dir,
                lease.workspace_identity,
                lease.owner_token,
                lease.owner_pid,
                lease.generation,
                lease.expires_at_millis,
                Utc::now().to_rfc3339(),
                lease.owner_start_identity,
            ],
        )
        .context("failed to acquire global Telegram runner binding")?;
    if changed != 1 {
        bail!("global Telegram runner binding changed during acquisition");
    }
    Ok(())
}

fn rebind_inactive_lease(
    transaction: &rusqlite::Transaction<'_>,
    stream_key: &str,
    lease: &LeaseWrite<'_>,
    now_millis: i64,
) -> Result<()> {
    let changed = transaction
        .execute(
            "UPDATE telegram_global_runner_bindings
             SET canonical_workspace_dir = ?2, workspace_identity = ?3, owner_token = ?4,
                 owner_pid = ?5, lease_expires_at_epoch_millis = ?6,
                 lease_generation = ?7, updated_at = ?8,
                 owner_process_start_identity = ?9
             WHERE stream_key = ?1
               AND (owner_token IS NULL OR lease_expires_at_epoch_millis <= ?10)",
            params![
                stream_key,
                lease.workspace_dir,
                lease.workspace_identity,
                lease.owner_token,
                lease.owner_pid,
                lease.expires_at_millis,
                lease.generation,
                Utc::now().to_rfc3339(),
                lease.owner_start_identity,
                now_millis,
            ],
        )
        .context("failed to rebind inactive global Telegram runner stream")?;
    if changed != 1 {
        bail!("global Telegram runner became active while workspace rebinding was requested");
    }
    Ok(())
}

fn validate_inputs(
    workspace_dir: &str,
    stream_key: &str,
    owner_token: &str,
    owner_pid: u32,
) -> Result<()> {
    if workspace_dir.is_empty()
        || workspace_dir.len() > MAX_WORKSPACE_BYTES
        || workspace_dir.chars().any(char::is_control)
    {
        bail!("canonical Telegram workspace binding is empty, too long, or contains control text");
    }
    if stream_key.is_empty()
        || stream_key.len() > MAX_STREAM_KEY_BYTES
        || stream_key.chars().any(char::is_control)
    {
        bail!("Telegram stream key is empty, too long, or contains control text");
    }
    let raw_bot_id = stream_key.strip_prefix("bot-id:").ok_or_else(|| {
        anyhow!("global Telegram stream key must use the canonical bot-id namespace")
    })?;
    let bot_id = raw_bot_id
        .parse::<u64>()
        .context("global Telegram stream key must contain a positive numeric bot id")?;
    if bot_id == 0 || stream_key != format!("bot-id:{bot_id}") {
        bail!("global Telegram stream key must contain a canonical positive numeric bot id");
    }
    if owner_token.is_empty()
        || owner_token.len() > MAX_OWNER_TOKEN_BYTES
        || owner_token.chars().any(char::is_control)
    {
        bail!("Telegram runner owner token is empty, too long, or contains control text");
    }
    if owner_pid == 0 {
        bail!("Telegram runner owner PID must be positive");
    }
    Ok(())
}

fn validate_process_start_identity(identity: Option<&str>) -> Result<()> {
    if identity.is_some_and(|identity| {
        identity.is_empty()
            || identity.len() > MAX_PROCESS_START_IDENTITY_BYTES
            || identity.chars().any(char::is_control)
    }) {
        bail!(
            "Telegram runner process start identity is empty, too long, or contains control text"
        );
    }
    Ok(())
}

fn validate_canonical_workspace(workspace_dir: &str) -> Result<String> {
    let supplied = Path::new(workspace_dir);
    if !supplied.is_absolute() {
        bail!("Telegram workspace binding must be an absolute canonical path");
    }
    let canonical = supplied
        .canonicalize()
        .with_context(|| format!("failed to canonicalize Telegram workspace {workspace_dir}"))?;
    let canonical_text = canonical.display().to_string();
    if canonical_text != workspace_dir {
        bail!(
            "Telegram workspace binding must already be canonical: supplied `{workspace_dir}`, canonical `{canonical_text}`"
        );
    }
    Ok(canonical_text)
}

fn workspace_identity_for_stored_path(workspace_dir: &str) -> String {
    workspace_identity(Path::new(workspace_dir))
        .unwrap_or_else(|_| format!("legacy-path:{workspace_dir}"))
}

#[cfg(unix)]
fn workspace_identity(path: &Path) -> Result<String> {
    use std::os::unix::fs::MetadataExt;

    let metadata = path.metadata().with_context(|| {
        format!(
            "failed to inspect Telegram workspace identity {}",
            path.display()
        )
    })?;
    if !metadata.is_dir() {
        bail!("Telegram workspace identity must reference a directory");
    }
    #[cfg(target_os = "linux")]
    if let Some(identity) = linux_workspace_birth_identity(path)? {
        return Ok(identity);
    }
    #[cfg(target_vendor = "apple")]
    {
        use std::os::darwin::fs::MetadataExt as DarwinMetadataExt;

        return Ok(format!(
            "darwin:{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.st_birthtime(),
            metadata.st_birthtime_nsec()
        ));
    }
    #[allow(unreachable_code)]
    Ok(format!("unix:{}:{}", metadata.dev(), metadata.ino()))
}

#[cfg(target_os = "linux")]
fn linux_workspace_birth_identity(path: &Path) -> Result<Option<String>> {
    use std::os::unix::ffi::OsStrExt;

    let path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .context("Telegram workspace path contains a NUL byte")?;
    let mut statx = std::mem::MaybeUninit::<libc::statx>::zeroed();
    let status = unsafe {
        libc::statx(
            libc::AT_FDCWD,
            path.as_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
            libc::STATX_INO | libc::STATX_BTIME | libc::STATX_MNT_ID,
            statx.as_mut_ptr(),
        )
    };
    if status != 0 {
        let error = std::io::Error::last_os_error();
        if matches!(error.raw_os_error(), Some(code) if code == libc::ENOSYS || code == libc::EOPNOTSUPP)
        {
            return Ok(None);
        }
        return Err(error).context("failed to inspect Telegram workspace birth identity");
    }
    let statx = unsafe { statx.assume_init() };
    if statx.stx_mask & libc::STATX_BTIME == 0 {
        return Ok(None);
    }
    let mount_id = statx.stx_mnt_id;
    let inode = statx.stx_ino;
    let birth_seconds = statx.stx_btime.tv_sec;
    let birth_nanos = statx.stx_btime.tv_nsec;
    Ok(Some(format!(
        "linux:{mount_id}:{inode}:{birth_seconds}:{birth_nanos}"
    )))
}

#[cfg(windows)]
fn workspace_identity(path: &Path) -> Result<String> {
    use std::os::windows::fs::OpenOptionsExt;

    use crate::private_fs::{
        WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
        WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ, WINDOWS_READ_CONTROL,
        validate_windows_path_identity_only, windows_file_identity_key,
    };

    let directory = std::fs::OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .with_context(|| {
            format!(
                "failed to open Telegram workspace identity {}",
                path.display()
            )
        })?;
    validate_windows_path_identity_only(path, &directory, true)?;
    windows_file_identity_key(&directory)
}

#[cfg(not(any(unix, windows)))]
fn workspace_identity(path: &Path) -> Result<String> {
    Ok(format!("path:{}", path.display()))
}

fn lease_deadline(lease_ttl_seconds: u64) -> Result<(i64, i64)> {
    if lease_ttl_seconds == 0 {
        bail!("global Telegram runner lease TTL must be greater than zero");
    }
    let now_millis = Utc::now().timestamp_millis();
    let ttl_millis = i64::try_from(lease_ttl_seconds)
        .context("global Telegram runner lease TTL is too large")?
        .checked_mul(1_000)
        .context("global Telegram runner lease TTL overflowed")?;
    let expires_at_millis = now_millis
        .checked_add(ttl_millis)
        .context("global Telegram runner lease deadline overflowed")?;
    Ok((now_millis, expires_at_millis))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Barrier};

    use super::*;

    struct Fixture {
        root: PathBuf,
        store: PathBuf,
        first_workspace: String,
        second_workspace: String,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let nonce = rand::random::<u64>();
            let root = std::env::temp_dir().join(format!(
                "akra-telegram-global-{label}-{}-{nonce}",
                std::process::id()
            ));
            let first = root.join("workspace-a");
            let second = root.join("workspace-b");
            fs::create_dir_all(&first).expect("first Telegram workspace should create");
            fs::create_dir_all(&second).expect("second Telegram workspace should create");
            let store = root
                .join("akra-home")
                .join(GLOBAL_DIRECTORY)
                .join(TELEGRAM_DIRECTORY)
                .join(RUNTIME_DIRECTORY)
                .join(STORE_FILE_NAME);
            Self {
                root,
                store,
                first_workspace: first
                    .canonicalize()
                    .expect("first workspace should canonicalize")
                    .display()
                    .to_string(),
                second_workspace: second
                    .canonicalize()
                    .expect("second workspace should canonicalize")
                    .display()
                    .to_string(),
            }
        }

        fn adapter(&self) -> SqliteTelegramGlobalRunnerLeaseAdapter {
            SqliteTelegramGlobalRunnerLeaseAdapter::with_store_path(self.store.clone())
        }

        fn expire(&self, stream_key: &str) {
            let adapter = self.adapter();
            let connection = adapter
                .open_connection()
                .expect("global Telegram store should open for test expiry");
            connection
                .execute(
                    "UPDATE telegram_global_runner_bindings
                     SET lease_expires_at_epoch_millis = 0 WHERE stream_key = ?1",
                    [stream_key],
                )
                .expect("global Telegram lease should expire");
        }

        fn replace_owner_start_identity(&self, stream_key: &str, identity: Option<&str>) {
            let adapter = self.adapter();
            let connection = adapter
                .open_connection()
                .expect("global Telegram store should open for identity replacement");
            connection
                .execute(
                    "UPDATE telegram_global_runner_bindings
                     SET owner_process_start_identity = ?2 WHERE stream_key = ?1",
                    params![stream_key, identity],
                )
                .expect("global Telegram owner identity should update");
        }

        fn simulate_crashed_owner(&self, stream_key: &str, owner_pid: u32) {
            let adapter = self.adapter();
            let connection = adapter
                .open_connection()
                .expect("global Telegram store should open for crashed owner simulation");
            connection
                .execute(
                    "UPDATE telegram_global_runner_bindings
                     SET owner_pid = ?2,
                         owner_process_start_identity = 'simulated-crashed-process'
                     WHERE stream_key = ?1",
                    params![stream_key, owner_pid],
                )
                .expect("global Telegram crashed owner should be simulated");
        }

        fn owner_start_identity(&self, stream_key: &str) -> Option<String> {
            self.adapter()
                .open_connection()
                .expect("global Telegram store should open for identity inspection")
                .query_row(
                    "SELECT owner_process_start_identity
                     FROM telegram_global_runner_bindings WHERE stream_key = ?1",
                    [stream_key],
                    |row| row.get(0),
                )
                .expect("global Telegram owner identity should load")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn terminated_process_id() -> u32 {
        #[cfg(unix)]
        let mut child = std::process::Command::new("sh")
            .args(["-c", "exit 0"])
            .spawn()
            .expect("short-lived Unix child should spawn");
        #[cfg(windows)]
        let mut child = std::process::Command::new("cmd")
            .args(["/C", "exit", "0"])
            .spawn()
            .expect("short-lived Windows child should spawn");
        #[cfg(not(any(unix, windows)))]
        let mut child = std::process::Command::new("false")
            .spawn()
            .expect("short-lived child should spawn");
        let pid = child.id();
        child.wait().expect("short-lived child should exit");
        pid
    }

    #[test]
    fn global_telegram_runner_blocks_the_same_bot_in_another_workspace() {
        let fixture = Fixture::new("cross-workspace-active");
        let first = fixture.adapter();
        let second = fixture.adapter();

        assert_eq!(
            first
                .try_acquire_global_runner_lease(
                    &fixture.first_workspace,
                    "bot-id:123456",
                    "owner-first",
                    std::process::id(),
                    300,
                    TelegramWorkspaceBindingPolicy::Preserve,
                )
                .expect("first global Telegram runner should acquire"),
            TelegramGlobalRunnerLeaseClaimDecision::Acquired { generation: 1 }
        );
        assert_eq!(
            second
                .try_acquire_global_runner_lease(
                    &fixture.second_workspace,
                    "bot-id:123456",
                    "owner-second",
                    std::process::id(),
                    300,
                    TelegramWorkspaceBindingPolicy::RebindInactive,
                )
                .expect("second global Telegram runner should be inspected"),
            TelegramGlobalRunnerLeaseClaimDecision::ActiveRunner {
                bound_workspace_dir: fixture.first_workspace.clone(),
            }
        );
        assert!(
            second
                .renew_global_runner_lease(
                    &fixture.second_workspace,
                    "bot-id:123456",
                    "owner-second",
                    std::process::id(),
                    1,
                    300,
                )
                .expect_err("blocked workspace must not renew the bot stream")
                .to_string()
                .contains("no longer owned")
        );
    }

    #[test]
    fn concurrent_first_open_initializes_one_global_store_without_split_brain() {
        let fixture = Fixture::new("concurrent-first-open");
        let barrier = Arc::new(Barrier::new(3));
        let candidates = [
            (
                fixture.first_workspace.clone(),
                "bot-id:123457",
                "owner-first",
            ),
            (
                fixture.second_workspace.clone(),
                "bot-id:123458",
                "owner-second",
            ),
        ];
        let handles = candidates.map(|(workspace, stream_key, owner)| {
            let adapter = fixture.adapter();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                adapter.try_acquire_global_runner_lease(
                    &workspace,
                    stream_key,
                    owner,
                    std::process::id(),
                    300,
                    TelegramWorkspaceBindingPolicy::Preserve,
                )
            })
        });
        barrier.wait();
        for handle in handles {
            assert_eq!(
                handle
                    .join()
                    .expect("first-open thread should finish")
                    .expect("concurrent first open should succeed"),
                TelegramGlobalRunnerLeaseClaimDecision::Acquired { generation: 1 }
            );
        }
        let connection = fixture
            .adapter()
            .open_connection()
            .expect("initialized global store should reopen");
        let binding_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM telegram_global_runner_bindings",
                [],
                |row| row.get(0),
            )
            .expect("global binding count should load");
        assert_eq!(binding_count, 2);
    }

    #[test]
    fn global_telegram_binding_survives_release_and_requires_explicit_rebind() {
        let fixture = Fixture::new("persistent-binding");
        let adapter = fixture.adapter();
        let stream_key = "bot-id:654321";
        adapter
            .try_acquire_global_runner_lease(
                &fixture.first_workspace,
                stream_key,
                "owner-first",
                std::process::id(),
                300,
                TelegramWorkspaceBindingPolicy::Preserve,
            )
            .expect("first binding should acquire");
        assert!(
            adapter
                .release_global_runner_lease(
                    &fixture.first_workspace,
                    stream_key,
                    "owner-first",
                    std::process::id(),
                    1,
                )
                .expect("owned global lease should release")
        );
        assert_eq!(
            fixture
                .adapter()
                .try_acquire_global_runner_lease(
                    &fixture.second_workspace,
                    stream_key,
                    "owner-second",
                    std::process::id(),
                    300,
                    TelegramWorkspaceBindingPolicy::Preserve,
                )
                .expect("preserved binding should be inspected"),
            TelegramGlobalRunnerLeaseClaimDecision::WorkspaceBindingMismatch {
                bound_workspace_dir: fixture.first_workspace.clone(),
            }
        );
        assert_eq!(
            fixture
                .adapter()
                .try_acquire_global_runner_lease(
                    &fixture.second_workspace,
                    stream_key,
                    "owner-second",
                    std::process::id(),
                    300,
                    TelegramWorkspaceBindingPolicy::RebindInactive,
                )
                .expect("explicit inactive rebind should acquire"),
            TelegramGlobalRunnerLeaseClaimDecision::Acquired { generation: 2 }
        );
        assert!(
            adapter
                .renew_global_runner_lease(
                    &fixture.first_workspace,
                    stream_key,
                    "owner-first",
                    std::process::id(),
                    1,
                    300,
                )
                .expect_err("old workspace must not renew after rebind")
                .to_string()
                .contains("no longer owned")
        );
    }

    #[test]
    fn global_telegram_runner_recovers_an_expired_crash_without_rebinding() {
        let fixture = Fixture::new("stale-recovery");
        let stream_key = "bot-id:777001";
        let dead_pid = terminated_process_id();
        fixture
            .adapter()
            .try_acquire_global_runner_lease(
                &fixture.first_workspace,
                stream_key,
                "crashed-owner",
                std::process::id(),
                300,
                TelegramWorkspaceBindingPolicy::Preserve,
            )
            .expect("owner should acquire before the simulated crash");
        fixture.simulate_crashed_owner(stream_key, dead_pid);
        assert_eq!(
            fixture
                .adapter()
                .try_acquire_global_runner_lease(
                    &fixture.first_workspace,
                    stream_key,
                    "premature-owner",
                    std::process::id(),
                    300,
                    TelegramWorkspaceBindingPolicy::Preserve,
                )
                .expect("dead PID must remain quarantined until its request deadline"),
            TelegramGlobalRunnerLeaseClaimDecision::ActiveRunner {
                bound_workspace_dir: fixture.first_workspace.clone(),
            }
        );
        fixture.expire(stream_key);

        assert_eq!(
            fixture
                .adapter()
                .try_acquire_global_runner_lease(
                    &fixture.first_workspace,
                    stream_key,
                    "recovered-owner",
                    std::process::id(),
                    300,
                    TelegramWorkspaceBindingPolicy::Preserve,
                )
                .expect("same workspace should recover an expired lease"),
            TelegramGlobalRunnerLeaseClaimDecision::Acquired { generation: 2 }
        );
        assert!(
            fixture
                .adapter()
                .release_global_runner_lease(
                    &fixture.first_workspace,
                    stream_key,
                    "crashed-owner",
                    dead_pid,
                    1,
                )
                .expect_err("terminated PID cannot establish release ownership")
                .to_string()
                .contains("process birth identity")
        );
    }

    #[test]
    fn global_telegram_crash_recovery_acknowledges_an_ambiguous_update_exactly_once() {
        let fixture = Fixture::new("crashed-update-recovery");
        let adapter = fixture.adapter();
        let stream_key = "bot-id:777008";
        let crashed_pid = terminated_process_id();
        assert_eq!(
            adapter
                .try_acquire_global_runner_lease(
                    &fixture.first_workspace,
                    stream_key,
                    "crashed-owner",
                    std::process::id(),
                    300,
                    TelegramWorkspaceBindingPolicy::Preserve,
                )
                .expect("owner should acquire before the simulated failure"),
            TelegramGlobalRunnerLeaseClaimDecision::Acquired { generation: 1 }
        );
        assert_eq!(
            adapter
                .claim_update(&fixture.first_workspace, stream_key, "crashed-owner", 77)
                .expect("ambiguous update should be durably claimed before its side effect"),
            TelegramUpdateClaimDecision::Execute
        );
        fixture.simulate_crashed_owner(stream_key, crashed_pid);
        fixture.expire(stream_key);
        assert_eq!(
            adapter
                .try_acquire_global_runner_lease(
                    &fixture.first_workspace,
                    stream_key,
                    "replacement-owner",
                    std::process::id(),
                    300,
                    TelegramWorkspaceBindingPolicy::Preserve,
                )
                .expect("replacement owner should recover the expired dead process"),
            TelegramGlobalRunnerLeaseClaimDecision::Acquired { generation: 2 }
        );
        assert_eq!(
            adapter
                .load_cursor_and_recover_inflight(
                    &fixture.first_workspace,
                    stream_key,
                    "replacement-owner",
                )
                .expect("ambiguous update should advance the at-most-once cursor"),
            Some(78)
        );
        assert_eq!(
            adapter
                .claim_update(
                    &fixture.first_workspace,
                    stream_key,
                    "replacement-owner",
                    77,
                )
                .expect("recovered update should remain durably fenced"),
            TelegramUpdateClaimDecision::AlreadyCompleted
        );
    }

    #[test]
    fn expired_global_telegram_lease_cannot_replace_a_live_process() {
        let fixture = Fixture::new("live-owner-after-deadline");
        let stream_key = "bot-id:777003";
        let adapter = fixture.adapter();
        assert_eq!(
            adapter
                .try_acquire_global_runner_lease(
                    &fixture.first_workspace,
                    stream_key,
                    "live-owner",
                    std::process::id(),
                    300,
                    TelegramWorkspaceBindingPolicy::Preserve,
                )
                .expect("live owner should acquire"),
            TelegramGlobalRunnerLeaseClaimDecision::Acquired { generation: 1 }
        );
        fixture.expire(stream_key);

        assert_eq!(
            adapter
                .try_acquire_global_runner_lease(
                    &fixture.first_workspace,
                    stream_key,
                    "replacement-owner",
                    std::process::id(),
                    300,
                    TelegramWorkspaceBindingPolicy::Preserve,
                )
                .expect("expired live owner should still fence replacement"),
            TelegramGlobalRunnerLeaseClaimDecision::ActiveRunner {
                bound_workspace_dir: fixture.first_workspace.clone(),
            }
        );
        adapter
            .renew_global_runner_lease(
                &fixture.first_workspace,
                stream_key,
                "live-owner",
                std::process::id(),
                1,
                300,
            )
            .expect("the same live generation may renew after a long operation");
    }

    #[test]
    fn expired_global_telegram_lease_allows_takeover_after_pid_reuse() {
        let fixture = Fixture::new("pid-reuse-after-deadline");
        let stream_key = "bot-id:777009";
        let adapter = fixture.adapter();
        assert_eq!(
            adapter
                .try_acquire_global_runner_lease(
                    &fixture.first_workspace,
                    stream_key,
                    "original-owner",
                    std::process::id(),
                    300,
                    TelegramWorkspaceBindingPolicy::Preserve,
                )
                .expect("original process should acquire"),
            TelegramGlobalRunnerLeaseClaimDecision::Acquired { generation: 1 }
        );
        fixture.expire(stream_key);
        fixture.replace_owner_start_identity(
            stream_key,
            Some("simulated-previous-process-start-identity"),
        );

        assert_eq!(
            adapter
                .try_acquire_global_runner_lease(
                    &fixture.first_workspace,
                    stream_key,
                    "replacement-owner",
                    std::process::id(),
                    300,
                    TelegramWorkspaceBindingPolicy::Preserve,
                )
                .expect("reused PID with a different birth identity must not fence takeover"),
            TelegramGlobalRunnerLeaseClaimDecision::Acquired { generation: 2 }
        );
        assert!(
            !adapter
                .release_global_runner_lease(
                    &fixture.first_workspace,
                    stream_key,
                    "original-owner",
                    std::process::id(),
                    1,
                )
                .expect("stale PID owner release should remain a fenced CAS no-op")
        );
    }

    #[test]
    fn legacy_active_lease_backfills_process_identity_on_renewal() {
        let fixture = Fixture::new("legacy-process-identity-backfill");
        let stream_key = "bot-id:777010";
        let adapter = fixture.adapter();
        adapter
            .try_acquire_global_runner_lease(
                &fixture.first_workspace,
                stream_key,
                "legacy-owner",
                std::process::id(),
                300,
                TelegramWorkspaceBindingPolicy::Preserve,
            )
            .expect("legacy fixture owner should acquire");
        fixture.replace_owner_start_identity(stream_key, None);

        adapter
            .renew_global_runner_lease(
                &fixture.first_workspace,
                stream_key,
                "legacy-owner",
                std::process::id(),
                1,
                300,
            )
            .expect("legacy active owner should backfill identity while renewing");

        assert_eq!(
            fixture.owner_start_identity(stream_key),
            Some(
                required_process_start_identity(std::process::id())
                    .expect("current process should expose a birth identity")
            )
        );
    }

    #[test]
    fn global_update_ledger_survives_cross_workspace_rebind() {
        let fixture = Fixture::new("cross-workspace-ledger");
        let adapter = fixture.adapter();
        let stream_key = "bot-id:777004";
        assert_eq!(
            adapter
                .try_acquire_global_runner_lease(
                    &fixture.first_workspace,
                    stream_key,
                    "owner-first",
                    std::process::id(),
                    300,
                    TelegramWorkspaceBindingPolicy::Preserve,
                )
                .expect("first workspace should acquire"),
            TelegramGlobalRunnerLeaseClaimDecision::Acquired { generation: 1 }
        );
        assert_eq!(
            adapter
                .claim_update(&fixture.first_workspace, stream_key, "owner-first", 77)
                .expect("first workspace should claim update"),
            TelegramUpdateClaimDecision::Execute
        );
        assert_eq!(
            adapter
                .complete_update(&fixture.first_workspace, stream_key, "owner-first", 77)
                .expect("first workspace should complete update"),
            Some(78)
        );
        adapter
            .release_global_runner_lease(
                &fixture.first_workspace,
                stream_key,
                "owner-first",
                std::process::id(),
                1,
            )
            .expect("first workspace should release");
        assert_eq!(
            adapter
                .try_acquire_global_runner_lease(
                    &fixture.second_workspace,
                    stream_key,
                    "owner-second",
                    std::process::id(),
                    300,
                    TelegramWorkspaceBindingPolicy::RebindInactive,
                )
                .expect("second workspace should rebind"),
            TelegramGlobalRunnerLeaseClaimDecision::Acquired { generation: 2 }
        );

        assert_eq!(
            adapter
                .load_cursor_and_recover_inflight(
                    &fixture.second_workspace,
                    stream_key,
                    "owner-second",
                )
                .expect("global cursor should follow the bot"),
            Some(78)
        );
        assert_eq!(
            adapter
                .claim_update(&fixture.second_workspace, stream_key, "owner-second", 77)
                .expect("completed update should remain globally fenced"),
            TelegramUpdateClaimDecision::AlreadyCompleted
        );
    }

    #[test]
    fn repo_local_ledger_migrates_once_into_the_global_bot_stream() {
        let fixture = Fixture::new("legacy-ledger-migration");
        let stream_key = "bot-id:777006";
        let legacy = SqlitePlanningAuthorityAdapter::new();
        assert_eq!(
            legacy
                .try_acquire_runner_lease(
                    &fixture.first_workspace,
                    stream_key,
                    "legacy-owner",
                    300,
                )
                .expect("legacy workspace lease should acquire"),
            TelegramRunnerLeaseClaimDecision::Acquired
        );
        assert_eq!(
            legacy
                .claim_update(&fixture.first_workspace, stream_key, "legacy-owner", 91,)
                .expect("legacy update should claim"),
            TelegramUpdateClaimDecision::Execute
        );
        assert_eq!(
            legacy
                .complete_update(&fixture.first_workspace, stream_key, "legacy-owner", 91,)
                .expect("legacy update should complete"),
            Some(92)
        );
        legacy
            .release_runner_lease(&fixture.first_workspace, stream_key, "legacy-owner")
            .expect("legacy workspace lease should release");

        let global = fixture.adapter();
        assert_eq!(
            global
                .try_acquire_global_runner_lease(
                    &fixture.first_workspace,
                    stream_key,
                    "global-owner",
                    std::process::id(),
                    300,
                    TelegramWorkspaceBindingPolicy::Preserve,
                )
                .expect("global stream should acquire and migrate"),
            TelegramGlobalRunnerLeaseClaimDecision::Acquired { generation: 1 }
        );
        assert_eq!(
            global
                .load_cursor_and_recover_inflight(
                    &fixture.first_workspace,
                    stream_key,
                    "global-owner",
                )
                .expect("migrated global cursor should load"),
            Some(92)
        );
        assert_eq!(
            global
                .claim_update(&fixture.first_workspace, stream_key, "global-owner", 91,)
                .expect("migrated update must not execute twice"),
            TelegramUpdateClaimDecision::AlreadyCompleted
        );
        global
            .try_acquire_global_runner_lease(
                &fixture.first_workspace,
                stream_key,
                "global-owner",
                std::process::id(),
                300,
                TelegramWorkspaceBindingPolicy::Preserve,
            )
            .expect("same owner reacquisition should remain idempotent");
        let connection = global
            .open_connection()
            .expect("global migration store should reopen");
        let migration_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM telegram_global_ledger_migrations WHERE stream_key = ?1",
                [stream_key],
                |row| row.get(0),
            )
            .expect("migration count should load");
        assert_eq!(migration_count, 1);
    }

    #[test]
    fn recreated_workspace_path_requires_explicit_rebind() {
        let fixture = Fixture::new("workspace-identity");
        let adapter = fixture.adapter();
        let stream_key = "bot-id:777005";
        adapter
            .try_acquire_global_runner_lease(
                &fixture.first_workspace,
                stream_key,
                "owner-first",
                std::process::id(),
                300,
                TelegramWorkspaceBindingPolicy::Preserve,
            )
            .expect("first workspace should bind");
        adapter
            .release_global_runner_lease(
                &fixture.first_workspace,
                stream_key,
                "owner-first",
                std::process::id(),
                1,
            )
            .expect("first workspace should release");
        fs::remove_dir(&fixture.first_workspace).expect("old workspace should remove");
        fs::create_dir(&fixture.first_workspace).expect("replacement workspace should create");

        assert!(matches!(
            adapter
                .try_acquire_global_runner_lease(
                    &fixture.first_workspace,
                    stream_key,
                    "owner-second",
                    std::process::id(),
                    300,
                    TelegramWorkspaceBindingPolicy::Preserve,
                )
                .expect("replacement workspace should be inspected"),
            TelegramGlobalRunnerLeaseClaimDecision::WorkspaceBindingMismatch { .. }
        ));
        assert_eq!(
            adapter
                .try_acquire_global_runner_lease(
                    &fixture.first_workspace,
                    stream_key,
                    "owner-second",
                    std::process::id(),
                    300,
                    TelegramWorkspaceBindingPolicy::RebindInactive,
                )
                .expect("explicit replacement rebind should acquire"),
            TelegramGlobalRunnerLeaseClaimDecision::Acquired { generation: 2 }
        );
    }

    #[test]
    fn concurrent_explicit_rebind_allows_exactly_one_new_workspace_owner() {
        let fixture = Fixture::new("concurrent-rebind");
        let stream_key = "bot-id:777002";
        let seed = fixture.adapter();
        seed.try_acquire_global_runner_lease(
            &fixture.first_workspace,
            stream_key,
            "seed-owner",
            std::process::id(),
            300,
            TelegramWorkspaceBindingPolicy::Preserve,
        )
        .expect("seed binding should acquire");
        seed.release_global_runner_lease(
            &fixture.first_workspace,
            stream_key,
            "seed-owner",
            std::process::id(),
            1,
        )
        .expect("seed binding should become inactive");

        let third_workspace_path = fixture.root.join("workspace-c");
        fs::create_dir(&third_workspace_path).expect("third Telegram workspace should create");
        let third_workspace = third_workspace_path
            .canonicalize()
            .expect("third Telegram workspace should canonicalize")
            .display()
            .to_string();
        let barrier = Arc::new(Barrier::new(3));
        let candidates = [
            (fixture.second_workspace.clone(), "owner-second"),
            (third_workspace, "owner-third"),
        ];
        let handles = candidates.map(|(workspace, owner)| {
            let barrier = barrier.clone();
            let adapter = fixture.adapter();
            std::thread::spawn(move || {
                barrier.wait();
                let decision = adapter
                    .try_acquire_global_runner_lease(
                        &workspace,
                        stream_key,
                        owner,
                        std::process::id(),
                        300,
                        TelegramWorkspaceBindingPolicy::RebindInactive,
                    )
                    .expect("concurrent rebind attempt should return a decision");
                (workspace, decision)
            })
        });
        barrier.wait();
        let results = handles.map(|handle| handle.join().expect("rebind thread should finish"));
        let acquired = results
            .iter()
            .filter(|(_, decision)| {
                matches!(
                    decision,
                    TelegramGlobalRunnerLeaseClaimDecision::Acquired { .. }
                )
            })
            .count();
        assert_eq!(acquired, 1, "only one inactive binding rebind may win");
        let winner = results
            .iter()
            .find_map(|(workspace, decision)| {
                matches!(
                    decision,
                    TelegramGlobalRunnerLeaseClaimDecision::Acquired { .. }
                )
                .then_some(workspace.as_str())
            })
            .expect("one rebind winner should exist");
        let loser_binding = results.iter().find_map(|(_, decision)| match decision {
            TelegramGlobalRunnerLeaseClaimDecision::ActiveRunner {
                bound_workspace_dir,
            } => Some(bound_workspace_dir.as_str()),
            _ => None,
        });
        assert_eq!(loser_binding, Some(winner));
    }

    #[test]
    fn global_telegram_runner_rejects_noncanonical_workspace_bindings() {
        let fixture = Fixture::new("canonical-workspace");
        let noncanonical = Path::new(&fixture.first_workspace).join(".");
        let error = fixture
            .adapter()
            .try_acquire_global_runner_lease(
                &noncanonical.display().to_string(),
                "bot-id:880001",
                "owner-first",
                std::process::id(),
                300,
                TelegramWorkspaceBindingPolicy::Preserve,
            )
            .expect_err("noncanonical workspace text must fail closed");
        assert!(error.to_string().contains("already be canonical"));
    }

    #[test]
    fn global_telegram_store_rejects_relative_data_roots() {
        let fixture = Fixture::new("relative-data-root");
        let adapter = SqliteTelegramGlobalRunnerLeaseAdapter::with_store_path(PathBuf::from(
            "relative-akra/global/telegram/runtime/runner-leases.db",
        ));
        let error = adapter
            .try_acquire_global_runner_lease(
                &fixture.first_workspace,
                "bot-id:880002",
                "owner-first",
                std::process::id(),
                300,
                TelegramWorkspaceBindingPolicy::Preserve,
            )
            .expect_err("a relative global data root must fail closed");
        assert!(error.to_string().contains("absolute Akra data root"));
    }

    #[cfg(unix)]
    #[test]
    fn global_telegram_v1_schema_migrates_transactionally_to_v3() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = Fixture::new("v1-schema-migration");
        fs::create_dir_all(fixture.store.parent().expect("store parent should exist"))
            .expect("legacy store parent should create");
        let connection = Connection::open(&fixture.store).expect("legacy store should create");
        connection
            .execute_batch(&format!(
                "PRAGMA application_id = {GLOBAL_TELEGRAM_STORE_APPLICATION_ID};
                 PRAGMA user_version = {LEGACY_GLOBAL_TELEGRAM_STORE_SCHEMA_VERSION};
                 CREATE TABLE telegram_global_runner_bindings (
                     stream_key TEXT PRIMARY KEY,
                     canonical_workspace_dir TEXT NOT NULL,
                     owner_token TEXT,
                     lease_expires_at_epoch_millis INTEGER,
                     updated_at TEXT NOT NULL
                 );"
            ))
            .expect("legacy schema should create");
        connection
            .execute(
                "INSERT INTO telegram_global_runner_bindings
                 (stream_key, canonical_workspace_dir, owner_token,
                  lease_expires_at_epoch_millis, updated_at)
                 VALUES (?1, ?2, NULL, NULL, ?3)",
                params![
                    "bot-id:880003",
                    fixture.first_workspace,
                    Utc::now().to_rfc3339()
                ],
            )
            .expect("legacy binding should seed");
        drop(connection);
        fs::set_permissions(&fixture.store, fs::Permissions::from_mode(0o600))
            .expect("legacy store should become private");

        assert_eq!(
            fixture
                .adapter()
                .try_acquire_global_runner_lease(
                    &fixture.first_workspace,
                    "bot-id:880003",
                    "owner-first",
                    std::process::id(),
                    300,
                    TelegramWorkspaceBindingPolicy::Preserve,
                )
                .expect("legacy store should migrate and acquire"),
            TelegramGlobalRunnerLeaseClaimDecision::Acquired { generation: 1 }
        );
        let connection = fixture
            .adapter()
            .open_connection()
            .expect("migrated store should reopen");
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("schema version should load");
        assert_eq!(version, GLOBAL_TELEGRAM_STORE_SCHEMA_VERSION);
        validate_global_telegram_schema(&connection)
            .expect("migrated store should satisfy the structural contract");
    }

    #[test]
    fn global_telegram_v2_schema_adds_nullable_process_start_identity() {
        let mut connection = Connection::open_in_memory().expect("v2 fixture store should open");
        connection
            .execute_batch(&format!(
                "PRAGMA user_version = {PREVIOUS_GLOBAL_TELEGRAM_STORE_SCHEMA_VERSION};
                 CREATE TABLE telegram_global_runner_bindings (
                     stream_key TEXT PRIMARY KEY,
                     canonical_workspace_dir TEXT NOT NULL,
                     workspace_identity TEXT NOT NULL,
                     owner_token TEXT,
                     owner_pid INTEGER,
                     lease_generation INTEGER NOT NULL DEFAULT 0,
                     lease_expires_at_epoch_millis INTEGER,
                     updated_at TEXT NOT NULL
                 );
                 INSERT INTO telegram_global_runner_bindings
                     (stream_key, canonical_workspace_dir, workspace_identity, owner_token,
                      owner_pid, lease_generation, lease_expires_at_epoch_millis, updated_at)
                 VALUES ('bot-id:880004', '/legacy', 'legacy-id', 'legacy-owner', 42, 7, 0, 'now');"
            ))
            .expect("v2 fixture schema should create");

        migrate_global_telegram_schema_v2(&mut connection)
            .expect("v2 schema should migrate transactionally");

        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("migrated schema version should load");
        let identity: Option<String> = connection
            .query_row(
                "SELECT owner_process_start_identity
                 FROM telegram_global_runner_bindings WHERE stream_key = 'bot-id:880004'",
                [],
                |row| row.get(0),
            )
            .expect("migrated legacy identity should load");
        assert_eq!(version, GLOBAL_TELEGRAM_STORE_SCHEMA_VERSION);
        assert_eq!(identity, None);
    }

    #[cfg(unix)]
    #[test]
    fn global_telegram_store_rejects_matching_version_with_malformed_schema() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = Fixture::new("malformed-v2-schema");
        fs::create_dir_all(fixture.store.parent().expect("store parent should exist"))
            .expect("malformed store parent should create");
        let connection = Connection::open(&fixture.store).expect("malformed store should create");
        connection
            .execute_batch(&format!(
                "PRAGMA application_id = {GLOBAL_TELEGRAM_STORE_APPLICATION_ID};
                 PRAGMA user_version = {GLOBAL_TELEGRAM_STORE_SCHEMA_VERSION};
                 CREATE TABLE telegram_global_runner_bindings (stream_key TEXT PRIMARY KEY);"
            ))
            .expect("malformed schema should seed");
        drop(connection);
        fs::set_permissions(&fixture.store, fs::Permissions::from_mode(0o600))
            .expect("malformed store should become private");

        let error = fixture
            .adapter()
            .open_connection()
            .expect_err("malformed matching-version schema must fail closed");
        assert!(format!("{error:#}").contains("unexpected column layout"));
    }

    #[cfg(unix)]
    #[test]
    fn global_telegram_store_is_private_and_rejects_hardlinks() {
        use std::os::unix::fs::MetadataExt;

        let fixture = Fixture::new("private-store");
        fixture
            .adapter()
            .try_acquire_global_runner_lease(
                &fixture.first_workspace,
                "bot-id:990001",
                "owner-first",
                std::process::id(),
                300,
                TelegramWorkspaceBindingPolicy::Preserve,
            )
            .expect("private global store should initialize");
        let metadata = fs::metadata(&fixture.store).expect("global store metadata should load");
        assert_eq!(metadata.mode() & 0o777, 0o600);
        assert_eq!(metadata.nlink(), 1);
        let second_link = fixture.root.join("runner-leases-hardlink.db");
        fs::hard_link(&fixture.store, second_link).expect("hardlink fixture should create");
        let error = fixture
            .adapter()
            .renew_global_runner_lease(
                &fixture.first_workspace,
                "bot-id:990001",
                "owner-first",
                std::process::id(),
                1,
                300,
            )
            .expect_err("hardlinked global store must fail closed");
        assert!(format!("{error:#}").contains("one link"));
    }

    #[cfg(windows)]
    #[test]
    fn global_telegram_store_rejects_hardlinks_on_windows() {
        let fixture = Fixture::new("windows-hardlink-store");
        fixture
            .adapter()
            .try_acquire_global_runner_lease(
                &fixture.first_workspace,
                "bot-id:990002",
                "owner-first",
                std::process::id(),
                300,
                TelegramWorkspaceBindingPolicy::Preserve,
            )
            .expect("private Windows global store should initialize");
        let second_link = fixture.root.join("runner-leases-hardlink.db");
        fs::hard_link(&fixture.store, second_link).expect("Windows hardlink fixture should create");
        let error = fixture
            .adapter()
            .renew_global_runner_lease(
                &fixture.first_workspace,
                "bot-id:990002",
                "owner-first",
                std::process::id(),
                1,
                300,
            )
            .expect_err("hardlinked Windows global store must fail closed");
        assert!(format!("{error:#}").contains("single-link"));
    }
}
