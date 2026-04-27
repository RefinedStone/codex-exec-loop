use std::sync::atomic::{AtomicU64, Ordering};

use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityOfficialRefreshClaimStatus,
    PlanningAuthorityPort, PlanningAuthorityRuntimeProjectionSnapshot,
};
use crate::application::port::outbound::planning_worker_port::{
    PlanningWorkerPort, PlanningWorkerRequest, PlanningWorkerResponse,
};
use crate::domain::planning::{
    PlanningAuthorityLocation, PlanningAuthorityShadowStoreInspection,
    PlanningAuthorityShadowStoreSyncState,
};

pub(super) struct NoopPlanningWorkerPort;

impl PlanningWorkerPort for NoopPlanningWorkerPort {
    fn run_planning_session(
        &self,
        request: PlanningWorkerRequest,
    ) -> anyhow::Result<PlanningWorkerResponse> {
        Ok(PlanningWorkerResponse {
            operation: request.operation,
            final_agent_message: Some("planner worker disabled".to_string()),
            changed_planning_file_paths: Vec::new(),
        })
    }
}

#[derive(Default)]
pub(super) struct NoopPlanningAuthorityPort {
    next_refresh_order: AtomicU64,
}

impl PlanningAuthorityPort for NoopPlanningAuthorityPort {
    fn resolve_authority_location(
        &self,
        workspace_dir: &str,
    ) -> anyhow::Result<PlanningAuthorityLocation> {
        Ok(PlanningAuthorityLocation {
            workspace_root: workspace_dir.to_string(),
            canonical_repo_root: workspace_dir.to_string(),
            runtime_dir: String::new(),
            authority_store_path: String::new(),
        })
    }

    fn inspect_shadow_store(
        &self,
        workspace_dir: &str,
    ) -> anyhow::Result<PlanningAuthorityShadowStoreInspection> {
        Ok(PlanningAuthorityShadowStoreInspection {
            location: self.resolve_authority_location(workspace_dir)?,
            sync_state: PlanningAuthorityShadowStoreSyncState::InSync,
            mirrored_document_count: 0,
            parity_issue_count: 0,
            parity_issue_examples: Vec::new(),
        })
    }

    fn reserve_next_official_refresh_order(&self, _workspace_dir: &str) -> anyhow::Result<u64> {
        Ok(self.next_refresh_order.fetch_add(1, Ordering::Relaxed) + 1)
    }

    fn acquire_official_refresh_claim(
        &self,
        _workspace_dir: &str,
        _refresh_order: u64,
        _owner_token: &str,
    ) -> anyhow::Result<PlanningAuthorityOfficialRefreshClaimStatus> {
        Ok(PlanningAuthorityOfficialRefreshClaimStatus::Acquired)
    }

    fn release_official_refresh_claim(
        &self,
        _workspace_dir: &str,
        _refresh_order: u64,
        _owner_token: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn try_acquire_distributor_queue_claim(
        &self,
        _workspace_dir: &str,
        _queue_item_id: &str,
        _owner_token: &str,
    ) -> anyhow::Result<bool> {
        Ok(true)
    }

    fn release_distributor_queue_claim(
        &self,
        _workspace_dir: &str,
        _queue_item_id: &str,
        _owner_token: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn load_runtime_projections(
        &self,
        _workspace_dir: &str,
    ) -> anyhow::Result<PlanningAuthorityRuntimeProjectionSnapshot> {
        Ok(PlanningAuthorityRuntimeProjectionSnapshot::default())
    }

    fn upsert_runtime_slot_lease(
        &self,
        _workspace_dir: &str,
        _lease: &crate::domain::parallel_mode::ParallelModeSlotLeaseSnapshot,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn remove_runtime_slot_lease(
        &self,
        _workspace_dir: &str,
        _slot_id: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn upsert_runtime_session_detail(
        &self,
        _workspace_dir: &str,
        _detail: &crate::domain::parallel_mode::ParallelModeAgentSessionDetailSnapshot,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn upsert_runtime_distributor_queue_record(
        &self,
        _workspace_dir: &str,
        _record: &PlanningAuthorityDistributorQueueRecord,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}
