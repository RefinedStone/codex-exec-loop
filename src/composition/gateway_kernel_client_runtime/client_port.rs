use super::GatewayKernelClientRuntime;
use super::commands::Mutation;
use crate::adapter::inbound::kernel_protocol::{ApprovalDecision, KernelCommand};
use crate::core::app::{
    AppCommand, AppEvent, AppSnapshot, ConversationSnapshot, CoreInput, ParallelModeProjection,
    PlanningParallelProjection, RevisionedPlanningParallelProjection, SessionCatalogSnapshot,
};
use crate::core::native_client_port::{
    NativeClientDispatchOutcome, NativeClientEvent, NativeClientPort,
    NativeParallelDispatchOutcome, ParallelModeControlPlaneEpochSnapshot,
    ParallelModeControlPlanePresentationProjection,
};
use std::time::{Duration, Instant};

impl NativeClientPort for GatewayKernelClientRuntime {
    fn dispatch_client_event(&mut self, event: NativeClientEvent) -> NativeClientDispatchOutcome {
        let events = match event {
            NativeClientEvent::Core(input) => match *input {
                CoreInput::Command(AppCommand::RunStartupChecks {
                    workspace_directory,
                }) => self.startup_requested(workspace_directory),
                CoreInput::Command(AppCommand::LoadConversation { thread_id, .. }) => {
                    let needs_selection = self
                        .commands
                        .as_ref()
                        .is_some_and(|commands| commands.needs_thread_selection());
                    let scoped_selection = matches!(&self.snapshot.session_catalog,
                        SessionCatalogSnapshot::Ready(ready) if ready.catalog.recent_sessions()
                            .is_some_and(|sessions| sessions.items.iter().any(|item| item.id == thread_id)));
                    if needs_selection && !(self.synchronized && scoped_selection) {
                        vec![AppEvent::GatewayCommandStatus(
                            "Open Sessions after Gateway reconnect and select a thread from the scoped list".to_string(),
                        )]
                    } else {
                        self.load_conversation(thread_id)
                    }
                }
                CoreInput::Command(AppCommand::InvalidateConversationLoad) => {
                    self.conversation.cancel();
                    self.snapshot.revision = self.snapshot.revision.saturating_add(1);
                    self.snapshot.conversation = ConversationSnapshot::Idle;
                    self.refresh_conversation_activity();
                    vec![AppEvent::ConversationChanged {
                        correlation: None,
                        snapshot: ConversationSnapshot::Idle,
                    }]
                }
                CoreInput::Command(AppCommand::LoadSessionCatalog(intent)) => {
                    if self
                        .commands
                        .as_ref()
                        .is_some_and(|commands| commands.needs_thread_selection())
                    {
                        let events = self.recover_thread_catalog();
                        return self.outcome(events);
                    }
                    // Initial sync ensures a load; explicit refresh requests are coalesced.
                    if (intent.mode == crate::core::app::SessionCatalogLoadMode::Refresh
                        || matches!(self.snapshot.session_catalog, SessionCatalogSnapshot::Idle))
                        && let Some(refresh) = &self.refresh
                    {
                        let _ = refresh.try_send(());
                    }
                    vec![AppEvent::SessionCatalogChanged(
                        self.snapshot.session_catalog.clone(),
                    )]
                }
                CoreInput::Command(AppCommand::SetAutoFollowMaxTurns { value }) => {
                    self.send_mutation(KernelCommand::SetAutoFollowMaxTurns { value })
                }
                CoreInput::Command(AppCommand::PausePostTurnContinuation) => {
                    self.send_mutation(KernelCommand::PauseAutoFollow {})
                }
                CoreInput::Command(AppCommand::RequestStopAllSessions) => {
                    self.send_mutation(KernelCommand::StopAll {})
                }
                CoreInput::Command(AppCommand::CreateGatewayConversation) => self
                    .submit_gateway_write(Mutation::StartThread(self.workspace_directory.clone())),
                CoreInput::Command(AppCommand::RenameSession(request)) => {
                    self.send_mutation(KernelCommand::RenameSession {
                        thread_id: request.thread_id,
                        name: request.name,
                    })
                }
                CoreInput::Command(AppCommand::SubmitThreadPrompt(request)) => {
                    self.send_mutation(KernelCommand::SubmitPrompt {
                        thread_id: request.thread_id,
                        prompt: request.prompt,
                    })
                }
                CoreInput::Command(AppCommand::SteerTurn(request)) => {
                    self.send_mutation(KernelCommand::SteerTurn {
                        thread_id: request.thread_id,
                        expected_turn_id: request.expected_turn_id,
                        prompt: request.prompt,
                    })
                }
                CoreInput::Command(AppCommand::SubmitApprovalDecision {
                    request_identity,
                    decision,
                }) => self.send_mutation(KernelCommand::DecideApproval {
                    approval_id: request_identity.approval_id,
                    server_request_id: request_identity.server_request_id,
                    decision: match decision {
                        crate::domain::conversation::ConversationApprovalDecision::Accept => {
                            ApprovalDecision::Accept
                        }
                        crate::domain::conversation::ConversationApprovalDecision::Decline => {
                            ApprovalDecision::Decline
                        }
                    },
                }),
                // No local effect executor for operations without a Gateway mapping.
                _ => Vec::new(),
            },
            NativeClientEvent::ParallelCommand(_)
            | NativeClientEvent::ParallelTick { .. }
            | NativeClientEvent::ParallelViewChanged(_)
            | NativeClientEvent::ClearParallelDispatchWithheldReason => {
                return NativeClientDispatchOutcome::Parallel(
                    NativeParallelDispatchOutcome::from_events(Vec::new()),
                );
            }
        };
        self.outcome(events)
    }

    #[cfg(test)]
    fn poll_pending_client_event(&mut self) -> Option<NativeClientDispatchOutcome> {
        let update = self.updates.try_recv().ok()?;
        let events = self.apply(update);
        Some(self.outcome(events))
    }

    fn run_host_cycle(
        &mut self,
        _now: Instant,
        wait: Duration,
        max_inputs: usize,
    ) -> Vec<NativeClientDispatchOutcome> {
        let mut outcomes = Vec::new();
        let mut next = if wait.is_zero() {
            self.updates.try_recv().ok()
        } else {
            self.updates.recv_timeout(wait).ok()
        };
        let mut applied = 0;
        while let Some(update) = next {
            let events = self.apply(update);
            if !events.is_empty() {
                outcomes.push(self.outcome(events));
            }
            applied += 1;
            next = if applied < max_inputs {
                self.updates.try_recv().ok()
            } else {
                None
            };
        }
        outcomes
    }

    fn snapshot(&self) -> AppSnapshot {
        self.snapshot.clone()
    }

    fn revisioned_planning_parallel_projection(&self) -> RevisionedPlanningParallelProjection {
        RevisionedPlanningParallelProjection {
            revision: 0,
            planning_parallel: PlanningParallelProjection::initial(),
        }
    }

    fn parallel_mode_projection(&self) -> ParallelModeProjection {
        ParallelModeProjection::default()
    }

    fn parallel_mode_enabled(&self) -> bool {
        false
    }

    fn parallel_control_plane_projection(&self) -> ParallelModeControlPlanePresentationProjection {
        ParallelModeControlPlanePresentationProjection {
            mode_enabled: false,
            control_effect_in_flight: false,
            last_dispatch_withheld_reason: None,
            global_runtime_notices: Vec::new(),
        }
    }

    fn parallel_epoch_snapshot(&self) -> ParallelModeControlPlaneEpochSnapshot {
        ParallelModeControlPlaneEpochSnapshot {
            workspace_directory: None,
            current_epoch_id: None,
        }
    }

    fn is_gateway_kernel_client(&self) -> bool {
        true
    }

    #[cfg(test)]
    fn current_parallel_epoch_id_for_workspace(&self, _workspace_directory: &str) -> Option<u64> {
        None
    }

    #[cfg(test)]
    fn dispatch_parallel_completion_for_test(
        &mut self,
        _event: crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneBackgroundEvent,
    ) -> NativeClientDispatchOutcome {
        unreachable!("the Gateway kernel client has no parallel control plane")
    }

    #[cfg(test)]
    fn begin_test_turn_submission(&mut self) -> crate::core::app::TurnSubmissionCorrelation {
        unreachable!("the Gateway kernel client does not submit turns")
    }

    #[cfg(test)]
    fn force_parallel_mode_for_test(&self, _workspace_directory: &str, _enabled: bool) {
        unreachable!("the Gateway kernel client has no parallel control plane")
    }

    #[cfg(test)]
    fn force_parallel_epoch_for_test(&self, _workspace_directory: &str, _epoch_id: u64) {
        unreachable!("the Gateway kernel client has no parallel control plane")
    }

    #[cfg(test)]
    fn parallel_automation_epoch_is_active_for_test(
        &self,
        _workspace_directory: &str,
        _epoch_id: u64,
    ) -> bool {
        false
    }

    #[cfg(test)]
    fn recv_parallel_completion_for_test(
        &self,
        _timeout: Duration,
    ) -> crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneBackgroundEvent{
        unreachable!("the Gateway kernel client has no parallel control plane")
    }
}

impl GatewayKernelClientRuntime {
    fn recover_thread_catalog(&mut self) -> Vec<AppEvent> {
        let mut events = self.submit_gateway_write(Mutation::ListRecoveryThreads(
            self.workspace_directory.clone(),
        ));
        events.push(AppEvent::SessionCatalogChanged(
            self.snapshot.session_catalog.clone(),
        ));
        events
    }
    fn send_mutation(&mut self, command: KernelCommand) -> Vec<AppEvent> {
        self.submit_gateway_write(Mutation::Kernel(command))
    }

    fn submit_gateway_write(&mut self, command: Mutation) -> Vec<AppEvent> {
        let status = match (self.synchronized, &mut self.commands) {
            (true, Some(commands)) => commands.submit(command),
            (true, None) | (false, _) => "Gateway not synchronized; nothing was sent",
        };
        vec![AppEvent::GatewayCommandStatus(status.to_string())]
    }
}
