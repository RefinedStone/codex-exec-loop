use super::GatewayKernelClientRuntime;
use super::commands::{GatewayCommands, MutationOutcome};
use crate::core::app::{
    AppEvent, ConversationSnapshot, SessionCatalogReadySnapshot, SessionCatalogSnapshot,
};

impl GatewayKernelClientRuntime {
    pub(super) fn apply_mutation(
        &mut self,
        outcome: MutationOutcome,
        accepted_prompt: Option<(String, String)>,
    ) -> Vec<AppEvent> {
        let recovering_before = self
            .commands
            .as_ref()
            .is_some_and(GatewayCommands::needs_thread_selection);
        if let Some(commands) = &mut self.commands {
            commands.finish(&outcome);
        }
        self.snapshot.revision = self.snapshot.revision.saturating_add(1);
        let message = match outcome {
            MutationOutcome::Accepted(id) => format!("Gateway accepted operation {id}"),
            MutationOutcome::ThreadStarted(thread) => {
                self.conversation.cancel();
                self.snapshot.conversation = ConversationSnapshot::Ready(thread);
                self.refresh_conversation_activity();
                return vec![AppEvent::ConversationChanged {
                    correlation: None,
                    snapshot: self.snapshot.conversation.clone(),
                }];
            }
            MutationOutcome::RecoveryCatalog(catalog) => {
                if !self.synchronized {
                    return Vec::new();
                }
                self.snapshot.session_catalog = SessionCatalogSnapshot::Ready(
                    SessionCatalogReadySnapshot::from_catalog(catalog),
                );
                return vec![AppEvent::SessionCatalogChanged(self.snapshot.session_catalog.clone())];
            }
            MutationOutcome::RecoveryUnavailable => {
                "Thread creation outcome unknown. Could not list scoped sessions; reopen Sessions to try the read again.".to_string()
            }
            MutationOutcome::Refused(reason) => reason,
            MutationOutcome::Unknown => {
                if !recovering_before
                    && self.commands.as_ref().is_some_and(GatewayCommands::needs_thread_selection)
                {
                    self.snapshot.session_catalog = SessionCatalogSnapshot::Idle;
                    self.conversation.cancel();
                    return vec![
                        AppEvent::SessionCatalogChanged(self.snapshot.session_catalog.clone()),
                        AppEvent::GatewayCommandStatus("Creation outcome unknown; Ctrl+O to select".to_string()),
                    ];
                }
                "Gateway command outcome unknown; refresh before trying again. It was not replayed".to_string()
            }
        };
        let mut events = vec![AppEvent::GatewayCommandStatus(message)];
        if let Some((thread_id, prompt)) = accepted_prompt {
            events.push(AppEvent::GatewayPromptAccepted { thread_id, prompt });
        }
        events
    }
}
