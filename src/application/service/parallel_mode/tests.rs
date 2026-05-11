use super::distributor::load_distributor_queue_records;
use super::{
    DEFAULT_POOL_SIZE, DEFAULT_PUSH_REMOTE_NAME, MAX_AGENT_BRANCH_SLUG_LEN, POOL_BASELINE_BRANCH,
    ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
    ParallelModeReadinessSnapshot, ParallelModeReadinessState, ParallelModeService,
    agent_session_detail_record_path, allocate_agent_branch_name, build_pool_board,
    command_succeeds, derive_default_pool_root, detect_canonical_repo_root, inspect_gh_auth,
    inspect_gh_binary, lease_session_key, local_branch_ref, parse_https_remote,
    read_agent_session_detail_record, reconcile_pool_board, record_assigned_session_detail,
    remote_branch_name, remote_tracking_branch_ref, resolve_workspace_slot_lease, run_command,
    sanitize_task_slug, short_branch_slug_hash, slot_id, slot_lease_file_path,
};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::application::port::outbound::github_automation_port::{
    GithubAutomationCapabilities, GithubAutomationPort, GithubAutomationPullRequest,
};
use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityOfficialRefreshClaimStatus,
};
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
};
use crate::application::service::planning::PlanningRuntimeProjection;
use crate::application::service::planning::shared::contract::RESULT_OUTPUT_FILE_PATH;
use crate::domain::parallel_mode::{
    ParallelModeAutomationTrigger, ParallelModeDispatchBlockReason,
    ParallelModeDispatchCommandSnapshot, ParallelModePoolResetPolicy,
    ParallelModePoolResetSlotAction, ParallelModePoolResetSlotOutcome, ParallelModePoolSlotState,
    ParallelModeQueueItemState, ParallelModeSlotLeaseRequest, ParallelModeSlotLeaseSnapshot,
    ParallelModeSlotLeaseState, ParallelModeSupervisorState, ParallelModeTaskDispatchBlockSnapshot,
};
use crate::domain::planning::{
    PriorityQueueProjection, PriorityQueueTask, TaskActor, TaskAuthorityDocument, TaskDefinition,
    TaskStatus,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// parallel_mode 서비스 테스트는 실제 git worktree, branch ref, pool 파일을 함께
// 다룬다. 이 fixture는 각 테스트가 독립 repo를 만들고 authority store와
// filesystem projection을 같은 root 아래에서 검증하게 해 준다.
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
        run_git(&repo_root, &["branch", POOL_BASELINE_BRANCH]);
        run_git(
            &repo_root,
            &[
                "update-ref",
                &remote_standard_tracking_ref(),
                POOL_BASELINE_BRANCH,
            ],
        );

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
                POOL_BASELINE_BRANCH,
            ],
        );
        slot_path
    }

    // agent slot은 pool slot 경로와 `akra-agent/slot-N/...` branch naming 규칙을
    // 동시에 만든다. distributor/pool 테스트가 실제 worktree layout을 우회하지
    // 않도록 여기에서 git worktree 명령을 직접 사용한다.
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
                POOL_BASELINE_BRANCH,
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
    fn delete_local_prerelease_branch(&self) {
        run_git(&self.repo_root, &["branch", "-D", POOL_BASELINE_BRANCH]);
    }
    fn delete_remote_standard_tracking_branch(&self) {
        run_git(
            &self.repo_root,
            &["update-ref", "-d", &remote_standard_tracking_ref()],
        );
    }
    fn create_bare_origin_remote(&self) -> PathBuf {
        let remote_path = self.root.join("origin.git");
        let output = Command::new("git")
            .args(["init", "--bare", "-q"])
            .arg(&remote_path)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .expect("git init --bare should spawn");
        assert!(
            output.status.success(),
            "git init --bare should succeed\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        run_git(
            &self.repo_root,
            &[
                "remote",
                "add",
                DEFAULT_PUSH_REMOTE_NAME,
                remote_path
                    .to_str()
                    .expect("remote path should be valid utf-8"),
            ],
        );
        remote_path
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
        run_git(&self.repo_root, &["checkout", POOL_BASELINE_BRANCH]);
        run_git(
            &self.repo_root,
            &["merge", "--ff-only", branch_name.as_str()],
        );
        self.set_remote_tracking_branch(&remote_standard_branch_name(), POOL_BASELINE_BRANCH);
        run_git(&self.repo_root, &["checkout", original_branch.as_str()]);
    }

    // branch 존재 여부는 cleanup과 lease allocator의 핵심 관찰값이다. 파일 상태가
    // 아니라 git ref database를 직접 조회해 실제 runtime adapter와 같은 기준을 쓴다.
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
        if current_branch(&self.repo_root) == POOL_BASELINE_BRANCH {
            self.set_remote_tracking_branch(&remote_standard_branch_name(), POOL_BASELINE_BRANCH);
        }
    }
}
impl Drop for TempGitRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

// git helper는 실패한 명령의 stdout/stderr를 assert 메시지에 싣는다. worktree
// 관련 테스트는 실패 원인이 repo state에 묻히기 쉬워서 command line도 함께 고정한다.
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
fn run_git_result(repo_root: &Path, args: &[&str]) -> anyhow::Result<()> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|error| anyhow::anyhow!("git command should spawn: git {args:?}: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    anyhow::bail!(
        "git command failed: git {:?}\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
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
fn remote_standard_branch_name() -> String {
    remote_branch_name(DEFAULT_PUSH_REMOTE_NAME, POOL_BASELINE_BRANCH)
}
fn remote_standard_tracking_ref() -> String {
    remote_tracking_branch_ref(DEFAULT_PUSH_REMOTE_NAME, POOL_BASELINE_BRANCH)
}
fn local_standard_ref() -> String {
    local_branch_ref(POOL_BASELINE_BRANCH)
}
fn sample_lease_request(
    task_id: &str,
    task_title: &str,
    agent_id: &str,
    task_slug: &str,
) -> ParallelModeSlotLeaseRequest {
    ParallelModeSlotLeaseRequest::new(task_id, task_title, agent_id, task_slug)
}

// readiness 검사는 실제 `gh` binary와 repo-local fallback script 중 어느 경로가
// ready로 판정되는지에 민감하다. fake runtime은 그 두 신호만 통제해 startup
// capability 계산을 좁게 검증한다.
#[derive(Debug, Default)]
struct FakeReadinessRuntime {
    gh_path: Option<PathBuf>,
    gh_auth_ok: bool,
    fallback_script_available: bool,
    fallback_auth_ok: bool,
}
impl ParallelModeRuntimePort for FakeReadinessRuntime {
    fn detect_git_repo_root(&self, _workspace_dir: &str) -> Option<String> {
        None
    }
    fn command_succeeds(&self, _program: &str, _args: &[&str]) -> bool {
        false
    }
    fn run_command(
        &self,
        program: &str,
        args: &[&str],
        _current_dir: Option<&str>,
    ) -> Option<String> {
        if program == "bash"
            && args.get(1) == Some(&"auth")
            && args.get(2) == Some(&"status")
            && self.fallback_auth_ok
        {
            return Some("Logged in to github.com as RefinedStone".to_string());
        }
        None
    }
    fn run_command_with_stdin(
        &self,
        _program: &str,
        _args: &[&str],
        _stdin_body: &str,
    ) -> Option<String> {
        None
    }
    fn find_executable(&self, program: &str) -> Option<PathBuf> {
        (program == "gh").then(|| self.gh_path.clone()).flatten()
    }
    fn gh_auth_status(&self, _repo_root: Option<&str>) -> bool {
        self.gh_auth_ok
    }
    fn current_timestamp(&self) -> String {
        "2026-05-01T00:00:00Z".to_string()
    }
    fn canonicalize_best_effort(&self, path: &Path) -> PathBuf {
        path.to_path_buf()
    }
    fn path_exists(&self, path: &Path) -> bool {
        self.fallback_script_available
            && path
                .to_str()
                .map(|value| value.ends_with("scripts/gh-refinedstone.sh"))
                .unwrap_or(false)
    }
    fn ensure_directory_exists(&self, _path: &Path) -> std::io::Result<()> {
        Ok(())
    }
    fn read_dir_paths(&self, _path: &Path) -> std::io::Result<Vec<PathBuf>> {
        Ok(Vec::new())
    }
    fn read_to_string(&self, _path: &Path) -> std::io::Result<String> {
        Ok(String::new())
    }
    fn write_string(&self, _path: &Path, _body: &str) -> std::io::Result<()> {
        Ok(())
    }
    fn rename(&self, _from: &Path, _to: &Path) -> std::io::Result<()> {
        Ok(())
    }
    fn remove_file(&self, _path: &Path) -> std::io::Result<()> {
        Ok(())
    }
}

// gh binary가 없더라도 repo-local RefinedStone fallback이 있고 auth가 통과하면
// parallel mode는 GitHub 자동화 가능 상태로 떠야 한다. 이 테스트는 사용자가 gh를
// 설치하지 않은 환경에서 startup gate가 과하게 막히지 않도록 한다.
#[test]
fn readiness_accepts_repo_github_fallback_when_gh_is_missing() {
    let runtime = FakeReadinessRuntime {
        fallback_script_available: true,
        fallback_auth_ok: true,
        ..Default::default()
    };
    let gh_binary = inspect_gh_binary(&runtime);
    assert_eq!(gh_binary.state, ParallelModeCapabilityState::Ready);
    assert!(gh_binary.detail.contains("RefinedStone API fallback"));
    let gh_auth = inspect_gh_auth(&runtime, &gh_binary, Some("/tmp/repo"));
    assert_eq!(gh_auth.state, ParallelModeCapabilityState::Ready);
    assert_eq!(gh_auth.detail, "GitHub automation authentication succeeded");
}

// distributor/supervisor 테스트는 GitHub side effect의 순서와 branch 인자를 봐야
// 한다. fake port는 실제 네트워크 호출 대신 operations log와 PR metadata를 남겨
// force-push 실패, PR ensure, inspect 흐름을 결정적으로 재현한다.
#[derive(Debug, Clone)]
struct FakeGithubAutomationPort {
    capabilities: GithubAutomationCapabilities,
    ensured_pull_request: GithubAutomationPullRequest,
    base_branch: Arc<Mutex<Option<String>>>,
    head_branch: Arc<Mutex<Option<String>>>,
    operations: Arc<Mutex<Vec<String>>>,
    source_push_error: Arc<Mutex<Option<String>>>,
    force_push_error: Arc<Mutex<Option<String>>>,
    integration_push_error: Arc<Mutex<Option<String>>>,
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
                POOL_BASELINE_BRANCH,
                "placeholder",
                false,
            ),
            base_branch: Arc::new(Mutex::new(None)),
            head_branch: Arc::new(Mutex::new(None)),
            operations: Arc::new(Mutex::new(Vec::new())),
            source_push_error: Arc::new(Mutex::new(None)),
            force_push_error: Arc::new(Mutex::new(None)),
            integration_push_error: Arc::new(Mutex::new(None)),
        }
    }
    fn with_capabilities(capabilities: GithubAutomationCapabilities) -> Self {
        Self {
            capabilities,
            ..Self::ready()
        }
    }

    // force-with-lease 실패는 recovery path에서만 발생시킨다. 일반 push 흐름은
    // 그대로 통과시켜 실패 주입이 다른 GitHub 동작을 가리지 않게 한다.
    fn with_force_push_error(error: &str) -> Self {
        let github = Self::ready();
        *github
            .force_push_error
            .lock()
            .expect("fake github force-push error mutex poisoned") = Some(error.to_string());
        github
    }

    fn with_source_push_error(error: &str) -> Self {
        let github = Self::ready();
        *github
            .source_push_error
            .lock()
            .expect("fake github source-push error mutex poisoned") = Some(error.to_string());
        github
    }

    fn with_integration_push_error(error: &str) -> Self {
        let github = Self::ready();
        *github
            .integration_push_error
            .lock()
            .expect("fake github integration-push error mutex poisoned") = Some(error.to_string());
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
        if !force_with_lease
            && let Some(error) = self
                .source_push_error
                .lock()
                .expect("fake github source-push error mutex poisoned")
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
            .unwrap_or_else(|| POOL_BASELINE_BRANCH.to_string());
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
        if let Some(error) = self
            .integration_push_error
            .lock()
            .expect("fake github integration-push error mutex poisoned")
            .clone()
        {
            anyhow::bail!(error);
        }
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

#[derive(Debug, Clone)]
struct GitBackedGithubAutomationPort {
    capabilities: GithubAutomationCapabilities,
    operations: Arc<Mutex<Vec<String>>>,
    next_pr_number: Arc<Mutex<u64>>,
    pull_requests: Arc<Mutex<BTreeMap<u64, GithubAutomationPullRequest>>>,
}
impl GitBackedGithubAutomationPort {
    fn ready() -> Self {
        Self {
            capabilities: GithubAutomationCapabilities::new(
                ParallelModeCapabilitySnapshot::new(
                    ParallelModeCapabilityKey::PushRemote,
                    ParallelModeCapabilityState::Ready,
                    "local origin push ready",
                    None,
                ),
                ParallelModeCapabilitySnapshot::new(
                    ParallelModeCapabilityKey::GhBinary,
                    ParallelModeCapabilityState::Ready,
                    "test PR facade ready",
                    None,
                ),
                ParallelModeCapabilitySnapshot::new(
                    ParallelModeCapabilityKey::GhAuth,
                    ParallelModeCapabilityState::Ready,
                    "test PR facade authenticated",
                    None,
                ),
            ),
            operations: Arc::new(Mutex::new(Vec::new())),
            next_pr_number: Arc::new(Mutex::new(900)),
            pull_requests: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }
}
impl GithubAutomationPort for GitBackedGithubAutomationPort {
    fn inspect_capabilities(&self, _repo_root: &str) -> GithubAutomationCapabilities {
        self.capabilities.clone()
    }
    fn push_branch(
        &self,
        repo_root: &str,
        branch_name: &str,
        force_with_lease: bool,
    ) -> anyhow::Result<()> {
        self.operations
            .lock()
            .expect("git-backed github operations mutex poisoned")
            .push(format!("push:{branch_name}:{force_with_lease}"));
        let mut args = vec!["push"];
        if force_with_lease {
            args.push("--force-with-lease");
        }
        args.extend([DEFAULT_PUSH_REMOTE_NAME, branch_name]);
        run_git_result(Path::new(repo_root), &args)?;
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
        self.operations
            .lock()
            .expect("git-backed github operations mutex poisoned")
            .push(format!("ensure-pr:{base_branch}:{head_branch}"));
        let mut next_pr_number = self
            .next_pr_number
            .lock()
            .expect("git-backed github PR counter mutex poisoned");
        let pr_number = *next_pr_number;
        *next_pr_number = next_pr_number.saturating_add(1);
        let pull_request = GithubAutomationPullRequest::new(
            pr_number,
            format!("https://example.invalid/pr/{pr_number}"),
            "OPEN",
            base_branch,
            head_branch,
            false,
        );
        self.pull_requests
            .lock()
            .expect("git-backed github PR map mutex poisoned")
            .insert(pr_number, pull_request.clone());
        Ok(pull_request)
    }
    fn inspect_pull_request(
        &self,
        _repo_root: &str,
        pr_number: u64,
    ) -> anyhow::Result<GithubAutomationPullRequest> {
        self.operations
            .lock()
            .expect("git-backed github operations mutex poisoned")
            .push(format!("inspect-pr:{pr_number}"));
        self.pull_requests
            .lock()
            .expect("git-backed github PR map mutex poisoned")
            .get(&pr_number)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("test pull request #{pr_number} was not ensured"))
    }
    fn push_integration_branch(&self, repo_root: &str, branch_name: &str) -> anyhow::Result<()> {
        self.operations
            .lock()
            .expect("git-backed github operations mutex poisoned")
            .push(format!("push-integration:{branch_name}"));
        run_git_result(
            Path::new(repo_root),
            &["push", DEFAULT_PUSH_REMOTE_NAME, branch_name],
        )?;
        Ok(())
    }
    fn close_pull_request(&self, _repo_root: &str, pr_number: u64) -> anyhow::Result<()> {
        self.operations
            .lock()
            .expect("git-backed github operations mutex poisoned")
            .push(format!("close-pr:{pr_number}"));
        Ok(())
    }
}

// 기본 서비스 fixture는 sqlite authority, fake GitHub automation, 실제 git runtime을
// 조합한다. 이렇게 해야 application layer contract는 가짜로 통제하면서 worktree
// 조작은 production adapter 경로와 같은 방식으로 검증된다.
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

fn test_parallel_runtime() -> GitParallelModeRuntimeAdapter {
    GitParallelModeRuntimeAdapter::new()
}

// 세부 시나리오는 dispatcher, pool, supervisor 하위 모듈로 나누되 같은 fixture를
// 공유한다. 이 파일은 공통 contract와 helper가 바뀔 때 전체 parallel_mode 테스트
// 표면이 함께 흔들리도록 묶어 두는 entry point다.
mod distributor;
mod orchestrator_loop;
mod pool;
mod runtime_events;
mod supervisor;

// HTTPS remote parsing은 GitHub automation URL 정규화의 narrow contract다.
// SSH remote는 다른 parser가 명시적으로 다루기 전까지 여기서 None으로 남겨 둔다.
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
