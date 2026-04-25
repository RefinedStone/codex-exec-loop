use super::distributor::load_distributor_queue_records;
use super::{
    DEFAULT_POOL_SIZE, MAX_AGENT_BRANCH_SLUG_LEN, ParallelModeCapabilityKey,
    ParallelModeCapabilitySnapshot, ParallelModeCapabilityState, ParallelModeReadinessSnapshot,
    ParallelModeReadinessState, ParallelModeService, ParallelModeSupervisorState,
    agent_session_detail_record_path, allocate_agent_branch_name, build_pool_board,
    derive_default_pool_root, derive_readiness, detect_canonical_repo_root, lease_session_key,
    parse_https_remote, read_agent_session_detail_record, reconcile_pool_board,
    resolve_workspace_slot_lease, run_command, sanitize_task_slug, short_branch_slug_hash,
    short_sha, slot_id, slot_lease_file_path,
};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::application::port::outbound::github_automation_port::{
    GithubAutomationCapabilities, GithubAutomationPort, GithubAutomationPullRequest,
};
use crate::application::service::planning::PlanningRuntimeSnapshot;
use crate::application::service::planning::shared::contract::DIRECTIONS_FILE_PATH;
use crate::domain::parallel_mode::{
    ParallelModePoolSlotState, ParallelModeQueueItemState, ParallelModeSlotLeaseRequest,
    ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct TempGitRepo {
    root: PathBuf,
    repo_root: PathBuf,
}

impl TempGitRepo {
    fn canonical_repo_root(&self) -> PathBuf {
        fs::canonicalize(&self.repo_root).unwrap_or_else(|_| self.repo_root.clone())
    }

    fn new(prefix: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("parallel-mode-{prefix}-{unique}"));
        let repo_root = root.join("repo");
        fs::create_dir_all(&repo_root).expect("temp repo root should be created");

        run_git(&repo_root, &["init", "-q"]);
        run_git(&repo_root, &["config", "user.name", "RefinedStone"]);
        run_git(
            &repo_root,
            &["config", "user.email", "chem.en.9273@gmail.com"],
        );
        fs::write(repo_root.join("README.md"), "seed\n").expect("seed file should write");
        fs::write(repo_root.join(".gitignore"), "*.tmp\n").expect("gitignore should write");
        run_git(&repo_root, &["add", "README.md"]);
        run_git(&repo_root, &["add", ".gitignore"]);
        run_git(&repo_root, &["commit", "-qm", "init"]);
        run_git(&repo_root, &["branch", "akra"]);

        Self { root, repo_root }
    }

    fn workspace_dir(&self) -> String {
        self.canonical_repo_root().display().to_string()
    }

    fn pool_root(&self) -> PathBuf {
        derive_default_pool_root(&self.canonical_repo_root())
    }

    fn slot_lease_path(&self, slot_number: usize) -> PathBuf {
        slot_lease_file_path(&self.pool_root(), &slot_id(slot_number))
    }

    fn session_detail_path(&self, session_key: &str) -> PathBuf {
        agent_session_detail_record_path(&self.pool_root(), session_key)
    }

    fn distributor_queue_path(&self, queue_item_id: &str) -> PathBuf {
        self.pool_root()
            .join(".distributor-queue")
            .join(format!("{queue_item_id}.json"))
    }

    fn read_slot_lease(&self, slot_number: usize) -> ParallelModeSlotLeaseSnapshot {
        let lease_body = fs::read_to_string(self.slot_lease_path(slot_number))
            .expect("slot lease should be readable");
        serde_json::from_str(&lease_body).expect("slot lease should deserialize")
    }

    fn create_detached_slot(&self, slot_number: usize) -> PathBuf {
        let slot_path = self.pool_root().join(slot_id(slot_number));
        fs::create_dir_all(
            slot_path
                .parent()
                .expect("slot path should have a parent directory"),
        )
        .expect("pool root should be created");
        run_git(
            &self.repo_root,
            &[
                "worktree",
                "add",
                "--detach",
                slot_path.to_str().expect("slot path should be valid utf-8"),
                "akra",
            ],
        );
        slot_path
    }

    fn create_agent_slot(&self, slot_number: usize, task_slug: &str) -> PathBuf {
        let slot_path = self.pool_root().join(slot_id(slot_number));
        fs::create_dir_all(
            slot_path
                .parent()
                .expect("slot path should have a parent directory"),
        )
        .expect("pool root should be created");
        let branch_name = format!("akra-agent/{}/{}", slot_id(slot_number), task_slug);
        run_git(
            &self.repo_root,
            &[
                "worktree",
                "add",
                "-b",
                branch_name.as_str(),
                slot_path.to_str().expect("slot path should be valid utf-8"),
                "akra",
            ],
        );
        slot_path
    }

    fn create_linked_worktree(&self, branch_name: &str) -> PathBuf {
        let slug = branch_name.replace('/', "-");
        let worktree_path = self.root.join("linked-worktrees").join(slug);
        fs::create_dir_all(
            worktree_path
                .parent()
                .expect("worktree path should have a parent directory"),
        )
        .expect("linked worktree parent should exist");
        run_git(
            &self.repo_root,
            &[
                "worktree",
                "add",
                "-b",
                branch_name,
                worktree_path
                    .to_str()
                    .expect("worktree path should be valid utf-8"),
            ],
        );
        worktree_path
    }

    fn delete_local_akra_branch(&self) {
        run_git(&self.repo_root, &["branch", "-D", "akra"]);
    }

    fn commit_file_in_slot(
        &self,
        slot_path: &Path,
        file_name: &str,
        contents: &str,
        message: &str,
    ) {
        fs::write(slot_path.join(file_name), contents).expect("slot file should be written");
        run_git(slot_path, &["add", file_name]);
        run_git(slot_path, &["commit", "-qm", message]);
    }

    fn merge_agent_slot_into_akra(&self, slot_path: &Path) {
        let branch_name = current_branch(slot_path);
        let original_branch = current_branch(&self.repo_root);
        run_git(&self.repo_root, &["checkout", "akra"]);
        run_git(
            &self.repo_root,
            &["merge", "--ff-only", branch_name.as_str()],
        );
        run_git(&self.repo_root, &["checkout", original_branch.as_str()]);
    }

    fn branch_exists(&self, branch_name: &str) -> bool {
        let output = Command::new("git")
            .current_dir(&self.repo_root)
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch_name}"),
            ])
            .env("GIT_TERMINAL_PROMPT", "0")
            .status()
            .expect("git show-ref should spawn");
        output.success()
    }

    fn head_sha(&self) -> String {
        run_command(
            "git",
            [
                "-C",
                self.repo_root
                    .to_str()
                    .expect("repo root should be valid utf-8"),
                "rev-parse",
                "HEAD",
            ],
            None,
        )
        .expect("head sha should resolve")
    }

    fn set_remote_tracking_branch(&self, branch_name: &str, target: &str) {
        run_git(
            &self.repo_root,
            &["update-ref", &format!("refs/remotes/{branch_name}"), target],
        );
    }

    fn commit_on_current_branch(&self, file_name: &str, contents: &str, message: &str) {
        fs::write(self.repo_root.join(file_name), contents).expect("repo file should write");
        run_git(&self.repo_root, &["add", file_name]);
        run_git(&self.repo_root, &["commit", "-qm", message]);
    }
}

impl Drop for TempGitRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn run_git(repo_root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git command should spawn");
    assert!(
        output.status.success(),
        "git command should succeed: git {:?}\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn current_branch(repo_root: &Path) -> String {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git rev-parse should spawn");
    assert!(
        output.status.success(),
        "git rev-parse should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("branch name should be utf-8")
        .trim()
        .to_string()
}

fn sample_lease_request(
    task_id: &str,
    task_title: &str,
    agent_id: &str,
    task_slug: &str,
) -> ParallelModeSlotLeaseRequest {
    ParallelModeSlotLeaseRequest::new(task_id, task_title, agent_id, task_slug)
}

#[derive(Debug, Clone)]
struct FakeGithubAutomationPort {
    capabilities: GithubAutomationCapabilities,
    ensured_pull_request: GithubAutomationPullRequest,
    base_branch: Arc<Mutex<Option<String>>>,
    head_branch: Arc<Mutex<Option<String>>>,
    operations: Arc<Mutex<Vec<String>>>,
    force_push_error: Arc<Mutex<Option<String>>>,
}

impl FakeGithubAutomationPort {
    fn ready() -> Self {
        Self {
            capabilities: GithubAutomationCapabilities::new(
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
            ),
            ensured_pull_request: GithubAutomationPullRequest::new(
                77,
                "https://github.com/RefinedStone/codex-exec-loop/pull/77",
                "OPEN",
                "akra",
                "placeholder",
                false,
            ),
            base_branch: Arc::new(Mutex::new(None)),
            head_branch: Arc::new(Mutex::new(None)),
            operations: Arc::new(Mutex::new(Vec::new())),
            force_push_error: Arc::new(Mutex::new(None)),
        }
    }

    fn with_capabilities(capabilities: GithubAutomationCapabilities) -> Self {
        Self {
            capabilities,
            ..Self::ready()
        }
    }

    fn with_force_push_error(error: &str) -> Self {
        let github = Self::ready();
        *github
            .force_push_error
            .lock()
            .expect("fake github force-push error mutex poisoned") = Some(error.to_string());
        github
    }
}

impl GithubAutomationPort for FakeGithubAutomationPort {
    fn inspect_capabilities(&self, _repo_root: &str) -> GithubAutomationCapabilities {
        self.capabilities.clone()
    }

    fn push_branch(
        &self,
        _repo_root: &str,
        branch_name: &str,
        force_with_lease: bool,
    ) -> anyhow::Result<()> {
        self.operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .push(format!("push:{branch_name}:{force_with_lease}"));
        if force_with_lease
            && let Some(error) = self
                .force_push_error
                .lock()
                .expect("fake github force-push error mutex poisoned")
                .clone()
        {
            anyhow::bail!(error);
        }
        Ok(())
    }

    fn ensure_pull_request(
        &self,
        _repo_root: &str,
        base_branch: &str,
        head_branch: &str,
        _title: &str,
        _body: &str,
    ) -> anyhow::Result<GithubAutomationPullRequest> {
        *self
            .base_branch
            .lock()
            .expect("fake github base branch mutex poisoned") = Some(base_branch.to_string());
        *self
            .head_branch
            .lock()
            .expect("fake github head branch mutex poisoned") = Some(head_branch.to_string());
        self.operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .push(format!("ensure-pr:{base_branch}:{head_branch}"));
        Ok(GithubAutomationPullRequest::new(
            self.ensured_pull_request.number,
            self.ensured_pull_request.url.clone(),
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
    ) -> anyhow::Result<GithubAutomationPullRequest> {
        let base_branch = self
            .base_branch
            .lock()
            .expect("fake github base branch mutex poisoned")
            .clone()
            .unwrap_or_else(|| "akra".to_string());
        let head_branch = self
            .head_branch
            .lock()
            .expect("fake github head branch mutex poisoned")
            .clone()
            .unwrap_or_else(|| self.ensured_pull_request.head_branch.clone());
        self.operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .push(format!("inspect-pr:{pr_number}"));
        Ok(GithubAutomationPullRequest::new(
            pr_number,
            format!("https://github.com/RefinedStone/codex-exec-loop/pull/{pr_number}"),
            "OPEN",
            base_branch,
            head_branch,
            false,
        ))
    }

    fn push_integration_branch(&self, _repo_root: &str, branch_name: &str) -> anyhow::Result<()> {
        self.operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .push(format!("push-integration:{branch_name}"));
        Ok(())
    }

    fn close_pull_request(&self, _repo_root: &str, pr_number: u64) -> anyhow::Result<()> {
        self.operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .push(format!("close-pr:{pr_number}"));
        Ok(())
    }
}

fn test_parallel_mode_service() -> ParallelModeService {
    ParallelModeService::new(
        Arc::new(SqlitePlanningAuthorityAdapter::new()),
        Arc::new(FakeGithubAutomationPort::ready()),
    )
}

fn test_parallel_mode_service_with_github(
    github: Arc<dyn GithubAutomationPort>,
) -> ParallelModeService {
    ParallelModeService::new(Arc::new(SqlitePlanningAuthorityAdapter::new()), github)
}

#[test]
fn derive_readiness_marks_blocked_when_any_blocker_exists() {
    let readiness = derive_readiness(&[
        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GitRepository,
            ParallelModeCapabilityState::Ready,
            "ready",
            None,
        ),
        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::Planning,
            ParallelModeCapabilityState::Blocked,
            "planning invalid",
            Some("repair planning".to_string()),
        ),
    ]);

    assert_eq!(readiness, ParallelModeReadinessState::Blocked);
}

#[test]
fn derive_readiness_marks_degraded_when_only_optional_capabilities_fail() {
    let readiness = derive_readiness(&[
        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GitRepository,
            ParallelModeCapabilityState::Ready,
            "ready",
            None,
        ),
        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::PushRemote,
            ParallelModeCapabilityState::Degraded,
            "push unavailable",
            Some("restore auth".to_string()),
        ),
    ]);

    assert_eq!(readiness, ParallelModeReadinessState::Degraded);
}

#[test]
fn parse_https_remote_extracts_host_and_path() {
    assert_eq!(
        parse_https_remote("https://github.com/RefinedStone/codex-exec-loop.git"),
        Some((
            "github.com".to_string(),
            "RefinedStone/codex-exec-loop.git".to_string()
        ))
    );
    assert_eq!(
        parse_https_remote("git@github.com:RefinedStone/codex-exec-loop.git"),
        None
    );
}

#[test]
fn build_supervisor_snapshot_marks_prepare_when_mode_is_off() {
    let service = test_parallel_mode_service();
    let snapshot = service.build_supervisor_snapshot("/tmp/root", false, None);

    assert_eq!(snapshot.state, ParallelModeSupervisorState::Prepare);
    assert_eq!(snapshot.pool.configured_size, DEFAULT_POOL_SIZE);
    assert_eq!(snapshot.roster.active_count(), 0);
    assert_eq!(snapshot.distributor.head_summary, "inactive");
}

#[test]
fn build_supervisor_snapshot_uses_recover_when_mode_enabled_but_blocked() {
    let service = test_parallel_mode_service();
    let readiness = ParallelModeReadinessSnapshot::new(
        "/tmp/root",
        ParallelModeReadinessState::Blocked,
        vec![],
        Some("planning: blocked".to_string()),
    );

    let snapshot = service.build_supervisor_snapshot("/tmp/root", true, Some(&readiness));

    assert_eq!(snapshot.state, ParallelModeSupervisorState::Recover);
    assert_eq!(snapshot.pool.unavailable_slots, DEFAULT_POOL_SIZE);
    assert_eq!(snapshot.distributor.head_summary, "paused");
}

#[test]
fn build_supervisor_snapshot_populates_roster_from_live_slot_leases() {
    let repo = TempGitRepo::new("supervisor-roster-starting");
    let service = test_parallel_mode_service();
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    let entry = snapshot
        .roster
        .entries
        .first()
        .expect("roster should contain the leased agent");

    assert_eq!(snapshot.roster.active_count(), 1);
    assert_eq!(entry.agent_id, "agent-1");
    assert_eq!(entry.task_title, "Task One");
    assert_eq!(entry.slot_id, "slot-1");
    assert_eq!(entry.branch_name, lease.branch_name);
    assert_eq!(entry.state_label, "starting");
    assert_eq!(entry.duration_label, "launch pending");
    assert_eq!(
        entry.latest_summary,
        "slot lease acquired and branch reserved for launch"
    );
}

#[test]
fn build_supervisor_snapshot_reads_store_backed_runtime_projections_after_mirror_loss() {
    let repo = TempGitRepo::new("supervisor-store-recovery");
    let service = test_parallel_mode_service();
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot should transition to running");

    let session_key = lease_session_key(&lease);
    fs::remove_file(repo.slot_lease_path(1)).expect("slot lease mirror should be removed");
    fs::remove_file(repo.session_detail_path(&session_key))
        .expect("session detail mirror should be removed");

    let recovered = test_parallel_mode_service();
    let snapshot =
        recovered.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));

    assert_eq!(snapshot.pool.running_slots, 1);
    assert_eq!(snapshot.roster.active_count(), 1);
    assert_eq!(snapshot.roster.entries[0].state_label, "running");
    assert_eq!(
        snapshot
            .detail
            .session
            .as_ref()
            .expect("session detail should be recovered from the authority store")
            .session_key,
        session_key
    );
}

#[test]
fn build_supervisor_snapshot_projects_running_and_cleanup_pending_roster_states() {
    let repo = TempGitRepo::new("supervisor-roster-lifecycle");
    let service = test_parallel_mode_service();
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());

    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");
    let running_snapshot =
        service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    let running_entry = running_snapshot
        .roster
        .entries
        .first()
        .expect("running roster entry should exist");
    assert_eq!(running_entry.state_label, "running");
    assert_ne!(running_entry.duration_label, "launch pending");
    assert_eq!(
        running_entry.latest_summary,
        "agent session entered the running state"
    );

    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    repo.merge_agent_slot_into_akra(&slot_path);
    service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to cleanup pending");
    let cleanup_snapshot =
        service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    let cleanup_entry = cleanup_snapshot
        .roster
        .entries
        .first()
        .expect("cleanup-pending roster entry should exist");
    assert_eq!(cleanup_entry.state_label, "cleanup_pending");
    assert_eq!(cleanup_entry.duration_label, "complete");
    assert_eq!(
        cleanup_entry.latest_summary,
        "agent branch is merged into akra and awaiting slot cleanup"
    );
}

#[test]
fn build_supervisor_snapshot_populates_detail_with_live_session_history() {
    let repo = TempGitRepo::new("supervisor-detail-live");
    let service = test_parallel_mode_service();
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    service
        .record_workspace_slot_thread_prepared(&lease.worktree_path, "thread-42")
        .expect("thread prepared should be recorded");
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");

    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    let detail = snapshot
        .detail
        .session
        .as_ref()
        .expect("detail should select the live agent session");

    assert_eq!(detail.agent_id, "agent-1");
    assert_eq!(detail.task_id, "task-1");
    assert_eq!(detail.thread_id.as_deref(), Some("thread-42"));
    assert_eq!(detail.state_label, "running");
    assert_eq!(detail.completion_state_label, "in_progress");
    assert_eq!(
        detail
            .history
            .iter()
            .map(|entry| entry.state_label.as_str())
            .collect::<Vec<_>>(),
        vec!["assigned", "starting", "running"]
    );
}

#[test]
fn build_supervisor_snapshot_keeps_cleaned_session_detail_after_slot_return() {
    let repo = TempGitRepo::new("supervisor-detail-cleaned");
    let service = test_parallel_mode_service();
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());

    service
        .record_workspace_slot_thread_prepared(&lease.worktree_path, "thread-77")
        .expect("thread prepared should be recorded");
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    repo.merge_agent_slot_into_akra(&slot_path);
    service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to cleanup pending");
    service
        .cleanup_workspace_slot_if_pending(&lease.worktree_path)
        .expect("cleanup should succeed")
        .expect("cleanup should return the cleaned lease");

    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    let detail = snapshot
        .detail
        .session
        .as_ref()
        .expect("detail should keep the last cleaned session");

    assert_eq!(snapshot.roster.active_count(), 0);
    assert_eq!(detail.thread_id.as_deref(), Some("thread-77"));
    assert_eq!(detail.state_label, "cleaned");
    assert_eq!(detail.completion_state_label, "cleaned");
    assert_eq!(
        detail.distributor_outcome.as_deref(),
        Some("branch merged into akra and the slot returned to idle")
    );
    assert_eq!(
        detail
            .history
            .iter()
            .map(|entry| entry.state_label.as_str())
            .collect::<Vec<_>>(),
        vec![
            "assigned",
            "starting",
            "running",
            "merged",
            "cleanup_pending",
            "cleaned"
        ]
    );
}

#[test]
fn build_supervisor_snapshot_projects_official_completion_and_commit_ready_states() {
    let repo = TempGitRepo::new("supervisor-official-completion");
    let service = test_parallel_mode_service();
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    service
        .record_workspace_slot_thread_prepared(&lease.worktree_path, "thread-88")
        .expect("thread prepared should be recorded");
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    let completion_report = service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-1",
            None,
            Some("Implemented official completion lifecycle."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be recorded")
        .expect("official completion contract should be returned");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("ledger refreshing state should be recorded");
    service
        .mark_workspace_commit_ready(
            &lease.worktree_path,
            "official ledger refresh succeeded: follow-up queued",
        )
        .expect("commit-ready state should be recorded");

    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    let roster_entry = snapshot
        .roster
        .entries
        .first()
        .expect("roster entry should exist");
    let detail = snapshot
        .detail
        .session
        .as_ref()
        .expect("detail should exist");

    assert_eq!(roster_entry.state_label, "commit_ready");
    assert_eq!(detail.state_label, "commit_ready");
    assert_eq!(detail.completion_state_label, "commit_ready");
    assert_eq!(completion_report.root_turn_id, "turn-1");
    assert_eq!(completion_report.refresh_order, 1);
    assert_eq!(completion_report.completion.task_id, "task-1");
    assert_eq!(completion_report.completion.agent_id, "agent-1");
    assert_eq!(snapshot.distributor.head_summary, "official");
    assert_eq!(
        snapshot.distributor.completion_feed[0].summary,
        "Implemented official completion lifecycle."
    );
    assert_eq!(
        snapshot.distributor.completion_feed[2].summary,
        "official ledger refresh accepted the completion report"
    );
    assert_eq!(
        detail
            .history
            .iter()
            .map(|entry| entry.state_label.as_str())
            .collect::<Vec<_>>(),
        vec![
            "assigned",
            "starting",
            "running",
            "reported_complete",
            "ledger_refreshing",
            "commit_ready"
        ]
    );
}

#[test]
fn process_distributor_queue_delivers_commit_ready_result_into_akra_and_cleans_slot() {
    let repo = TempGitRepo::new("distributor-queue-success");
    let github = FakeGithubAutomationPort::ready();
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    service
        .record_workspace_slot_thread_prepared(&lease.worktree_path, "thread-42")
        .expect("thread prepared should be recorded");
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-queue-success",
            None,
            Some("Distributor queue wiring completed."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("ledger refreshing should be recorded");
    service
        .mark_workspace_commit_ready(
            &lease.worktree_path,
            "official ledger refresh succeeded: distributor delivery approved",
        )
        .expect("commit-ready should be recorded");

    let queued_item = service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect("commit-ready result should be enqueued")
        .expect("queue item should be created");
    assert_eq!(queued_item.queue_state, ParallelModeQueueItemState::Queued);

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("distributor queue should process");
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("distributor integrated queue head into akra"))
    );
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("distributor returned slot to idle"))
    );

    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    let detail = snapshot
        .detail
        .session
        .as_ref()
        .expect("cleaned session detail should remain available");

    assert_eq!(snapshot.roster.active_count(), 0);
    assert_eq!(snapshot.distributor.head_summary, "idle");
    assert!(
        snapshot.distributor.completion_feed[3]
            .summary
            .contains("akra"),
        "merge-queued feed should reflect distributor integration: {}",
        snapshot.distributor.completion_feed[3].summary
    );
    assert_eq!(
        snapshot.distributor.completion_feed[4].summary,
        "slot cleaned and returned to the idle pool"
    );
    assert_eq!(detail.state_label, "cleaned");
    assert_eq!(
        detail
            .history
            .iter()
            .map(|entry| entry.state_label.as_str())
            .collect::<Vec<_>>(),
        vec![
            "assigned",
            "starting",
            "running",
            "reported_complete",
            "ledger_refreshing",
            "commit_ready",
            "merge_queued",
            "pushing",
            "pr_pending",
            "merge_pending",
            "integrating",
            "merged",
            "cleanup_pending",
            "cleaned"
        ]
    );
    assert_eq!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .clone(),
        vec![
            format!("push:{}:false", lease.branch_name),
            format!("ensure-pr:akra:{}", lease.branch_name),
            "inspect-pr:77".to_string(),
            "push-integration:akra".to_string(),
            "inspect-pr:77".to_string(),
            "close-pr:77".to_string(),
        ]
    );
    assert!(!repo.slot_lease_path(1).exists());
    assert!(!repo.branch_exists(&lease.branch_name));
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.workspace_dir().as_str(),
                "show",
                "akra:feature.txt",
            ],
            None,
        )
        .as_deref(),
        Some("done")
    );
}

#[test]
fn build_supervisor_snapshot_prefers_active_distributor_queue_head_for_selected_detail() {
    let repo = TempGitRepo::new("distributor-detail-selection");
    let service = test_parallel_mode_service();
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    let queued = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("queue-head slot lease should be acquired");
    let queued_slot_path = PathBuf::from(queued.worktree_path.clone());
    service
        .record_workspace_slot_thread_prepared(&queued.worktree_path, "thread-queue")
        .expect("queue-head thread prepared should be recorded");
    service
        .mark_workspace_slot_running(&queued.worktree_path)
        .expect("queue-head slot should transition to running");
    repo.commit_file_in_slot(&queued_slot_path, "queued.txt", "done\n", "queue head work");
    service
        .begin_workspace_official_completion(
            &queued.worktree_path,
            "turn-queue-head",
            None,
            Some("Queued result is waiting for distributor delivery."),
            Some("cargo test passed"),
            None,
        )
        .expect("queue-head official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&queued.worktree_path)
        .expect("queue-head ledger refreshing should be recorded");
    service
        .mark_workspace_commit_ready(
            &queued.worktree_path,
            "official ledger refresh succeeded: distributor delivery approved",
        )
        .expect("queue-head commit-ready should be recorded");
    service
        .enqueue_workspace_commit_ready_result(&queued.worktree_path)
        .expect("queue-head result should enqueue")
        .expect("queue-head item should be created");

    thread::sleep(Duration::from_millis(10));

    let running = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-2", "Task Two", "agent-2", "task-two"),
        )
        .expect("second slot lease should be acquired");
    service
        .record_workspace_slot_thread_prepared(&running.worktree_path, "thread-running")
        .expect("second thread prepared should be recorded");
    service
        .mark_workspace_slot_running(&running.worktree_path)
        .expect("second slot should transition to running");

    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    let detail = snapshot
        .detail
        .session
        .as_ref()
        .expect("selected detail should exist");

    assert_eq!(snapshot.distributor.head_summary, "queued");
    assert_eq!(snapshot.distributor.queue_depth(), 1);
    assert_eq!(snapshot.distributor.queue_items[0].source_agent, "agent-1");
    assert_eq!(detail.agent_id, "agent-1");
    assert_eq!(detail.task_id, "task-1");
    assert_eq!(detail.thread_id.as_deref(), Some("thread-queue"));
    assert_eq!(detail.state_label, "merge_queued");
    assert_eq!(
        detail.distributor_outcome.as_deref(),
        Some("distributor accepted the result and queued it for GitHub delivery")
    );
}

#[test]
fn distributor_queue_blocks_after_push_when_github_automation_is_unavailable() {
    let repo = TempGitRepo::new("distributor-queue-gh-blocked");
    let github = FakeGithubAutomationPort::with_capabilities(GithubAutomationCapabilities::new(
        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::PushRemote,
            ParallelModeCapabilityState::Ready,
            "test push remote ready",
            None,
        ),
        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GhBinary,
            ParallelModeCapabilityState::Degraded,
            "gh is missing in this test",
            Some("install gh".to_string()),
        ),
        ParallelModeCapabilitySnapshot::new(
            ParallelModeCapabilityKey::GhAuth,
            ParallelModeCapabilityState::Degraded,
            "gh auth cannot run without gh",
            Some("restore gh".to_string()),
        ),
    ));
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-gh-blocked",
            None,
            Some("Distributor queue wiring completed."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("ledger refreshing should be recorded");
    service
        .mark_workspace_commit_ready(
            &lease.worktree_path,
            "official ledger refresh succeeded: distributor delivery approved",
        )
        .expect("commit-ready should be recorded");
    service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect("commit-ready result should be enqueued")
        .expect("queue item should be created");

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("distributor queue should process");
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("GitHub automation is unavailable"))
    );
    assert_eq!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .clone(),
        vec![format!("push:{}:false", lease.branch_name)]
    );

    let queue_records = load_distributor_queue_records(&repo.pool_root());
    assert_eq!(queue_records.len(), 1);
    assert_eq!(
        queue_records[0].queue_state,
        ParallelModeQueueItemState::Blocked
    );
    assert!(
        queue_records[0]
            .integration_note
            .contains("GitHub automation is unavailable")
    );
    assert!(repo.slot_lease_path(1).exists());

    let detail = read_agent_session_detail_record(&repo.pool_root(), &lease_session_key(&lease))
        .expect("session detail should be persisted");
    assert_eq!(detail.state_label, "failed");
    assert_eq!(
        detail
            .history
            .iter()
            .map(|entry| entry.state_label.as_str())
            .collect::<Vec<_>>(),
        vec![
            "assigned",
            "running",
            "reported_complete",
            "ledger_refreshing",
            "commit_ready",
            "merge_queued",
            "pushing",
            "failed"
        ]
    );
}

#[test]
fn build_supervisor_snapshot_does_not_trigger_runtime_recovery_side_effects() {
    let repo = TempGitRepo::new("snapshot-no-recovery");
    let github = Arc::new(FakeGithubAutomationPort::ready());
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(github);
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-snapshot",
            None,
            Some("Snapshot render should stay read-only."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("ledger refreshing should be recorded");
    service
        .mark_workspace_commit_ready(
            &lease.worktree_path,
            "official ledger refresh succeeded: distributor delivery approved",
        )
        .expect("commit-ready should be recorded");
    service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect("commit-ready result should enqueue")
        .expect("queue item should be created");

    let mut queue_record = load_distributor_queue_records(&repo.pool_root())
        .into_iter()
        .next()
        .expect("queued record should exist");
    queue_record.queue_state = ParallelModeQueueItemState::MergePending;
    queue_record.pull_request_number = Some(77);
    queue_record.pull_request_url =
        Some("https://github.com/RefinedStone/codex-exec-loop/pull/77".to_string());
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &repo.workspace_dir(),
        &queue_record,
    )
    .expect("queue record should update");

    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));

    assert_eq!(snapshot.distributor.head_summary, "merge pending");
    assert!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .is_empty(),
        "snapshot rendering should not invoke GitHub recovery work"
    );
}

#[test]
fn reserved_official_completion_orders_survive_out_of_order_worker_start() {
    let repo = TempGitRepo::new("official-completion-refresh-order");
    let service = test_parallel_mode_service();

    let first = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("first slot lease should be acquired");
    let second = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-2", "Task Two", "agent-2", "task-two"),
        )
        .expect("second slot lease should be acquired");
    for lease in [&first, &second] {
        service
            .mark_workspace_slot_running(&lease.worktree_path)
            .expect("slot should transition to running");
    }

    let first_order = service
        .reserve_workspace_official_completion_refresh_order(&first.worktree_path)
        .expect("first order reservation should succeed")
        .expect("first running slot should reserve an order");
    let second_order = service
        .reserve_workspace_official_completion_refresh_order(&second.worktree_path)
        .expect("second order reservation should succeed")
        .expect("second running slot should reserve an order");

    let second_report = service
        .begin_workspace_official_completion(
            &second.worktree_path,
            "turn-2",
            Some(second_order),
            Some("second completion finished"),
            Some("cargo test passed"),
            None,
        )
        .expect("second official completion should be captured")
        .expect("second report should be returned");
    let first_report = service
        .begin_workspace_official_completion(
            &first.worktree_path,
            "turn-1",
            Some(first_order),
            Some("first completion finished"),
            Some("cargo test passed"),
            None,
        )
        .expect("first official completion should be captured")
        .expect("first report should be returned");

    assert_eq!(first_report.refresh_order, 1);
    assert_eq!(second_report.refresh_order, 2);
}

#[test]
fn distributor_queue_keeps_later_item_queued_behind_blocked_head() {
    let repo = TempGitRepo::new("distributor-queue-blocked-head");
    let service = test_parallel_mode_service();
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    let first = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("first slot lease should be acquired");
    let second = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-2", "Task Two", "agent-2", "task-two"),
        )
        .expect("second slot lease should be acquired");
    for lease in [&first, &second] {
        let slot_path = PathBuf::from(lease.worktree_path.clone());
        service
            .mark_workspace_slot_running(&lease.worktree_path)
            .expect("slot should transition to running");
        repo.commit_file_in_slot(
            &slot_path,
            &format!("{}.txt", lease.task_id),
            "done\n",
            "agent work",
        );
        service
            .begin_workspace_official_completion(
                &lease.worktree_path,
                &format!("turn-{}", lease.task_id),
                None,
                Some("Distributor queue slice completed."),
                Some("cargo test passed"),
                None,
            )
            .expect("official completion should be captured");
        service
            .mark_workspace_official_completion_refreshing(&lease.worktree_path)
            .expect("ledger refreshing should be recorded");
        service
            .mark_workspace_commit_ready(
                &lease.worktree_path,
                "official ledger refresh succeeded: queued for delivery",
            )
            .expect("commit-ready should be recorded");
        service
            .enqueue_workspace_commit_ready_result(&lease.worktree_path)
            .expect("commit-ready result should enqueue")
            .expect("queue item should be present");
    }

    fs::remove_dir_all(&first.worktree_path).expect("first slot worktree should be removed");

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("processing the queue head should not crash");
    assert!(notices.iter().any(|notice| {
        notice.contains("distributor queue head blocked")
            || notice.contains("distributor queue head is blocked")
    }));

    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    assert_eq!(snapshot.distributor.head_summary, "blocked");
    assert_eq!(snapshot.distributor.queue_depth(), 2);
    assert_eq!(
        snapshot.distributor.queue_items[0].queue_state,
        ParallelModeQueueItemState::Blocked
    );
    assert_eq!(
        snapshot.distributor.queue_items[1].queue_state,
        ParallelModeQueueItemState::Queued
    );
    assert!(
        snapshot
            .distributor
            .note
            .contains("source worktree is missing"),
        "queue note should explain the blocked head: {}",
        snapshot.distributor.note
    );
    assert!(
        snapshot
            .distributor
            .head_blocked_detail
            .as_deref()
            .expect("blocked head detail should be surfaced")
            .contains("source worktree is missing")
    );
}

#[test]
fn distributor_recovery_blocks_missing_worktree_from_store_backed_queue_record() {
    let repo = TempGitRepo::new("distributor-store-recovery-missing-worktree");
    let service = test_parallel_mode_service();

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-store-recovery",
            None,
            Some("Distributor recovery slice completed."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("ledger refreshing should be recorded");
    service
        .mark_workspace_commit_ready(
            &lease.worktree_path,
            "official ledger refresh succeeded: queued for delivery",
        )
        .expect("commit-ready should be recorded");
    service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect("commit-ready result should enqueue")
        .expect("queue item should be created");

    let session_key = lease_session_key(&lease);
    let queue_record = load_distributor_queue_records(&repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should exist before mirror loss");
    fs::remove_file(repo.slot_lease_path(1)).expect("slot lease mirror should be removed");
    fs::remove_file(repo.session_detail_path(&session_key))
        .expect("session detail mirror should be removed");
    fs::remove_file(repo.distributor_queue_path(&queue_record.queue_item_id))
        .expect("queue mirror should be removed");
    fs::remove_dir_all(&lease.worktree_path).expect("source worktree should be removed");

    let recovered = test_parallel_mode_service();
    let notices = recovered
        .process_distributor_queue(&repo.workspace_dir())
        .expect("recovery should classify the missing worktree as blocked");
    assert!(
        notices.iter().any(|notice| {
            notice.contains("blocked") && notice.contains("recovered after restart")
        }),
        "recovery notice should explain the blocked head: {notices:?}"
    );

    let recovered_record = load_distributor_queue_records(&repo.pool_root())
        .into_iter()
        .next()
        .expect("blocked queue record should be rewritten from the authority store");
    assert_eq!(
        recovered_record.queue_state,
        ParallelModeQueueItemState::Blocked
    );
    assert!(
        recovered_record
            .integration_note
            .contains("recovered after restart: source worktree is missing")
    );

    let recovered_detail = read_agent_session_detail_record(&repo.pool_root(), &session_key)
        .expect("failed session detail should be rewritten from the authority store");
    assert_eq!(recovered_detail.state_label, "failed");
    assert!(
        recovered_detail
            .history
            .last()
            .expect("failure history entry should exist")
            .summary
            .contains("recovered after restart")
    );
}

#[test]
fn supervisor_snapshot_reclassifies_integrated_queue_head_from_store_backed_recovery() {
    let repo = TempGitRepo::new("supervisor-store-recovery-integrated");
    let service = test_parallel_mode_service();

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-integrated-recovery",
            None,
            Some("Integrated queue recovery slice completed."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("ledger refreshing should be recorded");
    service
        .mark_workspace_commit_ready(
            &lease.worktree_path,
            "official ledger refresh succeeded: queued for delivery",
        )
        .expect("commit-ready should be recorded");
    service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect("commit-ready result should enqueue")
        .expect("queue item should be created");
    let session_key = lease_session_key(&lease);
    let queue_record = load_distributor_queue_records(&repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should exist");
    repo.merge_agent_slot_into_akra(&slot_path);
    fs::remove_file(repo.slot_lease_path(1)).expect("slot lease mirror should be removed");
    fs::remove_file(repo.session_detail_path(&session_key))
        .expect("session detail mirror should be removed");
    fs::remove_file(repo.distributor_queue_path(&queue_record.queue_item_id))
        .expect("queue mirror should be removed");

    let recovered = test_parallel_mode_service();
    let readiness = recovered.inspect_readiness(
        &repo.workspace_dir(),
        &PlanningRuntimeSnapshot::ready("prompt".into(), "queue".into(), None)
            .with_workspace_present(true),
    );
    let snapshot =
        recovered.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));

    assert_eq!(snapshot.distributor.head_summary, "cleaning");
    assert_eq!(snapshot.distributor.queue_depth(), 1);
    assert_eq!(
        snapshot.distributor.queue_items[0].queue_state,
        ParallelModeQueueItemState::Cleaning
    );
    assert!(
        snapshot
            .distributor
            .note
            .contains("recovered after restart"),
        "snapshot should surface the recovery note: {}",
        snapshot.distributor.note
    );
    assert_eq!(
        repo.read_slot_lease(1).state,
        ParallelModeSlotLeaseState::CleanupPending
    );
}

#[test]
fn distributor_snapshot_surfaces_rebase_provenance_for_blocked_head() {
    let repo = TempGitRepo::new("distributor-rebase-provenance");
    let service = test_parallel_mode_service_with_github(Arc::new(
        FakeGithubAutomationPort::with_force_push_error("force-with-lease rejected"),
    ));
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-rebase-provenance",
            None,
            Some("Distributor rebase provenance slice completed."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("ledger refreshing should be recorded");
    service
        .mark_workspace_commit_ready(
            &lease.worktree_path,
            "official ledger refresh succeeded: queued for delivery",
        )
        .expect("commit-ready should be recorded");
    service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect("commit-ready result should enqueue")
        .expect("queue item should be created");

    let original_queue_record = load_distributor_queue_records(&repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should exist");
    let original_commit_sha = original_queue_record.commit_sha;

    let original_branch = current_branch(&repo.repo_root);
    run_git(&repo.repo_root, &["checkout", "akra"]);
    fs::write(repo.repo_root.join("baseline.txt"), "baseline advanced\n")
        .expect("baseline file should be written");
    run_git(&repo.repo_root, &["add", "baseline.txt"]);
    run_git(&repo.repo_root, &["commit", "-qm", "advance akra baseline"]);
    run_git(&repo.repo_root, &["checkout", original_branch.as_str()]);

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("processing the queue head should succeed");
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("force-pushed") || notice.contains("blocked")),
        "processing should surface the blocked force-push outcome: {notices:?}"
    );

    let queue_record = load_distributor_queue_records(&repo.pool_root())
        .into_iter()
        .next()
        .expect("blocked queue record should persist");
    assert_eq!(
        queue_record.queue_state,
        ParallelModeQueueItemState::Blocked
    );
    assert_ne!(queue_record.commit_sha, original_commit_sha);
    assert!(
        queue_record
            .integration_note
            .contains("could not be force-pushed")
    );

    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    assert_eq!(snapshot.distributor.head_summary, "blocked");
    assert!(
        snapshot
            .distributor
            .head_blocked_detail
            .as_deref()
            .expect("blocked head detail should be surfaced")
            .contains("could not be force-pushed")
    );
    let provenance = snapshot
        .distributor
        .head_rebase_provenance
        .as_deref()
        .expect("rebase provenance should be surfaced");
    assert!(provenance.contains(short_sha(&original_commit_sha).as_str()));
    assert!(provenance.contains(short_sha(&queue_record.commit_sha).as_str()));
    assert!(provenance.contains("onto `akra`"));
}

#[test]
fn distributor_queue_blocks_rebase_conflict_for_operator_recovery() {
    let repo = TempGitRepo::new("distributor-rebase-conflict");
    let github = FakeGithubAutomationPort::ready();
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    let original_branch = current_branch(&repo.repo_root);
    run_git(&repo.repo_root, &["checkout", "akra"]);
    fs::write(repo.repo_root.join("conflict.txt"), "base\n")
        .expect("baseline conflict file should be written");
    run_git(&repo.repo_root, &["add", "conflict.txt"]);
    run_git(
        &repo.repo_root,
        &["commit", "-qm", "seed conflict baseline"],
    );
    run_git(&repo.repo_root, &["checkout", original_branch.as_str()]);

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    repo.commit_file_in_slot(
        &slot_path,
        "conflict.txt",
        "agent change\n",
        "agent updates conflict",
    );
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-rebase-conflict",
            None,
            Some("Distributor rebase conflict recovery slice completed."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("ledger refreshing should be recorded");
    service
        .mark_workspace_commit_ready(
            &lease.worktree_path,
            "official ledger refresh succeeded: queued for delivery",
        )
        .expect("commit-ready should be recorded");
    service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect("commit-ready result should enqueue")
        .expect("queue item should be created");

    run_git(&repo.repo_root, &["checkout", "akra"]);
    fs::write(repo.repo_root.join("conflict.txt"), "baseline change\n")
        .expect("advanced baseline conflict file should be written");
    run_git(&repo.repo_root, &["add", "conflict.txt"]);
    run_git(
        &repo.repo_root,
        &["commit", "-qm", "advance conflicting akra baseline"],
    );
    run_git(&repo.repo_root, &["checkout", original_branch.as_str()]);

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("processing the queue head should succeed");
    assert!(
        notices.iter().any(|notice| {
            notice.contains("distributor queue head blocked")
                && notice.contains("could not rebase onto `akra` cleanly")
        }),
        "processing should surface the rebase-conflict block: {notices:?}"
    );

    let queue_record = load_distributor_queue_records(&repo.pool_root())
        .into_iter()
        .next()
        .expect("blocked queue record should persist");
    assert_eq!(
        queue_record.queue_state,
        ParallelModeQueueItemState::Blocked
    );
    assert!(
        queue_record
            .integration_note
            .contains("could not rebase onto `akra` cleanly")
    );

    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    assert_eq!(snapshot.distributor.head_summary, "blocked");
    assert_eq!(snapshot.distributor.queue_depth(), 1);
    assert_eq!(
        snapshot.distributor.queue_items[0].queue_state,
        ParallelModeQueueItemState::Blocked
    );
    assert!(
        snapshot
            .distributor
            .head_blocked_detail
            .as_deref()
            .expect("blocked head detail should be surfaced")
            .contains("could not rebase onto `akra` cleanly")
    );
    assert!(
        snapshot.distributor.head_rebase_provenance.is_none(),
        "failed rebase should not report successful rebase provenance"
    );
    assert_eq!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .clone(),
        vec![
            format!("push:{}:false", lease.branch_name),
            format!("ensure-pr:akra:{}", lease.branch_name),
            "inspect-pr:77".to_string(),
        ]
    );
}

#[test]
fn unavailable_pool_board_does_not_report_exhausted() {
    let pool = build_pool_board(&SqlitePlanningAuthorityAdapter::new(), "/tmp/root", None);

    assert_eq!(pool.unavailable_slots, DEFAULT_POOL_SIZE);
    assert!(!pool.exhausted);
}

#[test]
fn reconcile_marks_missing_slots_when_pool_root_has_not_been_created() {
    let repo = TempGitRepo::new("missing-slots");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let pool = build_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
        Some(&readiness),
    );

    assert_eq!(pool.missing_slots, DEFAULT_POOL_SIZE);
    assert_eq!(pool.idle_slots, 0);
    assert!(!pool.exhausted);
    assert!(pool.reconcile_status.contains("missing slot"));
}

#[test]
fn detached_akra_slot_counts_as_idle_baseline() {
    let repo = TempGitRepo::new("idle-slot");
    repo.create_detached_slot(1);
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    let pool = build_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
        Some(&readiness),
    );
    let slot = &pool.slots[0];

    assert_eq!(slot.state, ParallelModePoolSlotState::Idle);
    assert_eq!(slot.branch_name, "akra (detached)");
    assert_eq!(pool.idle_slots, 1);
    assert_eq!(pool.missing_slots, DEFAULT_POOL_SIZE - 1);
}

#[test]
fn agent_branch_slot_is_marked_awaiting_cleanup() {
    let repo = TempGitRepo::new("cleanup-slot");
    repo.create_agent_slot(1, "task-one");
    let slot_path = repo.pool_root().join(slot_id(1));
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    repo.merge_agent_slot_into_akra(&slot_path);
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    let pool = build_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
        Some(&readiness),
    );
    let slot = &pool.slots[0];

    assert_eq!(slot.state, ParallelModePoolSlotState::AwaitingCleanup);
    assert!(slot.branch_name.starts_with("akra-agent/slot-1/"));
    assert_eq!(slot.owner_label, "cleanup pending");
    assert_eq!(pool.awaiting_cleanup_slots, 1);
}

#[test]
fn non_merged_agent_branch_without_lease_surfaces_operator_recovery_notice() {
    let repo = TempGitRepo::new("non-merged-slot");
    let service = test_parallel_mode_service();
    let slot_path = repo.create_agent_slot(1, "task-one");
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));
    let slot = &snapshot.pool.slots[0];

    assert_eq!(slot.state, ParallelModePoolSlotState::Blocked);
    assert_eq!(slot.owner_label, "operator recovery");
    assert!(slot.branch_name.starts_with("akra-agent/slot-1/"));
    assert!(
        slot.worktree_label
            .contains(super::NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_DETAIL)
    );
    assert!(
        snapshot
            .pool
            .reconcile_status
            .contains("next action: inspect the slot branch")
    );
    let notice = snapshot
        .top_notice
        .as_deref()
        .expect("operator recovery notice should be surfaced");
    assert!(notice.contains("pool: blocked"));
    assert!(notice.contains("slot-1"));
    assert!(notice.contains("not integrated into `akra`"));
    assert!(notice.contains("next action: inspect the slot branch"));
}

#[test]
fn dirty_akra_baseline_slot_is_blocked_for_operator_recovery() {
    let repo = TempGitRepo::new("dirty-slot");
    let slot_path = repo.create_detached_slot(1);
    fs::write(slot_path.join("README.md"), "dirty\n").expect("slot file should be updated");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );

    let pool = build_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
        Some(&readiness),
    );
    let slot = &pool.slots[0];

    assert_eq!(slot.state, ParallelModePoolSlotState::Blocked);
    assert_eq!(slot.owner_label, "operator recovery");
    assert!(slot.worktree_label.contains("unstaged changes"));
}

#[test]
fn reconcile_provisions_missing_slots_into_idle_baselines() {
    let repo = TempGitRepo::new("provision-slots");

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
    );

    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
    assert_eq!(pool.missing_slots, 0);
    assert!(pool.reconcile_status.contains("provisioned 3"));
    for slot_number in 1..=DEFAULT_POOL_SIZE {
        assert!(repo.pool_root().join(slot_id(slot_number)).exists());
    }
}

#[test]
fn pool_root_lives_in_repo_sibling_akra_worktrees_root() {
    let repo = TempGitRepo::new("pool-root");
    let pool_root = repo.pool_root();
    let normalized = pool_root.to_string_lossy().replace('\\', "/");

    assert!(
        normalized.contains("/repo-akra-worktrees/"),
        "pool root should live under a repo sibling akra worktrees root: {normalized}"
    );
    assert!(
        normalized.ends_with("/akra-pool"),
        "pool root should end at the akra pool directory: {normalized}"
    );
}

#[test]
fn reconcile_creates_local_akra_branch_before_provisioning_slots() {
    let repo = TempGitRepo::new("create-akra");
    repo.delete_local_akra_branch();
    assert!(!repo.branch_exists("akra"));

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
    );

    assert!(repo.branch_exists("akra"));
    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
    assert!(pool.reconcile_status.contains("created `akra`"));
}

#[test]
fn reconcile_resets_empty_akra_baseline_to_current_head() {
    let repo = TempGitRepo::new("reset-akra");
    let old_akra_head = repo.head_sha();
    repo.commit_on_current_branch("feature.txt", "new baseline\n", "advance user branch");
    let current_head = repo.head_sha();
    assert_ne!(old_akra_head, current_head);

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
    );

    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.repo_root.to_str().expect("repo root should be utf-8"),
                "rev-parse",
                "akra",
            ],
            None,
        )
        .expect("akra should resolve"),
        current_head
    );
    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
}

#[test]
fn reconcile_resets_clean_detached_slots_after_empty_akra_baseline_moves() {
    let repo = TempGitRepo::new("reset-detached-slots");
    let initial_pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
    );
    assert_eq!(initial_pool.idle_slots, DEFAULT_POOL_SIZE);

    repo.commit_on_current_branch("feature.txt", "new baseline\n", "advance user branch");

    let refreshed_pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
    );

    assert_eq!(refreshed_pool.idle_slots, DEFAULT_POOL_SIZE);
    assert_eq!(refreshed_pool.blocked_slots, 0);
    assert!(refreshed_pool.slots.iter().all(|slot| {
        !slot
            .worktree_label
            .contains("detached away from `akra` baseline")
    }));
}

#[test]
fn build_pool_board_uses_remote_akra_when_local_branch_is_missing() {
    let repo = TempGitRepo::new("remote-akra");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let head_sha = repo.head_sha();
    repo.delete_local_akra_branch();
    repo.set_remote_tracking_branch("origin/akra", &head_sha);

    let pool = build_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
        Some(&readiness),
    );

    assert_eq!(pool.blocked_slots, 0);
    assert_eq!(pool.missing_slots, DEFAULT_POOL_SIZE);
    assert!(
        pool.reconcile_status.contains("missing"),
        "unexpected reconcile status: {}",
        pool.reconcile_status
    );
}

#[test]
fn detect_canonical_repo_root_uses_workspace_relative_common_dir() {
    let repo = TempGitRepo::new("canonical-root");
    let nested_workspace = repo.repo_root.join("nested").join("deeper");
    fs::create_dir_all(&nested_workspace).expect("nested workspace should exist");

    let canonical_repo_root = detect_canonical_repo_root(
        &SqlitePlanningAuthorityAdapter::new(),
        nested_workspace.to_str().expect("valid nested path"),
    )
    .expect("canonical repo root should resolve");

    assert_eq!(
        canonical_repo_root,
        fs::canonicalize(&repo.repo_root).expect("repo root should canonicalize")
    );
}

#[test]
fn inspect_readiness_reports_authority_store_from_canonical_repo_root() {
    let repo = TempGitRepo::new("authority-readiness");
    let linked_worktree = repo.create_linked_worktree("feature/authority-readiness");
    SqlitePlanningAuthorityAdapter::replace_active_planning_file(
        linked_worktree
            .to_str()
            .expect("valid linked worktree path"),
        DIRECTIONS_FILE_PATH,
        Some("version = 1\n"),
    )
    .expect("authority store should seed active directions");
    let worktree_directions_path = linked_worktree.join(DIRECTIONS_FILE_PATH);
    fs::create_dir_all(
        worktree_directions_path
            .parent()
            .expect("worktree directions path should have a parent directory"),
    )
    .expect("worktree planning directory should exist");
    fs::write(&worktree_directions_path, "version = 0\n")
        .expect("linked-worktree directions should diverge");
    let service = test_parallel_mode_service();

    let snapshot = service.inspect_readiness(
        linked_worktree
            .to_str()
            .expect("valid linked worktree path"),
        &PlanningRuntimeSnapshot::ready("prompt".into(), "queue".into(), None)
            .with_workspace_present(true),
    );
    let capability = snapshot
        .capability(ParallelModeCapabilityKey::AuthorityStore)
        .expect("authority store capability should exist");

    assert_eq!(capability.state, ParallelModeCapabilityState::Ready);
    assert!(capability.detail.contains("shadow store"));
    assert!(capability.detail.contains(&repo.workspace_dir()));
    assert!(!capability.detail.contains("version = 0"));
}

#[test]
fn reconcile_cleans_merged_agent_slot_back_to_idle() {
    let repo = TempGitRepo::new("cleanup-execution");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    let branch_name = lease.branch_name.clone();
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");
    repo.merge_agent_slot_into_akra(&slot_path);
    service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to cleanup pending");
    fs::write(slot_path.join("scratch.tmp"), "transient\n")
        .expect("untracked file should be written");

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
    );
    let slot = &pool.slots[0];

    assert_eq!(slot.state, ParallelModePoolSlotState::Idle);
    assert!(slot.branch_name.starts_with("akra"));
    assert!(!slot_path.join("scratch.tmp").exists());
    assert!(!repo.branch_exists(&branch_name));
    assert!(!repo.slot_lease_path(1).exists());
    assert!(pool.reconcile_status.contains("cleaned 1"));
}

#[test]
fn acquire_slot_lease_persists_metadata_and_marks_slot_leased() {
    let repo = TempGitRepo::new("lease-slot");
    let service = test_parallel_mode_service();

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task one"),
        )
        .expect("slot lease should be acquired");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let pool = build_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
        Some(&readiness),
    );
    let persisted = repo.read_slot_lease(1);

    assert_eq!(lease.slot_id, "slot-1");
    assert_eq!(lease.state, ParallelModeSlotLeaseState::Leased);
    assert_eq!(persisted.state, ParallelModeSlotLeaseState::Leased);
    assert_eq!(persisted.agent_id, "agent-1");
    assert_eq!(persisted.task_id, "task-1");
    assert!(
        persisted
            .branch_name
            .starts_with("akra-agent/slot-1/task-one")
    );
    assert_eq!(pool.leased_slots, 1);
    assert_eq!(pool.running_slots, 0);
    assert_eq!(pool.slots[0].state, ParallelModePoolSlotState::Leased);
    assert_eq!(pool.slots[0].owner_label, "agent-1 / task-1");
}

#[test]
fn acquire_slot_lease_truncates_long_branch_slug_with_stable_hash() {
    let repo = TempGitRepo::new("lease-slot-long-branch");
    let service = test_parallel_mode_service();
    let long_slug = format!("{}tail", "very-long-task-segment-".repeat(8));
    let sanitized_slug = sanitize_task_slug(&long_slug).expect("long slug should sanitize");

    assert!(sanitized_slug.len() > MAX_AGENT_BRANCH_SLUG_LEN);

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", &long_slug),
        )
        .expect("slot lease should be acquired");
    let slug = lease
        .branch_name
        .strip_prefix("akra-agent/slot-1/")
        .expect("slot branch prefix should be present");

    assert!(slug.len() <= MAX_AGENT_BRANCH_SLUG_LEN);
    assert!(slug.ends_with(short_branch_slug_hash(&sanitized_slug).as_str()));
    assert!(repo.branch_exists(&lease.branch_name));
}

#[test]
fn allocate_agent_branch_name_numbers_collisions_without_exceeding_slug_limit() {
    let repo = TempGitRepo::new("lease-slot-branch-collision");
    let long_slug = format!("{}tail", "collision-prone-task-segment-".repeat(6));
    let sanitized_slug = sanitize_task_slug(&long_slug).expect("long slug should sanitize");

    assert!(sanitized_slug.len() > MAX_AGENT_BRANCH_SLUG_LEN);

    let first = allocate_agent_branch_name(
        &repo.workspace_dir(),
        "slot-1",
        &long_slug,
        "task-1",
        "Task One",
    );
    run_git(&repo.repo_root, &["branch", first.as_str(), "akra"]);

    let second = allocate_agent_branch_name(
        &repo.workspace_dir(),
        "slot-1",
        &long_slug,
        "task-1",
        "Task One",
    );
    let slug = second
        .strip_prefix("akra-agent/slot-1/")
        .expect("slot branch prefix should be present");
    let base_slug = slug
        .strip_suffix("-2")
        .expect("collision branch should carry a numbered suffix");

    assert_ne!(first, second);
    assert!(slug.len() <= MAX_AGENT_BRANCH_SLUG_LEN);
    assert!(base_slug.ends_with(short_branch_slug_hash(&sanitized_slug).as_str()));
}

#[test]
fn mark_slot_running_updates_persisted_lease_and_pool_state() {
    let repo = TempGitRepo::new("running-slot");
    let service = test_parallel_mode_service();

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let running_lease = service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let pool = build_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
        Some(&readiness),
    );
    let persisted = repo.read_slot_lease(1);

    assert_eq!(running_lease.state, ParallelModeSlotLeaseState::Running);
    assert!(running_lease.running_started_at.is_some());
    assert_eq!(persisted.state, ParallelModeSlotLeaseState::Running);
    assert!(persisted.running_started_at.is_some());
    assert_eq!(pool.leased_slots, 0);
    assert_eq!(pool.running_slots, 1);
    assert_eq!(pool.slots[0].state, ParallelModePoolSlotState::Running);
}

#[test]
fn mark_workspace_slot_running_updates_matching_lease() {
    let repo = TempGitRepo::new("workspace-running-slot");
    let service = test_parallel_mode_service();

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");

    let running_lease = service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("workspace lease transition should succeed")
        .expect("workspace should have an active lease");
    let persisted = repo.read_slot_lease(1);

    assert_eq!(running_lease.state, ParallelModeSlotLeaseState::Running);
    assert_eq!(persisted.state, ParallelModeSlotLeaseState::Running);
    assert!(persisted.running_started_at.is_some());
}

#[test]
fn resolve_workspace_slot_lease_matches_nested_worktree_directory() {
    let repo = TempGitRepo::new("nested-worktree-resolution");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let nested_workspace = PathBuf::from(&lease.worktree_path).join("nested");
    fs::create_dir_all(&nested_workspace).expect("nested worktree directory should exist");

    let resolution = resolve_workspace_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        nested_workspace
            .to_str()
            .expect("nested workspace should be valid utf-8"),
    )
    .expect("workspace lease lookup should succeed")
    .expect("workspace lease should resolve");

    assert_eq!(resolution.lease.slot_id, lease.slot_id);
    assert_eq!(
        resolution.workspace_path,
        fs::canonicalize(&lease.worktree_path).expect("slot worktree should canonicalize")
    );
}

#[test]
fn mark_workspace_slot_cleanup_pending_if_ready_waits_for_integrated_branch() {
    let repo = TempGitRepo::new("workspace-cleanup-ready");
    let service = test_parallel_mode_service();

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");

    let pending_before_merge = service
        .mark_workspace_slot_cleanup_pending_if_ready(&lease.worktree_path)
        .expect("cleanup-ready check should succeed before merge");
    assert!(pending_before_merge.is_none());
    assert_eq!(
        repo.read_slot_lease(1).state,
        ParallelModeSlotLeaseState::Running
    );

    repo.merge_agent_slot_into_akra(&slot_path);

    let pending_after_merge = service
        .mark_workspace_slot_cleanup_pending_if_ready(&lease.worktree_path)
        .expect("cleanup-ready check should succeed after merge")
        .expect("workspace should transition once branch is integrated");

    assert_eq!(
        pending_after_merge.state,
        ParallelModeSlotLeaseState::CleanupPending
    );
    assert_eq!(
        repo.read_slot_lease(1).state,
        ParallelModeSlotLeaseState::CleanupPending
    );
}

#[test]
fn cleanup_workspace_slot_if_pending_resets_slot_to_idle() {
    let repo = TempGitRepo::new("workspace-cleanup-slot");
    let service = test_parallel_mode_service();

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");
    repo.merge_agent_slot_into_akra(&slot_path);
    service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to cleanup pending");
    fs::write(slot_path.join("scratch.tmp"), "transient\n")
        .expect("untracked file should be written");

    let cleaned_lease = service
        .cleanup_workspace_slot_if_pending(&lease.worktree_path)
        .expect("cleanup-pending workspace should be cleaned")
        .expect("workspace should have an active cleanup-pending lease");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let pool = build_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
        Some(&readiness),
    );

    assert_eq!(cleaned_lease.slot_id, "slot-1");
    assert_eq!(
        cleaned_lease.state,
        ParallelModeSlotLeaseState::CleanupPending
    );
    assert!(!slot_path.join("scratch.tmp").exists());
    assert!(!repo.branch_exists(&lease.branch_name));
    assert!(!repo.slot_lease_path(1).exists());
    assert_eq!(pool.leased_slots, 0);
    assert_eq!(pool.running_slots, 0);
    assert_eq!(pool.awaiting_cleanup_slots, 0);
    assert_eq!(pool.slots[0].state, ParallelModePoolSlotState::Idle);
}

#[test]
fn release_workspace_slot_lease_after_failed_start_resets_clean_slot_to_idle() {
    let repo = TempGitRepo::new("release-unstarted-slot");
    let service = test_parallel_mode_service();

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let released_lease = service
        .release_workspace_slot_lease_after_failed_start(&lease.worktree_path)
        .expect("clean unstarted slot should be released")
        .expect("workspace should have an active lease");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let pool = build_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
        Some(&readiness),
    );

    assert_eq!(released_lease.slot_id, "slot-1");
    assert_eq!(released_lease.state, ParallelModeSlotLeaseState::Leased);
    assert!(!repo.slot_lease_path(1).exists());
    assert!(!repo.branch_exists(&lease.branch_name));
    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
    assert_eq!(pool.leased_slots, 0);
    assert_eq!(pool.slots[0].state, ParallelModePoolSlotState::Idle);
}

#[test]
fn release_workspace_slot_lease_after_failed_start_rejects_dirty_worktree() {
    let repo = TempGitRepo::new("release-dirty-slot");
    let service = test_parallel_mode_service();

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    fs::write(
        Path::new(&lease.worktree_path).join("dirty.txt"),
        "scratch\n",
    )
    .expect("worktree should become dirty");

    let error = service
        .release_workspace_slot_lease_after_failed_start(&lease.worktree_path)
        .expect_err("dirty unstarted slot should stay leased");

    assert!(error.contains("could not be released after startup failure"));
    assert!(repo.slot_lease_path(1).exists());
    assert!(repo.branch_exists(&lease.branch_name));
}

#[test]
fn mark_slot_cleanup_pending_requires_running_state_and_merged_branch() {
    let repo = TempGitRepo::new("cleanup-pending-guards");
    let service = test_parallel_mode_service();

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");

    let not_running_error = service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect_err("cleanup pending should require the running state");
    assert!(not_running_error.contains("has not entered running state"));

    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");
    let not_merged_error = service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect_err("cleanup pending should require an integrated branch");
    assert!(not_merged_error.contains("is not integrated into `akra` yet"));
}

#[test]
fn mark_slot_cleanup_pending_updates_persisted_lease_and_pool_state() {
    let repo = TempGitRepo::new("cleanup-pending-slot");
    let service = test_parallel_mode_service();

    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");
    repo.merge_agent_slot_into_akra(&slot_path);

    let cleanup_pending_lease = service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to cleanup pending");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let pool = build_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
        Some(&readiness),
    );
    let persisted = repo.read_slot_lease(1);

    assert_eq!(
        cleanup_pending_lease.state,
        ParallelModeSlotLeaseState::CleanupPending
    );
    assert_eq!(persisted.state, ParallelModeSlotLeaseState::CleanupPending);
    assert_eq!(pool.awaiting_cleanup_slots, 1);
    assert_eq!(pool.running_slots, 0);
    assert_eq!(
        pool.slots[0].state,
        ParallelModePoolSlotState::AwaitingCleanup
    );
    assert_eq!(pool.slots[0].owner_label, "agent-1 / task-1");
}

#[test]
fn reconcile_does_not_cleanup_pending_slot_with_new_unintegrated_commit() {
    let repo = TempGitRepo::new("cleanup-pending-reverify");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(&lease.worktree_path);
    let branch_name = lease.branch_name.clone();
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot should enter running state");
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    repo.merge_agent_slot_into_akra(&slot_path);
    service
        .mark_slot_cleanup_pending(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot should enter cleanup pending");
    repo.commit_file_in_slot(
        &slot_path,
        "late-change.txt",
        "late work\n",
        "late cleanup pending change",
    );

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
    );
    let slot = pool
        .slots
        .iter()
        .find(|slot| slot.slot_id == lease.slot_id)
        .expect("slot should be present");

    assert!(repo.branch_exists(&branch_name));
    assert_eq!(slot.state, ParallelModePoolSlotState::AwaitingCleanup);
    assert!(repo.slot_lease_path(1).exists());
}
