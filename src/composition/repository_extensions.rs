//! Repository-scoped extension catalog and execution lifetime.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::domain::repository_extension::ExtensionManifest;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::composition::production;

const BLOG: &str = "akra.tech-blog";
const VERSION: u32 = 2;
/// 작업을 시작하는 명령이다. 워커가 단계 1개씩 하고, 호스트는 남은 단계가 없을 때까지 다시 부른다.
const STARTS_JOB: [&str; 4] = ["generate", "resume", "revise", "review"];
const ACTIONS: [&str; 15] = [
    "catalog",
    "configure",
    "targets",
    "list",
    "open",
    "generate",
    "cancel",
    "resume",
    "revise",
    "review",
    "save",
    "restore",
    "figure",
    "export",
    "delete",
];

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
        let state = self.call(BLOG, VERSION, "catalog", json!({})).await?;
        Ok(vec![ExtensionManifest {
            id: BLOG,
            version: VERSION,
            scope: "repository",
            enabled: state["enabled"]
                .as_bool()
                .context("invalid activation state")?,
            panel: BLOG,
            label: "기술 블로그",
            actions: &ACTIONS,
            permissions: &["git.read", "article.write", "model.run", "workspace.write"],
        }])
    }

    pub(crate) async fn call(
        self: &Arc<Self>,
        extension: &str,
        version: u32,
        action: &str,
        input: Value,
    ) -> Result<Value> {
        if extension != BLOG || version != VERSION {
            bail!("지원하지 않는 확장 또는 버전입니다. Desktop 과 Akra 를 함께 업데이트하세요.");
        }
        if !ACTIONS.contains(&action) {
            bail!("지원하지 않는 확장 명령입니다.");
        }
        if STARTS_JOB.contains(&action) && self.busy() {
            // 취소는 다음 단계만 막는다. 진행 중인 모델 호출이 끝나기 전에 두 번째 호출을 시작하지 않는다.
            bail!("이전 작업의 모델 호출이 아직 끝나지 않았습니다. 잠시 뒤 다시 시도하세요.");
        }
        let workspace = self.workspace.to_string_lossy().into_owned();
        let runtime =
            tokio::task::spawn_blocking(move || production::delegation_runtime(&workspace))
                .await??;
        if action == "catalog" && !runtime.join("tech-blog.sqlite3").try_exists()? {
            return Ok(
                json!({"id":BLOG, "version":VERSION, "scope":"repository", "enabled":false}),
            );
        }
        let answer = self.invoke(&runtime, action, input).await?;
        if STARTS_JOB.contains(&action) {
            let id = answer["id"]
                .as_str()
                .context("missing article ID")?
                .to_owned();
            let job = answer["job"]["id"]
                .as_str()
                .context("missing job ID")?
                .to_owned();
            self.start_job(&runtime, json!({"id":id, "job":job}))
                .await?;
        }
        if !matches!(action, "catalog" | "targets" | "list" | "open") {
            let _ = self.changed.send(());
        }
        Ok(answer)
    }

    async fn start_job(self: &Arc<Self>, runtime: &Path, input: Value) -> Result<()> {
        let executor = tokio::runtime::Handle::current();
        let completion_executor = executor.clone();
        let work = self.clone();
        let work_dir = runtime.to_owned();
        let work_input = input.clone();
        let completion = self.clone();
        let completion_dir = runtime.to_owned();
        let completion_input = input.clone();
        self.jobs.fetch_add(1, Ordering::AcqRel);
        let started = super::core_effect_worker::try_spawn_guarded_job(
            move || executor.block_on(work.run_stages(&work_dir, work_input)),
            move |result| {
                completion_executor.block_on(completion.finish_job(
                    &completion_dir,
                    completion_input,
                    !matches!(result, Ok(Ok(_))),
                ))
            },
        );
        if let Err(error) = started {
            self.finish_job(runtime, input, true).await;
            return Err(error.into());
        }
        Ok(())
    }

    /// 단계마다 워커를 한 번 부른다. 모델 호출 1회가 워커 호출 1회의 제한 안에 들어간다.
    async fn run_stages(&self, runtime: &Path, input: Value) -> Result<()> {
        loop {
            let step = self.invoke(runtime, "execute", input.clone()).await?;
            let _ = self.changed.send(());
            if step["more"] != json!(true) {
                return Ok(());
            }
        }
    }

    async fn finish_job(&self, runtime: &Path, input: Value, failed: bool) {
        if failed {
            let _ = self.invoke(runtime, "failed", input).await;
        }
        self.jobs.fetch_sub(1, Ordering::AcqRel);
        let _ = self.changed.send(());
    }

    async fn invoke(&self, runtime: &Path, action: &str, input: Value) -> Result<Value> {
        crate::adapter::outbound::tech_blog_worker::invoke(
            &self.workspace,
            &self.owner,
            runtime,
            action,
            input,
        )
        .await
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
            let targets = host.invoke(&root, "targets", json!({"limit":1})).await?;
            let commit = targets["commits"][0]["sha"].clone();
            let article = host
                .invoke(&root, "generate", json!({"commits":[commit]}))
                .await?;
            anyhow::ensure!(article["state"] == "running" && article["stage"] == "collect");
            let collected = host
                .invoke(
                    &root,
                    "execute",
                    json!({"id":article["id"], "job":article["job"]["id"]}),
                )
                .await?;
            anyhow::ensure!(collected["more"] == true, "{collected}");
            let stopped = host
                .invoke(&root, "cancel", json!({"id":article["id"]}))
                .await?;
            anyhow::ensure!(stopped["state"] == "stopped" && stopped["stage"] == "write");
            let edited = host
                .invoke(
                    &root,
                    "save",
                    json!({"id":article["id"], "revision":stopped["revision"], "draft":"Updated"}),
                )
                .await?;
            anyhow::ensure!(edited["draft"] == "Updated" && edited["state"] == "stopped");
            anyhow::ensure!(
                host.invoke(
                    &root,
                    "save",
                    json!({"id":article["id"],"revision":stopped["revision"],"draft":"Old"})
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
