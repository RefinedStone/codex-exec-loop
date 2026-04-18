use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::application::port::outbound::github_automation_port::GithubAutomationCapabilities;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, ParallelModeDistributorQueueItem,
    ParallelModeQueueItemState, ParallelModeSlotLeaseSnapshot,
};
use crate::domain::planning::{PlanningAuthorityLocation, PlanningAuthorityShadowStoreInspection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningAuthorityOfficialRefreshClaimStatus {
    Acquired,
    Waiting,
    AlreadyCompleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningAuthorityDistributorQueueRecord {
    pub queue_item_id: String,
    pub session_key: String,
    pub agent_id: String,
    pub task_id: String,
    pub task_title: String,
    pub branch_name: String,
    pub worktree_path: String,
    pub commit_sha: String,
    #[serde(default)]
    pub original_commit_sha: Option<String>,
    pub validation_summary: String,
    pub ledger_refresh_outcome: String,
    #[serde(default)]
    pub github_capabilities: Option<GithubAutomationCapabilities>,
    #[serde(default)]
    pub pull_request_number: Option<u64>,
    #[serde(default)]
    pub pull_request_url: Option<String>,
    pub queue_state: ParallelModeQueueItemState,
    pub integration_note: String,
    pub enqueued_at: String,
    pub updated_at: String,
}

impl PlanningAuthorityDistributorQueueRecord {
    pub fn display_item(&self) -> ParallelModeDistributorQueueItem {
        ParallelModeDistributorQueueItem::new(
            self.agent_id.clone(),
            self.task_title.clone(),
            self.queue_state,
            self.branch_name.clone(),
            self.commit_sha.chars().take(7).collect::<String>(),
            self.integration_note.clone(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlanningAuthorityRuntimeProjectionSnapshot {
    pub slot_leases: BTreeMap<String, ParallelModeSlotLeaseSnapshot>,
    pub invalid_slot_leases: BTreeSet<String>,
    pub session_details: Vec<ParallelModeAgentSessionDetailSnapshot>,
    pub distributor_queue_records: Vec<PlanningAuthorityDistributorQueueRecord>,
}

pub trait PlanningAuthorityPort: Send + Sync {
    fn resolve_authority_location(&self, workspace_dir: &str) -> Result<PlanningAuthorityLocation>;

    fn inspect_shadow_store(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityShadowStoreInspection>;

    fn reserve_next_official_refresh_order(&self, workspace_dir: &str) -> Result<u64>;

    fn acquire_official_refresh_claim(
        &self,
        workspace_dir: &str,
        refresh_order: u64,
        owner_token: &str,
    ) -> Result<PlanningAuthorityOfficialRefreshClaimStatus>;

    fn release_official_refresh_claim(
        &self,
        workspace_dir: &str,
        refresh_order: u64,
        owner_token: &str,
    ) -> Result<()>;

    fn try_acquire_distributor_queue_claim(
        &self,
        workspace_dir: &str,
        queue_item_id: &str,
        owner_token: &str,
    ) -> Result<bool>;

    fn release_distributor_queue_claim(
        &self,
        workspace_dir: &str,
        queue_item_id: &str,
        owner_token: &str,
    ) -> Result<()>;

    fn load_runtime_projections(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityRuntimeProjectionSnapshot>;

    fn upsert_runtime_slot_lease(
        &self,
        workspace_dir: &str,
        lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<()>;

    fn remove_runtime_slot_lease(&self, workspace_dir: &str, slot_id: &str) -> Result<()>;

    fn upsert_runtime_session_detail(
        &self,
        workspace_dir: &str,
        detail: &ParallelModeAgentSessionDetailSnapshot,
    ) -> Result<()>;

    fn upsert_runtime_distributor_queue_record(
        &self,
        workspace_dir: &str,
        record: &PlanningAuthorityDistributorQueueRecord,
    ) -> Result<()>;
}
