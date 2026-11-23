use crate::application::port::outbound::parallel_mode_runtime_port::ParallelCommitDiffRequest;
use crate::domain::issue_automation::delivery::IssueDeliveryDiff;

use super::ParallelModeService;

impl ParallelModeService {
    /// The composition caller supplies the current task/lease/queue's verified worktree and OIDs.
    /// This read does not refresh a remote, select a mutable branch, or issue a delivery permit.
    pub(crate) fn inspect_work_order_delivery_diff(
        &self,
        worktree: &str,
        base_sha: &str,
        source_sha: &str,
    ) -> Result<IssueDeliveryDiff, String> {
        self.parallel_runtime
            .inspect_parallel_commit_diff(ParallelCommitDiffRequest {
                workspace_directory: worktree,
                base_commit_sha: base_sha,
                source_commit_sha: source_sha,
            })
    }
}
