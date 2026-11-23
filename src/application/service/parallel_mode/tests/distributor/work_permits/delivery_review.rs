use super::*;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::application::port::outbound::parallel_mode_runtime_port::{
    ParallelCommitDiffRequest, ParallelModeRuntimePort,
};
use crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkDeliveryReview;
use crate::domain::issue_automation::delivery::{IssueDeliveryRiskReason, assess_delivery_diff};
use crate::domain::issue_automation::policy::IssueAutomationPolicy;

fn install_legacy_delivery_grants(fixture: &WorkPermitDeliveryFixture) {
    let grants = PlanningWorkPermitStage::ALL.map(|stage| PlanningWorkPermit {
        work_order_id: WORK_ORDER_ID.into(),
        stage,
        basis: fixture.basis.clone(),
        requires_human_review: stage == PlanningWorkPermitStage::Integrate,
    });
    let old_payload = serde_json::to_value(&grants).unwrap();
    assert!(
        old_payload
            .as_array()
            .unwrap()
            .iter()
            .all(|grant| grant["basis"].get("delivery_review").is_none())
    );
    let location = SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(
        &fixture.repo.workspace_dir(),
    )
    .unwrap();
    let connection = rusqlite::Connection::open(&location.authority_store_path).unwrap();
    // Simulate a pre-upgrade host that already issued every delivery stage permit.
    connection
        .execute(
            "UPDATE planning_work_orders SET permits_json=?2 WHERE work_order_id=?1",
            rusqlite::params![WORK_ORDER_ID, serde_json::to_string(&old_payload).unwrap()],
        )
        .unwrap();
}

#[test]
fn legacy_issue_permits_cannot_push_or_create_a_pr_before_coordinator_diff_reassessment() {
    let fixture = WorkPermitDeliveryFixture::with_origin_and_link_reference(
        "legacy-issue-diff-review-remote-fence",
        FakeGithubAutomationPort::ready(),
        "issue_automation",
        None,
    );
    install_legacy_delivery_grants(&fixture);
    let authority = SqlitePlanningAuthorityAdapter::new();
    let before = authority
        .load_work_order(&fixture.repo.workspace_dir(), WORK_ORDER_ID)
        .unwrap()
        .unwrap();
    assert_eq!(before.permits.len(), 4);
    for stage in [
        PlanningWorkPermitStage::Push,
        PlanningWorkPermitStage::PullRequest,
        PlanningWorkPermitStage::Integrate,
    ] {
        assert_eq!(
            authority
                .check_work_order_permit_for_task(
                    &fixture.repo.workspace_dir(),
                    TASK_ID,
                    stage,
                    Some(&fixture.basis.source_sha)
                )
                .unwrap()
                .blocked_reason(),
            Some("permit_missing"),
        );
    }
    let remote_before = remote_heads(&fixture.repo);
    let notices = fixture.process();
    fixture.assert_blocked("permit_missing", &notices);
    assert!(
        fixture.operations().is_empty(),
        "old Start permits must not publish any source or PR: {notices:?}"
    );
    assert_eq!(remote_heads(&fixture.repo), remote_before);

    let restarted =
        test_parallel_mode_service_with_autonomous_github(Arc::new(fixture.github.clone()));
    restarted
        .process_distributor_queue(&fixture.repo.workspace_dir())
        .unwrap();
    assert!(fixture.operations().is_empty());
    assert_eq!(remote_heads(&fixture.repo), remote_before);
    assert_eq!(
        authority
            .load_work_order(&fixture.repo.workspace_dir(), WORK_ORDER_ID)
            .unwrap()
            .unwrap(),
        before
    );
}

fn enqueue_protected_result(
    service: &ParallelModeService,
    repo: &TempGitRepo,
    turn_id: &str,
) -> ParallelModeSlotLeaseSnapshot {
    let lease = service
        .acquire_slot_lease(
            &repo.workspace_dir(),
            sample_lease_request(TASK_ID, "Protected result", "agent-1", "protected-result"),
        )
        .unwrap();
    service
        .mark_workspace_slot_running(&lease.worktree_path)
        .unwrap();
    let worktree = Path::new(&lease.worktree_path);
    fs::create_dir_all(worktree.join(".github/workflows")).unwrap();
    repo.commit_file_in_slot(
        worktree,
        ".github/workflows/delivery-review.yml",
        "name: protected earlier change\n",
        "protected change before the last commit",
    );
    repo.commit_file_in_slot(
        worktree,
        "reviewed-result.txt",
        "ordinary final change\n",
        "ordinary last commit",
    );
    service
        .begin_workspace_official_completion(
            &lease.worktree_path,
            turn_id,
            None,
            Some("The fixture committed the protected and ordinary results."),
            Some("Synthetic worker result; the host diff inspector verifies the full range."),
            None,
        )
        .unwrap();
    service
        .mark_workspace_official_completion_refreshing(&lease.worktree_path)
        .unwrap();
    service
        .mark_workspace_commit_ready(&lease.worktree_path, "fixture official refresh completed")
        .unwrap();
    assert!(
        service
            .enqueue_workspace_commit_ready_result(&lease.worktree_path)
            .unwrap()
            .is_some()
    );
    lease
}

fn protected_fixture(label: &str) -> WorkPermitDeliveryFixture {
    WorkPermitDeliveryFixture::with_result_builder(
        label,
        FakeGithubAutomationPort::with_review_decision("REVIEW_REQUIRED"),
        "issue_automation",
        None,
        enqueue_protected_result,
    )
}

fn review_the_complete_protected_range(
    fixture: &WorkPermitDeliveryFixture,
) -> PlanningWorkPermitBasis {
    let base = &fixture
        .lease
        .delivery_target
        .as_ref()
        .unwrap()
        .integration_base_commit_sha;
    let runtime = GitParallelModeRuntimeAdapter::new();
    let complete = runtime
        .inspect_parallel_commit_diff(ParallelCommitDiffRequest {
            workspace_directory: &fixture.lease.worktree_path,
            base_commit_sha: base,
            source_commit_sha: &fixture.basis.source_sha,
        })
        .unwrap();
    let policy = IssueAutomationPolicy::builtin();
    let verdict = assess_delivery_diff(&policy, &complete).unwrap();
    assert_eq!(verdict.reasons, [IssueDeliveryRiskReason::ProtectedPath]);
    assert_eq!(verdict.changed_files, 2);
    let parent = run_command(
        "git",
        [
            "-C",
            fixture.lease.worktree_path.as_str(),
            "rev-parse",
            "HEAD^",
        ],
        None,
    )
    .unwrap();
    let final_commit_only = runtime
        .inspect_parallel_commit_diff(ParallelCommitDiffRequest {
            workspace_directory: &fixture.lease.worktree_path,
            base_commit_sha: &parent,
            source_commit_sha: &fixture.basis.source_sha,
        })
        .unwrap();
    assert!(
        !assess_delivery_diff(&policy, &final_commit_only)
            .unwrap()
            .requires_human()
    );

    // Human confirmation is verified by the coordinator tests. This boundary consumes its
    // exact reviewed range and independently persists the matching planning evidence.
    let revision = fixture.basis.gate_revision + 1;
    let reviewed = PlanningWorkPermitBasis {
        gate_revision: revision,
        delivery_review: Some(PlanningWorkDeliveryReview {
            base_commit_sha: complete.base_commit_sha,
            source_commit_sha: complete.source_commit_sha,
            gate_revision: revision,
            authority: None,
        }),
        ..fixture.basis.clone()
    };
    set_basis_and_grant(
        &fixture.repo.workspace_dir(),
        &reviewed,
        &[
            PlanningWorkPermitStage::Push,
            PlanningWorkPermitStage::PullRequest,
            PlanningWorkPermitStage::Integrate,
        ],
        true,
    );
    reviewed
}

#[test]
fn protected_full_range_waits_for_review_then_publishes_only_the_exact_reviewed_source() {
    let fixture = protected_fixture("protected-full-range-reviewed-delivery");
    let before = remote_heads(&fixture.repo);
    let integration_before = remote_integration_head(&fixture.repo);
    let blocked = fixture.process();
    fixture.assert_blocked("permit_missing", &blocked);
    assert!(fixture.operations().is_empty());
    assert_eq!(remote_heads(&fixture.repo), before);

    let reviewed = review_the_complete_protected_range(&fixture);
    let notices = fixture.process();
    let operations = fixture.operations();
    assert_eq!(
        operations
            .iter()
            .filter(|operation| operation.starts_with("push:"))
            .count(),
        1,
        "{notices:?}"
    );
    assert_eq!(
        operations
            .iter()
            .filter(|operation| operation.starts_with("ensure-pr:"))
            .count(),
        1,
        "{notices:?}"
    );
    let published = format!(
        "refs/heads/{} {}",
        fixture.lease.branch_name, reviewed.source_sha
    );
    assert!(
        remote_heads(&fixture.repo)
            .lines()
            .any(|line| line == published)
    );
    assert_eq!(remote_integration_head(&fixture.repo), integration_before);
    fixture.assert_no_integration_push();
}

#[test]
fn a_commit_added_after_protected_diff_review_cannot_push_or_create_a_pr() {
    let fixture = protected_fixture("protected-reviewed-source-drift");
    let reviewed = review_the_complete_protected_range(&fixture);
    fixture.repo.commit_file_in_slot(
        Path::new(&fixture.lease.worktree_path),
        "after-review.txt",
        "new result after approval\n",
        "source changed after review",
    );
    let changed_source = run_command(
        "git",
        [
            "-C",
            fixture.lease.worktree_path.as_str(),
            "rev-parse",
            "HEAD",
        ],
        None,
    )
    .unwrap();
    assert_ne!(changed_source, reviewed.source_sha);
    for stage in [
        PlanningWorkPermitStage::Push,
        PlanningWorkPermitStage::PullRequest,
    ] {
        assert_eq!(
            SqlitePlanningAuthorityAdapter::new()
                .check_work_order_permit_for_task(
                    &fixture.repo.workspace_dir(),
                    TASK_ID,
                    stage,
                    Some(&changed_source),
                )
                .unwrap()
                .blocked_reason(),
            Some("permit_stale")
        );
    }
    let before = remote_heads(&fixture.repo);
    let notices = fixture.process();
    fixture.assert_blocked("branch head drifted", &notices);
    assert!(
        fixture.operations().is_empty(),
        "old review must not publish a changed source: {notices:?}"
    );
    assert_eq!(remote_heads(&fixture.repo), before);
}
