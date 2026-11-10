use super::planning_runtime::PlanningRuntimeCoordinator;
use super::planning_workspace::PlanningWorkspaceOperationCoordinator;
use super::{
    ManualPromptPreparationAdmission, ManualPromptPreparationIntent,
    PlanningRuntimeRefreshCorrelation, PlanningWorkspaceOperationAdmission,
    PlanningWorkspaceOperationCorrelation, PlanningWorkspaceOperationIntent,
    QueueMutationCorrelation, QueueMutationIntent,
};
use crate::domain::planning::{
    ExecutionSnapshot, ManualPromptCorrelation, ManualPromptRequest, PlanningWorkerPanelState,
    PlanningWorkerStatus, PostTurnExecution, PostTurnRequest, QueueIdlePolicy,
    RuntimeWorkspaceStatus,
};

#[derive(Debug, Clone)]
struct ActiveManualPromptPreparation {
    correlation: ManualPromptCorrelation,
    cancelled: bool,
    // thread 를 지정한 프롬프트이면 그 thread 이다 (plan 18 K3-3b). 준비가 끝나면 core 가 이 thread 에 제출한다.
    addressed_thread_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ManualPromptPreparationReduction {
    pub(super) admission: ManualPromptPreparationAdmission,
    pub(super) request: Option<Box<ManualPromptRequest>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ManualPromptCompletionDisposition {
    Publish,
    SuppressCancelled,
}

#[derive(Debug, Clone)]
pub(super) struct PlanningFeatureReducer {
    runtime_refresh: PlanningRuntimeCoordinator,
    workspace_operations: PlanningWorkspaceOperationCoordinator,
    next_queue_mutation_generation: u64,
    active_queue_mutation: Option<QueueMutationCorrelation>,
    next_manual_prompt_preparation_generation: u64,
    in_flight_manual_prompt_preparation: Option<ActiveManualPromptPreparation>,
    worker_panel_history_seed: PlanningWorkerPanelState,
    // 호스트가 알린 병렬 모드 값이다 (plan 18 K3-3b). 알리기 전에는 `None` 이다. thread 지정 프롬프트는 이 값으로
    // 준비를 받을지, 준비가 끝난 뒤 제출할지 정한다.
    host_parallel_mode: Option<bool>,
}

impl PlanningFeatureReducer {
    pub(super) fn new() -> Self {
        Self {
            runtime_refresh: PlanningRuntimeCoordinator::new(),
            workspace_operations: PlanningWorkspaceOperationCoordinator::new(),
            next_queue_mutation_generation: 1,
            active_queue_mutation: None,
            next_manual_prompt_preparation_generation: 1,
            in_flight_manual_prompt_preparation: None,
            worker_panel_history_seed: PlanningWorkerPanelState::default(),
            host_parallel_mode: None,
        }
    }

    pub(super) fn set_host_parallel_mode(&mut self, enabled: bool) {
        self.host_parallel_mode = Some(enabled);
    }

    pub(super) fn host_parallel_mode(&self) -> Option<bool> {
        self.host_parallel_mode
    }

    pub(super) fn begin_runtime_refresh(
        &mut self,
        workspace_directory: String,
    ) -> (
        PlanningRuntimeRefreshCorrelation,
        Option<PlanningRuntimeRefreshCorrelation>,
    ) {
        self.runtime_refresh.begin(workspace_directory)
    }

    pub(super) fn cancel_runtime_refresh(&mut self) -> Option<PlanningRuntimeRefreshCorrelation> {
        self.runtime_refresh.cancel()
    }

    pub(super) fn accept_runtime_refresh(
        &mut self,
        correlation: &PlanningRuntimeRefreshCorrelation,
    ) -> bool {
        self.runtime_refresh.accept(correlation)
    }

    pub(super) fn runtime_refresh_has_active(&self) -> bool {
        self.runtime_refresh.has_active()
    }

    pub(super) fn runtime_refresh_matches_workspace(&self, workspace_directory: &str) -> bool {
        self.runtime_refresh.matches_workspace(workspace_directory)
    }

    pub(super) fn restart_runtime_refresh_if_matches(
        &mut self,
        workspace_directory: &str,
    ) -> Option<(
        PlanningRuntimeRefreshCorrelation,
        PlanningRuntimeRefreshCorrelation,
    )> {
        self.runtime_refresh.restart_if_matches(workspace_directory)
    }

    pub(super) fn begin_workspace_operation(
        &mut self,
        intent: PlanningWorkspaceOperationIntent,
    ) -> PlanningWorkspaceOperationAdmission {
        self.workspace_operations.begin(intent)
    }

    pub(super) fn accept_workspace_operation(
        &mut self,
        correlation: &PlanningWorkspaceOperationCorrelation,
    ) -> bool {
        self.workspace_operations.accept(correlation)
    }

    pub(super) fn begin_queue_mutation(
        &mut self,
        intent: QueueMutationIntent,
    ) -> Option<QueueMutationCorrelation> {
        if self.active_queue_mutation.is_some() {
            return None;
        }
        let generation =
            take_generation(&mut self.next_queue_mutation_generation, "queue mutation");
        let correlation = QueueMutationCorrelation::new(generation, intent);
        self.active_queue_mutation = Some(correlation.clone());
        Some(correlation)
    }

    pub(super) fn complete_queue_mutation(
        &mut self,
        correlation: &QueueMutationCorrelation,
    ) -> bool {
        if self.active_queue_mutation.as_ref() != Some(correlation) {
            return false;
        }
        self.active_queue_mutation = None;
        true
    }

    pub(super) fn begin_manual_prompt_preparation(
        &mut self,
        intent: ManualPromptPreparationIntent,
    ) -> ManualPromptPreparationReduction {
        if let Some(active) = &self.in_flight_manual_prompt_preparation {
            return ManualPromptPreparationReduction {
                admission: ManualPromptPreparationAdmission::RejectedActive {
                    active_correlation: active.correlation.clone(),
                },
                request: None,
            };
        }

        let generation = take_generation(
            &mut self.next_manual_prompt_preparation_generation,
            "manual prompt preparation",
        );
        let ManualPromptPreparationIntent {
            workspace_directory,
            raw_prompt,
            parent_thread_id,
            parent_turn_id,
        } = intent;
        let correlation = ManualPromptCorrelation {
            request_id: generation,
            generation,
            workspace_directory,
        };
        let request = ManualPromptRequest {
            correlation: correlation.clone(),
            raw_prompt,
            parent_thread_id,
            parent_turn_id,
        };
        self.in_flight_manual_prompt_preparation = Some(ActiveManualPromptPreparation {
            correlation: correlation.clone(),
            cancelled: false,
            addressed_thread_id: None,
        });
        ManualPromptPreparationReduction {
            admission: ManualPromptPreparationAdmission::Accepted { correlation },
            request: Some(Box::new(request)),
        }
    }

    /// thread 를 지정한 프롬프트의 준비를 시작한다 (plan 18 K3-3b). 준비 칸은 TUI 의 준비와 같이 쓴다.
    pub(super) fn begin_thread_prompt_preparation(
        &mut self,
        intent: ManualPromptPreparationIntent,
        thread_id: String,
    ) -> ManualPromptPreparationReduction {
        let reduction = self.begin_manual_prompt_preparation(intent);
        if let (ManualPromptPreparationAdmission::Accepted { .. }, Some(active)) = (
            &reduction.admission,
            self.in_flight_manual_prompt_preparation.as_mut(),
        ) {
            active.addressed_thread_id = Some(thread_id);
        }
        reduction
    }

    /// 이 준비가 thread 를 지정한 프롬프트의 것이면 그 thread 이다 (plan 18 K3-3b).
    pub(super) fn manual_prompt_preparation_thread(
        &self,
        correlation: &ManualPromptCorrelation,
    ) -> Option<String> {
        self.in_flight_manual_prompt_preparation
            .as_ref()
            .filter(|active| &active.correlation == correlation)
            .and_then(|active| active.addressed_thread_id.clone())
    }

    /*
     * TUI 가 자기 준비를 취소한다. thread 를 지정한 준비는 원격 클라이언트의 것이라 취소하지 않는다 (plan 18 K3-3b).
     * TUI 는 대화를 바꿀 때마다 이 명령을 보내므로, 취소하면 원격 제출이 실패하고 만든 planning 작업만 남는다.
     */
    pub(super) fn cancel_manual_prompt_preparation(&mut self) -> Option<ManualPromptCorrelation> {
        let active = self.in_flight_manual_prompt_preparation.as_mut()?;
        if active.cancelled || active.addressed_thread_id.is_some() {
            return None;
        }
        active.cancelled = true;
        Some(active.correlation.clone())
    }

    pub(super) fn complete_manual_prompt_preparation(
        &mut self,
        correlation: &ManualPromptCorrelation,
    ) -> Option<ManualPromptCompletionDisposition> {
        let active = self.in_flight_manual_prompt_preparation.as_ref()?;
        if &active.correlation != correlation {
            return None;
        }
        let disposition = if active.cancelled {
            ManualPromptCompletionDisposition::SuppressCancelled
        } else {
            ManualPromptCompletionDisposition::Publish
        };
        self.in_flight_manual_prompt_preparation = None;
        Some(disposition)
    }

    pub(super) fn begin_post_turn_worker_panel(
        &self,
        request: &PostTurnRequest,
    ) -> PlanningWorkerPanelState {
        let mut state = self.worker_panel_history_seed.clone();
        if request.context.planning_settlement_paused {
            return state;
        }
        if request
            .changed_planning_file_paths
            .iter()
            .any(|path| ExecutionSnapshot::captures_path(path))
        {
            state.status = PlanningWorkerStatus::RepairRunning;
            return state;
        }
        if request
            .context
            .current_runtime_projection
            .workspace_status()
            == RuntimeWorkspaceStatus::ReadyNoTask
            && request
                .context
                .current_runtime_projection
                .queue_idle_policy()
                == QueueIdlePolicy::Stop
        {
            return state;
        }
        state.status = PlanningWorkerStatus::RefreshRunning;
        state
    }

    pub(super) fn accept_post_turn_worker_panel(&mut self, execution: &PostTurnExecution) {
        self.worker_panel_history_seed = execution.planning_worker_panel_state.clone();
    }

    pub(super) fn reset_worker_panel_history(&mut self) {
        self.worker_panel_history_seed = PlanningWorkerPanelState::default();
    }

    #[cfg(test)]
    pub(super) fn exhaust_runtime_refresh_generation(&mut self) {
        self.runtime_refresh.exhaust_generation();
    }

    #[cfg(test)]
    pub(super) fn exhaust_queue_mutation_generation(&mut self) {
        self.next_queue_mutation_generation = u64::MAX;
    }

    #[cfg(test)]
    pub(super) fn exhaust_manual_prompt_preparation_generation(&mut self) {
        self.next_manual_prompt_preparation_generation = u64::MAX;
    }

    #[cfg(test)]
    pub(super) fn active_queue_mutation(&self) -> Option<&QueueMutationCorrelation> {
        self.active_queue_mutation.as_ref()
    }

    #[cfg(test)]
    pub(super) fn manual_prompt_preparation_is_active(&self) -> bool {
        self.in_flight_manual_prompt_preparation.is_some()
    }

    /// 끝나지 않은 planning 작업(프롬프트 준비, 큐 변경, 작업 공간 작업)이 있다 (plan 18 K3-5 커널 내리기).
    pub(super) fn has_work_in_flight(&self) -> bool {
        self.in_flight_manual_prompt_preparation.is_some()
            || self.active_queue_mutation.is_some()
            || self.workspace_operations.is_active()
    }

    #[cfg(test)]
    pub(super) fn manual_prompt_preparation_is_cancelled(&self) -> bool {
        self.in_flight_manual_prompt_preparation
            .as_ref()
            .is_some_and(|active| active.cancelled)
    }

    #[cfg(test)]
    pub(super) fn worker_panel_history(&self) -> &PlanningWorkerPanelState {
        &self.worker_panel_history_seed
    }

    #[cfg(test)]
    pub(super) fn replace_worker_panel_history(&mut self, state: PlanningWorkerPanelState) {
        self.worker_panel_history_seed = state;
    }
}

impl Default for PlanningFeatureReducer {
    fn default() -> Self {
        Self::new()
    }
}

fn take_generation(next_generation: &mut u64, operation: &str) -> u64 {
    let generation = *next_generation;
    *next_generation = generation
        .checked_add(1)
        .unwrap_or_else(|| panic!("{operation} generation exhausted"));
    generation
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::{QueueMutationKind, QueueMutationTarget};

    fn queue_intent(workspace_directory: &str) -> QueueMutationIntent {
        QueueMutationIntent {
            workspace_directory: workspace_directory.to_string(),
            active_thread_id: Some("thread-1".to_string()),
            kind: QueueMutationKind::RemoveSelected,
            expected_planning_revision: 7,
            targets: vec![QueueMutationTarget {
                task_id: "task-1".to_string(),
                expected_status: crate::domain::planning::TaskStatus::Ready,
                expected_updated_at: "2026-07-28T00:00:00Z".to_string(),
            }],
            receipt_at_start: None,
        }
    }

    fn manual_intent(workspace_directory: &str, prompt: &str) -> ManualPromptPreparationIntent {
        ManualPromptPreparationIntent {
            workspace_directory: workspace_directory.to_string(),
            raw_prompt: prompt.to_string(),
            parent_thread_id: Some("thread-1".to_string()),
            parent_turn_id: Some("turn-1".to_string()),
        }
    }

    fn accepted_manual_correlation(
        reduction: &ManualPromptPreparationReduction,
    ) -> ManualPromptCorrelation {
        let ManualPromptPreparationAdmission::Accepted { correlation } = &reduction.admission
        else {
            panic!("expected accepted manual prompt preparation");
        };
        correlation.clone()
    }

    #[test]
    fn queue_mutation_completion_is_exact_once_across_same_intent_aba() {
        let mut reducer = PlanningFeatureReducer::new();
        let first = reducer
            .begin_queue_mutation(queue_intent("/workspace"))
            .expect("first mutation should start");
        assert_eq!(first.generation, 1);
        assert!(
            reducer
                .begin_queue_mutation(queue_intent("/workspace"))
                .is_none(),
            "the physical single-flight lease must coalesce concurrent dispatch"
        );

        let stale = QueueMutationCorrelation::new(99, queue_intent("/workspace"));
        assert!(!reducer.complete_queue_mutation(&stale));
        assert_eq!(reducer.active_queue_mutation(), Some(&first));
        assert!(reducer.complete_queue_mutation(&first));
        assert!(!reducer.complete_queue_mutation(&first));

        let same_intent_again = reducer
            .begin_queue_mutation(queue_intent("/workspace"))
            .expect("the settled gate should reopen");
        assert_eq!(same_intent_again.generation, 2);
        assert!(!reducer.complete_queue_mutation(&first));
        assert!(reducer.complete_queue_mutation(&same_intent_again));
    }

    #[test]
    fn manual_prompt_cancellation_retains_lease_until_exact_settlement() {
        let mut reducer = PlanningFeatureReducer::new();
        let first = reducer.begin_manual_prompt_preparation(manual_intent("/workspace", "first"));
        let first_correlation = accepted_manual_correlation(&first);
        assert_eq!(
            first.request.as_ref().map(|request| &request.correlation),
            Some(&first_correlation)
        );

        let rejected =
            reducer.begin_manual_prompt_preparation(manual_intent("/other", "concurrent"));
        assert_eq!(
            rejected.admission,
            ManualPromptPreparationAdmission::RejectedActive {
                active_correlation: first_correlation.clone(),
            }
        );
        assert!(rejected.request.is_none());

        assert_eq!(
            reducer.cancel_manual_prompt_preparation(),
            Some(first_correlation.clone())
        );
        assert!(reducer.cancel_manual_prompt_preparation().is_none());
        assert!(reducer.manual_prompt_preparation_is_cancelled());

        let stale = ManualPromptCorrelation {
            request_id: 99,
            generation: 99,
            workspace_directory: "/workspace".to_string(),
        };
        assert!(reducer.complete_manual_prompt_preparation(&stale).is_none());
        assert!(reducer.manual_prompt_preparation_is_active());
        assert_eq!(
            reducer.complete_manual_prompt_preparation(&first_correlation),
            Some(ManualPromptCompletionDisposition::SuppressCancelled)
        );
        assert!(
            reducer
                .complete_manual_prompt_preparation(&first_correlation)
                .is_none()
        );

        let second = reducer.begin_manual_prompt_preparation(manual_intent("/workspace", "second"));
        let second_correlation = accepted_manual_correlation(&second);
        assert_eq!(second_correlation.generation, 2);
        assert_ne!(second_correlation, first_correlation);
        assert_eq!(
            reducer.complete_manual_prompt_preparation(&second_correlation),
            Some(ManualPromptCompletionDisposition::Publish)
        );
    }

    #[test]
    #[should_panic(expected = "queue mutation generation exhausted")]
    fn queue_mutation_generation_fails_before_wraparound() {
        let mut reducer = PlanningFeatureReducer::new();
        reducer.exhaust_queue_mutation_generation();
        let _ = reducer.begin_queue_mutation(queue_intent("/workspace"));
    }

    #[test]
    #[should_panic(expected = "manual prompt preparation generation exhausted")]
    fn manual_prompt_generation_fails_before_wraparound() {
        let mut reducer = PlanningFeatureReducer::new();
        reducer.exhaust_manual_prompt_preparation_generation();
        let _ = reducer.begin_manual_prompt_preparation(manual_intent("/workspace", "exhausted"));
    }
}
