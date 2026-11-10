use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::time::{Duration, Instant};

use crate::application::port::outbound::conversation_thread_turn_options_port::ConversationThreadTurnOptionsPort;
#[cfg(test)]
use crate::application::port::outbound::conversation_thread_turn_options_port::NoopConversationThreadTurnOptionsPort;
use crate::application::service::conversation_service::ConversationService;
#[cfg(test)]
use crate::application::service::github_review_poller_service::GithubReviewPollerService;
use crate::application::service::parallel_mode::control_plane::{
    ParallelModeControlPlaneBackgroundEvent,
    ParallelModeControlPlaneCommand as ApplicationParallelModeControlPlaneCommand,
    ParallelModeControlPlaneComposition,
    ParallelModeControlPlaneEpochSnapshot as ApplicationParallelModeControlPlaneEpochSnapshot,
    ParallelModeControlPlaneEventSink, ParallelModeControlPlaneHandle,
    ParallelModeControlPlaneLoadingStage as ApplicationParallelModeControlPlaneLoadingStage,
    ParallelModeControlPlanePresentationEvent as ApplicationParallelModeControlPlanePresentationEvent,
    ParallelModeControlPlanePresentationProjection as ApplicationParallelModeControlPlanePresentationProjection,
};
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::planning::PlanningServices;
use crate::application::service::post_turn_evaluation::PostTurnEvaluationService;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::composition::clipboard_image_probe::ClipboardImageProbeTrigger;
use crate::composition::core_effect_runner::CoreEffectRunner;
use crate::core::app::{
    AppCommand, AppEvent, AppSnapshot, ConversationRuntimeSnapshot, CoreDispatchOutcome, CoreInput,
    ParallelModeProjection, PostTurnEvaluationCorrelation, PostTurnHostStep,
    PostTurnRouteResolution, PostTurnRouteWork, RevisionedPlanningParallelProjection,
};
#[cfg(test)]
use crate::core::app::{GithubReviewPollingSetupRequest, TurnSubmissionCorrelation};
use crate::core::native_client_port::{
    NativeClientDispatchOutcome, NativeClientEvent, NativeClientPort,
    NativeParallelDispatchOutcome, ParallelModeControlPlaneCommand,
    ParallelModeControlPlaneEpochSnapshot, ParallelModeControlPlaneLoadingStage,
    ParallelModeControlPlanePresentationEvent, ParallelModeControlPlanePresentationProjection,
    ParallelModeDispatchCleanupCorrelation, ParallelModeGlobalRuntimeNoticeProjection,
    ParallelViewState,
};
#[cfg(test)]
use crate::core::runtime::CoreInputSender;
use crate::core::runtime::{CoreEffectExecutor, CoreRuntime, core_input_channel};
use crate::domain::conversation::ConversationRuntimeControlTruth;
#[cfg(test)]
use crate::domain::github_review::GithubPullRequestTarget;
#[cfg(test)]
use crate::domain::parallel_mode::ParallelModeAutomationTrigger;
use crate::domain::planning::PostTurnContinuationAction;

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
    // 병렬 tick 이 쓰는 작업 공간과 supervisor 화면 상태이다. 클라이언트가 바뀔 때 알린다 (plan 18 K1-2).
    parallel_view: Option<ParallelViewState>,
    // GitHub 리뷰 폴링 설정 값과 마지막으로 설정을 요청한 작업 공간이다 (plan 18 K2-6).
    github_review_polling: Option<crate::core::app::GithubReviewPollingSettings>,
    github_review_setup_workspace: Option<String>,
    // core 에 마지막으로 알린 병렬 모드 값이다 (plan 18 K3-3b). 병렬 제어부 값과 다르면 다시 알린다.
    core_parallel_mode: Option<bool>,
}

struct NativeCoreRuntime {
    runtime: CoreRuntime<Box<dyn CoreEffectExecutor + Send>>,
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
    clipboard_image_probe_trigger: Arc<dyn ClipboardImageProbeTrigger>,
    github_review_polling: Option<crate::core::app::GithubReviewPollingSettings>,
}

pub(crate) struct BoundNativeTuiApplication {
    client_runtime: NativeClientRuntime,
    turn_control_truth: ConversationRuntimeControlTruth,
    clipboard_image_probe_trigger: Arc<dyn ClipboardImageProbeTrigger>,
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

/// 한 dispatch 에서 호스트가 하는 턴 뒤 단계의 최대 수이다. 단계마다 상태가 바뀌므로 보통 1~3 개로 끝난다.
const MAX_POST_TURN_HOST_STEPS: usize = 8;

/*
 * 턴 뒤 단계의 결과를 넣을 자리이다 (plan 19 Q2-2). core 는 그 단계가 생긴 사건의 결과 안에 표시 이벤트를 둔다.
 * 화면은 이벤트를 차례로 적용하므로, 결과를 그 자리에 넣어 뒤의 이벤트(예: planning 새로 고침 시작)보다 앞에 둔다.
 * 단계는 표시가 아니라 상태에서 정한다. 표시가 없으면(놓친 사건을 다음 비교가 이어 갈 때) 끝에 붙인다.
 */
fn post_turn_anchor_index(events: &[AppEvent], step: &PostTurnHostStep) -> Option<usize> {
    match step {
        PostTurnHostStep::StartEvaluation {
            planning_workspace_directory: wanted,
        } => events.iter().position(|event| {
            matches!(
                event,
                AppEvent::PostTurnEvaluationDue { planning_workspace_directory }
                    if planning_workspace_directory == wanted
            )
        }),
        PostTurnHostStep::RouteContinuation(work) => route_marker_index(events, &work.correlation),
        // 끝낼 평가가 여럿이라 자리도 여럿이다. 호스트가 평가마다 `route_marker_index` 로 찾는다.
        PostTurnHostStep::SettleWithoutContinuation { .. } => None,
    }
}

fn route_marker_index(
    events: &[AppEvent],
    correlation: &PostTurnEvaluationCorrelation,
) -> Option<usize> {
    events.iter().position(|event| {
        matches!(
            event,
            AppEvent::PostTurnContinuationRoutingRequested { correlation: marked, .. }
                if marked == correlation
        )
    })
}

/// 단계 결과를 자리의 표시 이벤트 대신 넣는다. 자리가 없으면 끝에 붙인다.
fn splice_post_turn_step(
    target: &mut CoreDispatchOutcome,
    anchor: Option<usize>,
    step_outcome: CoreDispatchOutcome,
) {
    let range = match anchor {
        Some(index) => index..index + 1,
        None => target.events.len()..target.events.len(),
    };
    target.events.splice(range, step_outcome.events);
    target.effects.extend(step_outcome.effects);
    target.snapshot = step_outcome.snapshot;
}

/// 표시 이벤트는 호스트 안에서만 쓴다. 클라이언트에 보내기 전에 남은 것을 지운다.
fn strip_post_turn_markers(outcome: &mut CoreDispatchOutcome) {
    outcome.events.retain(|event| {
        !matches!(
            event,
            AppEvent::PostTurnEvaluationDue { .. }
                | AppEvent::PostTurnContinuationRoutingRequested { .. }
        )
    });
}

/// 그 thread 의 자동 후속 설정이 병렬 경로를 허락하면 병렬 제어부에 묻는다.
fn route_parallel_post_turn_when_allowed<T>(
    runtime: &ConversationRuntimeSnapshot,
    route: impl FnOnce() -> T,
) -> Option<T> {
    runtime
        .auto_follow
        .parallel_post_turn_continuation_allowed()
        .then(route)
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

fn application_parallel_command(
    command: ParallelModeControlPlaneCommand,
) -> ApplicationParallelModeControlPlaneCommand {
    match command {
        ParallelModeControlPlaneCommand::OpenEpoch {
            workspace_directory,
        } => ApplicationParallelModeControlPlaneCommand::OpenEpoch {
            workspace_directory,
        },
        ParallelModeControlPlaneCommand::Enable {
            workspace_directory,
        } => ApplicationParallelModeControlPlaneCommand::Enable {
            workspace_directory,
        },
        ParallelModeControlPlaneCommand::Disable {
            workspace_directory,
        } => ApplicationParallelModeControlPlaneCommand::Disable {
            workspace_directory,
        },
        ParallelModeControlPlaneCommand::InspectSupervisor {
            workspace_directory,
            reconcile_pool,
            show_status,
        } => ApplicationParallelModeControlPlaneCommand::InspectSupervisor {
            workspace_directory,
            reconcile_pool,
            show_status,
        },
        ParallelModeControlPlaneCommand::RefreshSupervisor {
            workspace_directory,
        } => ApplicationParallelModeControlPlaneCommand::RefreshSupervisor {
            workspace_directory,
        },
        ParallelModeControlPlaneCommand::RequestDispatch {
            workspace_directory,
            trigger,
        } => ApplicationParallelModeControlPlaneCommand::RequestDispatch {
            workspace_directory,
            trigger,
        },
        ParallelModeControlPlaneCommand::RequestDispatchForEpoch {
            workspace_directory,
            trigger,
            epoch_id,
        } => ApplicationParallelModeControlPlaneCommand::RequestDispatchForEpoch {
            workspace_directory,
            trigger,
            epoch_id,
        },
    }
}

fn client_loading_stage(
    stage: ApplicationParallelModeControlPlaneLoadingStage,
) -> ParallelModeControlPlaneLoadingStage {
    match stage {
        ApplicationParallelModeControlPlaneLoadingStage::ReconcilingPool => {
            ParallelModeControlPlaneLoadingStage::ReconcilingPool
        }
    }
}

fn client_presentation_event(
    event: ApplicationParallelModeControlPlanePresentationEvent,
) -> ParallelModeControlPlanePresentationEvent {
    match event {
        ApplicationParallelModeControlPlanePresentationEvent::EnterProgress {
            workspace_directory,
            epoch_id: _,
            effect_id: _,
            readiness_snapshot,
            loading_stage,
            status_text,
        } => ParallelModeControlPlanePresentationEvent::EnterProgress {
            workspace_directory,
            readiness_snapshot,
            loading_stage: client_loading_stage(loading_stage),
            status_text,
        },
        ApplicationParallelModeControlPlanePresentationEvent::ReadinessSnapshotChanged {
            workspace_directory,
            snapshot,
        } => ParallelModeControlPlanePresentationEvent::ReadinessSnapshotChanged {
            workspace_directory,
            snapshot,
        },
        ApplicationParallelModeControlPlanePresentationEvent::SupervisorSnapshotChanged {
            workspace_directory,
            snapshot,
        } => ParallelModeControlPlanePresentationEvent::SupervisorSnapshotChanged {
            workspace_directory,
            snapshot,
        },
        ApplicationParallelModeControlPlanePresentationEvent::StatusShown {
            workspace_directory,
            status_text,
        } => ParallelModeControlPlanePresentationEvent::StatusShown {
            workspace_directory,
            status_text,
        },
        ApplicationParallelModeControlPlanePresentationEvent::ConversationRuntimeNotice {
            workspace_directory,
            notice,
        } => ParallelModeControlPlanePresentationEvent::ConversationRuntimeNotice {
            workspace_directory,
            notice,
        },
        ApplicationParallelModeControlPlanePresentationEvent::GlobalRuntimeNoticesChanged => {
            ParallelModeControlPlanePresentationEvent::GlobalRuntimeNoticesChanged
        }
        ApplicationParallelModeControlPlanePresentationEvent::PostTurnAutoFollowPromptConsumed => {
            ParallelModeControlPlanePresentationEvent::PostTurnAutoFollowPromptConsumed
        }
        ApplicationParallelModeControlPlanePresentationEvent::PlanningRuntimeRefreshRequested {
            workspace_directory,
        } => ParallelModeControlPlanePresentationEvent::PlanningRuntimeRefreshRequested {
            workspace_directory,
        },
        ApplicationParallelModeControlPlanePresentationEvent::ModeDisabled { .. } => {
            ParallelModeControlPlanePresentationEvent::ModeDisabled
        }
    }
}

fn client_presentation_events(
    events: Vec<ApplicationParallelModeControlPlanePresentationEvent>,
) -> Vec<ParallelModeControlPlanePresentationEvent> {
    events.into_iter().map(client_presentation_event).collect()
}

fn client_control_plane_projection(
    projection: ApplicationParallelModeControlPlanePresentationProjection,
) -> ParallelModeControlPlanePresentationProjection {
    ParallelModeControlPlanePresentationProjection {
        mode_enabled: projection.mode_enabled,
        control_effect_in_flight: projection.control_effect_in_flight,
        last_dispatch_withheld_reason: projection.last_dispatch_withheld_reason,
        global_runtime_notices: projection
            .global_runtime_notices
            .into_iter()
            .map(|projection| ParallelModeGlobalRuntimeNoticeProjection {
                cleanup_correlation: ParallelModeDispatchCleanupCorrelation {
                    operation_id: projection.cleanup_correlation.operation_id,
                    workspace_directory: projection.cleanup_correlation.workspace_directory,
                    epoch_id: projection.cleanup_correlation.epoch_id,
                    command_identity: projection.cleanup_correlation.command_identity,
                },
                notice: projection.notice,
            })
            .collect(),
    }
}

fn client_epoch_snapshot(
    snapshot: ApplicationParallelModeControlPlaneEpochSnapshot,
) -> ParallelModeControlPlaneEpochSnapshot {
    ParallelModeControlPlaneEpochSnapshot {
        workspace_directory: snapshot.workspace_directory,
        current_epoch_id: snapshot.current_epoch_id,
    }
}

impl NativeTuiApplicationComposition {
    pub(in crate::composition) fn from_services(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        parallel_control_plane: ParallelModeControlPlaneComposition,
        thread_turn_options_port: Arc<dyn ConversationThreadTurnOptionsPort>,
        clipboard_image_probe_trigger: Arc<dyn ClipboardImageProbeTrigger>,
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
            thread_turn_options_port.clone(),
        );
        Self {
            core_runtime,
            parallel_control_plane,
            turn_control_truth,
            clipboard_image_probe_trigger,
            github_review_polling: None,
        }
    }

    /// 프로세스 설정에서 읽은 GitHub 리뷰 폴링 값을 호스트에 넘긴다 (plan 18 K2-6).
    pub(crate) fn with_github_review_polling(
        mut self,
        settings: Option<crate::core::app::GithubReviewPollingSettings>,
    ) -> Self {
        self.github_review_polling = settings;
        self
    }

    pub(crate) fn bind_client_runtime(self) -> BoundNativeTuiApplication {
        let mut client_runtime =
            NativeClientRuntime::from_parts(self.core_runtime, self.parallel_control_plane);
        client_runtime.github_review_polling = self.github_review_polling;
        BoundNativeTuiApplication {
            client_runtime,
            turn_control_truth: self.turn_control_truth,
            clipboard_image_probe_trigger: self.clipboard_image_probe_trigger,
        }
    }

    /// 화면 없는 커널이 쓰는 런타임이다 (plan 18 K3-4). 커널 호스트가 이것을 자기 스레드로 가져간다.
    pub(crate) fn into_headless_client_runtime(self) -> NativeClientRuntime {
        self.bind_client_runtime().client_runtime
    }
}

impl BoundNativeTuiApplication {
    pub(crate) fn into_parts(
        self,
    ) -> (
        Box<dyn NativeClientPort>,
        ConversationRuntimeControlTruth,
        Arc<dyn ClipboardImageProbeTrigger>,
    ) {
        (
            Box::new(self.client_runtime),
            self.turn_control_truth,
            self.clipboard_image_probe_trigger,
        )
    }
}

impl NativeCoreRuntime {
    fn new(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        planning_feature: PlanningServices,
        parallel_mode_turn_service: ParallelModeTurnService,
        thread_turn_options_port: Arc<dyn ConversationThreadTurnOptionsPort>,
    ) -> Self {
        Self::new_with_effect_runner_configuration(
            startup_service,
            session_service,
            conversation_service,
            planning_feature,
            parallel_mode_turn_service,
            thread_turn_options_port,
            |runner| runner,
        )
    }

    fn new_with_effect_runner_configuration(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        planning_feature: PlanningServices,
        parallel_mode_turn_service: ParallelModeTurnService,
        thread_turn_options_port: Arc<dyn ConversationThreadTurnOptionsPort>,
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
            thread_turn_options_port,
            input_sender,
        ));

        Self {
            runtime: CoreRuntime::new(Box::new(effect_runner), input_receiver),
        }
    }
}

/// 런타임을 버리면 병렬 제어부의 작업 스레드가 더는 배분하지 못한다. 커널은 호스트 스레드에서 런타임을 버린 뒤
/// 저장소 잠금을 푼다. 그렇지 않으면 같은 저장소의 다음 커널과 이전 작업 스레드가 같은 큐를 배분할 수 있다 (K3-5).
impl Drop for NativeClientRuntime {
    fn drop(&mut self) {
        self.parallel_control_plane.stop_automation();
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
            parallel_view: None,
            github_review_polling: None,
            github_review_setup_workspace: None,
            core_parallel_mode: None,
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
                Arc::new(NoopConversationThreadTurnOptionsPort),
            ),
            parallel_control_plane,
        )
    }

    /// 효과 실행기를 시험이 정한다 (plan 19 Q1). 시험은 돌려받은 core 입력함의 보내는 쪽으로 효과의 완료를 넣는다.
    #[cfg(test)]
    pub(crate) fn new_with_effect_executor_for_test(
        parallel_control_plane: ParallelModeControlPlaneComposition,
        effect_executor: Box<dyn CoreEffectExecutor + Send>,
    ) -> (Self, CoreInputSender) {
        let (input_sender, input_receiver) = core_input_channel();
        let core = NativeCoreRuntime {
            runtime: CoreRuntime::new(effect_executor, input_receiver),
        };
        (Self::from_parts(core, parallel_control_plane), input_sender)
    }

    #[cfg(test)]
    #[allow(dead_code)]
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
                Arc::new(NoopConversationThreadTurnOptionsPort),
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
            NativeClientEvent::ParallelCommand(command) => NativeClientDispatchOutcome::Parallel(
                NativeParallelDispatchOutcome::from_events(client_presentation_events(
                    self.parallel_control_plane
                        .handle_command(application_parallel_command(command)),
                )),
            ),
            NativeClientEvent::ParallelTick { now } => {
                let events = match &self.parallel_view {
                    Some(view) => self.parallel_control_plane.tick(
                        now,
                        view.workspace_directory.clone(),
                        view.supervisor_view_active,
                    ),
                    None => Vec::new(),
                };
                NativeClientDispatchOutcome::Parallel(NativeParallelDispatchOutcome::from_events(
                    client_presentation_events(events),
                ))
            }
            NativeClientEvent::ParallelViewChanged(view) => {
                self.parallel_view = Some(view);
                NativeClientDispatchOutcome::Parallel(NativeParallelDispatchOutcome::from_events(
                    Vec::new(),
                ))
            }
            NativeClientEvent::ClearParallelDispatchWithheldReason => {
                self.parallel_control_plane.clear_dispatch_withheld_reason();
                NativeClientDispatchOutcome::Parallel(NativeParallelDispatchOutcome::from_events(
                    Vec::new(),
                ))
            }
        }
    }

    pub(crate) fn poll_pending_client_event(&mut self) -> Option<NativeClientDispatchOutcome> {
        // 앞의 병렬 완료가 병렬 모드를 바꿨을 수 있다. core 입력을 꺼내기 전에 맞춘다.
        self.report_parallel_mode_to_core();
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
                    .map(|outcome| self.reconcile_post_turn(outcome)),
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
        self.report_parallel_mode_to_core();
        let outcome = self.core.runtime.dispatch_input(input);
        self.reconcile_post_turn(outcome)
    }

    /*
     * core 가 아는 병렬 모드 값을 병렬 제어부 값에 맞춘다 (plan 18 K3-3b). core 는 thread 지정 프롬프트를 받을 때와
     * 준비가 끝나 제출할 때 이 값을 본다. 호스트는 core 입력을 넣기 전마다 값을 다시 읽어 바뀌었을 때만 알린다.
     * shortcut: 병렬 제어부는 공유 상태라 다른 스레드도 바꿀 수 있다. 커널 대기(`wait`) 중에 바뀌면 그 대기에서
     * 깨어난 core 입력 하나는 이전 값을 본다. 병렬 제어부를 커널에 합칠 때(plan 19 Q5) 없앤다.
     */
    fn report_parallel_mode_to_core(&mut self) {
        let enabled = self.parallel_mode_enabled();
        if self.core_parallel_mode == Some(enabled) {
            return;
        }
        self.core_parallel_mode = Some(enabled);
        // core 에 바로 넣는다. 턴 뒤 단계는 이 보고를 부른 바깥 dispatch 가 상태를 보고 한다 (plan 19 Q2-2).
        let outcome = self.core.runtime.dispatch_input(CoreInput::Command(
            AppCommand::SetParallelModeEnabled(enabled),
        ));
        debug_assert!(
            outcome.events.is_empty(),
            "reporting the parallel mode only updates core state"
        );
    }

    /*
     * 턴 뒤 단계를 상태에서 고른다 (plan 19 Q2-2). core 가 고른 단계를 하고 다시 묻는다. 평가 시작은 호스트가 병렬
     * 값(사용 여부, epoch)을 붙인다 (plan 18 K1-4). 경로 결정은 병렬 제어부에 묻고, AutoSubmit 이면 자동 후속 턴을
     * 제출한다 (plan 18 K1-5). 이전에는 core 결과의 표시 이벤트를 따라 단계를 이어 붙였다.
     */
    fn reconcile_post_turn(
        &mut self,
        mut core_outcome: CoreDispatchOutcome,
    ) -> NativeClientDispatchOutcome {
        let mut parallel_events = Vec::new();
        let mut routed = false;
        for _ in 0..MAX_POST_TURN_HOST_STEPS {
            let Some(step) = self.core.runtime.next_post_turn_host_step() else {
                break;
            };
            let anchor = post_turn_anchor_index(&core_outcome.events, &step);
            match step {
                PostTurnHostStep::StartEvaluation {
                    planning_workspace_directory,
                } => {
                    let started = self.start_post_turn_evaluation(&planning_workspace_directory);
                    splice_post_turn_step(&mut core_outcome, anchor, started);
                }
                PostTurnHostStep::SettleWithoutContinuation { correlations } => {
                    // 끝낼 평가마다 그 평가의 표시 자리를 따로 찾는다.
                    for correlation in correlations {
                        let anchor = route_marker_index(&core_outcome.events, &correlation);
                        let settled = self.resolve_post_turn_route(
                            correlation,
                            PostTurnRouteResolution::NoContinuation,
                        );
                        splice_post_turn_step(&mut core_outcome, anchor, settled);
                    }
                }
                PostTurnHostStep::RouteContinuation(work) => {
                    routed = true;
                    let (resolved, submitted, events) = self.route_post_turn_continuation(*work);
                    splice_post_turn_step(&mut core_outcome, anchor, resolved);
                    if let Some(submitted) = submitted {
                        splice_post_turn_step(&mut core_outcome, None, submitted);
                    }
                    parallel_events.extend(events);
                }
            }
        }
        debug_assert!(
            self.core.runtime.next_post_turn_host_step().is_none(),
            "post-turn host steps must settle within one dispatch"
        );
        strip_post_turn_markers(&mut core_outcome);
        if !routed {
            return NativeClientDispatchOutcome::Core(core_outcome);
        }
        NativeClientDispatchOutcome::Combined {
            core: core_outcome,
            parallel: NativeParallelDispatchOutcome {
                presentation_events: parallel_events,
            },
        }
    }

    fn start_post_turn_evaluation(
        &mut self,
        planning_workspace_directory: &str,
    ) -> CoreDispatchOutcome {
        let command = AppCommand::EvaluateCompletedTurn {
            parallel_mode_enabled: self.parallel_mode_enabled(),
            parallel_automation_epoch_id: self
                .current_parallel_epoch_id_for_workspace(planning_workspace_directory),
        };
        self.core.runtime.dispatch_command(command)
    }

    fn resolve_post_turn_route(
        &mut self,
        correlation: PostTurnEvaluationCorrelation,
        resolution: PostTurnRouteResolution,
    ) -> CoreDispatchOutcome {
        self.core
            .runtime
            .dispatch_command(AppCommand::ResolvePostTurnContinuation {
                correlation,
                resolution,
            })
    }

    /*
     * 경로를 기다리는 평가 하나의 경로를 고른다. 병렬 제어부가 그 평가의 자동 후속 프롬프트를 쓰면 병렬 경로이다.
     * 아니면 그 thread 가 자동 후속을 더 낼 수 있을 때 AutoSubmit 이다. 돌려주는 값은 경로 결정 결과, 자동 후속
     * 제출 결과(AutoSubmit 일 때), 병렬 제어부의 화면 이벤트이다.
     */
    fn route_post_turn_continuation(
        &mut self,
        work: PostTurnRouteWork,
    ) -> (
        CoreDispatchOutcome,
        Option<CoreDispatchOutcome>,
        Vec<ParallelModeControlPlanePresentationEvent>,
    ) {
        let PostTurnRouteWork {
            correlation,
            execution,
            runtime,
        } = work;
        let queued_prompt = match &execution.evaluation.action {
            PostTurnContinuationAction::QueueAutoPrompt(queued) => Some(queued.as_ref().clone()),
            PostTurnContinuationAction::SkipAutoFollow { .. } => None,
        };
        let parallel_outcome = route_parallel_post_turn_when_allowed(&runtime, || {
            self.parallel_control_plane.continue_post_turn_queue(
                execution.runtime_projection_workspace_directory.clone(),
                execution.evaluation.provenance.parallel_queue_signal,
                queued_prompt.is_some(),
                execution
                    .evaluation
                    .runtime_projection
                    .has_actionable_queue_head(),
            )
        });
        let parallel_consumed_prompt = parallel_outcome
            .as_ref()
            .is_some_and(|outcome| outcome.auto_follow_prompt_consumed);
        let parallel_events = parallel_outcome
            .map(|outcome| client_presentation_events(outcome.presentation_events))
            .unwrap_or_default();
        let resolution = post_turn_route_resolution(
            queued_prompt.is_some(),
            parallel_consumed_prompt,
            runtime.auto_follow.can_queue_next(),
        );
        let auto_follow_submission = match (&resolution, queued_prompt) {
            (PostTurnRouteResolution::AutoSubmit, Some(queued)) => {
                Some(crate::core::app::AutoFollowTurnSubmission {
                    source: correlation.clone(),
                    completed_turn_id: execution.evaluation.provenance.completed_turn_id.clone(),
                    prompt: queued.prompt,
                    transcript_text: queued.transcript_text,
                    mode_label: queued.mode_label,
                    parallel_mode_enabled: self.parallel_mode_enabled(),
                })
            }
            _ => None,
        };
        let resolved = self.resolve_post_turn_route(correlation, resolution);
        let submitted = auto_follow_submission.map(|submission| {
            self.core
                .runtime
                .dispatch_command(AppCommand::SubmitAutoFollowTurn(Box::new(submission)))
        });
        (resolved, submitted, parallel_events)
    }

    fn dispatch_parallel_completion(
        &mut self,
        completion: ParallelModeControlPlaneBackgroundEvent,
    ) -> NativeClientDispatchOutcome {
        NativeClientDispatchOutcome::Parallel(NativeParallelDispatchOutcome::from_events(
            client_presentation_events(
                self.parallel_control_plane
                    .handle_background_event(completion),
            ),
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

    /// 병렬 보기의 작업 공간이다 (plan 18 K3-6). 커널은 조립할 때 저장소의 대표 checkout 으로 정한다.
    pub(crate) fn parallel_view_workspace(&self) -> Option<String> {
        self.parallel_view
            .as_ref()
            .map(|view| view.workspace_directory.clone())
    }

    #[cfg(test)]
    pub(crate) fn parallel_view_for_test(&self) -> Option<&ParallelViewState> {
        self.parallel_view.as_ref()
    }

    pub(crate) fn run_host_cycle(
        &mut self,
        now: Instant,
        wait: Duration,
        max_inputs: usize,
    ) -> Vec<NativeClientDispatchOutcome> {
        let mut outcomes = Vec::new();
        self.report_parallel_mode_to_core();
        if !wait.is_zero()
            && let Some(outcome) = self.core.runtime.wait_pending_input(wait)
        {
            outcomes.push(self.reconcile_post_turn(outcome));
        }
        while outcomes.len() < max_inputs {
            let Some(outcome) = self.poll_pending_client_event() else {
                break;
            };
            outcomes.push(outcome);
        }
        outcomes.push(self.dispatch_core_input(CoreInput::Tick { now }));
        outcomes.push(self.dispatch_client_event(NativeClientEvent::ParallelTick { now }));
        if let Some(outcome) = self.start_github_review_polling_setup_when_due() {
            outcomes.push(outcome);
        }
        outcomes
    }

    /*
     * GitHub 리뷰 폴링 설정 시작을 호스트가 정한다 (plan 18 K2-6). 이전에는 TUI 가 첫 화면을 그린 뒤 보냈다.
     * 시작 점검이 성공한 뒤, 작업 공간이 처음 정해지거나 바뀔 때 한 번 보낸다. 작업 공간은 화면이 알린
     * planning 작업 공간이고, 화면이 없으면 시작 점검의 작업 공간이다. 탐색은 worker 에서 하므로 화면을
     * 막지 않는다.
     */
    fn start_github_review_polling_setup_when_due(
        &mut self,
    ) -> Option<NativeClientDispatchOutcome> {
        let settings = self.github_review_polling.as_ref()?;
        let startup_workspace = self.core.runtime.startup_ready_workspace()?;
        let workspace_directory = self
            .parallel_view
            .as_ref()
            .map(|view| view.workspace_directory.clone())
            .filter(|workspace| !workspace.trim().is_empty())
            .unwrap_or(startup_workspace);
        if self.github_review_setup_workspace.as_deref() == Some(workspace_directory.as_str()) {
            return None;
        }
        let request = settings.request_for(&workspace_directory);
        self.github_review_setup_workspace = Some(workspace_directory);
        Some(
            self.dispatch_core_input(CoreInput::Command(AppCommand::SetupGithubReviewPolling(
                request,
            ))),
        )
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

    /// 커널에 남은 일이 있다 (plan 18 K3-5). core 의 턴·승인·자동 후속·평가·planning 작업, 또는 병렬 제어부의
    /// 일(켜진 병렬 모드, 열린 epoch, 진행 중인 제어 효과, 배분된 작업 스레드)이다. 커널을 내려도 되는지 볼 때 쓴다.
    pub(crate) fn has_remaining_work(&self) -> bool {
        self.core.runtime.has_remaining_work() || self.parallel_control_plane.has_remaining_work()
    }

    /// 런타임을 다른 스레드로 옮긴 뒤에도 병렬 epoch 를 확인한다.
    #[cfg(test)]
    pub(crate) fn parallel_automation_probe_for_test(
        &self,
    ) -> impl Fn(&str, u64) -> bool + Send + 'static {
        let control_plane = self.parallel_control_plane.clone();
        move |workspace_directory, epoch_id| {
            control_plane.automation_epoch_is_active(workspace_directory, epoch_id)
        }
    }

    pub(crate) fn parallel_control_plane_projection(
        &self,
    ) -> ParallelModeControlPlanePresentationProjection {
        client_control_plane_projection(self.parallel_control_plane.presentation_projection())
    }

    pub(crate) fn parallel_epoch_snapshot(&self) -> ParallelModeControlPlaneEpochSnapshot {
        client_epoch_snapshot(self.parallel_control_plane.epoch_snapshot())
    }

    pub(crate) fn current_parallel_epoch_id_for_workspace(
        &self,
        workspace_directory: &str,
    ) -> Option<u64> {
        self.parallel_control_plane
            .current_epoch_id_for_workspace(workspace_directory)
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn last_parallel_automation_trigger(&self) -> Option<ParallelModeAutomationTrigger> {
        self.parallel_control_plane.last_automation_trigger()
    }

    #[cfg(test)]
    pub(crate) fn begin_test_turn_submission(&mut self) -> TurnSubmissionCorrelation {
        self.core.runtime.begin_test_turn_submission()
    }

    #[cfg(test)]
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    pub(crate) fn parallel_control_effect_in_flight_for_test(&self) -> bool {
        self.parallel_control_plane.control_effect_in_flight()
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn parallel_supervisor_refresh_in_flight_for_test(&self) -> bool {
        self.parallel_control_plane.supervisor_refresh_in_flight()
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn parallel_orchestrator_wake_in_flight_for_test(&self) -> bool {
        self.parallel_control_plane.orchestrator_wake_in_flight()
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn parallel_supervisor_refresh_due_for_test(
        &self,
        now: Instant,
        activity_pulse_visible: bool,
    ) -> bool {
        self.parallel_control_plane
            .supervisor_refresh_due(now, activity_pulse_visible)
    }
}

impl NativeClientPort for NativeClientRuntime {
    fn run_host_cycle(
        &mut self,
        now: Instant,
        wait: Duration,
        max_inputs: usize,
    ) -> Vec<NativeClientDispatchOutcome> {
        NativeClientRuntime::run_host_cycle(self, now, wait, max_inputs)
    }

    fn dispatch_client_event(&mut self, event: NativeClientEvent) -> NativeClientDispatchOutcome {
        NativeClientRuntime::dispatch_client_event(self, event)
    }

    #[cfg(test)]
    fn poll_pending_client_event(&mut self) -> Option<NativeClientDispatchOutcome> {
        NativeClientRuntime::poll_pending_client_event(self)
    }

    fn snapshot(&self) -> AppSnapshot {
        NativeClientRuntime::snapshot(self)
    }

    fn revisioned_planning_parallel_projection(&self) -> RevisionedPlanningParallelProjection {
        NativeClientRuntime::revisioned_planning_parallel_projection(self)
    }

    fn parallel_mode_projection(&self) -> ParallelModeProjection {
        NativeClientRuntime::parallel_mode_projection(self)
    }

    fn parallel_mode_enabled(&self) -> bool {
        NativeClientRuntime::parallel_mode_enabled(self)
    }

    #[cfg(test)]
    fn current_parallel_epoch_id_for_workspace(&self, workspace_directory: &str) -> Option<u64> {
        NativeClientRuntime::current_parallel_epoch_id_for_workspace(self, workspace_directory)
    }

    fn parallel_control_plane_projection(&self) -> ParallelModeControlPlanePresentationProjection {
        NativeClientRuntime::parallel_control_plane_projection(self)
    }

    fn parallel_epoch_snapshot(&self) -> ParallelModeControlPlaneEpochSnapshot {
        NativeClientRuntime::parallel_epoch_snapshot(self)
    }

    #[cfg(test)]
    fn dispatch_parallel_completion_for_test(
        &mut self,
        event: ParallelModeControlPlaneBackgroundEvent,
    ) -> NativeClientDispatchOutcome {
        NativeClientRuntime::dispatch_parallel_completion_for_test(self, event)
    }

    #[cfg(test)]
    fn begin_test_turn_submission(&mut self) -> TurnSubmissionCorrelation {
        NativeClientRuntime::begin_test_turn_submission(self)
    }

    #[cfg(test)]
    fn force_parallel_mode_for_test(&self, workspace_directory: &str, enabled: bool) {
        NativeClientRuntime::force_parallel_mode_for_test(self, workspace_directory, enabled);
    }

    #[cfg(test)]
    fn force_parallel_epoch_for_test(&self, workspace_directory: &str, epoch_id: u64) {
        NativeClientRuntime::force_parallel_epoch_for_test(self, workspace_directory, epoch_id);
    }

    #[cfg(test)]
    fn parallel_automation_epoch_is_active_for_test(
        &self,
        workspace_directory: &str,
        epoch_id: u64,
    ) -> bool {
        NativeClientRuntime::parallel_automation_epoch_is_active_for_test(
            self,
            workspace_directory,
            epoch_id,
        )
    }

    #[cfg(test)]
    fn recv_parallel_completion_for_test(
        &self,
        timeout: Duration,
    ) -> ParallelModeControlPlaneBackgroundEvent {
        NativeClientRuntime::recv_parallel_completion_for_test(self, timeout)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::Cell;
    use std::sync::Arc;

    use super::*;
    use crate::adapter::inbound::tui::app::test_helpers::{
        test_parallel_mode_control_plane_composition, test_planning_services,
    };
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::core::app::PostTurnAuthoritySnapshot;
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
        PlanningWorkerPanelState, PostTurnExecution, PostTurnOutcome, PostTurnProvenance,
        PostTurnQueuedPrompt, RuntimeProjection, TurnSnapshotCapture,
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
    fn parallel_post_turn_route_requires_the_thread_policy_gate() {
        let mut snapshot = AppSnapshot::initial();
        let route_calls = Cell::new(0);

        snapshot
            .conversation_runtime
            .auto_follow
            .continuation_paused = true;
        let blocked = route_parallel_post_turn_when_allowed(&snapshot.conversation_runtime, || {
            route_calls.set(route_calls.get() + 1);
        });
        assert!(blocked.is_none());
        assert_eq!(route_calls.get(), 0);

        snapshot
            .conversation_runtime
            .auto_follow
            .parallel_rearmed_after_stop = true;
        let allowed = route_parallel_post_turn_when_allowed(&snapshot.conversation_runtime, || {
            route_calls.set(route_calls.get() + 1);
        });
        assert_eq!(allowed, Some(()));
        assert_eq!(route_calls.get(), 1);
    }

    #[test]
    fn a_stale_route_marker_does_nothing_without_an_awaiting_route() {
        // plan 19 Q2-2: the host takes its steps from the post-turn state, not from markers. A marker whose
        // evaluation no longer awaits a route cannot reach the parallel control plane, and it never reaches clients.
        let mut runtime = native_client_runtime_for_routing_test();
        let stale = post_turn_correlation(7);
        let outcome = CoreDispatchOutcome {
            events: vec![
                AppEvent::ParallelModeSupervisorSnapshotInvalidated,
                routing_event(&stale),
                AppEvent::PostTurnEvaluationDue {
                    planning_workspace_directory: "/workspace".to_string(),
                },
            ],
            effects: Vec::new(),
            snapshot: Arc::new(AppSnapshot::initial()),
        };

        let NativeClientDispatchOutcome::Core(reconciled) = runtime.reconcile_post_turn(outcome)
        else {
            panic!("no route was taken, so the outcome stays Core-only");
        };
        assert_eq!(
            reconciled.events,
            vec![AppEvent::ParallelModeSupervisorSnapshotInvalidated]
        );
        assert!(reconciled.effects.is_empty());
    }

    #[test]
    fn a_step_result_replaces_its_marker_before_later_events() {
        let post_turn = post_turn_correlation(7);
        let refresh = PlanningRuntimeRefreshCorrelation::new(3, "/workspace");
        let mut resolved_snapshot = AppSnapshot::initial();
        resolved_snapshot.revision = 2;
        let mut outcome = CoreDispatchOutcome {
            events: vec![
                AppEvent::ParallelModeSupervisorSnapshotInvalidated,
                routing_event(&post_turn),
                AppEvent::PlanningRuntimeRefreshStarted {
                    correlation: refresh.clone(),
                },
            ],
            effects: vec![CoreEffect::LoadParallelPeekConversation {
                correlation: ParallelPeekLoadCorrelation::new(1, "thread-initial"),
            }],
            snapshot: Arc::new(AppSnapshot::initial()),
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
        let anchor = route_marker_index(&outcome.events, &post_turn);
        assert_eq!(anchor, Some(1));
        splice_post_turn_step(&mut outcome, anchor, resolved);

        assert!(matches!(
            outcome.events.as_slice(),
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
        assert_eq!(outcome.effects.len(), 2);
        assert_eq!(outcome.snapshot.revision, 2);
    }

    #[test]
    fn anchors_match_the_step_and_a_missing_anchor_appends() {
        let current = post_turn_correlation(8);
        let events = vec![
            routing_event(&post_turn_correlation(7)),
            AppEvent::PostTurnEvaluationDue {
                planning_workspace_directory: "/other".to_string(),
            },
            AppEvent::PostTurnEvaluationDue {
                planning_workspace_directory: "/workspace".to_string(),
            },
        ];
        let route = PostTurnHostStep::RouteContinuation(Box::new(PostTurnRouteWork {
            correlation: current.clone(),
            execution: Box::new(post_turn_execution(&current)),
            runtime: AppSnapshot::initial().conversation_runtime,
        }));
        let evaluation = PostTurnHostStep::StartEvaluation {
            planning_workspace_directory: "/workspace".to_string(),
        };

        assert_eq!(post_turn_anchor_index(&events, &route), None);
        assert_eq!(post_turn_anchor_index(&events, &evaluation), Some(2));

        let mut outcome = CoreDispatchOutcome {
            events: vec![AppEvent::ParallelModeSupervisorSnapshotInvalidated],
            effects: Vec::new(),
            snapshot: Arc::new(AppSnapshot::initial()),
        };
        let appended = CoreDispatchOutcome {
            events: vec![AppEvent::ThreadActivityChanged(Vec::new())],
            effects: Vec::new(),
            snapshot: Arc::new(AppSnapshot::initial()),
        };
        splice_post_turn_step(&mut outcome, None, appended);
        assert_eq!(
            outcome.events,
            vec![
                AppEvent::ParallelModeSupervisorSnapshotInvalidated,
                AppEvent::ThreadActivityChanged(Vec::new()),
            ]
        );
    }

    #[test]
    fn missed_post_turn_markers_are_picked_up_on_the_next_dispatch() {
        // plan 19 Q2-2: core inputs that bypass the host drop their markers. The next host dispatch (a Tick)
        // still sees the due evaluation and then the awaiting route in the post-turn state and continues.
        let mut runtime = native_client_runtime_for_routing_test();
        let _ = runtime.dispatch_client_event(NativeClientEvent::core(CoreInput::Command(
            AppCommand::PausePostTurnContinuation,
        )));
        let turn_correlation = runtime.begin_test_turn_submission();
        for event in completed_turn_stream_events("thread-1", "turn-1") {
            let _ = runtime
                .core
                .runtime
                .dispatch_input(CoreInput::ConversationStreamUpdated {
                    correlation: turn_correlation,
                    event,
                });
        }
        assert!(matches!(
            runtime.core.runtime.next_post_turn_host_step(),
            Some(PostTurnHostStep::StartEvaluation { .. })
        ));

        let NativeClientDispatchOutcome::Core(started) =
            runtime.dispatch_client_event(NativeClientEvent::core(CoreInput::Tick {
                now: Instant::now(),
            }))
        else {
            panic!("starting an evaluation is a Core-only step");
        };
        assert!(
            started
                .events
                .iter()
                .any(|event| matches!(event, AppEvent::PostTurnEvaluationStarted(_))),
            "the started evaluation reaches clients after the Tick events: {:?}",
            started.events
        );
        let correlation = match &runtime.snapshot().conversation_runtime.post_turn {
            PostTurnAuthoritySnapshot::Evaluating { correlation, .. } => correlation.clone(),
            other => panic!("the next dispatch should start the evaluation: {other:?}"),
        };

        let _ = runtime
            .core
            .runtime
            .dispatch_input(CoreInput::EffectCompleted(
                CoreEffectCompletion::PostTurnEvaluationCompleted {
                    correlation: correlation.clone(),
                    execution: Box::new(post_turn_execution(&correlation)),
                },
            ));
        let NativeClientDispatchOutcome::Combined { core, .. } =
            runtime.dispatch_client_event(NativeClientEvent::core(CoreInput::Tick {
                now: Instant::now(),
            }))
        else {
            panic!("the next dispatch should route the awaiting evaluation");
        };
        assert!(core.events.iter().any(|event| matches!(
            event,
            AppEvent::PostTurnEvaluationCompleted { correlation: completed, .. }
                if completed == &correlation
        )));
        assert!(runtime.core.runtime.next_post_turn_host_step().is_none());
    }

    /// 호스트를 거치지 않고 core 에 직접 넣어 경로 대기를 만든다. 표시 이벤트는 그대로 버려진다.
    fn await_route_bypassing_host(
        runtime: &mut NativeClientRuntime,
        thread_id: &str,
        turn_id: &str,
    ) -> PostTurnEvaluationCorrelation {
        let turn_correlation = runtime.begin_test_turn_submission();
        for event in completed_turn_stream_events(thread_id, turn_id) {
            let _ = runtime
                .core
                .runtime
                .dispatch_input(CoreInput::ConversationStreamUpdated {
                    correlation: turn_correlation,
                    event,
                });
        }
        let started = runtime
            .core
            .runtime
            .dispatch_command(AppCommand::EvaluateCompletedTurn {
                parallel_mode_enabled: false,
                parallel_automation_epoch_id: None,
            });
        let [CoreEffect::EvaluatePostTurn { correlation, .. }] = started.effects.as_slice() else {
            panic!("the due evaluation should start: {:?}", started.effects);
        };
        let correlation = correlation.clone();
        let _ = runtime
            .core
            .runtime
            .dispatch_input(CoreInput::EffectCompleted(
                CoreEffectCompletion::PostTurnEvaluationCompleted {
                    correlation: correlation.clone(),
                    execution: Box::new(post_turn_execution(&correlation)),
                },
            ));
        correlation
    }

    #[test]
    fn the_host_settles_several_awaiting_routes_without_the_parallel_control_plane() {
        let mut runtime = native_client_runtime_for_routing_test();
        let background = await_route_bypassing_host(&mut runtime, "thread-1", "turn-1");
        let _ = runtime
            .core
            .runtime
            .dispatch_command(AppCommand::InvalidateConversationLoad);
        let shown = await_route_bypassing_host(&mut runtime, "thread-2", "turn-2");
        assert!(matches!(
            runtime.core.runtime.next_post_turn_host_step(),
            Some(PostTurnHostStep::SettleWithoutContinuation { correlations })
                if correlations == vec![shown.clone(), background.clone()]
        ));

        let NativeClientDispatchOutcome::Core(settled) =
            runtime.dispatch_client_event(NativeClientEvent::core(CoreInput::Tick {
                now: Instant::now(),
            }))
        else {
            panic!("settling without continuation never asks the parallel control plane");
        };
        assert!(settled.events.iter().any(|event| matches!(
            event,
            AppEvent::PostTurnEvaluationCompleted {
                correlation,
                route_resolution: PostTurnRouteResolution::NoContinuation,
                ..
            } if correlation == &shown
        )));
        assert!(runtime.core.runtime.next_post_turn_host_step().is_none());
    }

    #[test]
    fn a_stale_marker_is_dropped_while_the_awaiting_route_is_routed_at_the_end() {
        let mut runtime = native_client_runtime_for_routing_test();
        let _ = runtime.dispatch_client_event(NativeClientEvent::core(CoreInput::Command(
            AppCommand::PausePostTurnContinuation,
        )));
        let current = await_route_bypassing_host(&mut runtime, "thread-1", "turn-1");
        let stale = post_turn_correlation(99);
        let outcome = CoreDispatchOutcome {
            events: vec![
                routing_event(&stale),
                AppEvent::ParallelModeSupervisorSnapshotInvalidated,
            ],
            effects: Vec::new(),
            snapshot: Arc::new(AppSnapshot::initial()),
        };

        let NativeClientDispatchOutcome::Combined { core, .. } =
            runtime.reconcile_post_turn(outcome)
        else {
            panic!("the awaiting route is routed");
        };
        // 오래된 표시는 지워지고, 경로 결정 결과는 표시가 없으므로 원래 이벤트 뒤에 붙는다.
        assert!(matches!(
            core.events.first(),
            Some(AppEvent::ParallelModeSupervisorSnapshotInvalidated)
        ));
        assert!(core.events.iter().skip(1).any(|event| matches!(
            event,
            AppEvent::PostTurnEvaluationCompleted { correlation, .. } if correlation == &current
        )));
        assert!(
            !core.events.iter().any(|event| matches!(
                event,
                AppEvent::PostTurnContinuationRoutingRequested { .. }
            ))
        );
    }

    fn completed_turn_stream_events(thread_id: &str, turn_id: &str) -> [TurnStreamEvent; 3] {
        [
            TurnStreamEvent::ThreadPrepared {
                thread_id: thread_id.to_string(),
                title: "Native routing".to_string(),
                cwd: "/workspace".to_string(),
                runtime_envelope: Box::<ConversationRuntimeEnvelope>::default(),
            },
            TurnStreamEvent::TurnStarted {
                turn_id: turn_id.to_string(),
                runtime_request: Box::<ConversationRuntimeConfigurationRequest>::default(),
            },
            TurnStreamEvent::TurnTerminal {
                receipt: ConversationTurnTerminalReceipt::completed(thread_id, turn_id, Vec::new())
                    .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed),
                execution_snapshot_capture: Some(TurnSnapshotCapture::capture_failed(
                    "/workspace",
                    "test capture skipped".to_string(),
                )),
            },
        ]
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
        // 턴이 끝나면 호스트가 평가를 시작한다 (plan 18 K1-4).
        let correlation = match &runtime
            .core
            .runtime
            .snapshot()
            .conversation_runtime
            .post_turn
        {
            PostTurnAuthoritySnapshot::Evaluating { correlation, .. } => correlation.clone(),
            other => panic!("the host should start the post-turn evaluation: {other:?}"),
        };
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
            _image_paths: &[String],
            _options: ConversationTurnOptions,
            _event_sender: crate::application::port::conversation_stream::ConversationStreamSender,
        ) -> anyhow::Result<ConversationTurnTerminalReceipt> {
            unreachable!("routing test does not run turn effects")
        }

        fn run_turn_stream(
            &self,
            _thread_id: &str,
            _prompt: &str,
            _image_paths: &[String],
            _options: ConversationTurnOptions,
            _event_sender: crate::application::port::conversation_stream::ConversationStreamSender,
        ) -> anyhow::Result<ConversationTurnTerminalReceipt> {
            unreachable!("routing test does not run turn effects")
        }
    }

    /// 턴 실행 요청만 기록하는 포트이다. 실제 app-server 는 부르지 않는다.
    #[derive(Clone, Default)]
    struct RecordingTurnRuntimePort {
        turns: Arc<std::sync::Mutex<Vec<(String, String)>>>,
    }

    impl crate::application::port::outbound::startup_probe_port::StartupProbePort
        for RecordingTurnRuntimePort
    {
        fn load_startup_context(
            &self,
        ) -> anyhow::Result<
            crate::application::port::outbound::startup_probe_port::AppServerStartupContext,
        > {
            unreachable!("headless test does not run startup effects")
        }
    }

    impl crate::application::port::outbound::session_catalog_port::SessionCatalogPort
        for RecordingTurnRuntimePort
    {
        /// 커널 호스트 시험이 thread 를 지정할 수 있게 `thread-2` 하나를 알린다 (plan 18 K3-3b).
        fn load_session_catalog(
            &self,
            _request: SessionCatalogRequest,
        ) -> anyhow::Result<SessionCatalog> {
            Ok(crate::domain::recent_sessions::RecentSessions {
                items: vec![crate::domain::session_summary::SessionSummary {
                    id: "thread-2".to_string(),
                    name: Some("Listed thread".to_string()),
                    preview: String::new(),
                    cwd: "/workspace".to_string(),
                    source: "test".to_string(),
                    model_provider: "test".to_string(),
                    updated_at_epoch: 1,
                    status_type: "ready".to_string(),
                    path: String::new(),
                    git_branch: None,
                }],
                warnings: Vec::new(),
                next_cursor: None,
            }
            .into())
        }
    }

    impl crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort
        for RecordingTurnRuntimePort
    {
        fn runtime_control_truth(&self) -> ConversationRuntimeControlTruth {
            ConversationRuntimeControlTruth::codex_app_server()
        }

        fn load_conversation_snapshot(
            &self,
            _thread_id: &str,
        ) -> anyhow::Result<ConversationSnapshot> {
            unreachable!("headless test does not run conversation load effects")
        }

        fn request_stop_all_sessions(&self) -> anyhow::Result<()> {
            unreachable!("headless test does not run stop effects")
        }

        fn run_new_thread_stream(
            &self,
            _cwd: &str,
            _prompt: &str,
            _image_paths: &[String],
            _options: ConversationTurnOptions,
            _event_sender: crate::application::port::conversation_stream::ConversationStreamSender,
        ) -> anyhow::Result<ConversationTurnTerminalReceipt> {
            unreachable!("headless test continues an existing thread")
        }

        fn run_turn_stream(
            &self,
            thread_id: &str,
            prompt: &str,
            _image_paths: &[String],
            _options: ConversationTurnOptions,
            _event_sender: crate::application::port::conversation_stream::ConversationStreamSender,
        ) -> anyhow::Result<ConversationTurnTerminalReceipt> {
            self.turns
                .lock()
                .expect("recorded turns should lock")
                .push((thread_id.to_string(), prompt.to_string()));
            Err(anyhow::anyhow!("headless test stops after the follow-up turn starts"))
        }
    }

    /*
     * 헤드리스 시험이다 (plan 18 K1-6). TUI 없이 런타임 호스트의 주기만으로
     * 턴 종료 -> 평가 시작 -> AutoSubmit -> 다음 턴 제출이 이어지는지 본다.
     */
    #[test]
    fn host_cycle_runs_turn_end_evaluation_and_auto_follow_without_a_client() {
        run_auto_follow_loop_with_host_cycles(false);
    }

    #[test]
    fn the_host_reports_parallel_mode_before_a_thread_prompt() {
        // plan 18 K3-3b: a client cannot claim the parallel mode; the host reports its control plane.
        let (mut runtime, _turns) = recording_runtime_for_test();
        let NativeClientDispatchOutcome::Core(outcome) = runtime.dispatch_client_event(
            NativeClientEvent::core(CoreInput::Command(AppCommand::SubmitThreadPrompt(
                Box::new(crate::core::app::ThreadPromptSubmission {
                    workspace_directory: "/workspace".to_string(),
                    thread_id: "thread-9".to_string(),
                    prompt: "fix the test".to_string(),
                }),
            ))),
        ) else {
            panic!("a thread prompt is a core command");
        };
        assert!(
            outcome
                .events
                .contains(&AppEvent::ThreadPromptAdmissionResolved {
                    thread_id: "thread-9".to_string(),
                    admission: crate::core::app::ThreadPromptAdmission::RejectedUnknownThread,
                }),
            "parallel mode is off in the host, so core checks the thread: {:?}",
            outcome.events
        );
    }

    #[test]
    fn host_cycle_finishes_the_auto_follow_loop_of_a_conversation_the_client_left() {
        // plan 18 K2-3: the client opens a new draft while the turn runs. The left conversation
        // still ends, gets evaluated, and submits its follow-up turn in the background.
        run_auto_follow_loop_with_host_cycles(true);
    }

    /// 턴 실행 요청을 기록하는 런타임이다. 커널 호스트 시험도 쓴다 (plan 18 K3-2).
    pub(crate) type RecordedTurns = Arc<std::sync::Mutex<Vec<(String, String)>>>;

    pub(crate) fn recording_runtime_for_test() -> (NativeClientRuntime, RecordedTurns) {
        let planning = test_planning_services(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));
        let parallel = test_parallel_mode_control_plane_composition(planning.clone());
        let parallel_turns = parallel.parallel_mode_turn_service();
        let port = Arc::new(RecordingTurnRuntimePort::default());
        let turns = port.turns.clone();
        let runtime = NativeClientRuntime::new_for_test(
            StartupService::new(port.clone()),
            SessionService::new(port.clone()),
            ConversationService::new(port),
            planning,
            parallel_turns,
            parallel,
        );
        (runtime, turns)
    }

    fn run_auto_follow_loop_with_host_cycles(leave_conversation_while_running: bool) {
        let planning = test_planning_services(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));
        let parallel = test_parallel_mode_control_plane_composition(planning.clone());
        let parallel_turns = parallel.parallel_mode_turn_service();
        let port = Arc::new(RecordingTurnRuntimePort::default());
        let turns = port.turns.clone();
        let mut runtime = NativeClientRuntime::new_for_test(
            StartupService::new(port.clone()),
            SessionService::new(port.clone()),
            ConversationService::new(port),
            planning,
            parallel_turns,
            parallel,
        );
        let _ = runtime.dispatch_client_event(NativeClientEvent::core(CoreInput::Command(
            AppCommand::SetAutoFollowMaxTurns { value: 2 },
        )));
        let turn_correlation = runtime.begin_test_turn_submission();
        for event in [
            TurnStreamEvent::ThreadPrepared {
                thread_id: "thread-1".to_string(),
                title: "Headless loop".to_string(),
                cwd: "/workspace".to_string(),
                runtime_envelope: Box::<ConversationRuntimeEnvelope>::default(),
            },
            TurnStreamEvent::TurnStarted {
                turn_id: "turn-1".to_string(),
                runtime_request: Box::<ConversationRuntimeConfigurationRequest>::default(),
            },
        ] {
            let _ = runtime.dispatch_client_event(NativeClientEvent::core(
                CoreInput::ConversationStreamUpdated {
                    correlation: turn_correlation,
                    event,
                },
            ));
        }
        if leave_conversation_while_running {
            let _ = runtime.dispatch_client_event(NativeClientEvent::core(CoreInput::Command(
                AppCommand::InvalidateConversationLoad,
            )));
            assert!(
                runtime
                    .snapshot()
                    .conversation_runtime
                    .active_turn
                    .is_none()
            );
        }
        let _ = runtime.dispatch_client_event(NativeClientEvent::core(
            CoreInput::ConversationStreamUpdated {
                correlation: turn_correlation,
                event: TurnStreamEvent::TurnTerminal {
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
            },
        ));
        let correlation = match &runtime.snapshot().conversation_runtime.post_turn {
            PostTurnAuthoritySnapshot::Evaluating { correlation, .. } => correlation.clone(),
            // The shown draft has no evaluation; the left conversation evaluates its first one.
            PostTurnAuthoritySnapshot::Idle if leave_conversation_while_running => {
                post_turn_correlation(1)
            }
            other => panic!("the host should start the post-turn evaluation: {other:?}"),
        };
        let _ = runtime.dispatch_client_event(NativeClientEvent::core(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted {
                correlation: correlation.clone(),
                execution: Box::new(post_turn_execution(&correlation)),
            },
        )));

        let mut client_saw_auto_follow = false;
        let deadline = Instant::now() + WORKER_COMPLETION_TIMEOUT_FOR_HOST_TEST;
        while turns.lock().expect("recorded turns should lock").is_empty() {
            assert!(
                Instant::now() < deadline,
                "the host should submit the follow-up turn without a client"
            );
            for outcome in runtime.run_host_cycle(Instant::now(), Duration::from_millis(50), 16) {
                let (NativeClientDispatchOutcome::Core(core)
                | NativeClientDispatchOutcome::Combined { core, .. }) = outcome
                else {
                    continue;
                };
                client_saw_auto_follow |= core.events.iter().any(|event| {
                    matches!(event, AppEvent::AutoFollowTurnSubmissionResolved { .. })
                });
            }
        }
        assert_eq!(
            turns.lock().expect("recorded turns should lock").as_slice(),
            [("thread-1".to_string(), "continue".to_string())]
        );
        if leave_conversation_while_running {
            assert!(
                !client_saw_auto_follow,
                "the follow-up turn of a left conversation must not reach the shown transcript"
            );
        }
    }

    const WORKER_COMPLETION_TIMEOUT_FOR_HOST_TEST: Duration = Duration::from_secs(10);

    #[test]
    fn parallel_tick_waits_for_a_view_and_keeps_the_latest_one() {
        let mut runtime = native_client_runtime_for_routing_test();
        let NativeClientDispatchOutcome::Parallel(before_view) =
            runtime.dispatch_client_event(NativeClientEvent::ParallelTick {
                now: Instant::now(),
            })
        else {
            panic!("a parallel tick should route to the parallel lane");
        };
        assert!(
            before_view.presentation_events.is_empty(),
            "no workspace is known before the client reports its view"
        );

        let view = ParallelViewState {
            workspace_directory: "/workspace".to_string(),
            supervisor_view_active: false,
        };
        for _ in 0..2 {
            let _ =
                runtime.dispatch_client_event(NativeClientEvent::ParallelViewChanged(view.clone()));
        }
        assert_eq!(runtime.parallel_view.as_ref(), Some(&view));
    }

    #[test]
    fn host_cycle_starts_github_review_polling_setup_once_per_workspace_after_startup() {
        // plan 18 K2-6: the host, not the TUI first frame, decides when GitHub review setup starts.
        use crate::core::app::{
            GithubReviewPollingSettings, GithubReviewPollingSetupMode, StartupAttachmentSnapshot,
            StartupCheckCorrelation, StartupDiagnosticSnapshot, StartupReadySnapshot,
        };
        fn setup_count(outcomes: Vec<NativeClientDispatchOutcome>) -> usize {
            outcomes
                .into_iter()
                .filter(|outcome| {
                    let (NativeClientDispatchOutcome::Core(core)
                    | NativeClientDispatchOutcome::Combined { core, .. }) = outcome
                    else {
                        return false;
                    };
                    core.events.iter().any(|event| {
                        matches!(event, AppEvent::GithubReviewPollingSetupStarted { .. })
                    })
                })
                .count()
        }
        let diagnostic = |detail: &str| StartupDiagnosticSnapshot {
            ok: true,
            detail: detail.to_string(),
        };
        let mut runtime = native_client_runtime_for_routing_test();
        runtime.github_review_polling = Some(GithubReviewPollingSettings {
            mode: GithubReviewPollingSetupMode::Explicit {
                target: crate::domain::github_review::GithubPullRequestTarget::new(
                    "acme/widgets",
                    42,
                ),
            },
            poll_interval: Duration::from_secs(60),
        });

        assert_eq!(
            setup_count(runtime.run_host_cycle(Instant::now(), Duration::ZERO, 16)),
            0,
            "no setup before the startup checks succeed"
        );
        let _ = runtime.dispatch_client_event(NativeClientEvent::core(CoreInput::Command(
            AppCommand::RunStartupChecks {
                workspace_directory: "/tmp/workspace".to_string(),
            },
        )));
        let _ = runtime.dispatch_client_event(NativeClientEvent::core(CoreInput::EffectCompleted(
            CoreEffectCompletion::StartupChecksLoaded {
                correlation: StartupCheckCorrelation::new(1, "/tmp/workspace"),
                result: Ok(Box::new(StartupReadySnapshot {
                    cwd: "/tmp/workspace".to_string(),
                    workspace_path: "/tmp/workspace".to_string(),
                    can_continue: true,
                    codex_binary: diagnostic("/usr/bin/codex"),
                    workspace: diagnostic("git repo: /tmp/workspace"),
                    app_server_initialize: diagnostic("initialized"),
                    account: diagnostic("authenticated"),
                    attachment: StartupAttachmentSnapshot {
                        mode_label: "provider-launched".to_string(),
                        recovery_anchor_label: "provider-thread-id".to_string(),
                    },
                    warnings: Vec::new(),
                    schema_snapshot: "embedded schema".to_string(),
                })),
            },
        )));

        assert_eq!(
            setup_count(runtime.run_host_cycle(Instant::now(), Duration::ZERO, 16)),
            1,
            "the startup workspace starts the setup without a client"
        );
        assert_eq!(
            setup_count(runtime.run_host_cycle(Instant::now(), Duration::ZERO, 16)),
            0,
            "the same workspace does not start the setup again"
        );
        let _ = runtime.dispatch_client_event(NativeClientEvent::ParallelViewChanged(
            ParallelViewState {
                workspace_directory: "/tmp/other".to_string(),
                supervisor_view_active: false,
            },
        ));
        assert_eq!(
            setup_count(runtime.run_host_cycle(Instant::now(), Duration::ZERO, 16)),
            1,
            "a new shown workspace starts a new setup"
        );
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

    fn routing_event(correlation: &PostTurnEvaluationCorrelation) -> AppEvent {
        AppEvent::PostTurnContinuationRoutingRequested {
            correlation: correlation.clone(),
            execution: Box::new(post_turn_execution(correlation)),
        }
    }

    pub(crate) fn post_turn_correlation(generation: u64) -> PostTurnEvaluationCorrelation {
        PostTurnEvaluationCorrelation::new(
            generation,
            "thread-1",
            "turn-1",
            "/workspace",
            "/workspace",
        )
    }

    pub(crate) fn post_turn_execution(
        correlation: &PostTurnEvaluationCorrelation,
    ) -> PostTurnExecution {
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
