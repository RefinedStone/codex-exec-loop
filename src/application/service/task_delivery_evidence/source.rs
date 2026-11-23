use super::*;
use crate::domain::planning::evidence::TaskEvidenceTree;

fn read_tree(
    dependencies: &TaskDeliveryEvidenceDependencies,
    commit: &str,
) -> Result<String, TaskDeliveryEvidenceReason> {
    TaskEvidenceTree {
        head_commit_sha: commit.to_owned(),
        tree_sha: commit.to_owned(),
        worktree_clean: true,
    }
    .validate()
    .map_err(|_| TaskDeliveryEvidenceReason::EvidenceStale)?;
    let tree = dependencies
        .runtime
        .read_commit_tree(&dependencies.workspace, commit)
        .map_err(|_| TaskDeliveryEvidenceReason::EvidenceUnavailable)?;
    TaskEvidenceTree {
        head_commit_sha: commit.to_owned(),
        tree_sha: tree.clone(),
        worktree_clean: true,
    }
    .validate()
    .map_err(|_| TaskDeliveryEvidenceReason::EvidenceUnavailable)?;
    if tree.len() != commit.len() {
        return Err(TaskDeliveryEvidenceReason::EvidenceUnavailable);
    }
    Ok(tree)
}

pub(super) fn observe(
    dependencies: &TaskDeliveryEvidenceDependencies,
    record: &PlanningAuthorityDistributorQueueRecord,
    stage: TaskEvidenceDeliveryStage,
    integration: Option<&TaskDeliveryEvidenceIntegration>,
) -> Result<TaskEvidenceDeliverySource, TaskDeliveryEvidenceReason> {
    // Legacy branch/commit fallbacks can still be delivered in Observe, but are never proof.
    if record.source_commit_sha.len() != record.source_base_commit_sha.len() {
        return Err(TaskDeliveryEvidenceReason::EvidenceStale);
    }
    let source = TaskEvidenceDeliverySource {
        base_commit_sha: record.source_base_commit_sha.clone(),
        base_tree_sha: read_tree(dependencies, &record.source_base_commit_sha)?,
        source_commit_sha: record.source_commit_sha.clone(),
        source_tree_sha: read_tree(dependencies, &record.source_commit_sha)?,
        integration: match (stage, integration) {
            (TaskEvidenceDeliveryStage::Integrate, Some(result)) => {
                if record.integration_base_commit_sha.as_deref()
                    != Some(result.base_commit_sha.as_str())
                    || record.integration_commit_sha.as_deref()
                        != Some(result.result_commit_sha.as_str())
                {
                    return Err(TaskDeliveryEvidenceReason::IntegrationSourceMismatch);
                }
                Some(TaskEvidenceIntegrationResult {
                    base_commit_sha: result.base_commit_sha.clone(),
                    result_commit_sha: result.result_commit_sha.clone(),
                    result_tree_sha: read_tree(dependencies, &result.result_commit_sha)?,
                })
            }
            (TaskEvidenceDeliveryStage::Integrate, None) => {
                return Err(TaskDeliveryEvidenceReason::IntegrationSourceMismatch);
            }
            (_, None) => None,
            (_, Some(_)) => return Err(TaskDeliveryEvidenceReason::IntegrationSourceMismatch),
        },
    };
    source
        .validate()
        .map_err(|_| TaskDeliveryEvidenceReason::EvidenceUnavailable)?;
    Ok(source)
}
