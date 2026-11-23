//! Completion workers share the repository's live Host owner. Verification records
//! execution facts before official refresh; it does not authorize delivery or Done.
use std::sync::Arc;

use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkPermitDecision, PlanningWorkPermitStage,
};
use crate::application::service::task_verification::{
    AutomaticTaskVerificationOutcome, TaskVerificationContinuation,
};
use crate::domain::parallel_mode::ParallelModeSlotLeaseSnapshot;
use crate::domain::planning::{
    PostTurnContinuationPermit, TaskAcceptanceChannel, validate_task_acceptance,
};

use super::ParallelModeService;

pub(super) enum AutomaticCompletionContinuation {
    Binding(
        Box<AutomaticCompletionContinuation>,
        super::binding_action::CapturedTaskAction,
    ),
    Epoch(super::ParallelModeAutomationPermit),
    PostTurn(PostTurnContinuationPermit),
    Unrestricted,
    Closed,
}

impl TaskVerificationContinuation for AutomaticCompletionContinuation {
    fn is_current(&self) -> bool {
        match self {
            Self::Binding(continuation, action) => action.is_current() && continuation.is_current(),
            Self::Epoch(permit) => permit.is_active(),
            Self::PostTurn(permit) => permit.is_current(),
            Self::Unrestricted => true,
            Self::Closed => false,
        }
    }

    fn with_current_commit(&self, operation: &mut dyn FnMut()) -> bool {
        match self {
            Self::Binding(continuation, action) => {
                let Ok(guard) = action.acquire() else {
                    return false;
                };
                if guard.validate().is_err() {
                    return false;
                }
                continuation.with_current_commit(operation)
            }
            Self::Epoch(permit) => permit.with_active_commit(operation).is_some(),
            Self::PostTurn(permit) => permit.with_current(operation).is_some(),
            Self::Unrestricted => {
                operation();
                true
            }
            Self::Closed => false,
        }
    }
}

#[cfg(test)]
thread_local! {
    static BEFORE_COMPLETION_COMMIT: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_before_automatic_completion_commit_hook(hook: impl FnOnce() + 'static) {
    BEFORE_COMPLETION_COMMIT.with(|slot| {
        assert!(slot.replace(Some(Box::new(hook))).is_none());
    });
}

#[cfg(test)]
fn run_before_automatic_completion_commit_hook() {
    BEFORE_COMPLETION_COMMIT.with(|slot| {
        if let Some(hook) = slot.take() {
            hook();
        }
    });
}

impl ParallelModeService {
    fn has_automatic_completion_recipe(
        &self,
        workspace: &str,
        expected: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<bool, String> {
        let Some(configuration) = self.task_evidence.as_ref() else {
            return Ok(false);
        };
        let Some(snapshot) = configuration
            .repository
            .load_task_authority_snapshot(workspace)
            .map_err(|_| "자동 호스트 검증의 완료 기준을 확인하지 못했습니다.".to_string())?
        else {
            return Ok(false);
        };
        let Some(task) = snapshot
            .task_authority
            .tasks
            .iter()
            .find(|task| task.id == expected.task_id)
        else {
            return Ok(false);
        };
        validate_task_acceptance(&task.acceptance, task.acceptance_revision)
            .map_err(|_| "자동 호스트 검증의 완료 기준이 유효하지 않습니다.".to_string())?;
        Ok(task.acceptance.iter().any(|criterion| {
            criterion.essential
                && criterion.channel == TaskAcceptanceChannel::Command
                && criterion.verification_recipe.is_some()
        }))
    }

    fn require_automatic_completion_current(
        &self,
        workspace: &str,
        expected: &ParallelModeSlotLeaseSnapshot,
        continuation: &dyn TaskVerificationContinuation,
    ) -> Result<(), String> {
        if !continuation.is_current() {
            return Err("자동 완료 처리가 중단되어 호스트 검증을 진행하지 않습니다.".into());
        }
        if self
            .planning_authority
            .worker_task_requires_preservation(workspace, &expected.task_id)
            .map_err(|_| "작업의 사용자 대기 상태를 확인하지 못했습니다.".to_string())?
        {
            return Err("사용자 답변을 기다리는 작업 공간을 보존합니다.".into());
        }
        let base = expected
            .delivery_target
            .as_ref()
            .ok_or_else(|| "작업의 고정된 검증 기준을 확인하지 못했습니다.".to_string())?;
        match self.planning_authority.check_work_order_permit_for_task(
            workspace,
            &expected.task_id,
            PlanningWorkPermitStage::Dispatch,
            Some(&base.integration_base_commit_sha),
        ) {
            Ok(PlanningWorkPermitDecision::Legacy | PlanningWorkPermitDecision::Granted { .. }) => {
                Ok(())
            }
            _ => Err("작업 실행 권한이 바뀌어 자동 호스트 검증을 보류했습니다.".into()),
        }
    }

    pub(crate) fn prepare_automatic_verification_for_official_completion(
        &self,
        workspace: &str,
        expected: &ParallelModeSlotLeaseSnapshot,
        completed_turn_id: &str,
        continuation: Arc<dyn TaskVerificationContinuation>,
    ) -> Result<AutomaticTaskVerificationOutcome, String> {
        // The ordinary post-turn path keeps its existing behavior when no essential
        // criterion explicitly selects a supported host recipe, including an unbound owner.
        if !self.has_automatic_completion_recipe(workspace, expected)? {
            return Ok(AutomaticTaskVerificationOutcome::NotApplicable);
        }
        self.require_automatic_completion_current(workspace, expected, continuation.as_ref())?;
        let source = self.prepare_workspace_worker_commit(expected)?;
        self.verify_host_commit_for_official_completion(
            workspace,
            expected,
            &source.commit_sha,
            completed_turn_id,
            continuation,
        )
    }

    pub(crate) fn verify_host_commit_for_official_completion(
        &self,
        workspace: &str,
        expected: &ParallelModeSlotLeaseSnapshot,
        source_commit_sha: &str,
        completed_turn_id: &str,
        continuation: Arc<dyn TaskVerificationContinuation>,
    ) -> Result<AutomaticTaskVerificationOutcome, String> {
        if !self.has_automatic_completion_recipe(workspace, expected)? {
            return Ok(AutomaticTaskVerificationOutcome::NotApplicable);
        }
        self.require_automatic_completion_current(workspace, expected, continuation.as_ref())?;
        let result = self
            .task_verification
            .verify_automatic_completion(
                workspace,
                expected,
                source_commit_sha,
                completed_turn_id,
                continuation.clone(),
            )
            .map_err(|_| {
                "호스트 검증의 최종 기록을 확인하지 못해 공식 완료 갱신을 보류했습니다.".to_string()
            })?;
        if matches!(
            result,
            AutomaticTaskVerificationOutcome::AwaitingUser { .. }
        ) {
            #[cfg(test)]
            run_before_automatic_completion_commit_hook();
            return Ok(result);
        }
        self.require_automatic_completion_current(workspace, expected, continuation.as_ref())?;
        #[cfg(test)]
        run_before_automatic_completion_commit_hook();
        Ok(result)
    }
}
