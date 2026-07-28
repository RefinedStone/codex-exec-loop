use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const DEFAULT_STEP_INTERVAL: Duration = Duration::from_millis(2_400);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminDebugScenario {
    HappyPath,
    BlockedRecovery,
    QueuePressure,
}

impl AdminDebugScenario {
    pub fn key(self) -> &'static str {
        match self {
            Self::HappyPath => "happy_path",
            Self::BlockedRecovery => "blocked_recovery",
            Self::QueuePressure => "queue_pressure",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::HappyPath => "정상 배포 루프",
            Self::BlockedRecovery => "차단 및 복구",
            Self::QueuePressure => "대기열 압력",
        }
    }

    pub fn from_key(value: &str) -> Option<Self> {
        match value.trim() {
            "happy_path" => Some(Self::HappyPath),
            "blocked_recovery" => Some(Self::BlockedRecovery),
            "queue_pressure" => Some(Self::QueuePressure),
            _ => None,
        }
    }

    fn stages(self) -> &'static [AdminDebugStage] {
        match self {
            Self::HappyPath => &[
                AdminDebugStage::Ready,
                AdminDebugStage::Intake,
                AdminDebugStage::Dispatching,
                AdminDebugStage::Working,
                AdminDebugStage::Reviewing,
                AdminDebugStage::Delivering,
                AdminDebugStage::Cleanup,
                AdminDebugStage::Complete,
            ],
            Self::BlockedRecovery => &[
                AdminDebugStage::Ready,
                AdminDebugStage::Intake,
                AdminDebugStage::Dispatching,
                AdminDebugStage::Working,
                AdminDebugStage::Blocked,
                AdminDebugStage::Recovering,
                AdminDebugStage::Working,
                AdminDebugStage::Reviewing,
                AdminDebugStage::Delivering,
                AdminDebugStage::Complete,
            ],
            Self::QueuePressure => &[
                AdminDebugStage::Ready,
                AdminDebugStage::Intake,
                AdminDebugStage::QueuePressure,
                AdminDebugStage::Dispatching,
                AdminDebugStage::Working,
                AdminDebugStage::Reviewing,
                AdminDebugStage::Delivering,
                AdminDebugStage::Complete,
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminDebugStage {
    Ready,
    Intake,
    QueuePressure,
    Dispatching,
    Working,
    Reviewing,
    Blocked,
    Recovering,
    Delivering,
    Cleanup,
    Complete,
}

impl AdminDebugStage {
    pub fn key(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Intake => "intake",
            Self::QueuePressure => "queue_pressure",
            Self::Dispatching => "dispatching",
            Self::Working => "working",
            Self::Reviewing => "reviewing",
            Self::Blocked => "blocked",
            Self::Recovering => "recovering",
            Self::Delivering => "delivering",
            Self::Cleanup => "cleanup",
            Self::Complete => "complete",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "준비",
            Self::Intake => "작업 접수",
            Self::QueuePressure => "대기열 압력",
            Self::Dispatching => "요원 배정",
            Self::Working => "병렬 작업",
            Self::Reviewing => "리뷰",
            Self::Blocked => "차단",
            Self::Recovering => "복구",
            Self::Delivering => "배포",
            Self::Cleanup => "정리",
            Self::Complete => "완료",
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            Self::Ready => "세 개의 슬롯과 워커 프로필이 시나리오 입력을 기다립니다.",
            Self::Intake => "검증된 작업 세 건이 애플리케이션 큐에 접수되었습니다.",
            Self::QueuePressure => "슬롯 용량보다 많은 작업이 들어와 분배 대기열이 증가했습니다.",
            Self::Dispatching => "애플리케이션 하네스가 작업과 워커 임대를 연결하고 있습니다.",
            Self::Working => "세 워커가 각 작업 공간에서 병렬 구현을 진행합니다.",
            Self::Reviewing => "완료 보고가 검토 게이트와 검증 단계로 이동했습니다.",
            Self::Blocked => "검증 실패가 한 레인을 차단하고 복구 판단을 요청했습니다.",
            Self::Recovering => "차단된 레인이 새 세대의 임대로 복구되고 있습니다.",
            Self::Delivering => "검토된 변경이 push, PR, rebase 통합 순서로 전달됩니다.",
            Self::Cleanup => "병합된 레인의 worktree와 임대를 정리합니다.",
            Self::Complete => "모든 레인이 통합되어 다음 작업을 기다립니다.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminDebugScenarioOption {
    pub key: &'static str,
    pub label: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminDebugStageRecord {
    pub index: usize,
    pub stage: AdminDebugStage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminDebugHarnessProjection {
    pub enabled: bool,
    pub playing: bool,
    pub scenario: AdminDebugScenario,
    pub stage: AdminDebugStage,
    pub stage_index: usize,
    pub stage_count: usize,
    pub revision: u64,
    pub run_id: u64,
    pub step_interval_ms: u64,
    pub scenarios: Vec<AdminDebugScenarioOption>,
    pub history: Vec<AdminDebugStageRecord>,
}

impl AdminDebugHarnessProjection {
    pub fn progress_percent(&self) -> u8 {
        if self.stage_count <= 1 {
            return 100;
        }
        ((self.stage_index * 100) / (self.stage_count - 1)) as u8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminDebugHarnessCommand {
    Play,
    Pause,
    Step,
    Reset,
    SelectScenario(AdminDebugScenario),
}

#[derive(Debug, Clone, Copy)]
pub struct AdminDebugHarnessConfig {
    pub enabled: bool,
    pub step_interval: Duration,
}

impl AdminDebugHarnessConfig {
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            step_interval: DEFAULT_STEP_INTERVAL,
        }
    }

    pub fn disabled() -> Self {
        Self {
            enabled: false,
            step_interval: DEFAULT_STEP_INTERVAL,
        }
    }

    #[cfg(test)]
    fn enabled_with_interval(step_interval: Duration) -> Self {
        Self {
            enabled: true,
            step_interval,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdminDebugHarnessError {
    Disabled,
}

impl fmt::Display for AdminDebugHarnessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => formatter.write_str("admin debug harness is disabled"),
        }
    }
}

impl std::error::Error for AdminDebugHarnessError {}

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
