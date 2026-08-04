use std::sync::{Arc, Mutex};
use std::time::Instant;

#[cfg(test)]
use crate::application::port::inbound::admin_debug_port::AdminDebugStage;
use crate::application::port::inbound::admin_debug_port::{
    AdminDebugHarnessCommand, AdminDebugHarnessConfig, AdminDebugHarnessError,
    AdminDebugHarnessProjection, AdminDebugPort, AdminDebugScenario, AdminDebugScenarioOption,
    AdminDebugStageRecord,
};

#[derive(Debug)]
struct AdminDebugHarnessState {
    playing: bool,
    scenario: AdminDebugScenario,
    stage_index: usize,
    revision: u64,
    run_id: u64,
    last_transition: Instant,
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
                scenario: AdminDebugScenario::HappyPath,
                stage_index: 0,
                revision: 1,
                run_id: 1,
                last_transition: Instant::now(),
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
            }
            AdminDebugHarnessCommand::SelectScenario(scenario) => {
                state.playing = false;
                state.scenario = scenario;
                state.stage_index = 0;
                state.run_id = state.run_id.saturating_add(1);
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
            scenarios: [
                AdminDebugScenario::HappyPath,
                AdminDebugScenario::BlockedRecovery,
                AdminDebugScenario::QueuePressure,
            ]
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
        assert_eq!(stepped.stage, AdminDebugStage::Intake);
        assert_eq!(stepped.stage_index, 1);
        assert_eq!(stepped.history.len(), 2);

        let selected = service
            .execute(AdminDebugHarnessCommand::SelectScenario(
                AdminDebugScenario::BlockedRecovery,
            ))
            .expect("enabled harness should select a scenario");
        assert_eq!(selected.scenario, AdminDebugScenario::BlockedRecovery);
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
