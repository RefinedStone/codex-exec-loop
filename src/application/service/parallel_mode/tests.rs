use super::distributor::load_distributor_queue_records;
use super::{
    DEFAULT_POOL_SIZE, MAX_AGENT_BRANCH_SLUG_LEN, ParallelModeCapabilityKey,
    ParallelModeCapabilitySnapshot, ParallelModeCapabilityState, ParallelModeReadinessSnapshot,
    ParallelModeReadinessState, ParallelModeService, agent_session_detail_record_path,
    allocate_agent_branch_name, build_pool_board, derive_default_pool_root,
    detect_canonical_repo_root, lease_session_key, parse_https_remote,
    read_agent_session_detail_record, reconcile_pool_board, resolve_workspace_slot_lease,
    run_command, sanitize_task_slug, short_branch_slug_hash, slot_id, slot_lease_file_path,
};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::application::port::outbound::github_automation_port::{
    GithubAutomationCapabilities, GithubAutomationPort, GithubAutomationPullRequest,
};
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityDistributorQueueRecord;
use crate::application::service::planning::PlanningRuntimeSnapshot;
use crate::application::service::planning::shared::contract::RESULT_OUTPUT_FILE_PATH;
use crate::domain::parallel_mode::{
    ParallelModePoolSlotState, ParallelModeQueueItemState, ParallelModeSlotLeaseRequest,
    ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState, ParallelModeSupervisorState,
};
use crate::domain::planning::{PriorityQueueProjection, PriorityQueueTask, TaskStatus};
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
        run_git(&repo_root, &["branch", "prerelease"]);

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
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    )
}

fn test_parallel_mode_service_with_github(
    github: Arc<dyn GithubAutomationPort>,
) -> ParallelModeService {
    ParallelModeService::new(
        Arc::new(SqlitePlanningAuthorityAdapter::new()),
        github,
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    )
}

mod distributor;
mod pool;
mod supervisor;

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
