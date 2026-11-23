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

pub(crate) struct RepositoryExtensions {
    workspace: PathBuf,
    owner: String,
    changed: broadcast::Sender<()>,
    jobs: AtomicUsize,
    common: std::sync::Mutex<
        Option<Arc<crate::application::service::kernel_extensions::ExtensionService>>,
    >,
}

impl RepositoryExtensions {
    pub(crate) fn new(workspace: &str) -> Arc<Self> {
        Arc::new(Self {
            workspace: workspace.into(),
            owner: format!("{:032x}", rand::random::<u128>()),
            changed: broadcast::channel(8).0,
            jobs: AtomicUsize::new(0),
            common: std::sync::Mutex::new(None),
        })
    }

    pub(crate) fn common(
        &self,
        kernel: &str,
    ) -> Result<Arc<crate::application::service::kernel_extensions::ExtensionService>> {
        let mut common = self.common.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(host) = &*common {
            return Ok(host.clone());
        }
        let host = crate::composition::kernel_extensions::build_repository(
            &self.workspace.to_string_lossy(),
            kernel,
        )?;
        *common = Some(host.clone());
        Ok(host)
    }
    pub(crate) fn shutdown(&self) {
        if let Some(host) = self
            .common
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            host.shutdown();
        }
    }
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<()> {
        self.changed.subscribe()
    }

    pub(crate) fn busy(&self) -> bool {
        self.jobs.load(Ordering::Acquire) != 0
            || self
                .common
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .as_ref()
                .is_some_and(|h| h.busy())
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
            self.start_job(&runtime, json!({"id":id, "job":job}))
                .await?;
        }
        if !matches!(action, "catalog" | "list" | "open") {
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
            move || executor.block_on(work.invoke(&work_dir, "execute", work_input)),
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
