use std::sync::Arc;

use anyhow::Result;

use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::application::port::outbound::github_automation_port::{
    GithubAutomationCapabilities, GithubAutomationPort, GithubAutomationPullRequest,
};
use crate::application::port::outbound::planning_authority_port::NoopPlanningAuthorityPort;
use crate::application::port::outbound::planning_task_repository_port::NoopPlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::service::parallel_mode::ParallelModeService;
use crate::application::service::planning::{PlanningRuntimeSnapshot, PlanningServices};
use crate::domain::parallel_mode::{
    ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
};
use crate::domain::planning::{
    PriorityQueueProjection, PriorityQueueSkippedTask, PriorityQueueTask, TaskStatus,
};

/*
 * TUI tests need realistic planning and parallel-mode snapshots without booting the full app-server
 * workflow. These helpers create domain/application objects rather than handwritten display strings,
 * so shell rendering, conversation model, and overlay tests exercise the same projection code that
 * production uses after planning services reduce queue state.
 */
pub(crate) fn sample_queue_head() -> PriorityQueueTask {
    /*
     * The sample head is deliberately "ready" and high priority. Many footer and queue assertions
     * look for the actionable queue-head path, so this fixture keeps the happy-path queue vocabulary
     * stable while still carrying direction and rank metadata used by detail panels.
     */
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
    /*
     * This snapshot models the normal ready queue: a head task, another active task, and one blocked
     * task that should appear only in skipped/diagnostic surfaces. Tests can vary prompt and summary
     * copy while preserving a queue shape rich enough for footer, popup, and inline-tail projections.
     */
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
    /*
     * Proposal-only state is distinct from an empty queue: the planning worker has candidate work, but no
     * actionable head. Rendering tests use this to ensure proposal copy does not masquerade as a
     * runnable task and that planning notices still surface without active work.
     */
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

pub(crate) fn test_planning_services(
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
) -> PlanningServices {
    PlanningServices::from_ports(
        planning_workspace_port,
        Arc::new(NoopPlanningAuthorityPort::default()),
        Arc::new(NoopPlanningTaskRepositoryPort),
        Arc::new(NoopPlanningWorkerPort),
    )
}

#[derive(Debug, Default)]
struct TestGithubAutomationPort;

impl GithubAutomationPort for TestGithubAutomationPort {
    /*
     * Parallel-mode tests often care about pool/distributor behavior, not host GitHub tooling.
     * The fake port reports every capability as ready so readiness failures in those tests must
     * come from the scenario under test rather than from missing local gh/push setup.
     */
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

    // Mutating GitHub operations are no-ops because tests assert service state transitions locally.
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
        /*
         * Preserve caller-provided base/head branches in the fake PR. Distributor tests inspect
         * those fields to verify that slot branches and integration targets were wired correctly.
         */
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
        /*
         * Inspection returns a stable open PR for the requested number. The synthetic branch names
         * match the parallel-mode slot vocabulary so roster/detail projections can be exercised
         * without asking GitHub for real PR metadata.
         */
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
    // Default service uses the ready fake GitHub port for tests that are not about GitHub failures.
    test_parallel_mode_service_with_github(Arc::new(TestGithubAutomationPort))
}

pub(crate) fn test_parallel_mode_service_with_github(
    github_automation: Arc<dyn GithubAutomationPort>,
) -> ParallelModeService {
    /*
     * The service still uses the real SQLite authority and Git runtime adapters. That keeps pool
     * reconciliation and planning-authority interactions close to production while letting tests
     * inject only the external GitHub boundary that would otherwise require network/CLI state.
     */
    ParallelModeService::new(
        Arc::new(SqlitePlanningAuthorityAdapter::new()),
        github_automation,
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    )
}
