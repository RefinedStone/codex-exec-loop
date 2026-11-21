use super::*;

#[test]
fn legacy_delivery_advances_shared_worktree_after_work_order_integration() {
    let fixture = WorkPermitDeliveryFixture::new(
        "work-permit-legacy-work-order-legacy",
        FakeGithubAutomationPort::ready(),
    );
    // Queue B first but withhold delivery permits so legacy A is delivered first.
    let waiting = fixture.process();
    fixture.assert_blocked("permit_missing", &waiting);
    let service = test_parallel_mode_service_with_autonomous_github(Arc::new(
        GitBackedGithubAutomationPort::ready(),
    ));
    enqueue_unrelated_delivery(&service, &fixture.repo);
    let notices = service
        .process_distributor_queue(&fixture.repo.workspace_dir())
        .expect("legacy A should complete while B waits for permission");
    let records =
        load_distributor_queue_records(&test_parallel_runtime(), &fixture.repo.pool_root());
    let first = records
        .iter()
        .find(|record| record.task_id == "task-2")
        .expect("legacy A should retain its queue record");
    assert_eq!(
        first.queue_state,
        ParallelModeQueueItemState::Done,
        "{notices:?}"
    );
    let target = first.delivery_target.as_ref().unwrap();
    let shared_path = derive_integration_worktree_path(
        &fixture.repo.pool_root(),
        &target.push_remote,
        &target.github_repository,
        &target.integration_branch,
    );
    let head_at = |path: &Path| {
        run_command(
            "git",
            ["-C", path.to_str().unwrap(), "rev-parse", "HEAD"],
            None,
        )
        .expect("integration worktree HEAD should resolve")
    };
    let first_head = remote_integration_head(&fixture.repo);
    assert_eq!(head_at(&shared_path), first_head);

    fixture.grant(
        &[
            PlanningWorkPermitStage::Push,
            PlanningWorkPermitStage::PullRequest,
            PlanningWorkPermitStage::Integrate,
        ],
        false,
    );
    let notices = fixture.process();
    let records =
        load_distributor_queue_records(&test_parallel_runtime(), &fixture.repo.pool_root());
    let work_order = records
        .iter()
        .find(|record| record.task_id == TASK_ID)
        .expect("B should retain its work-order queue record");
    assert_eq!(
        work_order.queue_state,
        ParallelModeQueueItemState::Done,
        "{notices:?}"
    );
    let work_order_head = remote_integration_head(&fixture.repo);
    assert_ne!(work_order_head, first_head);
    assert_eq!(
        head_at(&shared_path),
        first_head,
        "B must use its separate integration worktree and leave A's shared HEAD behind"
    );
    assert!(command_succeeds(
        "git",
        [
            "-C",
            shared_path.to_str().unwrap(),
            "merge-base",
            "--is-ancestor",
            first_head.as_str(),
            work_order_head.as_str(),
        ]
    ));

    let lease = service
        .acquire_slot_lease(
            &fixture.repo.workspace_dir(),
            sample_lease_request("task-3", "Task Three", "agent-3", "task-three"),
        )
        .expect("legacy C should acquire a slot from B's updated target");
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .expect("legacy C should start");
    fixture.repo.commit_file_in_slot(
        Path::new(&lease.worktree_path),
        "after-work-order.txt",
        "third result\n",
        "legacy result after work-order integration",
    );
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            "turn-after-work-order",
            None,
            Some("Legacy C completed."),
            Some("fixture validation passed"),
            None,
        )
        .expect("legacy C completion should persist");
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .expect("legacy C official refresh should persist");
    service
        .mark_workspace_commit_ready(&lease.worktree_path, "legacy C is ready")
        .expect("legacy C should become commit-ready");
    service
        .enqueue_workspace_commit_ready_result(&lease.worktree_path)
        .expect("legacy C should enqueue")
        .expect("legacy C should have a queue identity");
    let queued =
        load_distributor_queue_records(&test_parallel_runtime(), &fixture.repo.pool_root())
            .into_iter()
            .find(|record| record.task_id == "task-3")
            .expect("legacy C should retain its queue record");
    assert!(queued.integration_base_commit_sha.is_none());
    assert!(queued.integration_commit_sha.is_none());

    let notices = service
        .process_distributor_queue(&fixture.repo.workspace_dir())
        .expect("legacy C should advance the shared worktree and complete");
    let records =
        load_distributor_queue_records(&test_parallel_runtime(), &fixture.repo.pool_root());
    assert_eq!(records.len(), 3);
    assert!(
        records
            .iter()
            .all(|record| record.queue_state == ParallelModeQueueItemState::Done),
        "A, B, and C should all finish: {records:?}; notices: {notices:?}"
    );
    let last = records
        .iter()
        .find(|record| record.task_id == "task-3")
        .unwrap();
    assert_eq!(
        last.integration_base_commit_sha.as_deref(),
        Some(work_order_head.as_str()),
        "C must freeze B's current remote result as its integration base"
    );
    let final_head = remote_integration_head(&fixture.repo);
    assert_ne!(final_head, work_order_head);
    assert_eq!(head_at(&shared_path), final_head);
    for (file, contents) in [
        ("unrelated.txt", "independent result"),
        ("feature.txt", "done"),
        ("after-work-order.txt", "third result"),
    ] {
        assert_eq!(
            run_command(
                "git",
                [
                    "-C",
                    fixture.repo.workspace_dir().as_str(),
                    "show",
                    &format!("{final_head}:{file}"),
                ],
                None,
            )
            .as_deref(),
            Some(contents),
            "the final integration must retain each delivered result"
        );
    }
}

#[test]
fn legacy_shared_worktree_reuse_preserves_saved_dirty_and_diverged_state() {
    for (scenario, expected_reason) in [
        ("saved-base", "integration target moved from frozen base"),
        ("dirty", "integration worktree must be clean"),
        ("diverged", "without a persisted integration base"),
    ] {
        let repo = TempGitRepo::new(&format!("legacy-worktree-reuse-{scenario}"));
        let github = FakeGithubAutomationPort::ready();
        let operations = github.operations.clone();
        let service = test_parallel_mode_service_with_autonomous_github(Arc::new(github));
        blocked::enqueue_single_commit_ready_result(&service, &repo, scenario);
        let mut record =
            load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
                .into_iter()
                .next()
                .expect("the legacy delivery should be queued");
        let target = record.delivery_target.as_ref().unwrap();
        let shared_path = derive_integration_worktree_path(
            &repo.pool_root(),
            &target.push_remote,
            &target.github_repository,
            &target.integration_branch,
        );
        fs::create_dir_all(shared_path.parent().unwrap())
            .expect("integration worktree parent should exist");
        let original_head = repo.head_sha();
        run_git(
            &repo.repo_root,
            &[
                "worktree",
                "add",
                "--detach",
                shared_path.to_str().unwrap(),
                original_head.as_str(),
            ],
        );
        // Advance only the remote target. The shared integration worktree remains older.
        run_git(&repo.repo_root, &["checkout", POOL_BASELINE_BRANCH]);
        repo.commit_on_current_branch(
            "remote-progress.txt",
            "independent remote progress\n",
            "advance remote integration target",
        );
        let remote_before = remote_integration_head(&repo);
        assert_ne!(remote_before, original_head);

        match scenario {
            "saved-base" => {
                // A restart after freezing the base must not adopt a different target.
                record.integration_base_commit_sha = Some(original_head);
                SqlitePlanningAuthorityAdapter::upsert_runtime_distributor_queue_record(
                    &repo.workspace_dir(),
                    &record,
                )
                .expect("the previously frozen integration base should persist");
            }
            "dirty" => {
                fs::write(
                    shared_path.join("preserved-local.txt"),
                    "preserve local state\n",
                )
                .expect("operator changes should be present in the shared worktree");
            }
            "diverged" => {
                repo.commit_file_in_slot(
                    &shared_path,
                    "preserved-local.txt",
                    "preserve local state\n",
                    "preserve a local integration commit",
                );
            }
            _ => unreachable!(),
        }
        let head_at = || {
            run_command(
                "git",
                ["-C", shared_path.to_str().unwrap(), "rev-parse", "HEAD"],
                None,
            )
            .expect("the shared integration HEAD should resolve")
        };
        let local_before = head_at();
        let notices = service
            .process_distributor_queue(&repo.workspace_dir())
            .expect("unsafe shared worktree reuse should be durably blocked");
        let blocked = load_distributor_queue_records(&test_parallel_runtime(), &repo.pool_root())
            .into_iter()
            .next()
            .expect("the blocked delivery should retain its queue record");
        assert_eq!(
            blocked.queue_state,
            ParallelModeQueueItemState::Blocked,
            "{scenario}: {notices:?}"
        );
        assert!(
            blocked.integration_note.contains(expected_reason),
            "{scenario}: {}; notices: {notices:?}",
            blocked.integration_note
        );
        assert_eq!(
            head_at(),
            local_before,
            "{scenario}: preserve the local HEAD"
        );
        assert_eq!(
            remote_integration_head(&repo),
            remote_before,
            "{scenario}: do not publish"
        );
        assert_eq!(
            blocked.integration_base_commit_sha, record.integration_base_commit_sha,
            "{scenario}: preserve the saved integration evidence"
        );
        assert!(blocked.integration_commit_sha.is_none());
        assert!(
            !operations
                .lock()
                .expect("fake GitHub operations should remain readable")
                .iter()
                .any(|operation| operation.starts_with("push-integration:")),
            "{scenario}: do not call the integration push adapter"
        );
        if scenario != "saved-base" {
            assert_eq!(
                fs::read_to_string(shared_path.join("preserved-local.txt"))
                    .expect("local work should survive the blocked delivery"),
                "preserve local state\n",
                "{scenario}: preserve local files"
            );
        }
    }
}
