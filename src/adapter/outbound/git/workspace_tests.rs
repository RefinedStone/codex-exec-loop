//! `workspace.rs` 의 시험이다. 실제 git 과 실제 작업 엔진을 임시 저장소에서 돌린다. 작업 엔진(application 서비스)을
//! 쓰므로 adapter 의 본문과 다른 파일에 둔다: outbound adapter 의 본문은 서비스에 의존하지 않는다.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use super::*;
use crate::adapter::outbound::git::history::GitHistoryAdapter;
use crate::application::port::inbound::git_operation_port::{
    GitOperationOutcome, GitOperationPort,
};
use crate::application::port::outbound::git_operation_journal_port::GitOperationJournalPort;
use crate::application::service::git_operation::GitOperationService;
use crate::domain::git_operation::{
    GitOperation, GitOperationBlockerCode, GitOperationRecord, fold_git_operation_records,
};
use crate::test_utils::git_command;

fn git(directory: &Path, args: &[&str]) {
    let status = git_command()
        .args(args)
        .current_dir(directory)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("git should run");
    assert!(status.success(), "git {args:?} failed");
}

fn git_text(directory: &Path, args: &[&str]) -> String {
    let output = git_command()
        .args(args)
        .current_dir(directory)
        .output()
        .expect("git should run");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn temp_repository(label: &str) -> PathBuf {
    let directory = crate::test_utils::platform_safe_temp_dir()
        .join(format!("akra-git-workspace-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    git(&directory, &["init", "--quiet", "--initial-branch", "main"]);
    git(&directory, &["config", "user.name", "Akra Test"]);
    git(
        &directory,
        &["config", "user.email", "akra-test@example.com"],
    );
    directory
}

/// 파일 하나를 쓰고 커밋한다. 그 커밋의 sha 를 돌려준다.
fn commit_file(directory: &Path, name: &str, body: &str, subject: &str) -> String {
    std::fs::write(directory.join(name), body).unwrap();
    git(directory, &["add", name]);
    git(directory, &["commit", "--quiet", "-m", subject]);
    git_text(directory, &["rev-parse", "HEAD"])
}

/// 브랜치와 태그와 HEAD 이다. 백업 ref 는 뺀다: 작업과 그 되돌리기 뒤에도 남는다.
fn refs_of(directory: &Path) -> BTreeMap<String, String> {
    let mut refs: BTreeMap<String, String> = git_text(
        directory,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/heads",
            "refs/tags",
        ],
    )
    .lines()
    .filter_map(|line| line.split_once(' '))
    .map(|(name, sha)| (name.to_string(), sha.to_string()))
    .collect();
    refs.insert(
        "HEAD".to_string(),
        git_text(directory, &["rev-parse", "HEAD"]),
    );
    refs.insert(
        "HEAD branch".to_string(),
        git_text(directory, &["branch", "--show-current"]),
    );
    refs
}

#[derive(Default)]
struct MemoryJournal {
    lines: Mutex<Vec<GitOperationRecord>>,
}

impl GitOperationJournalPort for MemoryJournal {
    fn append(&self, _repository_dir: &str, record: &GitOperationRecord) -> Result<()> {
        self.lines.lock().unwrap().push(record.clone());
        Ok(())
    }

    fn load(&self, _repository_dir: &str) -> Result<Vec<GitOperationRecord>> {
        Ok(fold_git_operation_records(
            self.lines.lock().unwrap().clone(),
        ))
    }
}

/// 실제 git 과 실제 서비스이다. 일지만 메모리에 둔다.
fn engine() -> GitOperationService {
    let nonce = AtomicU32::new(0);
    GitOperationService::new(
        Arc::new(GitHistoryAdapter),
        Arc::new(GitWorkspaceAdapter),
        Arc::new(MemoryJournal::default()),
        Arc::new(move || (1_795_000_000, nonce.fetch_add(1, Ordering::SeqCst))),
        Arc::default(),
    )
}

fn run_operation(
    engine: &GitOperationService,
    repository: &str,
    operation: GitOperation,
) -> GitOperationRecord {
    let plan = engine.plan(repository, operation.clone()).unwrap();
    assert!(
        plan.blockers.is_empty(),
        "{operation:?}: {:?}",
        plan.blockers
    );
    match engine
        .execute(repository, operation, plan.expectation)
        .unwrap()
    {
        GitOperationOutcome::Done { record } => record,
        other => panic!("the operation did not finish: {other:?}"),
    }
}

fn undo(engine: &GitOperationService, repository: &str, record: &GitOperationRecord) {
    match engine.undo(repository, &record.op_id).unwrap() {
        GitOperationOutcome::Done { .. } => {}
        other => panic!("the undo did not finish: {other:?}"),
    }
}

/// plan 27 §4 의 흐름 가운데 G3 의 부분이다: 현재 브랜치를 옛 커밋으로 reset 하고, 다른 브랜치를 옮기고, 브랜치와
/// 커밋을 체크아웃한다. 작업마다 되돌리면 ref 는 작업 전과 같다 (§10 의 불변 조건).
#[test]
fn each_operation_changes_the_repository_and_its_undo_restores_every_ref() {
    let directory = temp_repository("scenario");
    let repository = directory.display().to_string();
    let base = commit_file(&directory, "a.txt", "one\n", "base");
    git(&directory, &["branch", "topic"]);
    let second = commit_file(&directory, "a.txt", "two\n", "second");
    let third = commit_file(&directory, "b.txt", "new\n", "third");
    let engine = engine();
    let start = refs_of(&directory);

    // 1. 현재 브랜치를 `base` 로 reset 한다. 작업 트리도 그 커밋이 된다.
    let plan = engine
        .plan(
            &repository,
            GitOperation::Reset {
                target: base.clone(),
            },
        )
        .unwrap();
    let impact = plan.impact.expect("a reset has an impact");
    // `second` 와 `third` 가 빠진다. 다른 브랜치가 닿지 못한다.
    assert_eq!(
        (impact.dropped, impact.unreachable, impact.gained),
        (2, 2, 0)
    );
    let reset = run_operation(
        &engine,
        &repository,
        GitOperation::Reset {
            target: base.clone(),
        },
    );
    assert_eq!(git_text(&directory, &["rev-parse", "main"]), base);
    // 줄 끝은 이 기기의 git 설정을 따른다 (Windows 의 `core.autocrlf=true` 는 CRLF 로 쓴다).
    assert_eq!(
        std::fs::read_to_string(directory.join("a.txt"))
            .unwrap()
            .replace("\r\n", "\n"),
        "one\n"
    );
    assert!(!directory.join("b.txt").exists());
    // 백업 ref 가 빠진 커밋을 지킨다.
    let backup = reset.backup_ref.clone().expect("a reset writes a backup");
    assert_eq!(git_text(&directory, &["rev-parse", &backup]), third);
    undo(&engine, &repository, &reset);
    assert_eq!(refs_of(&directory), start);
    assert!(directory.join("b.txt").exists());

    // 2. 체크아웃하지 않은 브랜치를 옮긴다. 작업 트리는 그대로이다.
    let moved = run_operation(
        &engine,
        &repository,
        GitOperation::MoveBranch {
            branch: "topic".to_string(),
            target: second.clone(),
        },
    );
    assert_eq!(git_text(&directory, &["rev-parse", "topic"]), second);
    assert_eq!(git_text(&directory, &["branch", "--show-current"]), "main");
    // `base` 는 `second` 의 조상이다. 빠지는 커밋이 없다.
    assert_eq!(
        git_text(
            &directory,
            &["rev-parse", &moved.backup_ref.clone().unwrap()]
        ),
        base
    );
    undo(&engine, &repository, &moved);
    assert_eq!(refs_of(&directory), start);

    // 3. 브랜치를 체크아웃하고, 4. 커밋을 브랜치 없이 체크아웃한다.
    let to_topic = run_operation(
        &engine,
        &repository,
        GitOperation::CheckoutBranch {
            branch: "topic".to_string(),
        },
    );
    assert_eq!(git_text(&directory, &["branch", "--show-current"]), "topic");
    assert_eq!(to_topic.backup_ref, None);
    let detached = run_operation(
        &engine,
        &repository,
        GitOperation::CheckoutCommit {
            target: second.clone(),
        },
    );
    assert_eq!(git_text(&directory, &["branch", "--show-current"]), "");
    assert_eq!(git_text(&directory, &["rev-parse", "HEAD"]), second);
    undo(&engine, &repository, &detached);
    assert_eq!(git_text(&directory, &["branch", "--show-current"]), "topic");
    undo(&engine, &repository, &to_topic);
    assert_eq!(refs_of(&directory), start);

    // 일지에는 작업 4 개와 되돌리기 4 개가 있다. 새 것이 앞이다.
    let listed = engine.operations(&repository).unwrap();
    assert_eq!(listed.len(), 8);
    assert!(listed[0].undo_of.is_some());
    let _ = std::fs::remove_dir_all(&directory);
}

#[path = "workspace/group_tests.rs"]
mod group_tests;

#[path = "workspace/sequence_tests.rs"]
mod sequence_tests;

#[path = "workspace/paused_tests.rs"]
mod paused_tests;

#[test]
fn a_reset_stops_before_it_overwrites_an_untracked_file() {
    let directory = temp_repository("untracked");
    let repository = directory.display().to_string();
    let base = commit_file(&directory, "a.txt", "one\n", "base");
    // `base` 에는 `b.txt` 가 있었고, 다음 커밋이 지웠다. 사용자가 같은 이름의 파일을 새로 만들었다.
    let with_file = commit_file(&directory, "b.txt", "tracked\n", "add b");
    git(&directory, &["rm", "--quiet", "b.txt"]);
    git(&directory, &["commit", "--quiet", "-m", "remove b"]);
    std::fs::write(directory.join("b.txt"), "mine\n").unwrap();
    let engine = engine();
    let reset = GitOperation::Reset {
        target: with_file.clone(),
    };
    let plan = engine.plan(&repository, reset.clone()).unwrap();
    assert!(plan.blockers.is_empty());

    let outcome = engine
        .execute(&repository, reset, plan.expectation)
        .unwrap();

    let GitOperationOutcome::Failed { record } = outcome else {
        panic!("the reset must stop");
    };
    assert!(record.error.is_some());
    assert_eq!(
        std::fs::read_to_string(directory.join("b.txt")).unwrap(),
        "mine\n"
    );
    assert_ne!(git_text(&directory, &["rev-parse", "main"]), with_file);
    assert_ne!(git_text(&directory, &["rev-parse", "main"]), base);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_branch_checked_out_in_another_worktree_is_not_moved_or_checked_out() {
    let directory = temp_repository("held");
    let linked = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-git-workspace-held-linked-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&linked);
    let repository = directory.display().to_string();
    let base = commit_file(&directory, "a.txt", "one\n", "base");
    commit_file(&directory, "a.txt", "two\n", "second");
    git(
        &directory,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "topic",
            &linked.display().to_string(),
        ],
    );
    let engine = engine();

    for operation in [
        GitOperation::MoveBranch {
            branch: "topic".to_string(),
            target: base.clone(),
        },
        GitOperation::CheckoutBranch {
            branch: "topic".to_string(),
        },
    ] {
        let plan = engine.plan(&repository, operation).unwrap();
        assert_eq!(
            plan.blockers[0].code,
            GitOperationBlockerCode::BranchCheckedOut
        );
    }
    let _ = std::fs::remove_dir_all(&linked);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_branch_that_a_rebase_or_a_bisect_uses_is_reported() {
    let directory = temp_repository("in-use");
    let repository = directory.display().to_string();
    commit_file(&directory, "a.txt", "one\n", "base");
    let adapter = GitWorkspaceAdapter;
    assert!(
        adapter
            .read_condition(&repository)
            .unwrap()
            .busy_branches
            .is_empty()
    );

    // git 이 rebase 와 bisect 를 시작할 때 쓰는 파일이다. 대표 작업 트리와 연결된 작업 트리에 하나씩 둔다.
    let git_dir = directory.join(".git");
    std::fs::create_dir_all(git_dir.join("worktrees/linked/rebase-merge")).unwrap();
    std::fs::write(
        git_dir.join("worktrees/linked/rebase-merge/head-name"),
        "refs/heads/feature/a\n",
    )
    .unwrap();
    std::fs::write(git_dir.join("BISECT_START"), "topic\n").unwrap();

    let condition = adapter.read_condition(&repository).unwrap();

    assert_eq!(condition.busy_branches, ["feature/a", "topic"]);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_filter_in_the_repository_configuration_refuses_the_repository() {
    let directory = temp_repository("filter-config");
    let repository = directory.display().to_string();
    let base = commit_file(&directory, "a.txt", "one\n", "base");
    commit_file(&directory, "a.txt", "two\n", "second");
    let engine = engine();
    let reset = GitOperation::Reset {
        target: base.clone(),
    };
    assert!(
        engine
            .plan(&repository, reset.clone())
            .unwrap()
            .blockers
            .is_empty()
    );
    // 저장소 설정의 필터이다. 어느 파일도 이 필터를 쓰지 않는다. 그래도 체크아웃이 실행할 수 있는 프로그램이다.
    git(
        &directory,
        &["config", "filter.demo.smudge", "missing-filter-program"],
    );

    let plan = engine.plan(&repository, reset.clone()).unwrap();

    assert_eq!(
        plan.blockers[0].code,
        GitOperationBlockerCode::ExecutionConfig
    );
    assert!(plan.blockers[0].message.contains("filter.demo.smudge"));
    // 계획 뒤에 설정이 생긴 경우이다. 고치는 명령이 실행 직전에 다시 본다.
    assert!(GitWorkspaceAdapter.reset_to(&repository, &base).is_err());
    assert_ne!(git_text(&directory, &["rev-parse", "main"]), base);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn the_line_endings_of_the_user_reach_the_commands_in_the_order_of_git() {
    let system = "core.autocrlf true\ncore.eol CRLF\ncore.fscache true\n";
    assert_eq!(
        line_ending_overrides(&["", "", system]),
        ["-c", "core.autocrlf=true", "-c", "core.eol=crlf"]
    );
    // 사용자 설정이 시스템 설정을 이긴다.
    assert_eq!(
        line_ending_overrides(&["", "core.autocrlf false\n", system]),
        ["-c", "core.autocrlf=false", "-c", "core.eol=crlf"]
    );
    // 저장소 설정에 있는 키는 주지 않는다. 이 git 이 그 값을 이미 읽고, `-c` 는 그 값을 덮는다.
    assert_eq!(
        line_ending_overrides(&["core.autocrlf input\n", "core.autocrlf false\n", system]),
        ["-c", "core.eol=crlf"]
    );
    // 목록에 없는 값은 명령에 주지 않는다.
    assert!(line_ending_overrides(&["", "core.autocrlf yes; rm -rf\n", ""]).is_empty());
    assert!(line_ending_overrides(&["", "", ""]).is_empty());
}

#[test]
fn every_command_that_touches_files_stays_out_of_submodules() {
    let directory = temp_repository("overrides");
    commit_file(&directory, "a.txt", "one\n", "base");

    let overrides = CommandOverrides::read(&directory).unwrap();
    let args = overrides.with(&["switch", "--detach"]);

    // `-c` 는 하위 명령 앞에 온다. 줄 끝 값은 이 기기의 설정에 따라 그 사이에 올 수 있다.
    assert_eq!(args[..2], ["-c", "submodule.recurse=false"]);
    assert_eq!(args[args.len() - 2..], ["switch", "--detach"]);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_repository_whose_files_use_a_filter_is_refused() {
    let directory = temp_repository("filter-attribute");
    let repository = directory.display().to_string();
    let base = commit_file(&directory, "a.txt", "one\n", "base");
    let engine = engine();
    let reset = GitOperation::Reset {
        target: base.clone(),
    };

    // 필터를 끄는 줄은 필터를 쓰는 것이 아니다. 이름이 다른 파일의 글도 아니다.
    commit_file(&directory, ".gitattributes", "*.txt -filter\n", "unset");
    let plain = commit_file(&directory, "notes.gitattributes", "filter=demo\n", "other");
    assert!(
        engine
            .plan(&repository, reset.clone())
            .unwrap()
            .blockers
            .is_empty()
    );

    std::fs::create_dir_all(directory.join("assets")).unwrap();
    let filtered = commit_file(
        &directory,
        "assets/.gitattributes",
        "*.bin filter=demo\n",
        "use a filter",
    );
    let plan = engine.plan(&repository, reset.clone()).unwrap();
    assert_eq!(
        plan.blockers[0].code,
        GitOperationBlockerCode::ExecutionConfig
    );
    assert!(plan.blockers[0].message.contains("assets/.gitattributes"));
    let outcome = engine
        .execute(&repository, reset, plan.expectation)
        .unwrap();
    assert!(matches!(outcome, GitOperationOutcome::Stale { .. }));

    // 지금 커밋은 필터를 쓰지 않고 대상 커밋만 쓴다. 계획은 알지 못한다. 고치는 명령이 실행 직전에 멈춘다.
    git(&directory, &["checkout", "--quiet", "--detach", &plain]);
    let checkout = GitOperation::CheckoutCommit { target: filtered };
    let plan = engine.plan(&repository, checkout.clone()).unwrap();
    assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
    let outcome = engine
        .execute(&repository, checkout, plan.expectation)
        .unwrap();
    let GitOperationOutcome::Failed { record } = outcome else {
        panic!("the checkout must stop");
    };
    assert!(record.error.unwrap().contains("content filter"));
    assert_eq!(git_text(&directory, &["rev-parse", "HEAD"]), plain);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_configuration_that_runs_another_program_refuses_the_repository() {
    let directory = temp_repository("unsafe-config");
    let repository = directory.display().to_string();
    commit_file(&directory, "a.txt", "one\n", "base");
    git(&directory, &["config", "diff.external", "some-program"]);

    let condition = GitWorkspaceAdapter.read_condition(&repository).unwrap();

    assert!(condition.refusal.unwrap().contains("diff.external"));
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_backup_ref_is_created_once_and_a_relative_folder_is_refused() {
    let directory = temp_repository("backup-ref");
    let repository = directory.display().to_string();
    let base = commit_file(&directory, "a.txt", "one\n", "base");
    let adapter = GitWorkspaceAdapter;
    let name = "refs/akra/backup/main/op-1";

    adapter.create_ref(&repository, name, &base).unwrap();
    assert_eq!(git_text(&directory, &["rev-parse", name]), base);
    // 같은 이름을 다시 쓰지 않는다. 앞의 백업을 덮어쓰면 그 커밋을 잃는다.
    assert!(adapter.create_ref(&repository, name, &base).is_err());
    adapter.delete_ref(&repository, name).unwrap();
    assert!(
        adapter
            .resolve_commit(&repository, &base)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        adapter
            .resolve_commit(&repository, &"0".repeat(40))
            .unwrap(),
        None
    );

    assert!(adapter.create_ref(&repository, "heads/x", &base).is_err());
    assert!(adapter.read_condition("relative/path").is_err());
    assert!(adapter.reset_to("relative/path", &base).is_err());
    let _ = std::fs::remove_dir_all(&directory);
}
