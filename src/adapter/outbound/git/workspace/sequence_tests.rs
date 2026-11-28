use super::*;
use crate::domain::git_operation::cherry_pick::{PickAction, PickPrediction};

#[test]
fn ordered_sequence_previews_and_recovers_across_engines() {
    let directory = temp_repository("pick-restart");
    let cwd = directory.to_str().unwrap();
    let base = commit_file(&directory, "a", "base\n", "base");
    git(&directory, &["switch", "-c", "topic"]);
    let first = commit_file(&directory, "clean", "one\n", "first");
    let second = commit_file(&directory, "a", "topic\n", "second");
    let third = commit_file(&directory, "later", "three\n", "third");
    git(&directory, &["switch", "main"]);
    let before = commit_file(&directory, "a", "main\n", "divergence");
    let runtime = directory.with_extension("journal");
    std::fs::create_dir_all(&runtime).unwrap();
    let boot = |stamp| {
        let path = runtime.clone();
        GitOperationService::new(Arc::new(GitHistoryAdapter),Arc::new(GitWorkspaceAdapter),Arc::new(crate::adapter::outbound::filesystem::git_operation_journal::GitOperationJournalAdapter::new(move |_|Ok(path.clone()))),Arc::new(move || (stamp,11)),Arc::default())
    };
    let all_refs = git_text(&directory, &["show-ref"]);
    let e = boot(1795000001);
    let operation = GitOperation::CherryPick {
        commits: vec![third.clone(), first.clone(), second.clone(), base],
    };
    let plan = e.plan(cwd, operation.clone()).unwrap();
    assert_eq!(
        plan.picks.iter().map(|p| &p.prediction).collect::<Vec<_>>(),
        [
            &PickPrediction::AlreadyApplied,
            &PickPrediction::Clean,
            &PickPrediction::Conflict {
                paths: vec!["a".to_owned()]
            },
            &PickPrediction::AfterConflict
        ]
    );
    assert_eq!(git_text(&directory, &["rev-parse", "HEAD"]), before);
    let GitOperationOutcome::Paused { record } =
        e.execute(cwd, operation, plan.expectation).unwrap()
    else {
        panic!("expected conflict");
    };
    assert_eq!(record.pick_index, Some(1));
    drop(e);
    let restored = boot(1795000002);
    std::fs::write(directory.join("a"), "resolved\n").unwrap();
    git(&directory, &["add", "a"]);
    let GitOperationOutcome::Done { record: done } = restored
        .resolve_pick(cwd, &record.op_id, PickAction::Continue)
        .unwrap()
    else {
        panic!("expected completion");
    };
    assert_eq!(done.pick_index, Some(3));
    assert!(directory.join("later").is_file());
    assert!(matches!(
        restored.undo(cwd, &done.op_id).unwrap(),
        GitOperationOutcome::Done { .. }
    ));
    assert_eq!(git_text(&directory, &["rev-parse", "HEAD"]), before);
    assert_eq!(git_text(&directory, &["show-ref"]), all_refs);
    std::fs::remove_dir_all(runtime).unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn skip_and_abort_only_recover_the_owned_sequence() {
    for action in [PickAction::Skip, PickAction::Abort] {
        let directory = temp_repository(&format!("skip-abort-{action:?}"));
        let cwd = directory.to_str().unwrap();
        commit_file(&directory, "a", "base", "base");
        git(&directory, &["switch", "-c", "topic"]);
        let clean = commit_file(&directory, "b", "first", "clean");
        let conflict = commit_file(&directory, "a", "topic", "conflict");
        let later = commit_file(&directory, "c", "later", "later");
        git(&directory, &["switch", "main"]);
        commit_file(&directory, "a", "main", "diverged");
        let initial = git_text(&directory, &["show-ref"]);
        let e = engine();
        let op = GitOperation::CherryPick {
            commits: vec![clean, conflict, later],
        };
        let plan = e.plan(cwd, op.clone()).unwrap();
        let GitOperationOutcome::Paused { record } = e.execute(cwd, op, plan.expectation).unwrap()
        else {
            panic!("expected conflict")
        };
        let GitOperationOutcome::Done { record: after } =
            e.resolve_pick(cwd, &record.op_id, action).unwrap()
        else {
            panic!("expected completion")
        };
        match action {
            PickAction::Abort => {
                assert_eq!(
                    after.status,
                    crate::domain::git_operation::GitOperationStatus::Aborted
                );
                assert_eq!(git_text(&directory, &["show-ref"]), initial);
            }
            PickAction::Skip => {
                assert_eq!(
                    std::fs::read_to_string(directory.join("a")).unwrap(),
                    "main"
                );
                assert!(directory.join("c").exists());
            }
            PickAction::Continue => unreachable!(),
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
#[test]
fn equivalent_patch_is_excluded_and_merge_input_is_rejected_before_writing() {
    let directory = temp_repository("pick-duplicates");
    let cwd = directory.to_str().unwrap();
    commit_file(&directory, "a", "base", "base");
    git(&directory, &["switch", "-c", "source"]);
    let source = commit_file(&directory, "b", "same", "source id");
    git(&directory, &["switch", "main"]);
    commit_file(&directory, "b", "same", "different id");
    let e = engine();
    let plan = e
        .plan(
            cwd,
            GitOperation::CherryPick {
                commits: vec![source],
            },
        )
        .unwrap();
    assert_eq!(plan.picks[0].prediction, PickPrediction::AlreadyApplied);
    assert!(!plan.blockers.is_empty());
    git(&directory, &["merge", "--no-ff", "-m", "join", "source"]);
    let head = git_text(&directory, &["rev-parse", "HEAD"]);
    assert!(
        e.plan(
            cwd,
            GitOperation::CherryPick {
                commits: vec![head]
            }
        )
        .unwrap_err()
        .to_string()
        .contains("merge commit")
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn abort_intent_written_before_backup_releases_the_worktree_without_changing_refs() {
    let directory = temp_repository("pick-before-backup");
    let cwd = directory.to_str().unwrap();
    let commit = commit_file(&directory, "a", "base", "base");
    let initial = git_text(&directory, &["show-ref"]);
    let position = crate::domain::git_operation::GitHeadPosition {
        branch: Some("main".into()),
        commit: commit.clone(),
    };
    let journal = Arc::new(MemoryJournal::default());
    journal
        .append(
            cwd,
            &GitOperationRecord {
                op_id: "interrupted".into(),
                worktree: cwd.into(),
                operation: GitOperation::CherryPick {
                    commits: vec![commit],
                },
                before: position.clone(),
                after: position,
                backup_ref: Some("refs/akra/backup/main/interrupted".into()),
                status: crate::domain::git_operation::GitOperationStatus::Started,
                started_at: 1,
                finished_at: None,
                undo_of: None,
                undone_by: None,
                error: None,
                pick_index: Some(0),
            },
        )
        .unwrap();
    let e = GitOperationService::new(
        Arc::new(GitHistoryAdapter),
        Arc::new(GitWorkspaceAdapter),
        journal,
        Arc::new(|| (2, 1)),
        Arc::default(),
    );
    assert!(
        matches!(e.resolve_pick(cwd,"interrupted",PickAction::Abort).unwrap(),GitOperationOutcome::Done {record} if record.status==crate::domain::git_operation::GitOperationStatus::Aborted)
    );
    assert_eq!(git_text(&directory, &["show-ref"]), initial);
    std::fs::remove_dir_all(directory).unwrap();
}
