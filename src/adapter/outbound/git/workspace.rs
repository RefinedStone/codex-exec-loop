//! 작업 트리의 상태를 읽고 저장소를 고친다 (plan 27 G3). 이 저장소에서 사용자 저장소를 고치는 git 명령은 이 파일에만
//! 있다: `update-ref` (백업 ref, 브랜치 이동), `reset --keep`, `switch`.
//!
//! 이 파일의 git 은 `git_subprocess` 의 git 이다. 그 git 은 저장소 설정만 읽는다. 사용자 설정(`~/.gitconfig`)과
//! 시스템 설정은 읽지 않고, hook 을 끈다. 그 위에 이 파일이 지키는 규칙:
//!
//! - 저장소 설정이 프로그램을 실행하면 (필터, `core.worktree`, `diff.external` 등) 거절한다. 판정은
//!   `git_execution_guard` 의 것이다.
//! - 필터 프로그램은 실행되지 않는다. 사용자 설정의 필터(`git lfs install` 이 쓰는 것)를 이 git 은 읽지 않고, 저장소
//!   설정의 필터는 위에서 거절한다. 그런데 필터 없이 체크아웃하면 필터를 쓰는 파일의 내용이 틀린다 (LFS 는 포인터
//!   파일이 된다). 그래서 `.gitattributes` 가 필터를 쓰는 저장소는 거절한다.
//! - 서브모듈로 내려가지 않는다 (`submodule.recurse=false`). 서브모듈의 설정과 속성은 보지 않았다.
//! - 줄 끝 설정은 사용자의 것을 쓴다. Git for Windows 는 `core.autocrlf=true` 를 시스템 설정에 둔다. 그 값을 모르면
//!   사용자의 git 이 CRLF 로 쓴 파일이 모두 "수정됨"으로 보이고, 체크아웃이 LF 로 쓴다. 그래서 줄 끝 키 둘만 사용자
//!   설정과 시스템 설정에서 읽어 명령에 준다. 순서는 git 과 같다: 저장소 설정, 사용자 설정, 시스템 설정.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, bail};

use crate::application::port::outbound::git_workspace_port::{
    GitWorkspaceCondition, GitWorkspacePort,
};
use crate::domain::git_operation::GitOperationImpact;
use crate::{git_execution_guard, git_subprocess, subprocess};

mod conversation_worktree;

use super::progress_markers::IN_PROGRESS_MARKERS;

/// git 이 실패했을 때 올리는 글의 길이이다.
const FAILURE_TEXT_LIMIT: usize = 600;
/// `.gitattributes` 에서 필터를 정하는 글이다.
const FILTER_ATTRIBUTE: &str = "filter=";
/// 어느 작업 트리의 rebase 나 bisect 가 쓰는 브랜치를 적은 파일들이다. 작업 트리의 git 폴더 안에 있다.
const BRANCH_IN_USE_FILES: &[&str] = &[
    "rebase-merge/head-name",
    "rebase-apply/head-name",
    "BISECT_START",
];
/// 사용자 설정과 시스템 설정에서 읽어 오는 키와 받는 값이다. 값은 이 목록의 글만 명령에 준다.
const LINE_ENDING_KEYS: &[(&str, &[&str])] = &[
    ("core.autocrlf", &["true", "false", "input"]),
    ("core.eol", &["lf", "crlf", "native"]),
];
const LINE_ENDING_KEYS_REGEX: &str = "^core\\.(autocrlf|eol)$";
/// 저장소의 모든 폴더의 `.gitattributes` 이다.
const ATTRIBUTE_FILES_PATHSPEC: &str = ":(glob)**/.gitattributes";

pub(crate) struct GitWorkspaceAdapter;

impl GitWorkspacePort for GitWorkspaceAdapter {
    fn create_conversation_worktree(
        &self,
        repository_dir: &str,
        destination: &str,
        branch: &str,
        base_commit: &str,
    ) -> Result<()> {
        conversation_worktree::create(repository_dir, destination, branch, base_commit)
    }

    fn inspect_conversation_worktree(
        &self,
        repository_dir: &str,
        destination: &str,
    ) -> Result<
        Option<crate::application::port::outbound::git_workspace_port::GitConversationWorktree>,
    > {
        conversation_worktree::inspect(repository_dir, destination)
    }

    fn read_condition(&self, repository_dir: &str) -> Result<GitWorkspaceCondition> {
        let repository = absolute(repository_dir)?;
        if let Some(refusal) = configuration_refusal(repository) {
            return Ok(refused(refusal));
        }
        let overrides = CommandOverrides::read(repository)?;
        let dirs = GitDirs::read(repository)?;
        // 지금 커밋의 파일과 stage 한 파일을 본다. 대상 커밋의 파일은 고치는 명령이 실행 직전에 본다.
        for source in [Some("HEAD"), None] {
            if let Some(refusal) = filter_attribute_refusal(repository, &dirs, source)? {
                return Ok(refused(refusal));
            }
        }
        let status = read(
            repository,
            &overrides.with(&["status", "--porcelain=v1", "--untracked-files=no"]),
        )?;
        let operation_in_progress = IN_PROGRESS_MARKERS
            .iter()
            .find(|(marker, _)| dirs.worktree.join(marker).exists())
            .map(|(_, name)| (*name).to_string());
        Ok(GitWorkspaceCondition {
            refusal: None,
            tracked_changes: !status.trim().is_empty(),
            operation_in_progress,
            busy_branches: branches_in_use(&dirs.common),
        })
    }

    fn resolve_commit(&self, repository_dir: &str, commit: &str) -> Result<Option<String>> {
        let repository = absolute(repository_dir)?;
        let revision = format!("{commit}^{{commit}}");
        Ok(
            run(repository, &["rev-parse", "--verify", "--quiet", &revision])?
                .ok()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty()),
        )
    }

    fn read_move_impact(
        &self,
        repository_dir: &str,
        branch: &str,
        from: &str,
        to: &str,
    ) -> Result<GitOperationImpact> {
        let repository = absolute(repository_dir)?;
        let exclude_branch = format!("--exclude={branch}");
        // `--not` 뒤의 것은 모두 빼는 쪽이다. 옮기는 브랜치는 아직 `from` 을 가리키므로 `--branches` 에서 뺀다.
        let unreachable = [
            "rev-list",
            "--count",
            from,
            "--not",
            to,
            exclude_branch.as_str(),
            "--branches",
            "--remotes",
            "--tags",
            "--",
        ];
        Ok(GitOperationImpact {
            dropped: count(
                repository,
                &["rev-list", "--count", from, "--not", to, "--"],
            )?,
            unreachable: count(repository, &unreachable)?,
            gained: count(
                repository,
                &["rev-list", "--count", to, "--not", from, "--"],
            )?,
        })
    }

    fn create_ref(&self, repository_dir: &str, full_ref: &str, commit: &str) -> Result<()> {
        let repository = absolute(repository_dir)?;
        require_full_ref(full_ref)?;
        // 빈 옛 값은 "이 ref 가 없어야 한다"는 뜻이다. ref 만 쓴다. 파일을 쓰지 않으므로 필터와 관계가 없다.
        read(
            repository,
            &["update-ref", "--no-deref", full_ref, commit, ""],
        )
        .map(|_| ())
    }

    fn delete_ref(&self, repository_dir: &str, full_ref: &str) -> Result<()> {
        let repository = absolute(repository_dir)?;
        require_full_ref(full_ref)?;
        read(repository, &["update-ref", "--no-deref", "-d", full_ref]).map(|_| ())
    }

    fn reset_to(&self, repository_dir: &str, commit: &str) -> Result<()> {
        // 추적하는 파일에 변경이 없으면 `--keep` 은 `--hard` 와 같은 결과이다. 다른 점은 하나이다: 추적하지 않는
        // 파일을 덮어써야 하면 `--keep` 은 멈추고 `--hard` 는 덮어쓴다.
        write_files(repository_dir, commit, &["reset", "--keep", commit])
    }

    fn move_branch(&self, repository_dir: &str, branch: &str, to: &str, from: &str) -> Result<()> {
        let repository = absolute(repository_dir)?;
        let full_ref = format!("refs/heads/{branch}");
        read(
            repository,
            &["update-ref", "-m", "akra: move branch", &full_ref, to, from],
        )
        .map(|_| ())
    }

    fn switch_branch(&self, repository_dir: &str, branch: &str) -> Result<()> {
        let target = format!("refs/heads/{branch}");
        // `--no-guess` 는 같은 이름의 원격 브랜치로 새 브랜치를 만들지 않게 한다.
        write_files(repository_dir, &target, &["switch", "--no-guess", branch])
    }

    fn switch_detached(&self, repository_dir: &str, commit: &str) -> Result<()> {
        write_files(repository_dir, commit, &["switch", "--detach", commit])
    }
}

fn refused(refusal: String) -> GitWorkspaceCondition {
    GitWorkspaceCondition {
        refusal: Some(refusal),
        ..GitWorkspaceCondition::default()
    }
}

/// 저장소 설정이 프로그램을 실행하면 그 이유이다. 검사를 하지 못한 때도 이유로 본다: 모르는 저장소는 고치지 않는다.
fn configuration_refusal(repository: &Path) -> Option<String> {
    git_execution_guard::ensure_host_git_execution_config_safe(repository)
        .err()
        .map(|error| format!("{error:#}"))
}

/// 작업 트리의 파일을 읽거나 쓰는 명령에 주는 `-c` 인자들이다: 서브모듈로 내려가지 않고, 사용자의 줄 끝 설정을 쓴다.
struct CommandOverrides(Vec<String>);

impl CommandOverrides {
    fn read(repository: &Path) -> Result<Self> {
        let mut overrides = vec!["-c".to_string(), "submodule.recurse=false".to_string()];
        // 찾지 못하면 git 은 실패(1)와 빈 출력으로 답한다.
        let listed = |scope: &str, user_configuration: bool| -> Result<String> {
            let args = ["config", scope, "--get-regexp", LINE_ENDING_KEYS_REGEX];
            let mut command = git_subprocess::command(args);
            if user_configuration {
                // `git_subprocess` 는 사용자 설정 파일을 가린다. 이 읽기 하나만 그 파일을 본다. 키 둘의 값만 읽고,
                // 값은 `LINE_ENDING_KEYS` 의 글일 때만 쓴다. `--global` 은 그 파일의 include 를 따라가지 않는다.
                command.env_remove("GIT_CONFIG_GLOBAL");
            }
            Ok(output_of(repository, command)?.unwrap_or_default())
        };
        overrides.extend(line_ending_overrides(&[
            &listed("--local", false)?,
            &listed("--global", true)?,
            &listed("--system", false)?,
        ]));
        Ok(Self(overrides))
    }

    /// `-c` 를 붙인 명령의 인자이다. `-c` 는 하위 명령 앞에 온다.
    fn with<'args>(&'args self, args: &[&'args str]) -> Vec<&'args str> {
        self.0
            .iter()
            .map(String::as_str)
            .chain(args.iter().copied())
            .collect()
    }
}

/// 줄 끝 값을 주는 `-c` 인자들이다. `listings` 는 저장소 설정, 사용자 설정, 시스템 설정의 `git config --get-regexp`
/// 출력이다. 이 순서로 이긴다. 저장소 설정에 있는 키는 주지 않는다: 이 git 이 이미 읽는다.
fn line_ending_overrides(listings: &[&str; 3]) -> Vec<String> {
    let value_of = |listing: &str, key: &str| -> Option<String> {
        listing
            .lines()
            .filter_map(|line| line.split_once(' '))
            .filter(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.trim().to_ascii_lowercase())
            .next_back()
    };
    let [local, user, system] = listings;
    LINE_ENDING_KEYS
        .iter()
        .filter(|(key, _)| value_of(local, key).is_none())
        .filter_map(|(key, allowed)| {
            let value = value_of(user, key).or_else(|| value_of(system, key))?;
            allowed
                .contains(&value.as_str())
                .then(|| ["-c".to_string(), format!("{key}={value}")])
        })
        .flatten()
        .collect()
}

/// 어느 작업 트리의 끝나지 않은 rebase 나 bisect 가 쓰는 브랜치들이다. `common` 은 저장소의 git 폴더이다. 대표 작업
/// 트리의 표시는 그 폴더에, 연결된 작업 트리의 표시는 `worktrees/<이름>` 에 있다. 읽지 못한 폴더는 건너뛴다.
fn branches_in_use(common: &Path) -> Vec<String> {
    let mut git_dirs = vec![common.to_path_buf()];
    if let Ok(entries) = std::fs::read_dir(common.join("worktrees")) {
        git_dirs.extend(entries.flatten().map(|entry| entry.path()));
    }
    let mut branches: Vec<String> = git_dirs
        .iter()
        .flat_map(|git_dir| {
            BRANCH_IN_USE_FILES
                .iter()
                .map(move |file| git_dir.join(file))
        })
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .map(|name| {
            let name = name.trim();
            name.strip_prefix("refs/heads/").unwrap_or(name).to_string()
        })
        .filter(|name| !name.is_empty())
        .collect();
    branches.sort();
    branches.dedup();
    branches
}

/// `source` 의 파일이 필터를 쓰면 거절하는 이유이다. `source` 는 커밋이나 ref 이고, `None` 은 stage 한 파일이다.
/// `.gitattributes` 의 글만 본다. git 의 속성 계산을 하지 않는다: 필터를 끄는 줄(`-filter`)만 있는 저장소는
/// 거절하지 않고, 필터를 켜는 줄이 하나라도 있으면 그 줄이 어느 파일에도 맞지 않아도 거절한다.
fn filter_attribute_refusal(
    repository: &Path,
    dirs: &GitDirs,
    source: Option<&str>,
) -> Result<Option<String>> {
    // 속성은 저장소의 git 폴더의 `info/attributes` 에도 둘 수 있다.
    let local =
        std::fs::read_to_string(dirs.common.join("info").join("attributes")).unwrap_or_default();
    if local.contains(FILTER_ATTRIBUTE) {
        return Ok(Some(filter_refusal("info/attributes")));
    }
    let mut args = vec![
        "grep",
        "--no-textconv",
        "-I",
        "-l",
        "-F",
        "-e",
        FILTER_ATTRIBUTE,
    ];
    args.push(source.unwrap_or("--cached"));
    args.extend(["--", ATTRIBUTE_FILES_PATHSPEC]);
    // 찾으면 0, 없으면 1 과 빈 출력이다. 오류는 글이 있다.
    match run(repository, &args)? {
        Ok(found) if found.trim().is_empty() => Ok(None),
        Ok(found) => Ok(Some(filter_refusal(found.lines().next().unwrap_or("")))),
        Err(reason) if reason.is_empty() => Ok(None),
        Err(reason) => bail!("git grep failed: {reason}"),
    }
}

fn filter_refusal(place: &str) -> String {
    format!(
        "files in this repository use a content filter such as Git LFS (`{place}`); Akra does not run filters, so use git in a terminal"
    )
}

fn absolute(repository_dir: &str) -> Result<&Path> {
    let repository = Path::new(repository_dir);
    if !repository.is_absolute() {
        bail!("the repository folder must be an absolute path");
    }
    Ok(repository)
}

fn require_full_ref(full_ref: &str) -> Result<()> {
    if !full_ref.starts_with("refs/") {
        bail!("the ref name must start with `refs/`");
    }
    Ok(())
}

/// 저장소의 git 폴더 둘이다.
struct GitDirs {
    /// 이 작업 트리의 git 폴더이다. 연결된 작업 트리는 `.git/worktrees/<이름>` 이다. 끝나지 않은 작업의 표시는
    /// 여기에 있다.
    worktree: PathBuf,
    /// 모든 작업 트리가 함께 쓰는 git 폴더이다. `info/attributes` 는 여기에 있다.
    common: PathBuf,
}

impl GitDirs {
    fn read(repository: &Path) -> Result<Self> {
        let printed = read(repository, &["rev-parse", "--git-dir", "--git-common-dir"])?;
        let mut lines = printed.lines().map(|line| {
            let path = PathBuf::from(line.trim());
            if path.is_absolute() {
                path
            } else {
                repository.join(path)
            }
        });
        match (lines.next(), lines.next()) {
            (Some(worktree), Some(common)) => Ok(Self { worktree, common }),
            _ => bail!("git rev-parse did not print the git folders"),
        }
    }
}

fn count(repository: &Path, args: &[&str]) -> Result<usize> {
    read(repository, args)?
        .trim()
        .parse()
        .context("git rev-list --count printed something that is not a number")
}

/// 작업 트리의 파일을 쓰는 명령이다. 설정과 `target` 의 파일을 실행 직전에 다시 본다: 계획 뒤에 설정이 바뀌었을 수
/// 있고, 대상 커밋만 필터를 쓸 수 있다.
fn write_files(repository_dir: &str, target: &str, args: &[&str]) -> Result<()> {
    let repository = absolute(repository_dir)?;
    if let Some(refusal) = configuration_refusal(repository) {
        bail!(refusal);
    }
    let overrides = CommandOverrides::read(repository)?;
    let dirs = GitDirs::read(repository)?;
    if let Some(refusal) = filter_attribute_refusal(repository, &dirs, Some(target))? {
        bail!(refusal);
    }
    read(repository, &overrides.with(args)).map(|_| ())
}

/// 실패하면 안 되는 명령의 출력이다.
fn read(repository: &Path, args: &[&str]) -> Result<String> {
    let name = args
        .iter()
        .find(|arg| !arg.starts_with('-') && !arg.contains('='))
        .copied()
        .unwrap_or("command");
    run(repository, args)?.map_err(|reason| anyhow::anyhow!("git {name} failed: {reason}"))
}

/// 명령이 성공하면 `Ok(출력)` 이고, 실패하면 `Err(git 이 쓴 오류)` 이다. 실행하지 못하면 바깥 오류이다.
fn run(repository: &Path, args: &[&str]) -> Result<std::result::Result<String, String>> {
    output_of(repository, git_subprocess::command(args.iter().copied()))
}

fn output_of(
    repository: &Path,
    mut command: std::process::Command,
) -> Result<std::result::Result<String, String>> {
    command
        .current_dir(repository)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = subprocess::command_output(&mut command, "git workspace")
        .with_context(|| format!("failed to run git in `{}`", repository.display()))?;
    if output.status.success() {
        return Ok(Ok(String::from_utf8_lossy(&output.stdout).into_owned()));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let reason: String = stderr.trim().chars().take(FAILURE_TEXT_LIMIT).collect();
    Ok(Err(reason))
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;
