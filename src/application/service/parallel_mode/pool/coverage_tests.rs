use super::*;
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::application::port::outbound::planning_authority_port::{
    NoopPlanningAuthorityPort, PlanningAuthorityPort,
};
use crate::diagnostics::trace_event_log::AKRA_EVENT_TARGET;
use std::process::Command;
use std::sync::Mutex;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

#[derive(Default)]
struct MirrorRuntime {
    existing_paths: BTreeSet<PathBuf>,
    failing_paths: BTreeSet<PathBuf>,
    removed_paths: Mutex<Vec<PathBuf>>,
}

impl MirrorRuntime {
    fn with_existing(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            existing_paths: paths.into_iter().collect(),
            ..Default::default()
        }
    }

    fn with_failure(path: PathBuf) -> Self {
        Self {
            existing_paths: BTreeSet::from([path.clone()]),
            failing_paths: BTreeSet::from([path]),
            removed_paths: Mutex::new(Vec::new()),
        }
    }

    fn removed_paths(&self) -> Vec<PathBuf> {
        self.removed_paths
            .lock()
            .expect("removed path log should not be poisoned")
            .clone()
    }
}

impl ParallelModeRuntimePort for MirrorRuntime {
    fn detect_git_repo_root(&self, workspace_dir: &str) -> Option<String> {
        Some(workspace_dir.to_string())
    }

    fn command_succeeds(&self, _program: &str, _args: &[&str]) -> bool {
        false
    }

    fn run_command(
        &self,
        _program: &str,
        _args: &[&str],
        _current_dir: Option<&str>,
    ) -> Option<String> {
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

    fn find_executable(&self, _program: &str) -> Option<PathBuf> {
        None
    }

    fn gh_auth_status(&self, _repo_root: Option<&str>) -> bool {
        false
    }

    fn current_timestamp(&self) -> String {
        "2026-05-23T00:00:00Z".to_string()
    }

    fn canonicalize_best_effort(&self, path: &Path) -> PathBuf {
        path.to_path_buf()
    }

    fn path_exists(&self, path: &Path) -> bool {
        self.existing_paths.contains(path)
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

    fn remove_file(&self, path: &Path) -> std::io::Result<()> {
        if self.failing_paths.contains(path) {
            return Err(std::io::Error::other("remove failed"));
        }
        self.removed_paths
            .lock()
            .expect("removed path log should not be poisoned")
            .push(path.to_path_buf());
        Ok(())
    }
}

fn lease(
    slot_id: &str,
    task_id: &str,
    state: ParallelModeSlotLeaseState,
    leased_at: &str,
) -> ParallelModeSlotLeaseSnapshot {
    ParallelModeSlotLeaseSnapshot::new(
        slot_id,
        task_id,
        format!("Task {task_id}"),
        format!("agent-{task_id}"),
        format!("akra-agent/{slot_id}/{task_id}"),
        format!("/tmp/{slot_id}"),
        state,
        leased_at,
        None,
    )
}

fn session_detail(
    lease: &ParallelModeSlotLeaseSnapshot,
    thread_id: Option<String>,
    state_label: &str,
    completion_state_label: &str,
) -> ParallelModeAgentSessionDetailSnapshot {
    ParallelModeAgentSessionDetailSnapshot::new(
        lease.session_key(),
        lease.agent_id.clone(),
        lease.task_id.clone(),
        lease.task_title.clone(),
        lease.slot_id.clone(),
        thread_id,
        lease.worktree_path.clone(),
        lease.branch_name.clone(),
        lease.leased_at.clone(),
        state_label,
        completion_state_label,
        "summary",
        "validation",
        "authority",
        None,
        Vec::new(),
        "2026-05-23T00:00:00Z",
    )
}

fn queue_record(
    queue_item_id: &str,
    slot_id: &str,
    task_id: &str,
) -> PlanningAuthorityDistributorQueueRecord {
    PlanningAuthorityDistributorQueueRecord {
        queue_item_id: queue_item_id.to_string(),
        queue_order_key: 1,
        session_key: format!("{slot_id}@2026-05-23T00:00:00Z"),
        slot_id: slot_id.to_string(),
        agent_id: format!("agent-{task_id}"),
        task_id: task_id.to_string(),
        task_title: format!("Task {task_id}"),
        source_branch: POOL_BASELINE_BRANCH.to_string(),
        source_commit_sha: "abcdef1234567890".to_string(),
        branch_name: format!("akra-agent/{slot_id}/{task_id}"),
        worktree_path: format!("/tmp/{slot_id}"),
        commit_sha: "abcdef1234567890".to_string(),
        original_commit_sha: None,
        planning_refresh_state: "done".to_string(),
        integration_state: "queued".to_string(),
        conflict_files: Vec::new(),
        recovery_note: None,
        validation_summary: "validation".to_string(),
        authority_refresh_outcome: "authority".to_string(),
        github_capabilities: None,
        pull_request_number: None,
        pull_request_url: None,
        queue_state: crate::domain::parallel_mode::ParallelModeQueueItemState::Queued,
        integration_note: "queued".to_string(),
        enqueued_at: "2026-05-23T00:00:00Z".to_string(),
        updated_at: "2026-05-23T00:00:00Z".to_string(),
    }
}

fn context_with_runtime_rows(
    leases: Vec<ParallelModeSlotLeaseSnapshot>,
    session_details: Vec<ParallelModeAgentSessionDetailSnapshot>,
    queue_records: Vec<PlanningAuthorityDistributorQueueRecord>,
) -> PoolRuntimeContext {
    PoolRuntimeContext {
        repo_root: "/tmp/repo".to_string(),
        canonical_repo_root: PathBuf::from("/tmp/repo"),
        pool_root: PathBuf::from("/tmp/repo-akra-worktrees/akra-pool"),
        baseline_head: "abcdef1234567890".to_string(),
        worktree_records: Vec::new(),
        slot_leases: leases
            .into_iter()
            .map(|lease| (lease.slot_id.clone(), lease))
            .collect(),
        invalid_slot_leases: BTreeSet::new(),
        session_details,
        task_dispatch_blocks: Vec::new(),
        distributor_queue_records: queue_records,
        runtime_events: Vec::new(),
    }
}

fn reset_report_for_slots(slot_ids: &[&str]) -> ParallelModePoolResetReport {
    let mut report = ParallelModePoolResetReport::new(
        ParallelModePoolResetRunId::new("test-reset"),
        ParallelModePoolResetPolicy::ProtectLive,
    );
    for slot_id in slot_ids {
        report
            .slot_reports
            .push(ParallelModePoolResetSlotReport::new(
                *slot_id,
                ParallelModePoolResetSlotAction::Reset,
                ParallelModePoolResetSlotOutcome::Succeeded,
                "reset",
            ));
    }
    report
}

fn temp_repo_path(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "akra-pool-{prefix}-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ))
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git command should launch");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_temp_git_repo(prefix: &str) -> PathBuf {
    let repo = temp_repo_path(prefix);
    fs::create_dir_all(&repo).expect("repo directory should be created");
    let output = Command::new("git")
        .arg("init")
        .arg(&repo)
        .output()
        .expect("git init should launch");
    assert!(
        output.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    run_git(&repo, &["config", "user.email", "akra@example.test"]);
    run_git(&repo, &["config", "user.name", "Akra Test"]);
    fs::write(repo.join("README.md"), "pool test\n").expect("readme should be written");
    run_git(&repo, &["add", "README.md"]);
    run_git(&repo, &["commit", "-m", "Initial commit"]);
    run_git(&repo, &["branch", "-M", POOL_BASELINE_BRANCH]);
    let remote_ref = remote_tracking_branch_ref(DEFAULT_PUSH_REMOTE_NAME, POOL_BASELINE_BRANCH);
    run_git(
        &repo,
        &["update-ref", remote_ref.as_str(), POOL_BASELINE_BRANCH],
    );
    fs::canonicalize(repo).expect("repo should canonicalize")
}

fn create_detached_pool_slot(repo: &Path, slot_number: usize) -> PathBuf {
    let slot_path = derive_default_pool_root(repo).join(slot_id(slot_number));
    fs::create_dir_all(
        slot_path
            .parent()
            .expect("slot path should have pool parent"),
    )
    .expect("pool parent should be created");
    let slot_path_string = path_string(&slot_path);
    run_git(
        repo,
        &[
            "worktree",
            "add",
            "--detach",
            slot_path_string.as_str(),
            POOL_BASELINE_BRANCH,
        ],
    );
    slot_path
}

fn create_agent_pool_slot(repo: &Path, slot_number: usize, task_slug: &str) -> PathBuf {
    let slot_path = derive_default_pool_root(repo).join(slot_id(slot_number));
    fs::create_dir_all(
        slot_path
            .parent()
            .expect("slot path should have pool parent"),
    )
    .expect("pool parent should be created");
    let branch_name = format!("akra-agent/{}/{task_slug}", slot_id(slot_number));
    let slot_path_string = path_string(&slot_path);
    run_git(
        repo,
        &[
            "worktree",
            "add",
            "-b",
            branch_name.as_str(),
            slot_path_string.as_str(),
            POOL_BASELINE_BRANCH,
        ],
    );
    slot_path
}

fn remove_pool_artifacts(repo: &Path) {
    let pool_root = derive_default_pool_root(repo);
    if let Some(pool_workspace_root) = pool_root.parent().and_then(Path::parent) {
        let _ = fs::remove_dir_all(pool_workspace_root);
    }
    let _ = fs::remove_dir_all(repo);
}

fn with_akra_event_trace<T>(body: impl FnOnce() -> T) -> T {
    let subscriber = tracing_subscriber::registry()
        .with(EnvFilter::new(format!("{AKRA_EVENT_TARGET}=debug")))
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::sink));
    tracing::subscriber::with_default(subscriber, body)
}

#[test]
fn slot_git_status_copy_covers_all_dirty_labels_and_readiness_gates() {
    let dirty = SlotGitStatus {
        has_staged: true,
        has_unstaged: true,
        has_untracked: true,
        has_pending_operation: true,
    };
    let untracked_only = SlotGitStatus {
        has_untracked: true,
        ..Default::default()
    };

    assert_eq!(
        dirty.detail_label(),
        "staged changes, unstaged changes, untracked files, merge/rebase metadata"
    );
    assert!(!dirty.is_clean_baseline());
    assert!(!dirty.is_ready_for_integration());
    assert!(!untracked_only.is_clean_baseline());
    assert!(untracked_only.is_ready_for_integration());
    assert_eq!(SlotGitStatus::default().detail_label(), "clean");
}

#[test]
fn leased_reset_protection_distinguishes_recent_invalid_and_stale_startup_leases() {
    let stale_lease = lease(
        "slot-1",
        "task-1",
        ParallelModeSlotLeaseState::Leased,
        "2020-01-01T00:00:00Z",
    );
    let resettable_detail = session_detail(&stale_lease, None, "assigned", "in_progress");
    let running_like_detail = session_detail(
        &stale_lease,
        Some("thread-1".to_string()),
        "running",
        "in_progress",
    );
    let invalid_timestamp_lease = lease(
        "slot-1",
        "task-1",
        ParallelModeSlotLeaseState::Leased,
        "not-a-timestamp",
    );
    let recent_lease = lease(
        "slot-1",
        "task-1",
        ParallelModeSlotLeaseState::Leased,
        &Utc::now().to_rfc3339(),
    );
    let cleanup_pending_lease = lease(
        "slot-1",
        "task-1",
        ParallelModeSlotLeaseState::CleanupPending,
        "2020-01-01T00:00:00Z",
    );

    assert!(!live_lease_blocks_parallel_entry_reset(
        &stale_lease,
        &[resettable_detail]
    ));
    assert!(live_lease_blocks_parallel_entry_reset(
        &stale_lease,
        &[running_like_detail]
    ));
    assert!(live_lease_blocks_parallel_entry_reset(
        &invalid_timestamp_lease,
        &[]
    ));
    assert!(live_lease_blocks_parallel_entry_reset(&recent_lease, &[]));
    assert!(live_lease_blocks_parallel_entry_reset(
        &cleanup_pending_lease,
        &[]
    ));
}

#[test]
fn reset_projection_helpers_deduplicate_sessions_queues_and_disposable_task_ids() {
    let slot_one = lease(
        "slot-1",
        "task-a",
        ParallelModeSlotLeaseState::Leased,
        "2020-01-01T00:00:00Z",
    );
    let slot_two = lease(
        "slot-2",
        " task-b ",
        ParallelModeSlotLeaseState::Running,
        "2020-01-01T00:01:00Z",
    );
    let slot_one_detail = session_detail(&slot_one, None, "assigned", "in_progress");
    let slot_two_detail = session_detail(&slot_two, None, "assigned", "in_progress");
    let context = context_with_runtime_rows(
        vec![slot_one.clone(), slot_two.clone()],
        vec![
            slot_one_detail.clone(),
            slot_one_detail.clone(),
            slot_two_detail.clone(),
        ],
        vec![
            queue_record("queue-1", "slot-1", "task-a"),
            queue_record("queue-1", "slot-1", "task-a"),
            queue_record("queue-2", "slot-2", "task-c"),
        ],
    );
    let mut report = reset_report_for_slots(&["slot-1"]);

    collect_reset_projection_keys(&mut report, &context, "slot-1");
    collect_reset_projection_keys(&mut report, &context, "slot-1");

    assert_eq!(report.reset_session_keys, vec![slot_one.session_key()]);
    assert_eq!(report.reset_queue_item_ids, vec!["queue-1"]);
    assert_eq!(
        disposable_runtime_task_ids(&context),
        vec!["task-a", "task-b", "task-c"]
    );
}

#[test]
fn clear_pool_runtime_mirrors_removes_present_lease_session_and_queue_paths() {
    let pool_root = PathBuf::from("/tmp/pool");
    let mut report = reset_report_for_slots(&["slot-1"]);
    report.reset_session_keys.push("slot-1@lease".to_string());
    report.reset_queue_item_ids.push("queue-1".to_string());
    let lease_path = slot_lease_file_path(&pool_root, "slot-1");
    let session_path = agent_session_detail_record_path(&pool_root, "slot-1@lease");
    let queue_path = pool_root.join(".distributor-queue").join("queue-1.json");
    let runtime = MirrorRuntime::with_existing([
        lease_path.clone(),
        session_path.clone(),
        queue_path.clone(),
    ]);

    clear_pool_runtime_mirrors_for_report(&runtime, &pool_root, &report)
        .expect("present mirrors should be removed");

    assert_eq!(
        runtime.removed_paths(),
        vec![lease_path, session_path, queue_path]
    );
}

#[test]
fn clear_pool_runtime_mirrors_reports_remove_errors_by_projection_kind() {
    let pool_root = PathBuf::from("/tmp/pool");

    let lease_path = slot_lease_file_path(&pool_root, "slot-1");
    let lease_error = clear_pool_runtime_mirrors_for_report(
        &MirrorRuntime::with_failure(lease_path),
        &pool_root,
        &reset_report_for_slots(&["slot-1"]),
    )
    .expect_err("lease mirror failure should be reported");
    assert!(lease_error.contains("failed to remove reset lease mirror"));

    let session_path = agent_session_detail_record_path(&pool_root, "slot-1@lease");
    let mut session_report = reset_report_for_slots(&[]);
    session_report
        .reset_session_keys
        .push("slot-1@lease".to_string());
    let session_error = clear_pool_runtime_mirrors_for_report(
        &MirrorRuntime::with_failure(session_path),
        &pool_root,
        &session_report,
    )
    .expect_err("session mirror failure should be reported");
    assert!(session_error.contains("failed to remove reset session mirror"));

    let queue_path = pool_root.join(".distributor-queue").join("queue-1.json");
    let mut queue_report = reset_report_for_slots(&[]);
    queue_report
        .reset_queue_item_ids
        .push("queue-1".to_string());
    let queue_error = clear_pool_runtime_mirrors_for_report(
        &MirrorRuntime::with_failure(queue_path),
        &pool_root,
        &queue_report,
    )
    .expect_err("queue mirror failure should be reported");
    assert!(queue_error.contains("failed to remove reset distributor mirror"));
}

#[test]
fn inspection_and_reset_entrypoints_surface_blocked_boards_for_non_git_workspaces() {
    let workspace = std::env::temp_dir().join(format!(
        "akra-pool-non-git-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir_all(&workspace).expect("non-git workspace should be created");
    let workspace_dir = workspace.to_string_lossy().to_string();
    let planning_authority = NoopPlanningAuthorityPort::default();

    let blocked_pool = inspect_pool_board(&planning_authority, &workspace_dir);
    let inspected = inspect_pool_board_and_context(&planning_authority, &workspace_dir)
        .expect_err("non-git workspace should block inspection");
    let reset_error = reset_pool_for_parallel_enable(
        &planning_authority,
        &MirrorRuntime::default(),
        &workspace_dir,
        ParallelModePoolResetPolicy::ProtectLive,
    )
    .expect_err("non-git workspace should block reset");

    assert!(blocked_pool.reconcile_status.contains("git repository"));
    assert_eq!(inspected.1, "repository inspection failed");
    assert_eq!(reset_error, "git repository is unavailable");

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn low_level_context_loaders_report_missing_git_inventory_or_baseline() {
    let workspace = std::env::temp_dir().join(format!(
        "akra-pool-context-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir_all(&workspace).expect("workspace should be created");
    let workspace_dir = workspace.to_string_lossy().to_string();

    assert!(load_worktree_records(&workspace_dir).is_none());
    assert_eq!(
        load_pool_runtime_context_from_roots(
            &NoopPlanningAuthorityPort::default(),
            &workspace_dir,
            &workspace
        )
        .expect_err("missing baseline should block context load"),
        "pool baseline is unavailable during inspection"
    );

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn utility_helpers_cover_pool_ids_heads_and_reconcile_action_flags() {
    assert_eq!(slot_id(7), "slot-7");
    assert_eq!(short_sha("abcdef1234567890"), "abcdef1");

    assert!(!PoolReconcileExecution::default().has_actions());
    assert!(
        PoolReconcileExecution {
            created_baseline_branch: true,
            ..Default::default()
        }
        .has_actions()
    );
    assert!(
        PoolReconcileExecution {
            created_pool_root: true,
            ..Default::default()
        }
        .has_actions()
    );
    assert!(
        PoolReconcileExecution {
            provisioned_slots: 1,
            ..Default::default()
        }
        .has_actions()
    );
    assert!(
        PoolReconcileExecution {
            cleaned_slots: 1,
            ..Default::default()
        }
        .has_actions()
    );

    let repo = init_temp_git_repo("utility");
    let head_sha = resolve_workspace_head_sha(&repo).expect("repo head should resolve");

    assert_eq!(head_sha.len(), 40);
    assert!(resolve_workspace_head_sha(&repo.join("missing")).is_none());

    remove_pool_artifacts(&repo);
}

#[test]
fn resolve_workspace_slot_lease_reports_duplicate_detached_and_branch_mismatch_edges() {
    let adapter = SqlitePlanningAuthorityAdapter::new();

    let duplicate_repo = init_temp_git_repo("duplicate-lease");
    let duplicate_workspace = path_string(&duplicate_repo);
    let mut first_lease = lease(
        "slot-1",
        "task-a",
        ParallelModeSlotLeaseState::Leased,
        "2020-01-01T00:00:00Z",
    );
    first_lease.worktree_path = duplicate_workspace.clone();
    first_lease.branch_name = POOL_BASELINE_BRANCH.to_string();
    let mut second_lease = lease(
        "slot-2",
        "task-b",
        ParallelModeSlotLeaseState::Leased,
        "2020-01-01T00:01:00Z",
    );
    second_lease.worktree_path = duplicate_workspace.clone();
    second_lease.branch_name = POOL_BASELINE_BRANCH.to_string();
    adapter
        .upsert_runtime_slot_lease(&duplicate_workspace, &first_lease)
        .expect("first duplicate lease should be stored");
    adapter
        .upsert_runtime_slot_lease(&duplicate_workspace, &second_lease)
        .expect("second duplicate lease should be stored");

    let duplicate_error = resolve_workspace_slot_lease(&adapter, &duplicate_workspace)
        .expect_err("duplicate worktree lease should be rejected");
    assert!(duplicate_error.contains("matched multiple slot leases"));

    let detached_repo = init_temp_git_repo("detached-lease");
    let detached_workspace = path_string(&detached_repo);
    let mut detached_lease = lease(
        "slot-1",
        "task-detached",
        ParallelModeSlotLeaseState::Leased,
        "2020-01-01T00:02:00Z",
    );
    detached_lease.worktree_path = detached_workspace.clone();
    detached_lease.branch_name = POOL_BASELINE_BRANCH.to_string();
    adapter
        .upsert_runtime_slot_lease(&detached_workspace, &detached_lease)
        .expect("detached lease should be stored");
    run_git(&detached_repo, &["checkout", "--detach", "HEAD"]);

    let detached_error = resolve_workspace_slot_lease(&adapter, &detached_workspace)
        .expect_err("detached workspace should not resolve as a lease owner");
    assert!(
        detached_error.contains("does not currently resolve to a branch")
            || detached_error.contains("is on `HEAD` but slot lease expects"),
        "unexpected detached lease error: {detached_error}"
    );

    let mismatch_repo = init_temp_git_repo("branch-mismatch");
    let mismatch_workspace = path_string(&mismatch_repo);
    let mut mismatch_lease = lease(
        "slot-1",
        "task-mismatch",
        ParallelModeSlotLeaseState::Leased,
        "2020-01-01T00:03:00Z",
    );
    mismatch_lease.worktree_path = mismatch_workspace.clone();
    mismatch_lease.branch_name = "akra-agent/slot-1/task-mismatch".to_string();
    adapter
        .upsert_runtime_slot_lease(&mismatch_workspace, &mismatch_lease)
        .expect("mismatch lease should be stored");

    let mismatch_error = resolve_workspace_slot_lease(&adapter, &mismatch_workspace)
        .expect_err("branch mismatch should be rejected");
    assert!(mismatch_error.contains("slot lease expects `akra-agent/slot-1/task-mismatch`"));

    remove_pool_artifacts(&duplicate_repo);
    remove_pool_artifacts(&detached_repo);
    remove_pool_artifacts(&mismatch_repo);
}

#[test]
fn traced_parallel_enable_reset_covers_live_blocker_and_reset_event_payloads() {
    let repo = init_temp_git_repo("traced-reset-events");
    let workspace = path_string(&repo);
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let live_slot_path = create_agent_pool_slot(&repo, 1, "task-live");
    let reset_slot_path = create_detached_pool_slot(&repo, 2);
    let mut running_lease = lease(
        "slot-1",
        "task-live",
        ParallelModeSlotLeaseState::Running,
        "2020-01-01T00:00:00Z",
    );
    running_lease.worktree_path = path_string(&live_slot_path);
    adapter
        .upsert_runtime_slot_lease(&workspace, &running_lease)
        .expect("running lease should be stored");
    fs::write(reset_slot_path.join("scratch.tmp"), "reset me\n")
        .expect("reset slot scratch file should be written");

    let report = with_akra_event_trace(|| {
        reset_pool_for_parallel_enable(
            &adapter,
            &MirrorRuntime::default(),
            &workspace,
            ParallelModePoolResetPolicy::ProtectLive,
        )
    })
    .expect("protect-live reset should report live blocker and reset idle slot");

    assert_eq!(report.live_blocker_count(), 1);
    assert_eq!(report.succeeded_reset_slot_count(), 1);
    assert!(live_slot_path.join(".git").exists());
    assert!(!reset_slot_path.join("scratch.tmp").exists());

    remove_pool_artifacts(&repo);
}
