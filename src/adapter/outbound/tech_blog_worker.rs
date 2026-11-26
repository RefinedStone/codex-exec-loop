//! Run the installed blog worker independently of files in the attached repository.
use crate::trusted_executable::{resolve_blog_program, resolve_codex_command_from_path};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::io::AsyncWriteExt;

/// 블로그 집필 규칙이다 (#2872). 워커가 이 글을 모델 입력의 앞에 붙인다.
const TECH_BLOG_WRITE_SKILL: &str =
    include_str!("../../../assets/app-server/skills/akra-tech-blog/SKILL.md");

pub(crate) async fn invoke(
    workspace: &Path,
    owner: &str,
    runtime: &Path,
    action: &str,
    input: Value,
) -> Result<Value> {
    let program = if cfg!(windows) { "python" } else { "python3" };
    let python = resolve_blog_program(program, workspace)
        .context("기술 블로그 확장에 Python 3가 필요합니다.")?;
    let git = if matches!(action, "targets" | "generate" | "execute") {
        Some(resolve_blog_program("git", workspace)?)
    } else {
        None
    };
    let model_command = if matches!(action, "targets" | "execute") {
        std::env::var_os("PATH")
            .and_then(|path| resolve_codex_command_from_path(&path, workspace).ok())
            .filter(|cmd| {
                !cmd.program.starts_with(workspace) && !cmd.source_executable.starts_with(workspace)
            })
            .map(|cmd| {
                std::iter::once(cmd.program.into_os_string())
                    .chain(cmd.prefix_args)
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
            })
    } else {
        None
    };
    let mut command = tokio::process::Command::new(python);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW for a Desktop-initiated worker.
    let mut child = command
        .args([
            "-I",
            "-c",
            include_str!("../../../scripts/tech_blog_bootstrap.py"),
        ])
        .current_dir(runtime)
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let request = json!({
        "scripts": {
            "tech_blog.py": include_str!("../../../scripts/tech_blog.py"),
            "tech_blog_studio.py": include_str!("../../../scripts/tech_blog_studio.py"),
            "tech_blog_review.schema.json": include_str!("../../../scripts/tech_blog_review.schema.json"),
            "tech_blog_write.schema.json": include_str!("../../../scripts/tech_blog_write.schema.json"),
            "akra-tech-blog.SKILL.md": TECH_BLOG_WRITE_SKILL,
        },
        "git": git, "model_command":model_command, "workspace": workspace, "runtime":runtime, "owner":owner, "action":action, "input":input
    });
    let mut stdin = child.stdin.take().context("worker stdin unavailable")?;
    let timeout = Duration::from_secs(if action == "execute" { 640 } else { 30 });
    let output = tokio::time::timeout(timeout, async move {
        stdin.write_all(&serde_json::to_vec(&request)?).await?;
        drop(stdin);
        child.wait_with_output().await
    })
    .await
    .context("블로그 워커 응답 시간 초과")??;
    if !output.status.success() {
        bail!("블로그 워커가 종료됐습니다. Python 설치 상태를 확인하세요.");
    }
    let result: Value = serde_json::from_slice(&output.stdout)?;
    if let Some(error) = result["error"].as_str() {
        bail!("{error}");
    }
    result
        .get("value")
        .cloned()
        .context("블로그 워커 응답 형식 오류")
}

#[cfg(test)]
mod tests {
    use super::TECH_BLOG_WRITE_SKILL;

    #[test]
    fn the_worker_ships_the_writing_skill_that_the_bootstrap_unpacks() {
        assert!(TECH_BLOG_WRITE_SKILL.contains("name: akra-tech-blog"));
        assert!(TECH_BLOG_WRITE_SKILL.contains("한국어 기술 블로그의 필자다"));
        assert!(
            include_str!("../../../scripts/tech_blog_bootstrap.py")
                .contains("\"akra-tech-blog.SKILL.md\"")
        );
        assert!(
            include_str!("../../../scripts/tech_blog_studio.py")
                .contains("WRITE_SKILL = \"akra-tech-blog.SKILL.md\"")
        );
    }
}
