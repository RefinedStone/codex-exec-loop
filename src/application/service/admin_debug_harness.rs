use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;
use chrono::Utc;

use crate::application::port::inbound::admin_debug_port::AdminDebugStage;
use crate::application::port::inbound::admin_debug_port::{
    AdminDebugHarnessCommand, AdminDebugHarnessConfig, AdminDebugHarnessError,
    AdminDebugHarnessProjection, AdminDebugPort, AdminDebugScenario, AdminDebugScenarioOption,
    AdminDebugStageRecord,
};
use crate::application::port::inbound::pr_validation_command_port::{
    PrValidationAdminCommandAction, PrValidationAdminCommandRejection,
    PrValidationAdminCommandRequest, PrValidationAdminCommandResult, PrValidationAdminCommandState,
    PrValidationCommandPort,
};
use crate::application::port::inbound::pr_validation_query_port::{
    PrValidationAdminCheck, PrValidationAdminCheckStatus, PrValidationAdminCommandAvailability,
    PrValidationAdminCorrelation, PrValidationAdminPhase, PrValidationAdminProvider,
    PrValidationAdminRecord, PrValidationAdminSchedule, PrValidationAdminSeverity,
    PrValidationAdminTimelineEntry, PrValidationAdminWorkflow, PrValidationBoardRequest,
    PrValidationBoardSnapshot, PrValidationBoardSummary, PrValidationDetailRequest,
    PrValidationQueryPort, PrValidationRolloutSnapshot, PrValidationStatusRequest,
};
use crate::domain::parallel_mode::{PrValidationOperatorSummary, PrValidationSchedulerMode};

#[derive(Debug, Clone)]
struct StoredDebugValidationCommand {
    fingerprint: String,
    result: PrValidationAdminCommandResult,
}

#[derive(Debug)]
struct AdminDebugHarnessState {
    playing: bool,
    scenario: AdminDebugScenario,
    stage_index: usize,
    revision: u64,
    run_id: u64,
    last_transition: Instant,
    validation_paused: bool,
    validation_acknowledged_at: Option<String>,
    validation_commands: BTreeMap<String, StoredDebugValidationCommand>,
}

#[derive(Debug, Clone)]
pub struct AdminDebugHarnessService {
    config: AdminDebugHarnessConfig,
    state: Arc<Mutex<AdminDebugHarnessState>>,
}

impl AdminDebugHarnessService {
    pub fn new(config: AdminDebugHarnessConfig) -> Self {
        Self {
            config,
            state: Arc::new(Mutex::new(AdminDebugHarnessState {
                playing: false,
                scenario: AdminDebugScenario::PostMergeSuccess,
                stage_index: 0,
                revision: 1,
                run_id: 1,
                last_transition: Instant::now(),
                validation_paused: false,
                validation_acknowledged_at: None,
                validation_commands: BTreeMap::new(),
            })),
        }
    }

    pub fn projection(&self) -> AdminDebugHarnessProjection {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.advance_for_elapsed(&mut state, Instant::now());
        self.project(&state)
    }

    pub fn execute(
        &self,
        command: AdminDebugHarnessCommand,
    ) -> Result<AdminDebugHarnessProjection, AdminDebugHarnessError> {
        if !self.config.enabled {
            return Err(AdminDebugHarnessError::Disabled);
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.advance_for_elapsed(&mut state, Instant::now());
        match command {
            AdminDebugHarnessCommand::Play => {
                let last_index = state.scenario.stages().len().saturating_sub(1);
                if state.stage_index >= last_index {
                    state.stage_index = 0;
                    state.run_id = state.run_id.saturating_add(1);
                }
                state.playing = true;
            }
            AdminDebugHarnessCommand::Pause => state.playing = false,
            AdminDebugHarnessCommand::Step => {
                state.playing = false;
                state.stage_index =
                    (state.stage_index + 1).min(state.scenario.stages().len().saturating_sub(1));
            }
            AdminDebugHarnessCommand::Reset => {
                state.playing = false;
                state.stage_index = 0;
                state.run_id = state.run_id.saturating_add(1);
                state.validation_paused = false;
                state.validation_acknowledged_at = None;
                state.validation_commands.clear();
            }
            AdminDebugHarnessCommand::SelectScenario(scenario) => {
                state.playing = false;
                state.scenario = scenario;
                state.stage_index = 0;
                state.run_id = state.run_id.saturating_add(1);
                state.validation_paused = false;
                state.validation_acknowledged_at = None;
                state.validation_commands.clear();
            }
        }
        state.revision = state.revision.saturating_add(1);
        state.last_transition = Instant::now();
        Ok(self.project(&state))
    }

    fn advance_for_elapsed(&self, state: &mut AdminDebugHarnessState, now: Instant) {
        if !self.config.enabled || !state.playing || self.config.step_interval.is_zero() {
            return;
        }
        let elapsed = now.saturating_duration_since(state.last_transition);
        let elapsed_steps = elapsed.as_millis() / self.config.step_interval.as_millis();
        if elapsed_steps == 0 {
            return;
        }
        let last_index = state.scenario.stages().len().saturating_sub(1);
        let next_index = state
            .stage_index
            .saturating_add(elapsed_steps as usize)
            .min(last_index);
        if next_index != state.stage_index {
            state.stage_index = next_index;
            state.revision = state.revision.saturating_add(1);
        }
        if state.stage_index >= last_index {
            state.playing = false;
        }
        state.last_transition = now;
    }

    fn project(&self, state: &AdminDebugHarnessState) -> AdminDebugHarnessProjection {
        let stages = state.scenario.stages();
        let stage_index = state.stage_index.min(stages.len().saturating_sub(1));
        AdminDebugHarnessProjection {
            enabled: self.config.enabled,
            playing: self.config.enabled && state.playing,
            scenario: state.scenario,
            stage: stages[stage_index],
            stage_index,
            stage_count: stages.len(),
            revision: state.revision,
            run_id: state.run_id,
            step_interval_ms: self.config.step_interval.as_millis() as u64,
            scenarios: AdminDebugScenario::ALL
                .into_iter()
                .map(|scenario| AdminDebugScenarioOption {
                    key: scenario.key(),
                    label: scenario.label(),
                })
                .collect(),
            history: stages
                .iter()
                .copied()
                .take(stage_index + 1)
                .enumerate()
                .map(|(index, stage)| AdminDebugStageRecord { index, stage })
                .collect(),
        }
    }
}

impl AdminDebugPort for AdminDebugHarnessService {
    fn projection(&self) -> AdminDebugHarnessProjection {
        AdminDebugHarnessService::projection(self)
    }

    fn execute(
        &self,
        command: AdminDebugHarnessCommand,
    ) -> Result<AdminDebugHarnessProjection, AdminDebugHarnessError> {
        AdminDebugHarnessService::execute(self, command)
    }
}

impl PrValidationQueryPort for AdminDebugHarnessService {
    fn status_for_pr(
        &self,
        _request: PrValidationStatusRequest,
    ) -> Result<Option<PrValidationOperatorSummary>> {
        Ok(None)
    }

    fn load_board(&self, _request: PrValidationBoardRequest) -> Result<PrValidationBoardSnapshot> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.advance_for_elapsed(&mut state, Instant::now());
        let projection = self.project(&state);
        Ok(debug_validation_board(
            &projection,
            state.validation_paused,
            state.validation_acknowledged_at.clone(),
        ))
    }

    fn load_board_revision(&self) -> Result<i64> {
        Ok(i64::try_from(self.projection().revision).unwrap_or(i64::MAX))
    }

    fn load_detail(
        &self,
        request: PrValidationDetailRequest,
    ) -> Result<Option<PrValidationAdminRecord>> {
        let board = self.load_board(PrValidationBoardRequest::default())?;
        Ok(board
            .records
            .into_iter()
            .find(|record| record.record_key == request.record_key))
    }
}

impl PrValidationCommandPort for AdminDebugHarnessService {
    fn execute(
        &self,
        request: PrValidationAdminCommandRequest,
    ) -> Result<PrValidationAdminCommandResult> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.advance_for_elapsed(&mut state, Instant::now());
        let projection = self.project(&state);
        let fingerprint = format!(
            "{}|{}|{}",
            request.record_key,
            request.action.key(),
            request.expected_revision
        );
        if let Some(stored) = state.validation_commands.get(&request.command_id) {
            if stored.fingerprint == fingerprint {
                let mut result = stored.result.clone();
                result.duplicate = true;
                return Ok(result);
            }
            return Ok(debug_command_result(
                &request,
                PrValidationAdminCommandState::Rejected,
                Some(PrValidationAdminCommandRejection::IdempotencyConflict),
                true,
                None,
                projection.revision,
                "command id is already bound to a different fake request",
            ));
        }

        let record = debug_validation_record(
            &projection,
            state.validation_paused,
            state.validation_acknowledged_at.clone(),
        );
        let rejection = if request.record_key != record.record_key {
            Some(PrValidationAdminCommandRejection::NotFound)
        } else if request.expected_revision != record.observation_revision {
            Some(PrValidationAdminCommandRejection::StaleRevision)
        } else {
            match request.action {
                PrValidationAdminCommandAction::RetryNow
                    if state.validation_paused || record.verified =>
                {
                    Some(PrValidationAdminCommandRejection::InvalidState)
                }
                PrValidationAdminCommandAction::RetryNow
                    if projection.scenario == AdminDebugScenario::RateLimitRecovery
                        && projection.stage == AdminDebugStage::Blocked =>
                {
                    Some(PrValidationAdminCommandRejection::RateLimitActive)
                }
                PrValidationAdminCommandAction::Pause if state.validation_paused => {
                    Some(PrValidationAdminCommandRejection::InvalidState)
                }
                PrValidationAdminCommandAction::Resume if !state.validation_paused => {
                    Some(PrValidationAdminCommandRejection::InvalidState)
                }
                PrValidationAdminCommandAction::QueueRemediation
                    if record.finding_count <= record.remediation_count
                        || state.validation_paused =>
                {
                    Some(PrValidationAdminCommandRejection::NoActionableFinding)
                }
                PrValidationAdminCommandAction::Acknowledge
                    if state.validation_acknowledged_at.is_some() =>
                {
                    Some(PrValidationAdminCommandRejection::InvalidState)
                }
                _ => None,
            }
        };

        let observed_revision =
            (request.record_key == record.record_key).then_some(record.observation_revision);
        let (state_label, message) = if let Some(rejection) = rejection {
            let message = match rejection {
                PrValidationAdminCommandRejection::NotFound => "fake validation record not found",
                PrValidationAdminCommandRejection::StaleRevision => {
                    "fake record changed; refresh before retrying"
                }
                PrValidationAdminCommandRejection::RateLimitActive => {
                    "fake rate-limit reset has not elapsed"
                }
                PrValidationAdminCommandRejection::NoActionableFinding => {
                    "fake scenario has no uncorrelated finding"
                }
                _ => "fake command is unavailable in this state",
            };
            (PrValidationAdminCommandState::Rejected, message)
        } else {
            match request.action {
                PrValidationAdminCommandAction::RetryNow => {
                    if projection.scenario == AdminDebugScenario::ProviderOutageRetry
                        && projection.stage == AdminDebugStage::Blocked
                    {
                        state.stage_index = (state.stage_index + 1)
                            .min(state.scenario.stages().len().saturating_sub(1));
                    }
                }
                PrValidationAdminCommandAction::Pause => state.validation_paused = true,
                PrValidationAdminCommandAction::Resume => state.validation_paused = false,
                PrValidationAdminCommandAction::QueueRemediation => {
                    if projection.scenario == AdminDebugScenario::CheckFailureRecovery
                        && projection.stage == AdminDebugStage::Blocked
                    {
                        state.stage_index = (state.stage_index + 1)
                            .min(state.scenario.stages().len().saturating_sub(1));
                    }
                }
                PrValidationAdminCommandAction::Acknowledge => {
                    state.validation_acknowledged_at = Some(Utc::now().to_rfc3339())
                }
            }
            state.revision = state.revision.saturating_add(1);
            state.last_transition = Instant::now();
            (
                PrValidationAdminCommandState::Applied,
                "fake command applied through the application harness",
            )
        };
        let result = debug_command_result(
            &request,
            state_label,
            rejection,
            false,
            observed_revision,
            state.revision,
            message,
        );
        state.validation_commands.insert(
            request.command_id.clone(),
            StoredDebugValidationCommand {
                fingerprint,
                result: result.clone(),
            },
        );
        Ok(result)
    }
}

fn debug_command_result(
    request: &PrValidationAdminCommandRequest,
    state: PrValidationAdminCommandState,
    rejection: Option<PrValidationAdminCommandRejection>,
    duplicate: bool,
    observed_revision: Option<u64>,
    board_revision: u64,
    message: &str,
) -> PrValidationAdminCommandResult {
    PrValidationAdminCommandResult {
        command_id: request.command_id.clone(),
        record_key: request.record_key.clone(),
        action: request.action,
        state,
        rejection,
        duplicate,
        expected_revision: request.expected_revision,
        observed_revision,
        board_revision: i64::try_from(board_revision).unwrap_or(i64::MAX),
        message: message.to_string(),
        applied_at: Utc::now().to_rfc3339(),
    }
}

fn debug_validation_board(
    projection: &AdminDebugHarnessProjection,
    paused: bool,
    acknowledged_at: Option<String>,
) -> PrValidationBoardSnapshot {
    let record = debug_validation_record(projection, paused, acknowledged_at);
    let active = usize::from(
        !record.verified
            && !matches!(
                record.phase,
                PrValidationAdminPhase::Blocked | PrValidationAdminPhase::Failed
            ),
    );
    let summary = PrValidationBoardSummary {
        active,
        integrated: usize::from(record.integrated),
        verifying: usize::from(record.phase == PrValidationAdminPhase::Verifying),
        remediation: usize::from(matches!(
            record.phase,
            PrValidationAdminPhase::RemediationQueued | PrValidationAdminPhase::RemediationRunning
        )),
        remediation_queued: usize::from(record.phase == PrValidationAdminPhase::RemediationQueued),
        verified: usize::from(record.verified),
        blocked: usize::from(record.phase == PrValidationAdminPhase::Blocked),
        failed: usize::from(record.phase == PrValidationAdminPhase::Failed),
        stale: usize::from(record.stale),
    };
    PrValidationBoardSnapshot {
        revision: i64::try_from(projection.revision).unwrap_or(i64::MAX),
        scheduler_mode: "remediate".to_string(),
        rollout: PrValidationRolloutSnapshot::from_scheduler_mode(
            PrValidationSchedulerMode::Remediate,
        ),
        summary,
        records: vec![record],
        next_cursor: None,
        cursor_reset_required: false,
        generated_at: Utc::now().to_rfc3339(),
    }
}

fn debug_validation_record(
    projection: &AdminDebugHarnessProjection,
    paused: bool,
    acknowledged_at: Option<String>,
) -> PrValidationAdminRecord {
    let scenario = projection.scenario;
    let stage = projection.stage;
    let stage_index = projection.stage_index;
    let phase = debug_validation_phase(scenario, stage, stage_index);
    let verified = phase == PrValidationAdminPhase::Verified;
    let provider_blocked = matches!(
        scenario,
        AdminDebugScenario::RateLimitRecovery | AdminDebugScenario::ProviderOutageRetry
    ) && matches!(
        stage,
        AdminDebugStage::Blocked | AdminDebugStage::Recovering
    );
    let stale = scenario == AdminDebugScenario::RestartVerifying
        && matches!(stage, AdminDebugStage::Ready | AdminDebugStage::Reviewing)
        && stage_index <= 1;
    let integrated = !(scenario == AdminDebugScenario::ClosedUnmergedAttested && stage_index <= 1);
    let required_status = if verified {
        PrValidationAdminCheckStatus::Succeeded
    } else if scenario == AdminDebugScenario::RequiredMissing && stage == AdminDebugStage::Blocked {
        PrValidationAdminCheckStatus::PolicyBlocked
    } else if scenario == AdminDebugScenario::CheckFailureRecovery
        && stage == AdminDebugStage::Blocked
    {
        PrValidationAdminCheckStatus::ActionableFailure
    } else {
        PrValidationAdminCheckStatus::Pending
    };
    let optional_status = if scenario == AdminDebugScenario::OptionalSkipped {
        PrValidationAdminCheckStatus::Skipped
    } else if verified {
        PrValidationAdminCheckStatus::Succeeded
    } else {
        PrValidationAdminCheckStatus::Pending
    };
    let finding_count = usize::from(
        matches!(
            scenario,
            AdminDebugScenario::CheckFailureRecovery
                | AdminDebugScenario::RequiredMissing
                | AdminDebugScenario::DuplicateLateReview
        ) && (stage == AdminDebugStage::Blocked
            || matches!(
                phase,
                PrValidationAdminPhase::RemediationQueued
                    | PrValidationAdminPhase::RemediationRunning
            )
            || scenario == AdminDebugScenario::DuplicateLateReview),
    );
    let remediation_count = usize::from(
        finding_count > 0
            && matches!(
                phase,
                PrValidationAdminPhase::RemediationQueued
                    | PrValidationAdminPhase::RemediationRunning
                    | PrValidationAdminPhase::Verifying
                    | PrValidationAdminPhase::Verified
            )
            && !(scenario == AdminDebugScenario::CheckFailureRecovery
                && stage == AdminDebugStage::Blocked)
            && scenario != AdminDebugScenario::RequiredMissing,
    );
    let correlations = if remediation_count == 0 {
        Vec::new()
    } else {
        let running = phase == PrValidationAdminPhase::RemediationRunning;
        vec![PrValidationAdminCorrelation {
            finding: if scenario == AdminDebugScenario::DuplicateLateReview {
                "late review request (duplicate suppressed)"
            } else {
                "Post-Merge Gate actionable failure"
            }
            .to_string(),
            remediation_akra_id: format!("debug-remediation-{}", projection.run_id),
            task_state: Some(if running { "running" } else { "accepted" }.to_string()),
            slot_id: running.then(|| "slot-1".to_string()),
            session_key: running.then(|| "debug-validation-worker".to_string()),
            worker_state: running.then(|| "working".to_string()),
            lease_active: running,
        }]
    };
    let error_class = if provider_blocked {
        Some(if scenario == AdminDebugScenario::RateLimitRecovery {
            "retryable_provider"
        } else {
            "authentication_blocked"
        })
    } else {
        None
    };
    let observation_revision = projection.revision;
    let commands = debug_command_availability(
        paused,
        acknowledged_at.is_some(),
        verified,
        provider_blocked,
        scenario == AdminDebugScenario::RateLimitRecovery && stage == AdminDebugStage::Blocked,
        finding_count > remediation_count,
    );
    let mut timeline = vec![PrValidationAdminTimelineEntry {
        kind: "integration".to_string(),
        label: if integrated {
            "attested evidence SHA"
        } else {
            "closed without trusted evidence"
        }
        .to_string(),
        state: if integrated { "integrated" } else { "blocked" }.to_string(),
        occurred_at: Some("2026-08-10T08:00:00+00:00".to_string()),
        attempt: None,
    }];
    timeline.push(PrValidationAdminTimelineEntry {
        kind: "workflow".to_string(),
        label: "Post-Merge Gate".to_string(),
        state: debug_check_status_label(required_status).to_string(),
        occurred_at: Some("2026-08-10T08:01:00+00:00".to_string()),
        attempt: Some(if stage_index >= 5 { 2 } else { 1 }),
    });
    if remediation_count > 0 {
        timeline.push(PrValidationAdminTimelineEntry {
            kind: "remediation".to_string(),
            label: correlations[0].remediation_akra_id.clone(),
            state: correlations[0]
                .worker_state
                .clone()
                .unwrap_or_else(|| "queued".to_string()),
            occurred_at: Some("2026-08-10T08:02:00+00:00".to_string()),
            attempt: None,
        });
    }
    if let Some(value) = acknowledged_at.clone() {
        timeline.push(PrValidationAdminTimelineEntry {
            kind: "operator".to_string(),
            label: "운영자 확인".to_string(),
            state: "acknowledged".to_string(),
            occurred_at: Some(value),
            attempt: None,
        });
    }
    PrValidationAdminRecord {
        record_key: format!("debug-validation-{}", scenario.key()),
        akra_id: format!("akra-debug-{:02}", debug_scenario_number(scenario)),
        repository: "RefinedStone/codex-exec-loop".to_string(),
        canonical_pr_url: format!(
            "https://github.com/RefinedStone/codex-exec-loop/pull/{}",
            2100 + debug_scenario_number(scenario)
        ),
        pull_request_number: 2100 + debug_scenario_number(scenario),
        phase,
        phase_label: debug_phase_label(phase).to_string(),
        severity: if matches!(
            phase,
            PrValidationAdminPhase::Blocked | PrValidationAdminPhase::Failed
        ) || required_status == PrValidationAdminCheckStatus::ActionableFailure
        {
            PrValidationAdminSeverity::Danger
        } else if provider_blocked
            || matches!(
                phase,
                PrValidationAdminPhase::RemediationQueued
                    | PrValidationAdminPhase::RemediationRunning
            )
        {
            PrValidationAdminSeverity::Warning
        } else if verified {
            PrValidationAdminSeverity::Success
        } else {
            PrValidationAdminSeverity::Info
        },
        integrated,
        verified,
        provider_blocked,
        stale,
        stale_seconds: if stale { 181 } else { 12 },
        target_sha: "1111111111111111111111111111111111111111".to_string(),
        base_before_sha: integrated.then(|| "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
        evidence_sha: integrated.then(|| "2222222222222222222222222222222222222222".to_string()),
        merge_sha: integrated.then(|| "2222222222222222222222222222222222222222".to_string()),
        target_short_sha: "1111111".to_string(),
        evidence_short_sha: integrated.then(|| "2222222".to_string()),
        merge_short_sha: integrated.then(|| "2222222".to_string()),
        integration_method: integrated.then(|| {
            if scenario == AdminDebugScenario::ClosedUnmergedAttested {
                "distributor_cherry_pick"
            } else {
                "github_rebase_merge"
            }
            .to_string()
        }),
        integration_pull_request_number: integrated
            .then_some(2100 + debug_scenario_number(scenario)),
        integrated_at: integrated.then(|| "2026-08-10T08:00:00+00:00".to_string()),
        remote_verified_at: integrated.then(|| "2026-08-10T08:00:05+00:00".to_string()),
        reason: projection.stage.summary().to_string(),
        blocker: matches!(
            phase,
            PrValidationAdminPhase::Blocked | PrValidationAdminPhase::Failed
        )
        .then(|| "typed fake validation blocker".to_string()),
        recovery_action: if provider_blocked {
            "retry after provider recovery"
        } else if finding_count > remediation_count {
            "queue the correlated remediation"
        } else {
            "observe the next durable transition"
        }
        .to_string(),
        checks: vec![
            PrValidationAdminCheck {
                context: "Post-Merge Gate".to_string(),
                app_slug: Some("github-actions".to_string()),
                required: true,
                status: required_status,
                latest_attempt: Some(if stage_index >= 5 { 2 } else { 1 }),
                started_at: Some("2026-08-10T08:00:20+00:00".to_string()),
                completed_at: verified.then(|| "2026-08-10T08:01:20+00:00".to_string()),
            },
            PrValidationAdminCheck {
                context: "Optional audit".to_string(),
                app_slug: Some("github-actions".to_string()),
                required: false,
                status: optional_status,
                latest_attempt: Some(1),
                started_at: Some("2026-08-10T08:00:25+00:00".to_string()),
                completed_at: matches!(
                    optional_status,
                    PrValidationAdminCheckStatus::Succeeded | PrValidationAdminCheckStatus::Skipped
                )
                .then(|| "2026-08-10T08:00:55+00:00".to_string()),
            },
        ],
        required_checks_succeeded: usize::from(
            required_status == PrValidationAdminCheckStatus::Succeeded,
        ),
        required_checks_total: 1,
        workflows: vec![PrValidationAdminWorkflow {
            name: "Post-Merge Gate".to_string(),
            status: debug_check_status_label(required_status).to_string(),
            run_attempt: if stage_index >= 5 { 2 } else { 1 },
            started_at: Some("2026-08-10T08:00:20+00:00".to_string()),
            updated_at: Some("2026-08-10T08:01:20+00:00".to_string()),
        }],
        providers: vec![PrValidationAdminProvider {
            key: "github:Actions".to_string(),
            lifecycle: "finite".to_string(),
            status: if provider_blocked {
                "failed"
            } else if verified {
                "complete"
            } else {
                "pending"
            }
            .to_string(),
            page_complete: verified,
        }],
        finding_count,
        remediation_count,
        correlations,
        schedule: PrValidationAdminSchedule {
            updated_at: "2026-08-10T08:01:30+00:00".to_string(),
            last_polled_at: Some("2026-08-10T08:01:30+00:00".to_string()),
            next_poll_at: (!verified).then(|| "2026-08-10T08:02:00+00:00".to_string()),
            poll_attempt: u64::try_from(stage_index + 1).unwrap_or(u64::MAX),
            consecutive_error_count: u32::from(provider_blocked),
            error_class: error_class.map(str::to_string),
            rate_limit_remaining: if scenario == AdminDebugScenario::RateLimitRecovery
                && stage == AdminDebugStage::Blocked
            {
                Some(0)
            } else {
                Some(4871)
            },
            rate_limit_reset_at: (scenario == AdminDebugScenario::RateLimitRecovery
                && stage == AdminDebugStage::Blocked)
                .then(|| "2099-08-10T08:05:00+00:00".to_string()),
        },
        paused,
        acknowledged_at,
        last_command_id: None,
        commands,
        timeline,
        observation_revision,
        post_merge_checkpoint_observed: stage_index > 0,
    }
}

fn debug_validation_phase(
    scenario: AdminDebugScenario,
    stage: AdminDebugStage,
    stage_index: usize,
) -> PrValidationAdminPhase {
    if stage == AdminDebugStage::Complete {
        return PrValidationAdminPhase::Verified;
    }
    match scenario {
        AdminDebugScenario::CheckFailureRecovery => match stage {
            AdminDebugStage::Intake | AdminDebugStage::Dispatching => {
                PrValidationAdminPhase::RemediationQueued
            }
            AdminDebugStage::Working => PrValidationAdminPhase::RemediationRunning,
            AdminDebugStage::Ready => PrValidationAdminPhase::Integrated,
            _ => PrValidationAdminPhase::Verifying,
        },
        AdminDebugScenario::RequiredMissing if stage == AdminDebugStage::Blocked => {
            PrValidationAdminPhase::Blocked
        }
        AdminDebugScenario::ClosedUnmergedAttested if stage_index <= 1 => {
            PrValidationAdminPhase::Blocked
        }
        AdminDebugScenario::DuplicateLateReview if stage == AdminDebugStage::QueuePressure => {
            PrValidationAdminPhase::RemediationQueued
        }
        _ if stage == AdminDebugStage::Ready => PrValidationAdminPhase::Integrated,
        _ => PrValidationAdminPhase::Verifying,
    }
}

fn debug_phase_label(phase: PrValidationAdminPhase) -> &'static str {
    match phase {
        PrValidationAdminPhase::Registered => "Registered",
        PrValidationAdminPhase::PreMerge => "Pre-merge",
        PrValidationAdminPhase::Integrated => "Integrated",
        PrValidationAdminPhase::Verifying => "Verifying",
        PrValidationAdminPhase::RemediationQueued => "Remediation queued",
        PrValidationAdminPhase::RemediationRunning => "Remediation running",
        PrValidationAdminPhase::Verified => "Verified",
        PrValidationAdminPhase::Blocked => "Blocked",
        PrValidationAdminPhase::Failed => "Failed",
    }
}

fn debug_check_status_label(status: PrValidationAdminCheckStatus) -> &'static str {
    match status {
        PrValidationAdminCheckStatus::Unobserved => "unobserved",
        PrValidationAdminCheckStatus::Skipped => "skipped",
        PrValidationAdminCheckStatus::Missing => "missing",
        PrValidationAdminCheckStatus::Pending => "in_progress",
        PrValidationAdminCheckStatus::Succeeded => "succeeded",
        PrValidationAdminCheckStatus::ActionableFailure => "failed",
        PrValidationAdminCheckStatus::PolicyBlocked => "policy_blocked",
    }
}

fn debug_command_availability(
    paused: bool,
    acknowledged: bool,
    verified: bool,
    provider_blocked: bool,
    rate_limited: bool,
    has_unremediated_finding: bool,
) -> Vec<PrValidationAdminCommandAvailability> {
    let item =
        |action: &str, label: &str, reason: Option<&str>| PrValidationAdminCommandAvailability {
            action: action.to_string(),
            label: label.to_string(),
            enabled: reason.is_none(),
            disabled_reason: reason.map(str::to_string),
        };
    vec![
        item(
            "retry_now",
            "지금 재시도",
            if paused {
                Some("일시정지됨")
            } else if verified {
                Some("이미 Verified")
            } else if rate_limited {
                Some("rate-limit reset 대기")
            } else {
                None
            },
        ),
        item(
            "pause",
            "검증 일시정지",
            paused.then_some("이미 일시정지됨"),
        ),
        item(
            "resume",
            "검증 재개",
            (!paused).then_some("일시정지 상태가 아님"),
        ),
        item(
            "queue_remediation",
            "복구 Queue 등록",
            (!has_unremediated_finding).then_some("미상관 actionable finding 없음"),
        ),
        item(
            "acknowledge",
            "확인 처리",
            if acknowledged {
                Some("이미 확인됨")
            } else if !provider_blocked && !has_unremediated_finding {
                Some("확인할 경보 없음")
            } else {
                None
            },
        ),
    ]
}

fn debug_scenario_number(scenario: AdminDebugScenario) -> u64 {
    AdminDebugScenario::ALL
        .iter()
        .position(|candidate| *candidate == scenario)
        .map(|index| u64::try_from(index + 1).unwrap_or(10))
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::{
        AdminDebugHarnessCommand, AdminDebugHarnessConfig, AdminDebugHarnessError,
        AdminDebugHarnessService, AdminDebugScenario, AdminDebugStage,
    };
    use std::thread;
    use std::time::Duration;

    #[test]
    fn disabled_harness_is_read_only_and_rejects_commands() {
        let service = AdminDebugHarnessService::new(AdminDebugHarnessConfig::disabled());
        assert!(!service.projection().enabled);
        assert_eq!(
            service.execute(AdminDebugHarnessCommand::Play),
            Err(AdminDebugHarnessError::Disabled)
        );
    }

    #[test]
    fn step_and_scenario_commands_are_deterministic() {
        let service = AdminDebugHarnessService::new(AdminDebugHarnessConfig::enabled());
        let stepped = service
            .execute(AdminDebugHarnessCommand::Step)
            .expect("enabled harness should step");
        assert_eq!(stepped.stage, AdminDebugStage::Delivering);
        assert_eq!(stepped.stage_index, 1);
        assert_eq!(stepped.history.len(), 2);

        let selected = service
            .execute(AdminDebugHarnessCommand::SelectScenario(
                AdminDebugScenario::CheckFailureRecovery,
            ))
            .expect("enabled harness should select a scenario");
        assert_eq!(selected.scenario, AdminDebugScenario::CheckFailureRecovery);
        assert_eq!(selected.stage, AdminDebugStage::Ready);
        assert_eq!(selected.stage_index, 0);
        assert!(!selected.playing);
    }

    #[test]
    fn play_advances_against_the_application_clock_and_stops_at_terminal_stage() {
        let service = AdminDebugHarnessService::new(
            AdminDebugHarnessConfig::enabled_with_interval(Duration::from_millis(2)),
        );
        service
            .execute(AdminDebugHarnessCommand::Play)
            .expect("enabled harness should play");
        thread::sleep(Duration::from_millis(25));

        let projection = service.projection();
        assert_eq!(projection.stage, AdminDebugStage::Complete);
        assert_eq!(projection.progress_percent(), 100);
        assert!(!projection.playing);
    }
}
