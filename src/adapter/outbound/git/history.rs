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
/// 글자를 고를 수 없는 필드(sha, 부모, 시각)가 앞이다. 작성자 이름이나 제목에 나눔 글자가 있어도 앞 필드는 밀리지
/// 않는다.
const LOG_FORMAT: &str = "--format=%H%x1f%P%x1f%at%x1f%an%x1f%s";
const REF_FORMAT: &str = "--format=%(refname)%09%(objectname)%09%(*objectname)%09%(HEAD)";

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
                ],
            )?,
            "git for-each-ref",
        )?;
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
            // 저장소 설정의 출력 인코딩을 따르지 않는다. 다른 인코딩은 나눔 글자를 깨뜨린다.
            "--encoding=UTF-8",
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
        let output = required(git_output(Path::new(repository_dir), &args)?, "git log")?;
        output
            .split('\0')
            .filter(|record| !record.is_empty())
            .map(parse_commit_record)
            .collect()
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
    }
}
