//! Inbound client contract for the framework-free Core runtime.

use std::time::Instant;

#[cfg(test)]
use crate::core::app::TurnSubmissionCorrelation;
use crate::core::app::{
    AppSnapshot, CoreDispatchOutcome, CoreInput, ParallelModeProjection,
    RevisionedPlanningParallelProjection,
};
use crate::domain::parallel_mode::{
    ParallelModeAutomationTrigger, ParallelModeReadinessSnapshot, ParallelModeSupervisorSnapshot,
};

/*
 * NativeClientPort is the single application entry boundary exposed to the
 * native TUI. Composition may own concrete services and control-plane workers,
 * but the driving adapter only dispatches commands and reads immutable client
 * projections through this contract.
 */
pub(crate) trait NativeClientPort: Send {
    #[must_use = "dispatch outcomes carry projection events that the inbound adapter must apply"]
    fn dispatch_client_event(&mut self, event: NativeClientEvent) -> NativeClientDispatchOutcome;

    // 시험 전용이다. 화면은 `run_host_cycle` 을 쓴다 (plan 18 K1-6).
    #[cfg(test)]
    fn poll_pending_client_event(&mut self) -> Option<NativeClientDispatchOutcome>;
    /*
     * 런타임 호스트의 한 주기이다 (plan 18 K1-6). 입력을 최대 `wait` 만큼 기다리고, 와 있는 입력을 최대
     * `max_inputs` 개 처리한 뒤 core 와 병렬 제어부에 시계를 넣는다. 화면은 0 으로 부르고, 화면 없는
     * 호스트는 기다린다.
     */
    fn run_host_cycle(
        &mut self,
        now: Instant,
        wait: std::time::Duration,
        max_inputs: usize,
    ) -> Vec<NativeClientDispatchOutcome>;
    fn snapshot(&self) -> AppSnapshot;
    fn revisioned_planning_parallel_projection(&self) -> RevisionedPlanningParallelProjection;
    fn parallel_mode_projection(&self) -> ParallelModeProjection;
    fn parallel_mode_enabled(&self) -> bool;
    fn parallel_control_plane_projection(&self) -> ParallelModeControlPlanePresentationProjection;
    fn parallel_epoch_snapshot(&self) -> ParallelModeControlPlaneEpochSnapshot;
    // 시험 전용이다. 평가 요청의 epoch 는 호스트가 직접 붙인다 (plan 18 K1-4).
    #[cfg(test)]
    fn current_parallel_epoch_id_for_workspace(&self, workspace_directory: &str) -> Option<u64>;

    #[cfg(test)]
    fn dispatch_parallel_completion_for_test(
        &mut self,
        event: crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneBackgroundEvent,
    ) -> NativeClientDispatchOutcome;

    #[cfg(test)]
    fn begin_test_turn_submission(&mut self) -> TurnSubmissionCorrelation;

    #[cfg(test)]
    fn force_parallel_mode_for_test(&self, workspace_directory: &str, enabled: bool);

    #[cfg(test)]
    fn force_parallel_epoch_for_test(&self, workspace_directory: &str, epoch_id: u64);

    #[cfg(test)]
    fn parallel_automation_epoch_is_active_for_test(
        &self,
        workspace_directory: &str,
        epoch_id: u64,
    ) -> bool;

    #[cfg(test)]
    fn recv_parallel_completion_for_test(
        &self,
        timeout: std::time::Duration,
    ) -> crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneBackgroundEvent;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParallelViewState {
    pub(crate) workspace_directory: String,
    pub(crate) supervisor_view_active: bool,
}

pub(crate) enum NativeClientEvent {
    Core(Box<CoreInput>),
    ParallelCommand(ParallelModeControlPlaneCommand),
    // 호스트 시계이다 (plan 18 K1-2). 깨우기 폴링과 정리 재시도는 병렬 제어부가 tick 마다 스스로 판단한다.
    ParallelTick { now: Instant },
    // 클라이언트가 보는 병렬 작업 공간과 supervisor 화면 상태이다. supervisor 새로 고침은 이 화면이 열려 있고
    // 활동이 있을 때만 한다. 같은 값이 다시 와도 결과는 같다.
    ParallelViewChanged(ParallelViewState),
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

impl NativeParallelDispatchOutcome {
    pub(crate) fn from_events(
        presentation_events: Vec<ParallelModeControlPlanePresentationEvent>,
    ) -> Self {
        Self {
            presentation_events,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParallelModeControlPlaneCommand {
    OpenEpoch {
        workspace_directory: String,
    },
    Enable {
        workspace_directory: String,
    },
    Disable {
        workspace_directory: String,
    },
    InspectSupervisor {
        workspace_directory: String,
        reconcile_pool: bool,
        show_status: bool,
    },
    RefreshSupervisor {
        workspace_directory: String,
    },
    RequestDispatch {
        workspace_directory: String,
        trigger: ParallelModeAutomationTrigger,
    },
    RequestDispatchForEpoch {
        workspace_directory: String,
        trigger: ParallelModeAutomationTrigger,
        epoch_id: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParallelModeControlPlaneLoadingStage {
    ReconcilingPool,
}

#[derive(Debug, Clone)]
pub(crate) enum ParallelModeControlPlanePresentationEvent {
    EnterProgress {
        workspace_directory: String,
        readiness_snapshot: Option<ParallelModeReadinessSnapshot>,
        loading_stage: ParallelModeControlPlaneLoadingStage,
        status_text: String,
    },
    ReadinessSnapshotChanged {
        workspace_directory: String,
        snapshot: ParallelModeReadinessSnapshot,
    },
    SupervisorSnapshotChanged {
        workspace_directory: String,
        snapshot: Box<ParallelModeSupervisorSnapshot>,
    },
    StatusShown {
        workspace_directory: String,
        status_text: String,
    },
    ConversationRuntimeNotice {
        workspace_directory: String,
        notice: String,
    },
    GlobalRuntimeNoticesChanged,
    PostTurnAutoFollowPromptConsumed,
    PlanningRuntimeRefreshRequested {
        workspace_directory: String,
    },
    ModeDisabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParallelModeDispatchCleanupCorrelation {
    pub(crate) operation_id: u64,
    pub(crate) workspace_directory: String,
    pub(crate) epoch_id: u64,
    pub(crate) command_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParallelModeGlobalRuntimeNoticeProjection {
    pub(crate) cleanup_correlation: ParallelModeDispatchCleanupCorrelation,
    pub(crate) notice: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParallelModeControlPlanePresentationProjection {
    pub(crate) mode_enabled: bool,
    pub(crate) control_effect_in_flight: bool,
    pub(crate) last_dispatch_withheld_reason: Option<String>,
    pub(crate) global_runtime_notices: Vec<ParallelModeGlobalRuntimeNoticeProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParallelModeControlPlaneEpochSnapshot {
    pub(crate) workspace_directory: Option<String>,
    pub(crate) current_epoch_id: Option<u64>,
}
