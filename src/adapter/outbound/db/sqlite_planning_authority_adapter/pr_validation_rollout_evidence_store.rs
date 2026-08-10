use anyhow::{Context, Result, bail};
use chrono::Utc;
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use crate::application::port::outbound::pr_validation_rollout_evidence_store_port::{
    PR_VALIDATION_EVIDENCE_STORE_MAX_BATCH, PR_VALIDATION_EVIDENCE_STORE_MAX_SNAPSHOT_BYTES,
    PR_VALIDATION_EVIDENCE_STORE_RETENTION, PrValidationEvidenceCollectionClaim,
    PrValidationEvidenceCollectionClaimRequest, PrValidationEvidenceCollectionSettlement,
    PrValidationEvidenceCollectionSettlementRequest, PrValidationEvidenceCollectionState,
    PrValidationEvidenceStoreSnapshot, PrValidationEvidenceStoredRecord,
    PrValidationRolloutEvidenceStorePort,
};

use super::{SqlitePlanningAuthorityAdapter, open_authority_connection};

const MAX_METADATA_CHARS: usize = 512;

impl PrValidationRolloutEvidenceStorePort for SqlitePlanningAuthorityAdapter {
    fn try_claim_collection(
        &self,
        workspace_dir: &str,
        request: &PrValidationEvidenceCollectionClaimRequest,
    ) -> Result<PrValidationEvidenceCollectionClaim> {
        validate_claim_request(request)?;
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open rollout evidence collection claim transaction")?;
        let updated_at = Utc::now().to_rfc3339();
        transaction
            .execute(
                "INSERT OR IGNORE INTO runtime_pr_validation_evidence_collection (
                     scope_key, next_collect_at_epoch_millis, revision,
                     stale_lease_recovery_count, identity_conflict_count, updated_at
                 ) VALUES (?1, 0, 0, 0, 0, ?2)",
                params![request.scope_key, updated_at],
            )
            .context("failed to initialize rollout evidence collection ownership")?;
        let current = transaction
            .query_row(
                "SELECT lease_owner, lease_expires_at_epoch_millis,
                        next_collect_at_epoch_millis
                 FROM runtime_pr_validation_evidence_collection
                 WHERE scope_key = ?1",
                [&request.scope_key],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .context("failed to inspect rollout evidence collection ownership")?;
        if current.0.is_some()
            && current
                .1
                .is_some_and(|expires_at| expires_at > request.now_epoch_millis)
        {
            transaction
                .commit()
                .context("failed to close held rollout evidence collection claim")?;
            return Ok(PrValidationEvidenceCollectionClaim::Held {
                lease_expires_at_epoch_millis: current.1.unwrap_or(request.now_epoch_millis),
            });
        }
        let recovered_stale_lease = current.0.is_some();
        if !recovered_stale_lease && current.2 > request.now_epoch_millis {
            transaction
                .commit()
                .context("failed to close cooled-down rollout evidence collection claim")?;
            return Ok(PrValidationEvidenceCollectionClaim::CoolingDown {
                retry_at_epoch_millis: current.2,
            });
        }
        let lease_expires_at_epoch_millis = request
            .now_epoch_millis
            .checked_add(request.lease_duration_millis)
            .ok_or_else(|| anyhow::anyhow!("rollout evidence lease timestamp overflowed"))?;
        transaction
            .execute(
                "UPDATE runtime_pr_validation_evidence_collection
                 SET lease_owner = ?2,
                     lease_token = ?3,
                     lease_expires_at_epoch_millis = ?4,
                     stale_lease_recovery_count = stale_lease_recovery_count + ?5,
                     updated_at = ?6
                 WHERE scope_key = ?1",
                params![
                    request.scope_key,
                    request.owner_token,
                    request.lease_token,
                    lease_expires_at_epoch_millis,
                    i64::from(recovered_stale_lease),
                    updated_at,
                ],
            )
            .context("failed to claim rollout evidence collection ownership")?;
        transaction
            .commit()
            .context("failed to commit rollout evidence collection claim")?;
        Ok(PrValidationEvidenceCollectionClaim::Acquired {
            lease_expires_at_epoch_millis,
            recovered_stale_lease,
        })
    }

    fn settle_collection(
        &self,
        workspace_dir: &str,
        request: PrValidationEvidenceCollectionSettlementRequest,
    ) -> Result<PrValidationEvidenceCollectionSettlement> {
        validate_settlement_request(&request)?;
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open rollout evidence collection settlement transaction")?;
        let ownership = transaction
            .query_row(
                "SELECT lease_owner, lease_token, lease_expires_at_epoch_millis,
                        last_outcome, last_error_class, revision
                 FROM runtime_pr_validation_evidence_collection
                 WHERE scope_key = ?1",
                [&request.scope_key],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()
            .context("failed to inspect rollout evidence settlement ownership")?;
        let Some((owner, token, expires_at, previous_outcome, previous_error, revision)) =
            ownership
        else {
            return Ok(PrValidationEvidenceCollectionSettlement::LostLease);
        };
        if owner.as_deref() != Some(request.owner_token.as_str())
            || token.as_deref() != Some(request.lease_token.as_str())
            || expires_at.is_none_or(|value| value < request.now_epoch_millis)
        {
            return Ok(PrValidationEvidenceCollectionSettlement::LostLease);
        }

        let mut inserted = 0usize;
        let mut duplicates = 0usize;
        let mut identity_conflicts = 0usize;
        for candidate in &request.candidates {
            let existing_artifact = transaction
                .query_row(
                    "SELECT artifact_sha
                     FROM runtime_pr_validation_evidence_snapshots
                     WHERE scope_key = ?1 AND dedupe_key = ?2",
                    params![request.scope_key, candidate.dedupe_key],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .context("failed to inspect rollout evidence dedupe identity")?;
            match existing_artifact {
                Some(artifact_sha) if artifact_sha == candidate.artifact_sha => {
                    duplicates += 1;
                }
                Some(_) => {
                    identity_conflicts += 1;
                }
                None => {
                    transaction
                        .execute(
                            "INSERT INTO runtime_pr_validation_evidence_snapshots (
                                 scope_key, dedupe_key, repository, base_branch, evidence_sha,
                                 generated_at, sort_at, observed_at, observation_kind,
                                 status_label, artifact_sha, content_json, stored_at
                             ) VALUES (
                                 ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13
                             )",
                            params![
                                request.scope_key,
                                candidate.dedupe_key,
                                candidate.repository,
                                candidate.base_branch,
                                candidate.evidence_sha,
                                candidate.generated_at,
                                candidate.sort_at,
                                candidate.observed_at,
                                candidate.observation_kind,
                                candidate.status_label,
                                candidate.artifact_sha,
                                candidate.snapshot_json,
                                request.collected_at,
                            ],
                        )
                        .context("failed to persist rollout evidence snapshot")?;
                    inserted += 1;
                }
            }
        }
        transaction
            .execute(
                "DELETE FROM runtime_pr_validation_evidence_snapshots
                 WHERE scope_key = ?1
                   AND sequence NOT IN (
                       SELECT sequence
                       FROM runtime_pr_validation_evidence_snapshots
                       WHERE scope_key = ?1
                       ORDER BY sort_at DESC, artifact_sha DESC, sequence DESC
                       LIMIT ?2
                   )",
                params![
                    request.scope_key,
                    PR_VALIDATION_EVIDENCE_STORE_RETENTION as i64
                ],
            )
            .context("failed to enforce rollout evidence retention")?;
        let state_changed = inserted > 0
            || identity_conflicts > 0
            || previous_outcome.as_deref() != Some(request.outcome.as_str())
            || previous_error != request.error_class;
        let next_revision = revision + i64::from(state_changed);
        let updated = transaction
            .execute(
                "UPDATE runtime_pr_validation_evidence_collection
                 SET lease_owner = NULL,
                     lease_token = NULL,
                     lease_expires_at_epoch_millis = NULL,
                     next_collect_at_epoch_millis = ?4,
                     last_collected_at = ?5,
                     last_outcome = ?6,
                     last_error_class = ?7,
                     revision = ?8,
                     identity_conflict_count = identity_conflict_count + ?9,
                     updated_at = ?5
                 WHERE scope_key = ?1 AND lease_owner = ?2 AND lease_token = ?3",
                params![
                    request.scope_key,
                    request.owner_token,
                    request.lease_token,
                    request.next_collect_at_epoch_millis,
                    request.collected_at,
                    request.outcome,
                    request.error_class,
                    next_revision,
                    identity_conflicts as i64,
                ],
            )
            .context("failed to release rollout evidence collection ownership")?;
        if updated != 1 {
            return Ok(PrValidationEvidenceCollectionSettlement::LostLease);
        }
        transaction
            .commit()
            .context("failed to commit rollout evidence collection settlement")?;
        Ok(PrValidationEvidenceCollectionSettlement::Stored {
            inserted,
            duplicates,
            identity_conflicts,
            revision: next_revision,
        })
    }

    fn load_snapshot(
        &self,
        workspace_dir: &str,
        scope_key: &str,
    ) -> Result<PrValidationEvidenceStoreSnapshot> {
        validate_bounded_text(scope_key, "rollout evidence scope")?;
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        let mut statement = connection
            .prepare(
                "SELECT sequence, dedupe_key, sort_at, artifact_sha, observation_kind,
                        content_json
                 FROM runtime_pr_validation_evidence_snapshots
                 WHERE scope_key = ?1
                 ORDER BY sort_at DESC, artifact_sha DESC, sequence DESC
                 LIMIT ?2",
            )
            .context("failed to prepare rollout evidence history query")?;
        let records = statement
            .query_map(
                params![scope_key, PR_VALIDATION_EVIDENCE_STORE_RETENTION as i64],
                |row| {
                    Ok(PrValidationEvidenceStoredRecord {
                        sequence: row.get(0)?,
                        dedupe_key: row.get(1)?,
                        sort_at: row.get(2)?,
                        artifact_sha: row.get(3)?,
                        observation_kind: row.get(4)?,
                        snapshot_json: row.get(5)?,
                    })
                },
            )
            .context("failed to load rollout evidence history")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to decode rollout evidence history")?;
        drop(statement);
        let now_epoch_millis = Utc::now().timestamp_millis();
        let collection = connection
            .query_row(
                "SELECT revision, lease_owner, lease_expires_at_epoch_millis,
                        next_collect_at_epoch_millis, last_collected_at, last_outcome,
                        last_error_class, stale_lease_recovery_count, identity_conflict_count
                 FROM runtime_pr_validation_evidence_collection
                 WHERE scope_key = ?1",
                [scope_key],
                |row| {
                    let owner = row.get::<_, Option<String>>(1)?;
                    let expires_at = row.get::<_, Option<i64>>(2)?;
                    Ok(PrValidationEvidenceCollectionState {
                        revision: row.get(0)?,
                        lease_active: owner.is_some()
                            && expires_at.is_some_and(|value| value > now_epoch_millis),
                        lease_expires_at_epoch_millis: expires_at,
                        next_collect_at_epoch_millis: row.get(3)?,
                        last_collected_at: row.get(4)?,
                        last_outcome: row.get(5)?,
                        last_error_class: row.get(6)?,
                        stale_lease_recovery_count: row.get::<_, i64>(7)?.max(0) as u64,
                        identity_conflict_count: row.get::<_, i64>(8)?.max(0) as u64,
                    })
                },
            )
            .optional()
            .context("failed to load rollout evidence collection state")?
            .unwrap_or_default();
        Ok(PrValidationEvidenceStoreSnapshot {
            records,
            collection,
        })
    }

    fn load_revision(&self, workspace_dir: &str, scope_key: &str) -> Result<i64> {
        validate_bounded_text(scope_key, "rollout evidence scope")?;
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let connection = open_authority_connection(&location)?;
        connection
            .query_row(
                "SELECT revision
                 FROM runtime_pr_validation_evidence_collection
                 WHERE scope_key = ?1",
                [scope_key],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .context("failed to load rollout evidence revision")
            .map(|revision| revision.unwrap_or(0))
    }
}

fn validate_claim_request(request: &PrValidationEvidenceCollectionClaimRequest) -> Result<()> {
    validate_bounded_text(&request.scope_key, "rollout evidence scope")?;
    validate_bounded_text(&request.owner_token, "rollout evidence owner token")?;
    validate_bounded_text(&request.lease_token, "rollout evidence lease token")?;
    if request.now_epoch_millis < 0 || request.lease_duration_millis <= 0 {
        bail!("rollout evidence collection claim timestamps are invalid");
    }
    Ok(())
}

fn validate_settlement_request(
    request: &PrValidationEvidenceCollectionSettlementRequest,
) -> Result<()> {
    validate_bounded_text(&request.scope_key, "rollout evidence scope")?;
    validate_bounded_text(&request.owner_token, "rollout evidence owner token")?;
    validate_bounded_text(&request.lease_token, "rollout evidence lease token")?;
    validate_bounded_text(
        &request.collected_at,
        "rollout evidence collection timestamp",
    )?;
    validate_bounded_text(&request.outcome, "rollout evidence collection outcome")?;
    if let Some(error_class) = &request.error_class {
        validate_bounded_text(error_class, "rollout evidence collection error class")?;
    }
    if request.now_epoch_millis < 0
        || request.next_collect_at_epoch_millis < request.now_epoch_millis
        || request.candidates.len() > PR_VALIDATION_EVIDENCE_STORE_MAX_BATCH
    {
        bail!("rollout evidence collection settlement violates its bounded contract");
    }
    for candidate in &request.candidates {
        for (value, label) in [
            (&candidate.dedupe_key, "rollout evidence dedupe key"),
            (&candidate.sort_at, "rollout evidence sort timestamp"),
            (
                &candidate.observed_at,
                "rollout evidence observation timestamp",
            ),
            (
                &candidate.observation_kind,
                "rollout evidence observation kind",
            ),
            (&candidate.status_label, "rollout evidence status"),
            (&candidate.artifact_sha, "rollout evidence artifact SHA"),
        ] {
            validate_bounded_text(value, label)?;
        }
        if candidate.snapshot_json.len() > PR_VALIDATION_EVIDENCE_STORE_MAX_SNAPSHOT_BYTES {
            bail!("rollout evidence snapshot exceeds the durable storage bound");
        }
    }
    Ok(())
}

fn validate_bounded_text(value: &str, label: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.chars().count() > MAX_METADATA_CHARS
        || value.chars().any(char::is_control)
    {
        bail!("{label} violates the bounded storage contract");
    }
    Ok(())
}
