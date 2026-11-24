use super::*;

fn reviewed_source() -> (
    ParallelModeDistributorQueueRecord,
    GithubAutomationPullRequest,
) {
    let source = "a".repeat(40);
    let record = serde_json::from_value(serde_json::json!({
        "queue_item_id": "queue:auto",
        "session_key": "session:auto",
        "agent_id": "agent-1",
        "task_id": "task-1",
        "task_title": "Auto checks",
        "source_branch": "agent/source",
        "source_commit_sha": source,
        "branch_name": "agent/source",
        "worktree_path": "/unused/slot",
        "commit_sha": source,
        "validation_summary": "internal validation retained",
        "authority_refresh_outcome": "complete",
        "queue_state": "merge_pending",
        "integration_note": "preflight",
        "enqueued_at": "2026-11-23T12:00:00Z",
        "updated_at": "2026-11-23T12:00:00Z"
    }))
    .unwrap();
    let pull_request = GithubAutomationPullRequest::new(
        77,
        "https://github.com/example/repository/pull/77",
        "OPEN",
        "prerelease",
        "agent/source",
        false,
    )
    .with_merge_gate("REVIEW_REQUIRED", "CLEAN", true)
    .with_head_commit_sha(source);
    (record, pull_request)
}

#[test]
fn autonomous_pr_readiness_requires_known_passing_checks_and_clean_mergeability() {
    let (record, ready) = reviewed_source();
    assert!(pull_request_readiness_error(&ready, &record, "prerelease", true, true).is_none());
    for checks in [None, Some(false)] {
        let mut pull_request = ready.clone();
        pull_request.required_checks_passed = checks;
        let reason = pull_request_readiness_error(&pull_request, &record, "prerelease", true, true)
            .expect("unknown or failed required checks must block Auto");
        assert!(reason.contains("required checks"), "{reason}");
    }
    for mergeability in [None, Some("UNKNOWN"), Some("DIRTY"), Some("BLOCKED")] {
        let mut pull_request = ready.clone();
        pull_request.merge_state_status = mergeability.map(str::to_owned);
        let reason = pull_request_readiness_error(&pull_request, &record, "prerelease", true, true)
            .expect("only explicit CLEAN is usable for Auto");
        assert!(reason.contains("CLEAN"), "{reason}");
    }
}

#[test]
fn legacy_autonomous_prs_without_status_checks_keep_the_previous_readiness_contract() {
    let (record, mut pull_request) = reviewed_source();
    pull_request.required_checks_passed = None;
    pull_request.merge_state_status = Some("UNKNOWN".into());
    assert!(
        pull_request_readiness_error(&pull_request, &record, "prerelease", true, false,).is_none()
    );
}
#[test]
fn autonomous_pr_readiness_preserves_the_frozen_head_and_target() {
    let (record, ready) = reviewed_source();
    for head in [None, Some("b".repeat(40))] {
        let mut pull_request = ready.clone();
        pull_request.head_commit_sha = head;
        assert!(
            pull_request_readiness_error(&pull_request, &record, "prerelease", true, true)
                .unwrap()
                .contains("frozen source commit")
        );
    }
    let mut pull_request = ready;
    pull_request.base_branch = "other-base".into();
    assert!(
        pull_request_readiness_error(&pull_request, &record, "prerelease", true, true)
            .unwrap()
            .contains("targets")
    );
}

#[test]
fn lack_of_process_opt_in_requires_human_approval_of_the_exact_source() {
    let (record, mut pull_request) = reviewed_source();
    assert!(
        pull_request_readiness_error(&pull_request, &record, "prerelease", false, false)
            .unwrap()
            .contains("APPROVED")
    );
    pull_request.review_decision = Some("APPROVED".into());
    pull_request.approved_review_commit_shas = vec!["b".repeat(40)];
    assert!(
        pull_request_readiness_error(&pull_request, &record, "prerelease", false, false)
            .unwrap()
            .contains("frozen source commit")
    );
    pull_request.approved_review_commit_shas = vec![record.source_commit_sha.clone()];
    assert!(
        pull_request_readiness_error(&pull_request, &record, "prerelease", false, false).is_none()
    );
}
