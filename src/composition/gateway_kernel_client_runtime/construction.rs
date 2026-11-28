//! Compose the two Gateway read paths without creating a local repository kernel.
use std::sync::{Arc, mpsc};

use anyhow::{Context, Result};

use super::commands::GatewayCommands;
use super::conversation::GatewayConversationClient;
use super::{ClientUpdate, GatewayKernelClientRuntime, TUI_KERNEL_CLIENT_NAME};
use crate::adapter::outbound::app_server::{CodexAppServerAdapter, SharedAppServer};
use crate::adapter::outbound::gateway_kernel_client::{
    GatewayKernelClientAdapter, spawn_kernel_value_watcher,
};
use crate::core::app::{AppSnapshot, SessionCatalogSnapshot};

impl GatewayKernelClientRuntime {
    pub(crate) fn attach(workspace_directory: String) -> Result<Self> {
        let (sender, updates) = mpsc::channel();
        let reader = Arc::new(
            CodexAppServerAdapter::from_environment(
                TUI_KERNEL_CLIENT_NAME,
                env!("CARGO_PKG_VERSION"),
            )
            .through_shared_app_server(SharedAppServer::Relay {
                relay_executable: std::env::current_exe()
                    .context("resolve the TUI relay executable")?,
                endpoint: crate::gateway_endpoint::client_endpoint()?,
            }),
        );
        let conversation = GatewayConversationClient::new(reader.clone(), sender.clone())?;
        let commands = GatewayCommands::new(
            Arc::new(GatewayKernelClientAdapter::new(
                workspace_directory.clone(),
                TUI_KERNEL_CLIENT_NAME.to_string(),
            )),
            Some(reader),
            sender.clone(),
        )?;
        let refresh = spawn_kernel_value_watcher(
            workspace_directory.clone(),
            TUI_KERNEL_CLIENT_NAME,
            sender,
        )?;
        let mut client = Self::with_updates(workspace_directory, updates, conversation);
        client.refresh = Some(refresh);
        client.commands = Some(commands);
        Ok(client)
    }

    pub(super) fn with_updates(
        workspace_directory: String,
        updates: mpsc::Receiver<ClientUpdate>,
        conversation: GatewayConversationClient,
    ) -> Self {
        let mut snapshot = AppSnapshot::initial();
        snapshot.session_catalog = SessionCatalogSnapshot::Loading;
        Self {
            workspace_directory,
            snapshot,
            startup_generation: 0,
            startup_workspace: None,
            synchronized: false,
            refresh: None,
            updates,
            conversation,
            commands: None,
        }
    }
}
