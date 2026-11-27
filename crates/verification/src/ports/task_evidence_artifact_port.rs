use std::path::PathBuf;

use anyhow::Result;

use crate::evidence::{TaskEvidenceArtifact, TaskEvidenceAttempt, TaskEvidenceTree};

/// Host-owned paths and byte capture. Callers independently guard the current lease
/// and compare worktree snapshots before and after this bounded filesystem operation.
pub trait TaskEvidenceArtifactPort: Send + Sync {
    /// The only extra writable root exposed to this generation's worker sandbox.
    fn prepare_lease_incoming_root(
        &self,
        workspace: &str,
        lease_generation: &str,
    ) -> Result<PathBuf>;

    /// Prepare the fixed file without truncating an earlier preparation or worker output.
    fn prepare_artifact(&self, workspace: &str, attempt: &TaskEvidenceAttempt) -> Result<PathBuf>;

    /// Copy stable, nonempty bounded bytes to a host-owned, no-replace sealed file.
    /// A retry may verify the exact same bytes; it must never overwrite a sealed artifact.
    fn observe_and_seal(
        &self,
        workspace: &str,
        attempt: &TaskEvidenceAttempt,
        observed_tree: &TaskEvidenceTree,
    ) -> Result<TaskEvidenceArtifact>;

    /// Verify persisted bytes again without recreating or repairing a missing artifact.
    fn verify_sealed(
        &self,
        workspace: &str,
        attempt: &TaskEvidenceAttempt,
        artifact: &TaskEvidenceArtifact,
    ) -> Result<()>;

    /// Seal host-generated capture only, in a separate verification namespace.
    /// The caller has already reserved this host attempt's artifact quota.
    fn seal_host_capture(
        &self,
        _workspace: &str,
        _attempt_id: &str,
        _bytes: &[u8],
    ) -> Result<TaskEvidenceArtifact> {
        anyhow::bail!("host verification artifact capture is unavailable")
    }

    fn verify_host_capture(
        &self,
        _workspace: &str,
        _attempt_id: &str,
        _artifact: &TaskEvidenceArtifact,
    ) -> Result<()> {
        anyhow::bail!("host verification artifact verification is unavailable")
    }
}
