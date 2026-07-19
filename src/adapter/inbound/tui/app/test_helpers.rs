use std::process::Command;
use std::sync::Arc;

use anyhow::Result;

use super::{ConversationState, NativeTuiApp, NativeTuiParallelModeBinding};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::application::port::outbound::github_automation_port::{
    GithubAutomationCapabilities, GithubAutomationPort, GithubAutomationPullRequest,
    GithubRepositoryVisibility, credential_redacted_canonical_github_push_url,
    parse_github_repository_identity,
};
use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
use crate::application::port::outbound::parallel_agent_worker_port::{
    NoopParallelAgentWorkerPort, ParallelAgentWorkerPort,
};
use crate::application::port::outbound::planning_authority_port::{
    NoopPlanningAuthorityPort, PlanningAuthorityPort,
};
use crate::application::port::outbound::planning_task_repository_port::{
    NoopPlanningTaskRepositoryPort, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::port::outbound::review_center_repository_port::ReviewCenterRepositoryPort;
use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
use crate::application::port::outbound::startup_probe_port::{
    AppServerStartupContext, StartupProbePort,
};
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::parallel_mode::ParallelModeService;
use crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneComposition;
use crate::application::service::planning::{PlanningRuntimeProjection, PlanningServices};
use crate::application::service::review_center::ReviewCenterReadService;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::conversation::ConversationSnapshot;
use crate::domain::parallel_mode::{
    ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
};
use crate::domain::planning::{
    PriorityQueueProjection, PriorityQueueSkippedTask, PriorityQueueTask, TaskStatus,
};
use crate::domain::recent_sessions::{RecentSessions, SessionCatalog, SessionCatalogRequest};
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

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

pub(crate) fn sample_planning_runtime_projection(
    prompt_fragment: &str,
    queue_summary: &str,
) -> PlanningRuntimeProjection {
    /*
     * This snapshot models the normal ready queue: a head task, another active task, and one blocked
     * task that should appear only in skipped/diagnostic surfaces. Tests can vary prompt and summary
     * copy while preserving a queue shape rich enough for footer, popup, and inline-tail projections.
     */
    let queue_head = sample_queue_head();
    PlanningRuntimeProjection::ready_with_queue_projection(
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

pub(crate) fn test_planning_services(
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
) -> PlanningServices {
    test_planning_services_with_task_repository(
        planning_workspace_port,
        Arc::new(NoopPlanningTaskRepositoryPort),
    )
}

pub(crate) fn test_planning_services_with_task_repository(
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
) -> PlanningServices {
    PlanningServices::from_ports(
        planning_workspace_port,
        Arc::new(NoopPlanningAuthorityPort::default()),
        planning_task_repository_port,
        Arc::new(NoopPlanningWorkerPort),
    )
}

#[derive(Debug, Default)]
struct TestGithubAutomationPort;

fn run_test_git(repo_root: &str, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(["-C", repo_root])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "test git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn test_push_url(repo_root: &str, push_remote: &str) -> Result<String> {
    let configured_url = run_test_git(repo_root, &["remote", "get-url", "--push", push_remote])?;
    Ok(credential_redacted_canonical_github_push_url(&configured_url).unwrap_or(configured_url))
}

fn require_test_delivery_target(
    repo_root: &str,
    push_remote: &str,
    credential_redacted_push_url: &str,
) -> Result<()> {
    let configured_url = test_push_url(repo_root, push_remote)?;
    anyhow::ensure!(
        configured_url == credential_redacted_push_url,
        "test frozen push URL does not match the configured push remote"
    );
    Ok(())
}

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

    fn repository_identity(&self, _repo_root: &str) -> Result<String> {
        Ok("RefinedStone/codex-exec-loop".to_string())
    }

    fn repository_visibility(&self, _repo_root: &str) -> Result<GithubRepositoryVisibility> {
        Ok(GithubRepositoryVisibility::Private)
    }

    fn credential_redacted_push_url_for_remote(
        &self,
        repo_root: &str,
        push_remote: &str,
    ) -> Result<String> {
        test_push_url(repo_root, push_remote)
    }

    fn repository_identity_for_push_url(
        &self,
        repo_root: &str,
        push_remote: &str,
        credential_redacted_push_url: &str,
    ) -> Result<String> {
        require_test_delivery_target(repo_root, push_remote, credential_redacted_push_url)?;
        Ok(
            parse_github_repository_identity(credential_redacted_push_url)
                .unwrap_or_else(|| "RefinedStone/codex-exec-loop".to_string()),
        )
    }

    fn repository_visibility_for_push_url(
        &self,
        repo_root: &str,
        push_remote: &str,
        credential_redacted_push_url: &str,
    ) -> Result<GithubRepositoryVisibility> {
        require_test_delivery_target(repo_root, push_remote, credential_redacted_push_url)?;
        Ok(GithubRepositoryVisibility::Private)
    }

    fn remote_branch_head(
        &self,
        repo_root: &str,
        push_remote: &str,
        branch_name: &str,
    ) -> Result<Option<String>> {
        let push_url = test_push_url(repo_root, push_remote)?;
        self.remote_branch_head_for_delivery_target(repo_root, push_remote, &push_url, branch_name)
    }

    fn remote_branch_head_for_delivery_target(
        &self,
        repo_root: &str,
        push_remote: &str,
        credential_redacted_push_url: &str,
        branch_name: &str,
    ) -> Result<Option<String>> {
        require_test_delivery_target(repo_root, push_remote, credential_redacted_push_url)?;
        let branch_ref = format!("refs/heads/{branch_name}");
        let output = Command::new("git")
            .args(["-C", repo_root, "ls-remote", "--heads"])
            .arg(credential_redacted_push_url)
            .arg(&branch_ref)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()?;
        if !output.status.success() {
            anyhow::bail!(
                "test remote head inspection failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut heads = stdout.lines().filter_map(|line| {
            let (oid, remote_ref) = line.split_once(char::is_whitespace)?;
            (remote_ref.trim() == branch_ref).then(|| oid.trim().to_string())
        });
        let head = heads.next();
        anyhow::ensure!(
            heads.next().is_none(),
            "test remote returned duplicate branch heads"
        );
        Ok(head)
    }

    fn remote_branch_names_for_prefix_for_delivery_target(
        &self,
        repo_root: &str,
        push_remote: &str,
        credential_redacted_push_url: &str,
        branch_prefix: &str,
    ) -> Result<Vec<String>> {
        require_test_delivery_target(repo_root, push_remote, credential_redacted_push_url)?;
        let remote_pattern = format!("refs/heads/{branch_prefix}*");
        let output = Command::new("git")
            .args(["-C", repo_root, "ls-remote", "--heads"])
            .arg(credential_redacted_push_url)
            .arg(&remote_pattern)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()?;
        if !output.status.success() {
            anyhow::bail!(
                "test remote branch listing failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let stdout = String::from_utf8(output.stdout)?;
        stdout
            .lines()
            .map(|line| {
                let (_, remote_ref) = line
                    .split_once(char::is_whitespace)
                    .ok_or_else(|| anyhow::anyhow!("test remote branch row is malformed"))?;
                remote_ref
                    .trim()
                    .strip_prefix("refs/heads/")
                    .map(str::to_string)
                    .ok_or_else(|| anyhow::anyhow!("test remote branch ref is malformed"))
            })
            .collect()
    }

    fn fetch_branch_to_tracking_ref_for_delivery_target(
        &self,
        repo_root: &str,
        push_remote: &str,
        credential_redacted_push_url: &str,
        branch_name: &str,
        tracking_ref: &str,
    ) -> Result<String> {
        require_test_delivery_target(repo_root, push_remote, credential_redacted_push_url)?;
        let expected_tracking_ref = format!("refs/remotes/{push_remote}/{branch_name}");
        anyhow::ensure!(
            tracking_ref == expected_tracking_ref,
            "test fetch target does not match the requested remote branch"
        );
        let refspec = format!("+refs/heads/{branch_name}:{tracking_ref}");
        run_test_git(
            repo_root,
            &[
                "fetch",
                "--quiet",
                "--no-tags",
                credential_redacted_push_url,
                &refspec,
            ],
        )?;
        run_test_git(
            repo_root,
            &["rev-parse", &format!("{tracking_ref}^{{commit}}")],
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

    fn push_integration_branch(
        &self,
        _repo_root: &str,
        _branch_name: &str,
        _expected_old_commit_sha: &str,
    ) -> Result<()> {
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

pub(crate) fn test_parallel_mode_service_with_authority(
    planning_authority: Arc<dyn PlanningAuthorityPort>,
) -> ParallelModeService {
    ParallelModeService::new(
        planning_authority,
        Arc::new(TestGithubAutomationPort),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    )
}

pub(crate) fn test_parallel_mode_control_plane_composition(
    planning: PlanningServices,
) -> ParallelModeControlPlaneComposition {
    test_parallel_mode_control_plane_composition_with_worker(
        test_parallel_mode_service(),
        planning,
        Arc::new(NoopParallelAgentWorkerPort),
    )
}

pub(crate) fn test_parallel_mode_control_plane_composition_with_worker(
    parallel_mode_service: ParallelModeService,
    planning: PlanningServices,
    worker_port: Arc<dyn ParallelAgentWorkerPort>,
) -> ParallelModeControlPlaneComposition {
    ParallelModeControlPlaneComposition::new(parallel_mode_service, planning, worker_port)
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

#[derive(Default)]
struct TestAppServerPort {
    approval_resolution_error: Option<String>,
}

impl StartupProbePort for TestAppServerPort {
    fn load_startup_context(&self) -> Result<AppServerStartupContext> {
        Ok(AppServerStartupContext {
            attachment_profile: TerminalBridgeAttachmentProfile::codex_app_server(),
            initialize_detail: "ok".to_string(),
            account_detail: "ok".to_string(),
            account_ok: true,
            warnings: Vec::new(),
        })
    }
}

impl SessionCatalogPort for TestAppServerPort {
    fn load_session_catalog(&self, _request: SessionCatalogRequest) -> Result<SessionCatalog> {
        Ok(RecentSessions {
            items: Vec::new(),
            warnings: Vec::new(),
            next_cursor: None,
        }
        .into())
    }
}

impl InteractiveTurnRuntimePort for TestAppServerPort {
    fn runtime_control_truth(
        &self,
    ) -> crate::domain::conversation::ConversationRuntimeControlTruth {
        crate::domain::conversation::ConversationRuntimeControlTruth::codex_app_server()
    }

    fn load_conversation_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot> {
        Ok(ConversationSnapshot {
            thread_id: thread_id.to_string(),
            title: "Loaded thread".to_string(),
            cwd: "/tmp/root".to_string(),
            messages: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
            item_lifecycle: Default::default(),
        })
    }

    fn request_stop_all_sessions(&self) -> Result<()> {
        Ok(())
    }

    fn resolve_approval_request(
        &self,
        _approval_id: &str,
        _decision: crate::domain::conversation::ConversationApprovalDecision,
    ) -> Result<()> {
        match &self.approval_resolution_error {
            Some(error) => Err(anyhow::anyhow!(error.clone())),
            None => Ok(()),
        }
    }

    fn run_new_thread_stream(
        &self,
        cwd: &str,
        _prompt: &str,
        _options: crate::domain::conversation::ConversationTurnOptions,
        event_sender: crate::application::service::conversation_runtime_event::ConversationStreamSender,
    ) -> Result<crate::domain::turn_terminal::ConversationTurnTerminalReceipt> {
        crate::application::service::conversation_runtime_event::emit_confirmed_test_terminal_receipt(
            &event_sender,
            "test-thread",
            cwd,
        )
    }

    fn run_turn_stream(
        &self,
        thread_id: &str,
        _prompt: &str,
        _options: crate::domain::conversation::ConversationTurnOptions,
        event_sender: crate::application::service::conversation_runtime_event::ConversationStreamSender,
    ) -> Result<crate::domain::turn_terminal::ConversationTurnTerminalReceipt> {
        crate::application::service::conversation_runtime_event::emit_confirmed_test_terminal_receipt(
            &event_sender,
            thread_id,
            "/tmp/test-workspace",
        )
    }
}

pub(super) fn test_native_tui_app() -> NativeTuiApp {
    test_native_tui_app_with_review_center_repository(Arc::new(
        SqlitePlanningAuthorityAdapter::new(),
    ))
}

pub(super) fn test_native_tui_app_with_approval_resolution_error(
    error: impl Into<String>,
) -> NativeTuiApp {
    let planning = test_planning_services(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));
    test_native_tui_app_with_planning_review_center_and_app_server(
        planning,
        Arc::new(SqlitePlanningAuthorityAdapter::new()),
        None,
        Arc::new(TestAppServerPort {
            approval_resolution_error: Some(error.into()),
        }),
    )
}

pub(super) fn test_native_tui_app_with_review_center_repository(
    review_center_repository: Arc<dyn ReviewCenterRepositoryPort>,
) -> NativeTuiApp {
    let planning = test_planning_services(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));
    test_native_tui_app_with_planning_and_review_center_repository(
        planning,
        review_center_repository,
        None,
    )
}

pub(super) fn test_native_tui_app_with_planning(planning: PlanningServices) -> NativeTuiApp {
    test_native_tui_app_with_planning_and_review_center_repository(
        planning,
        Arc::new(SqlitePlanningAuthorityAdapter::new()),
        None,
    )
}

pub(super) fn test_native_tui_app_with_parallel_mode_composition(
    composition: ParallelModeControlPlaneComposition,
) -> NativeTuiApp {
    test_native_tui_app_with_parallel_mode_binding(
        NativeTuiParallelModeBinding::from_composition(composition),
        Arc::new(SqlitePlanningAuthorityAdapter::new()),
        None,
        Arc::new(TestAppServerPort::default()),
    )
}

pub(super) fn test_native_tui_app_with_session_catalog_port(
    session_catalog_port: Arc<dyn SessionCatalogPort>,
) -> NativeTuiApp {
    test_native_tui_app_with_planning_and_review_center_repository(
        test_planning_services(Arc::new(FilesystemPlanningWorkspaceAdapter::new())),
        Arc::new(SqlitePlanningAuthorityAdapter::new()),
        Some(session_catalog_port),
    )
}

fn test_native_tui_app_with_planning_and_review_center_repository(
    planning: PlanningServices,
    review_center_repository: Arc<dyn ReviewCenterRepositoryPort>,
    session_catalog_port: Option<Arc<dyn SessionCatalogPort>>,
) -> NativeTuiApp {
    test_native_tui_app_with_planning_review_center_and_app_server(
        planning,
        review_center_repository,
        session_catalog_port,
        Arc::new(TestAppServerPort::default()),
    )
}

fn test_native_tui_app_with_planning_review_center_and_app_server(
    planning: PlanningServices,
    review_center_repository: Arc<dyn ReviewCenterRepositoryPort>,
    session_catalog_port: Option<Arc<dyn SessionCatalogPort>>,
    app_server_port: Arc<TestAppServerPort>,
) -> NativeTuiApp {
    test_native_tui_app_with_parallel_mode_binding(
        NativeTuiParallelModeBinding::from_composition(
            test_parallel_mode_control_plane_composition(planning),
        ),
        review_center_repository,
        session_catalog_port,
        app_server_port,
    )
}

fn test_native_tui_app_with_parallel_mode_binding(
    parallel_mode_binding: NativeTuiParallelModeBinding,
    review_center_repository: Arc<dyn ReviewCenterRepositoryPort>,
    session_catalog_port: Option<Arc<dyn SessionCatalogPort>>,
    app_server_port: Arc<TestAppServerPort>,
) -> NativeTuiApp {
    /*
     * Build the same production-shaped service graph used by TUI fixtures, with app-server IO
     * pinned to deterministic responses. Tests can then seed NativeTuiApp state directly while
     * still exercising reducer and lifecycle wiring through the real constructor.
     */
    let conversation_service =
        ConversationService::new(app_server_port.clone()).with_review_center_read_service(
            ReviewCenterReadService::new("/tmp/root", review_center_repository),
        );
    let session_catalog_port = session_catalog_port.unwrap_or_else(|| app_server_port.clone());
    let mut app = NativeTuiApp::new(
        StartupService::new(app_server_port.clone()),
        SessionService::new(session_catalog_port),
        conversation_service,
        parallel_mode_binding,
    );
    app.show_startup_ascii_art = false;
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start with a ready draft conversation");
    };
    conversation.cwd = "/tmp/root".to_string();
    conversation.draft_workspace_directory = "/tmp/root".to_string();
    app.refresh_ready_conversation_planning_runtime_projection();
    app.sync_ready_conversation_planning_runtime_projection(
        PlanningRuntimeProjection::uninitialized(),
    );
    app
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_server_port_fixture_covers_startup_catalog_and_stream_contracts() {
        let app_server_port = TestAppServerPort::default();

        let startup = app_server_port
            .load_startup_context()
            .expect("startup fixture should load");
        assert!(startup.account_ok);
        assert_eq!(startup.initialize_detail, "ok");

        let catalog = app_server_port
            .load_session_catalog(SessionCatalogRequest::for_workspace(10, "/tmp/root"))
            .expect("session catalog fixture should load");
        assert!(
            catalog
                .recent_sessions()
                .expect("ready catalog")
                .items
                .is_empty()
        );

        assert_eq!(
            app_server_port.runtime_control_truth(),
            crate::domain::conversation::ConversationRuntimeControlTruth::codex_app_server()
        );

        let snapshot = app_server_port
            .load_conversation_snapshot("thread-1")
            .expect("conversation fixture should load");
        assert_eq!(snapshot.thread_id, "thread-1");
        assert_eq!(snapshot.cwd, "/tmp/root");

        app_server_port
            .request_stop_all_sessions()
            .expect("stop fixture should succeed");
        let (event_sender, _event_receiver) =
            crate::application::service::conversation_runtime_event::conversation_stream_channel();
        app_server_port
            .run_new_thread_stream(
                "/tmp/root",
                "prompt",
                crate::domain::conversation::ConversationTurnOptions::default(),
                event_sender.clone(),
            )
            .expect("new thread stream fixture should succeed");
        app_server_port
            .run_turn_stream(
                "thread-1",
                "prompt",
                crate::domain::conversation::ConversationTurnOptions::default(),
                event_sender,
            )
            .expect("turn stream fixture should succeed");
    }

    #[test]
    fn native_tui_app_fixture_normalizes_draft_workspace_and_planning_projection() {
        let app = test_native_tui_app();

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("fixture should start with ready draft conversation");
        };
        assert_eq!(conversation.cwd, "/tmp/root");
        assert_eq!(conversation.draft_workspace_directory, "/tmp/root");
        assert!(!app.show_startup_ascii_art);
    }
}
