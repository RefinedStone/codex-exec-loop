use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use crate::application::port::outbound::telegram_update_ledger_port::{
    TelegramRunnerLeaseClaimDecision, TelegramUpdateClaimDecision, TelegramUpdateLedgerPort,
};

use super::store::upsert_authority_metadata;
use super::{SqlitePlanningAuthorityAdapter, open_authority_connection};

const COMPLETED_UPDATE_RETENTION_OFFSET: i64 = 511;
const MAX_STREAM_KEY_BYTES: usize = 128;
const MAX_OWNER_TOKEN_BYTES: usize = 128;

impl TelegramUpdateLedgerPort for SqlitePlanningAuthorityAdapter {
    fn try_acquire_runner_lease(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        lease_ttl_seconds: u64,
    ) -> Result<TelegramRunnerLeaseClaimDecision> {
        validate_stream_key(stream_key)?;
        validate_owner_token(owner_token)?;
        let (now_millis, expires_at_millis) = lease_deadline(lease_ttl_seconds)?;
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open Telegram runner lease transaction")?;
        let updated_at = Utc::now().to_rfc3339();
        let changed = transaction
            .execute(
                "INSERT INTO telegram_update_runner_leases
                 (stream_key, owner_token, lease_expires_at_epoch_millis, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(stream_key) DO UPDATE SET
                     owner_token = excluded.owner_token,
                     lease_expires_at_epoch_millis = excluded.lease_expires_at_epoch_millis,
                     updated_at = excluded.updated_at
                 WHERE telegram_update_runner_leases.owner_token = excluded.owner_token
                    OR telegram_update_runner_leases.lease_expires_at_epoch_millis <= ?5",
                params![
                    stream_key,
                    owner_token,
                    expires_at_millis,
                    updated_at,
                    now_millis
                ],
            )
            .context("failed to acquire Telegram runner lease")?;
        if changed > 0 {
            upsert_authority_metadata(
                &transaction,
                &location,
                "last_telegram_runner_lease_acquired_at",
            )?;
        }
        transaction
            .commit()
            .context("failed to commit Telegram runner lease transaction")?;
        Ok(if changed > 0 {
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
        validate_stream_key(stream_key)?;
        validate_owner_token(owner_token)?;
        let (now_millis, expires_at_millis) = lease_deadline(lease_ttl_seconds)?;
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open Telegram runner lease renewal transaction")?;
        let changed = transaction
            .execute(
                "UPDATE telegram_update_runner_leases
                 SET lease_expires_at_epoch_millis = ?4, updated_at = ?5
                 WHERE stream_key = ?1 AND owner_token = ?2
                   AND lease_expires_at_epoch_millis > ?3",
                params![
                    stream_key,
                    owner_token,
                    now_millis,
                    expires_at_millis,
                    Utc::now().to_rfc3339()
                ],
            )
            .context("failed to renew Telegram runner lease")?;
        if changed == 0 {
            bail!("Telegram runner lease is no longer owned or has expired");
        }
        transaction
            .commit()
            .context("failed to commit Telegram runner lease renewal")?;
        Ok(())
    }

    fn release_runner_lease(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
    ) -> Result<bool> {
        validate_stream_key(stream_key)?;
        validate_owner_token(owner_token)?;
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open Telegram runner lease release transaction")?;
        let changed = transaction
            .execute(
                "DELETE FROM telegram_update_runner_leases
                 WHERE stream_key = ?1 AND owner_token = ?2",
                params![stream_key, owner_token],
            )
            .context("failed to release Telegram runner lease")?;
        transaction
            .commit()
            .context("failed to commit Telegram runner lease release")?;
        Ok(changed > 0)
    }

    fn load_cursor_and_recover_inflight(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
    ) -> Result<Option<i64>> {
        validate_stream_key(stream_key)?;
        validate_owner_token(owner_token)?;
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open Telegram update recovery transaction")?;
        assert_active_lease(&transaction, stream_key, owner_token)?;

        let interrupted_update_id = transaction
            .query_row(
                "SELECT MAX(update_id)
                 FROM telegram_update_inbox
                 WHERE stream_key = ?1 AND update_state = 'executing'",
                [stream_key],
                |row| row.get::<_, Option<i64>>(0),
            )
            .context("failed to inspect interrupted Telegram updates")?;

        if let Some(update_id) = interrupted_update_id {
            let next_offset = checked_next_offset(update_id)?;
            let completed_at = Utc::now().to_rfc3339();
            transaction
                .execute(
                    "UPDATE telegram_update_inbox
                     SET update_state = 'completed', completed_at = ?2
                     WHERE stream_key = ?1 AND update_state = 'executing'",
                    params![stream_key, completed_at],
                )
                .context("failed to finalize interrupted Telegram updates")?;
            persist_cursor(&transaction, stream_key, next_offset, &completed_at)?;
            upsert_authority_metadata(
                &transaction,
                &location,
                "last_telegram_update_recovered_at",
            )?;
            prune_completed_updates(&transaction, stream_key)?;
        }

        let next_offset = load_cursor(&transaction, stream_key)?;
        transaction
            .commit()
            .context("failed to commit Telegram update recovery transaction")?;
        Ok(next_offset)
    }

    fn advance_cursor(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        next_offset: Option<i64>,
    ) -> Result<Option<i64>> {
        validate_stream_key(stream_key)?;
        validate_owner_token(owner_token)?;
        if next_offset.is_some_and(|offset| offset < 0) {
            bail!("Telegram update cursor must be non-negative");
        }

        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open Telegram cursor transaction")?;
        assert_active_lease(&transaction, stream_key, owner_token)?;
        if let Some(next_offset) = next_offset {
            let updated_at = Utc::now().to_rfc3339();
            persist_cursor(&transaction, stream_key, next_offset, &updated_at)?;
            upsert_authority_metadata(&transaction, &location, "last_telegram_cursor_updated_at")?;
        }
        let durable_offset = load_cursor(&transaction, stream_key)?;
        transaction
            .commit()
            .context("failed to commit Telegram cursor transaction")?;
        Ok(durable_offset)
    }

    fn claim_update(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        update_id: i64,
    ) -> Result<TelegramUpdateClaimDecision> {
        validate_stream_key(stream_key)?;
        validate_owner_token(owner_token)?;
        let _ = checked_next_offset(update_id)?;
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open Telegram update claim transaction")?;
        assert_active_lease(&transaction, stream_key, owner_token)?;

        if load_cursor(&transaction, stream_key)?.is_some_and(|offset| update_id < offset) {
            transaction
                .commit()
                .context("failed to commit completed Telegram update lookup")?;
            return Ok(TelegramUpdateClaimDecision::AlreadyCompleted);
        }

        let existing_state = transaction
            .query_row(
                "SELECT update_state
                 FROM telegram_update_inbox
                 WHERE stream_key = ?1 AND update_id = ?2",
                params![stream_key, update_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .context("failed to inspect Telegram update claim")?;
        if let Some(existing_state) = existing_state {
            let decision = match existing_state.as_str() {
                "completed" => TelegramUpdateClaimDecision::AlreadyCompleted,
                "executing" => TelegramUpdateClaimDecision::InFlight,
                unknown => {
                    return Err(anyhow!(
                        "Telegram update inbox contains unsupported state `{unknown}`"
                    ));
                }
            };
            transaction
                .commit()
                .context("failed to commit Telegram update claim lookup")?;
            return Ok(decision);
        }

        let received_at = Utc::now().to_rfc3339();
        transaction
            .execute(
                "INSERT INTO telegram_update_inbox
                 (stream_key, update_id, update_state, received_at, completed_at)
                 VALUES (?1, ?2, 'executing', ?3, NULL)",
                params![stream_key, update_id, received_at],
            )
            .context("failed to persist Telegram update before execution")?;
        upsert_authority_metadata(&transaction, &location, "last_telegram_update_claimed_at")?;
        transaction
            .commit()
            .context("failed to commit Telegram update claim")?;
        Ok(TelegramUpdateClaimDecision::Execute)
    }

    fn complete_update(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        update_id: i64,
    ) -> Result<Option<i64>> {
        validate_stream_key(stream_key)?;
        validate_owner_token(owner_token)?;
        let next_offset = checked_next_offset(update_id)?;
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open Telegram update completion transaction")?;
        assert_active_lease(&transaction, stream_key, owner_token)?;
        let completed_at = Utc::now().to_rfc3339();
        let changed = transaction
            .execute(
                "UPDATE telegram_update_inbox
                 SET update_state = 'completed', completed_at = ?3
                 WHERE stream_key = ?1 AND update_id = ?2 AND update_state = 'executing'",
                params![stream_key, update_id, completed_at],
            )
            .context("failed to complete Telegram update")?;
        if changed == 0 {
            let state = transaction
                .query_row(
                    "SELECT update_state
                     FROM telegram_update_inbox
                     WHERE stream_key = ?1 AND update_id = ?2",
                    params![stream_key, update_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .context("failed to verify Telegram update completion")?;
            match state.as_deref() {
                Some("completed") => {}
                Some(unknown) => {
                    bail!("Telegram update cannot complete from state `{unknown}`");
                }
                None => bail!("Telegram update must be claimed before completion"),
            }
        }

        persist_cursor(&transaction, stream_key, next_offset, &completed_at)?;
        upsert_authority_metadata(&transaction, &location, "last_telegram_update_completed_at")?;
        prune_completed_updates(&transaction, stream_key)?;
        let durable_offset = load_cursor(&transaction, stream_key)?;
        transaction
            .commit()
            .context("failed to commit Telegram update completion")?;
        Ok(durable_offset)
    }
}

fn validate_stream_key(stream_key: &str) -> Result<()> {
    if stream_key.is_empty() || stream_key.len() > MAX_STREAM_KEY_BYTES || !stream_key.is_ascii() {
        bail!("Telegram update stream key must be non-empty ASCII and at most 128 bytes");
    }
    Ok(())
}

fn validate_owner_token(owner_token: &str) -> Result<()> {
    if owner_token.is_empty()
        || owner_token.len() > MAX_OWNER_TOKEN_BYTES
        || !owner_token.bytes().all(|byte| byte.is_ascii_graphic())
    {
        bail!("Telegram runner owner token must be printable ASCII and at most 128 bytes");
    }
    Ok(())
}

fn lease_deadline(lease_ttl_seconds: u64) -> Result<(i64, i64)> {
    if lease_ttl_seconds == 0 {
        bail!("Telegram runner lease TTL must be greater than zero");
    }
    let ttl_millis = i64::try_from(lease_ttl_seconds)
        .context("Telegram runner lease TTL is too large")?
        .checked_mul(1_000)
        .context("Telegram runner lease TTL overflowed")?;
    let now_millis = Utc::now().timestamp_millis();
    let expires_at_millis = now_millis
        .checked_add(ttl_millis)
        .context("Telegram runner lease deadline overflowed")?;
    Ok((now_millis, expires_at_millis))
}

fn assert_active_lease(
    transaction: &Transaction<'_>,
    stream_key: &str,
    owner_token: &str,
) -> Result<()> {
    let now_millis = Utc::now().timestamp_millis();
    let owns_active_lease = transaction
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM telegram_update_runner_leases
                 WHERE stream_key = ?1 AND owner_token = ?2
                   AND lease_expires_at_epoch_millis > ?3
             )",
            params![stream_key, owner_token, now_millis],
            |row| row.get::<_, bool>(0),
        )
        .context("failed to validate Telegram runner lease")?;
    if !owns_active_lease {
        bail!("Telegram runner lease is no longer owned or has expired");
    }
    Ok(())
}

fn checked_next_offset(update_id: i64) -> Result<i64> {
    if update_id < 0 {
        bail!("Telegram update id must be non-negative");
    }
    update_id
        .checked_add(1)
        .context("Telegram update cursor overflowed")
}

fn load_cursor(transaction: &Transaction<'_>, stream_key: &str) -> Result<Option<i64>> {
    transaction
        .query_row(
            "SELECT next_offset FROM telegram_update_streams WHERE stream_key = ?1",
            [stream_key],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .context("failed to load durable Telegram update cursor")
}

fn persist_cursor(
    transaction: &Transaction<'_>,
    stream_key: &str,
    next_offset: i64,
    updated_at: &str,
) -> Result<()> {
    transaction
        .execute(
            "INSERT INTO telegram_update_streams (stream_key, next_offset, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(stream_key) DO UPDATE SET
                 next_offset = MAX(telegram_update_streams.next_offset, excluded.next_offset),
                 updated_at = CASE
                     WHEN excluded.next_offset >= telegram_update_streams.next_offset
                     THEN excluded.updated_at
                     ELSE telegram_update_streams.updated_at
                 END",
            params![stream_key, next_offset, updated_at],
        )
        .context("failed to persist durable Telegram update cursor")?;
    Ok(())
}

fn prune_completed_updates(transaction: &Transaction<'_>, stream_key: &str) -> Result<()> {
    transaction
        .execute(
            "DELETE FROM telegram_update_inbox
             WHERE stream_key = ?1
               AND update_state = 'completed'
               AND update_id < COALESCE(
                   (SELECT update_id
                    FROM telegram_update_inbox
                    WHERE stream_key = ?1 AND update_state = 'completed'
                    ORDER BY update_id DESC
                    LIMIT 1 OFFSET ?2),
                   0
               )",
            params![stream_key, COMPLETED_UPDATE_RETENTION_OFFSET],
        )
        .context("failed to prune completed Telegram update inbox")?;
    Ok(())
}
