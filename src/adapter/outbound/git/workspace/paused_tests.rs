use super::*;

#[test]
fn uncommitted_changes_and_an_unfinished_merge_block_a_reset() {
    let directory = temp_repository("blocked");
    let repository = directory.display().to_string();
    let base = commit_file(&directory, "a.txt", "one\n", "base");
    commit_file(&directory, "a.txt", "two\n", "second");
    let engine = engine();
    let reset = GitOperation::Reset {
        target: base.clone(),
    };

    std::fs::write(directory.join("a.txt"), "edited\n").unwrap();
    let dirty = engine.plan(&repository, reset.clone()).unwrap();
    assert_eq!(
        dirty
            .blockers
            .iter()
            .map(|blocker| blocker.code)
            .collect::<Vec<_>>(),
        [GitOperationBlockerCode::UncommittedChanges]
    );
    // 막는 이유가 있는 계획은 실행하지 않는다. 파일은 그대로이다.
    let outcome = engine
        .execute(&repository, reset.clone(), dirty.expectation)
        .unwrap();
    assert!(matches!(outcome, GitOperationOutcome::Stale { .. }));
    assert_eq!(
        std::fs::read_to_string(directory.join("a.txt")).unwrap(),
        "edited\n"
    );

    // 추적하지 않는 파일은 막지 않는다.
    git(&directory, &["checkout", "--quiet", "--", "a.txt"]);
    std::fs::write(directory.join("untracked.txt"), "mine\n").unwrap();
    assert!(
        engine
            .plan(&repository, reset.clone())
            .unwrap()
            .blockers
            .is_empty()
    );

    std::fs::write(directory.join(".git/MERGE_HEAD"), format!("{base}\n")).unwrap();
    let merging = engine.plan(&repository, reset).unwrap();
    assert_eq!(
        merging.blockers[0].code,
        GitOperationBlockerCode::OperationInProgress
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn clean_worktree_in_a_paused_sequence_still_blocks_writes() {
    use crate::application::port::outbound::git_history_port::GitHistoryPort;
    let directory = temp_repository("paused-sequence");
    let repository = directory.display().to_string();
    let base = commit_file(&directory, "a.txt", "base\n", "base");
    git(&directory, &["switch", "-c", "topic"]);
    let first = commit_file(&directory, "a.txt", "topic\n", "first");
    let second = commit_file(&directory, "b.txt", "next\n", "second");
    git(&directory, &["switch", "main"]);
    commit_file(&directory, "a.txt", "main\n", "diverged");
    let stopped = git_command()
        .args(["cherry-pick", &first, &second])
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(!stopped.status.success());
    // Committing the conflict resolution clears CHERRY_PICK_HEAD, but does not replay the next pick.
    commit_file(&directory, "a.txt", "resolved\n", "resolution");
    let reset = GitOperation::Reset { target: base };
    assert!(git_text(&directory, &["status", "--porcelain"]).is_empty());
    assert!(!directory.join(".git/CHERRY_PICK_HEAD").exists());
    let fresh_engine = engine();
    let plan = fresh_engine.plan(&repository, reset.clone()).unwrap();
    assert!(
        plan.blockers
            .iter()
            .any(|b| b.code == GitOperationBlockerCode::OperationInProgress)
    );
    assert!(matches!(
        fresh_engine
            .execute(&repository, reset, plan.expectation)
            .unwrap(),
        GitOperationOutcome::Stale { .. }
    ));
    assert_eq!(
        GitHistoryAdapter
            .read_worktree_progress(&repository)
            .unwrap()
            .operation
            .as_deref(),
        Some("cherry-pick/revert")
    );
    git(&directory, &["cherry-pick", "--continue"]);
    assert!(
        GitHistoryAdapter
            .read_worktree_progress(&repository)
            .unwrap()
            .operation
            .is_none()
    );
    std::fs::remove_dir_all(directory).unwrap();
}
