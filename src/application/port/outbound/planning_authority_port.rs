use anyhow::Result;

use crate::domain::planning::{PlanningAuthorityLocation, PlanningAuthorityShadowStoreInspection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningAuthorityOfficialRefreshClaimStatus {
    Acquired,
    Waiting,
    AlreadyCompleted,
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
}
