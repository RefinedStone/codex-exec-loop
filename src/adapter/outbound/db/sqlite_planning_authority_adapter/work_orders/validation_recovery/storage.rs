use std::collections::BTreeSet;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::super::super::{
    SqlitePlanningAuthorityAdapter, open_authority_connection, runtime_projection,
};
use super::super::{ensure_work_order_tables, validation_links};
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningValidationRecoveryOutcome, PlanningValidationRecoveryReceipt,
    PlanningValidationRecoveryRequest, validate_work_order_id,
};
use crate::domain::parallel_mode::{PrValidationRecord, PrValidationRecordKey};

pub(super) struct Original {
    pub(super) content: String,
    pub(super) operator_paused: bool,
    poll_lease_token: Option<String>,
    poll_lease_expires_at: Option<String>,
}

impl Original {
    pub(super) fn poll_in_flight(&self, now: DateTime<Utc>) -> Result<bool> {
        if self.poll_lease_token.is_none() {
            return Ok(false);
        }
        self.poll_lease_expires_at
            .as_deref()
            .map(|expiry| {
                DateTime::parse_from_rfc3339(expiry)
                    .context("invalid original validation poll lease expiry")
                    .map(|expiry| expiry > now)
            })
            .unwrap_or(Ok(true))
    }
}

pub(super) fn original(connection: &Connection, key: &str) -> Result<Option<Original>> {
    connection
        .query_row(
            "SELECT content,operator_paused,poll_lease_token,poll_lease_expires_at
         FROM runtime_pr_validation_records WHERE record_key=?1",
            [key],
            |row| {
                Ok(Original {
                    content: row.get(0)?,
                    operator_paused: row.get::<_, i64>(1)? != 0,
                    poll_lease_token: row.get(2)?,
                    poll_lease_expires_at: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

pub(super) fn existing_receipt(
    connection: &Connection,
    request: &PlanningValidationRecoveryRequest<'_>,
    binding: &str,
) -> Result<Option<PlanningValidationRecoveryReceipt>> {
    let mut statement = connection.prepare(
        "SELECT request_json,receipt_json,successor_record_key
         FROM planning_work_order_validation_recoveries
         WHERE command_id=?1 OR (work_order_id=?2 AND terminal_record_id=?3)",
    )?;
    let rows = statement
        .query_map(
            params![
                request.command_id,
                request.work_order_id,
                request.terminal_record_id
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let [(stored, receipt, successor)] = rows.as_slice() else {
        if rows.is_empty() {
            return Ok(None);
        }
        bail!("validation recovery identity conflicts with another completed command");
    };
    if stored != binding {
        bail!("validation recovery command payload is immutable");
    }
    let receipt: PlanningValidationRecoveryReceipt = serde_json::from_str(receipt)?;
    if receipt.command_id != request.command_id
        || receipt.terminal_record_id != request.terminal_record_id
        || receipt.outcome
            != (PlanningValidationRecoveryOutcome::Registered {
                validation_record_key: successor.clone(),
            })
    {
        bail!("validation recovery receipt identity is inconsistent");
    }
    Ok(Some(receipt))
}

pub(super) fn revoked_at_or_after(
    connection: &Connection,
    work_order_id: &str,
    revision: u64,
) -> Result<bool> {
    let mut statement = connection.prepare(
        "SELECT gate_revision FROM planning_work_order_permit_revocations WHERE work_order_id=?1",
    )?;
    for stored in statement.query_map([work_order_id], |row| row.get::<_, String>(0))? {
        if stored?
            .parse::<u64>()
            .context("invalid revoked work-order gate revision")?
            >= revision
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn origin_in_connection(connection: &Connection, key: &str) -> Result<Option<String>> {
    let row: Option<(String, String)> = connection
        .query_row(
            "SELECT work_order_id,remediation_origin_record_key
         FROM planning_work_order_validation_recoveries WHERE successor_record_key=?1",
            [key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((owner, origin)) = row else {
        return Ok(None);
    };
    for correlated in [key, origin.as_str()] {
        let source = validation_links::read_source(connection, correlated)?
            .context("recovery remediation origin lost its immutable source")?;
        if source.work_order_id.as_deref() != Some(owner.as_str()) {
            bail!("recovery remediation origin belongs to another work order");
        }
    }
    Ok(Some(origin))
}

pub(crate) fn load_origin(workspace: &str, key: &str) -> Result<Option<String>> {
    validate_work_order_id(key)?;
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let connection = open_authority_connection(&location)?;
    ensure_work_order_tables(&connection)?;
    origin_in_connection(&connection, key)
}

/// A receipt, rather than matching PR numbers, establishes successor identity.
pub(crate) fn resolve_successor_key(
    workspace: &str,
    original: &PrValidationRecordKey,
) -> Result<PrValidationRecordKey> {
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let mut connection = open_authority_connection(&location)?;
    ensure_work_order_tables(&connection)?;
    // A read snapshot has a finite chain even while another process registers a later successor.
    let transaction = connection.transaction()?;
    resolve_in_snapshot(&transaction, original)
}

fn resolve_in_snapshot(
    connection: &Connection,
    original: &PrValidationRecordKey,
) -> Result<PrValidationRecordKey> {
    let mut current = original.clone();
    let mut visited = BTreeSet::new();
    loop {
        if !visited.insert(current.as_str().to_owned()) {
            bail!("validation recovery successor chain is cyclic");
        }
        let mut statement = connection.prepare(
            "SELECT successor_record_key,work_order_id FROM planning_work_order_validation_recoveries
             WHERE original_record_key=?1 LIMIT 2"
        )?;
        let rows = statement
            .query_map([current.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let [(next, owner)] = rows.as_slice() else {
            if rows.is_empty() {
                return Ok(current);
            }
            bail!("validation recovery successor identity is ambiguous");
        };
        for key in [current.as_str(), next.as_str()] {
            let source = validation_links::read_source(connection, key)?
                .context("validation recovery successor source is missing")?;
            if source.work_order_id.as_deref() != Some(owner.as_str()) {
                bail!("validation recovery successor belongs to another work order");
            }
        }
        let next = PrValidationRecordKey::new(next).map_err(anyhow::Error::msg)?;
        let content: String = connection
            .query_row(
                "SELECT content FROM runtime_pr_validation_records WHERE record_key=?1",
                [next.as_str()],
                |row| row.get(0),
            )
            .context("validation recovery successor record is missing")?;
        if serde_json::from_str::<PrValidationRecord>(&content)?.key() != &next {
            bail!("validation recovery successor record identity is inconsistent");
        }
        current = next;
    }
}

pub(super) fn insert_successor(
    tx: &Transaction<'_>,
    original_key: &str,
    original_content: &str,
    successor: &PrValidationRecord,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    let attestation = successor.integration_attestation();
    let remote_verified_at = attestation.map(|value| value.remote_verified_at().to_rfc3339());
    // All provider pacing facts survive. Recovery clears only the old poll lease and does not
    // force a new read before the previous next_poll_at / provider rate-limit deadline.
    let inserted = tx.execute(
        "INSERT INTO runtime_pr_validation_records (
            record_key,updated_at,content,validation_repository,validation_phase,review_watch_active,
            integration_method,integration_source_sha,integration_evidence_sha,integration_remote_verified_at,
            next_poll_at,last_polled_at,poll_attempt,consecutive_error_count,last_error_class,
            rate_limit_remaining,rate_limit_reset_at,operator_paused,
            operator_acknowledged_at,last_operator_command_id
         ) SELECT ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,
            COALESCE(next_poll_at,?2),last_polled_at,poll_attempt,consecutive_error_count,
            last_error_class,rate_limit_remaining,rate_limit_reset_at,operator_paused,
            operator_acknowledged_at,last_operator_command_id
         FROM runtime_pr_validation_records WHERE record_key=?11 AND content=?12 AND operator_paused=0",
        params![
            successor.key().as_str(),now,serde_json::to_string(successor)?,
            successor.target().repository(),successor.phase().storage_label(),
            i64::from(successor.review_watch_active()),
            attestation.map(|value|value.method().label()),
            attestation.map(|value|value.source_sha().as_str()),
            attestation.map(|value|value.evidence_sha().as_str()),
            remote_verified_at,original_key,original_content,
        ],
    )?;
    if inserted != 1 {
        bail!("validation recovery original changed before successor registration");
    }
    runtime_projection::append_runtime_event(
        tx,
        "pr_validation_phase_changed",
        "validation_record",
        successor.key().as_str(),
        "fresh validation successor registered",
        &serde_json::json!({
            "schema_version":1,
            "record_key":successor.key().as_str(),
            "previous_phase":null,
            "phase":successor.phase(),
            "observation_revision":successor.observation_revision(),
        })
        .to_string(),
    )?;
    Ok(())
}
