//! Read Codex-owned history without acquiring a writer or running a local kernel.

use std::sync::{Arc, mpsc};

use anyhow::{Context, Result, ensure};

use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
use crate::core::app::{ConversationLoadCorrelation, ConversationReadySnapshot};

use super::ClientUpdate;

pub(super) struct GatewayConversationClient {
    requests: mpsc::Sender<ConversationLoadCorrelation>,
    generation: u64,
    active: Option<ConversationLoadCorrelation>,
}

impl GatewayConversationClient {
    pub(super) fn new(
        reader: Arc<dyn InteractiveTurnRuntimePort>,
        updates: mpsc::Sender<ClientUpdate>,
    ) -> Result<Self> {
        let (requests, incoming) = mpsc::channel::<ConversationLoadCorrelation>();
        super::super::core_effect_worker::try_spawn_worker_with_panic_fallback(
            move || {
                for correlation in incoming {
                    let result = reader
                        .load_conversation_snapshot(&correlation.requested_thread_id)
                        .and_then(|snapshot| {
                            ensure!(
                                snapshot.thread_id == correlation.requested_thread_id,
                                "conversation provider returned a different thread"
                            );
                            Ok(ConversationReadySnapshot::from(snapshot))
                        })
                        .map_err(|error| {
                            tracing::warn!(
                                %error,
                                thread_id = %correlation.requested_thread_id,
                                "TUI Gateway history read failed"
                            );
                            format!("{error:#}")
                        });
                    if updates
                        .send(ClientUpdate::ConversationLoaded {
                            correlation,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            },
            || tracing::warn!("TUI Gateway history worker stopped unexpectedly"),
        )
        .context("failed to start the TUI conversation reader")?;
        Ok(Self {
            requests,
            generation: 0,
            active: None,
        })
    }

    pub(super) fn load(&mut self, thread_id: String) -> Result<ConversationLoadCorrelation> {
        self.generation = self
            .generation
            .checked_add(1)
            .context("conversation load generation exhausted")?;
        let correlation = ConversationLoadCorrelation::new(self.generation, thread_id);
        self.requests
            .send(correlation.clone())
            .context("the TUI conversation reader stopped")?;
        self.active = Some(correlation.clone());
        Ok(correlation)
    }

    pub(super) fn finish(&mut self, correlation: &ConversationLoadCorrelation) -> bool {
        if self.active.as_ref() != Some(correlation) {
            return false;
        }
        self.active = None;
        true
    }

    pub(super) fn cancel(&mut self) {
        self.active = None;
    }
}
