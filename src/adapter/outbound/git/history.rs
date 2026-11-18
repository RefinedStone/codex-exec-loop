//! 저장소의 ref 와 커밋 이력을 읽는다 (plan 27 G1). 읽기 명령만 쓴다. 저장소를 고치지 않는다.
//!
//! `git log` 는 커밋 머리만 읽는다. diff 를 만들지 않으므로 저장소 설정의 필터, textconv, diff 드라이버를 실행하지
//! 않는다. 서명 확인은 저장소 설정의 프로그램을 실행할 수 있어서 명령에서 끈다.

use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result, bail};

use crate::application::port::outbound::git_history_port::{GitHistoryPort, GitHistoryRefs};
use crate::domain::git_graph::{GitGraphCommit, GitGraphRef, GitGraphRefKind};
use crate::{git_subprocess, subprocess};

/// 커밋 하나의 필드를 나누는 글자이다. 커밋 사이는 NUL 이다 (`-z`).
const FIELD_SEPARATOR: char = '\u{1f}';
const LOG_FORMAT: &str = "--format=%H%x1f%P%x1f%an%x1f%at%x1f%s";
const REF_FORMAT: &str = "--format=%(refname)%09%(objectname)%09%(*objectname)%09%(HEAD)";

pub(crate) struct GitHistoryAdapter;

impl GitHistoryPort for GitHistoryAdapter {
    fn read_refs(&self, repository_dir: &str) -> Result<Option<GitHistoryRefs>> {
        let repository = Path::new(repository_dir);
        if !repository.is_absolute() {
            bail!("the repository folder must be an absolute path");
        }
        if git_stdout(repository, &["rev-parse", "--git-dir"])?.is_none() {
            return Ok(None);
        }
        let head = git_stdout(repository, &["rev-parse", "--verify", "--quiet", "HEAD"])?
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty());
        let listing = git_stdout(
            repository,
            &[
                "for-each-ref",
                REF_FORMAT,
                "refs/heads",
                "refs/remotes",
                "refs/tags",
            ],
        )?
        .context("git for-each-ref failed")?;
        let refs = listing.lines().filter_map(parse_ref_line).collect();
        Ok(Some(GitHistoryRefs { head, refs }))
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
            skip.as_str(),
            limit.as_str(),
            "--branches",
            "--remotes",
            "--tags",
        ];
        if include_head {
            args.push("HEAD");
        }
        args.push("--");
        let output = git_stdout(Path::new(repository_dir), &args)?.context("git log failed")?;
        Ok(output.split('\0').filter_map(parse_commit_record).collect())
    }
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

fn parse_commit_record(record: &str) -> Option<GitGraphCommit> {
    let mut fields = record.splitn(5, FIELD_SEPARATOR);
    let id = fields.next().filter(|id| !id.is_empty())?;
    let parents = fields.next()?;
    let author_name = fields.next()?;
    let author_time = fields.next()?.parse().ok()?;
    let subject = fields.next()?;
    Some(GitGraphCommit {
        id: id.to_string(),
        parents: parents.split_whitespace().map(str::to_string).collect(),
        author_name: author_name.to_string(),
        author_time,
        subject: subject.to_string(),
    })
}

/// 명령이 성공하면 출력이다. 실패하면 `None` 이다. 실행하지 못하면 오류이다.
fn git_stdout(repository: &Path, args: &[&str]) -> Result<Option<String>> {
    let mut command = git_subprocess::command(args.iter().copied());
    command
        .current_dir(repository)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = subprocess::command_output(&mut command, "git history")
        .with_context(|| format!("failed to run git in `{}`", repository.display()))?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
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

    #[test]
    fn a_plain_folder_is_not_a_repository_and_a_relative_path_is_refused() {
        let directory = temp_repository("plain");
        let adapter = GitHistoryAdapter;

        assert_eq!(
            adapter.read_refs(&directory.display().to_string()).unwrap(),
            None
        );
        assert!(adapter.read_refs("relative/path").is_err());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn remote_head_pointer_is_not_a_branch_label() {
        assert_eq!(parse_ref_line("refs/remotes/origin/HEAD\tabc\t\t "), None);
        let remote = parse_ref_line("refs/remotes/origin/main\tabc\t\t ").unwrap();
        assert_eq!(remote.name, "origin/main");
        assert_eq!(remote.kind, GitGraphRefKind::RemoteBranch);
        assert!(!remote.head);
    }
}
