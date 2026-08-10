use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;

use crate::application::port::inbound::pr_validation_command_port::{
    PrValidationAdminCommandAction, PrValidationAdminCommandRejection,
    PrValidationAdminCommandRequest, PrValidationAdminCommandResult, PrValidationAdminCommandState,
    PrValidationCommandPort,
};
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityPort, PrValidationAuthorityAdminAction,
    PrValidationAuthorityAdminCommandOutcome, PrValidationAuthorityAdminCommandRejection,
    PrValidationAuthorityAdminCommandRequest, PrValidationAuthorityAdminCommandState,
};
use crate::domain::parallel_mode::{PrValidationRecordKey, PrValidationSchedulerMode};

pub struct PrValidationCommandService {
    workspace_dir: String,
    planning_authority: Arc<dyn PlanningAuthorityPort>,
    scheduler_mode: PrValidationSchedulerMode,
}

impl PrValidationCommandService {
    pub fn new(
        workspace_dir: impl Into<String>,
        planning_authority: Arc<dyn PlanningAuthorityPort>,
        scheduler_mode: PrValidationSchedulerMode,
    ) -> Self {
        Self {
            workspace_dir: workspace_dir.into(),
            planning_authority,
            scheduler_mode,
        }
    }
}

fn map_admin_action(action: PrValidationAdminCommandAction) -> PrValidationAuthorityAdminAction {
    match action {
        PrValidationAdminCommandAction::RetryNow => PrValidationAuthorityAdminAction::RetryNow,
        PrValidationAdminCommandAction::Pause => PrValidationAuthorityAdminAction::Pause,
        PrValidationAdminCommandAction::Resume => PrValidationAuthorityAdminAction::Resume,
        PrValidationAdminCommandAction::QueueRemediation => {
            PrValidationAuthorityAdminAction::QueueRemediation
        }
        PrValidationAdminCommandAction::Acknowledge => {
            PrValidationAuthorityAdminAction::Acknowledge
        }
    }
}

fn map_admin_outcome(
    action: PrValidationAdminCommandAction,
    outcome: PrValidationAuthorityAdminCommandOutcome,
) -> PrValidationAdminCommandResult {
    PrValidationAdminCommandResult {
        command_id: outcome.command_id,
        record_key: outcome.record_key,
        action,
        state: match outcome.state {
            PrValidationAuthorityAdminCommandState::Applied => {
                PrValidationAdminCommandState::Applied
            }
            PrValidationAuthorityAdminCommandState::Rejected => {
                PrValidationAdminCommandState::Rejected
            }
        },
        rejection: outcome.rejection.map(|rejection| match rejection {
            PrValidationAuthorityAdminCommandRejection::NotFound => {
                PrValidationAdminCommandRejection::NotFound
            }
            PrValidationAuthorityAdminCommandRejection::StaleRevision => {
                PrValidationAdminCommandRejection::StaleRevision
            }
            PrValidationAuthorityAdminCommandRejection::IdempotencyConflict => {
                PrValidationAdminCommandRejection::IdempotencyConflict
            }
            PrValidationAuthorityAdminCommandRejection::ObserveModeAdmission => {
                PrValidationAdminCommandRejection::ObserveModeAdmission
            }
            PrValidationAuthorityAdminCommandRejection::InvalidState => {
                PrValidationAdminCommandRejection::InvalidState
            }
            PrValidationAuthorityAdminCommandRejection::RateLimitActive => {
                PrValidationAdminCommandRejection::RateLimitActive
            }
            PrValidationAuthorityAdminCommandRejection::NoActionableFinding => {
                PrValidationAdminCommandRejection::NoActionableFinding
            }
        }),
        duplicate: outcome.duplicate,
        expected_revision: outcome.expected_observation_revision,
        observed_revision: outcome.observed_revision,
        board_revision: outcome.board_revision,
        message: outcome.message,
        applied_at: outcome.applied_at,
    }
}

impl PrValidationCommandPort for PrValidationCommandService {
    fn execute(
        &self,
        request: PrValidationAdminCommandRequest,
    ) -> Result<PrValidationAdminCommandResult> {
        let record_key =
            PrValidationRecordKey::new(&request.record_key).map_err(anyhow::Error::msg)?;
        let action = map_admin_action(request.action);
        let outcome = self
            .planning_authority
            .execute_runtime_pr_validation_admin_command(
                &self.workspace_dir,
                PrValidationAuthorityAdminCommandRequest {
                    command_id: &request.command_id,
                    record_key: &record_key,
                    action,
                    expected_observation_revision: request.expected_revision,
                    requested_at: Utc::now(),
                    remediation_admission_allowed: self.scheduler_mode.admits_remediation(),
                },
            )
            .context("failed to execute PR validation Admin command")?;
        Ok(map_admin_outcome(request.action, outcome))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authority_outcome_maps_observe_mode_rejection_without_losing_revision_evidence() {
        let result = map_admin_outcome(
            PrValidationAdminCommandAction::QueueRemediation,
            PrValidationAuthorityAdminCommandOutcome {
                command_id: "command-1".to_string(),
                record_key: "record-1".to_string(),
                action: PrValidationAuthorityAdminAction::QueueRemediation,
                state: PrValidationAuthorityAdminCommandState::Rejected,
                rejection: Some(PrValidationAuthorityAdminCommandRejection::ObserveModeAdmission),
                duplicate: false,
                expected_observation_revision: 3,
                observed_revision: Some(3),
                board_revision: 8,
                message: "observe mode blocks remediation admission".to_string(),
                applied_at: "2026-08-10T00:00:00Z".to_string(),
            },
        );
        assert_eq!(
            result.rejection,
            Some(PrValidationAdminCommandRejection::ObserveModeAdmission)
        );
        assert_eq!(result.expected_revision, 3);
        assert_eq!(result.observed_revision, Some(3));
        assert_eq!(result.board_revision, 8);
    }
}
