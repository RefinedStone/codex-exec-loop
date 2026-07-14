use super::*;

struct LfNormalizationDriftFixture {
    slot_path: PathBuf,
    stale_head: String,
    target_head: String,
}

/*
Build the exact legacy state that made production pool slots look dirty: the old commit stores a
CRLF blob while its attributes require LF, the raw slot file still equals that malformed index
blob, and the next baseline commit renormalizes it. No operator byte edit is involved.
*/
fn create_lf_normalization_drift_fixture(repo: &TempGitRepo) -> LfNormalizationDriftFixture {
    create_lf_normalization_drift_fixture_with_primary(repo, b"legacy\r\n")
}

fn create_lf_normalization_drift_fixture_with_primary(
    repo: &TempGitRepo,
    primary_bytes: &[u8],
) -> LfNormalizationDriftFixture {
    run_git(&repo.repo_root, &["checkout", POOL_BASELINE_BRANCH]);
    let repo_root = repo
        .repo_root
        .to_str()
        .expect("fixture repository path should be utf-8");
    for (path, bytes) in [
        ("legacy.md", primary_bytes),
        ("legacy notes.md", b"notes\r\n".as_slice()),
    ] {
        fs::write(repo.repo_root.join(path), bytes).expect("legacy CRLF fixture should write");
        let raw_crlf_oid = run_command(
            "git",
            [
                "-C",
                repo_root,
                "hash-object",
                "-w",
                "--no-filters",
                "--",
                path,
            ],
            None,
        )
        .expect("raw CRLF blob should be written");
        run_git(
            &repo.repo_root,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                "100644",
                raw_crlf_oid.as_str(),
                path,
            ],
        );
    }
    run_git(&repo.repo_root, &["commit", "-qm", "add legacy CRLF blob"]);
    fs::write(repo.repo_root.join(".gitattributes"), b"*.md text eol=lf\n")
        .expect("LF attributes should write");
    run_git(&repo.repo_root, &["add", ".gitattributes"]);
    if !command_succeeds("git", ["-C", repo_root, "diff", "--cached", "--quiet"]) {
        run_git(
            &repo.repo_root,
            &["commit", "-qm", "declare LF normalization"],
        );
    }
    run_git(
        &repo.repo_root,
        &["push", "-q", DEFAULT_PUSH_REMOTE_NAME, POOL_BASELINE_BRANCH],
    );
    repo.set_remote_tracking_branch(&remote_standard_branch_name(), POOL_BASELINE_BRANCH);

    let stale_head = repo.head_sha();
    let slot_path = repo.create_detached_slot(1);
    fs::write(slot_path.join("legacy.md"), primary_bytes)
        .expect("slot raw bytes should match the malformed index blob");
    fs::write(slot_path.join("legacy notes.md"), b"notes\r\n")
        .expect("spaced slot path should match the malformed index blob");
    let stale_status = inspect_slot_git_status(&slot_path)
        .expect("legacy normalization drift status should be readable");
    assert!(stale_status.has_only_unstaged_changes());

    run_git(
        &repo.repo_root,
        &["add", "--renormalize", "legacy.md", "legacy notes.md"],
    );
    run_git(&repo.repo_root, &["commit", "-qm", "normalize legacy blob"]);
    run_git(
        &repo.repo_root,
        &["push", "-q", DEFAULT_PUSH_REMOTE_NAME, POOL_BASELINE_BRANCH],
    );
    repo.set_remote_tracking_branch(&remote_standard_branch_name(), POOL_BASELINE_BRANCH);

    LfNormalizationDriftFixture {
        slot_path,
        stale_head,
        target_head: repo.head_sha(),
    }
}

fn lf_normalization_test_service() -> ParallelModeService {
    ParallelModeService::new(
        Arc::new(NoopPlanningAuthorityPort::default()),
        Arc::new(FakeGithubAutomationPort::ready()),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    )
    .with_test_delivery_safety_policy(false, false)
}

fn fixture_normalization_quarantine(
    repo: &TempGitRepo,
    fixture: &LfNormalizationDriftFixture,
) -> PathBuf {
    normalization_quarantine_path(&repo.pool_root(), &slot_id(1), &fixture.stale_head)
        .expect("fixture source identity should produce a quarantine path")
}

fn fixture_normalization_replacements(
    repo: &TempGitRepo,
    fixture: &LfNormalizationDriftFixture,
) -> Vec<PathBuf> {
    let prefix = format!(
        ".normalization-replacement-{}-{}-",
        slot_id(1),
        fixture.target_head
    );
    let mut paths = fs::read_dir(repo.pool_root())
        .expect("pool root should be readable")
        .map(|entry| entry.expect("pool entry should be readable"))
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(&prefix))
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn fixture_single_normalization_replacement(
    repo: &TempGitRepo,
    fixture: &LfNormalizationDriftFixture,
) -> PathBuf {
    let paths = fixture_normalization_replacements(repo, fixture);
    assert_eq!(
        paths.len(),
        1,
        "expected one replacement artifact: {paths:?}"
    );
    paths.into_iter().next().expect("replacement should exist")
}

fn assert_normalization_quarantine_preserves_legacy_slot(
    quarantine_path: &Path,
    fixture: &LfNormalizationDriftFixture,
) {
    assert!(quarantine_path.is_dir());
    assert_eq!(
        fs::read(quarantine_path.join("legacy.md"))
            .expect("quarantined legacy file should be readable"),
        b"legacy\r\n"
    );
    assert_eq!(
        fs::read(quarantine_path.join("legacy notes.md"))
            .expect("quarantined spaced legacy file should be readable"),
        b"notes\r\n"
    );
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                quarantine_path
                    .to_str()
                    .expect("quarantine path should be utf-8"),
                "rev-parse",
                "HEAD",
            ],
            None,
        )
        .expect("quarantined slot head should resolve"),
        fixture.stale_head
    );
}

fn unrelated_session_detail(repo: &TempGitRepo) -> ParallelModeAgentSessionDetailSnapshot {
    ParallelModeAgentSessionDetailSnapshot::new(
        "session-elsewhere",
        "agent-elsewhere",
        "task-elsewhere",
        "Task elsewhere",
        slot_id(2),
        Some("thread-elsewhere".to_string()),
        repo.pool_root().join(slot_id(2)).display().to_string(),
        "akra-agent/slot-2/task-elsewhere",
        "2026-07-14T00:00:00Z",
        "running",
        "running",
        "unrelated live session",
        "pending",
        "pending",
        None,
        Vec::new(),
        "2026-07-14T00:00:00Z",
    )
}

fn unrelated_queue_record(repo: &TempGitRepo) -> PlanningAuthorityDistributorQueueRecord {
    PlanningAuthorityDistributorQueueRecord {
        queue_item_id: "queue-elsewhere".to_string(),
        queue_order_key: 1,
        session_key: "session-elsewhere".to_string(),
        slot_id: slot_id(2),
        agent_id: "agent-elsewhere".to_string(),
        task_id: "task-elsewhere".to_string(),
        task_title: "Task elsewhere".to_string(),
        delivery_target: None,
        source_branch: POOL_BASELINE_BRANCH.to_string(),
        source_base_commit_sha: repo.head_sha(),
        source_commit_sha: repo.head_sha(),
        branch_name: "akra-agent/slot-2/task-elsewhere".to_string(),
        worktree_path: repo.pool_root().join(slot_id(2)).display().to_string(),
        commit_sha: repo.head_sha(),
        original_commit_sha: None,
        planning_refresh_state: "pending".to_string(),
        integration_state: "queued".to_string(),
        integration_base_commit_sha: None,
        integration_commit_sha: None,
        conflict_files: Vec::new(),
        recovery_note: None,
        validation_summary: "pending".to_string(),
        authority_refresh_outcome: "pending".to_string(),
        github_capabilities: None,
        pull_request_number: None,
        pull_request_url: None,
        queue_state: ParallelModeQueueItemState::Queued,
        integration_note: "queued elsewhere".to_string(),
        enqueued_at: "2026-07-14T00:00:00Z".to_string(),
        updated_at: "2026-07-14T00:00:00Z".to_string(),
        retry_attempts: 0,
        retry_not_before: None,
    }
}

// pool directory가 아직 만들어지지 않은 상태는 장애가 아니라 초기 준비 상태다.
// board builder는 slot을 임의로 만들지 않고 missing으로만 보고해야 하며, 이때
// exhausted를 켜지 않아 dispatcher가 "용량 소진"과 "아직 provision 안 됨"을 구분한다.
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
fn build_pool_board_blocks_when_pool_baseline_is_missing_during_inspection() {
    let repo = TempGitRepo::new("inspect-missing-baseline");
    repo.delete_local_prerelease_branch();
    repo.delete_remote_standard_tracking_branch();
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

    assert_eq!(pool.blocked_slots, DEFAULT_POOL_SIZE);
    assert!(
        pool.reconcile_status
            .contains("pool runtime state could not be loaded")
    );
    assert!(
        pool.slots
            .iter()
            .all(|slot| slot.worktree_label == "pool baseline is unavailable during inspection")
    );
}

#[test]
fn parallel_enable_reset_reports_non_repository_workspace() {
    let repo = TempGitRepo::new("reset-non-repository");
    let non_repo = repo.root.join("plain-directory");
    fs::create_dir_all(&non_repo).expect("plain directory should be created");
    let service = test_parallel_mode_service();

    let error = service
        .reset_pool_on_parallel_enable_report(
            non_repo.to_str().expect("plain path should be utf-8"),
        )
        .expect_err("non-repository reset should be rejected");

    assert_eq!(error, "git repository is unavailable");
}

#[test]
fn parallel_enable_reset_reports_pool_root_creation_failure() {
    let repo = TempGitRepo::new("reset-pool-root-blocked");
    fs::write(repo.root.join("repo-akra-worktrees"), "not a directory\n")
        .expect("pool root parent should be blocked by a file");
    let service = test_parallel_mode_service();

    let error = service
        .reset_pool_on_parallel_enable_report(&repo.workspace_dir())
        .expect_err("blocked pool root parent should fail reset");

    assert!(error.contains("pool root could not be created"));
}

#[test]
fn parallel_enable_reset_reports_unseedable_agent_branch_baseline() {
    let repo = TempGitRepo::new("reset-agent-branch-baseline");
    repo.delete_local_prerelease_branch();
    repo.delete_remote_standard_tracking_branch();
    run_git(
        &repo.repo_root,
        &["checkout", "-b", "akra-agent/slot-1/manual"],
    );
    let service = test_parallel_mode_service();

    let error = service
        .reset_pool_on_parallel_enable_report(&repo.workspace_dir())
        .expect_err("agent branch should not seed the pool baseline");

    assert!(
        error.starts_with("exact pool integration target could not be fetched safely:"),
        "unexpected exact-fetch error: {error}"
    );
    assert!(
        error.contains("couldn't find remote ref refs/heads/prerelease"),
        "missing integration ref should remain explicit: {error}"
    );
}

#[test]
fn reconcile_reports_pool_root_creation_failure() {
    let repo = TempGitRepo::new("reconcile-pool-root-blocked");
    fs::write(repo.root.join("repo-akra-worktrees"), "not a directory\n")
        .expect("pool root parent should be blocked by a file");

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(pool.blocked_slots, DEFAULT_POOL_SIZE);
    assert!(
        pool.reconcile_status
            .contains("pool root could not be created")
    );
    assert!(
        pool.slots
            .iter()
            .all(|slot| slot.worktree_label == "pool root creation failed")
    );
}

#[test]
fn reconcile_reports_unseedable_agent_branch_baseline() {
    let repo = TempGitRepo::new("reconcile-agent-branch-baseline");
    repo.delete_local_prerelease_branch();
    repo.delete_remote_standard_tracking_branch();
    run_git(
        &repo.repo_root,
        &["checkout", "-b", "akra-agent/slot-1/manual"],
    );

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(pool.blocked_slots, DEFAULT_POOL_SIZE);
    assert!(
        pool.reconcile_status
            .contains("test reconcile blocked / verified fixture target is unavailable")
    );
    assert!(
        pool.slots.iter().all(
            |slot| slot.worktree_label == "test fixture pool integration target is unavailable"
        )
    );
}

// detached `prerelease` worktree는 재사용 가능한 idle baseline이다. branch 이름이
// 실제 local branch가 아니라 detached baseline임을 드러내면서도 slot 하나의
// capacity로 계산되어야 한다.
#[test]
fn detached_prerelease_slot_counts_as_idle_baseline() {
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
    assert_eq!(slot.branch_name, "prerelease (detached)");
    assert_eq!(pool.idle_slots, 1);
    assert_eq!(pool.missing_slots, DEFAULT_POOL_SIZE - 1);
}

// linked worktree git dir에 REBASE_HEAD만 stale하게 남을 수 있다. 실제 rebase 중이면
// rebase-merge/rebase-apply metadata가 함께 있으므로, clean detached baseline slot은
// 단독 REBASE_HEAD 때문에 blocked로 오인되면 안 된다.
#[test]
fn detached_prerelease_slot_with_stale_rebase_head_counts_as_idle_baseline() {
    let repo = TempGitRepo::new("stale-rebase-head-slot");
    let slot_path = repo.create_detached_slot(1);
    let git_dir = run_command(
        "git",
        [
            "-C",
            slot_path.to_str().expect("slot path should be utf-8"),
            "rev-parse",
            "--git-dir",
        ],
        None,
    )
    .expect("slot git dir should resolve");
    fs::write(
        Path::new(git_dir.trim()).join("REBASE_HEAD"),
        repo.head_sha(),
    )
    .expect("stale REBASE_HEAD should be written");
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
    assert_eq!(slot.branch_name, "prerelease (detached)");
    assert_eq!(pool.idle_slots, 1);
    assert_eq!(pool.blocked_slots, 0);
}

// Even a currently integrated orphan cannot be called cleanup-ready by a
// read-only board because the tracking ref has no fetch timestamp/proof.
#[test]
fn read_only_board_blocks_integrated_orphan_without_fetch_proof() {
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

    assert_eq!(slot.state, ParallelModePoolSlotState::Blocked);
    assert!(slot.branch_name.starts_with("akra-agent/slot-1/"));
    assert_eq!(slot.owner_label, "operator recovery");
    assert_eq!(pool.awaiting_cleanup_slots, 0);
    assert!(
        slot.worktree_label
            .contains("integration proof unavailable")
    );
}

#[test]
fn stale_tracking_ref_never_marks_remote_rolled_back_orphan_cleanup_ready() {
    let repo = TempGitRepo::new("stale-read-only-cleanup-proof");
    let slot_path = repo.create_agent_slot(1, "task-one");
    let integration_base = repo.head_sha();
    repo.commit_file_in_slot(&slot_path, "result.txt", "result\n", "agent result");
    repo.merge_agent_slot_into_akra(&slot_path);
    let origin = repo.create_bare_origin_remote();
    run_git(
        &origin,
        &[
            "update-ref",
            local_standard_ref().as_str(),
            &integration_base,
        ],
    );

    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let read_only = build_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &repo.workspace_dir(),
        Some(&readiness),
    );
    assert_eq!(read_only.slots[0].state, ParallelModePoolSlotState::Blocked);
    assert!(
        read_only.slots[0]
            .worktree_label
            .contains("integration proof unavailable")
    );

    let service = test_parallel_mode_service();
    let reconciled = service
        .reconcile_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness))
        .pool;
    assert_eq!(
        reconciled.slots[0].state,
        ParallelModePoolSlotState::Blocked
    );
    assert!(slot_path.join("result.txt").exists());
    assert!(repo.branch_exists("akra-agent/slot-1/task-one"));
}

// lease 없이 남은 agent branch가 아직 merge되지 않았다면 자동으로 지우면 안 된다.
// supervisor는 slot label, reconcile status, top notice에 모두 operator recovery
// 경로를 노출해 사용자가 branch 내용을 먼저 확인하도록 유도한다.
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
            .contains("integration proof unavailable")
    );
    assert!(
        snapshot
            .pool
            .reconcile_status
            .contains("run a remote reconcile fetch before cleanup")
    );
    let notice = snapshot
        .top_notice
        .as_deref()
        .expect("operator recovery notice should be surfaced");
    assert!(notice.contains("pool: blocked"));
    assert!(notice.contains("slot-1"));
    assert!(notice.contains("remote integration proof is unavailable"));
    assert!(notice.contains("run a remote reconcile fetch before cleanup"));
}

// board-only 경로는 사용자의 dirty baseline worktree를 고치지 않는다. detached
// prerelease slot에 unstaged change가 있으면 즉시 blocked로 표시해 reconcile 실행
// 전에도 위험 상태가 TUI에 보이도록 한다.
#[test]
fn dirty_prerelease_baseline_slot_is_blocked_for_operator_recovery() {
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

// lease가 없는 idle detached baseline도 dirty하면 operator-owned data로 보존한다.
#[test]
fn reconcile_resets_dirty_reusable_detached_baseline_slots() {
    let repo = TempGitRepo::new("dirty-reusable-slot");
    let slot_path = repo.create_detached_slot(1);
    fs::write(slot_path.join("README.md"), "dirty\n").expect("slot file should be updated");
    fs::write(slot_path.join("scratch.tmp"), "transient\n")
        .expect("untracked slot residue should be written");
    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE - 1);
    assert_eq!(pool.blocked_slots, 1);
    assert_eq!(
        fs::read_to_string(slot_path.join("README.md")).expect("README should be readable"),
        "dirty\n"
    );
    assert!(slot_path.join("scratch.tmp").exists());
}

#[test]
fn reconcile_recovers_target_equivalent_lf_normalization_drift() {
    let repo = TempGitRepo::new("recover-lf-normalization-drift");
    let fixture = create_lf_normalization_drift_fixture(&repo);
    let quarantine_path = fixture_normalization_quarantine(&repo, &fixture);

    let pool = reconcile_pool_board(
        &NoopPlanningAuthorityPort::default(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE, "pool={pool:#?}");
    assert_eq!(pool.blocked_slots, 0);
    assert_eq!(current_branch(&fixture.slot_path), "HEAD");
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                fixture
                    .slot_path
                    .to_str()
                    .expect("slot path should be utf-8"),
                "rev-parse",
                "HEAD",
            ],
            None,
        )
        .expect("recovered slot head should resolve"),
        fixture.target_head
    );
    assert!(
        inspect_slot_git_status(&fixture.slot_path)
            .expect("recovered slot status should be readable")
            .is_clean_baseline()
    );
    assert_normalization_quarantine_preserves_legacy_slot(&quarantine_path, &fixture);
    assert!(fixture_normalization_replacements(&repo, &fixture).is_empty());
    assert!(
        pool.reconcile_status
            .contains(&quarantine_path.display().to_string())
    );
}

#[test]
fn completed_normalization_quarantine_does_not_prevent_later_slot_reprovisioning() {
    let repo = TempGitRepo::new("reprovision-after-normalization-recovery");
    let fixture = create_lf_normalization_drift_fixture(&repo);
    let quarantine_path = fixture_normalization_quarantine(&repo, &fixture);
    let first_pool = reconcile_pool_board(
        &NoopPlanningAuthorityPort::default(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    assert_eq!(first_pool.idle_slots, DEFAULT_POOL_SIZE);
    run_git(
        &repo.repo_root,
        &[
            "worktree",
            "remove",
            "--force",
            fixture
                .slot_path
                .to_str()
                .expect("slot path should be utf-8"),
        ],
    );

    let second_pool = reconcile_pool_board(
        &NoopPlanningAuthorityPort::default(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(
        second_pool.idle_slots, DEFAULT_POOL_SIZE,
        "pool={second_pool:#?}"
    );
    assert_normalization_quarantine_preserves_legacy_slot(&quarantine_path, &fixture);
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                fixture
                    .slot_path
                    .to_str()
                    .expect("slot path should be utf-8"),
                "rev-parse",
                "HEAD",
            ],
            None,
        )
        .expect("reprovisioned slot head should resolve"),
        fixture.target_head
    );
    assert!(
        second_pool
            .reconcile_status
            .contains(&quarantine_path.display().to_string())
    );
}

#[test]
fn completed_normalization_quarantine_does_not_prevent_a_later_recovery() {
    let repo = TempGitRepo::new("repeat-normalization-recovery");
    let first_fixture = create_lf_normalization_drift_fixture(&repo);
    let first_quarantine = fixture_normalization_quarantine(&repo, &first_fixture);
    let first_pool = reconcile_pool_board(
        &NoopPlanningAuthorityPort::default(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    assert_eq!(first_pool.idle_slots, DEFAULT_POOL_SIZE);
    run_git(
        &repo.repo_root,
        &[
            "worktree",
            "remove",
            "--force",
            first_fixture
                .slot_path
                .to_str()
                .expect("slot path should be utf-8"),
        ],
    );

    let second_fixture = create_lf_normalization_drift_fixture(&repo);
    let second_quarantine = fixture_normalization_quarantine(&repo, &second_fixture);
    let second_pool = reconcile_pool_board(
        &NoopPlanningAuthorityPort::default(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(
        second_pool.idle_slots, DEFAULT_POOL_SIZE,
        "pool={second_pool:#?}"
    );
    assert_eq!(second_pool.blocked_slots, 0);
    assert!(first_quarantine.is_dir());
    assert_normalization_quarantine_preserves_legacy_slot(&second_quarantine, &second_fixture);
    assert!(
        inspect_slot_git_status(&second_fixture.slot_path)
            .expect("second recovered slot status should be readable")
            .is_clean_baseline()
    );
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                second_fixture
                    .slot_path
                    .to_str()
                    .expect("slot path should be utf-8"),
                "rev-parse",
                "HEAD",
            ],
            None,
        )
        .expect("second recovered slot head should resolve"),
        second_fixture.target_head
    );
}

#[test]
fn tracked_artifact_shaped_file_does_not_block_missing_slot_provisioning() {
    let repo = TempGitRepo::new("tracked-artifact-shaped-file");
    run_git(&repo.repo_root, &["checkout", POOL_BASELINE_BRANCH]);
    let artifact_shaped_name = format!(
        ".normalization-replacement-{}-{}-{}",
        slot_id(2),
        "a".repeat(40),
        "b".repeat(32)
    );
    fs::write(
        repo.repo_root.join(&artifact_shaped_name),
        b"tracked repository data\n",
    )
    .expect("artifact-shaped tracked file should write");
    run_git(&repo.repo_root, &["add", artifact_shaped_name.as_str()]);
    run_git(
        &repo.repo_root,
        &["commit", "-qm", "add artifact-shaped tracked file"],
    );
    run_git(
        &repo.repo_root,
        &["push", "-q", DEFAULT_PUSH_REMOTE_NAME, POOL_BASELINE_BRANCH],
    );
    repo.set_remote_tracking_branch(&remote_standard_branch_name(), POOL_BASELINE_BRANCH);

    let pool = reconcile_pool_board(
        &NoopPlanningAuthorityPort::default(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE, "pool={pool:#?}");
    assert_eq!(pool.blocked_slots, 0);
    assert!(
        repo.pool_root()
            .join(slot_id(1))
            .join(artifact_shaped_name)
            .is_file()
    );
}

#[test]
fn reconcile_preserves_trailing_space_edits_on_an_lf_normalization_candidate() {
    let repo = TempGitRepo::new("preserve-trailing-space-lf-normalization-drift");
    let fixture = create_lf_normalization_drift_fixture(&repo);
    fs::write(fixture.slot_path.join("legacy.md"), b"legacy \r\n")
        .expect("trailing-space operator edit should write");

    let pool = reconcile_pool_board(
        &NoopPlanningAuthorityPort::default(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE - 1);
    assert_eq!(pool.blocked_slots, 1);
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                fixture
                    .slot_path
                    .to_str()
                    .expect("slot path should be utf-8"),
                "rev-parse",
                "HEAD",
            ],
            None,
        )
        .expect("preserved slot head should resolve"),
        fixture.stale_head
    );
    assert_eq!(
        fs::read(fixture.slot_path.join("legacy.md"))
            .expect("preserved operator edit should be readable"),
        b"legacy \r\n"
    );
}

#[test]
fn reconcile_preserves_lf_normalization_drift_with_any_invalid_lease_authority() {
    let repo = TempGitRepo::new("preserve-invalid-lease-lf-normalization-drift");
    let fixture = create_lf_normalization_drift_fixture(&repo);
    let authority = NoopPlanningAuthorityPort::default().with_runtime_projection(
        PlanningAuthorityRuntimeProjectionSnapshot {
            invalid_slot_leases: std::collections::BTreeSet::from([slot_id(2)]),
            ..PlanningAuthorityRuntimeProjectionSnapshot::default()
        },
    );

    let pool = reconcile_pool_board(&authority, &test_parallel_runtime(), &repo.workspace_dir());

    assert!(pool.blocked_slots >= 1);
    assert_eq!(pool.slots[0].owner_label, "operator recovery");
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                fixture
                    .slot_path
                    .to_str()
                    .expect("slot path should be utf-8"),
                "rev-parse",
                "HEAD",
            ],
            None,
        )
        .expect("invalid-lease slot head should resolve"),
        fixture.stale_head
    );
}

#[test]
fn reconcile_preserves_lf_normalization_drift_with_any_session_or_queue_authority() {
    for authority_kind in ["session", "queue"] {
        let repo = TempGitRepo::new(&format!("preserve-{authority_kind}-lf-normalization-drift"));
        let fixture = create_lf_normalization_drift_fixture(&repo);
        let mut snapshot = PlanningAuthorityRuntimeProjectionSnapshot::default();
        match authority_kind {
            "session" => snapshot
                .session_details
                .push(unrelated_session_detail(&repo)),
            "queue" => snapshot
                .distributor_queue_records
                .push(unrelated_queue_record(&repo)),
            _ => unreachable!(),
        }
        let authority = NoopPlanningAuthorityPort::default().with_runtime_projection(snapshot);

        let pool =
            reconcile_pool_board(&authority, &test_parallel_runtime(), &repo.workspace_dir());

        assert_eq!(pool.slots[0].state, ParallelModePoolSlotState::Blocked);
        assert_eq!(
            run_command(
                "git",
                [
                    "-C",
                    fixture
                        .slot_path
                        .to_str()
                        .expect("slot path should be utf-8"),
                    "rev-parse",
                    "HEAD",
                ],
                None,
            )
            .expect("authority-protected slot head should resolve"),
            fixture.stale_head,
            "authority kind {authority_kind} must block normalization recovery"
        );
    }
}

#[test]
fn normalization_recovery_quarantines_a_late_staged_write() {
    let repo = TempGitRepo::new("quarantine-late-staged-write");
    let fixture = create_lf_normalization_drift_fixture(&repo);
    let quarantine_path = fixture_normalization_quarantine(&repo, &fixture);
    install_before_normalization_quarantine_hook(&fixture.slot_path, |slot_path| {
        fs::write(slot_path.join("legacy.md"), b"late staged write\r\n")
            .expect("late staged write should update the legacy slot");
        run_git(slot_path, &["add", "legacy.md"]);
    });

    let pool = reconcile_pool_board(
        &NoopPlanningAuthorityPort::default(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE, "pool={pool:#?}");
    assert!(
        inspect_slot_git_status(&fixture.slot_path)
            .expect("replacement slot status should be readable")
            .is_clean_baseline()
    );
    assert_eq!(
        fs::read(quarantine_path.join("legacy.md"))
            .expect("late write should remain in quarantine"),
        b"late staged write\r\n"
    );
    assert!(!command_succeeds(
        "git",
        [
            "-C",
            quarantine_path
                .to_str()
                .expect("quarantine path should be utf-8"),
            "diff",
            "--cached",
            "--quiet",
        ],
    ));
    assert!(fixture_normalization_replacements(&repo, &fixture).is_empty());
    assert!(
        pool.reconcile_status
            .contains(&quarantine_path.display().to_string())
    );
}

#[test]
fn normalization_recovery_never_checkouts_over_a_late_canonical_path_write() {
    let repo = TempGitRepo::new("preserve-post-move-canonical-write");
    let fixture = create_lf_normalization_drift_fixture(&repo);
    let quarantine_path = fixture_normalization_quarantine(&repo, &fixture);
    install_after_normalization_quarantine_move_hook(&fixture.slot_path, |slot_path| {
        fs::create_dir_all(slot_path).expect("late canonical directory should be recreated");
        fs::write(slot_path.join("legacy.md"), b"late canonical bytes\n")
            .expect("late canonical bytes should write");
    });

    let pool = reconcile_pool_board(
        &NoopPlanningAuthorityPort::default(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    let replacement_path = fixture_single_normalization_replacement(&repo, &fixture);

    assert_eq!(
        fs::read(fixture.slot_path.join("legacy.md")).expect("late canonical bytes must remain"),
        b"late canonical bytes\n"
    );
    assert_normalization_quarantine_preserves_legacy_slot(&quarantine_path, &fixture);
    assert!(replacement_path.is_dir());
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                replacement_path
                    .to_str()
                    .expect("replacement path should be utf-8"),
                "rev-parse",
                "HEAD",
            ],
            None,
        )
        .expect("staged replacement head should resolve"),
        fixture.target_head
    );
    assert!(
        inspect_slot_git_status(&replacement_path)
            .expect("staged replacement should be inspectable")
            .is_clean_baseline()
    );
    assert!(
        pool.reconcile_status
            .contains("incomplete normalization recovery")
    );
}

#[cfg(unix)]
#[test]
fn normalization_recovery_rejects_a_swapped_staging_identity_before_checkout() {
    use std::os::unix::fs::symlink;

    let repo = TempGitRepo::new("preserve-swapped-normalization-staging");
    let fixture = create_lf_normalization_drift_fixture(&repo);
    let quarantine_path = fixture_normalization_quarantine(&repo, &fixture);
    let outside_path = repo.root.join("outside-staging-target");
    let displaced_path = repo.root.join("displaced-empty-staging");
    fs::create_dir(&outside_path).expect("outside staging target should be created");
    let outside_for_hook = outside_path.clone();
    let displaced_for_hook = displaced_path.clone();
    install_before_normalization_staging_provision_hook(
        &fixture.slot_path,
        move |replacement_path| {
            fs::rename(replacement_path, &displaced_for_hook)
                .expect("staging directory should be displaced");
            symlink(&outside_for_hook, replacement_path)
                .expect("staging path should be replaced by a symlink");
        },
    );

    let pool = reconcile_pool_board(
        &NoopPlanningAuthorityPort::default(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    let replacement_path = fixture_single_normalization_replacement(&repo, &fixture);

    assert!(
        fs::symlink_metadata(&replacement_path)
            .expect("swapped staging symlink should remain")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_dir(&outside_path)
            .expect("outside staging target should remain readable")
            .count(),
        0
    );
    assert_eq!(
        fs::read_dir(&displaced_path)
            .expect("displaced staging should remain readable")
            .count(),
        0
    );
    assert!(!quarantine_path.exists());
    assert_eq!(current_branch(&fixture.slot_path), "HEAD");
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                fixture
                    .slot_path
                    .to_str()
                    .expect("slot path should be utf-8"),
                "rev-parse",
                "HEAD",
            ],
            None,
        )
        .expect("legacy slot head should resolve"),
        fixture.stale_head
    );
    assert_eq!(pool.slots[0].state, ParallelModePoolSlotState::Blocked);
    assert!(
        pool.reconcile_status
            .contains(&replacement_path.display().to_string())
    );
}

#[cfg(unix)]
#[test]
fn normalization_recovery_atomic_install_does_not_follow_a_late_canonical_symlink() {
    use std::os::unix::fs::symlink;

    let repo = TempGitRepo::new("preserve-late-canonical-symlink");
    let fixture = create_lf_normalization_drift_fixture(&repo);
    let quarantine_path = fixture_normalization_quarantine(&repo, &fixture);
    let outside_path = repo.root.join("outside-canonical-target");
    fs::create_dir(&outside_path).expect("outside canonical target should be created");
    let outside_for_hook = outside_path.clone();
    install_before_normalization_atomic_rename_hook(&fixture.slot_path, move |destination| {
        symlink(&outside_for_hook, destination).expect("late canonical symlink should be created");
    });

    let pool = reconcile_pool_board(
        &NoopPlanningAuthorityPort::default(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    let replacement_path = fixture_single_normalization_replacement(&repo, &fixture);

    assert!(
        fs::symlink_metadata(&fixture.slot_path)
            .expect("late canonical symlink should remain")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_link(&fixture.slot_path).expect("canonical symlink target should be readable"),
        outside_path
    );
    assert_eq!(
        fs::read_dir(&outside_path)
            .expect("outside target should remain readable")
            .count(),
        0
    );
    assert_normalization_quarantine_preserves_legacy_slot(&quarantine_path, &fixture);
    assert!(replacement_path.is_dir());
    assert!(
        pool.reconcile_status
            .contains("incomplete normalization recovery")
    );
}

#[cfg(unix)]
#[test]
fn normalization_recovery_atomic_quarantine_does_not_follow_a_late_symlink() {
    use std::os::unix::fs::symlink;

    let repo = TempGitRepo::new("preserve-late-quarantine-symlink");
    let fixture = create_lf_normalization_drift_fixture(&repo);
    let quarantine_path = fixture_normalization_quarantine(&repo, &fixture);
    let outside_path = repo.root.join("outside-quarantine-target");
    fs::create_dir(&outside_path).expect("outside quarantine target should be created");
    let outside_for_hook = outside_path.clone();
    install_before_normalization_atomic_rename_hook(&quarantine_path, move |destination| {
        symlink(&outside_for_hook, destination).expect("late quarantine symlink should be created");
    });

    let pool = reconcile_pool_board(
        &NoopPlanningAuthorityPort::default(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    let replacement_path = fixture_single_normalization_replacement(&repo, &fixture);

    assert_eq!(current_branch(&fixture.slot_path), "HEAD");
    assert_eq!(
        fs::read(fixture.slot_path.join("legacy.md"))
            .expect("legacy canonical bytes should remain"),
        b"legacy\r\n"
    );
    assert!(
        fs::symlink_metadata(&quarantine_path)
            .expect("late quarantine symlink should remain")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_dir(&outside_path)
            .expect("outside target should remain readable")
            .count(),
        0
    );
    assert!(replacement_path.is_dir());
    assert_eq!(pool.slots[0].state, ParallelModePoolSlotState::Blocked);
}

#[test]
fn normalization_recovery_preserves_a_late_branch_checkout() {
    let repo = TempGitRepo::new("preserve-late-branch-checkout");
    let fixture = create_lf_normalization_drift_fixture(&repo);
    let quarantine_path = fixture_normalization_quarantine(&repo, &fixture);
    install_before_normalization_quarantine_hook(&fixture.slot_path, |slot_path| {
        run_git(slot_path, &["switch", "-c", "operator-late-branch"]);
    });

    let pool = reconcile_pool_board(
        &NoopPlanningAuthorityPort::default(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    let replacement_path = fixture_single_normalization_replacement(&repo, &fixture);

    assert_eq!(current_branch(&fixture.slot_path), "operator-late-branch");
    assert_eq!(
        fs::read(fixture.slot_path.join("legacy.md"))
            .expect("late branch legacy bytes should remain"),
        b"legacy\r\n"
    );
    assert!(!quarantine_path.exists());
    assert!(replacement_path.is_dir());
    assert_eq!(pool.slots[0].state, ParallelModePoolSlotState::Blocked);
    assert!(
        pool.reconcile_status
            .contains(&replacement_path.display().to_string())
    );
}

#[test]
fn normalization_recovery_stops_when_authority_appears_after_the_initial_proof() {
    let repo = TempGitRepo::new("preserve-late-normalization-authority");
    let fixture = create_lf_normalization_drift_fixture(&repo);
    let quarantine_path = fixture_normalization_quarantine(&repo, &fixture);
    let session_detail = unrelated_session_detail(&repo);
    let shared_projection = Arc::new(Mutex::new(
        PlanningAuthorityRuntimeProjectionSnapshot::default(),
    ));
    let authority = NoopPlanningAuthorityPort::default()
        .with_shared_runtime_projection(Arc::clone(&shared_projection));
    install_before_normalization_quarantine_hook(&fixture.slot_path, move |_| {
        shared_projection
            .lock()
            .expect("shared projection should not be poisoned")
            .session_details
            .push(session_detail);
    });

    let pool = reconcile_pool_board(&authority, &test_parallel_runtime(), &repo.workspace_dir());
    let replacement_path = fixture_single_normalization_replacement(&repo, &fixture);

    assert!(!quarantine_path.exists());
    assert!(replacement_path.is_dir());
    assert_eq!(current_branch(&fixture.slot_path), "HEAD");
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                fixture
                    .slot_path
                    .to_str()
                    .expect("slot path should be utf-8"),
                "rev-parse",
                "HEAD",
            ],
            None,
        )
        .expect("authority-protected slot head should resolve"),
        fixture.stale_head
    );
    assert!(
        pool.reconcile_status
            .contains(&replacement_path.display().to_string())
    );
}

#[test]
fn normalization_recovery_rejects_an_oversized_candidate() {
    let repo = TempGitRepo::new("preserve-oversized-normalization-candidate");
    let mut oversized = vec![b'x'; 1024 * 1024];
    oversized.extend_from_slice(b"\r\n");
    let fixture = create_lf_normalization_drift_fixture_with_primary(&repo, &oversized);

    let pool = reconcile_pool_board(
        &NoopPlanningAuthorityPort::default(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(pool.slots[0].state, ParallelModePoolSlotState::Blocked);
    assert_eq!(
        fs::metadata(fixture.slot_path.join("legacy.md"))
            .expect("oversized candidate should remain")
            .len(),
        oversized.len() as u64
    );
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                fixture
                    .slot_path
                    .to_str()
                    .expect("slot path should be utf-8"),
                "rev-parse",
                "HEAD",
            ],
            None,
        )
        .expect("oversized slot head should resolve"),
        fixture.stale_head
    );
}

#[cfg(any(unix, windows))]
#[test]
fn normalization_recovery_preserves_a_hardlinked_candidate() {
    let repo = TempGitRepo::new("preserve-hardlinked-normalization-candidate");
    let fixture = create_lf_normalization_drift_fixture(&repo);
    let shared_path = repo.root.join("shared-legacy.md");
    fs::hard_link(fixture.slot_path.join("legacy.md"), &shared_path)
        .expect("legacy candidate should be hardlinked");

    let pool = reconcile_pool_board(
        &NoopPlanningAuthorityPort::default(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(pool.slots[0].state, ParallelModePoolSlotState::Blocked);
    assert_eq!(
        fs::read(&shared_path).expect("shared file should remain"),
        b"legacy\r\n"
    );
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                fixture
                    .slot_path
                    .to_str()
                    .expect("slot path should be utf-8"),
                "rev-parse",
                "HEAD",
            ],
            None,
        )
        .expect("hardlinked slot head should resolve"),
        fixture.stale_head
    );
}

#[test]
fn parallel_entry_reset_recovers_target_equivalent_lf_normalization_drift() {
    let repo = TempGitRepo::new("parallel-entry-lf-normalization-drift");
    let fixture = create_lf_normalization_drift_fixture(&repo);
    let quarantine_path = fixture_normalization_quarantine(&repo, &fixture);

    let report = lf_normalization_test_service()
        .reset_pool_on_parallel_enable_report(&repo.workspace_dir())
        .expect("parallel entry should recover target-equivalent normalization drift");

    assert!(report.succeeded_reset_slot_ids().contains(&slot_id(1)));
    assert_eq!(current_branch(&fixture.slot_path), "HEAD");
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                fixture
                    .slot_path
                    .to_str()
                    .expect("slot path should be utf-8"),
                "rev-parse",
                "HEAD",
            ],
            None,
        )
        .expect("reset slot head should resolve"),
        fixture.target_head
    );
    assert!(
        inspect_slot_git_status(&fixture.slot_path)
            .expect("reset slot status should be readable")
            .is_clean_baseline()
    );
    assert_normalization_quarantine_preserves_legacy_slot(&quarantine_path, &fixture);
    assert!(fixture_normalization_replacements(&repo, &fixture).is_empty());
    assert!(
        report
            .slot_reports
            .iter()
            .any(|slot| slot.reason.contains(&quarantine_path.display().to_string()))
    );
}

#[test]
fn parallel_entry_reset_does_not_reenter_an_incomplete_normalization_recovery() {
    let repo = TempGitRepo::new("parallel-entry-incomplete-normalization-recovery");
    let fixture = create_lf_normalization_drift_fixture(&repo);
    let artifact_path = repo.pool_root().join(format!(
        ".normalization-replacement-{}-{}-{}",
        slot_id(1),
        fixture.target_head,
        "00".repeat(16)
    ));
    fs::create_dir(&artifact_path).expect("incomplete replacement artifact should be created");

    let report = lf_normalization_test_service()
        .reset_pool_on_parallel_enable_report(&repo.workspace_dir())
        .expect("parallel entry should preserve an incomplete normalization recovery");

    assert!(!report.succeeded_reset_slot_ids().contains(&slot_id(1)));
    assert_eq!(
        fixture_normalization_replacements(&repo, &fixture),
        vec![artifact_path]
    );
    assert_eq!(current_branch(&fixture.slot_path), "HEAD");
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                fixture
                    .slot_path
                    .to_str()
                    .expect("slot path should be utf-8"),
                "rev-parse",
                "HEAD",
            ],
            None,
        )
        .expect("legacy slot head should resolve"),
        fixture.stale_head
    );
}

// 한 slot이 running인 동안에도 다른 idle baseline들은 표준 remote branch로 정리될 수
// 있어야 한다. 이 테스트는 실행 중인 lease를 보존하면서 reusable slot만 reset하고,
// canonical 표준 ref가 현재 작업 branch HEAD로 이동하지 않는지도 함께 확인한다.
#[test]
fn reconcile_resets_reusable_detached_slots_while_another_slot_is_running() {
    let repo = TempGitRepo::new("dirty-reusable-slot-with-running-lease");
    let service = test_parallel_mode_service();
    let origin_prerelease_head = run_command(
        "git",
        [
            "-C",
            repo.repo_root.to_str().expect("repo root should be utf-8"),
            "rev-parse",
            &remote_standard_tracking_ref(),
        ],
        None,
    )
    .expect("origin prerelease should resolve");
    let initial_pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    assert_eq!(initial_pool.idle_slots, DEFAULT_POOL_SIZE);
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    service
        .mark_slot_running(&repo.workspace_dir(), &lease.slot_id, "agent-1")
        .expect("slot lease should transition to running");
    repo.commit_on_current_branch("baseline.txt", "new baseline\n", "advance baseline");
    let current_head = repo.head_sha();
    assert_ne!(origin_prerelease_head, current_head);
    let reusable_slot_path = repo.pool_root().join(slot_id(2));
    fs::write(reusable_slot_path.join("README.md"), "dirty\n")
        .expect("idle slot should become dirty");
    let refreshed_pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(refreshed_pool.running_slots, 1);
    assert_eq!(refreshed_pool.idle_slots, DEFAULT_POOL_SIZE - 2);
    assert_eq!(refreshed_pool.blocked_slots, 1);
    assert_eq!(
        fs::read_to_string(reusable_slot_path.join("README.md"))
            .expect("README should be readable"),
        "dirty\n"
    );
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.repo_root.to_str().expect("repo root should be utf-8"),
                "rev-parse",
                POOL_BASELINE_BRANCH,
            ],
            None,
        )
        .expect("prerelease should resolve"),
        origin_prerelease_head
    );
}

// parallel mode를 off -> on으로 켜더라도 Running lease는 live execution 증거다.
// reset은 projection을 지우거나 worktree를 되돌리지 않고 blocked report만 남긴다.
#[test]
fn parallel_entry_from_off_preserves_live_running_slot_and_resets_idle_slots() {
    let repo = TempGitRepo::new("parallel-enable-reset-active");
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
    repo.commit_file_in_slot(&slot_path, "stale.txt", "stale\n", "stale agent work");
    fs::write(slot_path.join("scratch.tmp"), "discard me\n").expect("scratch file should write");

    let report = service
        .reset_pool_on_parallel_enable_report(&repo.workspace_dir())
        .expect("parallel enable reset should report live blockers");
    let snapshot = service.build_supervisor_snapshot(
        &repo.workspace_dir(),
        true,
        Some(&ParallelModeReadinessSnapshot::new(
            repo.workspace_dir(),
            ParallelModeReadinessState::Ready,
            Vec::new(),
            None,
        )),
    );

    assert_eq!(report.live_blocker_count(), 1);
    assert_eq!(report.succeeded_reset_slot_count(), DEFAULT_POOL_SIZE - 1);
    assert_eq!(
        report.slot_reports[0].action,
        ParallelModePoolResetSlotAction::PreserveLive
    );
    assert_eq!(
        report.slot_reports[0].outcome,
        ParallelModePoolResetSlotOutcome::Blocked
    );
    assert_eq!(snapshot.roster.active_count(), 1);
    assert!(repo.slot_lease_path(1).exists());
    assert!(slot_path.join("stale.txt").exists());
    assert!(slot_path.join("scratch.tmp").exists());
    assert!(current_branch(&slot_path).starts_with("akra-agent/slot-1/"));
}

#[test]
fn parallel_entry_from_off_preserves_recent_leased_slot_with_invalid_timestamp() {
    let repo = TempGitRepo::new("parallel-enable-invalid-lease-timestamp");
    let service = test_parallel_mode_service();
    let mut lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    lease.leased_at = "not-a-timestamp".to_string();
    write_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &lease,
    )
    .expect("invalid timestamp lease should be persisted");
    let slot_path = PathBuf::from(lease.worktree_path.clone());

    let report = service
        .reset_pool_on_parallel_enable_report(&repo.workspace_dir())
        .expect("parallel enable reset should preserve invalid timestamp startup lease");

    assert_eq!(report.live_blocker_count(), 1);
    assert_eq!(report.succeeded_reset_slot_count(), DEFAULT_POOL_SIZE - 1);
    assert_eq!(
        report.slot_reports[0].action,
        ParallelModePoolResetSlotAction::PreserveLive
    );
    assert!(report.slot_reports[0].reason.contains("live leased lease"));
    assert!(repo.slot_lease_path(1).exists());
    assert!(repo.branch_exists(&lease.branch_name));
    assert!(current_branch(&slot_path).starts_with("akra-agent/slot-1/"));
}

#[test]
fn parallel_entry_reset_preserves_clean_split_brain_running_lease() {
    let repo = TempGitRepo::new("parallel-reset-clean-split-brain");
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
    run_git(&slot_path, &["checkout", "--detach", POOL_BASELINE_BRANCH]);
    run_git(
        &repo.repo_root,
        &["branch", "-D", lease.branch_name.as_str()],
    );

    let report = service
        .reset_pool_on_parallel_enable_report(&repo.workspace_dir())
        .expect("split-brain running lease should remain protected");

    assert_eq!(report.live_blocker_count(), 1);
    assert_eq!(report.succeeded_reset_slot_count(), DEFAULT_POOL_SIZE - 1);
    assert!(repo.slot_lease_path(1).exists());
    assert!(!report.succeeded_reset_slot_ids().contains(&lease.slot_id));
}

// TUI 프로세스에서 처음 `:parallel`을 켜는 초기 설정은 이전 실행의 stale
// projection을 신뢰하지 않는다. failed/cleaned worker가 Running lease를 남겨도
// disposable pool 전체를 현재 prerelease baseline으로 강제 정렬해야 한다.
#[test]
fn parallel_initial_setup_forces_live_running_slots_back_to_baseline() {
    let repo = TempGitRepo::new("parallel-initial-force-reset-active");
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
    repo.commit_file_in_slot(&slot_path, "stale.txt", "stale\n", "stale agent work");
    fs::write(slot_path.join("scratch.tmp"), "discard me\n").expect("scratch file should write");

    let report = service
        .reset_pool_on_parallel_initial_setup_report(&repo.workspace_dir())
        .expect("initial setup reset should force disposable slots");
    let snapshot = service.build_supervisor_snapshot(
        &repo.workspace_dir(),
        true,
        Some(&ParallelModeReadinessSnapshot::new(
            repo.workspace_dir(),
            ParallelModeReadinessState::Ready,
            Vec::new(),
            None,
        )),
    );

    assert_eq!(report.policy, ParallelModePoolResetPolicy::ForceDisposable);
    assert_eq!(report.live_blocker_count(), 1);
    assert_eq!(report.succeeded_reset_slot_count(), DEFAULT_POOL_SIZE - 1);
    assert_eq!(snapshot.roster.active_count(), 1);
    assert!(repo.slot_lease_path(1).exists());
    assert!(slot_path.join("stale.txt").exists());
    assert!(slot_path.join("scratch.tmp").exists());
    assert_eq!(current_branch(&slot_path), lease.branch_name);
}

// 초기 설정 전에 pool worktree가 사라졌더라도 durable Running lease를 stale로 추측해 지우면
// 늦은 worker 결과나 교체 세대를 잃을 수 있다. ForceDisposable도 active lease와 연결된 runtime
// projection은 보존하고 operator recovery 대상으로 남겨야 한다.
#[test]
fn parallel_initial_setup_preserves_active_runtime_when_pool_worktree_is_missing() {
    let repo = TempGitRepo::new("parallel-initial-clears-missing-runtime");
    let service = test_parallel_mode_service();
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    record_assigned_session_detail(
        &adapter,
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &lease,
    )
    .expect("stale session detail should be recorded");
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &repo.workspace_dir(),
        &PlanningAuthorityDistributorQueueRecord {
            queue_item_id: "stale-queue-1".to_string(),
            queue_order_key: 1,
            session_key: lease.session_key(),
            slot_id: lease.slot_id.clone(),
            agent_id: lease.agent_id.clone(),
            task_id: lease.task_id.clone(),
            task_title: lease.task_title.clone(),
            delivery_target: None,
            source_branch: "prerelease".to_string(),
            source_base_commit_sha: "base".to_string(),
            source_commit_sha: repo.head_sha(),
            branch_name: lease.branch_name.clone(),
            worktree_path: lease.worktree_path.clone(),
            commit_sha: repo.head_sha(),
            original_commit_sha: None,
            planning_refresh_state: "failed".to_string(),
            integration_state: "blocked".to_string(),
            integration_base_commit_sha: None,
            integration_commit_sha: None,
            conflict_files: Vec::new(),
            recovery_note: Some("stale queue from previous runtime".to_string()),
            validation_summary: "stale validation".to_string(),
            authority_refresh_outcome: "stale official refresh".to_string(),
            github_capabilities: None,
            pull_request_number: None,
            pull_request_url: None,
            queue_state: ParallelModeQueueItemState::Blocked,
            integration_note: "stale blocked queue".to_string(),
            enqueued_at: "2026-05-08T08:55:15.467459643+00:00".to_string(),
            updated_at: "2026-05-08T09:10:40.820469463+00:00".to_string(),
            retry_attempts: 0,
            retry_not_before: None,
        },
    )
    .expect("stale distributor queue should persist");
    SqlitePlanningAuthorityAdapter::enqueue_runtime_dispatch_command(
        &repo.workspace_dir(),
        &ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
            ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            Some("stale-head".to_string()),
            Some(1),
            "2026-05-08T09:10:40.820469463+00:00",
        ),
    )
    .expect("stale dispatch command should persist");
    SqlitePlanningAuthorityAdapter::upsert_runtime_task_dispatch_block(
        &repo.workspace_dir(),
        &ParallelModeTaskDispatchBlockSnapshot::new(
            lease.task_id.as_str(),
            "2026-05-09T08:31:40Z",
            "2026-05-09T20:32:57.657951438+00:00",
            ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
        ),
    )
    .expect("stale dispatch block should persist");
    run_git(
        &repo.repo_root,
        &[
            "worktree",
            "remove",
            "--force",
            lease.worktree_path.as_str(),
        ],
    );

    let report = service
        .reset_pool_on_parallel_initial_setup_report(&repo.workspace_dir())
        .expect("initial setup reset should preserve active runtime for missing slots");
    let snapshot = service.reconcile_supervisor_snapshot(
        &repo.workspace_dir(),
        true,
        Some(&ParallelModeReadinessSnapshot::new(
            repo.workspace_dir(),
            ParallelModeReadinessState::Ready,
            Vec::new(),
            None,
        )),
    );
    let runtime_projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
            .expect("runtime projections should load");

    assert_eq!(report.policy, ParallelModePoolResetPolicy::ForceDisposable);
    assert_eq!(report.live_blocker_count(), 1);
    assert_eq!(report.succeeded_reset_slot_count(), DEFAULT_POOL_SIZE - 1);
    assert_eq!(snapshot.roster.active_count(), 1);
    assert_eq!(snapshot.pool.idle_slots, DEFAULT_POOL_SIZE - 1);
    assert_eq!(snapshot.pool.blocked_slots, 1);
    let persisted_lease = runtime_projection
        .slot_leases
        .get(&lease.slot_id)
        .expect("active missing-worktree lease should remain");
    assert!(persisted_lease.same_generation_as(&lease));
    assert_eq!(persisted_lease.state, ParallelModeSlotLeaseState::Running);
    assert!(!runtime_projection.session_details.is_empty());
    assert!(!runtime_projection.distributor_queue_records.is_empty());
    assert!(
        runtime_projection.dispatch_commands.is_empty(),
        "mode entry may cancel stale pending dispatch commands without deleting active lease ownership"
    );
    assert!(!runtime_projection.task_dispatch_blocks.is_empty());
    assert!(repo.slot_lease_path(1).exists());
    assert!(repo.session_detail_path(&lease.session_key()).exists());
    assert!(
        !Path::new(&lease.worktree_path).exists(),
        "reconcile must not recreate a missing worktree while its lease is active"
    );
}

// Dirty tracked files in no-lease reusable slots must not stop off -> on pool reset. Git checkout
// without --force can fail before reset --hard runs when a slot has committed and uncommitted edits
// to the same tracked file, which leaves the slot detached at stale work.
#[test]
fn parallel_entry_from_off_forces_dirty_no_lease_slot_back_to_baseline() {
    let repo = TempGitRepo::new("parallel-enable-reset-dirty-tracked");
    let service = test_parallel_mode_service();
    let initial_pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    assert_eq!(initial_pool.idle_slots, DEFAULT_POOL_SIZE);
    let slot_path = repo.pool_root().join(slot_id(1));
    fs::write(slot_path.join("README.md"), "dirty local version\n")
        .expect("dirty tracked file should write");

    let reset_count = service
        .reset_pool_on_parallel_enable(&repo.workspace_dir())
        .expect("parallel enable reset should force dirty tracked slots back to baseline");

    assert_eq!(reset_count, DEFAULT_POOL_SIZE - 1);
    assert!(!repo.slot_lease_path(1).exists());
    assert_eq!(
        fs::read_to_string(slot_path.join("README.md")).expect("readme should be readable"),
        "dirty local version\n"
    );
    assert_eq!(current_branch(&slot_path), "HEAD");
}

// Running lease는 slot worktree가 더 이상 해당 agent branch에 있지 않아도 자동 reset으로
// 없애지 않는다. branch drift는 destructive reset보다 operator recovery로 남겨야 한다.
#[test]
fn parallel_entry_from_off_preserves_running_branch_drift_and_resets_idle_slots() {
    let repo = TempGitRepo::new("parallel-enable-reset-stale-running");
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
    run_git(&slot_path, &["checkout", "--detach", POOL_BASELINE_BRANCH]);
    fs::write(slot_path.join("scratch.tmp"), "discard me\n").expect("scratch file should write");

    let report = service
        .reset_pool_on_parallel_enable_report(&repo.workspace_dir())
        .expect("running branch drift should be reported as live blocker");
    let snapshot = service.build_supervisor_snapshot(
        &repo.workspace_dir(),
        true,
        Some(&ParallelModeReadinessSnapshot::new(
            repo.workspace_dir(),
            ParallelModeReadinessState::Ready,
            Vec::new(),
            None,
        )),
    );

    assert_eq!(report.live_blocker_count(), 1);
    assert_eq!(report.succeeded_reset_slot_count(), DEFAULT_POOL_SIZE - 1);
    assert_eq!(snapshot.roster.active_count(), 1);
    assert!(repo.slot_lease_path(1).exists());
    assert!(slot_path.join("scratch.tmp").exists());
    assert_eq!(current_branch(&slot_path), "HEAD");
}

// 실제 장애 재현 케이스: authority store에는 Running lease가 남아 있지만 slot worktree는
// 이미 clean detached prerelease로 돌아와 있고 agent branch도 없다. 이 split-brain을
// live slot으로 계속 보존하면 dispatcher가 capacity를 잃어 병렬 실행이 멈춘다.
#[test]
fn reconcile_preserves_clean_baseline_split_brain_running_lease() {
    let repo = TempGitRepo::new("reconcile-clean-baseline-split-brain");
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
    run_git(&slot_path, &["checkout", "--detach", POOL_BASELINE_BRANCH]);
    run_git(
        &repo.repo_root,
        &["branch", "-D", lease.branch_name.as_str()],
    );

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    let runtime_projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
            .expect("runtime projections should load");
    let detail = read_agent_session_detail_record(
        &test_parallel_runtime(),
        &repo.pool_root(),
        &lease.session_key(),
    )
    .expect("stale session detail should be recorded");

    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE - 1);
    assert_eq!(pool.blocked_slots, 1);
    assert!(repo.slot_lease_path(1).exists());
    assert_eq!(
        runtime_projection
            .slot_leases
            .get("slot-1")
            .map(|lease| lease.state),
        Some(ParallelModeSlotLeaseState::Running)
    );
    assert!(
        runtime_projection
            .task_dispatch_blocks
            .iter()
            .all(|block| block.task_id != lease.task_id)
    );
    assert_eq!(detail.state_label, "running");
    assert_eq!(detail.completion_state_label, "in_progress");
}

#[test]
fn reconcile_split_brain_rejects_forged_user_branch_lease() {
    let repo = TempGitRepo::new("split-brain-forged-user-branch");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(&lease.worktree_path);
    run_git(&slot_path, &["checkout", "--detach", POOL_BASELINE_BRANCH]);
    run_git(
        &repo.repo_root,
        &["branch", "user-preserved-branch", POOL_BASELINE_BRANCH],
    );
    run_git(
        &repo.repo_root,
        &["branch", "-D", lease.branch_name.as_str()],
    );
    let mut forged = lease.clone();
    forged.branch_name = "user-preserved-branch".to_string();
    forged.state = ParallelModeSlotLeaseState::CleanupPending;
    write_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &forged,
    )
    .expect("forged split-brain lease should persist");

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert!(repo.branch_exists("user-preserved-branch"));
    assert!(repo.slot_lease_path(1).exists());
    assert_eq!(pool.slots[0].state, ParallelModePoolSlotState::Blocked);
    assert_eq!(current_branch(&slot_path), "HEAD");
    assert!(slot_path.join("README.md").exists());
}

// 다른 PR이 integration target을 전진시킨 뒤 slot worktree가 이전 clean detached
// commit에 남아 있어도 ancestry만으로 현재 baseline이라고 간주하면 안 된다. stale
// Running lease와 old HEAD를 보존해 operator가 split-brain 원인을 확인할 수 있게 한다.
#[test]
fn reconcile_preserves_clean_integrated_but_stale_detached_split_brain_lease() {
    let repo = TempGitRepo::new("reconcile-integrated-detached-split-brain");
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
    run_git(&slot_path, &["checkout", "--detach", POOL_BASELINE_BRANCH]);
    let detached_head = run_command(
        "git",
        [
            "-C",
            slot_path.to_str().expect("slot path should be utf-8"),
            "rev-parse",
            "HEAD",
        ],
        None,
    )
    .expect("slot head should resolve");
    run_git(&repo.repo_root, &["checkout", POOL_BASELINE_BRANCH]);
    repo.commit_on_current_branch("later.txt", "later\n", "advance prerelease");
    run_git(
        &repo.repo_root,
        &["branch", "-D", lease.branch_name.as_str()],
    );

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    let runtime_projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
            .expect("runtime projections should load");
    let refreshed_head = run_command(
        "git",
        [
            "-C",
            slot_path.to_str().expect("slot path should be utf-8"),
            "rev-parse",
            "HEAD",
        ],
        None,
    )
    .expect("refreshed slot head should resolve");

    assert_ne!(detached_head, repo.head_sha());
    assert_eq!(refreshed_head, detached_head);
    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE - 1);
    assert_eq!(pool.blocked_slots, 1);
    assert!(repo.slot_lease_path(1).exists());
    assert!(runtime_projection.slot_leases.contains_key("slot-1"));
    assert!(
        !runtime_projection
            .task_dispatch_blocks
            .iter()
            .any(|block| block.task_id == lease.task_id)
    );
}

// 같은 split-brain이라도 baseline worktree에 변경이 남아 있으면 자동 회수하면 안 된다.
// 이 경우는 사용자나 아직 늦게 쓰는 worker가 남긴 산출물일 수 있으므로 blocked로 보존한다.
#[test]
fn reconcile_preserves_dirty_baseline_split_brain_running_lease() {
    let repo = TempGitRepo::new("reconcile-dirty-baseline-split-brain");
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
    run_git(&slot_path, &["checkout", "--detach", POOL_BASELINE_BRANCH]);
    run_git(
        &repo.repo_root,
        &["branch", "-D", lease.branch_name.as_str()],
    );
    fs::write(slot_path.join("README.md"), "dirty\n").expect("dirty slot file should write");

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    let runtime_projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
            .expect("runtime projections should load");

    assert_eq!(pool.blocked_slots, 1);
    assert!(repo.slot_lease_path(1).exists());
    assert!(runtime_projection.slot_leases.contains_key("slot-1"));
    assert!(
        runtime_projection
            .task_dispatch_blocks
            .iter()
            .all(|block| block.task_id != lease.task_id)
    );
    assert_eq!(
        fs::read_to_string(slot_path.join("README.md")).expect("README should be readable"),
        "dirty\n"
    );
}

#[test]
fn parallel_entry_from_off_resets_stale_startup_slots_even_when_another_slot_is_running() {
    let repo = TempGitRepo::new("parallel-enable-reset-stale-with-running");
    let service = test_parallel_mode_service();
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let running_lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request(
                "task-running",
                "Running Task",
                "agent-running",
                "running-task",
            ),
        )
        .expect("running slot lease should be acquired");
    let running_slot_path = PathBuf::from(running_lease.worktree_path.clone());
    service
        .mark_workspace_slot_running(&running_lease.worktree_path)
        .expect("running slot should transition to running");
    fs::write(running_slot_path.join("keep-running.tmp"), "keep me\n")
        .expect("running scratch file should write");

    let mut stale_lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-stale", "Stale Task", "agent-stale", "stale-task"),
        )
        .expect("stale slot lease should be acquired");
    let stale_slot_path = PathBuf::from(stale_lease.worktree_path.clone());
    stale_lease.leased_at = "2020-01-01T00:00:00Z".to_string();
    write_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &stale_lease,
    )
    .expect("stale lease should be persisted");
    record_assigned_session_detail(
        &adapter,
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &stale_lease,
    )
    .expect("stale assigned detail should be recorded");
    fs::write(stale_slot_path.join("scratch.tmp"), "discard me\n")
        .expect("stale scratch file should write");

    let report = service
        .reset_pool_on_parallel_enable_report(&repo.workspace_dir())
        .expect("parallel enable reset should preserve live slots and reset stale slots");
    let snapshot = service.build_supervisor_snapshot(
        &repo.workspace_dir(),
        true,
        Some(&ParallelModeReadinessSnapshot::new(
            repo.workspace_dir(),
            ParallelModeReadinessState::Ready,
            Vec::new(),
            None,
        )),
    );

    assert_eq!(report.live_blocker_count(), 2);
    assert!(
        !report
            .succeeded_reset_slot_ids()
            .contains(&stale_lease.slot_id)
    );
    assert_eq!(snapshot.roster.active_count(), 2);
    assert!(repo.slot_lease_path(2).exists());
    assert!(
        repo.session_detail_path(&stale_lease.session_key())
            .exists()
    );
    assert!(stale_slot_path.join("scratch.tmp").exists());
    assert!(repo.slot_lease_path(1).exists());
    assert!(running_slot_path.join("keep-running.tmp").exists());
}

// Leased startup residue is also disposable on off -> on entry. The reset keeps planning task
// authority but clears runtime/session mirrors before the next dispatch pass.
#[test]
fn parallel_entry_from_off_resets_stale_startup_leases_and_slot_worktrees() {
    let repo = TempGitRepo::new("parallel-enable-reset-stale-startup");
    let service = test_parallel_mode_service();
    let mut lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(lease.worktree_path.clone());
    lease.leased_at = "2020-01-01T00:00:00Z".to_string();
    write_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &lease,
    )
    .expect("stale lease should be persisted");
    record_assigned_session_detail(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &lease,
    )
    .expect("assigned detail should be recorded");
    fs::write(slot_path.join("scratch.tmp"), "discard me\n").expect("scratch file should write");

    let reset_count = service
        .reset_pool_on_parallel_enable(&repo.workspace_dir())
        .expect("parallel re-entry from off should reset stale startup lease");
    let snapshot = service.build_supervisor_snapshot(
        &repo.workspace_dir(),
        true,
        Some(&ParallelModeReadinessSnapshot::new(
            repo.workspace_dir(),
            ParallelModeReadinessState::Ready,
            Vec::new(),
            None,
        )),
    );

    assert_eq!(reset_count, DEFAULT_POOL_SIZE - 1);
    assert_eq!(snapshot.roster.active_count(), 1);
    assert!(repo.slot_lease_path(1).exists());
    assert!(repo.session_detail_path(&lease.session_key()).exists());
    assert!(slot_path.join("scratch.tmp").exists());
}

// `:parallel` 진입 reset의 범위는 disposable pool runtime으로 한정된다. 기존
// planning task authority와 queue projection은 사용자가 만든 작업 원장이므로 보존해야 한다.
#[test]
fn parallel_entry_reset_preserves_existing_planning_tasks() {
    let repo = TempGitRepo::new("parallel-reset-preserves-tasks");
    let service = test_parallel_mode_service();
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let queue_task = queue_task(1, "task-1");
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: vec![TaskDefinition {
            id: "task-1".to_string(),
            direction_id: "direction-1".to_string(),
            direction_relation_note: "covers the reset scope contract".to_string(),
            title: "Keep existing task".to_string(),
            description: "This task must survive parallel pool reset.".to_string(),
            status: TaskStatus::Ready,
            base_priority: 99,
            dynamic_priority_delta: 0,
            priority_reason: String::new(),
            depends_on: Vec::new(),
            blocked_by: Vec::new(),
            created_by: TaskActor::User,
            last_updated_by: TaskActor::User,
            source_turn_id: None,
            provenance: Default::default(),
            updated_at: queue_task.updated_at.clone(),
        }],
    };
    let queue_projection = PriorityQueueProjection {
        next_task: Some(queue_task.clone()),
        active_tasks: vec![queue_task],
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };

    adapter
        .commit_task_authority_snapshot(
            &repo.workspace_dir(),
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &task_authority,
                queue_projection: &queue_projection,
            },
        )
        .expect("planning task authority should commit");

    service
        .reset_pool_on_parallel_enable(&repo.workspace_dir())
        .expect("parallel pool reset should succeed");

    let snapshot = adapter
        .load_task_authority_snapshot(&repo.workspace_dir())
        .expect("planning task authority should load after reset")
        .expect("planning task authority should remain present");
    assert_eq!(snapshot.task_authority, task_authority);
    assert_eq!(snapshot.queue_projection, queue_projection);
}

// reconcile은 비어 있는 pool root를 실제 capacity로 바꾸는 provisioning 단계다.
// 모든 slot worktree가 생성되고 missing count가 사라져야 dispatcher가 곧바로
// idle slot을 사용할 수 있다.
#[test]
fn reconcile_provisions_missing_slots_into_idle_baselines() {
    let repo = TempGitRepo::new("provision-slots");
    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(
        pool.idle_slots, DEFAULT_POOL_SIZE,
        "provisioned pool should be immediately reusable: {pool:#?}"
    );
    assert_eq!(pool.missing_slots, 0);
    assert!(pool.reconcile_status.contains("provisioned 3"));
    for slot_number in 1..=DEFAULT_POOL_SIZE {
        assert!(repo.pool_root().join(slot_id(slot_number)).exists());
    }
}

// Existing linked worktree 안에서 실행해도 worktree/add의 상대 destination 기준은
// canonical main checkout이어야 한다. 특히 Git for Windows는 verbatim absolute path를
// 피하기 위해 상대 경로를 쓰므로 command cwd와 계산 기준이 어긋나면 다른 위치가 생긴다.
#[test]
fn reconcile_provisions_missing_slots_from_an_existing_slot_workspace() {
    let repo = TempGitRepo::new("provision-slots-from-slot");
    let initial_pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    assert_eq!(initial_pool.idle_slots, DEFAULT_POOL_SIZE);
    let existing_slot = repo.pool_root().join(slot_id(1));
    for slot_number in 2..=DEFAULT_POOL_SIZE {
        fs::remove_dir_all(repo.pool_root().join(slot_id(slot_number)))
            .expect("test should remove the missing slot worktree");
    }
    run_git(&repo.repo_root, &["worktree", "prune", "--expire", "now"]);

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        existing_slot
            .to_str()
            .expect("existing slot path should be valid utf-8"),
    );

    assert_eq!(
        pool.idle_slots, DEFAULT_POOL_SIZE,
        "slot workspace reconcile should provision the canonical pool: {pool:#?}"
    );
    assert_eq!(pool.missing_slots, 0);
    assert!(pool.reconcile_status.contains("provisioned 2"));
    for slot_number in 1..=DEFAULT_POOL_SIZE {
        assert!(repo.pool_root().join(slot_id(slot_number)).exists());
    }
}

// git worktree inventory에 없는 slot path는 lease가 없어도 Akra 소유라고 증명할 수 없다.
// reconcile은 남은 파일을 보존하고 해당 slot을 operator recovery 대상으로 막아야 한다.
#[test]
fn reconcile_preserves_filesystem_residue_outside_worktree_inventory() {
    let repo = TempGitRepo::new("provision-over-residue");
    let residue_path = repo.pool_root().join(slot_id(1));
    fs::create_dir_all(&residue_path).expect("residue directory should be created");
    fs::write(residue_path.join("scratch.tmp"), "transient\n")
        .expect("residue file should be written");

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE - 1);
    assert_eq!(pool.blocked_slots, 1);
    assert_eq!(
        fs::read_to_string(residue_path.join("scratch.tmp"))
            .expect("unowned residue must remain readable"),
        "transient\n"
    );
    assert_eq!(pool.slots[0].branch_name, "unknown");
    assert!(
        pool.slots[0]
            .worktree_label
            .contains("directory exists outside git worktree inventory")
    );
}

// worker launch가 중간에 사라져 Leased 상태만 오래 남으면 roster가 계속 active로
// 보인다. reconcile은 오래된 launch-pending lease와 clean worktree를 startup
// failure로 회수해 slot을 다시 idle pool로 돌려야 한다.
#[test]
fn reconcile_releases_stale_leased_startup_slot() {
    let repo = TempGitRepo::new("stale-leased-startup");
    let service = test_parallel_mode_service();
    let mut lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    lease.leased_at = "2020-01-01T00:00:00Z".to_string();
    write_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &lease,
    )
    .expect("stale lease should be persisted");
    record_assigned_session_detail(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &lease,
    )
    .expect("assigned detail should be recorded");

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
    assert_eq!(pool.leased_slots, 0);
    assert_eq!(pool.running_slots, 0);
    assert!(!repo.slot_lease_path(1).exists());
    assert!(!repo.branch_exists(&lease.branch_name));
    assert_eq!(current_branch(&PathBuf::from(&lease.worktree_path)), "HEAD");
    assert_eq!(
        read_agent_session_detail_record(
            &test_parallel_runtime(),
            &repo.pool_root(),
            &lease.session_key()
        )
        .expect("failed startup detail should be recorded")
        .state_label,
        "failed"
    );
}

#[test]
fn reconcile_preserves_stale_startup_slot_when_dispatch_block_write_fails() {
    let repo = TempGitRepo::new("stale-leased-startup-block-failure");
    let service = test_parallel_mode_service();
    let mut lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
        )
        .expect("slot lease should be acquired");
    lease.leased_at = "2020-01-01T00:00:00Z".to_string();
    write_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &lease,
    )
    .expect("stale lease should be persisted");
    install_runtime_insert_failure(
        &repo,
        "fail_reconciled_startup_dispatch_block",
        "runtime_task_dispatch_blocks",
    );

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(pool.leased_slots, 1);
    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE - 1);
    assert!(repo.slot_lease_path(1).exists());
    assert!(repo.branch_exists(&lease.branch_name));
    assert_eq!(
        current_branch(&PathBuf::from(&lease.worktree_path)),
        lease.branch_name
    );
    let runtime_projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
            .expect("runtime projection should retain the active lease");
    assert!(runtime_projection.slot_leases.contains_key("slot-1"));
    assert!(runtime_projection.task_dispatch_blocks.is_empty());
}

// pool worktree는 repository 내부가 아니라 sibling `repo-akra-worktrees` 아래에 둔다.
// 이렇게 해야 원본 checkout의 status와 nested worktree 탐색이 agent slot 파일들로
// 오염되지 않는다.
#[test]
fn pool_root_lives_in_repo_sibling_akra_worktrees_root() {
    let repo = TempGitRepo::new("pool-root");
    let pool_root = repo.pool_root();
    let normalized = pool_root.to_string_lossy().replace('\\', "/");

    assert!(
        normalized.contains("/repo-akra-worktrees/"),
        "pool root should live under a repo sibling prerelease worktrees root: {normalized}"
    );
    assert!(
        normalized.ends_with("/akra-pool"),
        "pool root should end at the akra pool directory: {normalized}"
    );
}

// local integration branch가 없어도 remote-tracking baseline만으로 slot을 provision한다.
#[test]
fn reconcile_creates_local_prerelease_branch_before_provisioning_slots() {
    let repo = TempGitRepo::new("create-akra");
    repo.delete_local_prerelease_branch();
    assert!(!repo.branch_exists(POOL_BASELINE_BRANCH));
    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert!(!repo.branch_exists(POOL_BASELINE_BRANCH));
    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
    assert_eq!(pool.blocked_slots, 0);
}

// local integration branch drift는 보존하되 remote-tracking baseline pool은 계속 사용할 수 있다.
#[test]
fn reconcile_blocks_drifted_local_prerelease_without_discarding_commits() {
    let repo = TempGitRepo::new("reset-akra");
    let origin_prerelease_head = run_command(
        "git",
        [
            "-C",
            repo.repo_root.to_str().expect("repo root should be utf-8"),
            "rev-parse",
            &remote_standard_tracking_ref(),
        ],
        None,
    )
    .expect("origin prerelease should resolve");
    repo.commit_on_current_branch("feature.txt", "new baseline\n", "advance user branch");
    let current_head = repo.head_sha();
    assert_ne!(origin_prerelease_head, current_head);
    run_git(
        &repo.repo_root,
        &["branch", "-f", POOL_BASELINE_BRANCH, "HEAD"],
    );
    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_ne!(
        run_command(
            "git",
            [
                "-C",
                repo.repo_root.to_str().expect("repo root should be utf-8"),
                "rev-parse",
                POOL_BASELINE_BRANCH,
            ],
            None,
        )
        .expect("prerelease should resolve"),
        origin_prerelease_head
    );
    assert_eq!(pool.blocked_slots, 0);
    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE);
}

// local/remote 표준 branch가 모두 없으면 reconcile은 현재 HEAD를 원격 target으로 승격하지 않는다.
// operator가 integration branch를 명시적으로 만들기 전까지 pool 생성은 fail closed로 남아야 한다.
#[test]
fn reconcile_blocks_missing_remote_integration_branch_without_pushing_head() {
    let repo = TempGitRepo::new("seed-standard-branch");
    let origin_root = repo.create_bare_origin_remote();
    repo.delete_local_prerelease_branch();
    repo.delete_remote_standard_tracking_branch();
    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    assert!(
        run_command(
            "git",
            [
                "--git-dir",
                origin_root.to_str().expect("origin root should be utf-8"),
                "rev-parse",
                &local_standard_ref(),
            ],
            None,
        )
        .is_none(),
        "reconcile must not create the remote integration branch"
    );
    assert!(!repo.branch_exists(POOL_BASELINE_BRANCH));
    assert_eq!(pool.blocked_slots, DEFAULT_POOL_SIZE);
    assert!(
        pool.reconcile_status
            .contains("test reconcile blocked / verified fixture target is unavailable")
    );
}

// baseline ref가 이동하면 기존 clean detached slot들도 예전 commit에 떨어져 있을
// 수 있다. reconcile은 dirty하지 않은 slot을 새 baseline으로 reset해, board에
// "detached away" 경고가 남지 않도록 정렬한다.
#[test]
fn reconcile_resets_clean_detached_slots_after_empty_prerelease_baseline_moves() {
    let repo = TempGitRepo::new("reset-detached-slots");
    let initial_pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    assert_eq!(initial_pool.idle_slots, DEFAULT_POOL_SIZE);

    run_git(&repo.repo_root, &["checkout", POOL_BASELINE_BRANCH]);
    repo.commit_on_current_branch("feature.txt", "new baseline\n", "advance user branch");
    let refreshed_pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(refreshed_pool.idle_slots, DEFAULT_POOL_SIZE);
    assert_eq!(refreshed_pool.blocked_slots, 0);
    assert!(refreshed_pool.slots.iter().all(|slot| {
        !slot.worktree_label.contains(&format!(
            "detached away from `{POOL_BASELINE_BRANCH}` baseline"
        ))
    }));
}

// Linked worktree git metadata can retain an index.lock after an interrupted or
// still-running Git operation. Age alone cannot prove that the lock is abandoned,
// so reconcile must preserve it and leave the slot blocked for operator recovery.
#[test]
fn reconcile_preserves_detached_slot_index_lock() {
    let repo = TempGitRepo::new("reset-stale-index-lock");
    let initial_pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    assert_eq!(initial_pool.idle_slots, DEFAULT_POOL_SIZE);
    let slot_path = repo.pool_root().join(slot_id(1));
    let initial_slot_head = run_command(
        "git",
        [
            "-C",
            slot_path.to_str().expect("slot path should be utf-8"),
            "rev-parse",
            "HEAD",
        ],
        None,
    )
    .expect("initial slot head should resolve");
    run_git(&repo.repo_root, &["checkout", POOL_BASELINE_BRANCH]);
    repo.commit_on_current_branch("feature.txt", "new baseline\n", "advance baseline");
    let git_dir = run_command(
        "git",
        [
            "-C",
            slot_path.to_str().expect("slot path should be utf-8"),
            "rev-parse",
            "--git-dir",
        ],
        None,
    )
    .expect("slot git dir should resolve");
    let index_lock_path = Path::new(git_dir.trim()).join("index.lock");
    fs::write(&index_lock_path, "").expect("stale index.lock should be written");

    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );

    assert_eq!(pool.idle_slots, DEFAULT_POOL_SIZE - 1);
    assert_eq!(pool.blocked_slots, 1);
    assert!(index_lock_path.exists());
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                slot_path.to_str().expect("slot path should be utf-8"),
                "rev-parse",
                "HEAD",
            ],
            None,
        )
        .expect("slot head should resolve"),
        initial_slot_head
    );
}

// agent slot worktree에서 reconcile을 호출해도 canonical 표준 branch는 agent
// branch HEAD로 갱신되면 안 된다. root detection이 slot workspace를 원본 repo로
// 되돌려 계산하는지 확인하는 회귀 테스트다.
#[test]
fn reconcile_does_not_refresh_prerelease_from_agent_slot_workspace() {
    let repo = TempGitRepo::new("agent-slot-does-not-reset-akra");
    let slot_path = repo.create_agent_slot(1, "task-one");
    let original_prerelease_head = run_command(
        "git",
        [
            "-C",
            repo.repo_root.to_str().expect("repo root should be utf-8"),
            "rev-parse",
            POOL_BASELINE_BRANCH,
        ],
        None,
    )
    .expect("prerelease should resolve");
    repo.commit_file_in_slot(&slot_path, "feature.txt", "done\n", "agent work");
    assert_ne!(
        original_prerelease_head,
        run_command(
            "git",
            [
                "-C",
                slot_path.to_str().expect("slot path should be utf-8"),
                "rev-parse",
                "HEAD",
            ],
            None,
        )
        .expect("slot head should resolve")
    );
    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        slot_path.to_str().expect("slot path should be utf-8"),
    );

    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.repo_root.to_str().expect("repo root should be utf-8"),
                "rev-parse",
                POOL_BASELINE_BRANCH,
            ],
            None,
        )
        .expect("prerelease should resolve"),
        original_prerelease_head
    );
    assert!(pool.blocked_slots > 0);
}

// merged agent slot이라도 untracked data가 있으면 cleanup pending 상태로 보존한다.
#[test]
fn reconcile_preserves_merged_agent_slot_with_untracked_data() {
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
    fs::write(slot_path.join("scratch.untracked"), "transient\n")
        .expect("untracked file should be written");
    let pool = reconcile_pool_board(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
    );
    let slot = &pool.slots[0];

    assert_eq!(slot.state, ParallelModePoolSlotState::AwaitingCleanup);
    assert!(slot_path.join("scratch.untracked").exists());
    assert!(repo.branch_exists(&branch_name));
    assert!(repo.slot_lease_path(1).exists());
}
