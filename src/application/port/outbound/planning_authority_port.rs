use anyhow::Result;

use crate::domain::planning::{PlanningAuthorityLocation, PlanningAuthorityShadowStoreInspection};

pub trait PlanningAuthorityPort: Send + Sync {
    fn resolve_authority_location(&self, workspace_dir: &str) -> Result<PlanningAuthorityLocation>;

    fn inspect_shadow_store(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityShadowStoreInspection>;
}
