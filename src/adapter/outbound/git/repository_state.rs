//! 작업 공간의 git 상태를 읽는다 (plan 20 §4.1). 읽기 명령 두 개만 쓴다. 저장소를 고치지 않는다.

use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};

use crate::application::port::outbound::repository_state_port::RepositoryStatePort;
use crate::domain::kernel_event::RepositoryState;
use crate::{git_subprocess, subprocess};

pub(crate) struct GitRepositoryStateAdapter;

impl RepositoryStatePort for GitRepositoryStateAdapter {
    fn repository_state(&self, workspace_dir: &str) -> Result<Option<RepositoryState>> {
        let workspace = Path::new(workspace_dir);
        // 커밋이 없거나 저장소가 아니면 실패한다. 오류가 아니라 "상태 없음"이다.
        let Some(head) = git_line(workspace, &["rev-parse", "--verify", "--quiet", "HEAD"])? else {
            return Ok(None);
        };
        // 브랜치가 아닌 커밋을 체크아웃했으면 실패한다.
        let branch = git_line(workspace, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
        Ok(Some(RepositoryState { head, branch }))
    }
}

/// 명령이 성공하면 출력의 첫 줄이다. 실패했거나 출력이 비었으면 `None` 이다.
fn git_line(workspace: &Path, args: &[&str]) -> Result<Option<String>> {
    let mut command = git_subprocess::command(args.iter().copied());
    command
        .current_dir(workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = subprocess::command_output(&mut command, "git repository state")
        .with_context(|| format!("failed to run git in `{}`", workspace.display()))?;
    if !output.status.success() {
        return Ok(None);
    }
    let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((!line.is_empty()).then_some(line))
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn reads_the_commit_and_branch_and_reports_no_state_without_a_commit() {
        let directory = crate::test_utils::platform_safe_temp_dir()
            .join(format!("akra-repository-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let workspace = directory.display().to_string();
        let adapter = GitRepositoryStateAdapter;

        git(&directory, &["init", "--quiet", "--initial-branch", "main"]);
        assert_eq!(adapter.repository_state(&workspace).unwrap(), None);

        // 시험의 git 은 물려받은 설정을 버린다. 커밋에는 저장소의 신원이 필요하다.
        git(&directory, &["config", "user.name", "Akra Test"]);
        git(
            &directory,
            &["config", "user.email", "akra-test@example.com"],
        );
        git(
            &directory,
            &["commit", "--quiet", "--allow-empty", "-m", "first"],
        );
        let on_branch = adapter
            .repository_state(&workspace)
            .unwrap()
            .expect("a commit gives a state");
        assert_eq!(on_branch.branch.as_deref(), Some("main"));
        assert_eq!(on_branch.head.len(), 40);

        git(&directory, &["checkout", "--quiet", "--detach"]);
        let detached = adapter.repository_state(&workspace).unwrap().unwrap();
        assert_eq!(detached.head, on_branch.head);
        assert_eq!(detached.branch, None);
        let _ = std::fs::remove_dir_all(&directory);
    }
}
