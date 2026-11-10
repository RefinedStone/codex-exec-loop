//! 커널 결정적 시뮬레이션 시험이다 (plan 19 Q1, 성질 P2).
//!
//! 커널 호스트 루프(`KernelHostLoop`)와 그 아래 런타임·core 는 실제 코드 그대로 돈다. 바깥 세계만 가짜이다.
//!
//! - 가짜 시계: 커널 주기에 실제 시각 대신 `시작 시각 + 지난 시간` 을 준다. 커널은 기다리지 않는다 (`wait = 0`).
//! - 가짜 효과 실행기: core 가 낸 효과를 실행하지 않고 기록만 한다. 완료는 시뮬레이션이 정한 때에 core 입력함으로
//!   넣는다. 실제 worker 와 같은 길이다.
//! - 가짜 app-server: 턴을 시작하고, 시작 알림과 종료 알림을 따로 보낸다. 시작 전후에 스트림이 실패할 수 있다.
//!   중지 요청이 닿으면 그 요청이 가리키는 턴을 중단으로 끝낸다.
//! - 가짜 평가기: 턴 뒤 평가에 자동 후속 프롬프트 또는 멈춤을 답한다.
//!
//! 무엇을 언제 할지는 seed 하나로 만든 난수가 고른다. 같은 seed 는 같은 순서를 만든다. 실패하면 seed 를 알리므로
//! `AKRA_SIM_SEED=<seed>` 로 같은 실패를 다시 만든다.
//!
//! 지금 모델하는 범위: 수동 턴 제출(새 thread, 기존 thread), 중지 요청(성공, 실패, 무효), 턴 시작·종료·실패, 턴 뒤
//! 평가, 자동 후속, 클라이언트 떼기와 다시 붙기, 시계. 병렬 모드, 승인, steer, 원격 준비(thread 지정 프롬프트),
//! 커널 재시작은 아직 모델하지 않는다.

use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{KernelClientId, KernelHostEvent, KernelHostInput, KernelHostLoop, KernelIdleClock};
use crate::adapter::inbound::tui::app::test_helpers::{
    test_parallel_mode_control_plane_composition, test_planning_services,
};
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::application::port::outbound::kernel_event_log_port::KernelEventLogPort;
use crate::composition::native_client_runtime::NativeClientRuntime;
use crate::composition::native_client_runtime::tests::post_turn_execution;
use crate::core::app::{
    AppCommand, AppEvent, CoreEffect, CoreEffectCompletion, CoreInput, CorePromptOrigin,
    PostTurnEvaluationCorrelation, StopRequestAttempt, StopRequestCorrelation, TurnStreamEvent,
    TurnSubmissionCorrelation, TurnSubmissionRequest,
};
use crate::core::runtime::{CoreEffectExecutor, CoreInputSender};
use crate::domain::conversation_runtime_envelope::{
    ConversationRuntimeConfigurationRequest, ConversationRuntimeEnvelope,
};
use crate::domain::kernel_event::{KernelEvent, RecordedKernelEvent};
use crate::domain::planning::{
    PostTurnAutoFollowSkipReason, PostTurnContinuationAction, TurnSnapshotCapture,
};
use crate::domain::turn_terminal::{
    ConversationTurnApplicationDelivery, ConversationTurnTerminalOutcome,
    ConversationTurnTerminalReceipt,
};

const WORKSPACE: &str = "/akra-simulation/workspace";
/// 한 seed 에서 시뮬레이션이 고르는 행동 수이다.
const SCENARIO_STEPS: usize = 240;
/// 행동을 멈춘 뒤 커널이 일을 끝낼 때까지 허용하는 주기 수이다. 자동 후속 사슬이 끝날 만큼 넉넉하다.
const SETTLE_STEPS: usize = 400;
/// PR 마다 도는 seed 수이다 (plan 19 Q-Q4). 더 돌리려면 `AKRA_SIM_SEEDS` 를 준다.
const DEFAULT_SEED_COUNT: u64 = 24;
/// 실패 보고에 넣는 마지막 기록 줄 수이다.
const TRACE_TAIL: usize = 40;
/*
 * core 입력함은 제어 입력이 16개 쌓이면 보내는 쪽을 막는다 (`CORE_INPUT_CHANNEL_CAPACITY`). 비우는 쪽도 이 스레드라
 * 막히면 시험이 멈춘다. 행동 하나는 입력을 최대 3개 넣으므로, 이만큼 쌓이면 다음 행동 전에 커널 주기를 돌린다.
 */
const CORE_INPUT_FLUSH_THRESHOLD: usize = 10;
/*
 * 가짜 시계의 시작을 실제 시각보다 하루 뒤로 둔다. 커널이 가짜 시각과 실제 시각을 섞어 비교하면 차이가 커서 같은
 * seed 의 두 실행이 달라지고, 재현 시험이 그것을 드러낸다.
 */
const FAKE_CLOCK_OFFSET: Duration = Duration::from_secs(24 * 60 * 60);

/*
 * seed 하나로 정해지는 난수이다 (SplitMix64). 외부 crate 의 난수 알고리즘은 버전이 바뀌면 같은 seed 에서 다른 값을
 * 낼 수 있다. 그러면 이전에 보고된 seed 가 같은 실패를 만들지 못하므로 알고리즘을 여기에 고정한다.
 */
struct SimRng(u64);

impl SimRng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^ (value >> 31)
    }

    /// `0..upper` 의 값이다. 작은 범위만 쓰므로 나머지 연산의 치우침은 무시한다.
    fn below(&mut self, upper: u64) -> u64 {
        assert!(upper > 0, "the range must not be empty");
        self.next_u64() % upper
    }

    fn index(&mut self, len: usize) -> usize {
        self.below(len as u64) as usize
    }

    fn percent(&mut self, chance: u64) -> bool {
        self.below(100) < chance
    }
}

/// core 가 낸 효과를 기록만 한다. 시뮬레이션이 주기마다 꺼내 바깥 세계에 넘긴다.
#[derive(Clone, Default)]
struct RecordedEffects(Arc<Mutex<Vec<CoreEffect>>>);

impl RecordedEffects {
    fn take(&self) -> Vec<CoreEffect> {
        std::mem::take(
            &mut *self
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }
}

impl CoreEffectExecutor for RecordedEffects {
    fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(effect);
        None
    }
}

/// seed 하나로 정하는 시나리오 값이다.
#[derive(Debug, Clone, Copy)]
struct ScenarioSettings {
    max_auto_turns: usize,
    manual_turn_budget: usize,
    /// 중지 요청은 자동 후속을 멈춘다. 모든 seed 가 중지하면 자동 후속 사슬이 드물어지므로 seed 의 반만 중지한다.
    stop_requests: bool,
}

impl ScenarioSettings {
    fn from_rng(rng: &mut SimRng) -> Self {
        Self {
            max_auto_turns: rng.index(4),
            manual_turn_budget: 2 + rng.index(5),
            stop_requests: rng.percent(50),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnStage {
    /// app-server 가 제출을 받았다. 아직 시작 알림을 보내지 않았다.
    Submitted,
    /// 시작 알림을 보냈다. 다음 단계는 종료 알림이다.
    Started,
}

/// 가짜 app-server 에서 도는 턴이다. 한 번에 하나만 있을 수 있다 (불변식 I1).
#[derive(Debug, Clone)]
struct FakeTurn {
    correlation: TurnSubmissionCorrelation,
    thread_id: String,
    turn_id: String,
    stage: TurnStage,
    interrupt_requested: bool,
}

/// 바깥 세계가 나중에 돌려줄 완료이다. 어떤 순서로 돌려줄지는 난수가 고른다.
#[derive(Debug, Clone)]
enum PendingCompletion {
    PostTurnEvaluation {
        correlation: PostTurnEvaluationCorrelation,
    },
    StopAttempt {
        correlation: StopRequestCorrelation,
        attempt: StopRequestAttempt,
    },
}

/// 시뮬레이션이 한 걸음에 고를 수 있는 행동이다.
#[derive(Debug, Clone, Copy)]
enum SimAction {
    SubmitManualTurn,
    RequestStop,
    ToggleClient,
    AdvanceTurn,
    DeliverCompletion,
    RunHostCycle,
}

/// 불변식이 깨졌다. 어느 걸음에서 무엇이 깨졌는지 담는다.
#[derive(Debug)]
struct Violation {
    step: usize,
    message: String,
}

/// 불변식 I2, I3 을 보려고 모으는 기록이다. 어긋나면 위반 문장을 돌려준다.
#[derive(Default)]
struct InvariantLedger {
    evaluated_turns: BTreeSet<(String, String)>,
    /// 평가가 자동 후속 프롬프트를 답한 완료 턴이다. 자동 후속은 이 턴에서만 나올 수 있다.
    auto_prompted_turns: BTreeSet<(String, String)>,
    followed_turns: BTreeSet<(String, String)>,
    auto_turns_since_manual: usize,
}

impl InvariantLedger {
    /// I2: 완료 턴 하나는 한 번만 평가된다.
    fn record_evaluation_start(
        &mut self,
        correlation: &PostTurnEvaluationCorrelation,
    ) -> Result<(), String> {
        let key = turn_key(&correlation.thread_id, &correlation.completed_turn_id);
        if self.evaluated_turns.insert(key) {
            return Ok(());
        }
        Err(format!(
            "I2: {} on {} was evaluated twice",
            correlation.completed_turn_id, correlation.thread_id
        ))
    }

    fn record_auto_prompt(&mut self, correlation: &PostTurnEvaluationCorrelation) {
        self.auto_prompted_turns.insert(turn_key(
            &correlation.thread_id,
            &correlation.completed_turn_id,
        ));
    }

    /// I3: 자동 후속은 평가가 프롬프트를 답한 완료 턴마다 한 번이고, 수동 턴 뒤 최대 턴 수를 넘지 않는다.
    fn record_turn_submission(
        &mut self,
        request: &TurnSubmissionRequest,
        max_auto_turns: usize,
    ) -> Result<(), String> {
        if request.prompt_origin != CorePromptOrigin::AutoFollow {
            self.auto_turns_since_manual = 0;
            return Ok(());
        }
        self.auto_turns_since_manual += 1;
        if self.auto_turns_since_manual > max_auto_turns {
            return Err(format!(
                "I3: follow-up turn {} exceeds the limit {max_auto_turns}",
                self.auto_turns_since_manual
            ));
        }
        let Some(source) = &request.auto_follow_source else {
            return Err("I3: a follow-up turn has no source turn".to_string());
        };
        let key = turn_key(&source.thread_id, &source.completed_turn_id);
        if !self.auto_prompted_turns.contains(&key) {
            return Err(format!(
                "I3: a follow-up turn for {} on {} that the evaluation did not ask for",
                source.completed_turn_id, source.thread_id
            ));
        }
        if !self.followed_turns.insert(key) {
            return Err(format!(
                "I3: a second follow-up turn for {} on {}",
                source.completed_turn_id, source.thread_id
            ));
        }
        Ok(())
    }
}

fn turn_key(thread_id: &str, turn_id: &str) -> (String, String) {
    (thread_id.to_string(), turn_id.to_string())
}

/// 커널이 쓴 판단 사건이다 (plan 19 Q3-2). 기록기 스레드가 쓰고, 시뮬레이션은 커널을 버린 뒤에 읽는다.
#[derive(Default)]
struct SimKernelLog(Mutex<Vec<RecordedKernelEvent>>);

impl KernelEventLogPort for SimKernelLog {
    fn append_kernel_events(
        &self,
        _workspace_dir: &str,
        records: &[RecordedKernelEvent],
    ) -> anyhow::Result<()> {
        self.0
            .lock()
            .expect("kernel log should lock")
            .extend_from_slice(records);
        Ok(())
    }

    fn load_kernel_events(
        &self,
        _workspace_dir: &str,
        _kinds: &[&str],
    ) -> anyhow::Result<Vec<KernelEvent>> {
        Ok(Vec::new())
    }
}

/*
 * I6: 판단 사건 기록이 맞다 (plan 19 Q3-2). 같은 턴 제출은 한 번만 기록된다 (자동 후속은 core 입력을 다시 넣는다).
 * 같은 턴은 한 번 시작하고 한 번 끝나며, 끝은 시작 뒤에 온다.
 */
fn check_kernel_records(recorded: &[RecordedKernelEvent]) -> Result<(), String> {
    // 커널이 정한 sequence 는 1 부터 빠짐없이 늘어난다 (plan 19 Q3-3b).
    for (index, record) in recorded.iter().enumerate() {
        if record.sequence != index as i64 + 1 {
            return Err(format!(
                "I6: record {index} has sequence {}, expected {}",
                record.sequence,
                index + 1
            ));
        }
    }
    let records = recorded
        .iter()
        .map(|record| record.event.clone())
        .collect::<Vec<_>>();
    let mut submissions = BTreeSet::new();
    let mut started = BTreeSet::new();
    let mut ended = BTreeSet::new();
    for record in &records {
        match record {
            KernelEvent::TurnSubmitted { submission, .. } if !submissions.insert(*submission) => {
                return Err(format!(
                    "I6: turn submission {submission} was recorded twice"
                ));
            }
            KernelEvent::TurnStarted { thread_id, turn_id }
                if !started.insert(turn_key(thread_id, turn_id)) =>
            {
                return Err(format!("I6: {turn_id} on {thread_id} started twice"));
            }
            KernelEvent::TurnEnded {
                thread_id, turn_id, ..
            } => {
                let key = turn_key(thread_id, turn_id);
                if !started.contains(&key) || !ended.insert(key) {
                    return Err(format!(
                        "I6: {turn_id} on {thread_id} ended without one matching start"
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

struct Simulation {
    rng: SimRng,
    settings: ScenarioSettings,
    host: KernelHostLoop,
    inbox: Sender<KernelHostInput>,
    /// 붙어 있는 클라이언트이다. 떼면 `None` 이다. 커널은 클라이언트가 없어도 일을 이어 가야 한다 (plan 18 K2-3).
    client: Option<(KernelClientId, Receiver<KernelHostEvent>)>,
    effects: RecordedEffects,
    core_inputs: CoreInputSender,
    /// 마지막 커널 주기 뒤 core 입력함에 넣은 입력 수이다.
    queued_core_inputs: usize,
    started_at: Instant,
    elapsed: Duration,
    step: usize,
    trace: Vec<String>,
    // 가짜 app-server 와 가짜 평가기의 상태이다.
    turn: Option<FakeTurn>,
    pending: Vec<PendingCompletion>,
    invalidated_stop_requests: BTreeSet<u64>,
    known_threads: Vec<String>,
    next_thread: u64,
    next_turn: u64,
    manual_turns_sent: usize,
    ledger: InvariantLedger,
    kernel_log: Arc<SimKernelLog>,
}

impl Simulation {
    fn new(seed: u64) -> Self {
        let mut rng = SimRng::new(seed);
        let settings = ScenarioSettings::from_rng(&mut rng);
        let effects = RecordedEffects::default();
        let planning = test_planning_services(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));
        let parallel = test_parallel_mode_control_plane_composition(planning);
        /*
         * 실행 중인 커널과 달리 병렬 보기의 작업 공간을 정하지 않는다 (`show_workspace_to_parallel_control`). 정하면
         * 클라이언트가 붙을 때마다 병렬 제어부가 감독 점검을 실제 작업 스레드에서 돌리고, 그 완료 시점이 seed 를
         * 따르지 않는다. 병렬 제어부의 효과를 가짜로 바꾸는 일은 병렬 모드를 모델할 때 한다.
         */
        let (mut runtime, core_inputs) = NativeClientRuntime::new_with_effect_executor_for_test(
            parallel,
            Box::new(effects.clone()),
        );
        let kernel_log = Arc::new(SimKernelLog::default());
        runtime.attach_kernel_event_log(kernel_log.clone(), "/sim");
        let (inbox, inputs) = mpsc::channel();
        let host = KernelHostLoop::new(runtime, inputs, KernelIdleClock::default());
        let mut simulation = Self {
            rng,
            settings,
            host,
            inbox,
            client: None,
            effects,
            core_inputs,
            queued_core_inputs: 0,
            started_at: Instant::now() + FAKE_CLOCK_OFFSET,
            elapsed: Duration::ZERO,
            step: 0,
            trace: Vec::new(),
            turn: None,
            pending: Vec::new(),
            invalidated_stop_requests: BTreeSet::new(),
            known_threads: Vec::new(),
            next_thread: 1,
            kernel_log,
            next_turn: 1,
            manual_turns_sent: 0,
            ledger: InvariantLedger::default(),
        };
        simulation.record(format!("seed {seed}, {settings:?}"));
        simulation
    }

    /*
     * 시나리오를 돌리고, 행동을 멈춘 뒤 커널이 일을 끝내는지 본다. 성공하면 기록 전체를 돌려준다. 커널이 panic 해도
     * 그때까지의 기록을 실패 보고에 넣는다.
     */
    fn run(mut self) -> Result<Vec<String>, (Violation, Vec<String>)> {
        let outcome = catch_unwind(AssertUnwindSafe(|| self.run_scenario()));
        let result = match outcome {
            Ok(result) => result,
            Err(panic) => Err(self.violation(panic_message(panic.as_ref()))),
        };
        let result = result.and_then(|()| {
            let step = self.step;
            // 커널을 버려야 기록기가 남은 사건을 다 쓴다.
            drop(self.host);
            let records = self.kernel_log.0.lock().expect("kernel log should lock");
            check_kernel_records(&records).map_err(|message| Violation { step, message })
        });
        match result {
            Ok(()) => Ok(self.trace),
            Err(violation) => Err((violation, self.trace)),
        }
    }

    fn run_scenario(&mut self) -> Result<(), Violation> {
        self.attach_client();
        self.send_command(AppCommand::SetAutoFollowMaxTurns {
            value: self.settings.max_auto_turns,
        });
        self.run_host_cycle(Duration::ZERO)?;
        for _ in 0..SCENARIO_STEPS {
            self.step += 1;
            let action = self.choose_action();
            self.perform(action)?;
        }
        self.settle()
    }

    fn choose_action(&mut self) -> SimAction {
        if self.queued_core_inputs >= CORE_INPUT_FLUSH_THRESHOLD {
            return SimAction::RunHostCycle;
        }
        let mut choices = vec![(SimAction::RunHostCycle, 6), (SimAction::ToggleClient, 1)];
        if self.manual_turns_sent < self.settings.manual_turn_budget {
            choices.push((SimAction::SubmitManualTurn, 1));
        }
        if self.turn.is_some() {
            choices.push((SimAction::AdvanceTurn, 3));
        }
        if self.turn.is_some() && self.settings.stop_requests {
            choices.push((SimAction::RequestStop, 1));
        }
        if !self.pending.is_empty() {
            choices.push((SimAction::DeliverCompletion, 3));
        }
        let total: u64 = choices.iter().map(|(_, weight)| weight).sum();
        let mut pick = self.rng.below(total);
        for (action, weight) in choices {
            if pick < weight {
                return action;
            }
            pick -= weight;
        }
        unreachable!("the pick is below the total weight")
    }

    fn perform(&mut self, action: SimAction) -> Result<(), Violation> {
        match action {
            SimAction::RunHostCycle => {
                let advance = Duration::from_millis(self.rng.below(1501));
                return self.run_host_cycle(advance);
            }
            SimAction::SubmitManualTurn => self.submit_manual_turn(),
            SimAction::RequestStop => {
                self.record("client: stop all sessions".to_string());
                self.send_command(AppCommand::RequestStopAllSessions);
            }
            SimAction::ToggleClient => self.toggle_client(),
            SimAction::AdvanceTurn => self.advance_turn(),
            SimAction::DeliverCompletion => self.deliver_completion(),
        }
        Ok(())
    }

    /*
     * 행동을 멈추고 바깥 세계의 남은 일(진행 중 턴, 완료)을 모두 돌려준다. 그 뒤 커널에 남은 일이 없어야 한다
     * (불변식 I4). 자동 후속 사슬은 최대 턴 수로 끝나므로 정해진 주기 안에 끝나야 한다.
     */
    fn settle(&mut self) -> Result<(), Violation> {
        self.record("settle: no more client actions".to_string());
        for _ in 0..SETTLE_STEPS {
            self.step += 1;
            if self.turn.is_some() {
                self.advance_turn();
            } else if !self.pending.is_empty() {
                self.deliver_completion();
            }
            self.run_host_cycle(Duration::from_secs(1))?;
            if self.is_quiet() {
                return Ok(());
            }
        }
        Err(self.violation(format!(
            "I4: the kernel did not settle: remaining work {}, fake turn {:?}, pending completions {}",
            self.host.runtime.has_remaining_work(),
            self.turn,
            self.pending.len()
        )))
    }

    fn is_quiet(&self) -> bool {
        self.turn.is_none() && self.pending.is_empty() && !self.host.runtime.has_remaining_work()
    }

    fn attach_client(&mut self) {
        let (reply, answer) = mpsc::channel();
        self.send_host_input(KernelHostInput::Attach { reply });
        let client = answer
            .try_recv()
            .expect("the host should answer the attach in the same step");
        self.record(format!("client {} attached", client.0));
        self.client = Some(client);
    }

    fn toggle_client(&mut self) {
        match self.client.take() {
            Some((client_id, _)) => {
                self.record(format!("client {client_id} detached"));
                self.send_host_input(KernelHostInput::Detach { client_id });
            }
            None => self.attach_client(),
        }
    }

    /// 클라이언트 명령은 실제 클라이언트처럼 호스트 입력함으로 보낸다. 응답은 이벤트로도 오므로 버린다.
    fn send_command(&mut self, command: AppCommand) {
        let (reply, _answer) = mpsc::channel();
        self.send_host_input(KernelHostInput::Command { command, reply });
    }

    fn send_host_input(&mut self, input: KernelHostInput) {
        self.inbox
            .send(input)
            .expect("the simulation owns the host loop");
        assert!(
            self.host.handle_pending_inputs(),
            "the simulation never shuts the host down"
        );
    }

    fn submit_manual_turn(&mut self) {
        self.manual_turns_sent += 1;
        let thread_id = self.choose_thread_for_manual_turn();
        let prompt = format!("task {}", self.manual_turns_sent);
        self.record(format!("client: submit '{prompt}' to {thread_id:?}"));
        self.send_command(AppCommand::SubmitTurn(Box::new(TurnSubmissionRequest {
            workspace_directory: WORKSPACE.to_string(),
            thread_id,
            prompt: prompt.clone(),
            transcript_text: prompt,
            image_paths: Vec::new(),
            prompt_origin: CorePromptOrigin::Manual,
            auto_follow_source: None,
            planning_handoff: None,
            turn_options: Default::default(),
            slot_lease_handoff: None,
        })));
    }

    fn choose_thread_for_manual_turn(&mut self) -> Option<String> {
        if self.known_threads.is_empty() || self.rng.percent(50) {
            return None;
        }
        let index = self.rng.index(self.known_threads.len());
        Some(self.known_threads[index].clone())
    }

    /// 커널 한 주기를 돌리고, 그 주기에 나온 효과를 바깥 세계에 넘긴다. 가짜 시계는 `advance` 만큼 간다.
    fn run_host_cycle(&mut self, advance: Duration) -> Result<(), Violation> {
        self.elapsed += advance;
        let now = self.started_at + self.elapsed;
        assert!(
            self.host.handle_pending_inputs(),
            "the simulation never shuts the host down"
        );
        self.host.run_cycle(now, Duration::ZERO);
        self.queued_core_inputs = 0;
        self.drain_client_events();
        for effect in self.effects.take() {
            self.accept_effect(effect)?;
        }
        self.check_remaining_work_while_busy()
    }

    /*
     * I5: 바깥 세계에서 턴이 돌거나 평가가 진행 중이면 커널은 남은 일이 있다고 답해야 한다. 이 답으로 커널을 내릴지
     * 정하므로(plan 18 K3-5), 일이 있는데 없다고 하면 진행 중인 일을 잃는다.
     */
    fn check_remaining_work_while_busy(&self) -> Result<(), Violation> {
        let evaluating = self
            .pending
            .iter()
            .any(|pending| matches!(pending, PendingCompletion::PostTurnEvaluation { .. }));
        if (self.turn.is_some() || evaluating) && !self.host.runtime.has_remaining_work() {
            return Err(self.violation(format!(
                "I5: the kernel reports no remaining work while turn {:?} runs or an evaluation is pending ({evaluating})",
                self.turn.as_ref().map(|turn| &turn.turn_id)
            )));
        }
        Ok(())
    }

    fn drain_client_events(&mut self) {
        let Some((_, events)) = &self.client else {
            return;
        };
        let names: Vec<String> = events.try_iter().filter_map(client_event_name).collect();
        for name in names {
            self.record(format!("client event: {name}"));
        }
    }

    fn accept_effect(&mut self, effect: CoreEffect) -> Result<(), Violation> {
        match effect {
            CoreEffect::SubmitTurn {
                correlation,
                request,
            } => self.start_fake_turn(correlation, *request),
            CoreEffect::EvaluatePostTurn { correlation, .. } => {
                self.start_fake_evaluation(correlation)
            }
            CoreEffect::RequestStopAllSessions {
                correlation,
                attempt,
            } => {
                self.record(format!(
                    "app-server: stop request {} {attempt:?} for {:?}",
                    correlation.generation,
                    correlation.turn_submission.map(|turn| turn.generation)
                ));
                self.pending.push(PendingCompletion::StopAttempt {
                    correlation,
                    attempt,
                });
                Ok(())
            }
            CoreEffect::InvalidateStopRequest { correlation } => {
                self.record(format!(
                    "app-server: stop request {} invalidated",
                    correlation.generation
                ));
                self.invalidated_stop_requests
                    .insert(correlation.generation);
                Ok(())
            }
            CoreEffect::CancelManualPromptPreparation { .. } => Ok(()),
            other => Err(self.violation(format!(
                "the simulation does not model this effect yet: {}",
                variant_name(&format!("{other:?}"))
            ))),
        }
    }

    /// 불변식 I1(턴은 하나만)과 I3 을 본다.
    fn start_fake_turn(
        &mut self,
        correlation: TurnSubmissionCorrelation,
        request: TurnSubmissionRequest,
    ) -> Result<(), Violation> {
        if let Some(running) = &self.turn {
            return Err(self.violation(format!(
                "I1: a second turn was submitted while {} runs",
                running.turn_id
            )));
        }
        self.ledger
            .record_turn_submission(&request, self.settings.max_auto_turns)
            .map_err(|message| self.violation(message))?;
        let thread_id = request.thread_id.clone().unwrap_or_else(|| {
            let thread_id = format!("thread-{}", self.next_thread);
            self.next_thread += 1;
            thread_id
        });
        let turn_id = format!("turn-{}", self.next_turn);
        self.next_turn += 1;
        self.record(format!(
            "app-server: accept {turn_id} on {thread_id} ({:?}, submission {})",
            request.prompt_origin, correlation.generation
        ));
        self.turn = Some(FakeTurn {
            correlation,
            thread_id,
            turn_id,
            stage: TurnStage::Submitted,
            interrupt_requested: false,
        });
        Ok(())
    }

    fn start_fake_evaluation(
        &mut self,
        correlation: PostTurnEvaluationCorrelation,
    ) -> Result<(), Violation> {
        self.ledger
            .record_evaluation_start(&correlation)
            .map_err(|message| self.violation(message))?;
        self.record(format!(
            "evaluator: start {} on {} (generation {})",
            correlation.completed_turn_id, correlation.thread_id, correlation.generation
        ));
        self.pending
            .push(PendingCompletion::PostTurnEvaluation { correlation });
        Ok(())
    }

    /// 진행 중 턴을 한 단계 보낸다: 시작 알림, 또는 종료 알림. 시작 전후에 가끔 스트림이 실패한다.
    fn advance_turn(&mut self) {
        let Some(turn) = self.turn.clone() else {
            return;
        };
        let stream_fails = self.rng.percent(10);
        match turn.stage {
            _ if stream_fails => self.fail_turn(&turn),
            TurnStage::Submitted => self.start_turn(turn),
            TurnStage::Started => self.finish_turn(&turn),
        }
    }

    /// 실제 worker 는 시작 전이든 뒤든 스트림 오류를 `Failed` 하나로 알린다 (`publish_turn_submission_failure`).
    fn fail_turn(&mut self, turn: &FakeTurn) {
        self.record(format!(
            "app-server: {} failed while {:?}",
            turn.turn_id, turn.stage
        ));
        self.turn = None;
        self.send_core_input(CoreInput::ConversationStreamUpdated {
            correlation: turn.correlation,
            event: TurnStreamEvent::Failed {
                message: "simulated stream failure".to_string(),
            },
        });
    }

    fn start_turn(&mut self, mut turn: FakeTurn) {
        self.record(format!("app-server: {} started", turn.turn_id));
        if !self.known_threads.contains(&turn.thread_id) {
            self.known_threads.push(turn.thread_id.clone());
        }
        self.send_core_input(CoreInput::ConversationTurnWorkspaceChanged {
            correlation: turn.correlation,
            workspace_directory: WORKSPACE.to_string(),
        });
        self.send_core_input(CoreInput::ConversationStreamUpdated {
            correlation: turn.correlation,
            event: TurnStreamEvent::ThreadPrepared {
                thread_id: turn.thread_id.clone(),
                title: format!("Simulated {}", turn.thread_id),
                cwd: WORKSPACE.to_string(),
                runtime_envelope: Box::<ConversationRuntimeEnvelope>::default(),
            },
        });
        self.send_core_input(CoreInput::ConversationStreamUpdated {
            correlation: turn.correlation,
            event: TurnStreamEvent::TurnStarted {
                turn_id: turn.turn_id.clone(),
                runtime_request: Box::<ConversationRuntimeConfigurationRequest>::default(),
            },
        });
        turn.stage = TurnStage::Started;
        self.turn = Some(turn);
    }

    fn finish_turn(&mut self, turn: &FakeTurn) {
        let outcome = if turn.interrupt_requested {
            ConversationTurnTerminalOutcome::Interrupted
        } else {
            ConversationTurnTerminalOutcome::Completed
        };
        self.record(format!("app-server: {} ended {outcome:?}", turn.turn_id));
        self.turn = None;
        let receipt = ConversationTurnTerminalReceipt::new(
            turn.thread_id.clone(),
            turn.turn_id.clone(),
            outcome,
        )
        .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed);
        self.send_core_input(CoreInput::ConversationStreamUpdated {
            correlation: turn.correlation,
            event: TurnStreamEvent::TurnTerminal {
                receipt,
                execution_snapshot_capture: Some(TurnSnapshotCapture::capture_failed(
                    WORKSPACE,
                    "the simulation does not capture snapshots".to_string(),
                )),
            },
        });
    }

    /// 기다리는 완료 하나를 난수로 골라 돌려준다. 완료는 도착 순서와 다르게 돌아올 수 있다.
    fn deliver_completion(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let index = self.rng.index(self.pending.len());
        match self.pending.remove(index) {
            PendingCompletion::PostTurnEvaluation { correlation } => {
                self.complete_evaluation(correlation);
            }
            PendingCompletion::StopAttempt {
                correlation,
                attempt,
            } => self.complete_stop_attempt(correlation, attempt),
        }
    }

    /*
     * 평가 요청의 문맥(자동 후속 한도, 중지 키워드)은 일부러 보지 않는다. 실제 평가기는 한도를 넘으면 멈춤을 답한다.
     * 가짜는 한도와 상관없이 프롬프트를 답할 수 있으므로, 호스트와 core 의 두 번째 방어를 시험한다.
     */
    fn complete_evaluation(&mut self, correlation: PostTurnEvaluationCorrelation) {
        let mut execution = post_turn_execution(&correlation);
        if self.rng.percent(30) {
            execution.evaluation.action = PostTurnContinuationAction::SkipAutoFollow {
                reason: PostTurnAutoFollowSkipReason::StopKeywordMatched,
            };
        } else {
            self.ledger.record_auto_prompt(&correlation);
        }
        self.record(format!(
            "evaluator: {} on {} -> {}",
            correlation.completed_turn_id,
            correlation.thread_id,
            variant_name(&format!("{:?}", execution.evaluation.action))
        ));
        self.send_core_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::PostTurnEvaluationCompleted {
                correlation,
                execution: Box::new(execution),
            },
        ));
    }

    /*
     * 중지 요청이 app-server 에 닿았다. 실제 worker 처럼 무효가 된 요청은 실행하지 않고 실패로 답한다. 유효한 요청은
     * 가끔 실패하고, 성공하면 그 요청이 가리키는 턴이 시작된 경우에만 그 턴을 중단으로 끝낸다.
     */
    fn complete_stop_attempt(
        &mut self,
        correlation: StopRequestCorrelation,
        attempt: StopRequestAttempt,
    ) {
        let result = if self
            .invalidated_stop_requests
            .contains(&correlation.generation)
        {
            Err("stop request was superseded before provider execution".to_string())
        } else if self.rng.percent(10) {
            Err("simulated stop failure".to_string())
        } else {
            self.interrupt_target_turn(correlation);
            Ok(())
        };
        self.record(format!(
            "app-server: stop request {} {attempt:?} -> {result:?}",
            correlation.generation
        ));
        self.send_core_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::StopRequestAttemptCompleted {
                correlation,
                attempt,
                result,
            },
        ));
    }

    fn interrupt_target_turn(&mut self, correlation: StopRequestCorrelation) {
        let Some(target) = correlation.turn_submission else {
            return;
        };
        if let Some(turn) = self
            .turn
            .as_mut()
            .filter(|turn| turn.correlation == target && turn.stage == TurnStage::Started)
        {
            turn.interrupt_requested = true;
        }
    }

    fn send_core_input(&mut self, input: CoreInput) {
        self.queued_core_inputs += 1;
        self.core_inputs
            .send(input)
            .expect("the host loop keeps the core input receiver");
    }

    fn record(&mut self, line: String) {
        self.trace.push(format!("{:>4} {line}", self.step));
    }

    fn violation(&self, message: String) -> Violation {
        Violation {
            step: self.step,
            message,
        }
    }
}

fn client_event_name(event: KernelHostEvent) -> Option<String> {
    match event {
        KernelHostEvent::Core(event) => match *event {
            // 스냅샷은 크고 매 주기 바뀐다. 기록에는 이름만 남기고 스냅샷 이벤트는 뺀다.
            AppEvent::SnapshotChanged(_) => None,
            other => Some(variant_name(&format!("{other:?}")).to_string()),
        },
        KernelHostEvent::Parallel(_) => Some("Parallel".to_string()),
        KernelHostEvent::Planning(_) => Some("Planning".to_string()),
        KernelHostEvent::Resync => Some("Resync".to_string()),
        // 판단 기록은 I6 가 따로 본다. 기록에 넣으면 같은 seed 의 기록이 길어지기만 한다.
        KernelHostEvent::Recorded(_) => None,
    }
}

/// `Debug` 문자열에서 바깥 variant 이름만 꺼낸다.
fn variant_name(debug: &str) -> &str {
    let end = debug
        .find(|character: char| !(character.is_alphanumeric() || character == '_'))
        .unwrap_or(debug.len());
    &debug[..end]
}

/// seed 하나를 돌린다. 불변식이 깨지거나 커널이 panic 하면 seed 와 마지막 기록을 담은 문장을 돌려준다.
fn run_seed(seed: u64) -> Result<Vec<String>, String> {
    Simulation::new(seed).run().map_err(|(violation, trace)| {
        failure_report(
            seed,
            &format!("step {}: {}", violation.step, violation.message),
            &trace,
        )
    })
}

fn failure_report(seed: u64, message: &str, trace: &[String]) -> String {
    let tail_start = trace.len().saturating_sub(TRACE_TAIL);
    format!(
        "kernel simulation failed: {message}\nreproduce: AKRA_SIM_SEED={seed} cargo test --locked kernel_simulation\nlast trace lines:\n{}",
        trace[tail_start..].join("\n")
    )
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    let detail = panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|text| text.to_string()))
        .unwrap_or_else(|| "non-text panic".to_string());
    format!("panic: {detail}")
}

/// 돌릴 seed 목록이다. `AKRA_SIM_SEED` 는 seed 하나를, `AKRA_SIM_SEEDS` 는 0 부터 몇 개를 돌릴지 정한다.
fn seeds_to_run() -> Vec<u64> {
    if let Some(seed) = env_number("AKRA_SIM_SEED") {
        return vec![seed];
    }
    let count = env_number("AKRA_SIM_SEEDS").unwrap_or(DEFAULT_SEED_COUNT);
    (0..count).collect()
}

fn env_number(name: &str) -> Option<u64> {
    let value = std::env::var(name).ok()?;
    Some(
        value
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("{name} must be a whole number, got {value:?}")),
    )
}

#[test]
fn kernel_simulation_keeps_the_invariants_for_every_seed() {
    let failures: Vec<String> = seeds_to_run()
        .into_iter()
        .filter_map(|seed| run_seed(seed).err())
        .collect();
    assert!(
        failures.is_empty(),
        "{} seed(s) failed\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn kernel_simulation_replays_the_same_trace_for_the_same_seed() {
    // P2 의 바탕이다: 같은 seed 가 다른 순서를 만들면 실패를 seed 로 다시 만들 수 없다.
    for seed in 0..8 {
        let first = run_seed(seed).expect("the seed should pass");
        let second = run_seed(seed).expect("the seed should pass again");
        assert_eq!(first, second, "seed {seed} should replay the same trace");
    }
}

#[test]
fn kernel_simulation_reaches_follow_up_turns_and_stops() {
    // 하네스가 실제로 깊은 경로를 지나는지 본다. 이 경로가 없으면 불변식 I2, I3 는 비어 있는 확인이다.
    let traces: Vec<String> = (0..DEFAULT_SEED_COUNT)
        .filter_map(|seed| run_seed(seed).ok())
        .flatten()
        .collect();
    for expected in [
        "(AutoFollow",
        "ended Completed",
        "ended Interrupted",
        "failed while Submitted",
        "failed while Started",
        "-> SkipAutoFollow",
        "-> QueueAutoPrompt",
        "-> Err(\"stop request was superseded",
        "-> Err(\"simulated stop failure",
        "detached",
    ] {
        assert!(
            traces.iter().any(|line| line.contains(expected)),
            "no seed reached '{expected}'"
        );
    }
}

#[test]
fn kernel_simulation_random_numbers_stay_fixed() {
    // 보고된 seed 가 나중에도 같은 실패를 만들려면 난수 순서가 고정되어야 한다.
    let mut rng = SimRng::new(7);
    let values: Vec<u64> = (0..3).map(|_| rng.below(1000)).collect();
    assert_eq!(values, FIXED_SEED_7_VALUES);
}

const FIXED_SEED_7_VALUES: [u64; 3] = [487, 804, 346];
