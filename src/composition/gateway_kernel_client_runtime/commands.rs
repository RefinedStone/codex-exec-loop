//! Single-flight mutations of the Gateway-owned kernel. A transport failure never replays.

use std::sync::{Arc, mpsc};

use anyhow::{Context, Result};

use super::ClientUpdate;
use crate::adapter::inbound::kernel_protocol::{
    CommandRequest, CommandResponse, KernelCommand, OperationAdmission,
};
use crate::adapter::outbound::app_server::CodexAppServerAdapter;
use crate::application::port::outbound::kernel_gateway_port::{
    KernelGatewayPort, KernelGatewayReply,
};
use crate::core::app::ConversationReadySnapshot;

pub(super) struct GatewayCommands {
    requests: mpsc::Sender<Mutation>,
    pending: Option<Mutation>,
    uncertain: Vec<KernelCommand>,
    creation_unknown: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum Mutation {
    Kernel(KernelCommand),
    StartThread(String),
    ListRecoveryThreads(String),
}

pub(crate) enum MutationOutcome {
    Accepted(i64),
    ThreadStarted(Box<ConversationReadySnapshot>),
    RecoveryCatalog(crate::domain::recent_sessions::SessionCatalog),
    RecoveryUnavailable,
    Refused(String),
    Unknown,
}

impl GatewayCommands {
    pub(super) fn new(
        gateway: Arc<dyn KernelGatewayPort>,
        relay: Option<Arc<CodexAppServerAdapter>>,
        updates: mpsc::Sender<ClientUpdate>,
    ) -> Result<Self> {
        let (requests, incoming) = mpsc::channel::<Mutation>();
        let stopped = updates.clone();
        super::super::core_effect_worker::try_spawn_worker_with_panic_fallback(
            move || {
                for command in incoming {
                    let (outcome, accepted_prompt) = match command {
                        Mutation::Kernel(command) => {
                            let outcome = send_once(gateway.as_ref(), command.clone());
                            let accepted = match (&outcome, command) {
                                (MutationOutcome::Accepted(_), KernelCommand::SubmitPrompt { thread_id, prompt }) => Some((thread_id, prompt)),
                                _ => None,
                            };
                            (outcome, accepted)
                        }
                        Mutation::StartThread(cwd) => {
                            let outcome = match &relay {
                                Some(relay) => match relay.start_gateway_thread_once(&cwd) {
                                    Ok(thread) => MutationOutcome::ThreadStarted(Box::new(thread.into())),
                                    Err(error) => {
                                        tracing::warn!(%error, "TUI Gateway thread creation reply unavailable");
                                        MutationOutcome::Unknown
                                    }
                                },
                                None => MutationOutcome::Refused("No Gateway relay is available".to_string()),
                            };
                            (outcome, None)
                        }
                        Mutation::ListRecoveryThreads(cwd) => {
                            let outcome = match &relay {
                                Some(relay) => match relay.list_gateway_threads_for_recovery(&cwd) {
                                    Ok(catalog) => MutationOutcome::RecoveryCatalog(catalog),
                                    Err(error) => {
                                        tracing::warn!(%error, "TUI Gateway recovery list unavailable");
                                        MutationOutcome::RecoveryUnavailable
                                    }
                                },
                                None => MutationOutcome::RecoveryUnavailable,
                            };
                            (outcome, None)
                        }
                    };
                    if updates
                        .send(ClientUpdate::MutationFinished { outcome, accepted_prompt })
                        .is_err()
                    {
                        break;
                    }
                }
            },
            move || {
                tracing::warn!("TUI Gateway command worker stopped unexpectedly");
                if stopped
                    .send(ClientUpdate::MutationFinished {
                        outcome: MutationOutcome::Unknown,
                        accepted_prompt: None,
                    })
                    .is_err()
                {
                    tracing::debug!("TUI closed before the Gateway command worker stopped");
                }
            },
        )
        .context("start the TUI Gateway command worker")?;
        Ok(Self {
            requests,
            pending: None,
            uncertain: Vec::new(),
            creation_unknown: false,
        })
    }

    pub(super) fn submit(&mut self, command: Mutation) -> &'static str {
        if self.pending.is_some() {
            return "A Gateway command is already awaiting a reply";
        }
        if self.creation_unknown && matches!(command, Mutation::StartThread(_)) {
            return "Creation still unknown; Ctrl+O to select";
        }
        if let Mutation::Kernel(command) = &command {
            if self.uncertain.contains(command) {
                return "Unknown outcome: command not sent again";
            }
            if self.uncertain.len() >= 16 {
                return "Too many Gateway commands have unknown outcomes; review them before restarting the TUI";
            }
        }
        if self.requests.send(command.clone()).is_err() {
            return "The Gateway command worker is unavailable; nothing was sent";
        }
        self.pending = Some(command);
        "Waiting for Gateway command acknowledgment"
    }

    pub(super) fn finish(&mut self, outcome: &MutationOutcome) {
        let pending = self.pending.take();
        if matches!(outcome, MutationOutcome::Unknown) {
            match pending {
                Some(Mutation::StartThread(_)) => self.creation_unknown = true,
                Some(Mutation::Kernel(command)) => self.uncertain.push(command),
                Some(Mutation::ListRecoveryThreads(_)) | None => {}
            }
        }
    }

    pub(super) fn needs_thread_selection(&self) -> bool {
        self.creation_unknown
    }

    pub(super) fn confirm_thread_selection(&mut self) {
        self.creation_unknown = false;
    }
}

fn send_once(gateway: &dyn KernelGatewayPort, command: KernelCommand) -> MutationOutcome {
    let request = CommandRequest {
        command,
        idempotency_key: Some(format!("{:032x}", rand::random::<u128>())),
    };
    let params = match serde_json::to_value(request) {
        Ok(params) => params,
        Err(error) => return MutationOutcome::Refused(format!("Invalid Gateway command: {error}")),
    };
    match gateway.request("akra/kernel/command", params, false) {
        Ok(KernelGatewayReply::NoGateway) => {
            MutationOutcome::Refused("Gateway unavailable; nothing was sent".to_string())
        }
        Ok(KernelGatewayReply::KernelHeldElsewhere) => {
            MutationOutcome::Refused("Gateway does not own this repository kernel".to_string())
        }
        Ok(KernelGatewayReply::GatewayTooOld) => {
            MutationOutcome::Refused("Gateway does not support this command".to_string())
        }
        Ok(KernelGatewayReply::Answer(value)) => {
            match serde_json::from_value::<CommandResponse>(value) {
                Ok(response) => {
                    if let Some(rejection) = response.rejection {
                        return MutationOutcome::Refused(rejection.message);
                    }
                    if let Some(failure) = response.failure {
                        return MutationOutcome::Refused(failure);
                    }
                    match response.operation {
                        Some(operation)
                            if response.processed
                                && operation.state == OperationAdmission::Accepted =>
                        {
                            MutationOutcome::Accepted(operation.op_id)
                        }
                        Some(_) | None => MutationOutcome::Unknown,
                    }
                }
                Err(_) => MutationOutcome::Unknown,
            }
        }
        Err(error) => {
            tracing::warn!(%error, "TUI Gateway command reply unavailable");
            MutationOutcome::Unknown
        }
    }
}
