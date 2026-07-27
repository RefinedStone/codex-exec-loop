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
    ParallelModePostTurnQueueContinuationOutcome,
};
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::planning::PlanningServices;
use crate::application::service::post_turn_evaluation::PostTurnEvaluationService;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::composition::core_effect_runner::CoreEffectRunner;
use crate::core::app::{
    AppSnapshot, CoreDispatchOutcome, CoreInput, ParallelModeProjection,
    RevisionedPlanningParallelProjection,
};
#[cfg(test)]
use crate::core::app::{
    GithubReviewPollingSetupRequest, PostTurnEvaluationCorrelation, TurnSubmissionCorrelation,
};
use crate::core::runtime::{CoreRuntime, core_input_channel};
use crate::domain::conversation::ConversationRuntimeControlTruth;
#[cfg(test)]
use crate::domain::github_review::GithubPullRequestTarget;
#[cfg(test)]
use crate::domain::parallel_mode::ParallelModeAutomationTrigger;
use crate::domain::parallel_mode::ParallelModePostTurnQueueSignal;

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
    ParallelPostTurnQueue {
        workspace_directory: String,
        signal: Option<ParallelModePostTurnQueueSignal>,
        auto_follow_prompt_queued: bool,
        has_actionable_queue_head: bool,
    },
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
}

pub(crate) struct NativeParallelDispatchOutcome {
    pub(crate) presentation_events: Vec<ParallelModeControlPlanePresentationEvent>,
    pub(crate) auto_follow_prompt_consumed: bool,
}

impl NativeParallelDispatchOutcome {
    fn from_events(presentation_events: Vec<ParallelModeControlPlanePresentationEvent>) -> Self {
        Self {
            presentation_events,
            auto_follow_prompt_consumed: false,
        }
    }

    fn from_post_turn(outcome: ParallelModePostTurnQueueContinuationOutcome) -> Self {
        Self {
            presentation_events: outcome.presentation_events,
            auto_follow_prompt_consumed: outcome.auto_follow_prompt_consumed,
        }
    }
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
            NativeClientEvent::Core(input) => {
                NativeClientDispatchOutcome::Core(self.core.runtime.dispatch_input(*input))
            }
            NativeClientEvent::ParallelCommand(command) => {
                NativeClientDispatchOutcome::Parallel(NativeParallelDispatchOutcome::from_events(
                    self.parallel_control_plane.handle_command(command),
                ))
            }
            NativeClientEvent::ParallelPostTurnQueue {
                workspace_directory,
                signal,
                auto_follow_prompt_queued,
                has_actionable_queue_head,
            } => NativeClientDispatchOutcome::Parallel(
                NativeParallelDispatchOutcome::from_post_turn(
                    self.parallel_control_plane.continue_post_turn_queue(
                        workspace_directory,
                        signal,
                        auto_follow_prompt_queued,
                        has_actionable_queue_head,
                    ),
                ),
            ),
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
                    .map(NativeClientDispatchOutcome::Core),
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
