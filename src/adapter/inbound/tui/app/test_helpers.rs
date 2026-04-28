use std::sync::Arc;

use anyhow::Result;

use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::application::port::outbound::github_automation_port::{
    GithubAutomationCapabilities, GithubAutomationPort, GithubAutomationPullRequest,
};
use crate::application::service::parallel_mode::ParallelModeService;
use crate::application::service::planning::PlanningRuntimeSnapshot;
use crate::domain::parallel_mode::{
    ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
};
use crate::domain::planning::{
    PriorityQueueProjection, PriorityQueueSkippedTask, PriorityQueueTask, TaskStatus,
};

pub(crate) fn sample_queue_head() -> PriorityQueueTask {
    PriorityQueueTask {
        rank: 1,
        task_id: "task-1".to_string(),
        direction_id: "general-workstream".to_string(),
        direction_title: "General workstream".to_string(),
        task_title: "Implement shell planning status".to_string(),
        status: TaskStatus::Ready,
        combined_priority: 10,
        updated_at: "2026-04-10T00:00:00Z".to_string(),
        rank_reasons: vec!["status=ready".to_string()],
    }
}

pub(crate) fn sample_planning_runtime_snapshot(
    prompt_fragment: &str,
    queue_summary: &str,
) -> PlanningRuntimeSnapshot {
    let queue_head = sample_queue_head();
    PlanningRuntimeSnapshot::ready_with_queue_projection(
        prompt_fragment.to_string(),
        queue_summary.to_string(),
        None,
        Some(queue_head.clone()),
        PriorityQueueProjection {
            next_task: Some(queue_head.clone()),
            active_tasks: vec![
                queue_head,
                PriorityQueueTask {
                    rank: 2,
                    task_id: "task-2".to_string(),
                    direction_id: "general-workstream".to_string(),
                    direction_title: "General workstream".to_string(),
                    task_title: "Trim legacy shell code".to_string(),
                    status: TaskStatus::Ready,
                    combined_priority: 8,
                    updated_at: "2026-04-10T01:00:00Z".to_string(),
                    rank_reasons: vec!["status=ready".to_string()],
                },
            ],
            proposed_tasks: Vec::new(),
            skipped_tasks: vec![PriorityQueueSkippedTask {
                task_id: "task-blocked-1".to_string(),
                task_title: "Follow blocked review thread".to_string(),
                direction_id: "general-workstream".to_string(),
                status: TaskStatus::Blocked,
                reason: "blocked by tasks: task-2(in_progress)".to_string(),
            }],
        },
    )
}

pub(crate) fn sample_proposal_only_planning_runtime_snapshot(
    prompt_fragment: &str,
    queue_summary: &str,
    proposal_summary: &str,
) -> PlanningRuntimeSnapshot {
    PlanningRuntimeSnapshot::ready_with_queue_projection(
        prompt_fragment.to_string(),
        queue_summary.to_string(),
        Some(proposal_summary.to_string()),
        None,
        PriorityQueueProjection {
            next_task: None,
            active_tasks: Vec::new(),
            proposed_tasks: vec![PriorityQueueTask {
                rank: 1,
                task_id: "proposal-1".to_string(),
                direction_id: "general-workstream".to_string(),
                direction_title: "General workstream".to_string(),
                task_title: "Draft a queue inspection overlay".to_string(),
                status: TaskStatus::Proposed,
                combined_priority: 7,
                updated_at: "2026-04-10T02:00:00Z".to_string(),
                rank_reasons: vec!["combined_priority=7".to_string()],
            }],
            skipped_tasks: Vec::new(),
        },
    )
}

#[derive(Debug, Default)]
struct TestGithubAutomationPort;

impl GithubAutomationPort for TestGithubAutomationPort {
    fn inspect_capabilities(&self, _repo_root: &str) -> GithubAutomationCapabilities {
        GithubAutomationCapabilities::new(
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::PushRemote,
                ParallelModeCapabilityState::Ready,
                "test push remote ready",
                None,
            ),
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GhBinary,
                ParallelModeCapabilityState::Ready,
                "test gh binary ready",
                None,
            ),
            ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GhAuth,
                ParallelModeCapabilityState::Ready,
                "test gh auth ready",
                None,
            ),
        )
    }

    fn push_branch(
        &self,
        _repo_root: &str,
        _branch_name: &str,
        _force_with_lease: bool,
    ) -> Result<()> {
        Ok(())
    }

    fn ensure_pull_request(
        &self,
        _repo_root: &str,
        base_branch: &str,
        head_branch: &str,
        _title: &str,
        _body: &str,
    ) -> Result<GithubAutomationPullRequest> {
        Ok(GithubAutomationPullRequest::new(
            1,
            "https://github.com/RefinedStone/codex-exec-loop/pull/1",
            "OPEN",
            base_branch,
            head_branch,
            false,
        ))
    }

    fn inspect_pull_request(
        &self,
        _repo_root: &str,
        pr_number: u64,
    ) -> Result<GithubAutomationPullRequest> {
        Ok(GithubAutomationPullRequest::new(
            pr_number,
            format!("https://github.com/RefinedStone/codex-exec-loop/pull/{pr_number}"),
            "OPEN",
            "akra",
            "akra-agent/slot-1/task",
            false,
        ))
    }

    fn push_integration_branch(&self, _repo_root: &str, _branch_name: &str) -> Result<()> {
        Ok(())
    }

    fn close_pull_request(&self, _repo_root: &str, _pr_number: u64) -> Result<()> {
        Ok(())
    }
}

pub(crate) fn test_parallel_mode_service() -> ParallelModeService {
    test_parallel_mode_service_with_github(Arc::new(TestGithubAutomationPort))
}

pub(crate) fn test_parallel_mode_service_with_github(
    github_automation: Arc<dyn GithubAutomationPort>,
) -> ParallelModeService {
    ParallelModeService::new(
        Arc::new(SqlitePlanningAuthorityAdapter::new()),
        github_automation,
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    )
}
