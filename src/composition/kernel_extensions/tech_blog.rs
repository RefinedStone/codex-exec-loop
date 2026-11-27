//! The bundled tech blog studio registered on the common repository extension host.
//!
//! The Python studio keeps its article state machine, revisions and editions. The host owns
//! activation, job receipts, cancellation, retention and restart recovery.
use crate::application::service::kernel_extensions::ExtensionJobContext;
use crate::application::service::kernel_extensions::registry::{
    ExtensionHandler, ExtensionRegistration,
};
use crate::domain::kernel_extension::{
    EXTENSION_API_VERSION, ExtensionContext, ExtensionDescriptor, ExtensionScope,
};
use crate::domain::repository_extension::tech_blog::{self, ID, JOBS, VERSION};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const ACTIONS: [&str; 14] = [
    "targets", "list", "open", "seen", "generate", "stop", "resume", "revise", "review", "save",
    "restore", "figure", "export", "delete",
];
const STUDIO: &str = "tech-blog.sqlite3";

pub(super) fn registration(workspace: &Path, runtime: PathBuf) -> ExtensionRegistration {
    ExtensionRegistration {
        manifest: ExtensionDescriptor {
            id: ID.into(),
            version: VERSION,
            api_version: EXTENSION_API_VERSION,
            scope: ExtensionScope::Repository,
            label: "기술 블로그".into(),
            panel: Some(ID.into()),
            permissions: ["git.read", "article.write", "model.run", "workspace.write"]
                .map(String::from)
                .to_vec(),
            operations: tech_blog::operations(),
        },
        handler: Arc::new(TechBlogHandler {
            workspace: workspace.to_owned(),
            runtime,
            owner: format!("{:032x}", rand::random::<u128>()),
            studio_enabled: Mutex::new(false),
        }),
    }
}

/// True when a version 2 studio was enabled before the common host owned activation.
pub(super) fn legacy_enabled(runtime: &Path) -> bool {
    let path = runtime.join(STUDIO);
    path.is_file()
        && rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .and_then(|db| {
                db.query_row("SELECT enabled FROM settings WHERE id=1", [], |row| {
                    row.get::<_, bool>(0)
                })
            })
            .unwrap_or(false)
}

struct TechBlogHandler {
    workspace: PathBuf,
    runtime: PathBuf,
    owner: String,
    studio_enabled: Mutex<bool>,
}

impl TechBlogHandler {
    fn worker(&self, action: &str, input: Value) -> Result<Value> {
        self.block_on(self.worker_async(action, input))?
    }
    fn block_on<T>(&self, future: impl std::future::Future<Output = T>) -> Result<T> {
        // Handlers run on blocking threads; each call drives its own worker process.
        Ok(tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(future))
    }
    async fn worker_async(&self, action: &str, input: Value) -> Result<Value> {
        crate::adapter::outbound::tech_blog_worker::invoke(
            &self.workspace,
            &self.owner,
            &self.runtime,
            action,
            input,
        )
        .await
    }
    /// The studio keeps its own switch. The host is the only gate in version 3.
    fn ensure_studio(&self) -> Result<()> {
        let mut enabled = self
            .studio_enabled
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if !*enabled {
            std::fs::create_dir_all(&self.runtime)?;
            self.worker("configure", json!({"enabled": true}))?;
            *enabled = true;
        }
        Ok(())
    }
    /// Runs stages until none remain. A host cancel stops the article and its model process.
    fn drive(&self, context: &ExtensionJobContext, id: &str, job: &str) -> Result<()> {
        let step = json!({"id": id, "job": job});
        loop {
            // Never start the next (model) stage after a cancel request.
            if context.cancelled() {
                bail!("extension job was cancelled");
            }
            let (step, cancelled) = self.block_on(async {
                let execute = self.worker_async("execute", step.clone());
                tokio::pin!(execute);
                let poll = async {
                    while !context.cancelled() {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                };
                tokio::select! {
                    step = &mut execute => (step, false),
                    () = poll => {
                        // The studio stop ends the running model process; then the stage returns.
                        let _ = self.worker_async("cancel", json!({"id": id})).await;
                        (execute.await, true)
                    }
                }
            })?;
            if cancelled {
                bail!("extension job was cancelled");
            }
            let step = step?;
            let article = self.worker("open", json!({"id": id}))?;
            context.report_progress(json!({"article_id": id, "stage": article["stage"]}))?;
            if step["more"] != json!(true) {
                return Ok(());
            }
        }
    }
}

impl ExtensionHandler for TechBlogHandler {
    fn operations(&self) -> &'static [&'static str] {
        &ACTIONS
    }
    fn invoke(&self, _context: &ExtensionContext, action: &str, input: Value) -> Result<Value> {
        self.ensure_studio()?;
        let action = if action == "stop" { "cancel" } else { action };
        self.worker(action, input)
    }
    fn run(&self, context: &ExtensionJobContext, action: &str, input: Value) -> Result<Value> {
        anyhow::ensure!(JOBS.contains(&action), "not a tech blog job");
        self.ensure_studio()?;
        let article = self.worker(action, input)?;
        let id = article["id"]
            .as_str()
            .context("missing article ID")?
            .to_owned();
        let job = article["job"]["id"]
            .as_str()
            .context("missing job ID")?
            .to_owned();
        let outcome = context
            .report_progress(json!({"article_id": id, "stage": article["stage"]}))
            .and_then(|()| self.drive(context, &id, &job));
        if let Err(error) = outcome {
            // The studio job belongs to this owner; leave the article stopped, never running.
            let _ = if context.cancelled() {
                self.worker("cancel", json!({"id": id}))
            } else {
                self.worker("failed", json!({"id": id, "job": job}))
            };
            return Err(error);
        }
        self.worker("open", json!({"id": id}))
    }
    fn max_active_jobs(&self) -> Option<usize> {
        // One model call at a time per repository, as in version 2.
        Some(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_declares_exactly_the_implemented_actions() {
        let registration = registration(Path::new("."), PathBuf::from("."));
        let declared = registration
            .manifest
            .operations
            .iter()
            .map(|o| o.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(declared, ACTIONS.into_iter().collect());
        for operation in &registration.manifest.operations {
            let job = JOBS.contains(&operation.name.as_str());
            assert_eq!(
                operation.kind == crate::domain::kernel_extension::ExtensionOperationKind::Job,
                job,
                "{}",
                operation.name
            );
            assert_eq!(operation.progress_schema.is_some(), job);
        }
    }

    #[test]
    fn packaged_blog_worker_round_trips_without_a_model() {
        let root =
            std::env::temp_dir().join(format!("akra-blog-host-{:032x}", rand::random::<u128>()));
        std::fs::create_dir(&root).unwrap();
        let handler = TechBlogHandler {
            workspace: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            runtime: root.clone(),
            owner: "round-trip".into(),
            studio_enabled: Mutex::new(false),
        };
        let result = (|| {
            handler.ensure_studio()?;
            let targets = handler.worker("targets", json!({"limit":1}))?;
            let commit = targets["commits"][0]["sha"].clone();
            let article = handler.worker("generate", json!({"commits":[commit]}))?;
            anyhow::ensure!(article["state"] == "running" && article["stage"] == "collect");
            let collected = handler.worker(
                "execute",
                json!({"id":article["id"], "job":article["job"]["id"]}),
            )?;
            anyhow::ensure!(collected["more"] == true, "{collected}");
            // `stop` maps to the studio's article stop.
            let context = ExtensionContext {
                host_id: "host".into(),
                generation: "generation".into(),
                connection_id: "connection".into(),
                repository_key: Some("host".into()),
                verified_thread: None,
                conversation_generation: None,
                activation_revision: 1,
            };
            let stopped = handler.invoke(&context, "stop", json!({"id":article["id"]}))?;
            anyhow::ensure!(stopped["state"] == "stopped" && stopped["stage"] == "write");
            let edited = handler.worker(
                "save",
                json!({"id":article["id"], "revision":stopped["revision"], "draft":"Updated"}),
            )?;
            anyhow::ensure!(edited["draft"] == "Updated" && edited["state"] == "stopped");
            anyhow::ensure!(
                handler
                    .worker(
                        "save",
                        json!({"id":article["id"],"revision":stopped["revision"],"draft":"Old"})
                    )
                    .is_err()
            );
            Ok::<(), anyhow::Error>(())
        })();
        std::fs::remove_dir_all(root).unwrap();
        result.unwrap();
    }

    #[test]
    fn legacy_enabled_reads_only_an_enabled_version_two_studio() {
        let root =
            std::env::temp_dir().join(format!("akra-blog-legacy-{:032x}", rand::random::<u128>()));
        std::fs::create_dir(&root).unwrap();
        assert!(!legacy_enabled(&root));
        let db = rusqlite::Connection::open(root.join(STUDIO)).unwrap();
        db.execute_batch("CREATE TABLE settings (id INTEGER PRIMARY KEY, enabled INTEGER); INSERT INTO settings VALUES (1, 0);")
            .unwrap();
        assert!(!legacy_enabled(&root));
        db.execute("UPDATE settings SET enabled=1 WHERE id=1", [])
            .unwrap();
        drop(db);
        assert!(legacy_enabled(&root));
        std::fs::remove_dir_all(root).unwrap();
    }
}
