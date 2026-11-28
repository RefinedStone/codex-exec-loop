//! TUI client for the Gateway-owned repository kernel (plan 18 K6-2).

use crate::adapter::inbound::kernel_protocol::{AutoFollowSettings, SessionsState, ThreadActivity};
use crate::adapter::outbound::gateway_kernel_client::KernelValueUpdate as KernelClientUpdate;
use crate::core::app::{
    AppEvent, AppSnapshot, ConversationLoadCorrelation, ConversationReadySnapshot,
    ConversationSnapshot, CoreDispatchOutcome, SessionCatalogSnapshot, StartupAttachmentSnapshot,
    StartupCheckCorrelation, StartupDiagnosticSnapshot, StartupReadySnapshot, StartupSnapshot,
};
use crate::core::native_client_port::NativeClientDispatchOutcome;
use serde_json::Value;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
mod client_port;
mod commands;
mod construction;
mod conversation;
mod mutation_projection;
mod projection;
use conversation::GatewayConversationClient;
use projection::{session_catalog, thread_activity};
#[cfg(test)]
mod conversation_tests;
#[cfg(test)]
pub(crate) mod fixture;
#[cfg(test)]
pub(crate) mod tests;

pub(crate) enum ClientUpdate {
    Kernel(KernelClientUpdate),
    MutationFinished {
        outcome: commands::MutationOutcome,
        accepted_prompt: Option<(String, String)>,
    },
    ConversationLoaded {
        correlation: ConversationLoadCorrelation,
        result: Result<ConversationReadySnapshot, String>,
    },
}

impl From<KernelClientUpdate> for ClientUpdate {
    fn from(update: KernelClientUpdate) -> Self {
        Self::Kernel(update)
    }
}

/// 커널이 기록하는 이 클라이언트의 이름이다.
pub(crate) const TUI_KERNEL_CLIENT_NAME: &str = "akra-tui";
pub(crate) struct GatewayKernelClientRuntime {
    workspace_directory: String,
    snapshot: AppSnapshot,
    startup_generation: u64,
    startup_workspace: Option<String>,
    synchronized: bool,
    refresh: Option<tokio::sync::mpsc::Sender<()>>,
    updates: Receiver<ClientUpdate>,
    conversation: GatewayConversationClient,
    commands: Option<commands::GatewayCommands>,
}

impl GatewayKernelClientRuntime {
    fn outcome(&self, events: Vec<AppEvent>) -> NativeClientDispatchOutcome {
        NativeClientDispatchOutcome::Core(CoreDispatchOutcome {
            events,
            effects: Vec::new(),
            snapshot: Arc::new(self.snapshot.clone()),
        })
    }

    fn apply(&mut self, update: ClientUpdate) -> Vec<AppEvent> {
        match update {
            ClientUpdate::Kernel(update) => self.apply_kernel(update),
            ClientUpdate::MutationFinished {
                outcome,
                accepted_prompt,
            } => self.apply_mutation(outcome, accepted_prompt),
            ClientUpdate::ConversationLoaded {
                correlation,
                result,
            } => {
                if !self.conversation.finish(&correlation) {
                    return Vec::new();
                }
                self.snapshot.revision = self.snapshot.revision.saturating_add(1);
                self.snapshot.conversation = match result {
                    Ok(ready) => {
                        if let Some(commands) = &mut self.commands {
                            commands.confirm_thread_selection();
                        }
                        ConversationSnapshot::Ready(Box::new(ready))
                    }
                    Err(message) => ConversationSnapshot::Failed { message },
                };
                vec![AppEvent::ConversationChanged {
                    correlation: Some(correlation),
                    snapshot: self.snapshot.conversation.clone(),
                }]
            }
        }
    }

    fn load_conversation(&mut self, thread_id: String) -> Vec<AppEvent> {
        let correlation = match self.conversation.load(thread_id) {
            Ok(correlation) => {
                self.snapshot.conversation = ConversationSnapshot::Loading;
                Some(correlation)
            }
            Err(error) => {
                tracing::warn!(%error, "TUI conversation read could not start");
                self.snapshot.conversation = ConversationSnapshot::Failed {
                    message: format!("{error:#}"),
                };
                None
            }
        };
        self.snapshot.revision = self.snapshot.revision.saturating_add(1);
        vec![AppEvent::ConversationChanged {
            correlation,
            snapshot: self.snapshot.conversation.clone(),
        }]
    }

    fn apply_kernel(&mut self, update: KernelClientUpdate) -> Vec<AppEvent> {
        self.snapshot.revision += 1;
        match update {
            KernelClientUpdate::Value(value) => self.apply_value(value),
            KernelClientUpdate::Synchronized => {
                self.synchronized = true;
                self.publish_startup(self.ready_snapshot())
            }
            KernelClientUpdate::Disconnected(reason) => {
                self.conversation.cancel();
                self.synchronized = false;
                let message = format!(
                    "lost the connection to the Akra Gateway kernel ({reason}); reconnecting"
                );
                self.snapshot.session_catalog = SessionCatalogSnapshot::Failed {
                    message: message.clone(),
                };
                let mut events = vec![
                    AppEvent::SessionCatalogChanged(self.snapshot.session_catalog.clone()),
                    AppEvent::ThreadActivityChanged(Vec::new()),
                ];
                if matches!(self.snapshot.conversation, ConversationSnapshot::Loading) {
                    self.snapshot.conversation = ConversationSnapshot::Failed {
                        message: message.clone(),
                    };
                    events.push(AppEvent::ConversationChanged {
                        correlation: None,
                        snapshot: self.snapshot.conversation.clone(),
                    });
                }
                events.extend(self.publish_startup(StartupSnapshot::Failed { message }));
                events
            }
        }
    }

    fn apply_value(&mut self, value: Value) -> Vec<AppEvent> {
        match value.get("type").and_then(Value::as_str) {
            Some("sessions")
                if self
                    .commands
                    .as_ref()
                    .is_some_and(|commands| commands.needs_thread_selection()) =>
            {
                Vec::new()
            }
            Some("sessions") => match serde_json::from_value::<SessionsState>(value) {
                Ok(state) => {
                    self.snapshot.session_catalog = session_catalog(state);
                    vec![AppEvent::SessionCatalogChanged(
                        self.snapshot.session_catalog.clone(),
                    )]
                }
                Err(error) => unreadable("sessions", &error),
            },
            Some("auto_follow") => match serde_json::from_value::<AutoFollowSettings>(value) {
                Ok(settings) => {
                    let auto_follow = &mut self.snapshot.conversation_runtime.auto_follow;
                    auto_follow.max_auto_turns = settings.max_turns;
                    auto_follow.continuation_paused = settings.paused;
                    vec![AppEvent::ConversationRuntimeAuthorityChanged(Box::new(
                        self.snapshot.conversation_runtime.clone(),
                    ))]
                }
                Err(error) => unreadable("auto_follow", &error),
            },
            Some("thread_activity") => {
                match serde_json::from_value::<Vec<ThreadActivity>>(value["threads"].clone()) {
                    Ok(threads) => vec![AppEvent::ThreadActivityChanged(
                        threads.into_iter().map(thread_activity).collect(),
                    )],
                    Err(error) => unreadable("thread_activity", &error),
                }
            }
            _ => Vec::new(),
        }
    }

    fn ready_snapshot(&self) -> StartupSnapshot {
        let ready = |detail: &str| StartupDiagnosticSnapshot {
            ok: true,
            detail: detail.to_string(),
        };
        let not_checked = || StartupDiagnosticSnapshot {
            ok: false,
            detail: "not checked by the read-only Gateway kernel client".to_string(),
        };
        StartupSnapshot::Ready(Box::new(StartupReadySnapshot {
            cwd: self
                .startup_workspace
                .clone()
                .unwrap_or_else(|| self.workspace_directory.clone()),
            workspace_path: self.workspace_directory.clone(),
            // Read-only kernel projection is usable; provider diagnostics were not performed.
            can_continue: true,
            codex_binary: not_checked(),
            workspace: ready(&self.workspace_directory),
            app_server_initialize: StartupDiagnosticSnapshot {
                ok: false,
                detail: "Gateway kernel synchronized; app-server initialize not checked"
                    .to_string(),
            },
            account: not_checked(),
            attachment: StartupAttachmentSnapshot {
                mode_label: "gateway kernel client".to_string(),
                recovery_anchor_label: "Akra Gateway".to_string(),
            },
            warnings: Vec::new(),
            schema_snapshot: String::new(),
        }))
    }

    fn publish_startup(&mut self, snapshot: StartupSnapshot) -> Vec<AppEvent> {
        let Some(workspace_directory) = self.startup_workspace.clone() else {
            return Vec::new();
        };
        self.snapshot.startup = snapshot.clone();
        vec![AppEvent::StartupChanged {
            correlation: StartupCheckCorrelation {
                generation: self.startup_generation,
                workspace_directory,
            },
            snapshot,
        }]
    }

    fn startup_requested(&mut self, workspace_directory: String) -> Vec<AppEvent> {
        self.startup_generation += 1;
        self.startup_workspace = Some(workspace_directory);
        let snapshot = if self.synchronized {
            self.ready_snapshot()
        } else {
            StartupSnapshot::Loading
        };
        self.publish_startup(snapshot)
    }
}

fn unreadable(value: &str, error: &serde_json::Error) -> Vec<AppEvent> {
    tracing::warn!(%error, value, "the kernel client could not read a kernel value");
    Vec::new()
}
