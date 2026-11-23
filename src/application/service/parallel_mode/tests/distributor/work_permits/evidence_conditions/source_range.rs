use super::*;
use crate::application::service::parallel_mode::distributor::distributor_source_cherry_states;

fn linear_history(label: &str) -> (TempGitRepo, String, String, String) {
    let repo = TempGitRepo::new(label);
    let base = repo.head_sha();
    run_git(&repo.repo_root, &["checkout", "-b", "source-range"]);
    repo.commit_file_in_slot(
        &repo.repo_root,
        "first.txt",
        "first\n",
        "first source commit",
    );
    let first = repo.head_sha();
    repo.commit_file_in_slot(
        &repo.repo_root,
        "second.txt",
        "second\n",
        "second source commit",
    );
    let tip = repo.head_sha();
    (repo, base, first, tip)
}

fn frozen_record(
    workspace: &str,
    base: &str,
    tip: &str,
) -> PlanningAuthorityDistributorQueueRecord {
    serde_json::from_value(serde_json::json!({
        "queue_item_id":"source-range-check", "session_key":"source-range-session",
        "slot_id":"slot-1", "agent_id":"range-worker", "task_id":"range-task", "task_title":"Frozen range",
        "source_branch":"source-range", "branch_name":"source-range", "worktree_path":workspace,
        "source_base_commit_sha":base, "source_commit_sha":tip, "commit_sha":tip,
        "validation_summary":"", "authority_refresh_outcome":"", "queue_state":"queued",
        "integration_note":"range inspection", "enqueued_at":"2026-11-23T12:00:00Z", "updated_at":"2026-11-23T12:00:00Z"
    })).unwrap()
}

#[test]
fn revision_named_path_cannot_ambiguate_the_frozen_range_or_weaken_merge_rejection() {
    let (repo, base, first, tip) = linear_history("range-path-ambiguity");
    let workspace = repo.workspace_dir();
    let range = format!("{base}..{tip}");
    fs::write(repo.repo_root.join(&range), "this is not a path filter\n")
        .expect("revision-named untracked path should be created");
    let runtime = test_parallel_runtime();
    let states = distributor_source_cherry_states(
        &runtime,
        &workspace,
        &base,
        &frozen_record(&workspace, &base, &tip),
    )
    .expect("revisions must remain revisions even when an identical path exists");
    assert_eq!(states, vec![(first, false), (tip.clone(), false)]);
    assert!(
        distributor_source_cherry_states(
            &runtime,
            &workspace,
            &base,
            &frozen_record(&workspace, &tip, &base),
        )
        .unwrap_err()
        .contains("not an ancestor")
    );

    run_git(&repo.repo_root, &["checkout", "-b", "range-side", &base]);
    repo.commit_file_in_slot(&repo.repo_root, "side.txt", "side\n", "side source commit");
    run_git(&repo.repo_root, &["checkout", "source-range"]);
    run_git(
        &repo.repo_root,
        &["merge", "--no-ff", "--no-edit", "range-side"],
    );
    let merge_tip = repo.head_sha();
    let error = distributor_source_cherry_states(
        &runtime,
        &workspace,
        &base,
        &frozen_record(&workspace, &base, &merge_tip),
    )
    .unwrap_err();
    assert!(
        error.contains("source commit range contains merge commits"),
        "{error}"
    );
}

#[test]
fn long_linked_worktree_resolves_the_same_frozen_range_without_path_ambiguity() {
    let (repo, base, first, tip) = linear_history("range-long-worktree");
    let root_length = repo
        .root
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .count();
    let leaf_prefix = "integration-";
    let padding = 195usize
        .checked_sub(root_length + 1 + leaf_prefix.len())
        .expect("fixture root must leave room for a 195-character linked worktree");
    let worktree = repo
        .root
        .join(format!("{leaf_prefix}{}", "x".repeat(padding)));
    let workspace = worktree.to_str().expect("fixture paths are UTF-8");
    let range = format!("{base}..{tip}");
    assert_eq!(workspace.encode_utf16().count(), 195);
    assert!(workspace.encode_utf16().count() + 1 + range.encode_utf16().count() > 260);
    run_git(
        &repo.repo_root,
        &["worktree", "add", "--detach", workspace, &tip],
    );
    let runtime = test_parallel_runtime();
    for oid in [&base, &tip] {
        let revision = format!("{oid}^{{commit}}");
        let output =
            diagnostic_git_command(&runtime, workspace, &["rev-parse", "--verify", &revision])
                .unwrap();
        assert!(
            output.succeeded(),
            "{}",
            describe_git_result(&Ok(output.clone()))
        );
        assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), oid);
    }
    let record = frozen_record(workspace, &base, &tip);
    let expected = vec![(first, false), (tip, false)];
    assert_eq!(
        distributor_source_cherry_states(&runtime, &repo.workspace_dir(), &base, &record).unwrap(),
        expected
    );
    assert_eq!(
        distributor_source_cherry_states(&runtime, workspace, &base, &record).expect(
            "a long existing linked worktree must not turn the commit range into a path probe"
        ),
        expected
    );
}
