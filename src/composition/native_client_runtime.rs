use crate::application::service::conversation_service::ConversationService;
#[cfg(test)]
use crate::application::service::github_review_poller_service::GithubReviewPollerService;
use crate::application::service::parallel_mode::control_plane::{
    ParallelModeControlPlaneComposition, ParallelModeControlPlaneEventSink,
    ParallelModeControlPlaneHandle,
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

/*
 * NativeClientRuntime is the composition-owned client runtime boundary used by
 * the native shell. Inbound adapters dispatch typed inputs and read immutable
 * projections; construction of the mailbox, effect runner, and core driver
 * stays private to composition.
 */
pub(crate) struct NativeClientRuntime {
    runtime: CoreRuntime<CoreEffectRunner>,
}

/*
 * Production service wiring is consumed here, before the inbound TUI boundary.
 * The TUI can bind its event sink and receive only the runtime facade, the
 * application-owned control-plane handle, and immutable capability truth.
 */
pub(crate) struct NativeTuiApplicationComposition {
    client_runtime: NativeClientRuntime,
    parallel_control_plane: ParallelModeControlPlaneComposition,
    turn_control_truth: ConversationRuntimeControlTruth,
}

pub(crate) struct BoundNativeTuiApplication<S>
where
    S: ParallelModeControlPlaneEventSink,
{
    client_runtime: NativeClientRuntime,
    parallel_control_plane: ParallelModeControlPlaneHandle<S>,
    turn_control_truth: ConversationRuntimeControlTruth,
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
        let client_runtime = NativeClientRuntime::new(
            startup_service,
            session_service,
            conversation_service,
            planning_feature,
            parallel_mode_turn_service,
        );
        Self {
            client_runtime,
            parallel_control_plane,
            turn_control_truth,
        }
    }

    pub(crate) fn bind_event_sink<S>(self, event_sink: S) -> BoundNativeTuiApplication<S>
    where
        S: ParallelModeControlPlaneEventSink,
    {
        BoundNativeTuiApplication {
            client_runtime: self.client_runtime,
            parallel_control_plane: self.parallel_control_plane.bind_event_sink(event_sink),
            turn_control_truth: self.turn_control_truth,
        }
    }
}

impl<S> BoundNativeTuiApplication<S>
where
    S: ParallelModeControlPlaneEventSink,
{
    pub(crate) fn into_parts(
        self,
    ) -> (
        NativeClientRuntime,
        ParallelModeControlPlaneHandle<S>,
        ConversationRuntimeControlTruth,
    ) {
        (
            self.client_runtime,
            self.parallel_control_plane,
            self.turn_control_truth,
        )
    }
}

impl NativeClientRuntime {
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

    #[cfg(test)]
    pub(crate) fn new_for_test(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        planning_feature: PlanningServices,
        parallel_mode_turn_service: ParallelModeTurnService,
    ) -> Self {
        Self::new(
            startup_service,
            session_service,
            conversation_service,
            planning_feature,
            parallel_mode_turn_service,
        )
    }

    #[cfg(test)]
    pub(crate) fn new_with_github_review_polling_setup_loader(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        planning_feature: PlanningServices,
        parallel_mode_turn_service: ParallelModeTurnService,
        loader: impl Fn(
            &GithubReviewPollingSetupRequest,
        )
            -> anyhow::Result<Option<(GithubPullRequestTarget, GithubReviewPollerService)>>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self::new_with_effect_runner_configuration(
            startup_service,
            session_service,
            conversation_service,
            planning_feature,
            parallel_mode_turn_service,
            |runner| runner.with_github_review_polling_setup_loader(loader),
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

    #[must_use = "dispatch outcomes carry projection events that the inbound adapter must apply"]
    pub(crate) fn dispatch_client_event(&mut self, input: CoreInput) -> CoreDispatchOutcome {
        self.runtime.dispatch_input(input)
    }

    pub(crate) fn poll_pending_client_event(&mut self) -> Option<CoreDispatchOutcome> {
        self.runtime.poll_pending_input()
    }

    pub(crate) fn snapshot(&self) -> AppSnapshot {
        self.runtime.snapshot()
    }

    pub(crate) fn revisioned_planning_parallel_projection(
        &self,
    ) -> RevisionedPlanningParallelProjection {
        self.runtime.revisioned_planning_parallel_projection()
    }

    pub(crate) fn parallel_mode_projection(&self) -> ParallelModeProjection {
        self.runtime.parallel_mode_projection()
    }

    #[cfg(test)]
    pub(crate) fn begin_test_turn_submission(&mut self) -> TurnSubmissionCorrelation {
        self.runtime.begin_test_turn_submission()
    }

    #[cfg(test)]
    pub(crate) fn begin_test_post_turn_evaluation(
        &mut self,
        thread_id: &str,
        completed_turn_id: &str,
        turn_workspace_directory: &str,
        planning_workspace_directory: &str,
    ) -> PostTurnEvaluationCorrelation {
        self.runtime.begin_test_post_turn_evaluation(
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
        self.runtime
            .test_post_turn_evaluation_is_in_flight(thread_id, completed_turn_id)
    }
}
