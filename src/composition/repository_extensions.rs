//! Repository-scoped extension catalog and execution lifetime.
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::application::port::inbound::repository_extension::ExtensionManifest;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::sync::broadcast;

use crate::composition::production;
use crate::trusted_executable::{resolve_blog_program, resolve_codex_command_from_path};

const BLOG: &str = "akra.tech-blog";

pub(crate) struct RepositoryExtensions {
    workspace: PathBuf,
    owner: String,
    changed: broadcast::Sender<()>,
    jobs: AtomicUsize,
}

impl RepositoryExtensions {
    pub(crate) fn new(workspace: &str) -> Arc<Self> {
        Arc::new(Self {
            workspace: workspace.into(),
            owner: format!("{:032x}", rand::random::<u128>()),
            changed: broadcast::channel(8).0,
            jobs: AtomicUsize::new(0),
        })
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<()> {
        self.changed.subscribe()
    }

    pub(crate) fn busy(&self) -> bool {
        self.jobs.load(Ordering::Acquire) != 0
    }

    pub(crate) async fn catalog(self: &Arc<Self>) -> Result<Vec<ExtensionManifest>> {
        let state = self.call(BLOG, 1, "catalog", json!({})).await?;
        Ok(vec![ExtensionManifest {
            id: BLOG,
            version: 1,
            scope: "repository",
            enabled: state["enabled"]
                .as_bool()
                .context("invalid activation state")?,
            panel: BLOG,
            label: "기술 블로그",
            actions: &[
                "configure",
                "create",
                "open",
                "list",
                "save",
                "figure",
                "accept",
                "write",
                "review",
            ],
            permissions: &["git.read", "article.write", "model.run"],
        }])
    }

    pub(crate) async fn call(
        self: &Arc<Self>,
        extension: &str,
        version: u32,
        action: &str,
        input: Value,
    ) -> Result<Value> {
        if extension != BLOG || version != 1 {
            bail!("지원하지 않는 확장 또는 버전입니다.");
        }
        if !matches!(
            action,
            "catalog"
                | "configure"
                | "list"
                | "open"
                | "create"
                | "save"
                | "figure"
                | "accept"
                | "write"
                | "review"
        ) {
            bail!("지원하지 않는 확장 명령입니다.");
        }
        let workspace = self.workspace.to_string_lossy().into_owned();
        let runtime =
            tokio::task::spawn_blocking(move || production::delegation_runtime(&workspace))
                .await??;
        if action == "catalog" && !runtime.join("tech-blog.sqlite3").try_exists()? {
            return Ok(json!({"id":BLOG, "version":1, "scope":"repository", "enabled":false}));
        }
        let answer = self.invoke(&runtime, action, input).await?;
        if matches!(action, "write" | "review") {
            let id = answer["id"]
                .as_str()
                .context("missing article ID")?
                .to_owned();
            let job = answer["job"]["id"]
                .as_str()
                .context("missing job ID")?
                .to_owned();
            let host = self.clone();
            self.jobs.fetch_add(1, Ordering::AcqRel);
            tokio::spawn(async move {
                let input = json!({"id":id, "job":job});
                if host
                    .invoke(&runtime, "execute", input.clone())
                    .await
                    .is_err()
                {
                    let _ = host.invoke(&runtime, "failed", input).await;
                }
                host.jobs.fetch_sub(1, Ordering::AcqRel);
                let _ = host.changed.send(());
            });
        }
        if !matches!(action, "catalog" | "list" | "open") {
            let _ = self.changed.send(());
        }
        Ok(answer)
    }

    async fn invoke(&self, runtime: &Path, action: &str, input: Value) -> Result<Value> {
        let program = if cfg!(windows) { "python" } else { "python3" };
        let python = resolve_blog_program(program, &self.workspace)
            .context("기술 블로그 확장에 Python 3가 필요합니다.")?;
        let git = if matches!(action, "create" | "execute") {
            Some(resolve_blog_program("git", &self.workspace)?)
        } else {
            None
        };
        let model_command = if action == "execute" {
            std::env::var_os("PATH")
                .and_then(|path| resolve_codex_command_from_path(&path, &self.workspace).ok())
                .filter(|cmd| {
                    !cmd.program.starts_with(&self.workspace)
                        && !cmd.source_executable.starts_with(&self.workspace)
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
                include_str!("../../scripts/tech_blog_bootstrap.py"),
            ])
            .current_dir(runtime)
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let request = json!({
            "scripts": {
                "tech_blog.py": include_str!("../../scripts/tech_blog.py"),
                "tech_blog_studio.py": include_str!("../../scripts/tech_blog_studio.py"),
                "tech_blog_review.schema.json": include_str!("../../scripts/tech_blog_review.schema.json"),
                "tech_blog_write.schema.json": include_str!("../../scripts/tech_blog_write.schema.json"),
            },
            "git": git, "model_command":model_command, "workspace": self.workspace, "runtime":runtime, "owner":self.owner, "action":action, "input":input
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
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn packaged_blog_worker_round_trips_without_a_model() {
        let root =
            std::env::temp_dir().join(format!("akra-blog-host-{:032x}", rand::random::<u128>()));
        std::fs::create_dir(&root).unwrap();
        let host = RepositoryExtensions::new(env!("CARGO_MANIFEST_DIR"));
        let result = async {
            let enabled = host
                .invoke(&root, "configure", json!({"enabled": true}))
                .await?;
            anyhow::ensure!(enabled["enabled"] == true);
            let article = host
                .invoke(
                    &root,
                    "create",
                    json!({"topic":"Editing a draft","paths":["README.md"],
                "style":"PROBLEM_SOLVING","mode":"PROJECT_RETROSPECTIVE"}),
                )
                .await?;
            let edited = host
                .invoke(
                    &root,
                    "save",
                    json!({"id":article["id"], "revision":1, "draft":"Updated"}),
                )
                .await?;
            anyhow::ensure!(edited["draft"] == "Updated" && edited["revision"] == 2);
            anyhow::ensure!(
                host.invoke(
                    &root,
                    "save",
                    json!({"id":article["id"],"revision":1,"draft":"Old"})
                )
                .await
                .is_err()
            );
            Ok::<(), anyhow::Error>(())
        }
        .await;
        std::fs::remove_dir_all(root).unwrap();
        result.unwrap();
    }
}
