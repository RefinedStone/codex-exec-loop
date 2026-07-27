use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
#[cfg(test)]
use std::time::Duration;
use std::time::Instant;

use crate::application::service::conversation_service::ConversationService;
#[cfg(test)]
use crate::application::service::github_review_poller_service::GithubReviewPollerService;
use crate::application::service::parallel_mode::control_plane::{
    ParallelModeControlPlaneBackgroundEvent, ParallelModeControlPlaneCommand,
    ParallelModeControlPlaneComposition, ParallelModeControlPlaneEpochSnapshot,
    ParallelModeControlPlaneEventSink, ParallelModeControlPlaneHandle,
    ParallelModeControlPlanePresentationEvent, ParallelModeControlPlanePresentationProjection,
};
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::planning::PlanningServices;
use crate::application::service::post_turn_evaluation::PostTurnEvaluationService;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::composition::core_effect_runner::CoreEffectRunner;
use crate::core::app::{
    AppCommand, AppEvent, AppSnapshot, CoreDispatchOutcome, CoreInput, ParallelModeProjection,
    PostTurnAuthoritySnapshot, PostTurnEvaluationCorrelation, PostTurnRouteResolution,
    RevisionedPlanningParallelProjection,
};
#[cfg(test)]
use crate::core::app::{GithubReviewPollingSetupRequest, TurnSubmissionCorrelation};
use crate::core::runtime::{CoreRuntime, core_input_channel};
use crate::domain::conversation::ConversationRuntimeControlTruth;
#[cfg(test)]
use crate::domain::github_review::GithubPullRequestTarget;
#[cfg(test)]
use crate::domain::parallel_mode::ParallelModeAutomationTrigger;
use crate::domain::planning::{PostTurnContinuationAction, PostTurnExecution};

/*
 * NativeClientRuntime is the composition-owned client runtime boundary used by
 * the native shell. It privately owns both the framework-free Core driver and
 * the application-owned parallel control-plane handle. Inbound adapters
 * dispatch one typed ClientEvent contract and read immutable projections;
 * construction of either effect path stays private to composition.
 */
pub(crate) struct NativeClientRuntime {
    core: NativeCoreRuntime,
    parallel_control_plane: ParallelModeControlPlaneHandle<NativeParallelModeControlPlaneEventSink>,
    parallel_completion_rx: Receiver<ParallelModeControlPlaneBackgroundEvent>,
    next_poll_lane: NativeClientPollLane,
}

struct NativeCoreRuntime {
    runtime: CoreRuntime<CoreEffectRunner>,
}

/*
 * Production service wiring is consumed here, before the inbound TUI boundary.
 * The TUI receives only the runtime facade and immutable capability truth.
 * Control-plane handle, event sink, and completion mailbox ownership stay
 * private to this composition boundary.
 */
pub(crate) struct NativeTuiApplicationComposition {
    core_runtime: NativeCoreRuntime,
    parallel_control_plane: ParallelModeControlPlaneComposition,
    turn_control_truth: ConversationRuntimeControlTruth,
}

pub(crate) struct BoundNativeTuiApplication {
    client_runtime: NativeClientRuntime,
    turn_control_truth: ConversationRuntimeControlTruth,
}

#[derive(Clone)]
struct NativeParallelModeControlPlaneEventSink {
    tx: SyncSender<ParallelModeControlPlaneBackgroundEvent>,
}

impl ParallelModeControlPlaneEventSink for NativeParallelModeControlPlaneEventSink {
    fn send_control_plane_event(&self, event: ParallelModeControlPlaneBackgroundEvent) {
        let _ = self.tx.send(event);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeClientPollLane {
    Core,
    Parallel,
}

pub(crate) enum NativeClientEvent {
    Core(Box<CoreInput>),
    ParallelCommand(ParallelModeControlPlaneCommand),
    ParallelTick {
        now: Instant,
        workspace_directory: String,
        activity_pulse_visible: bool,
    },
    ClearParallelDispatchWithheldReason,
}

impl NativeClientEvent {
    pub(crate) fn core(input: CoreInput) -> Self {
        Self::Core(Box::new(input))
    }
}

pub(crate) enum NativeClientDispatchOutcome {
    Core(CoreDispatchOutcome),
    Parallel(NativeParallelDispatchOutcome),
    Combined {
        core: CoreDispatchOutcome,
        parallel: NativeParallelDispatchOutcome,
    },
}

pub(crate) struct NativeParallelDispatchOutcome {
    pub(crate) presentation_events: Vec<ParallelModeControlPlanePresentationEvent>,
}

struct InternalPostTurnRoutingRequest {
    event_index: usize,
    correlation: PostTurnEvaluationCorrelation,
    execution: Box<PostTurnExecution>,
}

impl NativeParallelDispatchOutcome {
    fn from_events(presentation_events: Vec<ParallelModeControlPlanePresentationEvent>) -> Self {
        Self {
            presentation_events,
        }
    }
}

fn take_internal_post_turn_routing_requests(
    outcome: &mut CoreDispatchOutcome,
) -> Vec<InternalPostTurnRoutingRequest> {
    let mut routing_requests = Vec::<InternalPostTurnRoutingRequest>::new();
    let mut external_events = Vec::with_capacity(outcome.events.len());
    for event in std::mem::take(&mut outcome.events) {
        match event {
            AppEvent::PostTurnContinuationRoutingRequested {
                correlation,
                execution,
            } => {
                routing_requests.push(InternalPostTurnRoutingRequest {
                    event_index: external_events.len(),
                    correlation,
                    execution,
                });
            }
            external => external_events.push(external),
        }
    }
    outcome.events = external_events;
    routing_requests
}

fn merge_core_dispatch_outcome_at(
    target: &mut CoreDispatchOutcome,
    resolved: CoreDispatchOutcome,
    event_index: usize,
) {
    let event_index = event_index.min(target.events.len());
    target
        .events
        .splice(event_index..event_index, resolved.events);
    target.effects.extend(resolved.effects);
    target.snapshot = resolved.snapshot;
}

fn pending_post_turn_route_correlation(
    snapshot: &AppSnapshot,
) -> Option<&PostTurnEvaluationCorrelation> {
    match &snapshot.conversation_runtime.post_turn {
        PostTurnAuthoritySnapshot::AwaitingRoute { correlation, .. } => Some(correlation),
        PostTurnAuthoritySnapshot::Idle
        | PostTurnAuthoritySnapshot::Evaluating { .. }
        | PostTurnAuthoritySnapshot::Settled { .. } => None,
    }
}

fn exact_single_post_turn_routing_request<'a>(
    snapshot: &AppSnapshot,
    routing_requests: &'a [InternalPostTurnRoutingRequest],
) -> Option<&'a InternalPostTurnRoutingRequest> {
    let [request] = routing_requests else {
        return None;
    };
    (pending_post_turn_route_correlation(snapshot) == Some(&request.correlation)).then_some(request)
}

fn route_parallel_post_turn_for_exact_single<T>(
    snapshot: &AppSnapshot,
    routing_requests: &[InternalPostTurnRoutingRequest],
    route: impl FnOnce(&InternalPostTurnRoutingRequest) -> T,
) -> Option<T> {
    let request = exact_single_post_turn_routing_request(snapshot, routing_requests)?;
    snapshot
        .conversation_runtime
        .auto_follow
        .parallel_post_turn_continuation_allowed()
        .then(|| route(request))
}

fn fail_closed_post_turn_settlement(
    snapshot: &AppSnapshot,
    routing_requests: &[InternalPostTurnRoutingRequest],
) -> Option<(usize, PostTurnEvaluationCorrelation)> {
    if routing_requests.len() <= 1 {
        return None;
    }
    let current = pending_post_turn_route_correlation(snapshot)?;
    routing_requests
        .iter()
        .find(|request| &request.correlation == current)
        .map(|request| (request.event_index, current.clone()))
}

const fn post_turn_route_resolution(
    auto_follow_prompt_queued: bool,
    parallel_consumed_prompt: bool,
    single_session_auto_follow_allowed: bool,
) -> PostTurnRouteResolution {
    if parallel_consumed_prompt {
        return PostTurnRouteResolution::ParallelConsumed;
    }
    if auto_follow_prompt_queued && single_session_auto_follow_allowed {
        return PostTurnRouteResolution::AutoSubmit;
    }
    PostTurnRouteResolution::NoContinuation
}

impl NativeTuiApplicationComposition {
    pub(in crate::composition) fn from_services(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        parallel_control_plane: ParallelModeControlPlaneComposition,
    ) -> Self {
        let turn_control_truth = conversation_service.runtime_control_truth();
        let planning_feature = parallel_control_plane.planning().clone();
        let parallel_mode_turn_service = parallel_control_plane.parallel_mode_turn_service();
        let core_runtime = NativeCoreRuntime::new(
            startup_service,
            session_service,
            conversation_service,
            planning_feature,
            parallel_mode_turn_service,
        );
        Self {
            core_runtime,
            parallel_control_plane,
            turn_control_truth,
        }
    }

    pub(crate) fn bind_client_runtime(self) -> BoundNativeTuiApplication {
        BoundNativeTuiApplication {
            client_runtime: NativeClientRuntime::from_parts(
                self.core_runtime,
                self.parallel_control_plane,
            ),
            turn_control_truth: self.turn_control_truth,
        }
    }
}

impl BoundNativeTuiApplication {
    pub(crate) fn into_parts(self) -> (NativeClientRuntime, ConversationRuntimeControlTruth) {
        (self.client_runtime, self.turn_control_truth)
    }
}

impl NativeCoreRuntime {
    fn new(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        planning_feature: PlanningServices,
        parallel_mode_turn_service: ParallelModeTurnService,
    ) -> Self {
        Self::new_with_effect_runner_configuration(
            startup_service,
            session_service,
            conversation_service,
            planning_feature,
            parallel_mode_turn_service,
            |runner| runner,
        )
    }

    fn new_with_effect_runner_configuration(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        planning_feature: PlanningServices,
        parallel_mode_turn_service: ParallelModeTurnService,
        configure_effect_runner: impl FnOnce(CoreEffectRunner) -> CoreEffectRunner,
    ) -> Self {
        let (input_sender, input_receiver) = core_input_channel();
        let post_turn_evaluation_service = PostTurnEvaluationService::new(
            planning_feature.clone(),
            parallel_mode_turn_service.clone(),
        );
        let effect_runner = configure_effect_runner(CoreEffectRunner::new(
            startup_service,
            session_service,
            conversation_service,
            planning_feature,
            parallel_mode_turn_service,
            post_turn_evaluation_service,
            input_sender,
        ));

        Self {
            runtime: CoreRuntime::new(effect_runner, input_receiver),
        }
    }
}

impl NativeClientRuntime {
    const PARALLEL_COMPLETION_CAPACITY: usize = 256;

    fn from_parts(
        core: NativeCoreRuntime,
        parallel_control_plane: ParallelModeControlPlaneComposition,
    ) -> Self {
        let (parallel_completion_tx, parallel_completion_rx) =
            mpsc::sync_channel(Self::PARALLEL_COMPLETION_CAPACITY);
        let parallel_control_plane =
            parallel_control_plane.bind_event_sink(NativeParallelModeControlPlaneEventSink {
                tx: parallel_completion_tx,
            });
        Self {
            core,
            parallel_control_plane,
            parallel_completion_rx,
            next_poll_lane: NativeClientPollLane::Core,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        planning_feature: PlanningServices,
        parallel_mode_turn_service: ParallelModeTurnService,
        parallel_control_plane: ParallelModeControlPlaneComposition,
    ) -> Self {
        Self::from_parts(
            NativeCoreRuntime::new(
                startup_service,
                session_service,
                conversation_service,
                planning_feature,
                parallel_mode_turn_service,
            ),
            parallel_control_plane,
        )
    }

    #[cfg(test)]
    pub(crate) fn new_with_github_review_polling_setup_loader(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        planning_feature: PlanningServices,
        parallel_mode_turn_service: ParallelModeTurnService,
        parallel_control_plane: ParallelModeControlPlaneComposition,
        loader: impl Fn(
            &GithubReviewPollingSetupRequest,
        )
            -> anyhow::Result<Option<(GithubPullRequestTarget, GithubReviewPollerService)>>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self::from_parts(
            NativeCoreRuntime::new_with_effect_runner_configuration(
                startup_service,
                session_service,
                conversation_service,
                planning_feature,
                parallel_mode_turn_service,
                |runner| runner.with_github_review_polling_setup_loader(loader),
            ),
            parallel_control_plane,
        )
    }

    #[must_use = "dispatch outcomes carry projection events that the inbound adapter must apply"]
    pub(crate) fn dispatch_client_event(
        &mut self,
        event: NativeClientEvent,
    ) -> NativeClientDispatchOutcome {
        match event {
            NativeClientEvent::Core(input) => self.dispatch_core_input(*input),
            NativeClientEvent::ParallelCommand(command) => {
                NativeClientDispatchOutcome::Parallel(NativeParallelDispatchOutcome::from_events(
                    self.parallel_control_plane.handle_command(command),
                ))
            }
            NativeClientEvent::ParallelTick {
                now,
                workspace_directory,
                activity_pulse_visible,
            } => NativeClientDispatchOutcome::Parallel(NativeParallelDispatchOutcome::from_events(
                self.parallel_control_plane
                    .tick(now, workspace_directory, activity_pulse_visible),
            )),
            NativeClientEvent::ClearParallelDispatchWithheldReason => {
                self.parallel_control_plane.clear_dispatch_withheld_reason();
                NativeClientDispatchOutcome::Parallel(NativeParallelDispatchOutcome::from_events(
                    Vec::new(),
                ))
            }
        }
    }

    pub(crate) fn poll_pending_client_event(&mut self) -> Option<NativeClientDispatchOutcome> {
        let lanes = match self.next_poll_lane {
            NativeClientPollLane::Core => {
                [NativeClientPollLane::Core, NativeClientPollLane::Parallel]
            }
            NativeClientPollLane::Parallel => {
                [NativeClientPollLane::Parallel, NativeClientPollLane::Core]
            }
        };
        for lane in lanes {
            let outcome = match lane {
                NativeClientPollLane::Core => self
                    .core
                    .runtime
                    .poll_pending_input()
                    .map(|outcome| self.resolve_internal_post_turn_routes(outcome)),
                NativeClientPollLane::Parallel => match self.parallel_completion_rx.try_recv() {
                    Ok(completion) => Some(self.dispatch_parallel_completion(completion)),
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
                },
            };
            if let Some(outcome) = outcome {
                self.next_poll_lane = match lane {
                    NativeClientPollLane::Core => NativeClientPollLane::Parallel,
                    NativeClientPollLane::Parallel => NativeClientPollLane::Core,
                };
                return Some(outcome);
            }
        }
        None
    }

    fn dispatch_core_input(&mut self, input: CoreInput) -> NativeClientDispatchOutcome {
        let outcome = self.core.runtime.dispatch_input(input);
        self.resolve_internal_post_turn_routes(outcome)
    }

    fn resolve_internal_post_turn_routes(
        &mut self,
        mut core_outcome: CoreDispatchOutcome,
    ) -> NativeClientDispatchOutcome {
        let routing_requests = take_internal_post_turn_routing_requests(&mut core_outcome);
        if routing_requests.is_empty() {
            return NativeClientDispatchOutcome::Core(core_outcome);
        }

        if let Some((event_index, correlation)) =
            fail_closed_post_turn_settlement(core_outcome.snapshot.as_ref(), &routing_requests)
        {
            let mut resolved =
                self.core
                    .runtime
                    .dispatch_command(AppCommand::ResolvePostTurnContinuation {
                        correlation,
                        resolution: PostTurnRouteResolution::NoContinuation,
                    });
            let _ = take_internal_post_turn_routing_requests(&mut resolved);
            merge_core_dispatch_outcome_at(&mut core_outcome, resolved, event_index);
            return NativeClientDispatchOutcome::Core(core_outcome);
        }

        let Some(request) = exact_single_post_turn_routing_request(
            core_outcome.snapshot.as_ref(),
            &routing_requests,
        ) else {
            return NativeClientDispatchOutcome::Core(core_outcome);
        };
        let auto_follow_prompt_queued = matches!(
            &request.execution.evaluation.action,
            PostTurnContinuationAction::QueueAutoPrompt(_)
        );
        let parallel_outcome = route_parallel_post_turn_for_exact_single(
            core_outcome.snapshot.as_ref(),
            &routing_requests,
            |request| {
                self.parallel_control_plane.continue_post_turn_queue(
                    request
                        .execution
                        .runtime_projection_workspace_directory
                        .clone(),
                    request
                        .execution
                        .evaluation
                        .provenance
                        .parallel_queue_signal,
                    auto_follow_prompt_queued,
                    request
                        .execution
                        .evaluation
                        .runtime_projection
                        .has_actionable_queue_head(),
                )
            },
        );
        let parallel_consumed_prompt = parallel_outcome
            .as_ref()
            .is_some_and(|outcome| outcome.auto_follow_prompt_consumed);
        let presentation_events = parallel_outcome
            .map(|outcome| outcome.presentation_events)
            .unwrap_or_default();

        let resolution = post_turn_route_resolution(
            auto_follow_prompt_queued,
            parallel_consumed_prompt,
            core_outcome
                .snapshot
                .conversation_runtime
                .auto_follow
                .can_queue_next(),
        );
        let event_index = request.event_index;
        let correlation = request.correlation.clone();
        let mut resolved =
            self.core
                .runtime
                .dispatch_command(AppCommand::ResolvePostTurnContinuation {
                    correlation,
                    resolution,
                });
        let _ = take_internal_post_turn_routing_requests(&mut resolved);
        merge_core_dispatch_outcome_at(&mut core_outcome, resolved, event_index);

        NativeClientDispatchOutcome::Combined {
            core: core_outcome,
            parallel: NativeParallelDispatchOutcome {
                presentation_events,
            },
        }
    }

    fn dispatch_parallel_completion(
        &mut self,
        completion: ParallelModeControlPlaneBackgroundEvent,
    ) -> NativeClientDispatchOutcome {
        NativeClientDispatchOutcome::Parallel(NativeParallelDispatchOutcome::from_events(
            self.parallel_control_plane
                .handle_background_event(completion),
        ))
    }

    #[cfg(test)]
    pub(crate) fn dispatch_parallel_completion_for_test(
        &mut self,
        completion: ParallelModeControlPlaneBackgroundEvent,
    ) -> NativeClientDispatchOutcome {
        self.dispatch_parallel_completion(completion)
    }

    pub(crate) fn snapshot(&self) -> AppSnapshot {
        self.core.runtime.snapshot()
    }

    pub(crate) fn revisioned_planning_parallel_projection(
        &self,
    ) -> RevisionedPlanningParallelProjection {
        self.core.runtime.revisioned_planning_parallel_projection()
    }

    pub(crate) fn parallel_mode_projection(&self) -> ParallelModeProjection {
        self.core.runtime.parallel_mode_projection()
    }

    pub(crate) fn parallel_mode_enabled(&self) -> bool {
        self.parallel_control_plane.mode_enabled()
    }

    pub(crate) fn parallel_control_plane_projection(
        &self,
    ) -> ParallelModeControlPlanePresentationProjection {
        self.parallel_control_plane.presentation_projection()
    }

    pub(crate) fn parallel_epoch_snapshot(&self) -> ParallelModeControlPlaneEpochSnapshot {
        self.parallel_control_plane.epoch_snapshot()
    }

    pub(crate) fn current_parallel_epoch_id_for_workspace(
        &self,
        workspace_directory: &str,
    ) -> Option<u64> {
        self.parallel_control_plane
            .current_epoch_id_for_workspace(workspace_directory)
    }

    #[cfg(test)]
    pub(crate) fn last_parallel_automation_trigger(&self) -> Option<ParallelModeAutomationTrigger> {
        self.parallel_control_plane.last_automation_trigger()
    }

    #[cfg(test)]
    pub(crate) fn begin_test_turn_submission(&mut self) -> TurnSubmissionCorrelation {
        self.core.runtime.begin_test_turn_submission()
    }

    #[cfg(test)]
    pub(crate) fn begin_test_post_turn_evaluation(
        &mut self,
        thread_id: &str,
        completed_turn_id: &str,
        turn_workspace_directory: &str,
        planning_workspace_directory: &str,
    ) -> PostTurnEvaluationCorrelation {
        self.core.runtime.begin_test_post_turn_evaluation(
            thread_id,
            completed_turn_id,
            turn_workspace_directory,
            planning_workspace_directory,
        )
    }

    #[cfg(test)]
    pub(crate) fn test_post_turn_evaluation_is_in_flight(
        &self,
        thread_id: &str,
        completed_turn_id: &str,
    ) -> bool {
        self.core
            .runtime
            .test_post_turn_evaluation_is_in_flight(thread_id, completed_turn_id)
    }

    #[cfg(test)]
    pub(crate) fn force_parallel_mode_for_test(
        &self,
        workspace_directory: impl Into<String>,
        enabled: bool,
    ) {
        self.parallel_control_plane
            .force_mode_for_test(workspace_directory, enabled);
    }

    #[cfg(test)]
    pub(crate) fn force_parallel_initial_pool_reset_completed_for_test(&self, completed: bool) {
        self.parallel_control_plane
            .force_initial_pool_reset_completed_for_test(completed);
    }

    #[cfg(test)]
    pub(crate) fn force_parallel_epoch_for_test(
        &self,
        workspace_directory: impl Into<String>,
        epoch_id: u64,
    ) {
        self.parallel_control_plane
            .force_epoch_for_test(workspace_directory, epoch_id);
    }

    #[cfg(test)]
    pub(crate) fn parallel_automation_epoch_is_active_for_test(
        &self,
        workspace_directory: &str,
        epoch_id: u64,
    ) -> bool {
        self.parallel_control_plane
            .automation_epoch_is_active(workspace_directory, epoch_id)
    }

    #[cfg(test)]
    pub(crate) fn recv_parallel_completion_for_test(
        &self,
        timeout: Duration,
    ) -> ParallelModeControlPlaneBackgroundEvent {
        self.parallel_completion_rx
            .recv_timeout(timeout)
            .expect("parallel completion should return through the private client-runtime mailbox")
    }

    #[cfg(test)]
    pub(crate) fn force_parallel_supervisor_refresh_in_flight_for_test(
        &self,
        workspace_directory: impl Into<String>,
        epoch_id: u64,
    ) -> crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneEffectId
    {
        self.parallel_control_plane
            .force_supervisor_refresh_in_flight_for_test(workspace_directory, epoch_id)
    }

    #[cfg(test)]
    pub(crate) fn parallel_control_effect_in_flight_for_test(&self) -> bool {
        self.parallel_control_plane.control_effect_in_flight()
    }

    #[cfg(test)]
    pub(crate) fn parallel_supervisor_refresh_in_flight_for_test(&self) -> bool {
        self.parallel_control_plane.supervisor_refresh_in_flight()
    }

    #[cfg(test)]
    pub(crate) fn parallel_orchestrator_wake_in_flight_for_test(&self) -> bool {
        self.parallel_control_plane.orchestrator_wake_in_flight()
    }

    #[cfg(test)]
    pub(crate) fn parallel_supervisor_refresh_due_for_test(
        &self,
        now: Instant,
        activity_pulse_visible: bool,
    ) -> bool {
        self.parallel_control_plane
            .supervisor_refresh_due(now, activity_pulse_visible)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::Arc;

    use super::*;
    use crate::adapter::inbound::tui::app::test_helpers::{
        test_parallel_mode_control_plane_composition, test_planning_services,
    };
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::core::app::{
        CoreEffect, CoreEffectCompletion, ParallelPeekLoadCorrelation,
        PlanningRuntimeRefreshCorrelation, TurnStreamEvent,
    };
    use crate::domain::conversation::{
        ConversationRuntimeControlTruth, ConversationSnapshot, ConversationTurnOptions,
    };
    use crate::domain::conversation_runtime_envelope::{
        ConversationRuntimeConfigurationRequest, ConversationRuntimeEnvelope,
    };
    use crate::domain::planning::{
        PlanningWorkerPanelState, PostTurnOutcome, PostTurnProvenance, PostTurnQueuedPrompt,
        RuntimeProjection, TurnSnapshotCapture,
    };
    use crate::domain::recent_sessions::{SessionCatalog, SessionCatalogRequest};
    use crate::domain::turn_terminal::{
        ConversationTurnApplicationDelivery, ConversationTurnTerminalReceipt,
    };

    #[test]
    fn route_resolution_prioritizes_parallel_consumption_then_exact_auto_submit_gate() {
        assert_eq!(
            post_turn_route_resolution(true, true, true),
            PostTurnRouteResolution::ParallelConsumed
        );
        assert_eq!(
            post_turn_route_resolution(true, false, true),
            PostTurnRouteResolution::AutoSubmit
        );
        assert_eq!(
            post_turn_route_resolution(true, false, false),
            PostTurnRouteResolution::NoContinuation
        );
        assert_eq!(
            post_turn_route_resolution(false, false, true),
            PostTurnRouteResolution::NoContinuation
        );
    }

    #[test]
    fn parallel_post_turn_route_requires_exact_awaiting_authority_and_policy_gate() {
        let correlation = post_turn_correlation(7);
        let requests = vec![internal_routing_request(&correlation, 0)];
        let mut snapshot = awaiting_route_snapshot(&correlation);
        let route_calls = Cell::new(0);

        snapshot
            .conversation_runtime
            .auto_follow
            .continuation_paused = true;
        let blocked = route_parallel_post_turn_for_exact_single(&snapshot, &requests, |_| {
            route_calls.set(route_calls.get() + 1);
        });
        assert!(blocked.is_none());
        assert_eq!(route_calls.get(), 0);

        snapshot
            .conversation_runtime
            .auto_follow
            .parallel_rearmed_after_stop = true;
        let allowed = route_parallel_post_turn_for_exact_single(&snapshot, &requests, |_| {
            route_calls.set(route_calls.get() + 1);
        });
        assert_eq!(allowed, Some(()));
        assert_eq!(route_calls.get(), 1);
    }

    #[test]
    fn stale_and_aba_route_markers_cannot_enter_the_parallel_side_effect() {
        let correlation = post_turn_correlation(7);
        let current = post_turn_correlation(8);
        let requests = vec![internal_routing_request(&correlation, 0)];
        let snapshot = awaiting_route_snapshot(&current);
        let route_calls = Cell::new(0);

        let routed = route_parallel_post_turn_for_exact_single(&snapshot, &requests, |_| {
            route_calls.set(route_calls.get() + 1);
        });

        assert!(routed.is_none());
        assert_eq!(route_calls.get(), 0);
        assert!(
            fail_closed_post_turn_settlement(&snapshot, &requests).is_none(),
            "a stale-only marker must not settle an unrelated current route"
        );
    }

    #[test]
    fn duplicate_route_markers_fail_closed_without_parallel_side_effect() {
        let correlation = post_turn_correlation(7);
        let requests = vec![
            internal_routing_request(&correlation, 0),
            internal_routing_request(&correlation, 1),
        ];
        let snapshot = awaiting_route_snapshot(&correlation);
        let route_calls = Cell::new(0);

        let routed = route_parallel_post_turn_for_exact_single(&snapshot, &requests, |_| {
            route_calls.set(route_calls.get() + 1);
        });

        assert!(routed.is_none());
        assert_eq!(route_calls.get(), 0);
        assert_eq!(
            fail_closed_post_turn_settlement(&snapshot, &requests),
            Some((0, correlation))
        );
    }

    #[test]
    fn multi_route_batch_settles_only_the_current_route_without_parallel_side_effect() {
        let stale = post_turn_correlation(7);
        let current = post_turn_correlation(8);
        let requests = vec![
            internal_routing_request(&stale, 0),
            internal_routing_request(&current, 2),
        ];
        let snapshot = awaiting_route_snapshot(&current);
        let route_calls = Cell::new(0);

        let routed = route_parallel_post_turn_for_exact_single(&snapshot, &requests, |_| {
            route_calls.set(route_calls.get() + 1);
        });

        assert!(routed.is_none());
        assert_eq!(route_calls.get(), 0);
        assert_eq!(
            fail_closed_post_turn_settlement(&snapshot, &requests),
            Some((2, current))
        );
    }

    #[test]
    fn routing_marker_extraction_preserves_fifo_positions_and_never_leaks_markers() {
        let first_correlation = post_turn_correlation(7);
        let second_correlation = post_turn_correlation(8);
        let mut outcome = CoreDispatchOutcome {
            events: vec![
                AppEvent::ParallelModeSupervisorSnapshotInvalidated,
                routing_event(&first_correlation),
                AppEvent::SnapshotChanged(Arc::new(AppSnapshot::initial())),
                AppEvent::PostTurnContinuationRoutingRequested {
                    correlation: second_correlation.clone(),
                    execution: Box::new(post_turn_execution(&second_correlation)),
                },
            ],
            effects: Vec::new(),
            snapshot: Arc::new(AppSnapshot::initial()),
        };

        let routing_requests = take_internal_post_turn_routing_requests(&mut outcome);

        assert_eq!(routing_requests.len(), 2);
        assert_eq!(routing_requests[0].correlation, first_correlation);
        assert_eq!(routing_requests[0].event_index, 1);
        assert_eq!(routing_requests[1].correlation, second_correlation);
        assert_eq!(routing_requests[1].event_index, 2);
        assert!(matches!(
            outcome.events.as_slice(),
            [
                AppEvent::ParallelModeSupervisorSnapshotInvalidated,
                AppEvent::SnapshotChanged(_)
            ]
        ));
    }

    #[test]
    fn resolved_completion_is_spliced_before_later_planning_refresh_events() {
        let post_turn = post_turn_correlation(7);
        let refresh = PlanningRuntimeRefreshCorrelation::new(3, "/workspace");
        let mut initial_snapshot = AppSnapshot::initial();
        initial_snapshot.revision = 1;
        let mut resolved_snapshot = AppSnapshot::initial();
        resolved_snapshot.revision = 2;
        let mut initial = CoreDispatchOutcome {
            events: vec![
                AppEvent::ParallelModeSupervisorSnapshotInvalidated,
                AppEvent::PlanningRuntimeRefreshStarted {
                    correlation: refresh.clone(),
                },
            ],
            effects: vec![CoreEffect::LoadParallelPeekConversation {
                correlation: ParallelPeekLoadCorrelation::new(1, "thread-initial"),
            }],
            snapshot: Arc::new(initial_snapshot),
        };
        let resolved = CoreDispatchOutcome {
            events: vec![
                AppEvent::SnapshotChanged(Arc::new(resolved_snapshot.clone())),
                AppEvent::PostTurnEvaluationCompleted {
                    correlation: post_turn.clone(),
                    execution: Box::new(post_turn_execution(&post_turn)),
                    route_resolution: PostTurnRouteResolution::NoContinuation,
                },
            ],
            effects: vec![CoreEffect::LoadParallelPeekConversation {
                correlation: ParallelPeekLoadCorrelation::new(2, "thread-resolved"),
            }],
            snapshot: Arc::new(resolved_snapshot),
        };

        merge_core_dispatch_outcome_at(&mut initial, resolved, 1);

        assert!(matches!(
            initial.events.as_slice(),
            [
                AppEvent::ParallelModeSupervisorSnapshotInvalidated,
                AppEvent::SnapshotChanged(_),
                AppEvent::PostTurnEvaluationCompleted {
                    correlation,
                    route_resolution: PostTurnRouteResolution::NoContinuation,
                    ..
                },
                AppEvent::PlanningRuntimeRefreshStarted {
                    correlation: refresh_correlation,
                },
            ] if correlation == &post_turn && refresh_correlation == &refresh
        ));
        assert_eq!(initial.effects.len(), 2);
        assert_eq!(initial.snapshot.revision, 2);
    }

    #[test]
    fn exact_runtime_route_settles_once_and_drops_duplicate_completion() {
        let mut runtime = native_client_runtime_for_routing_test();
        let _ = runtime.dispatch_client_event(NativeClientEvent::core(CoreInput::Command(
            AppCommand::PausePostTurnContinuation,
        )));
        let turn_correlation = runtime.begin_test_turn_submission();
        for event in [
            TurnStreamEvent::ThreadPrepared {
                thread_id: "thread-1".to_string(),
                title: "Native routing".to_string(),
                cwd: "/workspace".to_string(),
                runtime_envelope: Box::<ConversationRuntimeEnvelope>::default(),
            },
            TurnStreamEvent::TurnStarted {
                turn_id: "turn-1".to_string(),
                runtime_request: Box::<ConversationRuntimeConfigurationRequest>::default(),
            },
            TurnStreamEvent::TurnTerminal {
                receipt: ConversationTurnTerminalReceipt::completed(
                    "thread-1",
                    "turn-1",
                    Vec::new(),
                )
                .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed),
                execution_snapshot_capture: Some(TurnSnapshotCapture::capture_failed(
                    "/workspace",
                    "test capture skipped".to_string(),
                )),
            },
        ] {
            let _ = runtime.dispatch_client_event(NativeClientEvent::core(
                CoreInput::ConversationStreamUpdated {
                    correlation: turn_correlation,
                    event,
                },
            ));
        }
        let correlation = runtime.begin_test_post_turn_evaluation(
            "thread-1",
            "turn-1",
            "/workspace",
            "/workspace",
        );
        let completion =
            CoreInput::EffectCompleted(CoreEffectCompletion::PostTurnEvaluationCompleted {
                correlation: correlation.clone(),
                execution: Box::new(post_turn_execution(&correlation)),
            });

        let NativeClientDispatchOutcome::Combined { core, parallel } =
            runtime.dispatch_client_event(NativeClientEvent::core(completion.clone()))
        else {
            panic!("an exact route must return the combined Core/parallel outcome");
        };
        assert!(parallel.presentation_events.is_empty());
        assert_eq!(
            core.events
                .iter()
                .filter(|event| matches!(
                    event,
                    AppEvent::PostTurnEvaluationCompleted {
                        correlation: completed,
                        route_resolution: PostTurnRouteResolution::NoContinuation,
                        ..
                    } if completed == &correlation
                ))
                .count(),
            1
        );
        assert!(
            !core.events.iter().any(|event| matches!(
                event,
                AppEvent::PostTurnContinuationRoutingRequested { .. }
            ))
        );
        assert!(matches!(
            &core.snapshot.conversation_runtime.post_turn,
            PostTurnAuthoritySnapshot::Settled {
                correlation: settled,
                resolution: PostTurnRouteResolution::NoContinuation,
            } if settled == &correlation
        ));

        let NativeClientDispatchOutcome::Core(duplicate) =
            runtime.dispatch_client_event(NativeClientEvent::core(completion))
        else {
            panic!("a duplicate completion must remain a Core-only no-op");
        };
        assert!(duplicate.events.is_empty());
        assert!(duplicate.effects.is_empty());
    }

    #[derive(Debug)]
    struct UnusedNativeRuntimePort;

    impl crate::application::port::outbound::startup_probe_port::StartupProbePort
        for UnusedNativeRuntimePort
    {
        fn load_startup_context(
            &self,
        ) -> anyhow::Result<
            crate::application::port::outbound::startup_probe_port::AppServerStartupContext,
        > {
            unreachable!("routing test does not run startup effects")
        }
    }

    impl crate::application::port::outbound::session_catalog_port::SessionCatalogPort
        for UnusedNativeRuntimePort
    {
        fn load_session_catalog(
            &self,
            _request: SessionCatalogRequest,
        ) -> anyhow::Result<SessionCatalog> {
            unreachable!("routing test does not run session effects")
        }
    }

    impl crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort
        for UnusedNativeRuntimePort
    {
        fn runtime_control_truth(&self) -> ConversationRuntimeControlTruth {
            ConversationRuntimeControlTruth::codex_app_server()
        }

        fn load_conversation_snapshot(
            &self,
            _thread_id: &str,
        ) -> anyhow::Result<ConversationSnapshot> {
            unreachable!("routing test does not run conversation load effects")
        }

        fn request_stop_all_sessions(&self) -> anyhow::Result<()> {
            unreachable!("routing test does not run stop effects")
        }

        fn run_new_thread_stream(
            &self,
            _cwd: &str,
            _prompt: &str,
            _options: ConversationTurnOptions,
            _event_sender: crate::application::service::conversation_runtime_event::ConversationStreamSender,
        ) -> anyhow::Result<ConversationTurnTerminalReceipt> {
            unreachable!("routing test does not run turn effects")
        }

        fn run_turn_stream(
            &self,
            _thread_id: &str,
            _prompt: &str,
            _options: ConversationTurnOptions,
            _event_sender: crate::application::service::conversation_runtime_event::ConversationStreamSender,
        ) -> anyhow::Result<ConversationTurnTerminalReceipt> {
            unreachable!("routing test does not run turn effects")
        }
    }

    fn native_client_runtime_for_routing_test() -> NativeClientRuntime {
        let planning = test_planning_services(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));
        let parallel = test_parallel_mode_control_plane_composition(planning.clone());
        let parallel_turns = parallel.parallel_mode_turn_service();
        let port = Arc::new(UnusedNativeRuntimePort);
        NativeClientRuntime::new_for_test(
            StartupService::new(port.clone()),
            SessionService::new(port.clone()),
            ConversationService::new(port),
            planning,
            parallel_turns,
            parallel,
        )
    }

    fn awaiting_route_snapshot(correlation: &PostTurnEvaluationCorrelation) -> AppSnapshot {
        let mut snapshot = AppSnapshot::initial();
        snapshot.conversation_runtime.post_turn = PostTurnAuthoritySnapshot::AwaitingRoute {
            correlation: correlation.clone(),
            started_at: Instant::now(),
        };
        snapshot
    }

    fn internal_routing_request(
        correlation: &PostTurnEvaluationCorrelation,
        event_index: usize,
    ) -> InternalPostTurnRoutingRequest {
        InternalPostTurnRoutingRequest {
            event_index,
            correlation: correlation.clone(),
            execution: Box::new(post_turn_execution(correlation)),
        }
    }

    fn routing_event(correlation: &PostTurnEvaluationCorrelation) -> AppEvent {
        AppEvent::PostTurnContinuationRoutingRequested {
            correlation: correlation.clone(),
            execution: Box::new(post_turn_execution(correlation)),
        }
    }

    fn post_turn_correlation(generation: u64) -> PostTurnEvaluationCorrelation {
        PostTurnEvaluationCorrelation::new(
            generation,
            "thread-1",
            "turn-1",
            "/workspace",
            "/workspace",
        )
    }

    fn post_turn_execution(correlation: &PostTurnEvaluationCorrelation) -> PostTurnExecution {
        PostTurnExecution {
            thread_id: correlation.thread_id.clone(),
            completed_turn_id: correlation.completed_turn_id.clone(),
            runtime_projection_workspace_directory: correlation.turn_workspace_directory.clone(),
            evaluation: PostTurnOutcome {
                provenance: PostTurnProvenance::new(correlation.completed_turn_id.clone()),
                runtime_projection: RuntimeProjection::uninitialized(),
                planning_repair_state: None,
                runtime_notices: Vec::new(),
                action: PostTurnContinuationAction::QueueAutoPrompt(Box::new(
                    PostTurnQueuedPrompt {
                        prompt: "continue".to_string(),
                        mode_label: "planning queue".to_string(),
                        transcript_text: "continue".to_string(),
                    },
                )),
                operator_alerts: Vec::new(),
            },
            planning_worker_panel_state: PlanningWorkerPanelState::default(),
        }
    }
}
