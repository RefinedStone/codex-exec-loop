use crate::domain::planning::evidence::TaskEvidenceTree;
use crate::domain::planning::verification::{
    TaskVerificationExecutionManifest, TaskVerificationNotRunReason, TaskVerificationObservation,
    TaskVerificationRecipe,
};

/// Host-selected repository state. No worker-supplied command, environment, or artifact path.
pub struct TaskVerificationPrepareRequest<'a> {
    pub workspace_directory: &'a str,
    pub recipe: TaskVerificationRecipe,
    pub base_commit_sha: &'a str,
    pub source: &'a TaskEvidenceTree,
}

pub enum TaskVerificationRunOutcome {
    Executed {
        observation: Box<TaskVerificationObservation>,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    NotRun {
        reason: TaskVerificationNotRunReason,
    },
}

/// Affine prepared execution: the context and pinned executable survive until this
/// object is consumed. The application binds its manifest durably before run().
pub trait PreparedTaskVerification: Send {
    fn manifest(&self) -> &TaskVerificationExecutionManifest;
    fn run(self: Box<Self>, cancelled: &dyn Fn() -> bool) -> TaskVerificationRunOutcome;
}

pub trait TaskVerificationRunnerPort: Send + Sync {
    fn prepare(
        &self,
        request: TaskVerificationPrepareRequest<'_>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Box<dyn PreparedTaskVerification>, TaskVerificationNotRunReason>;

    /// The environment depends on recipe and Git object format (validated tree OID
    /// length), not the particular repository, base/source identity, or temporary
    /// path. Queries share one initial/final observation for each such key.
    fn environment_fingerprint(
        &self,
        manifest: &TaskVerificationExecutionManifest,
    ) -> Result<String, TaskVerificationNotRunReason>;
}
