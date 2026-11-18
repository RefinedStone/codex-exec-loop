//! 저장소의 ref 와 커밋 이력을 읽는다 (plan 27 G1, G2). 읽기 명령만 쓴다. 저장소를 고치지 않는다.
//!
//! `git log` 는 커밋 머리만 읽는다. diff 를 만들지 않으므로 저장소 설정의 필터, textconv, diff 드라이버를 실행하지
//! 않는다. 서명 확인은 저장소 설정의 프로그램을 실행할 수 있어서 명령에서 끈다.
//!
//! 커밋 상세의 `git show` 는 diff 를 만든다. 커밋과 그 부모의 트리를 비교하므로 작업 트리의 필터(clean, smudge)는
//! 돌지 않는다. 바깥 diff 프로그램과 textconv 는 명령에서 끈다 (`--no-ext-diff`, `--no-textconv`).

use std::collections::HashSet;
use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result, bail};

use crate::application::port::outbound::git_history_port::{GitHistoryPort, GitHistoryRefs};
use crate::domain::git_graph::{
    GIT_BACKUP_REF_PREFIX, GitCommitDetail, GitGraphCommit, GitGraphRef, GitGraphRefKind,
    GitGraphWorktree,
};
use crate::{git_subprocess, subprocess};

/// 커밋 하나의 필드를 나누는 글자이다. 커밋 사이는 NUL 이다 (`-z`).
const FIELD_SEPARATOR: char = '\u{1f}';
/// 글자를 고를 수 없는 필드(sha, 부모, 시각)가 앞이다. 작성자 이름이나 제목에 나눔 글자가 있어도 앞 필드는 밀리지
/// 않는다.
const LOG_FORMAT: &str = "--format=%H%x1f%P%x1f%at%x1f%an%x1f%s";
const REF_FORMAT: &str = "--format=%(refname)%09%(objectname)%09%(*objectname)%09%(HEAD)";
/// 백업 ref 를 고르는 인자이다. `*` 는 `/` 도 맞춘다. 그래서 `<branch>/<op-id>` 의 모든 깊이를 고른다.
const BACKUP_GLOB: &str = "--glob=refs/akra/backup/*";
/// 백업에서만 닿는 커밋을 이 수까지 찾는다.
// shortcut: 이 수를 넘는 백업 커밋은 흐리게 그리지 않는다, 백업 정리(G3)가 수를 묶지 못하면 쪽마다 범위를 좁힌다.
const BACKUP_ONLY_COMMIT_LIMIT: &str = "--max-count=10000";
/// 상세의 필드를 나누는 글자는 NUL 이다. 이름, 메일, 본문에 NUL 은 올 수 없다.
const DETAIL_FORMAT: &str = "--format=%H%x00%P%x00%at%x00%ct%x00%an%x00%ae%x00%cn%x00%B";
const DETAIL_FIELD_COUNT: usize = 8;
/// 커밋 하나의 변경을 이 크기까지 읽는다. Desktop 의 `변경` 패널이 커밋에 쓰는 한도와 같다.
const COMMIT_DIFF_LIMIT_BYTES: usize = 2 << 20;
const COMMIT_DIFF_STDERR_LIMIT_BYTES: usize = 64 * 1024;

pub(crate) struct GitHistoryAdapter;

impl GitHistoryPort for GitHistoryAdapter {
    fn read_refs(&self, repository_dir: &str) -> Result<Option<GitHistoryRefs>> {
        let repository = Path::new(repository_dir);
        if !repository.is_absolute() {
            bail!("the repository folder must be an absolute path");
        }
        if git_output(repository, &["rev-parse", "--git-dir"])?.is_err() {
            return Ok(None);
        }
        // 커밋이 없는 저장소에서는 실패한다. 오류가 아니라 "HEAD 없음"이다.
        let head = git_output(repository, &["rev-parse", "--verify", "--quiet", "HEAD"])?
            .ok()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty());
        let listing = required(
            git_output(
                repository,
                &[
                    "for-each-ref",
                    REF_FORMAT,
                    "refs/heads",
                    "refs/remotes",
                    "refs/tags",
                    GIT_BACKUP_REF_PREFIX.trim_end_matches('/'),
                ],
            )?,
            "git for-each-ref",
        )?;
        let refs = listing.lines().filter_map(parse_ref_line).collect();
        let worktrees = parse_worktrees(&required(
            git_output(repository, &["worktree", "list", "--porcelain"])?,
            "git worktree list",
        )?);
        Ok(Some(GitHistoryRefs {
            head,
            refs,
            worktrees,
        }))
    }

    fn read_commits(
        &self,
        repository_dir: &str,
        include_head: bool,
        skip: usize,
        limit: usize,
    ) -> Result<Vec<GitGraphCommit>> {
        let skip = format!("--skip={skip}");
        let limit = format!("--max-count={limit}");
        let mut args = vec![
            "log",
            "-z",
            LOG_FORMAT,
            "--date-order",
            "--no-show-signature",
            "--no-color",
            // 저장소 설정의 출력 인코딩을 따르지 않는다. 다른 인코딩은 나눔 글자를 깨뜨린다.
            "--encoding=UTF-8",
            skip.as_str(),
            limit.as_str(),
            "--branches",
            "--remotes",
            "--tags",
            BACKUP_GLOB,
        ];
        if include_head {
            args.push("HEAD");
        }
        args.push("--");
        let output = required(git_output(Path::new(repository_dir), &args)?, "git log")?;
        output
            .split('\0')
            .filter(|record| !record.is_empty())
            .map(parse_commit_record)
            .collect()
    }

    fn read_backup_only_commits(
        &self,
        repository_dir: &str,
        include_head: bool,
    ) -> Result<HashSet<String>> {
        // `--not` 뒤의 모든 ref 는 빼는 쪽이다.
        let mut args = vec![
            "rev-list",
            BACKUP_ONLY_COMMIT_LIMIT,
            BACKUP_GLOB,
            "--not",
            "--branches",
            "--remotes",
            "--tags",
        ];
        if include_head {
            args.push("HEAD");
        }
        args.push("--");
        let output = required(
            git_output(Path::new(repository_dir), &args)?,
            "git rev-list",
        )?;
        Ok(output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect())
    }

    fn read_commit_detail(
        &self,
        repository_dir: &str,
        commit: &str,
    ) -> Result<Option<GitCommitDetail>> {
        let repository = Path::new(repository_dir);
        if !repository.is_absolute() {
            bail!("the repository folder must be an absolute path");
        }
        // 짧은 sha 를 전체 sha 로 바꾼다. 그 커밋이 없으면 (또는 커밋이 아니면) 실패한다.
        let revision = format!("{commit}^{{commit}}");
        let Ok(resolved) =
            git_output(repository, &["rev-parse", "--verify", "--quiet", &revision])?
        else {
            return Ok(None);
        };
        let id = resolved.trim();
        let header = required(
            git_output(
                repository,
                &[
                    "show",
                    "--no-patch",
                    DETAIL_FORMAT,
                    "--no-show-signature",
                    "--no-color",
                    "--encoding=UTF-8",
                    id,
                    "--",
                ],
            )?,
            "git show",
        )?;
        let mut detail = read_detail_header(&header)
            .context("git show printed a commit header in an unknown format")?;
        match read_commit_diff(repository, id)? {
            Some(diff) => detail.diff = diff,
            None => detail.diff_too_large = true,
        }
        Ok(Some(detail))
    }
}

/// 커밋의 변경을 읽는다. 한도를 넘으면 `None` 이다. 접두사를 명령에서 정한다: 저장소 설정(`diff.noprefix`,
/// `diff.mnemonicPrefix`)이 바꾸면 화면이 파일 이름을 읽지 못한다.
fn read_commit_diff(repository: &Path, id: &str) -> Result<Option<String>> {
    let mut command = git_subprocess::command([
        "show",
        "--format=",
        "--first-parent",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--no-show-signature",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        id,
        "--",
    ]);
    command
        .current_dir(repository)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = subprocess::spawn(&mut command)
        .with_context(|| format!("failed to run git in `{}`", repository.display()))?;
    let output = match subprocess::wait_with_output_timeout_and_limits(
        child,
        "git commit diff",
        subprocess::configured_subprocess_timeout(),
        COMMIT_DIFF_LIMIT_BYTES,
        COMMIT_DIFF_STDERR_LIMIT_BYTES,
    ) {
        Ok(output) => output,
        // 출력이 한도를 넘었다. 실행기는 이 종류로만 알린다.
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => return Ok(None),
        Err(error) => return Err(error).context("failed to read the commit diff"),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "git show failed: {}",
            stderr.lines().next().unwrap_or("").trim()
        );
    }
    Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
}

fn read_detail_header(header: &str) -> Option<GitCommitDetail> {
    let mut fields = header.splitn(DETAIL_FIELD_COUNT, '\0');
    let id = fields.next().filter(|id| !id.is_empty())?;
    let parents = fields.next()?;
    let author_time = fields.next()?.parse().ok()?;
    let committer_time = fields.next()?.parse().ok()?;
    let author_name = fields.next()?;
    let author_email = fields.next()?;
    let committer_name = fields.next()?;
    let message = fields.next()?;
    Some(GitCommitDetail {
        id: id.to_string(),
        parents: parents.split_whitespace().map(str::to_string).collect(),
        author_name: author_name.to_string(),
        author_email: author_email.to_string(),
        author_time,
        committer_name: committer_name.to_string(),
        committer_time,
        message: message.trim_end().to_string(),
        diff: String::new(),
        diff_too_large: false,
    })
}

/// `git worktree list --porcelain` 을 읽는다. 작업 트리마다 `worktree <폴더>` 로 시작하는 줄 묶음이다.
fn parse_worktrees(listing: &str) -> Vec<GitGraphWorktree> {
    let mut worktrees: Vec<GitGraphWorktree> = Vec::new();
    for line in listing.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            worktrees.push(GitGraphWorktree {
                path: path.to_string(),
                head: None,
                branch: None,
            });
        } else if let Some(worktree) = worktrees.last_mut() {
            if let Some(head) = line.strip_prefix("HEAD ") {
                worktree.head = Some(head.to_string());
            } else if let Some(branch) = line.strip_prefix("branch refs/heads/") {
                worktree.branch = Some(branch.to_string());
            }
        }
    }
    worktrees
}

/// `refname<TAB>objectname<TAB>*objectname<TAB>HEAD` 한 줄을 읽는다. `origin/HEAD` 같은 가리킴 ref 는 뺀다.
fn parse_ref_line(line: &str) -> Option<GitGraphRef> {
    let mut fields = line.split('\t');
    let full_name = fields.next()?;
    let object = fields.next()?;
    let peeled = fields.next()?;
    let head = fields.next()? == "*";
    let (kind, name) = if let Some(name) = full_name.strip_prefix("refs/heads/") {
        (GitGraphRefKind::LocalBranch, name)
    } else if let Some(name) = full_name.strip_prefix("refs/remotes/") {
        if name.ends_with("/HEAD") {
            return None;
        }
        (GitGraphRefKind::RemoteBranch, name)
    } else if let Some(name) = full_name.strip_prefix(GIT_BACKUP_REF_PREFIX) {
        (GitGraphRefKind::Backup, name)
    } else {
        (GitGraphRefKind::Tag, full_name.strip_prefix("refs/tags/")?)
    };
    // 주석 태그는 태그 객체가 아니라 태그가 가리키는 커밋을 쓴다.
    let target = if peeled.is_empty() { object } else { peeled };
    Some(GitGraphRef {
        name: name.to_string(),
        kind,
        target: target.to_string(),
        head,
    })
}

/// 커밋 하나를 읽는다. 읽지 못한 커밋을 건너뛰지 않는다: 줄 하나가 빠지면 그 자식의 선이 끊기고 쪽의 수가
/// 어긋난다. 그래서 오류로 올린다.
fn parse_commit_record(record: &str) -> Result<GitGraphCommit> {
    read_commit_record(record).context("git log printed a commit record in an unknown format")
}

fn read_commit_record(record: &str) -> Option<GitGraphCommit> {
    let mut fields = record.splitn(5, FIELD_SEPARATOR);
    let id = fields.next().filter(|id| !id.is_empty())?;
    let parents = fields.next()?;
    let author_time = fields.next()?.parse().ok()?;
    let author_name = fields.next()?;
    let subject = fields.next()?;
    Some(GitGraphCommit {
        id: id.to_string(),
        parents: parents.split_whitespace().map(str::to_string).collect(),
        author_name: author_name.to_string(),
        author_time,
        subject: subject.to_string(),
    })
}

/// 명령이 성공하면 `Ok(출력)` 이고, 실패하면 `Err(git 이 쓴 오류의 첫 줄)` 이다. 실행하지 못하면 바깥 오류이다.
fn git_output(repository: &Path, args: &[&str]) -> Result<std::result::Result<String, String>> {
    let mut command = git_subprocess::command(args.iter().copied());
    command
        .current_dir(repository)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = subprocess::command_output(&mut command, "git history")
        .with_context(|| format!("failed to run git in `{}`", repository.display()))?;
    if output.status.success() {
        return Ok(Ok(String::from_utf8_lossy(&output.stdout).into_owned()));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(Err(stderr.lines().next().unwrap_or("").trim().to_string()))
}

/// 실패하면 안 되는 명령의 출력이다. 실패하면 git 의 오류 글을 붙여 올린다.
fn required(output: std::result::Result<String, String>, label: &str) -> Result<String> {
    output.map_err(|reason| anyhow::anyhow!("{label} failed: {reason}"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
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

    fn commit(directory: &Path, subject: &str) {
        git(
            directory,
            &["commit", "--quiet", "--allow-empty", "-m", subject],
        );
    }

    fn temp_repository(label: &str) -> PathBuf {
        let directory = crate::test_utils::platform_safe_temp_dir()
            .join(format!("akra-git-history-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn initialize(directory: &Path) {
        git(directory, &["init", "--quiet", "--initial-branch", "main"]);
        git(directory, &["config", "user.name", "Akra Test"]);
        git(
            directory,
            &["config", "user.email", "akra-test@example.com"],
        );
    }

    #[test]
    fn reads_refs_and_commits_of_a_branched_repository() {
        let directory = temp_repository("branched");
        let repository = directory.display().to_string();
        let adapter = GitHistoryAdapter;
        initialize(&directory);

        let empty = adapter
            .read_refs(&repository)
            .unwrap()
            .expect("a repository");
        assert_eq!(empty.head, None);
        assert!(empty.refs.is_empty());

        commit(&directory, "base");
        git(&directory, &["checkout", "--quiet", "-b", "topic"]);
        commit(&directory, "topic work | with a pipe");
        git(&directory, &["tag", "-a", "v1", "-m", "annotated"]);
        git(&directory, &["checkout", "--quiet", "main"]);
        commit(&directory, "main work");

        let refs = adapter.read_refs(&repository).unwrap().unwrap();
        let commits = adapter.read_commits(&repository, true, 0, 10).unwrap();
        assert_eq!(commits.len(), 3);
        let by_subject = |subject: &str| {
            commits
                .iter()
                .find(|commit| commit.subject == subject)
                .unwrap_or_else(|| panic!("commit `{subject}` is missing"))
        };
        let base = by_subject("base");
        let topic = by_subject("topic work | with a pipe");
        let main = by_subject("main work");
        assert_eq!(base.parents, Vec::<String>::new());
        assert_eq!(topic.parents, vec![base.id.clone()]);
        assert_eq!(main.parents, vec![base.id.clone()]);
        assert_eq!(main.author_name, "Akra Test");
        assert!(main.author_time > 0);
        // 부모는 자식보다 뒤에 나온다.
        assert_eq!(commits.last().unwrap().id, base.id);

        assert_eq!(refs.head.as_deref(), Some(main.id.as_str()));
        let names: Vec<(&str, GitGraphRefKind, &str, bool)> = refs
            .refs
            .iter()
            .map(|reference| {
                (
                    reference.name.as_str(),
                    reference.kind,
                    reference.target.as_str(),
                    reference.head,
                )
            })
            .collect();
        assert_eq!(
            names,
            [
                ("main", GitGraphRefKind::LocalBranch, main.id.as_str(), true),
                (
                    "topic",
                    GitGraphRefKind::LocalBranch,
                    topic.id.as_str(),
                    false
                ),
                // 주석 태그의 대상은 태그 객체가 아니라 커밋이다.
                ("v1", GitGraphRefKind::Tag, topic.id.as_str(), false),
            ]
        );

        let second_page = adapter.read_commits(&repository, true, 2, 10).unwrap();
        assert_eq!(second_page.len(), 1);
        assert_eq!(second_page[0].id, base.id);
        let _ = std::fs::remove_dir_all(&directory);
    }

    fn head_sha(directory: &Path) -> String {
        let output = git_command()
            .args(["rev-parse", "HEAD"])
            .current_dir(directory)
            .output()
            .expect("git should run");
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[test]
    fn backup_refs_are_read_and_their_own_commits_are_backup_only() {
        let directory = temp_repository("backup");
        let repository = directory.display().to_string();
        let adapter = GitHistoryAdapter;
        initialize(&directory);
        commit(&directory, "base");
        let base = head_sha(&directory);
        commit(&directory, "dropped work");
        let dropped = head_sha(&directory);
        // G3 의 reset 이 할 일이다: 백업 ref 를 쓰고 브랜치를 뒤로 옮긴다.
        git(
            &directory,
            &["update-ref", "refs/akra/backup/main/op-1", &dropped],
        );
        git(&directory, &["reset", "--quiet", "--hard", &base]);

        let refs = adapter.read_refs(&repository).unwrap().unwrap();
        let backup = refs
            .refs
            .iter()
            .find(|reference| reference.kind == GitGraphRefKind::Backup)
            .expect("the backup ref is listed");
        assert_eq!(backup.name, "main/op-1");
        assert_eq!(backup.target, dropped);

        // 브랜치에서 빠진 커밋이 그래프에 남는다.
        let commits = adapter.read_commits(&repository, true, 0, 10).unwrap();
        let ids: Vec<&str> = commits.iter().map(|commit| commit.id.as_str()).collect();
        assert_eq!(ids, [dropped.as_str(), base.as_str()]);
        assert_eq!(
            adapter.read_backup_only_commits(&repository, true).unwrap(),
            HashSet::from([dropped.clone()])
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn worktrees_are_read_with_their_branch_and_head() {
        let directory = temp_repository("worktrees");
        let linked = temp_repository("worktrees-linked");
        let _ = std::fs::remove_dir_all(&linked);
        initialize(&directory);
        commit(&directory, "base");
        let base = head_sha(&directory);
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

        let refs = GitHistoryAdapter
            .read_refs(&directory.display().to_string())
            .unwrap()
            .unwrap();

        assert_eq!(refs.worktrees.len(), 2);
        assert_eq!(refs.worktrees[0].branch.as_deref(), Some("main"));
        assert_eq!(refs.worktrees[0].head.as_deref(), Some(base.as_str()));
        assert_eq!(refs.worktrees[1].branch.as_deref(), Some("topic"));
        assert!(
            refs.worktrees[1]
                .path
                .replace('\\', "/")
                .ends_with(&format!("worktrees-linked-{}", std::process::id()))
        );
        let _ = std::fs::remove_dir_all(&linked);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn worktree_listing_reads_detached_and_bare_entries() {
        let listing = "worktree /repo\nbare\n\nworktree /repo-a\nHEAD abc\nbranch refs/heads/feature/a\n\nworktree /repo-b\nHEAD def\ndetached\n";
        let worktrees = parse_worktrees(listing);

        assert_eq!(worktrees.len(), 3);
        assert_eq!(
            (worktrees[0].head.as_deref(), worktrees[0].branch.as_deref()),
            (None, None)
        );
        assert_eq!(worktrees[1].branch.as_deref(), Some("feature/a"));
        assert_eq!(
            (worktrees[2].head.as_deref(), worktrees[2].branch.as_deref()),
            (Some("def"), None)
        );
    }

    #[test]
    fn commit_detail_carries_the_message_the_people_and_the_diff() {
        let directory = temp_repository("detail");
        let repository = directory.display().to_string();
        let adapter = GitHistoryAdapter;
        initialize(&directory);
        std::fs::write(directory.join("note.txt"), "one\n").unwrap();
        git(&directory, &["add", "note.txt"]);
        git(&directory, &["commit", "--quiet", "-m", "first"]);
        let first = head_sha(&directory);
        std::fs::write(directory.join("note.txt"), "one\ntwo\n").unwrap();
        // 저장소 설정이 접두사를 바꿔도 화면이 읽는 `a/`, `b/` 가 나온다.
        git(&directory, &["config", "diff.noprefix", "true"]);
        git(
            &directory,
            &[
                "commit",
                "--quiet",
                "-am",
                "second\n\nbody line | with a pipe",
            ],
        );
        let second = head_sha(&directory);

        let detail = adapter
            .read_commit_detail(&repository, &second[..10])
            .unwrap()
            .expect("the commit exists");

        assert_eq!(detail.id, second);
        assert_eq!(detail.parents, vec![first]);
        assert_eq!(detail.author_name, "Akra Test");
        assert_eq!(detail.author_email, "akra-test@example.com");
        assert_eq!(detail.committer_name, "Akra Test");
        assert!(detail.author_time > 0 && detail.committer_time > 0);
        assert_eq!(detail.message, "second\n\nbody line | with a pipe");
        assert!(detail.diff.contains("diff --git a/note.txt b/note.txt"));
        assert!(detail.diff.contains("+two"));
        assert!(!detail.diff_too_large);

        assert_eq!(
            adapter
                .read_commit_detail(&repository, "0000000000000000000000000000000000000000")
                .unwrap(),
            None
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_commit_diff_over_the_limit_is_reported_and_not_read() {
        let directory = temp_repository("large-diff");
        let repository = directory.display().to_string();
        initialize(&directory);
        let line = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n";
        std::fs::write(
            directory.join("large.txt"),
            line.repeat(COMMIT_DIFF_LIMIT_BYTES / line.len() + 1),
        )
        .unwrap();
        git(&directory, &["add", "large.txt"]);
        git(&directory, &["commit", "--quiet", "-m", "large"]);

        let detail = GitHistoryAdapter
            .read_commit_detail(&repository, &head_sha(&directory))
            .unwrap()
            .unwrap();

        assert_eq!(detail.message, "large");
        assert!(detail.diff_too_large);
        assert!(detail.diff.is_empty());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_plain_folder_is_not_a_repository_and_a_relative_path_is_refused() {
        let directory = temp_repository("plain");
        let adapter = GitHistoryAdapter;

        assert_eq!(
            adapter.read_refs(&directory.display().to_string()).unwrap(),
            None
        );
        assert!(adapter.read_refs("relative/path").is_err());
        assert!(
            adapter
                .read_commit_detail("relative/path", "abcdef1")
                .is_err()
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn separator_bytes_in_the_author_name_do_not_drop_the_commit() {
        let directory = temp_repository("separator");
        let repository = directory.display().to_string();
        initialize(&directory);
        git(&directory, &["config", "user.name", "Ev\u{1f}il"]);
        commit(&directory, "first");
        git(&directory, &["config", "i18n.logOutputEncoding", "UTF-16"]);
        commit(&directory, "second");

        let commits = GitHistoryAdapter
            .read_commits(&repository, true, 0, 10)
            .unwrap();

        // 두 커밋이 모두 나온다. 나눔 글자는 이름과 제목 사이만 흐트린다.
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[1].parents, Vec::<String>::new());
        assert_eq!(commits[0].parents, vec![commits[1].id.clone()]);
        assert!(commits.iter().all(|commit| commit.author_time > 0));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_record_in_an_unknown_format_is_an_error() {
        assert!(parse_commit_record("abc\u{1f}\u{1f}not-a-time\u{1f}author\u{1f}subject").is_err());
        assert!(parse_commit_record("abc\u{1f}\u{1f}12\u{1f}author\u{1f}subject").is_ok());
    }

    #[test]
    fn a_broken_ref_reports_the_reason_git_gave() {
        let directory = temp_repository("broken-ref");
        initialize(&directory);
        commit(&directory, "first");
        std::fs::write(
            directory.join(".git/refs/heads/broken"),
            "1111111111111111111111111111111111111111\n",
        )
        .unwrap();

        let error = GitHistoryAdapter
            .read_commits(&directory.display().to_string(), true, 0, 10)
            .unwrap_err();

        assert!(format!("{error:#}").starts_with("git log failed: "));
        assert!(format!("{error:#}").len() > "git log failed: ".len());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn remote_head_pointer_is_not_a_branch_label() {
        assert_eq!(parse_ref_line("refs/remotes/origin/HEAD\tabc\t\t "), None);
        let remote = parse_ref_line("refs/remotes/origin/main\tabc\t\t ").unwrap();
        assert_eq!(remote.name, "origin/main");
        assert_eq!(remote.kind, GitGraphRefKind::RemoteBranch);
        assert!(!remote.head);
        let backup = parse_ref_line("refs/akra/backup/feature/a/op-7\tabc\t\t ").unwrap();
        assert_eq!(backup.name, "feature/a/op-7");
        assert_eq!(backup.kind, GitGraphRefKind::Backup);
    }
}
