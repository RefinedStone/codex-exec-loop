use anyhow::Result;

pub const PR_VALIDATION_EVIDENCE_STORE_RETENTION: usize = 64;
pub const PR_VALIDATION_EVIDENCE_STORE_MAX_BATCH: usize = 32;
pub const PR_VALIDATION_EVIDENCE_STORE_MAX_SNAPSHOT_BYTES: usize = 256 * 1024;
pub const PR_VALIDATION_EVIDENCE_COLLECTION_LEASE_MILLIS: i64 = 15_000;
pub const PR_VALIDATION_EVIDENCE_COLLECTION_COOLDOWN_MILLIS: i64 = 5_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrValidationEvidenceCollectionClaimRequest {
    pub scope_key: String,
    pub owner_token: String,
    pub lease_token: String,
    pub now_epoch_millis: i64,
    pub lease_duration_millis: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrValidationEvidenceCollectionClaim {
    Acquired {
        lease_expires_at_epoch_millis: i64,
        recovered_stale_lease: bool,
    },
    Held {
        lease_expires_at_epoch_millis: i64,
    },
    CoolingDown {
        retry_at_epoch_millis: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrValidationEvidenceStoredCandidate {
    pub dedupe_key: String,
    pub repository: Option<String>,
    pub base_branch: Option<String>,
    pub evidence_sha: Option<String>,
    pub generated_at: Option<String>,
    pub sort_at: String,
    pub observed_at: String,
    pub observation_kind: String,
    pub status_label: String,
    pub artifact_sha: String,
    pub snapshot_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrValidationEvidenceCollectionSettlementRequest {
    pub scope_key: String,
    pub owner_token: String,
    pub lease_token: String,
    pub now_epoch_millis: i64,
    pub next_collect_at_epoch_millis: i64,
    pub collected_at: String,
    pub outcome: String,
    pub error_class: Option<String>,
    pub candidates: Vec<PrValidationEvidenceStoredCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrValidationEvidenceCollectionSettlement {
    Stored {
        inserted: usize,
        duplicates: usize,
        identity_conflicts: usize,
        revision: i64,
    },
    LostLease,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrValidationEvidenceStoredRecord {
    pub sequence: i64,
    pub dedupe_key: String,
    pub sort_at: String,
    pub artifact_sha: String,
    pub observation_kind: String,
    pub snapshot_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrValidationEvidenceCollectionState {
    pub revision: i64,
    pub lease_active: bool,
    pub lease_expires_at_epoch_millis: Option<i64>,
    pub next_collect_at_epoch_millis: Option<i64>,
    pub last_collected_at: Option<String>,
    pub last_outcome: Option<String>,
    pub last_error_class: Option<String>,
    pub stale_lease_recovery_count: u64,
    pub identity_conflict_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrValidationEvidenceStoreSnapshot {
    pub records: Vec<PrValidationEvidenceStoredRecord>,
    pub collection: PrValidationEvidenceCollectionState,
}

/// Durable, repository-scoped storage for already validated and redacted rollout evidence.
///
/// Implementations must never persist the raw provider document. The application service owns
/// validation and supplies only bounded snapshots plus the minimum identity metadata required for
/// deduplication and diagnostics.
pub trait PrValidationRolloutEvidenceStorePort: Send + Sync {
    fn try_claim_collection(
        &self,
        workspace_dir: &str,
        request: &PrValidationEvidenceCollectionClaimRequest,
    ) -> Result<PrValidationEvidenceCollectionClaim>;

    fn settle_collection(
        &self,
        workspace_dir: &str,
        request: PrValidationEvidenceCollectionSettlementRequest,
    ) -> Result<PrValidationEvidenceCollectionSettlement>;

    fn load_snapshot(
        &self,
        workspace_dir: &str,
        scope_key: &str,
    ) -> Result<PrValidationEvidenceStoreSnapshot>;

    fn load_revision(&self, workspace_dir: &str, scope_key: &str) -> Result<i64>;
}
