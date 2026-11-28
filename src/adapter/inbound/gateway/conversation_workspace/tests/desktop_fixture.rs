//! Stdio transport for the opt-in Desktop scenario. Routes and storage are native.
use std::io::{BufRead, Write};
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc;

use super::super::spawn_request;
use crate::adapter::outbound::db::conversation_workspace_store::ConversationWorkspaceStoreAdapter;
use crate::adapter::outbound::git::{history::GitHistoryAdapter, workspace::GitWorkspaceAdapter};
use crate::application::port::outbound::conversation_workspace_store_port::ConversationWorkspaceStoreLocation;
use crate::application::service::conversation_workspace::ConversationWorkspaceService;

#[derive(Deserialize)]
struct Request {
    id: u64,
    method: String,
}

#[tokio::test]
#[ignore = "manual bridge for desktop/test/e2e/workspace-native.mjs"]
async fn desktop_workspace_fixture_bridge() -> Result<()> {
    ensure!(
        std::env::var("AKRA_WORKSPACE_FIXTURE_SERVER").as_deref() == Ok("1"),
        "launch through the disposable Desktop workspace harness"
    );
    let root = std::env::current_dir()?;
    ensure!(
        root.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("akra-workspace-desktop-")),
        "expected the generated fixture directory"
    );
    let repository_dir = root.join("repo").display().to_string();
    let store = ConversationWorkspaceStoreAdapter::new(move |cwd, _| {
        ensure!(
            cwd == repository_dir,
            "only the fixture repository is allowed"
        );
        Ok(Some(ConversationWorkspaceStoreLocation {
            repository_dir: repository_dir.clone(),
            runtime_dir: root.join("runtime"),
        }))
    });
    let service = Arc::new(ConversationWorkspaceService::new(
        Arc::new(GitHistoryAdapter),
        Arc::new(GitWorkspaceAdapter),
        Arc::new(store),
    ));
    let (outbox, mut replies) = mpsc::channel(1);
    for line in std::io::stdin().lock().lines() {
        let text = line?;
        let request: Request = serde_json::from_str(&text)?;
        ensure!(request.method.starts_with(super::super::PREFIX));
        spawn_request(
            service.clone(),
            Value::from(request.id),
            request.method,
            text,
            outbox.clone(),
        );
        let reply = replies.recv().await.context("workspace route stopped")?;
        println!("AKRA_WORKSPACE_FIXTURE {}", reply.to_text()?);
        std::io::stdout().flush()?;
    }
    Ok(())
}
