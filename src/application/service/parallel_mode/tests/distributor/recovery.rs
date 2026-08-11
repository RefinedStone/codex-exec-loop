use super::super::*;
use crate::application::service::parallel_mode::{
    ParallelModeAutomationGuard, ParallelModeOrchestratorTrigger,
};

const TEST_COORDINATION_TIMEOUT: Duration = Duration::from_secs(10);

// supervisor snapshot은 관찰 전용이어야 한다. queue head가 merge-pending 상태여도
// snapshot 렌더링 과정에서 GitHub inspect, push, recovery 같은 runtime 작업이
// 실행되면 TUI 조회만으로 상태가 바뀌므로, fake GitHub 호출이 비어 있음을 확인한다.
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
    repo.commit_file_in_slot(
        Path::new(&lease.worktree_path),
        "snapshot.txt",
        "snapshot result\n",
        "record snapshot result",
    );
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
    let mut queue_record =
        load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
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
fn readiness_does_not_write_a_missing_commit_ready_handoff() {
    let repo = TempGitRepo::new("readiness-missing-handoff-observation");
    let service =
        test_parallel_mode_service_with_github(Arc::new(FakeGithubAutomationPort::ready()));
    let lease = prepare_missing_commit_ready_result(
        &service,
        &repo,
        "task-readiness",
        "agent-readiness",
        "readiness",
    );
    assert!(
        load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root()).is_empty(),
        "the fixture must begin in the interrupted handoff state"
    );
    let readiness = service.inspect_readiness(
        &repo.workspace_dir(),
        &PlanningRuntimeProjection::ready("prompt".into(), "queue".into(), None)
            .with_workspace_present(true),
    );
    assert!(readiness.allows_parallel_mode());
    assert!(
        load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root()).is_empty(),
        "readiness inspection must not enqueue a missing commit-ready handoff"
    );
    let commit_ready_detail =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
            .expect("readiness authority projection should load")
            .session_details
            .into_iter()
            .find(|detail| detail.session_key == lease.session_key())
            .expect("commit-ready authority detail should remain present");
    assert_eq!(commit_ready_detail.state_label, "commit_ready");
    assert!(
        service
            .pending_commit_ready_recovery_signature(&repo.workspace_dir())
            .expect("pending recovery signature should be readable")
            .is_some(),
        "a commit-ready authority row without a queue record must remain wakeable"
    );
}

#[test]
fn manual_unguarded_tick_recovers_and_processes_a_missing_commit_ready_handoff() {
    let repo = TempGitRepo::new("manual-missing-handoff-recovery");
    let service =
        test_parallel_mode_service_with_github(Arc::new(FakeGithubAutomationPort::ready()));
    let lease = prepare_missing_commit_ready_result(
        &service,
        &repo,
        "task-manual",
        "agent-manual",
        "manual",
    );

    let tick = service
        .run_orchestrator_tick(
            &repo.workspace_dir(),
            ParallelModeOrchestratorTrigger::ManualDispatch,
        )
        .expect("manual unguarded processing should recover and deliver the missing handoff");
    assert!(!tick.blocked);
    assert!(
        tick.notices
            .iter()
            .any(|notice| notice.contains("distributor integrated queue head into prerelease")),
        "manual missing-handoff recovery should integrate in the same tick: {:?}",
        tick.notices
    );
    let records = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].session_key, lease.session_key());
    assert_eq!(records[0].queue_state, ParallelModeQueueItemState::Done);
    assert_eq!(
        service
            .pending_commit_ready_recovery_signature(&repo.workspace_dir())
            .expect("manual recovery signature should load"),
        None
    );
}

#[test]
fn closed_guarded_tick_does_not_recover_a_missing_commit_ready_handoff() {
    let repo = TempGitRepo::new("closed-guarded-missing-handoff");
    let service =
        test_parallel_mode_service_with_github(Arc::new(FakeGithubAutomationPort::ready()));
    let lease = prepare_missing_commit_ready_result(
        &service,
        &repo,
        "task-closed",
        "agent-closed",
        "closed",
    );
    let guard = ParallelModeAutomationGuard::default();
    guard.activate(repo.workspace_dir(), 1);
    let closed_permit = guard.permit(repo.workspace_dir(), 1);
    guard.cancel(&repo.workspace_dir());
    let closed_tick = service
        .run_orchestrator_tick_guarded(
            &repo.workspace_dir(),
            ParallelModeOrchestratorTrigger::ManualDispatch,
            &closed_permit,
        )
        .expect("closed epoch tick should fail closed");
    assert!(closed_tick.blocked);
    assert!(load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root()).is_empty());
    let projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
            .expect("closed guarded authority projection should load");
    let detail = projection
        .session_details
        .iter()
        .find(|detail| detail.session_key == lease.session_key())
        .expect("closed guarded commit-ready detail should remain present");
    assert_eq!(detail.state_label, "commit_ready");
    assert!(
        service
            .pending_commit_ready_recovery_signature(&repo.workspace_dir())
            .expect("closed guarded recovery signature should load")
            .is_some()
    );
}

#[test]
fn stale_guarded_enqueue_preflight_does_not_block_or_mutate_replacement_generation() {
    let repo = TempGitRepo::new("stale-enqueue-preflight-replacement");
    let service =
        test_parallel_mode_service_with_github(Arc::new(FakeGithubAutomationPort::ready()));
    let lease = prepare_missing_commit_ready_result(
        &service,
        &repo,
        "task-old-enqueue",
        "agent-old-enqueue",
        "old-enqueue",
    );

    let continuation_gate = PostTurnContinuationGate::default();
    let automation_guard = ParallelModeAutomationGuard::default();
    automation_guard.activate(repo.workspace_dir(), 17);
    let old_permit = automation_guard
        .permit(repo.workspace_dir(), 17)
        .with_continuation_permit(continuation_gate.capture());
    let (preflight_entered_sender, preflight_entered_receiver) = std::sync::mpsc::channel();
    let (resume_old_sender, resume_old_receiver) = std::sync::mpsc::channel();
    let (old_done_sender, old_done_receiver) = std::sync::mpsc::channel();
    let old_service = service.clone();
    let old_lease = lease.clone();
    let old_thread = thread::spawn(move || {
        install_after_distributor_enqueue_preflight_hook(move || {
            preflight_entered_sender
                .send(())
                .expect("old preflight entry should be observed");
            resume_old_receiver
                .recv_timeout(TEST_COORDINATION_TIMEOUT)
                .expect("old enqueue preflight should be released");
        });
        let result = old_service
            .enqueue_workspace_commit_ready_result_for_lease_guarded(&old_lease, &old_permit);
        old_done_sender
            .send(result)
            .expect("old enqueue result should be observed");
    });
    preflight_entered_receiver
        .recv_timeout(TEST_COORDINATION_TIMEOUT)
        .expect("old enqueue should pause after its unlocked preflight");

    continuation_gate.advance();
    let replacement_continuation = continuation_gate.capture();
    let mut replacement = lease.clone();
    replacement.task_id = "task-replacement".to_string();
    replacement.task_title = "Replacement".to_string();
    replacement.agent_id = "agent-replacement".to_string();
    replacement.leased_at = "2099-01-01T00:00:00Z".to_string();
    replacement.running_started_at = Some("2099-01-01T00:00:01Z".to_string());
    replacement.lease_generation =
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string());
    replacement.state = ParallelModeSlotLeaseState::Running;
    let expected_replacement = replacement.clone();
    let transition_workspace = repo.workspace_dir();
    let transition_pool_root = repo.pool_root();
    let (transition_done_sender, transition_done_receiver) = std::sync::mpsc::channel();
    let transition_thread = thread::spawn(move || {
        let result = replacement_continuation
            .with_current(|| -> Result<(), String> {
                let authority = SqlitePlanningAuthorityAdapter::new();
                let runtime = test_parallel_runtime();
                let pool_lock =
                    acquire_pool_mutation_lock(&authority, &runtime, &transition_workspace)?;
                pool_lock.verify_pool_root(&transition_pool_root)?;
                write_slot_lease(
                    &authority,
                    &runtime,
                    &transition_workspace,
                    &transition_pool_root,
                    &replacement,
                )?;
                record_running_session_detail(
                    &authority,
                    &runtime,
                    &transition_workspace,
                    &transition_pool_root,
                    &replacement,
                )?;
                Ok(())
            })
            .ok_or_else(|| "replacement continuation unexpectedly became stale".to_string())
            .and_then(|result| result);
        transition_done_sender
            .send(result)
            .expect("replacement transition result should be observed");
    });
    transition_done_receiver
        .recv_timeout(TEST_COORDINATION_TIMEOUT)
        .expect("replacement continuation must not wait on the old enqueue preflight")
        .expect("replacement pool transition should succeed");

    resume_old_sender
        .send(())
        .expect("old enqueue preflight should resume");
    let old_result = old_done_receiver
        .recv_timeout(TEST_COORDINATION_TIMEOUT)
        .expect("stale old enqueue should terminate promptly")
        .expect("stale old enqueue should fail closed without an error");
    assert!(old_result.is_none());
    old_thread.join().expect("old enqueue thread should join");
    transition_thread
        .join()
        .expect("replacement transition thread should join");

    let projection =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
            .expect("replacement authority projection should load");
    assert_eq!(
        projection.slot_leases.get(&expected_replacement.slot_id),
        Some(&expected_replacement)
    );
    let replacement_detail = projection
        .session_details
        .iter()
        .find(|detail| detail.session_key == expected_replacement.session_key())
        .expect("replacement running detail should remain present");
    assert_eq!(replacement_detail.state_label, "running");
    assert_eq!(replacement_detail.completion_state_label, "in_progress");
    assert!(projection.distributor_queue_records.is_empty());
    assert!(load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root()).is_empty());
}

#[test]
fn guarded_enqueue_pool_busy_stops_within_one_retry_interval_after_gate_advance() {
    let repo = TempGitRepo::new("enqueue-pool-busy-retry");
    let service =
        test_parallel_mode_service_with_github(Arc::new(FakeGithubAutomationPort::ready()));
    let lease = prepare_missing_commit_ready_result(
        &service,
        &repo,
        "task-pool-busy",
        "agent-pool-busy",
        "pool-busy",
    );

    let continuation_gate = PostTurnContinuationGate::default();
    let automation_guard = ParallelModeAutomationGuard::default();
    automation_guard.activate(repo.workspace_dir(), 23);
    let enqueue_permit = automation_guard
        .permit(repo.workspace_dir(), 23)
        .with_continuation_permit(continuation_gate.capture());
    let (preflight_entered_sender, preflight_entered_receiver) = std::sync::mpsc::channel();
    let (resume_preflight_sender, resume_preflight_receiver) = std::sync::mpsc::channel();
    let (pool_busy_sender, pool_busy_receiver) = std::sync::mpsc::channel();
    let (release_busy_sender, release_busy_receiver) = std::sync::mpsc::channel();
    let (enqueue_done_sender, enqueue_done_receiver) = std::sync::mpsc::channel();
    let enqueue_service = service.clone();
    let enqueue_lease = lease.clone();
    let enqueue_thread = thread::spawn(move || {
        install_after_distributor_enqueue_preflight_hook(move || {
            preflight_entered_sender
                .send(())
                .expect("enqueue preflight entry should be observed");
            resume_preflight_receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("enqueue preflight should resume");
        });
        install_after_distributor_enqueue_pool_busy_hook(move || {
            pool_busy_sender
                .send(())
                .expect("pool-busy attempt should be observed");
            release_busy_receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("pool-busy retry should be released");
        });
        let result = enqueue_service.enqueue_workspace_commit_ready_result_for_lease_guarded(
            &enqueue_lease,
            &enqueue_permit,
        );
        enqueue_done_sender
            .send(result)
            .expect("enqueue result should be observed");
    });
    preflight_entered_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("enqueue should pause after preflight");

    let authority = SqlitePlanningAuthorityAdapter::new();
    let runtime = test_parallel_runtime();
    let pool_lock = acquire_pool_mutation_lock(&authority, &runtime, &repo.workspace_dir())
        .expect("test should hold the pool lock across the first final attempt");
    pool_lock
        .verify_pool_root(&repo.pool_root())
        .expect("test pool lock should match the fixture");
    resume_preflight_sender
        .send(())
        .expect("enqueue final phase should start");
    pool_busy_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("first nonblocking final attempt should observe pool contention");

    let barrier_permit = continuation_gate.capture();
    let (barrier_done_sender, barrier_done_receiver) = std::sync::mpsc::channel();
    let barrier_thread = thread::spawn(move || {
        let entered = barrier_permit.with_current(|| ());
        barrier_done_sender
            .send(entered)
            .expect("continuation barrier result should be observed");
    });
    assert_eq!(
        barrier_done_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("pool-busy retry must release the continuation mutex"),
        Some(())
    );
    barrier_thread
        .join()
        .expect("continuation barrier thread should join");

    continuation_gate.advance();
    drop(pool_lock);
    release_busy_sender
        .send(())
        .expect("stale pool-busy retry should leave its hook");
    let stale_result = enqueue_done_receiver
        .recv_timeout(Duration::from_millis(25))
        .expect("stale pool-busy retry should stop before one retry sleep elapses")
        .expect("stale pool-busy retry should fail closed without an error");
    assert!(stale_result.is_none());
    enqueue_thread
        .join()
        .expect("stale enqueue retry thread should join");
    assert!(load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root()).is_empty());
    let detail = SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
        .expect("stale retry authority projection should load")
        .session_details
        .into_iter()
        .find(|detail| detail.session_key == lease.session_key())
        .expect("stale retry commit-ready detail should remain present");
    assert_eq!(detail.state_label, "commit_ready");
}

#[test]
fn dirty_missing_commit_ready_candidate_does_not_starve_an_existing_queue_head() {
    let repo = TempGitRepo::new("commit-ready-recovery-does-not-starve-head");
    let service =
        test_parallel_mode_service_with_github(Arc::new(GitBackedGithubAutomationPort::ready()));
    let queued_lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-a", "Queued A", "agent-a", "queued-a"),
        )
        .expect("queued slot should lease");
    service
        .mark_workspace_slot_running(&queued_lease.worktree_path)
        .expect("queued slot should become running");
    repo.commit_file_in_slot(
        Path::new(&queued_lease.worktree_path),
        "queued-a.txt",
        "queued A result\n",
        "record queued A result",
    );
    service
        .begin_workspace_official_completion(
            &queued_lease.worktree_path,
            "turn-a",
            None,
            Some("Queued A complete."),
            Some("cargo test passed"),
            None,
        )
        .expect("queued A completion should capture");
    service
        .mark_workspace_official_completion_refreshing(&queued_lease.worktree_path)
        .expect("queued A should enter refreshing");
    service
        .mark_workspace_commit_ready(&queued_lease.worktree_path, "queued A accepted")
        .expect("queued A should become commit-ready");
    service
        .enqueue_workspace_commit_ready_result(&queued_lease.worktree_path)
        .expect("queued A should enqueue")
        .expect("queued A queue record should exist");

    let missing_lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-b", "Recover B", "agent-b", "recover-b"),
        )
        .expect("recovery slot should lease");
    service
        .mark_workspace_slot_running(&missing_lease.worktree_path)
        .expect("recovery slot should become running");
    repo.commit_file_in_slot(
        Path::new(&missing_lease.worktree_path),
        "recover-b.txt",
        "recover B result\n",
        "record recoverable B result",
    );
    service
        .begin_workspace_official_completion(
            &missing_lease.worktree_path,
            "turn-b",
            None,
            Some("Recover B complete."),
            Some("cargo test passed"),
            None,
        )
        .expect("recovery B completion should capture");
    service
        .mark_workspace_official_completion_refreshing(&missing_lease.worktree_path)
        .expect("recovery B should enter refreshing");
    service
        .mark_workspace_commit_ready(&missing_lease.worktree_path, "recovery B accepted")
        .expect("recovery B should become commit-ready");
    let dirty_path = Path::new(&missing_lease.worktree_path).join("operator-dirty.txt");
    fs::write(&dirty_path, "operator inspection in progress\n")
        .expect("dirty recovery fixture should write");

    let guard = ParallelModeAutomationGuard::default();
    guard.activate(repo.workspace_dir(), 7);
    let permit = guard.permit(repo.workspace_dir(), 7);
    let queued_tick = service
        .run_orchestrator_tick_guarded(
            &repo.workspace_dir(),
            ParallelModeOrchestratorTrigger::ManualDispatch,
            &permit,
        )
        .expect("existing queue head should run before missing-result recovery");
    assert!(queued_tick.notices.iter().any(|notice| {
        notice.contains("distributor integrated queue head into prerelease")
            && notice.contains("agent-a")
    }));
    let records = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].task_id, "task-a");
    assert_eq!(records[0].queue_state, ParallelModeQueueItemState::Done);

    let dirty_signature = service
        .pending_commit_ready_recovery_signature(&repo.workspace_dir())
        .expect("dirty recovery signature should load")
        .expect("dirty recovery candidate should remain wakeable");
    assert!(dirty_signature.contains("untracked files"));
    let dirty_error = service
        .run_orchestrator_tick_guarded(
            &repo.workspace_dir(),
            ParallelModeOrchestratorTrigger::ManualDispatch,
            &permit,
        )
        .expect_err("dirty missing result should fail its own recovery tick");
    assert!(dirty_error.contains("nonignored worktree changes"));
    assert_eq!(
        service
            .pending_commit_ready_recovery_signature(&repo.workspace_dir())
            .expect("unchanged recovery signature should load")
            .as_deref(),
        Some(dirty_signature.as_str()),
        "unchanged dirty state must retain one dedupe signature"
    );

    fs::remove_file(&dirty_path).expect("operator should be able to repair the dirty candidate");
    let clean_signature_before_commit = service
        .pending_commit_ready_recovery_signature(&repo.workspace_dir())
        .expect("clean recovery signature should load")
        .expect("clean recovery candidate should remain wakeable until delivery");
    assert_ne!(clean_signature_before_commit, dirty_signature);
    assert!(clean_signature_before_commit.contains("source:clean"));
    repo.commit_file_in_slot(
        Path::new(&missing_lease.worktree_path),
        "recover-b-late.txt",
        "late clean recovery evidence\n",
        "record late clean recovery evidence",
    );
    let clean_signature_after_commit = service
        .pending_commit_ready_recovery_signature(&repo.workspace_dir())
        .expect("post-commit recovery signature should load")
        .expect("post-commit recovery candidate should remain wakeable until delivery");
    assert_ne!(clean_signature_after_commit, clean_signature_before_commit);
    assert!(clean_signature_after_commit.contains("source:clean"));
    let recovered_tick = service
        .run_orchestrator_tick_guarded(
            &repo.workspace_dir(),
            ParallelModeOrchestratorTrigger::ManualDispatch,
            &permit,
        )
        .expect("the same automation epoch should retry after external repair");
    assert!(
        recovered_tick.notices.iter().any(|notice| {
            notice.contains("distributor integrated queue head into prerelease")
                && notice.contains("agent-b")
        }),
        "repaired recovery should integrate B in the same epoch: {:?}",
        recovered_tick.notices
    );
    assert_eq!(
        service
            .pending_commit_ready_recovery_signature(&repo.workspace_dir())
            .expect("completed recovery signature should load"),
        None
    );
}

fn prepare_missing_commit_ready_result(
    service: &ParallelModeService,
    repo: &TempGitRepo,
    task_id: &str,
    agent_id: &str,
    task_slug: &str,
) -> ParallelModeSlotLeaseSnapshot {
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request(task_id, "Recoverable", agent_id, task_slug),
        )
        .expect("recovery slot should lease");
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("recovery slot should become running");
    repo.commit_file_in_slot(
        Path::new(&lease.worktree_path),
        &format!("{task_slug}.txt"),
        "recoverable result\n",
        "record recoverable result",
    );
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            &format!("turn-{task_id}"),
            None,
            Some("Recoverable result complete."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should capture");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("official completion should enter refreshing");
    service
        .mark_workspace_commit_ready(&lease.worktree_path, "recoverable result accepted")
        .expect("official completion should become commit-ready");
    lease
}

#[test]
fn stale_base_enqueue_rejects_source_that_already_incorporated_the_advanced_target() {
    let repo = TempGitRepo::new("commit-ready-recovery-source-drift");
    let service =
        test_parallel_mode_service_with_github(Arc::new(FakeGithubAutomationPort::ready()));
    let lease = prepare_missing_commit_ready_result(
        &service,
        &repo,
        "task-source-drift",
        "agent-source-drift",
        "source-drift",
    );
    run_git(&repo.repo_root, &["checkout", POOL_BASELINE_BRANCH]);
    fs::write(repo.repo_root.join("advanced-target.txt"), "advanced\n")
        .expect("advanced target fixture should write");
    run_git(&repo.repo_root, &["add", "advanced-target.txt"]);
    run_git(
        &repo.repo_root,
        &["commit", "-qm", "advance integration target"],
    );
    let advanced_target = run_command(
        "git",
        ["-C", repo.workspace_dir().as_str(), "rev-parse", "HEAD"],
        None,
    )
    .expect("advanced target should resolve");
    run_git(
        &repo.repo_root,
        &["push", "-q", DEFAULT_PUSH_REMOTE_NAME, POOL_BASELINE_BRANCH],
    );
    run_git(
        Path::new(&lease.worktree_path),
        &["rebase", advanced_target.as_str()],
    );

    let error = service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect_err("source that incorporated the target advance must fail closed");
    assert!(
        error.contains("no longer has lease-frozen base"),
        "unexpected source-drift rejection: {error}"
    );
    assert!(load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root()).is_empty());
}

#[test]
fn stale_base_enqueue_rejects_non_linear_target_rewrite() {
    let repo = TempGitRepo::new("commit-ready-recovery-target-rewrite");
    let service =
        test_parallel_mode_service_with_github(Arc::new(FakeGithubAutomationPort::ready()));
    let lease = prepare_missing_commit_ready_result(
        &service,
        &repo,
        "task-target-rewrite",
        "agent-target-rewrite",
        "target-rewrite",
    );
    let rewritten_repo = repo.root.join("rewritten-target");
    fs::create_dir_all(&rewritten_repo).expect("rewritten target repo should exist");
    run_git(&rewritten_repo, &["init", "-q"]);
    run_git(&rewritten_repo, &["config", "user.name", "RefinedStone"]);
    run_git(
        &rewritten_repo,
        &["config", "user.email", "akra@example.invalid"],
    );
    fs::write(rewritten_repo.join("rewritten.txt"), "unrelated history\n")
        .expect("rewritten target fixture should write");
    run_git(&rewritten_repo, &["add", "rewritten.txt"]);
    run_git(
        &rewritten_repo,
        &["commit", "-qm", "rewrite target history"],
    );
    let origin = repo.create_bare_origin_remote();
    run_git(
        &rewritten_repo,
        &[
            "remote",
            "add",
            DEFAULT_PUSH_REMOTE_NAME,
            origin.to_str().expect("origin path should be valid utf-8"),
        ],
    );
    run_git(
        &rewritten_repo,
        &[
            "push",
            "-q",
            "--force",
            DEFAULT_PUSH_REMOTE_NAME,
            &format!("HEAD:{POOL_BASELINE_BRANCH}"),
        ],
    );

    let error = service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect_err("non-linear target rewrite must fail closed");
    assert!(
        error.contains("moved outside lease-frozen history"),
        "unexpected target-rewrite rejection: {error}"
    );
    assert!(load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root()).is_empty());
}

#[test]
fn merge_queued_uses_authority_detail_when_mirror_is_missing_or_stale() {
    for (fixture_name, stale_mirror) in [
        ("merge-queued-missing-session-mirror", false),
        ("merge-queued-stale-session-mirror", true),
    ] {
        let repo = TempGitRepo::new(fixture_name);
        let service = test_parallel_mode_service();
        let lease = service
            .acquire_slot_lease(
                &repo.workspace_dir(),
                sample_lease_request("task-1", "Task One", "agent-1", "task-one"),
            )
            .expect("slot lease should be acquired");
        service
            .mark_workspace_slot_running(&lease.worktree_path)
            .expect("slot should transition to running");
        repo.commit_file_in_slot(
            Path::new(&lease.worktree_path),
            "authority-detail.txt",
            "authority detail survives mirror drift\n",
            "record authority detail result",
        );
        service
            .begin_workspace_official_completion(
                &lease.worktree_path,
                "turn-authority-detail",
                None,
                Some("Final response survives missing or stale mirror state."),
                Some("cargo test --lib preserved authority detail"),
                None,
            )
            .expect("official completion should be captured");
        service
            .mark_workspace_official_completion_refreshing(&lease.worktree_path)
            .expect("ledger refreshing should be recorded");
        service
            .mark_workspace_commit_ready(
                &lease.worktree_path,
                "official authority refresh outcome survives mirror drift",
            )
            .expect("commit-ready should be recorded");

        let session_key = lease.session_key();
        let authority_before =
            SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
                .expect("authority projection should load")
                .session_details
                .into_iter()
                .find(|detail| detail.session_key == session_key)
                .expect("commit-ready authority detail should exist");
        let mirror_path = repo.session_detail_path(&session_key);
        if stale_mirror {
            let mut stale = authority_before.clone();
            stale.latest_summary = "stale mirror summary".to_string();
            stale.validation_summary = "stale mirror validation".to_string();
            stale.authority_refresh_outcome = "stale mirror refresh outcome".to_string();
            stale.history.clear();
            fs::write(
                &mirror_path,
                serde_json::to_string_pretty(&stale).expect("stale mirror should serialize"),
            )
            .expect("stale mirror should be installed");
        } else {
            fs::remove_file(&mirror_path).expect("session mirror should be removable");
        }

        service
            .enqueue_workspace_commit_ready_result(&lease.worktree_path)
            .expect("commit-ready result should enqueue")
            .expect("queue item should be created");

        let projection =
            SqlitePlanningAuthorityAdapter::load_runtime_projections(&repo.workspace_dir())
                .expect("updated authority projection should load");
        let detail = projection
            .session_details
            .iter()
            .find(|detail| detail.session_key == session_key)
            .expect("merge-queued authority detail should remain available");
        assert_eq!(detail.state_label, "merge_queued");
        assert_eq!(detail.completion_state_label, "merge_queued");
        assert_eq!(
            detail.validation_summary,
            authority_before.validation_summary
        );
        assert_eq!(
            detail.authority_refresh_outcome,
            authority_before.authority_refresh_outcome
        );
        assert_eq!(
            &detail.history[..authority_before.history.len()],
            authority_before.history.as_slice()
        );
        assert!(detail.history.iter().any(|entry| {
            entry
                .summary
                .contains("Final response survives missing or stale mirror state.")
        }));
        assert_eq!(
            projection.distributor_queue_records[0].validation_summary,
            authority_before.validation_summary
        );
        assert_eq!(
            projection.distributor_queue_records[0].authority_refresh_outcome,
            authority_before.authority_refresh_outcome
        );
        assert_eq!(
            read_agent_session_detail_record(
                &test_parallel_runtime(),
                &repo.pool_root(),
                &session_key,
            )
            .expect("merge-queued mirror should be projected from authority"),
            *detail
        );
    }
}

#[test]
fn readiness_recovery_marks_stale_ledger_refreshing_session_for_manual_recovery() {
    let repo = TempGitRepo::new("stale-ledger-refreshing");
    let service = test_parallel_mode_service();
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
            "turn-stale-ledger",
            None,
            Some("Completed before the refresh worker disappeared."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("ledger refreshing should be recorded");

    let session_key = lease.session_key();
    let mut detail =
        read_agent_session_detail_record(&test_parallel_runtime(), &repo.pool_root(), &session_key)
            .expect("session detail should be mirrored");
    detail.updated_at = "2020-01-01T00:00:00+00:00".to_string();
    if let Some(history) = detail.history.last_mut() {
        history.timestamp = detail.updated_at.clone();
    }
    SqlitePlanningAuthorityAdapter::upsert_runtime_session_detail(&repo.workspace_dir(), &detail)
        .expect("stale detail should update authority projection");
    fs::write(
        repo.session_detail_path(&session_key),
        serde_json::to_string_pretty(&detail).expect("detail should serialize"),
    )
    .expect("stale detail mirror should update");

    let readiness = service.inspect_readiness(
        &repo.workspace_dir(),
        &PlanningRuntimeProjection::ready("prompt".into(), "queue".into(), None)
            .with_workspace_present(true),
    );
    assert!(readiness.allows_parallel_mode());
    let snapshot = service.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));

    let recovered = snapshot
        .detail
        .session
        .expect("stale session should remain selected for operator recovery");
    assert_eq!(recovered.state_label, "official_refresh_recovery_needed");
    assert_eq!(
        recovered.latest_summary,
        "official completion refresh needs recovery"
    );
    assert!(
        recovered
            .authority_refresh_outcome
            .contains("stale official ledger refresh"),
        "{}",
        recovered.authority_refresh_outcome
    );
    assert_eq!(snapshot.distributor.head_summary, "recovery needed");
    assert_eq!(snapshot.roster.active_count(), 0);
}

#[test]
fn stale_official_refresh_recovery_abandons_only_the_head_order() {
    let repo = TempGitRepo::new("stale-official-refresh-single-order");
    let service = test_parallel_mode_service();
    let first_order =
        SqlitePlanningAuthorityAdapter::reserve_next_official_refresh_order(&repo.workspace_dir())
            .expect("first refresh order should reserve");
    let second_order =
        SqlitePlanningAuthorityAdapter::reserve_next_official_refresh_order(&repo.workspace_dir())
            .expect("second refresh order should reserve");
    assert_eq!(first_order, 1);
    assert_eq!(second_order, 2);

    let readiness = service.inspect_readiness(
        &repo.workspace_dir(),
        &PlanningRuntimeProjection::ready("prompt".into(), "queue".into(), None)
            .with_workspace_present(true),
    );
    assert!(readiness.allows_parallel_mode());

    assert_eq!(
        SqlitePlanningAuthorityAdapter::acquire_official_refresh_claim(
            &repo.workspace_dir(),
            first_order,
            "first-worker",
        )
        .expect("first claim should be readable"),
        PlanningAuthorityOfficialRefreshClaimStatus::AlreadyCompleted
    );
    assert_eq!(
        SqlitePlanningAuthorityAdapter::acquire_official_refresh_claim(
            &repo.workspace_dir(),
            second_order,
            "second-worker",
        )
        .expect("second claim should still be executable"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
}

#[test]
fn passive_readiness_inspection_does_not_recover_or_mutate_refresh_orders() {
    let repo = TempGitRepo::new("passive-readiness-refresh-order");
    let service = test_parallel_mode_service();
    let refresh_order =
        SqlitePlanningAuthorityAdapter::reserve_next_official_refresh_order(&repo.workspace_dir())
            .expect("refresh order should reserve");
    let runtime_projection =
        PlanningRuntimeProjection::ready("prompt".into(), "queue".into(), None)
            .with_workspace_present(true);
    let planning_projection =
        PlanningApplicationProjection::from_runtime_projection(&runtime_projection);

    let readiness = service.inspect_readiness_passively_from_planning_projection(
        &repo.workspace_dir(),
        &planning_projection,
    );

    assert!(readiness.allows_parallel_mode());
    assert_eq!(
        SqlitePlanningAuthorityAdapter::acquire_official_refresh_claim(
            &repo.workspace_dir(),
            refresh_order,
            "dashboard-reader",
        )
        .expect("refresh claim should remain readable"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired,
        "a dashboard/readiness GET must not run distributor recovery"
    );
}

// official completion refresh order는 worker가 실제 완료를 보고하는 순서와 별도로
// 예약된 순서를 따라야 한다. 늦게 시작한 completion이 먼저 보고되어도 feed와
// distributor queue가 stable ordering을 유지하도록 reservation 값을 보존한다.
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

// distributor queue는 head-of-line blocking 모델이다. 첫 번째 item이 source
// worktree 손실로 blocked되면 뒤 item이 준비되어 있어도 통합을 진행하지 않고
// held queue count와 blocked reason을 supervisor에 노출해야 한다.
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
        snapshot.distributor.orchestrator_status.queue_head,
        "agent-1 / task-1 / blocked"
    );
    assert_eq!(
        snapshot.distributor.orchestrator_status.barrier_state,
        "blocked"
    );
    assert_eq!(snapshot.distributor.orchestrator_status.held_queue_count, 1);
    assert!(
        snapshot
            .distributor
            .orchestrator_status
            .blocked_reason
            .as_deref()
            .expect("blocked reason should be surfaced")
            .contains("source worktree is missing")
    );
    assert!(
        snapshot
            .distributor
            .orchestrator_status
            .integration_worktree_readiness
            .contains("dedicated integration worktree"),
        "integration worktree readiness should be surfaced: {}",
        snapshot
            .distributor
            .orchestrator_status
            .integration_worktree_readiness
    );
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

// mirror 파일들이 모두 사라진 재시작 상황에서도 sqlite authority store의 queue
// record가 복구 기준이 된다. source worktree 자체가 없으면 자동 통합 대신 blocked
// record와 failed session detail을 재작성해 operator recovery로 넘긴다.
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
    let queue_record = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
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
    let recovered_record =
        load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
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
    let recovered_detail =
        read_agent_session_detail_record(&test_parallel_runtime(), &repo.pool_root(), &session_key)
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
fn recovery_does_not_treat_a_reset_mutable_source_branch_as_frozen_result_integration() {
    let repo = TempGitRepo::new("distributor-recovery-reset-source-branch");
    let github = FakeGithubAutomationPort::ready();
    let operations = github.operations.clone();
    let service = test_parallel_mode_service_with_github(Arc::new(github));
    let lease = super::blocked::enqueue_single_commit_ready_result(
        &service,
        &repo,
        "turn-reset-source-recovery",
    );
    let queued = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should exist");
    let frozen_commit = queued.effective_source_commit_sha();

    run_git(
        Path::new(&lease.worktree_path),
        &["reset", "--hard", POOL_BASELINE_BRANCH],
    );
    assert!(
        command_succeeds(
            "git",
            [
                "-C",
                lease.worktree_path.as_str(),
                "cat-file",
                "-e",
                &format!("{frozen_commit}^{{commit}}"),
            ],
        ),
        "frozen result must still be addressable by its durable OID"
    );

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("reset source branch should block without cleanup");

    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("branch head drifted from expected commit")),
        "frozen result loss risk should surface as head drift: {notices:?}"
    );
    let recovered = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should remain durable");
    assert_eq!(recovered.queue_state, ParallelModeQueueItemState::Blocked);
    assert!(repo.branch_exists(&lease.branch_name));
    assert!(repo.slot_lease_path(1).exists());
    assert!(
        operations
            .lock()
            .expect("fake github operations mutex poisoned")
            .is_empty(),
        "recovery must not push, create a PR, or clean a reset source branch"
    );
    assert_eq!(
        run_command(
            "git",
            [
                "-C",
                repo.workspace_dir().as_str(),
                "show",
                "refs/remotes/origin/prerelease:feature.txt",
            ],
            None,
        ),
        None,
        "frozen result must not be falsely reported as integrated"
    );
}

// 반대로 queue head의 변경이 이미 `prerelease`에 fast-forward로 들어간 상태라면
// 재시작 후 snapshot은 이를 blocked가 아니라 cleaning으로 재분류해야 한다. mirror
// 손실 후에도 lease를 cleanup-pending으로 되살려 slot 회수 흐름을 이어간다.
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
    let queue_record = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should exist");
    let original_branch = current_branch(&repo.repo_root);
    run_git(&repo.repo_root, &["checkout", "prerelease"]);
    run_git(
        &repo.repo_root,
        &["merge", "--ff-only", lease.branch_name.as_str()],
    );
    run_git(
        &repo.repo_root,
        &["push", DEFAULT_PUSH_REMOTE_NAME, "prerelease"],
    );
    run_git(&repo.repo_root, &["checkout", original_branch.as_str()]);
    assert!(
        command_succeeds(
            "git",
            [
                "-C",
                repo.workspace_dir().as_str(),
                "merge-base",
                "--is-ancestor",
                queue_record.effective_source_commit_sha().as_str(),
                "refs/remotes/origin/prerelease",
            ],
        ),
        "frozen source tip must be a literal ancestor of the recovery target"
    );
    let frozen_states =
        crate::application::service::parallel_mode::distributor::distributor_source_cherry_states(
            &test_parallel_runtime(),
            &repo.workspace_dir(),
            "refs/remotes/origin/prerelease",
            &queue_record,
        )
        .expect("frozen source range should remain classifiable for recovery");
    assert!(
        frozen_states.iter().all(|(_, equivalent)| *equivalent),
        "every frozen source commit should be equivalent in the recovery target: {frozen_states:?}"
    );
    fs::remove_file(repo.slot_lease_path(1)).expect("slot lease mirror should be removed");
    fs::remove_file(repo.session_detail_path(&session_key))
        .expect("session detail mirror should be removed");
    fs::remove_file(repo.distributor_queue_path(&queue_record.queue_item_id))
        .expect("queue mirror should be removed");
    let recovered = test_parallel_mode_service();
    let readiness = recovered.inspect_readiness(
        &repo.workspace_dir(),
        &PlanningRuntimeProjection::ready("prompt".into(), "queue".into(), None)
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

// cleanup 실패로 blocked된 record라도 source branch가 이미 integration branch에
// 들어갔다면 재시작 복구는 사람 개입 상태로 고정하지 않고 slot 반환 단계로
// 다시 보내야 한다. 그렇지 않으면 통합이 끝난 작업이 blocked head로 계속 큐를 막는다.
#[test]
fn distributor_retries_blocked_cleanup_failure_after_integrated_recovery() {
    let repo = TempGitRepo::new("distributor-blocked-cleanup-recovery");
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
            "turn-blocked-cleanup-recovery",
            None,
            Some("Cleanup recovery slice completed."),
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
    repo.merge_agent_slot_into_akra(&slot_path);
    assert!(
        command_succeeds(
            "git",
            [
                "-C",
                repo.workspace_dir().as_str(),
                "merge-base",
                "--is-ancestor",
                lease.branch_name.as_str(),
                POOL_BASELINE_BRANCH,
            ],
        ),
        "source branch should be integrated before cleanup recovery"
    );

    let mut cleanup_pending_lease = lease.clone();
    cleanup_pending_lease.state = ParallelModeSlotLeaseState::CleanupPending;
    write_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &cleanup_pending_lease,
    )
    .expect("cleanup pending lease should be persisted");

    let mut queue_record =
        load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
            .into_iter()
            .next()
            .expect("queue record should exist");
    queue_record.queue_state = ParallelModeQueueItemState::Blocked;
    queue_record.integration_state = "blocked".to_string();
    queue_record.integration_note =
        "slot `slot-1` cleanup failed after distributor delivery".to_string();
    queue_record.recovery_note = Some(queue_record.integration_note.clone());
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &repo.workspace_dir(),
        &queue_record,
    )
    .expect("blocked cleanup queue record should be persisted");

    let recovered = test_parallel_mode_service();
    let notices = recovered
        .process_distributor_queue(&repo.workspace_dir())
        .expect("cleanup recovery should process");
    let readiness = ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    );
    let snapshot =
        recovered.build_supervisor_snapshot(&repo.workspace_dir(), true, Some(&readiness));

    assert!(
        !notices
            .iter()
            .any(|notice| notice.contains("distributor queue head is blocked")),
        "cleanup recovery should not leave a blocked head: {notices:?}"
    );
    assert_eq!(snapshot.distributor.head_summary, "idle");
    assert!(!repo.slot_lease_path(1).exists());
    assert!(!repo.branch_exists(&lease.branch_name));
}

#[test]
fn stale_queue_record_never_adopts_a_reused_slot_session() {
    let repo = TempGitRepo::new("recovery-stale-queue-reused-slot");
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-reused", "Reused slot", "agent-1", "reused-slot"),
        )
        .expect("slot lease should be acquired");
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    let slot_path = PathBuf::from(&lease.worktree_path);
    repo.commit_file_in_slot(&slot_path, "result.txt", "done\n", "stale session result");
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-stale-session",
            None,
            Some("Stale session result."),
            Some("cargo test passed"),
            None,
        )
        .expect("official completion should be captured");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("ledger refreshing should be recorded");
    service
        .mark_workspace_commit_ready(&lease.worktree_path, "stale session ready")
        .expect("commit-ready should be recorded");
    service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect("commit-ready result should enqueue")
        .expect("queue item should be created");
    repo.merge_agent_slot_into_akra(&slot_path);

    let mut stale_record =
        load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
            .into_iter()
            .next()
            .expect("stale queue record should exist");
    stale_record.queue_state = ParallelModeQueueItemState::Blocked;
    stale_record.integration_state = "blocked".to_string();
    stale_record.integration_note = "slot cleanup failed after distributor delivery".to_string();
    stale_record.recovery_note = Some(stale_record.integration_note.clone());
    SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
        &repo.workspace_dir(),
        &stale_record,
    )
    .expect("stale queue record should persist");

    let mut replacement = lease.clone();
    replacement.leased_at = "2099-01-01T00:00:00Z".to_string();
    replacement.lease_generation =
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string());
    replacement.state = ParallelModeSlotLeaseState::Running;
    replacement.running_started_at = Some("2099-01-01T00:00:01Z".to_string());
    assert_ne!(lease.session_key(), replacement.session_key());
    write_slot_lease(
        &SqlitePlanningAuthorityAdapter::new(),
        &test_parallel_runtime(),
        &repo.workspace_dir(),
        &repo.pool_root(),
        &replacement,
    )
    .expect("replacement live lease should persist");

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("stale queue recovery should fail closed");

    assert!(
        notices.iter().any(|notice| {
            notice.contains("stale distributor session")
                && notice.contains("live session")
                && notice.contains("is preserved")
        }),
        "session conflict should be explicit: {notices:?}"
    );
    let preserved = repo.read_slot_lease(1);
    assert_eq!(preserved.session_key(), replacement.session_key());
    assert_eq!(preserved.state, ParallelModeSlotLeaseState::Running);
    assert_eq!(current_branch(&slot_path), replacement.branch_name);
    let blocked = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("stale queue record should remain inspectable");
    assert_eq!(blocked.queue_state, ParallelModeQueueItemState::Blocked);
    assert!(
        blocked
            .integration_note
            .contains("stale distributor session")
    );
}

#[test]
fn distributor_fetch_failure_after_remote_rewrite_preserves_the_only_local_result() {
    let repo = TempGitRepo::new("recovery-fetch-failure-preserves-local-result");
    let origin_path = repo.create_bare_origin_remote();
    let service = test_parallel_mode_service();
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request("task-local", "Local Result", "agent-local", "local-result"),
        )
        .expect("slot lease should be acquired");
    let slot_path = PathBuf::from(&lease.worktree_path);
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("slot should transition to running");
    repo.commit_file_in_slot(
        &slot_path,
        "only-local.txt",
        "preserve me\n",
        "record the only local result",
    );
    let source_commit = run_command(
        "git",
        ["-C", lease.worktree_path.as_str(), "rev-parse", "HEAD"],
        None,
    )
    .expect("source commit should resolve");
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-local-result",
            None,
            Some("The only copy of the result is still local."),
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
            "official ledger refresh succeeded: preserve local result",
        )
        .expect("commit-ready should be recorded");
    service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect("commit-ready result should enqueue")
        .expect("queue item should be created");
    let queue_before = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should exist");

    let original_branch = current_branch(&repo.repo_root);
    run_git(&repo.repo_root, &["checkout", POOL_BASELINE_BRANCH]);
    run_git(&repo.repo_root, &["cherry-pick", source_commit.as_str()]);
    run_git(
        &repo.repo_root,
        &["commit", "--amend", "-qm", "stale equivalent integration"],
    );
    run_git(
        &repo.repo_root,
        &["push", DEFAULT_PUSH_REMOTE_NAME, POOL_BASELINE_BRANCH],
    );
    let fetch_refspec = format!(
        "+refs/heads/{POOL_BASELINE_BRANCH}:{}",
        remote_standard_tracking_ref()
    );
    run_git(
        &repo.repo_root,
        &["fetch", DEFAULT_PUSH_REMOTE_NAME, fetch_refspec.as_str()],
    );
    let tracking_ref = remote_standard_tracking_ref();
    let stale_tracking_head = run_command(
        "git",
        [
            "-C",
            repo.workspace_dir().as_str(),
            "rev-parse",
            tracking_ref.as_str(),
        ],
        None,
    )
    .expect("stale remote-tracking head should resolve");
    assert_ne!(stale_tracking_head, queue_before.source_base_commit_sha);
    run_git(&repo.repo_root, &["checkout", original_branch.as_str()]);

    let remote_integration_ref = local_standard_ref();
    run_git(
        &origin_path,
        &[
            "update-ref",
            remote_integration_ref.as_str(),
            queue_before.source_base_commit_sha.as_str(),
        ],
    );
    assert_eq!(
        run_command(
            "git",
            [
                "--git-dir",
                origin_path
                    .to_str()
                    .expect("origin path should be valid utf-8"),
                "show",
                &format!("{POOL_BASELINE_BRANCH}:only-local.txt"),
            ],
            None,
        ),
        None,
        "the rewritten remote integration branch must not contain the result"
    );
    let remote_source_ref = format!("refs/heads/{}", lease.branch_name);
    assert!(
        !command_succeeds(
            "git",
            [
                "--git-dir",
                origin_path
                    .to_str()
                    .expect("origin path should be valid utf-8"),
                "show-ref",
                "--verify",
                "--quiet",
                remote_source_ref.as_str(),
            ],
        ),
        "the remote source branch must already be absent"
    );

    let unavailable_remote = repo.root.join("unavailable-origin.git");
    run_git(
        &repo.repo_root,
        &[
            "remote",
            "set-url",
            DEFAULT_PUSH_REMOTE_NAME,
            unavailable_remote
                .to_str()
                .expect("unavailable remote path should be valid utf-8"),
        ],
    );

    let notices = service
        .process_distributor_queue(&repo.workspace_dir())
        .expect("remote rewrite should be durably blocked without following the new target");

    assert!(
        notices.iter().any(|notice| {
            notice.contains("immutable distributor delivery target drifted")
                && notice.contains("credential-redacted push URL changed")
        }),
        "remote rewrite must be rejected before any target fetch or cleanup: {notices:?}"
    );
    assert!(repo.branch_exists(&lease.branch_name));
    assert_eq!(current_branch(&slot_path), lease.branch_name);
    assert_eq!(
        run_command(
            "git",
            ["-C", lease.worktree_path.as_str(), "rev-parse", "HEAD"],
            None,
        )
        .as_deref(),
        Some(source_commit.as_str())
    );
    assert_eq!(
        fs::read_to_string(slot_path.join("only-local.txt"))
            .expect("the local result file should survive"),
        "preserve me\n"
    );
    assert_eq!(
        repo.read_slot_lease(1).state,
        ParallelModeSlotLeaseState::Running
    );
    let queue_after = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
        .into_iter()
        .next()
        .expect("queue record should remain durable");
    assert_eq!(queue_after.queue_state, ParallelModeQueueItemState::Blocked);
    assert!(
        queue_after
            .integration_note
            .contains("immutable distributor delivery target drifted"),
        "durable block should retain the immutable-target failure: {}",
        queue_after.integration_note
    );
    assert_eq!(queue_after.delivery_target, queue_before.delivery_target);
}
