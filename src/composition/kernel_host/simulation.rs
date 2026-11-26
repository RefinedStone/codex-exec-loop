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
//! - 가짜 planner: thread 지정 프롬프트의 준비에 작업 하나와 실행할 프롬프트를 답한다.
//! - 재시작: 커널을 버리고 같은 기록으로 다시 조립한다. 이전 커널의 진행 중 일은 바깥 세계에서도 버린다.
//!
//! 무엇을 언제 할지는 seed 하나로 만든 난수가 고른다. 같은 seed 는 같은 순서를 만든다. 실패하면 seed 를 알리므로
//! `AKRA_SIM_SEED=<seed>` 로 같은 실패를 다시 만든다.
//!
//! 지금 모델하는 범위: 수동 턴 제출(새 thread, 기존 thread), thread 지정 프롬프트(준비, 큐에 남김, 다시 냄), 중지
//! 요청(성공, 실패, 무효), 턴 시작·종료·실패, 턴 뒤 평가, 자동 후속, 클라이언트 떼기와 다시 붙기, 커널 재시작(기록을
//! 다 쓴 뒤), 기록을 다 쓰지 못한 강제 종료(#2806: 기록의 끝을 잃음, 마지막 효과를 버림), 승인 요청과 그 끝(app-server
//! 가 알린 결정, 결정 없이 끝난 턴, 재시작), 멱등 키가 있는 thread 지정 프롬프트의 재전송, 클라이언트의 승인 결정
//! 명령과 steer(#2805: 효과의 완료는 늦게 올 수 있고, 지난 턴이나 끝난 승인을 가리킬 수 있다), 시계. 병렬 모드는 아직
//! 모델하지 않는다. 강제 종료 뒤의 기대값은 `docs/reference/kernel-crash-recovery.md` 의 계약 C1~C5 를 따른다.
//!
//! 동시 턴 수: 기본 seed 는 제품 값 1 로 돈다. 값 2 의 seed 는 다른 thread 의 턴 둘을 함께 돌린다 (plan 18 Q5-3).
//! 그때 자동 후속을 이어 가는 thread 는 하나여야 한다 (I12).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{
    KernelClientId, KernelCommandOutcome, KernelHostEvent, KernelHostInput, KernelHostLoop,
    KernelIdleClock,
};
use crate::adapter::inbound::tui::app::test_helpers::{
    test_parallel_mode_control_plane_composition, test_planning_services,
};
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::application::port::outbound::kernel_event_log_port::KernelEventLogPort;
use crate::composition::native_client_runtime::NativeClientRuntime;
use crate::composition::native_client_runtime::tests::post_turn_execution;
use crate::core::app::{
    AppCommand, AppEvent, ApprovalDecisionCorrelation, CoreEffect, CoreEffectCompletion, CoreInput,
    CorePromptOrigin, PlanningWorkspaceOperationCorrelation, PlanningWorkspaceResetIntent,
    PlanningWorkspaceResetSnapshot, PlanningWorkspaceResetTarget, PostTurnEvaluationCorrelation,
    SessionCatalogLoadCorrelation, SessionCatalogLoadIntent, SessionCatalogReadySnapshot,
    StopRequestAttempt, StopRequestCorrelation, ThreadPromptSubmission, TurnConcurrencyPolicy,
    TurnSteerCorrelation, TurnStreamEvent, TurnSubmissionCorrelation, TurnSubmissionRequest,
};
use crate::core::runtime::{CoreEffectExecutor, CoreInputSender};
use crate::domain::conversation::{
    ConversationApprovalDecision, ConversationApprovalRequest, ConversationApprovalRequestIdentity,
    ConversationApprovalRequestKind, ConversationApprovalResolution, ConversationTurnSteerReceipt,
    ConversationTurnSteerRequest,
};
use crate::domain::conversation_runtime_envelope::{
    ConversationRuntimeConfigurationRequest, ConversationRuntimeEnvelope,
};
use crate::domain::kernel_event::{
    ApprovalResolution, KernelCommandDescriptor, KernelEvent, RecordedKernelEvent,
    ThreadPromptSettlement, open_queued_thread_prompts,
};
use crate::domain::kernel_spec::{KernelCommandKind, OperationState};
use crate::domain::planning::{
    ManualPromptCorrelation, ManualPromptIntakeOutcome, ManualPromptMainSessionHandoff,
    ManualPromptOutcome, PostTurnAutoFollowSkipReason, PostTurnContinuationAction,
    PostTurnContinuationPermit, RuntimeProjection, TurnSnapshotCapture,
};
use crate::domain::recent_sessions::RecentSessions;
use crate::domain::session_summary::SessionSummary;
use crate::domain::turn_terminal::{
    ConversationTurnApplicationDelivery, ConversationTurnTerminalOutcome,
    ConversationTurnTerminalReceipt,
};

const WORKSPACE: &str = "/akra-simulation/workspace";
/// 커널을 켜기 전부터 app-server 에 있는 thread 이다. 세션 목록에 있어야 thread 지정 프롬프트를 받는다.
const CATALOG_THREADS: [&str; 2] = ["thread-a", "thread-b"];
/// 한 seed 에서 커널을 다시 켜는 최대 횟수이다.
const MAX_RESTARTS: usize = 2;
/// 한 seed 에서 thread 지정 프롬프트를 같은 멱등 키로 다시 보내는 최대 횟수이다 (plan 19 Q3-6).
const MAX_PROMPT_RESENDS: usize = 2;
/// 한 seed 에서 시뮬레이션이 고르는 행동 수이다.
const SCENARIO_STEPS: usize = 240;
/// 행동을 멈춘 뒤 커널이 일을 끝낼 때까지 허용하는 주기 수이다. 자동 후속 사슬이 끝날 만큼 넉넉하다.
const SETTLE_STEPS: usize = 400;
/// PR 마다 도는 seed 수이다 (plan 19 Q-Q4). 더 돌리려면 `AKRA_SIM_SEEDS` 를 준다.
const DEFAULT_SEED_COUNT: u64 = 24;
/// 커널이 thread 지정 프롬프트를 바로 거절했을 때의 기록 줄 머리이다. I10 이 거절을 실제로 보았는지 세는 데 쓴다.
const REJECTED_AT_ONCE: &str = "kernel: rejected at once";
/// 가짜 app-server 가 다른 턴이 도는 동안 턴을 받았을 때의 기록 줄 끝이다. 값 2 의 seed 가 실제로 턴 둘을 돌렸는지 센다.
const ALONGSIDE_ANOTHER_TURN: &str = "alongside another turn";
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
    /// 클라이언트가 thread 를 지정해 보내는 프롬프트 수이다 (plan 18 K3-3b). 커널이 바쁘면 큐에 남는다 (Q2-3).
    thread_prompt_budget: usize,
    /// 커널을 무작위 시점에 버리고 기록으로 다시 조립한다 (plan 19 Q3-7).
    restarts: bool,
    /// 가짜 app-server 가 도는 턴에서 요청하는 승인 수이다 (plan 19 Q3-5).
    approval_budget: usize,
    /// 클라이언트가 보내는 승인 결정 명령 수이다 (#2805). 끝난 승인을 가리키는 늦은 결정도 섞는다.
    decision_budget: usize,
    /// 클라이언트가 보내는 steer 명령 수이다 (#2805). 끝난 턴을 가리키는 늦은 steer 도 섞는다.
    steer_budget: usize,
    /// 커널의 동시 턴 수이다 (plan 18 Q5-3). 제품 값은 1 이다. 값 2 의 seed 는 따로 돈다.
    turn_concurrency: usize,
}

impl ScenarioSettings {
    fn from_rng(rng: &mut SimRng) -> Self {
        Self {
            max_auto_turns: rng.index(4),
            manual_turn_budget: 3 + rng.index(6),
            stop_requests: rng.percent(50),
            thread_prompt_budget: rng.index(5),
            restarts: rng.percent(50),
            approval_budget: rng.index(4),
            decision_budget: rng.index(4),
            steer_budget: rng.index(4),
            // 난수를 쓰지 않는다. 값 1 의 seed 가 이전과 같은 시나리오를 만든다.
            turn_concurrency: 1,
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

/// 가짜 app-server 에서 도는 턴이다. 동시 턴 수만큼, thread 마다 하나씩 있을 수 있다 (불변식 I1).
#[derive(Debug, Clone)]
struct FakeTurn {
    correlation: TurnSubmissionCorrelation,
    thread_id: String,
    origin: CorePromptOrigin,
    turn_id: String,
    stage: TurnStage,
    interrupt_requested: bool,
    /// 이 턴이 결정을 기다리는 승인의 id 이다.
    approval: Option<String>,
    /// app-server 가 그 승인에 받은 클라이언트의 결정이다. 승인 하나는 결정을 한 번만 받는다 (I8).
    decision: Option<ConversationApprovalDecision>,
}

/*
 * 가짜 app-server 가 요청한 승인 하나이다 (I8). 기대값은 시뮬레이션이 한 일에서만 만든다. 커널의 기록을 보고 만들지
 * 않는다. 승인 id 는 실제 app-server adapter 처럼 커널을 다시 켤 때마다 1 부터 다시 센다.
 */
#[derive(Debug, Clone)]
struct SimApproval {
    approval_id: String,
    thread_id: String,
    turn_id: String,
    /// 이 승인을 요청한 커널 실행이다. 재시작 수와 같다.
    lifetime: usize,
    /// 커널이 요청 입력을 처리했다. 입력을 넣은 뒤에 커널 주기가 돌았다.
    request_delivered: bool,
    /// app-server 가 한 일로 정해진 끝이다. 없으면 아직 결정을 기다린다.
    end: Option<ApprovalResolution>,
    /// 커널이 그 끝을 알리는 입력을 처리했다.
    end_delivered: bool,
    /// 커널이 요청을 받기 전에 다시 켜졌다. 기록에 없어야 한다.
    never_recorded: bool,
}

/// 바깥 세계가 나중에 돌려줄 완료이다. 어떤 순서로 돌려줄지는 난수가 고른다.
#[derive(Debug, Clone)]
enum PendingCompletion {
    PostTurnEvaluation {
        correlation: PostTurnEvaluationCorrelation,
        /// 커널이 이 평가의 결과를 아직 받는지 알려 준다. 정책이 바뀌면(중지 등) 커널이 무효로 만든다.
        permit: PostTurnContinuationPermit,
    },
    StopAttempt {
        correlation: StopRequestCorrelation,
        attempt: StopRequestAttempt,
        /// 있으면 이 (thread, 턴) 하나만 멈추는 요청이다 (#2304).
        target: Option<(String, String)>,
    },
    SessionCatalog {
        correlation: SessionCatalogLoadCorrelation,
    },
    /// thread 지정 프롬프트의 준비이다. 가짜 planner 가 작업 하나와 실행할 프롬프트를 답한다.
    PromptPreparation {
        correlation: ManualPromptCorrelation,
        raw_prompt: String,
    },
    /// 클라이언트의 승인 결정을 app-server 에 보낸다 (#2805). `op` 는 그 명령의 `client_ops` 자리이다.
    ApprovalDecision {
        correlation: ApprovalDecisionCorrelation,
        op: usize,
    },
    /// 클라이언트의 steer 를 app-server 에 보낸다 (#2805).
    Steer {
        correlation: TurnSteerCorrelation,
        request: ConversationTurnSteerRequest,
        op: usize,
    },
    /// planning 초기화이다. 가짜 실행기는 지울 파일이 없다고 답한다.
    PlanningReset {
        correlation: PlanningWorkspaceOperationCorrelation,
        op: Option<usize>,
    },
    /// app-server 는 일을 했지만 worker 의 완료가 늦게 온다 (#2805). 그 사이 턴과 승인은 이어서 바뀐다.
    LateCompletion {
        input: CoreInput,
        op: usize,
        succeeded: bool,
    },
}

/*
 * 클라이언트가 보낸, 끝을 기록으로 알리는 명령 하나이다 (#2805: 승인 결정, steer, planning 초기화). 기대하는 끝은
 * 시뮬레이션이 한 일에서만 정한다: 바로 거절, 효과의 성공·실패, 또는 커널이 그 완료를 처리하기 전에 다시 켜져 `lost`.
 * 강제 종료가 받은 기록을 잃은 명령은 그 연산 id 를 새 커널이 다시 쓸 수 있어 지운다 (`forgotten`).
 */
#[derive(Debug, Clone)]
struct ClientOperation {
    op_id: i64,
    label: String,
    lifetime: usize,
    /// steer 가 가리킨 턴이다. 커널은 그 턴 그대로 app-server 에 보내야 한다.
    steer_turn: Option<String>,
    /// 기대하는 끝이다. `None` 이면 아직 효과가 끝나지 않았다.
    expected: Option<OperationState>,
    /// 기대하는 끝을 정한 입력을 커널이 처리했다 (그 뒤 커널 주기가 돌았다).
    end_processed: bool,
    forgotten: bool,
}

/// 시뮬레이션이 한 걸음에 고를 수 있는 행동이다.
#[derive(Debug, Clone, Copy)]
enum SimAction {
    SubmitManualTurn,
    SubmitThreadPrompt,
    /// 보낸 thread 지정 프롬프트를 같은 멱등 키로 다시 보낸다 (응답을 받지 못한 클라이언트).
    ResendThreadPrompt,
    RestartKernel,
    RequestStop,
    /// 승인 결정을 보낸다. 지금 기다리는 승인이거나, 이미 끝난 승인이다 (#2805).
    DecideApproval,
    /// steer 를 보낸다. 지금 도는 턴이거나, 이미 끝난 턴이다 (#2805).
    SteerTurn,
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

#[derive(Debug, Clone)]
struct PlannedPrompt {
    task_id: String,
    thread_id: String,
    submissions: usize,
    /// 강제 종료가 제출의 끝남 기록을 잃어 새 커널이 다시 낼 수 있는 횟수이다 (#2806 계약 C2).
    resubmissions_allowed: usize,
    /// 강제 종료가 기록은 남기고 제출 효과를 버렸다. app-server 가 한 번도 받지 못했을 수 있다 (계약 C3).
    submission_may_be_lost: bool,
}

/// 불변식 I2, I3 을 보려고 모으는 기록이다. 어긋나면 위반 문장을 돌려준다.
#[derive(Default)]
struct InvariantLedger {
    evaluated_turns: BTreeSet<(String, String)>,
    /// 평가가 자동 후속 프롬프트를 답한 완료 턴이다. 자동 후속은 이 턴에서만 나올 수 있다.
    auto_prompted_turns: BTreeSet<(String, String)>,
    followed_turns: BTreeSet<(String, String)>,
    /// thread 마다 사람이 낸 턴 뒤의 자동 후속 수이다. 한도는 thread(session)마다 따로 센다.
    auto_turns_since_manual: BTreeMap<String, usize>,
    /// 가짜 planner 가 답한 프롬프트마다 그 작업, 받을 thread, app-server 가 받은 횟수이다 (I7).
    planned_prompts: BTreeMap<String, PlannedPrompt>,
    /// 가짜 app-server 가 요청한 승인이다 (I8). 요청한 차례대로이다.
    approvals: Vec<SimApproval>,
    /// 가짜 planner 가 준비한 프롬프트이다 (I9). 프롬프트 하나는 한 번만 준비된다.
    prepared_prompts: BTreeSet<String>,
    /// 자동 후속으로 받은 턴이다 (I12).
    auto_follow_turns: BTreeSet<(String, String)>,
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

    /*
     * I7: thread 지정 프롬프트는 커널을 다시 켜도 많아야 한 번, 자기 thread 에만 닿는다. 강제 종료가 끝남 기록을 잃은
     * 만큼만 더 닿을 수 있다 (계약 C2).
     */
    fn record_planned_prompt_submission(
        &mut self,
        request: &TurnSubmissionRequest,
    ) -> Result<(), String> {
        let Some(planned) = self.planned_prompts.get_mut(&request.prompt) else {
            return Ok(());
        };
        planned.submissions += 1;
        if planned.submissions > 1 + planned.resubmissions_allowed {
            return Err(format!(
                "I7: the prompt of {} reached the app-server {} times",
                planned.task_id, planned.submissions
            ));
        }
        if request.thread_id.as_deref() != Some(planned.thread_id.as_str()) {
            return Err(format!(
                "I7: the prompt of {} for {} reached {:?}",
                planned.task_id, planned.thread_id, request.thread_id
            ));
        }
        Ok(())
    }

    /*
     * I7 의 끝 확인이다. 기록에 남김이 열린 채 남으면 안 된다. 다시 내서 끝났다고 기록한 남김은 app-server 가 그
     * 프롬프트를 정확히 한 번 받았어야 한다. 강제 종료가 있었으면 계약 C2, C3 이 정한 범위 안이다. 기록은 커널을 버린
     * 뒤에 읽는다.
     */
    fn check_queued_prompt_records(&self, recorded: &[RecordedKernelEvent]) -> Result<(), String> {
        let events = || recorded.iter().map(|record| record.event.clone());
        if let Some(open) = open_queued_thread_prompts(events()).first() {
            return Err(format!("I7: the prompt of {} stayed queued", open.task_id));
        }
        for event in events() {
            let KernelEvent::ThreadPromptSettled {
                task_id,
                settlement: ThreadPromptSettlement::Submitted,
                ..
            } = event
            else {
                continue;
            };
            let planned = self
                .planned_prompts
                .values()
                .find(|planned| planned.task_id == task_id);
            let submissions = planned.map_or(0, |planned| planned.submissions);
            let fewest =
                usize::from(!planned.is_some_and(|planned| planned.submission_may_be_lost));
            let most = 1 + planned.map_or(0, |planned| planned.resubmissions_allowed);
            if !(fewest..=most).contains(&submissions) {
                return Err(format!(
                    "I7: the record says the queued prompt of {task_id} was submitted, but the app-server got it {submissions} times"
                ));
            }
        }
        Ok(())
    }

    /// 이 커널 실행에서 아직 끝이 정해지지 않은 그 id 의 승인에 끝을 정한다. 이미 끝난 승인은 그대로 둔다.
    fn end_approval(&mut self, approval_id: &str, lifetime: usize, end: ApprovalResolution) {
        let open = self.approvals.iter_mut().rev().find(|approval| {
            approval.approval_id == approval_id
                && approval.lifetime == lifetime
                && approval.end.is_none()
        });
        if let Some(approval) = open {
            approval.end = Some(end);
        }
    }

    /// 커널 주기가 돌았다. 그 전에 넣은 입력은 커널이 모두 처리했다.
    fn mark_approval_inputs_delivered(&mut self, lifetime: usize) {
        for approval in &mut self.approvals {
            if approval.lifetime == lifetime {
                approval.request_delivered = true;
                approval.end_delivered = approval.end.is_some();
            }
        }
    }

    /*
     * 커널을 다시 켤 때 부른다. 커널이 요청을 처리하지 못했으면 그 승인은 기록에 없어야 한다. 요청은 처리했지만 끝을
     * 처리하지 못했으면(끝이 없거나, 끝을 알리는 입력이 아직 큐에 있었다) 새 커널이 잃은 것으로 닫아야 한다.
     */
    fn expect_approvals_after_restart(&mut self, ended_lifetime: usize) {
        for approval in &mut self.approvals {
            if approval.lifetime != ended_lifetime {
                continue;
            }
            if !approval.request_delivered {
                approval.never_recorded = true;
            } else if !approval.end_delivered {
                approval.end = Some(ApprovalResolution::Lost);
                approval.end_delivered = true;
            }
        }
    }

    /*
     * 강제 종료가 끝난 커널의 기록 일부를 잃었다 (#2806 계약 C4). `kept` 는 그 커널이 쓴 기록 가운데 디스크에 닿은
     * 것이다. 요청 기록을 잃은 승인은 기록에 없다. 요청은 남고 끝 기록을 잃은 승인은 새 커널이 `lost` 로 닫는다.
     * 승인 id 는 커널마다 1 부터 다시 세므로, 끝은 같은 id 의 요청 뒤에서만 찾는다 (앞에는 이전 커널의 승인을 닫은
     * 기록이 있을 수 있다).
     */
    fn forget_lost_approval_records(
        &mut self,
        ended_lifetime: usize,
        kept: &[RecordedKernelEvent],
    ) {
        for approval in &mut self.approvals {
            if approval.lifetime != ended_lifetime || !approval.request_delivered {
                continue;
            }
            let request = kept.iter().position(|record| {
                matches!(&record.event, KernelEvent::ApprovalRequested(request)
                    if request.approval_id == approval.approval_id)
            });
            let Some(request) = request else {
                approval.never_recorded = true;
                continue;
            };
            let end_kept = kept[request..].iter().any(|record| {
                matches!(&record.event, KernelEvent::ApprovalResolved { approval_id, .. }
                    if approval_id == &approval.approval_id)
            });
            if approval.end_delivered && !end_kept {
                approval.end = Some(ApprovalResolution::Lost);
            }
        }
    }

    /*
     * I8: 승인 기록이 맞다 (plan 19 Q3-5). 커널이 받은 승인은 요청한 차례대로 한 번씩 기록되고, thread 와 턴이 맞다.
     * 끝은 그 id 의 가장 가까운 앞 요청에 붙고, 한 번이며, 종류는 가짜 app-server 가 한 일과 같다. 커널이 받지 못한
     * 요청은 기록에 없다. 시나리오가 끝나면 열린 승인이 없다. 기록은 커널을 버린 뒤에 읽는다.
     */
    fn check_approval_records(&self, recorded: &[RecordedKernelEvent]) -> Result<(), String> {
        struct Seen {
            thread_id: String,
            turn_id: Option<String>,
            ends: Vec<ApprovalResolution>,
        }
        let mut seen: BTreeMap<String, Vec<Seen>> = BTreeMap::new();
        for record in recorded {
            match &record.event {
                KernelEvent::ApprovalRequested(approval) => {
                    seen.entry(approval.approval_id.clone())
                        .or_default()
                        .push(Seen {
                            thread_id: approval.thread_id.clone(),
                            turn_id: approval.turn_id.clone(),
                            ends: Vec::new(),
                        });
                }
                KernelEvent::ApprovalResolved {
                    thread_id,
                    approval_id,
                    resolution,
                } => {
                    let Some(request) = seen.get_mut(approval_id).and_then(|all| all.last_mut())
                    else {
                        return Err(format!("I8: {approval_id} ended before it was requested"));
                    };
                    if &request.thread_id != thread_id {
                        return Err(format!(
                            "I8: {approval_id} was requested on {} but ended on {thread_id}",
                            request.thread_id
                        ));
                    }
                    request.ends.push(*resolution);
                }
                _ => {}
            }
        }
        let mut expected: BTreeMap<&str, Vec<&SimApproval>> = BTreeMap::new();
        for approval in self.approvals.iter().filter(|a| !a.never_recorded) {
            expected
                .entry(approval.approval_id.as_str())
                .or_default()
                .push(approval);
        }
        if let Some((approval_id, requests)) = seen
            .iter()
            .find(|(id, requests)| expected.get(id.as_str()).map_or(0, Vec::len) != requests.len())
        {
            return Err(format!(
                "I8: the record has {} request(s) of {approval_id}, but the kernel received {}",
                requests.len(),
                expected.get(approval_id.as_str()).map_or(0, Vec::len)
            ));
        }
        for (approval_id, approvals) in expected {
            let requests = seen.get(approval_id).map_or(&[][..], Vec::as_slice);
            if requests.len() != approvals.len() {
                return Err(format!(
                    "I8: the kernel received {} request(s) of {approval_id}, but the record has {}",
                    approvals.len(),
                    requests.len()
                ));
            }
            for (approval, request) in approvals.iter().zip(requests) {
                let place_matches = request.thread_id == approval.thread_id
                    && request.turn_id.as_deref() == Some(approval.turn_id.as_str());
                if !place_matches {
                    return Err(format!(
                        "I8: {approval_id} belongs to {} on {}, but the record has {:?} on {}",
                        approval.turn_id, approval.thread_id, request.turn_id, request.thread_id
                    ));
                }
                if approval.end.is_none() || request.ends != approval.end.as_slice() {
                    return Err(format!(
                        "I8: {approval_id} of {} should end {:?}, but the record has ends {:?}",
                        approval.turn_id, approval.end, request.ends
                    ));
                }
            }
        }
        Ok(())
    }

    /// I3: 자동 후속은 평가가 프롬프트를 답한 완료 턴마다 한 번이고, 수동 턴 뒤 최대 턴 수를 넘지 않는다.
    fn record_turn_submission(
        &mut self,
        request: &TurnSubmissionRequest,
        max_auto_turns: usize,
    ) -> Result<(), String> {
        if request.prompt_origin != CorePromptOrigin::AutoFollow {
            if let Some(thread_id) = &request.thread_id {
                self.auto_turns_since_manual.remove(thread_id);
            }
            return Ok(());
        }
        let Some(source) = &request.auto_follow_source else {
            return Err("I3: a follow-up turn has no source turn".to_string());
        };
        let follow_ups = self
            .auto_turns_since_manual
            .entry(source.thread_id.clone())
            .or_default();
        *follow_ups += 1;
        if *follow_ups > max_auto_turns {
            return Err(format!(
                "I3: follow-up turn {follow_ups} on {} exceeds the limit {max_auto_turns}",
                source.thread_id
            ));
        }
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

/// 강제 종료한 커널이 쓴 기록 가운데 디스크에 닿은 개수를 고른다 (#2806).
type KeepWritten<'a> = dyn Fn(&[RecordedKernelEvent]) -> usize + 'a;

impl SimKernelLog {
    /*
     * 기록을 다 쓰지 못한 강제 종료이다 (#2806). 앞의 `kept` 개만 디스크에 닿았고 그 뒤는 잃었다. 기록기는 차례대로
     * 쓰므로 잃는 것은 언제나 끝 부분이다. 잃은 기록을 돌려준다.
     */
    fn lose_tail(&self, kept: usize) -> Vec<RecordedKernelEvent> {
        self.0
            .lock()
            .expect("kernel log should lock")
            .split_off(kept)
    }
}

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

    fn last_kernel_event_sequence(&self, _workspace_dir: &str) -> anyhow::Result<i64> {
        let records = self.0.lock().expect("kernel log should lock");
        Ok(records.last().map_or(0, |record| record.sequence))
    }

    /// 다시 켠 커널이 남김 종류의 사건을 읽는다 (plan 19 Q3-1).
    fn load_kernel_events(
        &self,
        _workspace_dir: &str,
        kinds: &[&str],
    ) -> anyhow::Result<Vec<KernelEvent>> {
        let records = self.0.lock().expect("kernel log should lock");
        Ok(records
            .iter()
            .filter(|record| kinds.contains(&record.event.kind()))
            .map(|record| record.event.clone())
            .collect())
    }
}

/// 가짜 바깥 세계에 연결한 커널 하나이다. 기록은 아직 붙이지 않았다.
struct SimKernel {
    host: KernelHostLoop,
    inbox: Sender<KernelHostInput>,
    effects: RecordedEffects,
    core_inputs: CoreInputSender,
}

fn build_sim_kernel() -> SimKernel {
    build_sim_kernel_with(1)
}

/// 동시 턴 수를 정한 커널이다 (plan 18 Q5-3). 제품 값은 1 이다.
fn build_sim_kernel_with(turn_concurrency: usize) -> SimKernel {
    let effects = RecordedEffects::default();
    let planning = test_planning_services(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));
    let parallel = test_parallel_mode_control_plane_composition(planning);
    /*
     * 실행 중인 커널과 달리 병렬 보기의 작업 공간을 정하지 않는다 (`show_workspace_to_parallel_control`). 정하면
     * 클라이언트가 붙을 때마다 병렬 제어부가 감독 점검을 실제 작업 스레드에서 돌리고, 그 완료 시점이 seed 를
     * 따르지 않는다. 병렬 제어부의 효과를 가짜로 바꾸는 일은 병렬 모드를 모델할 때 한다.
     */
    let (runtime, core_inputs) = NativeClientRuntime::new_with_turn_concurrency_for_test(
        parallel,
        Box::new(effects.clone()),
        TurnConcurrencyPolicy::new(turn_concurrency),
    );
    let (inbox, inputs) = mpsc::channel();
    SimKernel {
        host: KernelHostLoop::new(runtime, inputs, KernelIdleClock::default()),
        inbox,
        effects,
        core_inputs,
    }
}

/*
 * I6: 판단 사건 기록이 맞다 (plan 19 Q3-2). 같은 턴 제출은 한 번만 기록된다 (자동 후속은 core 입력을 다시 넣는다).
 * 같은 턴은 한 번 시작하고 한 번 끝나며, 끝은 시작 뒤에 온다.
 */
fn check_kernel_records(
    recorded: &[RecordedKernelEvent],
    lifetime_starts: &[usize],
) -> Result<(), String> {
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
    // 연산 id 는 그 명령을 받은 기록의 sequence 이다 (plan 19 Q3-6).
    if let Some(record) = recorded.iter().find(|record| {
        matches!(&record.event, KernelEvent::CommandReceived { op_id, .. } if *op_id != record.sequence)
    }) {
        return Err(format!(
            "I6: the command record at sequence {} names another operation id",
            record.sequence
        ));
    }
    check_rejections_are_in_the_spec(&records)?;
    for (index, record) in records.iter().enumerate() {
        // 제출 번호는 커널마다 1 부터 다시 센다. 다시 켠 커널의 번호는 이전 커널의 번호와 겹칠 수 있다 (Q3-7).
        if lifetime_starts.contains(&index) {
            submissions.clear();
        }
        match record {
            KernelEvent::TurnSubmitted { submission, .. } if !submissions.insert(*submission) => {
                return Err(format!(
                    "I6: turn submission {submission} was recorded twice"
                ));
            }
            KernelEvent::TurnStarted {
                thread_id, turn_id, ..
            } if !started.insert(turn_key(thread_id, turn_id)) => {
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

/*
 * I10: 거절의 완전성 (plan 18 K4-3, 설계 §3.9). 커널이 명령에 낸 모든 이유 코드는 그 명령의 스펙 `rejections` 에 있다.
 * 클라이언트는 스펙에 없는 이유를 받지 않는다. 명령의 이름은 같은 연산 id 의 받은 기록에서 읽는다.
 */
fn check_rejections_are_in_the_spec(records: &[KernelEvent]) -> Result<(), String> {
    use crate::domain::kernel_spec::KernelCommandKind;
    let mut commands = BTreeMap::new();
    for record in records {
        match record {
            KernelEvent::CommandReceived { op_id, command, .. } => {
                commands.insert(*op_id, command.as_str());
            }
            KernelEvent::CommandRejected { op_id, reason, .. } => {
                let Some(name) = commands.get(op_id).copied() else {
                    return Err(format!(
                        "I10: operation {op_id} was rejected without a received record"
                    ));
                };
                let listed = KernelCommandKind::ALL
                    .iter()
                    .find(|kind| kind.name() == name)
                    .is_some_and(|kind| kind.spec().rejections.contains(reason));
                if !listed {
                    return Err(format!(
                        "I10: operation {op_id} (`{name}`) was rejected with `{}`, which its spec does not list",
                        reason.name()
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/*
 * I11: 연산의 끝 (plan 18 K4-4, 설계 §3.9). 기록된 중지 명령도 같다 (K4-4b). 시나리오가 끝나고 커널이 일을 마치면, 클라이언트가 보낸 모든 thread 지정
 * 프롬프트의 연산은 끝난 상태이다: `done`, `rejected`, `failed`, `lost` 중 하나이다. 상태는 기록에서만 계산한다.
 * 열린 채로 남은 연산이 있으면 클라이언트는 그 명령이 끝났는지 알 수 없다.
 */
fn check_operations_ended(
    recorded: &[RecordedKernelEvent],
    operations: &[(i64, &str)],
) -> Result<(), String> {
    use crate::domain::kernel_event::operation_states;
    use crate::domain::kernel_spec::CommandCompletion;
    let states = operation_states(recorded.iter().map(|record| &record.event));
    // 보낸 프롬프트와, 끝을 기록으로 알리는 다른 명령(중지)이다 (plan 18 K4-4b).
    let other_commands = recorded.iter().filter_map(|record| match &record.event {
        KernelEvent::CommandReceived {
            op_id,
            command,
            completion: Some(CommandCompletion::Records),
            ..
        } => Some((*op_id, command.as_str())),
        _ => None,
    });
    for (op_id, prompt) in operations.iter().copied().chain(other_commands) {
        let op_id = &op_id;
        match states.get(op_id) {
            Some(state) if state.is_end() => {}
            other => {
                let own = recorded
                    .iter()
                    .filter(|record| record.event.op_id() == Some(*op_id))
                    .map(|record| record.event.kind())
                    .collect::<Vec<_>>();
                return Err(format!(
                    "I11: operation {op_id} ('{prompt}') did not end: state {other:?}, its records {own:?}"
                ));
            }
        }
    }
    Ok(())
}

/*
 * I11 의 확장이다 (#2805). 승인 결정, steer, planning 초기화의 연산은 끝날 뿐 아니라 그 명령의 효과가 받은 결과로
 * 끝난다: 바로 거절이면 `rejected`, 효과가 성공하면 `done`, 실패하면 `failed`, 커널이 완료를 처리하기 전에 다시
 * 켜졌으면 `lost` 이다. 늦은 완료가 다른 명령(다른 턴의 steer, 다른 승인의 결정)의 끝으로 적히면 여기서 걸린다.
 */
fn check_client_operation_ends(
    recorded: &[RecordedKernelEvent],
    operations: &[ClientOperation],
) -> Result<(), String> {
    let states =
        crate::domain::kernel_event::operation_states(recorded.iter().map(|record| &record.event));
    for operation in operations.iter().filter(|operation| !operation.forgotten) {
        let recorded_state = states.get(&operation.op_id).copied();
        if operation.expected.is_none() || recorded_state != operation.expected {
            return Err(format!(
                "I11: operation {} ({}) should end {:?}, but the record says {recorded_state:?}",
                operation.op_id, operation.label, operation.expected
            ));
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
    /// 마지막 주기 뒤에 입력이 커널을 깨웠다 (클라이언트 입력함 또는 core 입력함). 실제 호스트의 깨우기와 같다.
    host_woken: bool,
    started_at: Instant,
    elapsed: Duration,
    step: usize,
    trace: Vec<String>,
    // 가짜 app-server 와 가짜 평가기의 상태이다. 도는 턴은 받은 차례이다.
    turns: Vec<FakeTurn>,
    pending: Vec<PendingCompletion>,
    invalidated_stop_requests: BTreeSet<u64>,
    /// 가짜 클라이언트(화면)가 보는 대화이다. 수동 턴은 TUI 처럼 이 대화에만 낸다. 새 대화를 열면 `None` 이다.
    viewed_thread: Option<String>,
    next_thread: u64,
    next_turn: u64,
    manual_turns_sent: usize,
    thread_prompts_sent: usize,
    /// 보낸 thread 지정 프롬프트의 `(thread, 프롬프트, 처음 받은 연산 id)` 이다. 재전송은 여기서 고른다.
    sent_thread_prompts: Vec<(String, String, i64)>,
    prompt_resends: usize,
    /*
     * 강제 종료가 받은 기록(`command_received`)을 잃은 thread 지정 프롬프트이다 (#2806 계약 C1). 새 커널은 그 키를
     * 모르므로 재전송을 다시 실행한다. 그 연산 id 는 새 커널이 다른 명령에 다시 줄 수 있어 결과 불명이다 (I11 에서 뺀다).
     */
    lost_command_prompts: BTreeSet<String>,
    next_task: u64,
    next_approval: u64,
    restarts: usize,
    /// 다시 켠 커널이 쓰기 시작한 기록 위치이다. I6 이 커널마다 다시 세는 값을 여기서 끊는다.
    lifetime_starts: Vec<usize>,
    /// 마지막으로 커널을 다시 켤 때 큐에 남아 있던 프롬프트이다. 다시 나가면 기록에 적는다.
    carried_prompts: BTreeSet<String>,
    /// 클라이언트가 보낸 thread 지정 프롬프트와 그 thread 이다. 준비가 끝난 프롬프트가 닿을 곳이다.
    thread_prompt_targets: BTreeMap<String, String>,
    /// 턴이 도는 동안 준비 완료를 넣었다. 다음 주기에서 커널이 그 프롬프트를 큐에 남긴다.
    prompt_prepared_behind_turn: bool,
    /// 턴이 도는 동안 준비가 끝난 프롬프트가 있다. 그 프롬프트는 큐에 남았을 가능성이 크다.
    prompt_probably_queued: bool,
    ledger: InvariantLedger,
    kernel_log: Arc<SimKernelLog>,
    /// 클라이언트가 보낸 승인 결정, steer, planning 초기화이다 (#2805). I11 이 그 끝의 종류를 본다.
    client_ops: Vec<ClientOperation>,
    /// 받아들인 승인 결정의 `client_ops` 자리이다. 커널은 받아들인 차례대로 결정 효과를 낸다.
    accepted_decisions: VecDeque<usize>,
    /// 받아들인 planning 초기화의 `client_ops` 자리이다.
    accepted_resets: VecDeque<usize>,
    /// 가짜 app-server 가 끝낸 턴이다. 늦은 steer 가 가리킨다.
    ended_turns: Vec<(String, String)>,
    /// 가짜 바깥 세계가 본 위반이다. 다음 커널 주기가 돌려준다.
    world_violation: Option<String>,
}

impl Simulation {
    fn new(seed: u64, turn_concurrency: usize) -> Self {
        let mut rng = SimRng::new(seed);
        let mut settings = ScenarioSettings::from_rng(&mut rng);
        settings.turn_concurrency = turn_concurrency;
        let mut simulation = Self::with_settings(rng, settings);
        simulation.record(format!("seed {seed}, {settings:?}"));
        simulation
    }

    /// 고정 재생은 난수를 거치지 않고 명시한 설정으로 같은 하네스를 조립한다.
    fn with_settings(rng: SimRng, settings: ScenarioSettings) -> Self {
        let SimKernel {
            mut host,
            inbox,
            effects,
            core_inputs,
        } = build_sim_kernel_with(settings.turn_concurrency);
        let kernel_log = Arc::new(SimKernelLog::default());
        host.runtime
            .attach_kernel_event_log(kernel_log.clone(), "/sim");
        Self {
            rng,
            settings,
            host,
            inbox,
            client: None,
            effects,
            core_inputs,
            queued_core_inputs: 0,
            host_woken: false,
            started_at: Instant::now() + FAKE_CLOCK_OFFSET,
            elapsed: Duration::ZERO,
            step: 0,
            trace: Vec::new(),
            turns: Vec::new(),
            pending: Vec::new(),
            invalidated_stop_requests: BTreeSet::new(),
            viewed_thread: None,
            next_thread: 1,
            kernel_log,
            next_turn: 1,
            manual_turns_sent: 0,
            thread_prompts_sent: 0,
            sent_thread_prompts: Vec::new(),
            prompt_resends: 0,
            lost_command_prompts: BTreeSet::new(),
            next_task: 1,
            next_approval: 1,
            restarts: 0,
            lifetime_starts: Vec::new(),
            carried_prompts: BTreeSet::new(),
            thread_prompt_targets: BTreeMap::new(),
            prompt_prepared_behind_turn: false,
            prompt_probably_queued: false,
            ledger: InvariantLedger::default(),
            client_ops: Vec::new(),
            accepted_decisions: VecDeque::new(),
            accepted_resets: VecDeque::new(),
            ended_turns: Vec::new(),
            world_violation: None,
        }
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
        self.finish(result)
    }

    /// 커널을 버리고 기록을 확인한다 (I6, I7, I8, I10, I11). 성공하면 기록 전체를 돌려준다.
    fn finish(
        self,
        result: Result<(), Violation>,
    ) -> Result<Vec<String>, (Violation, Vec<String>)> {
        let result = result.and_then(|()| {
            let step = self.step;
            // 커널을 버려야 기록기가 남은 사건을 다 쓴다.
            drop(self.host);
            let records = self.kernel_log.0.lock().expect("kernel log should lock");
            let operations = self
                .sent_thread_prompts
                .iter()
                .filter(|(_, prompt, _)| !self.lost_command_prompts.contains(prompt))
                .map(|(_, prompt, op_id)| (*op_id, prompt.as_str()))
                .collect::<Vec<_>>();
            check_kernel_records(&records, &self.lifetime_starts)
                .and_then(|()| self.ledger.check_queued_prompt_records(&records))
                .and_then(|()| self.ledger.check_approval_records(&records))
                .and_then(|()| check_operations_ended(&records, &operations))
                .and_then(|()| check_client_operation_ends(&records, &self.client_ops))
                .map_err(|message| Violation { step, message })
        });
        match result {
            Ok(()) => Ok(self.trace),
            Err(violation) => Err((violation, self.trace)),
        }
    }

    fn run_scenario(&mut self) -> Result<(), Violation> {
        self.open_client_session();
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
        let mut choices = vec![(SimAction::RunHostCycle, 24), (SimAction::ToggleClient, 4)];
        if self.manual_turns_sent < self.settings.manual_turn_budget {
            // 준비 중인 thread 지정 프롬프트가 있으면 턴을 먼저 시작해 그 프롬프트가 큐에 남는 경우를 자주 만든다.
            let preparing = self
                .pending
                .iter()
                .any(|pending| matches!(pending, PendingCompletion::PromptPreparation { .. }));
            let weight = if preparing && self.turns.is_empty() {
                24
            } else {
                4
            };
            choices.push((SimAction::SubmitManualTurn, weight));
        }
        if self.thread_prompts_sent < self.settings.thread_prompt_budget {
            choices.push((SimAction::SubmitThreadPrompt, 4));
        }
        if !self.sent_thread_prompts.is_empty() && self.prompt_resends < MAX_PROMPT_RESENDS {
            choices.push((SimAction::ResendThreadPrompt, 3));
        }
        if self.settings.restarts && self.restarts < MAX_RESTARTS {
            // 큐에 프롬프트가 남았을 때 다시 켜는 경우가 이 시험의 목적이다. 그때 더 자주 고른다.
            let queued = self.prompt_probably_queued && !self.turns.is_empty();
            let weight = if queued { 48 } else { 1 };
            choices.push((SimAction::RestartKernel, weight));
        }
        if !self.turns.is_empty() {
            choices.push((SimAction::AdvanceTurn, 12));
        }
        if !self.turns.is_empty() && self.settings.stop_requests {
            choices.push((SimAction::RequestStop, 4));
        }
        if self.decisions_sent() < self.settings.decision_budget
            && !self.ledger.approvals.is_empty()
        {
            let waiting = self.turns.iter().any(|turn| turn.approval.is_some());
            choices.push((SimAction::DecideApproval, if waiting { 12 } else { 2 }));
        }
        if self.steers_sent() < self.settings.steer_budget
            && (!self.turns.is_empty() || !self.ended_turns.is_empty())
        {
            choices.push((SimAction::SteerTurn, 4));
        }
        if !self.pending.is_empty() {
            choices.push((SimAction::DeliverCompletion, 12));
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
            SimAction::RestartKernel => {
                if !self.rng.percent(50) {
                    return self.restart_kernel();
                }
                // 잃는 위치는 기록기가 다 쓴 뒤에 정한다. 그 전의 길이는 기록기 스레드의 속도를 따라 seed 를 따르지 않는다.
                let pick = self.rng.next_u64();
                let drop_effects = self.rng.percent(50);
                return self.crash_kernel(
                    &|written: &[RecordedKernelEvent]| (pick % (written.len() as u64 + 1)) as usize,
                    drop_effects,
                );
            }
            SimAction::SubmitManualTurn => self.submit_manual_turn(),
            SimAction::SubmitThreadPrompt => self.submit_thread_prompt(),
            SimAction::ResendThreadPrompt => return self.resend_thread_prompt(),
            SimAction::RequestStop => {
                self.record("client: stop all sessions".to_string());
                self.send_recorded_stop();
            }
            SimAction::DecideApproval => self.decide_approval(),
            SimAction::SteerTurn => self.steer_turn(),
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
            if !self.turns.is_empty() {
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
            "I4: the kernel did not settle: remaining work {}, fake turns {:?}, pending completions {}, busy threads {:?}, viewed runtime {:?}",
            self.host.runtime.has_remaining_work(),
            self.turns,
            self.pending.len(),
            self.host.thread_activity,
            self.host.runtime.snapshot().conversation_runtime
        )))
    }

    fn is_quiet(&self) -> bool {
        self.turns.is_empty() && self.pending.is_empty() && !self.host.runtime.has_remaining_work()
    }

    /// 클라이언트가 커널에 붙어 하는 처음 일이다: 자동 후속 한도를 정하고 세션 목록을 읽는다.
    fn open_client_session(&mut self) {
        self.attach_client();
        self.send_command(AppCommand::SetAutoFollowMaxTurns {
            value: self.settings.max_auto_turns,
        });
        self.send_command(AppCommand::LoadSessionCatalog(
            SessionCatalogLoadIntent::ensure_loaded(10, WORKSPACE),
        ));
    }

    /*
     * 커널을 버리고 같은 기록으로 다시 조립한다 (plan 19 Q3-7). 이전 커널의 worker 는 커널과 함께 사라지므로,
     * 진행 중이던 턴과 완료는 바깥 세계에서도 버린다. 지금 커널은 그 일들을 기록에 남기지 않는다 (P1 의 남은 부분).
     * 큐에 남긴 thread 지정 프롬프트는 기록에 있으므로 새 커널이 되돌린다 (Q3-1).
     */
    fn restart_kernel(&mut self) -> Result<(), Violation> {
        // 이전 커널이 마지막으로 낸 효과를 먼저 받는다. 그 효과는 app-server 에 닿은 것으로 센다.
        self.accept_effects()?;
        self.replace_kernel(None)
    }

    /*
     * 기록을 다 쓰지 못한 강제 종료 뒤의 재시작이다 (#2806). 기록 확정과 바깥 효과의 완료를 따로 정한다. `keep` 은 죽은
     * 커널이 쓴 기록 가운데 디스크에 닿은 개수를 고른다. `drop_effects` 이면 그 커널이 마지막으로 낸 효과는 바깥 세계에
     * 닿지 않았다 (기록은 남았을 수 있다). 아니면 효과는 닿았다 (기록은 잃었을 수 있다).
     */
    fn crash_kernel(
        &mut self,
        keep: &KeepWritten<'_>,
        drop_effects: bool,
    ) -> Result<(), Violation> {
        let mut dropped_prompts = BTreeSet::new();
        if drop_effects {
            for effect in self.effects.take() {
                if let CoreEffect::SubmitTurn { request, .. } = &effect {
                    dropped_prompts.insert(request.prompt.clone());
                }
                self.record(format!(
                    "crash: the effect {} did not leave the kernel",
                    variant_name(&format!("{effect:?}"))
                ));
            }
        } else {
            self.accept_effects()?;
        }
        self.replace_kernel(Some((keep, dropped_prompts)))
    }

    /*
     * 커널을 버리고 같은 기록으로 다시 조립한다. `crash` 가 있으면 죽은 커널이 쓴 기록의 끝을 잃는다. 그 기록이 없으므로
     * 새 커널이 할 수 있는 일을 계약(`docs/reference/kernel-crash-recovery.md`)대로 기대값에 반영한다.
     */
    fn replace_kernel(
        &mut self,
        crash: Option<(&KeepWritten<'_>, BTreeSet<String>)>,
    ) -> Result<(), Violation> {
        self.restarts += 1;
        let SimKernel {
            host,
            inbox,
            effects,
            core_inputs,
        } = build_sim_kernel_with(self.settings.turn_concurrency);
        // 이전 커널을 버려야 기록기가 남은 사건을 다 쓴다. 새 커널은 그 뒤에 기록을 읽는다.
        drop(std::mem::replace(&mut self.host, host));
        let crashed = crash.is_some();
        if let Some((keep, dropped_prompts)) = crash {
            self.lose_record_tail(keep, &dropped_prompts);
        }
        self.expect_client_ops_after_restart(crashed);
        self.accepted_decisions.clear();
        self.accepted_resets.clear();
        self.inbox = inbox;
        self.effects = effects;
        self.core_inputs = core_inputs;
        self.client = None;
        self.viewed_thread = None;
        self.turns.clear();
        self.pending.clear();
        self.invalidated_stop_requests.clear();
        self.queued_core_inputs = 0;
        self.prompt_probably_queued = false;
        self.prompt_prepared_behind_turn = false;
        self.ledger.auto_turns_since_manual.clear();
        self.ledger
            .expect_approvals_after_restart(self.restarts - 1);
        self.next_approval = 1;
        let (lifetime_start, carried) = {
            let records = self.kernel_log.0.lock().expect("kernel log should lock");
            let events = records.iter().map(|record| record.event.clone());
            (records.len(), open_queued_thread_prompts(events))
        };
        self.lifetime_starts.push(lifetime_start);
        self.record(format!(
            "kernel restarted with {} queued thread prompt(s)",
            carried.len()
        ));
        self.carried_prompts = carried
            .into_iter()
            .map(|prompt| prompt.handoff.prompt)
            .collect();
        self.host
            .runtime
            .attach_kernel_event_log(self.kernel_log.clone(), "/sim");
        // 되돌린 프롬프트는 기록을 붙이는 dispatch 안에서 바로 다시 나갈 수 있다.
        self.accept_effects()?;
        self.open_client_session();
        Ok(())
    }

    /// 죽은 커널이 쓴 기록의 끝을 잃고, 그 기록이 없어서 달라지는 기대값을 고친다 (#2806 계약 C1~C4).
    fn lose_record_tail(&mut self, keep: &KeepWritten<'_>, dropped_prompts: &BTreeSet<String>) {
        let lifetime_start = self.lifetime_starts.last().copied().unwrap_or(0);
        let kept = {
            let records = self.kernel_log.0.lock().expect("kernel log should lock");
            let written = &records[lifetime_start..];
            lifetime_start + keep(written).min(written.len())
        };
        let lost = self.kernel_log.lose_tail(kept);
        self.record(format!(
            "kernel crashed: {} record(s) did not reach the disk, {} turn submission(s) dropped",
            lost.len(),
            dropped_prompts.len()
        ));
        {
            let records = self.kernel_log.0.lock().expect("kernel log should lock");
            self.ledger
                .forget_lost_approval_records(self.restarts - 1, &records[lifetime_start..]);
        }
        for prompt in dropped_prompts {
            if let Some(planned) = self.ledger.planned_prompts.get_mut(prompt) {
                planned.submission_may_be_lost = true;
            }
        }
        for record in &lost {
            match &record.event {
                KernelEvent::CommandReceived { op_id, .. } => {
                    for (_, prompt, _) in self
                        .sent_thread_prompts
                        .iter()
                        .filter(|(_, _, sent)| sent == op_id)
                    {
                        self.lost_command_prompts.insert(prompt.clone());
                    }
                }
                KernelEvent::ThreadPromptSettled {
                    task_id,
                    settlement: ThreadPromptSettlement::Submitted,
                    ..
                } => {
                    let planned =
                        self.ledger
                            .planned_prompts
                            .iter_mut()
                            .find(|(prompt, planned)| {
                                planned.task_id == *task_id && !dropped_prompts.contains(*prompt)
                            });
                    if let Some((_, planned)) = planned {
                        planned.resubmissions_allowed += 1;
                    }
                }
                _ => {}
            }
        }
    }

    /*
     * 커널이 다시 켜졌다. 그 커널이 받은 명령 가운데 끝을 처리하지 못한 것은 새 커널이 `lost` 로 닫는다. 강제 종료가
     * 기록을 잃었으면 (#2806 계약 C1, C3): 받은 기록을 잃은 명령은 잊고, 끝 기록을 잃은 명령은 `lost` 로 닫힌다.
     */
    fn expect_client_ops_after_restart(&mut self, crashed: bool) {
        let ended_lifetime = self.restarts - 1;
        let lifetime_start = self.lifetime_starts.last().copied().unwrap_or(0);
        let (received, ended) = {
            let records = self.kernel_log.0.lock().expect("kernel log should lock");
            let kept = &records[lifetime_start.min(records.len())..];
            let received = kept
                .iter()
                .filter(|record| matches!(record.event, KernelEvent::CommandReceived { .. }))
                .filter_map(|record| record.event.op_id())
                .collect::<BTreeSet<_>>();
            let ended = crate::domain::kernel_event::operation_states(
                records.iter().map(|record| &record.event),
            )
            .into_iter()
            .filter(|(_, state)| state.is_end())
            .map(|(op_id, _)| op_id)
            .collect::<BTreeSet<_>>();
            (received, ended)
        };
        for operation in &mut self.client_ops {
            if operation.lifetime != ended_lifetime || operation.forgotten {
                continue;
            }
            if crashed && !received.contains(&operation.op_id) {
                operation.forgotten = true;
            } else if !operation.end_processed || (crashed && !ended.contains(&operation.op_id)) {
                operation.expected = Some(OperationState::Lost);
                operation.end_processed = true;
            }
        }
    }

    fn decisions_sent(&self) -> usize {
        self.client_ops
            .iter()
            .filter(|operation| operation.label.starts_with("decide"))
            .count()
    }

    fn steers_sent(&self) -> usize {
        self.client_ops
            .iter()
            .filter(|operation| operation.steer_turn.is_some())
            .count()
    }

    /*
     * 끝을 기록으로 알리는 명령을 Gateway 처럼 설명과 함께 보내고 영수증을 받는다. 바로 거절된 명령은 그 자리에서
     * 끝난다. 받아들인 명령은 그 효과의 완료가 끝을 정한다. 돌려주는 값은 받아들였을 때의 `client_ops` 자리이다.
     */
    fn send_client_operation(
        &mut self,
        command: AppCommand,
        kind: KernelCommandKind,
        thread_id: &str,
        label: String,
        steer_turn: Option<String>,
    ) -> Option<usize> {
        let (reply, answer) = mpsc::channel();
        self.send_host_input(KernelHostInput::Command {
            command: crate::composition::native_client_runtime::KernelClientCommand::Core(command),
            descriptor: Some(KernelCommandDescriptor {
                command: kind,
                thread_id: thread_id.to_string(),
                fingerprint: label.clone(),
                idempotency_key: None,
                caller: None,
                confirmed: kind == KernelCommandKind::ResetPlanning,
            }),
            reply,
        });
        let outcome = answer
            .try_recv()
            .expect("the host answers a command when it handles its inputs");
        let KernelCommandOutcome::Handled {
            operation: Some(receipt),
            ..
        } = outcome
        else {
            panic!("the command '{label}' did not run with a receipt: {outcome:?}");
        };
        let rejected = receipt.rejection.is_some();
        if let Some(reason) = receipt.rejection {
            self.record(format!(
                "{REJECTED_AT_ONCE} '{label}' with {}",
                reason.name()
            ));
        }
        self.client_ops.push(ClientOperation {
            op_id: receipt.op_id,
            label,
            lifetime: self.restarts,
            steer_turn,
            expected: rejected.then_some(OperationState::Rejected),
            end_processed: rejected,
            forgotten: false,
        });
        (!rejected).then(|| self.client_ops.len() - 1)
    }

    /// 승인 결정을 보낸다. 지금 턴이 기다리는 승인을 주로 고르고, 가끔 이미 끝난 승인을 고른다 (늦은 결정).
    fn decide_approval(&mut self) {
        let waiting = self
            .turns
            .iter()
            .find_map(|turn| turn.approval.clone().map(|id| (turn.thread_id.clone(), id)));
        let (thread_id, approval_id) = match waiting {
            Some(waiting) if self.rng.percent(80) => waiting,
            _ => {
                let index = self.rng.index(self.ledger.approvals.len());
                let approval = &self.ledger.approvals[index];
                (approval.thread_id.clone(), approval.approval_id.clone())
            }
        };
        let decision = if self.rng.percent(50) {
            ConversationApprovalDecision::Accept
        } else {
            ConversationApprovalDecision::Decline
        };
        self.send_decision(&thread_id, approval_id, decision);
    }

    fn send_decision(
        &mut self,
        thread_id: &str,
        approval_id: String,
        decision: ConversationApprovalDecision,
    ) -> Option<usize> {
        let label = format!("decide {approval_id} {decision:?}");
        self.record(format!("client: {label} on {thread_id}"));
        let command = AppCommand::SubmitApprovalDecision {
            request_identity: ConversationApprovalRequestIdentity {
                server_request_id: format!("server-{approval_id}"),
                approval_id,
            },
            decision,
        };
        let op = self.send_client_operation(
            command,
            KernelCommandKind::DecideApproval,
            thread_id,
            label,
            None,
        )?;
        self.accepted_decisions.push_back(op);
        Some(op)
    }

    /// steer 를 보낸다. 지금 도는 턴을 주로 고르고, 가끔 이미 끝난 턴을 고른다 (늦은 steer).
    fn steer_turn(&mut self) {
        let running = self
            .turns
            .first()
            .map(|turn| (turn.thread_id.clone(), turn.turn_id.clone()));
        let (thread_id, turn_id) = match running {
            Some(running) if self.ended_turns.is_empty() || self.rng.percent(70) => running,
            _ => self.ended_turns[self.rng.index(self.ended_turns.len())].clone(),
        };
        self.send_steer(&thread_id, &turn_id);
    }

    fn send_steer(&mut self, thread_id: &str, turn_id: &str) -> Option<usize> {
        let prompt = format!("steer {}", self.steers_sent() + 1);
        self.record(format!("client: {prompt} for {turn_id} on {thread_id}"));
        let command = AppCommand::SteerTurn(ConversationTurnSteerRequest {
            thread_id: thread_id.to_string(),
            expected_turn_id: turn_id.to_string(),
            prompt: prompt.clone(),
        });
        self.send_client_operation(
            command,
            KernelCommandKind::SteerTurn,
            thread_id,
            prompt,
            Some(turn_id.to_string()),
        )
    }

    /// planning 초기화를 확인과 함께 보낸다 (파괴 명령, plan 18 K4-5).
    fn reset_planning(&mut self) {
        let label = format!("reset planning {}", self.client_ops.len() + 1);
        self.record(format!("client: {label}"));
        let command = AppCommand::ResetPlanningWorkspace(PlanningWorkspaceResetIntent::new(
            WORKSPACE,
            PlanningWorkspaceResetTarget::Queue,
        ));
        if let Some(op) =
            self.send_client_operation(command, KernelCommandKind::ResetPlanning, "", label, None)
        {
            self.accepted_resets.push_back(op);
        }
    }

    /*
     * 명령의 효과가 끝났다는 완료를 커널에 넣는다. 기대하는 끝은 이 입력으로 정해진다. 반은 바로 넣고, 반은 늦게 넣는다:
     * 그 사이 턴과 승인이 바뀐다. 커널은 늦은 완료를 그 명령의 끝으로만 적어야 한다.
     */
    fn complete_client_operation(&mut self, op: usize, input: CoreInput, succeeded: bool) {
        if self.rng.percent(50) {
            self.record(format!(
                "worker: the completion of '{}' is late",
                self.client_ops[op].label
            ));
            self.pending.push(PendingCompletion::LateCompletion {
                input,
                op,
                succeeded,
            });
        } else {
            self.send_client_completion(op, input, succeeded);
        }
    }

    /// 기대하는 끝은 완료를 커널에 넣을 때 정한다. 커널이 처리하기 전에 다시 켜지면 `lost` 로 바뀐다.
    fn send_client_completion(&mut self, op: usize, input: CoreInput, succeeded: bool) {
        let operation = &mut self.client_ops[op];
        operation.expected = Some(if succeeded {
            OperationState::Done
        } else {
            OperationState::Failed
        });
        operation.end_processed = false;
        self.send_core_input(input);
    }

    /// app-server 가 승인 결정을 받는다. 그 승인을 기다리는 턴이 있으면 결정을 적는다. 두 번째 결정은 위반이다 (I8).
    fn deliver_approval_decision(&mut self, correlation: ApprovalDecisionCorrelation, op: usize) {
        let delivered = self.rng.percent(80);
        let approval_id = correlation.request_identity.approval_id.clone();
        if delivered
            && let Some(turn) = self
                .turns
                .iter_mut()
                .find(|turn| turn.approval.as_deref() == Some(approval_id.as_str()))
        {
            if turn.decision.is_some() {
                self.world_violation = Some(format!(
                    "I8: {approval_id} reached the app-server with a second decision"
                ));
            }
            turn.decision = Some(correlation.decision);
        }
        let result = if delivered {
            Ok(())
        } else {
            Err("simulated decision delivery failure".to_string())
        };
        self.record(format!(
            "app-server: decision {:?} for {approval_id} -> {result:?}",
            correlation.decision
        ));
        let input = CoreInput::EffectCompleted(CoreEffectCompletion::ApprovalDecisionSubmitted {
            correlation,
            result,
        });
        self.complete_client_operation(op, input, delivered);
    }

    /// app-server 가 steer 를 받는다. 그 턴이 지금 돌 때만 받아들인다 (`expectedTurnId`). 다른 턴에는 적용하지 않는다.
    fn deliver_steer(
        &mut self,
        correlation: TurnSteerCorrelation,
        request: ConversationTurnSteerRequest,
        op: usize,
    ) {
        let applies = self.turns.iter().any(|turn| {
            turn.stage == TurnStage::Started
                && turn.thread_id == request.thread_id
                && turn.turn_id == request.expected_turn_id
        }) && !self.rng.percent(10);
        let result = if applies {
            Ok(ConversationTurnSteerReceipt {
                turn_id: request.expected_turn_id.clone(),
            })
        } else {
            Err("expected turn is not running".to_string())
        };
        self.record(format!(
            "app-server: {} for {} -> {result:?}",
            request.prompt, request.expected_turn_id
        ));
        let input = CoreInput::EffectCompleted(CoreEffectCompletion::TurnSteered {
            correlation,
            result,
        });
        self.complete_client_operation(op, input, applies);
    }

    fn deliver_planning_reset(
        &mut self,
        correlation: PlanningWorkspaceOperationCorrelation,
        op: Option<usize>,
    ) {
        let target = correlation
            .reset_target()
            .expect("the simulation resets only through the reset operation");
        let input =
            CoreInput::EffectCompleted(CoreEffectCompletion::PlanningWorkspaceResetCompleted {
                correlation,
                result: Ok(Box::new(PlanningWorkspaceResetSnapshot {
                    target,
                    rewritten_paths: Vec::new(),
                    removed_paths: Vec::new(),
                })),
            });
        match op {
            Some(op) => self.complete_client_operation(op, input, true),
            None => self.send_core_input(input),
        }
    }

    fn submit_thread_prompt(&mut self) {
        self.thread_prompts_sent += 1;
        let thread_id = CATALOG_THREADS[self.rng.index(CATALOG_THREADS.len())].to_string();
        let prompt = format!("request {}", self.thread_prompts_sent);
        self.record(format!("client: thread prompt '{prompt}' to {thread_id}"));
        self.thread_prompt_targets
            .insert(prompt.clone(), thread_id.clone());
        // 처음 보낸 명령은 실행되고 영수증을 받는다. 그 연산 id 를 재전송의 답과 견준다 (I9).
        let KernelCommandOutcome::Handled {
            operation: Some(receipt),
            ..
        } = self.send_thread_prompt(&thread_id, &prompt)
        else {
            panic!("I9: the first send of '{prompt}' did not run with a receipt");
        };
        if let Some(reason) = receipt.rejection {
            self.record(format!(
                "{REJECTED_AT_ONCE} '{prompt}' with {}",
                reason.name()
            ));
        }
        self.sent_thread_prompts
            .push((thread_id, prompt, receipt.op_id));
    }

    /// thread 지정 프롬프트를 멱등 키와 함께 보낸다 (plan 19 Q3-6). 키는 프롬프트마다 하나이다.
    fn send_thread_prompt(&mut self, thread_id: &str, prompt: &str) -> KernelCommandOutcome {
        let (reply, answer) = mpsc::channel();
        self.send_host_input(KernelHostInput::Command {
            command: crate::composition::native_client_runtime::KernelClientCommand::Core(
                AppCommand::SubmitThreadPrompt(Box::new(ThreadPromptSubmission {
                    workspace_directory: WORKSPACE.to_string(),
                    thread_id: thread_id.to_string(),
                    prompt: prompt.to_string(),
                    op_id: None,
                })),
            ),
            descriptor: Some(KernelCommandDescriptor {
                command: crate::domain::kernel_spec::KernelCommandKind::SubmitPrompt,
                thread_id: thread_id.to_string(),
                fingerprint: format!("{thread_id}:{prompt}"),
                idempotency_key: Some(format!("key of {prompt}")),
                caller: None,
                confirmed: false,
            }),
            reply,
        });
        answer
            .try_recv()
            .expect("the host answers a command when it handles its inputs")
    }

    /*
     * I9: 같은 멱등 키로 다시 보낸 명령은 다시 실행되지 않는다 (plan 19 Q3-6). 커널을 다시 켠 뒤에도 같다. 커널은
     * 처음의 영수증을 답한다. 다시 실행하면 planner 가 같은 프롬프트를 두 번 준비하고 작업이 둘이 된다.
     *
     * 예외는 계약 C1 이다: 강제 종료가 받은 기록을 잃었으면 새 커널은 그 키를 모르고 명령을 새 연산으로 다시 실행한다.
     */
    fn resend_thread_prompt(&mut self) -> Result<(), Violation> {
        self.prompt_resends += 1;
        let index = self.rng.index(self.sent_thread_prompts.len());
        let (thread_id, prompt, first_op_id) = self.sent_thread_prompts[index].clone();
        if self.lost_command_prompts.remove(&prompt) {
            self.ledger.prepared_prompts.remove(&prompt);
            let KernelCommandOutcome::Handled {
                operation: Some(receipt),
                ..
            } = self.send_thread_prompt(&thread_id, &prompt)
            else {
                return Err(self.violation(format!(
                    "C1: the resend of '{prompt}' after its record was lost did not run again"
                )));
            };
            self.record(format!(
                "client: resend of '{prompt}' ran again as operation {} after a crash lost operation {first_op_id}",
                receipt.op_id
            ));
            if let Some(reason) = receipt.rejection {
                self.record(format!(
                    "{REJECTED_AT_ONCE} '{prompt}' with {}",
                    reason.name()
                ));
            }
            self.sent_thread_prompts[index].2 = receipt.op_id;
            return Ok(());
        }
        let outcome = self.send_thread_prompt(&thread_id, &prompt);
        match outcome {
            KernelCommandOutcome::Replayed(receipt) if receipt.op_id == first_op_id => {
                self.record(format!(
                    "client: resend of '{prompt}' answered with the receipt of operation {}",
                    receipt.op_id
                ));
                Ok(())
            }
            other => Err(self.violation(format!(
                "I9: the resend of '{prompt}' was not answered with the first receipt: {}",
                variant_name(&format!("{other:?}"))
            ))),
        }
    }

    fn accept_effects(&mut self) -> Result<(), Violation> {
        for effect in self.effects.take() {
            self.accept_effect(effect)?;
        }
        Ok(())
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

    /*
     * 중지 명령을 Gateway 처럼 설명과 함께 보낸다 (plan 18 K4-4b). 커널이 그 명령을 기록하고, I11 이 그 연산의 끝을
     * 본다. 프롬프트가 아닌 명령의 끝(효과의 완료, 한 일이 없는 명령, 재시작)을 이 명령이 대표한다.
     */
    fn send_recorded_stop(&mut self) {
        let (reply, _answer) = mpsc::channel();
        self.send_host_input(KernelHostInput::Command {
            command: crate::composition::native_client_runtime::KernelClientCommand::Core(
                AppCommand::RequestStopAllSessions,
            ),
            descriptor: Some(KernelCommandDescriptor {
                command: crate::domain::kernel_spec::KernelCommandKind::StopAll,
                thread_id: String::new(),
                fingerprint: "stop".to_string(),
                idempotency_key: None,
                caller: None,
                confirmed: false,
            }),
            reply,
        });
    }

    /// 클라이언트 명령은 실제 클라이언트처럼 호스트 입력함으로 보낸다. 응답은 이벤트로도 오므로 버린다.
    fn send_command(&mut self, command: AppCommand) {
        let (reply, _answer) = mpsc::channel();
        self.send_host_input(KernelHostInput::Command {
            command: crate::composition::native_client_runtime::KernelClientCommand::Core(command),
            descriptor: None,
            reply,
        });
    }

    fn send_host_input(&mut self, input: KernelHostInput) {
        self.host_woken = true;
        self.inbox
            .send(input)
            .expect("the simulation owns the host loop");
        assert!(
            self.host.handle_pending_inputs(),
            "the simulation never shuts the host down"
        );
    }

    /*
     * 화면(TUI)이 내는 턴이다. 보는 대화를 이어 가거나, 새 대화를 열고 낸다. 보지 않는 thread 에는 내지 않는다.
     * 실제 클라이언트도 그렇다: TUI 는 대화를 먼저 열고, 원격 클라이언트는 이 명령 대신 thread 지정 프롬프트를 쓴다.
     * 보지 않는 thread 에 내면 그 thread 의 session 이 둘이 될 수 있고, 커널은 그 입력을 막지 않는다 (plan 19 Q3-7).
     * 새 대화는 중지가 끝나기 전에도 연다. 그 session 은 뒤에서 턴을 이어 간다.
     */
    fn submit_manual_turn(&mut self) {
        let continues_viewed = self.viewed_thread.is_some() && self.rng.percent(50);
        let prompt = format!("task {}", self.manual_turns_sent + 1);
        self.submit_manual_turn_with(prompt, continues_viewed);
    }

    fn submit_manual_turn_with(&mut self, prompt: String, continues_viewed: bool) {
        self.manual_turns_sent += 1;
        if !continues_viewed && self.viewed_thread.take().is_some() {
            self.record("client: open a new conversation".to_string());
            // TUI 는 새 대화를 열 때 자기 프롬프트 준비를 먼저 취소한다. thread 지정 준비는 취소되면 안 된다.
            self.send_command(AppCommand::CancelManualPromptPreparation);
            self.send_command(AppCommand::InvalidateConversationLoad);
        }
        let thread_id = self.viewed_thread.clone();
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

    /// 커널 한 주기를 돌리고, 그 주기에 나온 효과를 바깥 세계에 넘긴다. 가짜 시계는 `advance` 만큼 간다.
    fn run_host_cycle(&mut self, advance: Duration) -> Result<(), Violation> {
        self.elapsed += advance;
        let now = self.started_at + self.elapsed;
        /*
         * 실제 커널 호스트처럼 돈다 (plan 19 Q3-4): 입력이 깨웠거나 마감 시각이 되었을 때만 주기를 돈다. 30초 안전망은
         * 흉내 내지 않는다. 그래서 깨우지도 않고 마감도 내지 않는 일이 커널에 있으면, 그 일은 끝나지 않고 I4 가 잡는다.
         */
        let deadline_reached = self
            .host
            .runtime
            .next_deadline(now)
            .is_some_and(|deadline| deadline <= now);
        if !self.host_woken && !deadline_reached {
            return Ok(());
        }
        self.host_woken = false;
        assert!(
            self.host.handle_pending_inputs(),
            "the simulation never shuts the host down"
        );
        self.host.run_cycle(now, Duration::ZERO);
        self.queued_core_inputs = 0;
        self.ledger.mark_approval_inputs_delivered(self.restarts);
        for operation in &mut self.client_ops {
            if operation.lifetime == self.restarts && operation.expected.is_some() {
                operation.end_processed = true;
            }
        }
        if let Some(message) = self.world_violation.take() {
            return Err(self.violation(message));
        }
        // 커널이 준비 완료를 처리한 뒤에야 프롬프트가 큐에 있다. 턴이 없으면 커널은 큐를 바로 비운다.
        self.prompt_probably_queued = !self.turns.is_empty()
            && (self.prompt_probably_queued || self.prompt_prepared_behind_turn);
        self.prompt_prepared_behind_turn = false;
        self.drain_client_events();
        self.accept_effects()?;
        self.check_remaining_work_while_busy()
    }

    /*
     * I5: 바깥 세계에서 턴이 돌거나 평가가 진행 중이면 커널은 남은 일이 있다고 답해야 한다. 이 답으로 커널을 내릴지
     * 정하므로(plan 18 K3-5), 일이 있는데 없다고 하면 진행 중인 일을 잃는다. 커널이 무효로 만든 평가는 세지 않는다.
     * 중지처럼 정책이 바뀌면 커널은 평가를 버리고, 그 worker 의 결과는 받지 않는다.
     */
    fn check_remaining_work_while_busy(&self) -> Result<(), Violation> {
        let evaluating = self.pending.iter().any(|pending| {
            matches!(pending, PendingCompletion::PostTurnEvaluation { permit, .. } if permit.is_current())
        });
        // 받아들인 thread 지정 프롬프트의 준비도 남은 일이다. 커널을 내리면 그 프롬프트를 잃는다.
        let preparing = self
            .pending
            .iter()
            .any(|pending| matches!(pending, PendingCompletion::PromptPreparation { .. }));
        if (!self.turns.is_empty() || evaluating || preparing)
            && !self.host.runtime.has_remaining_work()
        {
            return Err(self.violation(format!(
                "I5: the kernel reports no remaining work while turns {:?} run, an evaluation is pending ({evaluating}), or a prompt is being prepared ({preparing})",
                self.turns.iter().map(|turn| &turn.turn_id).collect::<Vec<_>>()
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
            CoreEffect::EvaluatePostTurn {
                correlation,
                request,
            } => self.start_fake_evaluation(correlation, request.continuation_permit),
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
                    target: None,
                });
                Ok(())
            }
            CoreEffect::InterruptTurn {
                correlation,
                thread_id,
                turn_id,
            } => {
                self.record(format!(
                    "app-server: stop request {} for {turn_id} on {thread_id}",
                    correlation.generation
                ));
                self.pending.push(PendingCompletion::StopAttempt {
                    correlation,
                    attempt: StopRequestAttempt::AfterTurnStarted,
                    target: Some((thread_id, turn_id)),
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
            CoreEffect::LoadSessionCatalog { correlation } => {
                self.pending
                    .push(PendingCompletion::SessionCatalog { correlation });
                Ok(())
            }
            CoreEffect::PrepareManualPrompt(request) => {
                self.record(format!("planner: prepare '{}'", request.raw_prompt));
                // I9 의 바깥쪽 확인이다: 프롬프트 하나는 한 번만 준비된다. 재전송이 다시 실행되면 여기서 걸린다.
                if !self
                    .ledger
                    .prepared_prompts
                    .insert(request.raw_prompt.clone())
                {
                    return Err(self.violation(format!(
                        "I9: the planner prepared '{}' twice",
                        request.raw_prompt
                    )));
                }
                self.pending.push(PendingCompletion::PromptPreparation {
                    correlation: request.correlation,
                    raw_prompt: request.raw_prompt,
                });
                Ok(())
            }
            CoreEffect::SubmitApprovalDecision { correlation } => {
                // 결정 효과는 받아들인 결정 명령마다 하나이고, 받아들인 차례대로 나온다.
                let Some(op) = self.accepted_decisions.pop_front() else {
                    return Err(self.violation(format!(
                        "a decision for {} left the kernel without an accepted command",
                        correlation.request_identity.approval_id
                    )));
                };
                self.pending
                    .push(PendingCompletion::ApprovalDecision { correlation, op });
                Ok(())
            }
            CoreEffect::SteerTurn {
                correlation,
                request,
            } => {
                // 커널은 클라이언트가 가리킨 턴 그대로 보낸다. 늦은 steer 를 지금 도는 다른 턴으로 바꾸면 안 된다.
                let op = self.client_ops.iter().position(|operation| {
                    operation.label == request.prompt && operation.lifetime == self.restarts
                });
                let Some(op) = op.filter(|op| {
                    self.client_ops[*op].steer_turn.as_deref()
                        == Some(request.expected_turn_id.as_str())
                }) else {
                    return Err(self.violation(format!(
                        "'{}' left the kernel for {}, which no client steer named",
                        request.prompt, request.expected_turn_id
                    )));
                };
                self.pending.push(PendingCompletion::Steer {
                    correlation,
                    request,
                    op,
                });
                Ok(())
            }
            CoreEffect::ResetPlanningWorkspace { correlation } => {
                let op = self.accepted_resets.pop_front();
                self.pending
                    .push(PendingCompletion::PlanningReset { correlation, op });
                Ok(())
            }
            // 시뮬레이션의 준비는 모두 thread 지정 프롬프트의 것이다. 화면의 취소가 그 준비를 끊으면 안 된다
            // (plan 18 K3-3b 의 회귀).
            CoreEffect::CancelManualPromptPreparation { .. } => Err(self.violation(
                "a client cancel reached the preparation of a thread prompt".to_string(),
            )),
            other => Err(self.violation(format!(
                "the simulation does not model this effect yet: {}",
                variant_name(&format!("{other:?}"))
            ))),
        }
    }

    /// 불변식 I1(턴은 동시 턴 수까지, thread 마다 하나), I3, I12 를 본다.
    fn start_fake_turn(
        &mut self,
        correlation: TurnSubmissionCorrelation,
        request: TurnSubmissionRequest,
    ) -> Result<(), Violation> {
        if self.turns.len() >= self.settings.turn_concurrency {
            return Err(self.violation(format!(
                "I1: a second turn was submitted while {} runs",
                self.turns
                    .iter()
                    .map(|turn| turn.turn_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        if let Some(running) = request
            .thread_id
            .as_ref()
            .and_then(|thread_id| self.turns.iter().find(|turn| &turn.thread_id == thread_id))
        {
            return Err(self.violation(format!(
                "I1: a second turn on {} was submitted while {} runs",
                running.thread_id, running.turn_id
            )));
        }
        self.check_one_auto_follow_thread(&request)?;
        self.ledger
            .record_turn_submission(&request, self.settings.max_auto_turns)
            .and_then(|()| self.ledger.record_planned_prompt_submission(&request))
            .map_err(|message| self.violation(message))?;
        if self.carried_prompts.remove(&request.prompt) {
            self.record(format!(
                "restored prompt '{}' submitted after the restart",
                request.prompt
            ));
        }
        let thread_id = request.thread_id.clone().unwrap_or_else(|| {
            let thread_id = format!("thread-{}", self.next_thread);
            self.next_thread += 1;
            thread_id
        });
        if request.prompt_origin == CorePromptOrigin::Manual {
            self.viewed_thread = Some(thread_id.clone());
        }
        let turn_id = format!("turn-{}", self.next_turn);
        self.next_turn += 1;
        let alongside = if self.turns.is_empty() {
            String::new()
        } else {
            format!(", {ALONGSIDE_ANOTHER_TURN}")
        };
        self.record(format!(
            "app-server: accept {turn_id} on {thread_id} ({:?}, submission {}{alongside})",
            request.prompt_origin, correlation.generation
        ));
        if request.prompt_origin == CorePromptOrigin::AutoFollow {
            self.ledger
                .auto_follow_turns
                .insert(turn_key(&thread_id, &turn_id));
        }
        self.turns.push(FakeTurn {
            correlation,
            thread_id,
            origin: request.prompt_origin,
            turn_id,
            stage: TurnStage::Submitted,
            interrupt_requested: false,
            approval: None,
            decision: None,
        });
        Ok(())
    }

    fn start_fake_evaluation(
        &mut self,
        correlation: PostTurnEvaluationCorrelation,
        permit: PostTurnContinuationPermit,
    ) -> Result<(), Violation> {
        self.ledger
            .record_evaluation_start(&correlation)
            .map_err(|message| self.violation(message))?;
        self.record(format!(
            "evaluator: start {} on {} (generation {})",
            correlation.completed_turn_id, correlation.thread_id, correlation.generation
        ));
        self.pending.push(PendingCompletion::PostTurnEvaluation {
            correlation,
            permit,
        });
        Ok(())
    }

    /// 진행 중 턴을 한 단계 보낸다: 시작 알림, 또는 종료 알림. 시작 전후에 가끔 스트림이 실패한다.
    fn advance_turn(&mut self) {
        // 턴이 하나이면 난수를 쓰지 않는다. 값 1 의 seed 가 이전과 같은 시나리오를 만든다.
        let index = match self.turns.len() {
            0 => return,
            1 => 0,
            running => self.rng.index(running),
        };
        let turn = self.turns[index].clone();
        let stream_fails = self.rng.percent(10);
        match turn.stage {
            _ if stream_fails => self.fail_turn(&turn),
            TurnStage::Submitted => self.start_turn(turn),
            // app-server 는 승인을 기다리는 턴을 끝내지 않는다. 중단 요청이 닿은 턴만 끝낸다.
            TurnStage::Started if turn.approval.is_some() && !turn.interrupt_requested => {
                self.resolve_approval(turn);
            }
            TurnStage::Started if self.turn_requests_approval(&turn) => self.request_approval(turn),
            TurnStage::Started => self.finish_turn(&turn),
        }
    }

    fn turn_requests_approval(&mut self, turn: &FakeTurn) -> bool {
        !turn.interrupt_requested
            && self.ledger.approvals.len() < self.settings.approval_budget
            && self.rng.percent(50)
    }

    /// 도는 턴이 승인을 요청한다 (plan 19 Q3-5). 그 턴은 보지 않는 thread 의 것일 수 있다.
    fn request_approval(&mut self, mut turn: FakeTurn) {
        let approval_id = format!("approval-{}", self.next_approval);
        self.next_approval += 1;
        self.record(format!(
            "app-server: {} requests {approval_id}",
            turn.turn_id
        ));
        self.ledger.approvals.push(SimApproval {
            approval_id: approval_id.clone(),
            thread_id: turn.thread_id.clone(),
            turn_id: turn.turn_id.clone(),
            lifetime: self.restarts,
            request_delivered: false,
            end: None,
            end_delivered: false,
            never_recorded: false,
        });
        self.send_core_input(CoreInput::ConversationStreamUpdated {
            correlation: turn.correlation,
            event: TurnStreamEvent::ApprovalRequested {
                request: ConversationApprovalRequest {
                    server_request_id: format!("server-{approval_id}"),
                    approval_id: approval_id.clone(),
                    method: "item/commandExecution/requestApproval".to_string(),
                    kind: ConversationApprovalRequestKind::CommandExecution,
                    summary: "Command execution requested.".to_string(),
                    details: Vec::new(),
                },
            },
        });
        turn.approval = Some(approval_id);
        turn.decision = None;
        self.store_turn(turn);
    }

    /// app-server 가 승인의 끝을 알린다. 다른 클라이언트의 결정이거나 시간 초과이다.
    fn resolve_approval(&mut self, mut turn: FakeTurn) {
        let Some(approval_id) = turn.approval.take() else {
            return;
        };
        // 클라이언트의 결정을 받았으면 app-server 는 그 결정으로 승인을 닫는다 (#2805).
        let choice = match turn.decision.take() {
            Some(ConversationApprovalDecision::Accept) => 0,
            Some(ConversationApprovalDecision::Decline) => 1,
            None => self.rng.below(3),
        };
        let (sent, expected) = match choice {
            0 => (
                ConversationApprovalResolution::Accepted,
                ApprovalResolution::Accepted,
            ),
            1 => (
                ConversationApprovalResolution::Declined,
                ApprovalResolution::Declined,
            ),
            _ => (
                ConversationApprovalResolution::TimedOut,
                ApprovalResolution::TimedOut,
            ),
        };
        self.record(format!("app-server: {approval_id} ended {sent:?}"));
        self.ledger
            .end_approval(&approval_id, self.restarts, expected);
        self.send_core_input(CoreInput::ConversationStreamUpdated {
            correlation: turn.correlation,
            event: TurnStreamEvent::ApprovalResolved {
                request_identity: ConversationApprovalRequestIdentity {
                    server_request_id: format!("server-{approval_id}"),
                    approval_id,
                },
                resolution: sent,
            },
        });
        self.store_turn(turn);
    }

    /// 실제 worker 는 시작 전이든 뒤든 스트림 오류를 `Failed` 하나로 알린다 (`publish_turn_submission_failure`).
    fn fail_turn(&mut self, turn: &FakeTurn) {
        self.record(format!(
            "app-server: {} failed while {:?}",
            turn.turn_id, turn.stage
        ));
        if let Some(approval_id) = &turn.approval {
            self.ledger
                .end_approval(approval_id, self.restarts, ApprovalResolution::Withdrawn);
        }
        self.drop_turn(turn.correlation);
        self.ended_turns
            .push((turn.thread_id.clone(), turn.turn_id.clone()));
        self.send_core_input(CoreInput::ConversationStreamUpdated {
            correlation: turn.correlation,
            event: TurnStreamEvent::Failed {
                message: "simulated stream failure".to_string(),
            },
        });
    }

    fn start_turn(&mut self, mut turn: FakeTurn) {
        self.record(format!("app-server: {} started", turn.turn_id));
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
        self.store_turn(turn);
    }

    fn finish_turn(&mut self, turn: &FakeTurn) {
        let outcome = if turn.interrupt_requested {
            ConversationTurnTerminalOutcome::Interrupted
        } else {
            ConversationTurnTerminalOutcome::Completed
        };
        self.record(format!("app-server: {} ended {outcome:?}", turn.turn_id));
        if let Some(approval_id) = &turn.approval {
            self.ledger
                .end_approval(approval_id, self.restarts, ApprovalResolution::Withdrawn);
        }
        self.drop_turn(turn.correlation);
        self.ended_turns
            .push((turn.thread_id.clone(), turn.turn_id.clone()));
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
            PendingCompletion::PostTurnEvaluation { correlation, .. } => {
                self.complete_evaluation(correlation);
            }
            PendingCompletion::StopAttempt {
                correlation,
                attempt,
                target,
            } => self.complete_stop_attempt(correlation, attempt, target),
            PendingCompletion::SessionCatalog { correlation } => {
                self.complete_session_catalog(correlation);
            }
            PendingCompletion::PromptPreparation {
                correlation,
                raw_prompt,
            } => self.complete_prompt_preparation(correlation, raw_prompt),
            PendingCompletion::ApprovalDecision { correlation, op } => {
                self.deliver_approval_decision(correlation, op);
            }
            PendingCompletion::Steer {
                correlation,
                request,
                op,
            } => self.deliver_steer(correlation, request, op),
            PendingCompletion::PlanningReset { correlation, op } => {
                self.deliver_planning_reset(correlation, op);
            }
            PendingCompletion::LateCompletion {
                input,
                op,
                succeeded,
            } => {
                self.record(format!(
                    "worker: the late completion of '{}' arrives",
                    self.client_ops[op].label
                ));
                self.send_client_completion(op, input, succeeded);
            }
        }
    }

    fn complete_session_catalog(&mut self, correlation: SessionCatalogLoadCorrelation) {
        let items = CATALOG_THREADS.map(catalog_session).to_vec();
        self.send_core_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::SessionCatalogLoaded {
                correlation,
                result: Ok(SessionCatalogReadySnapshot::from_catalog(
                    RecentSessions {
                        items,
                        warnings: Vec::new(),
                        next_cursor: None,
                    }
                    .into(),
                )),
            },
        ));
    }

    /// 가짜 planner 가 작업 하나를 만들고 실행할 프롬프트를 답한다. 프롬프트 글은 작업마다 다르다 (I7 이 센다).
    fn complete_prompt_preparation(
        &mut self,
        correlation: ManualPromptCorrelation,
        raw_prompt: String,
    ) {
        let task_id = format!("task-{}", self.next_task);
        let planned_prompt = format!("planned {raw_prompt} for {task_id}");
        self.prompt_prepared_behind_turn |= !self.turns.is_empty();
        let thread_id = self
            .thread_prompt_targets
            .get(&raw_prompt)
            .cloned()
            .unwrap_or_default();
        self.ledger.planned_prompts.insert(
            planned_prompt.clone(),
            PlannedPrompt {
                task_id: task_id.clone(),
                thread_id,
                submissions: 0,
                resubmissions_allowed: 0,
                submission_may_be_lost: false,
            },
        );
        self.record(format!("planner: '{raw_prompt}' -> {task_id}"));
        let outcome = ManualPromptOutcome::PromptReady {
            correlation,
            transcript_text: raw_prompt.clone(),
            runtime_projection: Box::new(RuntimeProjection::invalid("simulation")),
            intake: Box::new(ManualPromptIntakeOutcome::TaskCommitted {
                committed_task_id: task_id,
                committed_planning_revision: self.next_task as i64,
                handoff: ManualPromptMainSessionHandoff {
                    prompt: planned_prompt,
                    transcript_text: raw_prompt,
                    task: None,
                },
            }),
        };
        self.next_task += 1;
        self.send_core_input(CoreInput::EffectCompleted(
            CoreEffectCompletion::ManualPromptPrepared(Box::new(outcome)),
        ));
    }

    /*
     * 평가 요청의 문맥(자동 후속 한도, 중지 키워드)은 일부러 보지 않는다. 실제 평가기는 한도를 넘으면 멈춤을 답한다.
     * 가짜는 한도와 상관없이 프롬프트를 답할 수 있으므로, 호스트와 core 의 두 번째 방어를 시험한다.
     */
    fn complete_evaluation(&mut self, correlation: PostTurnEvaluationCorrelation) {
        let skip_auto_follow = self.rng.percent(30);
        self.complete_evaluation_with(correlation, skip_auto_follow);
    }

    fn complete_evaluation_with(
        &mut self,
        correlation: PostTurnEvaluationCorrelation,
        skip_auto_follow: bool,
    ) {
        let mut execution = post_turn_execution(&correlation);
        if skip_auto_follow {
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
        target: Option<(String, String)>,
    ) {
        let invalidated = self
            .invalidated_stop_requests
            .contains(&correlation.generation);
        let result = if invalidated && self.rng.percent(70) {
            Err("stop request was superseded before provider execution".to_string())
        } else if invalidated {
            // 무효가 되기 전에 worker 가 이미 요청을 보냈다. 정지는 그대로 app-server 에 닿는다.
            self.interrupt_running_turn(target.as_ref());
            Ok(())
        } else if self.rng.percent(25) {
            // 기본 seed 들이 실패한 정지를 운에 기대지 않고 지나도록 실패를 자주 낸다 (#2304).
            Err("simulated stop failure".to_string())
        } else {
            self.interrupt_running_turn(target.as_ref());
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

    /// 실제 app-server adapter 처럼 지금 도는 턴을 모두 중단한다 (모든 스트림). 보지 않는 thread 의 턴일 수 있다.
    /// `target` 이 있으면 그 (thread, 턴) 만 중단한다 (#2304).
    fn interrupt_running_turn(&mut self, target: Option<&(String, String)>) {
        for turn in self.turns.iter_mut().filter(|turn| {
            turn.stage == TurnStage::Started
                && target.is_none_or(|(thread_id, turn_id)| {
                    &turn.thread_id == thread_id && &turn.turn_id == turn_id
                })
        }) {
            turn.interrupt_requested = true;
        }
    }

    /// 단계를 바꾼 턴을 그 자리에 다시 둔다.
    fn store_turn(&mut self, turn: FakeTurn) {
        match self
            .turns
            .iter_mut()
            .find(|running| running.correlation == turn.correlation)
        {
            Some(running) => *running = turn,
            None => self.turns.push(turn),
        }
    }

    fn drop_turn(&mut self, correlation: TurnSubmissionCorrelation) {
        self.turns.retain(|turn| turn.correlation != correlation);
    }

    /*
     * I12: 자동 후속을 이어 가는 thread 는 하나이다 (plan 18 Q5-3). 자동 후속 턴을 받을 때, 다른 thread 의 자동 후속
     * 턴이 돌거나 그 턴의 평가가 진행 중이면 안 된다. 둘이 함께 이어 가면 planning 큐의 같은 작업을 둘이 집는다.
     */
    fn check_one_auto_follow_thread(
        &self,
        request: &TurnSubmissionRequest,
    ) -> Result<(), Violation> {
        /*
         * 값 1 은 차선이 없다 (plan 18 Q5-3 은 값 1 의 동작을 바꾸지 않는다). 그때는 한 thread 의 자동 후속 평가가 도는
         * 동안 다른 thread 의 턴 뒤 자동 후속이 나갈 수 있다. 턴은 하나씩 돌고, 늦은 쪽의 제출은 거절된다.
         */
        if self.settings.turn_concurrency < 2
            || request.prompt_origin != CorePromptOrigin::AutoFollow
        {
            return Ok(());
        }
        let Some(thread_id) = request
            .auto_follow_source
            .as_ref()
            .map(|source| &source.thread_id)
        else {
            return Ok(());
        };
        let running = self.turns.iter().find(|turn| {
            turn.origin == CorePromptOrigin::AutoFollow && &turn.thread_id != thread_id
        });
        let evaluating = self.pending.iter().find_map(|pending| match pending {
            PendingCompletion::PostTurnEvaluation {
                correlation,
                permit,
            } if permit.is_current()
                && &correlation.thread_id != thread_id
                && self.ledger.auto_follow_turns.contains(&turn_key(
                    &correlation.thread_id,
                    &correlation.completed_turn_id,
                )) =>
            {
                Some(&correlation.thread_id)
            }
            _ => None,
        });
        match running.map(|turn| &turn.thread_id).or(evaluating) {
            Some(other) => Err(self.violation(format!(
                "I12: {thread_id} continues auto-follow while {other} also continues it"
            ))),
            None => Ok(()),
        }
    }

    fn send_core_input(&mut self, input: CoreInput) {
        self.host_woken = true;
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

fn catalog_session(thread_id: &str) -> SessionSummary {
    SessionSummary {
        id: thread_id.to_string(),
        name: Some(format!("Simulated {thread_id}")),
        preview: String::new(),
        cwd: WORKSPACE.to_string(),
        source: "simulation".to_string(),
        model_provider: "simulation".to_string(),
        updated_at_epoch: 1,
        status_type: "ready".to_string(),
        path: format!("{WORKSPACE}/{thread_id}"),
        git_branch: None,
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
        KernelHostEvent::Issues(_) => Some("Issues".to_string()),
        KernelHostEvent::Resync => Some("Resync".to_string()),
        // 판단 기록은 I6 가 따로 본다. 기록에 넣으면 같은 seed 의 기록이 길어지기만 한다.
        KernelHostEvent::Recorded(_) => None,
        KernelHostEvent::IssueDecision { .. } => None,
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
    run_seed_with(seed, 1)
}

fn run_seed_with(seed: u64, turn_concurrency: usize) -> Result<Vec<String>, String> {
    Simulation::new(seed, turn_concurrency)
        .run()
        .map_err(|(violation, trace)| {
            failure_report(
                seed,
                turn_concurrency,
                &format!("step {}: {}", violation.step, violation.message),
                &trace,
            )
        })
}

fn failure_report(seed: u64, turn_concurrency: usize, message: &str, trace: &[String]) -> String {
    // `AKRA_SIM_TRACE_TAIL` 로 보고에 넣는 기록 줄 수를 늘린다 (실패를 조사할 때).
    let tail = env_number("AKRA_SIM_TRACE_TAIL").map_or(TRACE_TAIL, |lines| lines as usize);
    let tail_start = trace.len().saturating_sub(tail);
    format!(
        "kernel simulation failed (turn concurrency {turn_concurrency}): {message}\nreproduce: AKRA_SIM_SEED={seed} cargo test --locked kernel_simulation\nlast trace lines:\n{}",
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
    let seeds = seeds_to_run();
    let seed_count = seeds.len() as u64;
    let (traces, failures): (Vec<_>, Vec<_>) =
        seeds.into_iter().map(run_seed).partition(Result::is_ok);
    let failures = failures
        .into_iter()
        .filter_map(Result::err)
        .collect::<Vec<_>>();
    assert!(
        failures.is_empty(),
        "{} seed(s) failed\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
    // I10 checks nothing when no command is rejected. The default seeds must reject at least one prompt.
    if seed_count >= DEFAULT_SEED_COUNT {
        let rejected = traces
            .iter()
            .flatten()
            .flatten()
            .filter(|line| line.contains(REJECTED_AT_ONCE))
            .count();
        assert!(rejected > 0, "no seed rejected a thread prompt at once");
        // #2805: the default seeds must reach decisions, steers on an ended turn, and late completions.
        for expected in [
            "app-server: decision",
            "expected turn is not running",
            "worker: the late completion of",
        ] {
            assert!(
                traces
                    .iter()
                    .flatten()
                    .flatten()
                    .any(|line| line.contains(expected)),
                "no seed reached '{expected}'"
            );
        }
    }
}

/*
 * 동시 턴 수 2 의 커널이다 (plan 18 Q5-3). 제품 값은 1 이므로 시험에서만 돈다. 같은 불변식에 I12 (자동 후속을 이어 가는
 * thread 는 하나)를 더해 본다. 기본 seed 는 턴 둘이 실제로 함께 돌고 자동 후속이 나와야 한다. 아니면 I12 가 아무것도
 * 보지 않는다.
 */
#[test]
fn kernel_simulation_keeps_the_invariants_with_two_concurrent_turns() {
    let seeds = seeds_to_run();
    let seed_count = seeds.len() as u64;
    let (traces, failures): (Vec<_>, Vec<_>) = seeds
        .into_iter()
        .map(|seed| run_seed_with(seed, 2))
        .partition(Result::is_ok);
    let failures = failures
        .into_iter()
        .filter_map(Result::err)
        .collect::<Vec<_>>();
    assert!(
        failures.is_empty(),
        "{} seed(s) failed\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
    if seed_count >= DEFAULT_SEED_COUNT {
        let lines = || traces.iter().flatten().flatten();
        assert!(
            lines().any(|line| line.contains(ALONGSIDE_ANOTHER_TURN)),
            "no seed ran two turns at once"
        );
        assert!(
            lines()
                .any(|line| line.contains(ALONGSIDE_ANOTHER_TURN) && line.contains("AutoFollow")),
            "no seed continued auto-follow while another turn ran"
        );
    }
}

#[test]
fn kernel_simulation_restores_a_queued_prompt_after_a_restart() {
    /*
     * plan 19 Q3-7 의 중심 경로를 정해진 순서로 지난다. 무작위 시나리오에서는 이 경로가 seed 의 일부에서만 나온다.
     * 순서: 세션 목록을 읽는다 → thread 지정 프롬프트를 보낸다 → 다른 대화에서 턴이 돈다 → 준비가 끝나 프롬프트가
     * 큐에 남는다 → 커널을 다시 켠다 → 새 커널이 그 프롬프트를 한 번 낸다.
     */
    let mut simulation = Simulation::new(0, 1);
    let script = |simulation: &mut Simulation| -> Result<(), Violation> {
        let cycle = Duration::from_millis(10);
        simulation.open_client_session();
        simulation.run_host_cycle(cycle)?;
        simulation.deliver_completion();
        simulation.run_host_cycle(cycle)?;

        simulation.submit_thread_prompt();
        simulation.run_host_cycle(cycle)?;
        simulation.submit_manual_turn();
        simulation.run_host_cycle(cycle)?;
        let turn = simulation
            .turns
            .first()
            .cloned()
            .expect("the manual turn should reach the app-server");
        simulation.start_turn(turn);
        simulation.run_host_cycle(cycle)?;
        simulation.deliver_completion();
        simulation.run_host_cycle(cycle)?;

        simulation.restart_kernel()?;
        simulation.settle()
    };
    let result = script(&mut simulation);
    let trace = match simulation.finish(result) {
        Ok(trace) => trace,
        Err((violation, trace)) => panic!(
            "step {}: {}\n{}",
            violation.step,
            violation.message,
            trace.join("\n")
        ),
    };
    for expected in [
        "kernel restarted with 1 queued thread prompt(s)",
        "submitted after the restart",
    ] {
        assert!(
            trace.iter().any(|line| line.contains(expected)),
            "the scripted run did not reach '{expected}':\n{}",
            trace.join("\n")
        );
    }
}

#[test]
fn kernel_simulation_closes_an_open_approval_as_lost_after_a_restart() {
    /*
     * plan 19 Q3-5 의 중심 경로를 정해진 순서로 지난다. 순서: 턴이 돈다 → 승인을 요청한다 → 커널을 다시 켠다 → 새
     * 커널이 그 승인을 잃은 것으로 닫는다. 결정 없이 턴이 끝난 승인은 결정 없이 사라진 것으로 남는다.
     */
    let mut simulation = Simulation::new(0, 1);
    let script = |simulation: &mut Simulation| -> Result<(), Violation> {
        let cycle = Duration::from_millis(10);
        let run_turn_to_approval = |simulation: &mut Simulation| -> Result<FakeTurn, Violation> {
            simulation.submit_manual_turn();
            simulation.run_host_cycle(cycle)?;
            let turn = simulation
                .turns
                .first()
                .cloned()
                .expect("the manual turn should reach the app-server");
            simulation.start_turn(turn);
            simulation.run_host_cycle(cycle)?;
            let turn = simulation
                .turns
                .first()
                .cloned()
                .expect("the turn should run");
            simulation.request_approval(turn);
            simulation.run_host_cycle(cycle)?;
            Ok(simulation
                .turns
                .first()
                .cloned()
                .expect("the turn should wait"))
        };
        simulation.open_client_session();
        simulation.run_host_cycle(cycle)?;
        simulation.deliver_completion();
        simulation.run_host_cycle(cycle)?;

        let mut waiting = run_turn_to_approval(simulation)?;
        waiting.interrupt_requested = true;
        simulation.finish_turn(&waiting);
        simulation.run_host_cycle(cycle)?;
        simulation.settle()?;

        run_turn_to_approval(simulation)?;
        simulation.restart_kernel()?;
        simulation.settle()
    };
    let result = script(&mut simulation);
    let log = simulation.kernel_log.clone();
    if let Err((violation, trace)) = simulation.finish(result) {
        panic!(
            "step {}: {}\n{}",
            violation.step,
            violation.message,
            trace.join("\n")
        );
    }
    let ends = log
        .0
        .lock()
        .expect("kernel log should lock")
        .iter()
        .filter_map(|record| match &record.event {
            KernelEvent::ApprovalResolved {
                approval_id,
                resolution,
                ..
            } => Some((approval_id.clone(), *resolution)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        ends,
        [
            ("approval-1".to_string(), ApprovalResolution::Withdrawn),
            ("approval-2".to_string(), ApprovalResolution::Lost),
        ]
    );
}

#[test]
fn kernel_simulation_answers_a_resend_with_the_first_receipt_after_a_restart() {
    // plan 19 Q3-6 의 중심 경로를 정해진 순서로 지난다: 키가 있는 프롬프트를 보낸다 → 커널을 다시 켠다 → 같은 키로 다시
    // 보낸다. 새 커널은 기록에서 키를 되돌렸으므로 실행하지 않고 처음의 영수증을 답한다.
    let mut simulation = Simulation::new(0, 1);
    let script = |simulation: &mut Simulation| -> Result<(), Violation> {
        let cycle = Duration::from_millis(10);
        simulation.open_client_session();
        simulation.run_host_cycle(cycle)?;
        simulation.deliver_completion();
        simulation.run_host_cycle(cycle)?;

        simulation.submit_thread_prompt();
        simulation.run_host_cycle(cycle)?;
        simulation.resend_thread_prompt()?;
        simulation.restart_kernel()?;
        simulation.run_host_cycle(cycle)?;
        simulation.resend_thread_prompt()?;
        simulation.settle()
    };
    let result = script(&mut simulation);
    let trace = match simulation.finish(result) {
        Ok(trace) => trace,
        Err((violation, trace)) => panic!(
            "step {}: {}\n{}",
            violation.step,
            violation.message,
            trace.join("\n")
        ),
    };
    let resends = trace
        .iter()
        .filter(|line| line.contains("answered with the receipt of operation"))
        .count();
    let prepared = trace
        .iter()
        .filter(|line| line.contains("planner: prepare 'request 1'"))
        .count();
    assert_eq!((resends, prepared), (2, 1), "{}", trace.join("\n"));
}

/// 정해진 순서의 시나리오를 돌리고, 불변식이 깨지면 그 걸음과 기록으로 실패한다. 성공하면 기록과 커널 기록을 돌려준다.
fn run_script(
    script: impl FnOnce(&mut Simulation) -> Result<(), Violation>,
) -> (Vec<String>, Vec<RecordedKernelEvent>) {
    let mut simulation = Simulation::new(0, 1);
    let result = script(&mut simulation);
    let log = simulation.kernel_log.clone();
    match simulation.finish(result) {
        Ok(trace) => (trace, log.0.lock().expect("kernel log should lock").clone()),
        Err((violation, trace)) => panic!(
            "step {}: {}\n{}",
            violation.step,
            violation.message,
            trace.join("\n")
        ),
    }
}

/// 죽은 커널이 쓴 기록 가운데 처음으로 `matches` 에 맞는 기록 앞까지만 디스크에 닿았다.
fn keep_before(matches: impl Fn(&KernelEvent) -> bool) -> impl Fn(&[RecordedKernelEvent]) -> usize {
    move |written| {
        written
            .iter()
            .position(|record| matches(&record.event))
            .expect("the dying kernel should have written the record")
    }
}

#[test]
fn kernel_simulation_submits_a_queued_prompt_again_when_a_crash_lost_its_settlement() {
    /*
     * #2806 계약 C2: 남김은 디스크에 있고 끝남(`submitted`)은 잃었다. 새 커널은 그 프롬프트를 다시 낸다. 순서: 프롬프트가
     * 큐에 남는다 → 정상 재시작 → 새 커널이 그 프롬프트를 낸다 → 끝남 기록이 닿기 전에 죽는다 → 다음 커널이 한 번 더 낸다.
     */
    let (trace, _) = run_script(|simulation| {
        let cycle = Duration::from_millis(10);
        simulation.open_client_session();
        simulation.run_host_cycle(cycle)?;
        simulation.deliver_completion();
        simulation.run_host_cycle(cycle)?;

        simulation.submit_thread_prompt();
        simulation.run_host_cycle(cycle)?;
        simulation.submit_manual_turn();
        simulation.run_host_cycle(cycle)?;
        let turn = simulation
            .turns
            .first()
            .cloned()
            .expect("the manual turn should reach the app-server");
        simulation.start_turn(turn);
        simulation.run_host_cycle(cycle)?;
        simulation.deliver_completion();
        simulation.run_host_cycle(cycle)?;

        simulation.restart_kernel()?;
        simulation.crash_kernel(
            &keep_before(|event| {
                matches!(
                    event,
                    KernelEvent::ThreadPromptSettled {
                        settlement: ThreadPromptSettlement::Submitted,
                        ..
                    }
                )
            }),
            false,
        )?;
        simulation.settle()
    });
    let resubmitted = trace
        .iter()
        .filter(|line| line.contains("submitted after the restart"))
        .count();
    assert_eq!(resubmitted, 2, "{}", trace.join("\n"));
}

#[test]
fn kernel_simulation_runs_a_resend_again_when_a_crash_lost_its_command_record() {
    /*
     * #2806 계약 C1: 받은 기록을 잃었다. 새 커널은 멱등 키를 모르므로 같은 키의 재전송을 새 연산으로 다시 실행한다.
     * 죽은 커널이 낸 준비는 바깥에 닿았으므로 planner 는 같은 프롬프트를 두 번 준비한다.
     */
    let (trace, _) = run_script(|simulation| {
        let cycle = Duration::from_millis(10);
        simulation.open_client_session();
        simulation.run_host_cycle(cycle)?;
        simulation.deliver_completion();
        simulation.run_host_cycle(cycle)?;

        simulation.submit_thread_prompt();
        simulation.crash_kernel(
            &keep_before(|event| matches!(event, KernelEvent::CommandReceived { .. })),
            false,
        )?;
        simulation.run_host_cycle(cycle)?;
        simulation.deliver_completion();
        simulation.run_host_cycle(cycle)?;
        simulation.resend_thread_prompt()?;
        simulation.settle()
    });
    let reruns = trace
        .iter()
        .filter(|line| line.contains("ran again as operation"))
        .count();
    let prepared = trace
        .iter()
        .filter(|line| line.contains("planner: prepare 'request 1'"))
        .count();
    assert_eq!((reruns, prepared), (1, 2), "{}", trace.join("\n"));
}

#[test]
fn kernel_simulation_closes_a_command_as_lost_when_a_crash_dropped_its_effect() {
    // #2806 계약 C3: 받은 기록은 디스크에 있고 준비 효과는 커널 밖으로 나가지 않았다. 새 커널은 다시 내지 않고 `lost` 로 닫는다.
    let mut op_id = None;
    let (trace, records) = run_script(|simulation| {
        let cycle = Duration::from_millis(10);
        simulation.open_client_session();
        simulation.run_host_cycle(cycle)?;
        simulation.deliver_completion();
        simulation.run_host_cycle(cycle)?;

        simulation.submit_thread_prompt();
        op_id = simulation.sent_thread_prompts.first().map(|sent| sent.2);
        simulation.crash_kernel(&|written| written.len(), true)?;
        simulation.settle()
    });
    let op_id = op_id.expect("the prompt should have an operation");
    let states = crate::domain::kernel_event::operation_states(records.iter().map(|r| &r.event));
    assert_eq!(
        states.get(&op_id),
        Some(&crate::domain::kernel_spec::OperationState::Lost),
        "{}",
        trace.join("\n")
    );
    assert!(
        !trace.iter().any(|line| line.contains("planner: prepare")),
        "{}",
        trace.join("\n")
    );
}

/// 정해진 순서의 시험용 시뮬레이션이다. 자동 후속과 무작위 승인이 없어 스크립트가 바깥 세계를 정한다.
fn scripted_simulation() -> Simulation {
    Simulation::with_settings(
        SimRng::new(0),
        ScenarioSettings {
            max_auto_turns: 0,
            manual_turn_budget: 0,
            stop_requests: false,
            thread_prompt_budget: 0,
            restarts: false,
            approval_budget: 0,
            decision_budget: 0,
            steer_budget: 0,
            turn_concurrency: 1,
        },
    )
}

/// 스크립트를 돌리고 불변식을 본 뒤 기록과 클라이언트 명령을 돌려준다.
fn run_client_script(
    script: impl FnOnce(&mut Simulation) -> Result<(), Violation>,
) -> (Vec<RecordedKernelEvent>, Vec<ClientOperation>) {
    let mut simulation = scripted_simulation();
    let result = script(&mut simulation);
    let log = simulation.kernel_log.clone();
    let operations = simulation.client_ops.clone();
    if let Err((violation, trace)) = simulation.finish(result) {
        panic!(
            "step {}: {}\n{}",
            violation.step,
            violation.message,
            trace.join("\n")
        );
    }
    let records = log.0.lock().expect("kernel log should lock").clone();
    (records, operations)
}

fn start_scripted_session(simulation: &mut Simulation, cycle: Duration) -> Result<(), Violation> {
    simulation.open_client_session();
    simulation.run_host_cycle(cycle)?;
    deliver_everything(simulation, cycle)
}

/// 바깥 세계가 기다리는 완료(늦은 완료 포함)를 모두 돌려준다.
fn deliver_everything(simulation: &mut Simulation, cycle: Duration) -> Result<(), Violation> {
    for _ in 0..50 {
        if simulation.pending.is_empty() {
            return Ok(());
        }
        simulation.deliver_completion();
        simulation.run_host_cycle(cycle)?;
    }
    panic!("the scripted completions did not run out");
}

fn start_scripted_turn(
    simulation: &mut Simulation,
    prompt: &str,
    continues_viewed: bool,
    cycle: Duration,
) -> Result<FakeTurn, Violation> {
    simulation.submit_manual_turn_with(prompt.to_string(), continues_viewed);
    simulation.run_host_cycle(cycle)?;
    let turn = simulation
        .turns
        .first()
        .cloned()
        .expect("the manual turn should reach the app-server");
    simulation.start_turn(turn);
    simulation.run_host_cycle(cycle)?;
    Ok(simulation
        .turns
        .first()
        .cloned()
        .expect("the turn should run"))
}

fn operation_state_of(records: &[RecordedKernelEvent], op_id: i64) -> OperationState {
    crate::domain::kernel_event::operation_state(op_id, records.iter().map(|record| &record.event))
}

#[test]
fn kernel_simulation_ends_an_approval_decision_and_a_planning_reset_with_their_completion() {
    /*
     * #2805: 승인 결정과 파괴 명령(planning 초기화)이 그 효과의 완료로 `command_completed` 까지 간다. 순서: 턴이 승인을
     * 요청한다 → 클라이언트가 결정한다 → 결정이 가는 동안 두 번째 결정은 바로 거절된다 → app-server 가 그 결정으로
     * 승인을 닫는다 → planning 초기화가 끝난다. 결정 효과의 완료는 늦게 올 수 있다.
     */
    let cycle = Duration::from_millis(10);
    let mut decision = None;
    let mut second = None;
    let mut reset = None;
    let (records, operations) = run_client_script(|simulation| {
        start_scripted_session(simulation, cycle)?;
        let turn = start_scripted_turn(simulation, "task 1", false, cycle)?;
        let thread_id = turn.thread_id.clone();
        simulation.request_approval(turn);
        simulation.run_host_cycle(cycle)?;
        decision = simulation.send_decision(
            &thread_id,
            "approval-1".to_string(),
            ConversationApprovalDecision::Accept,
        );
        second = Some(simulation.client_ops.len());
        let accepted_twice = simulation.send_decision(
            &thread_id,
            "approval-1".to_string(),
            ConversationApprovalDecision::Decline,
        );
        assert_eq!(
            accepted_twice, None,
            "one approval takes one decision at a time"
        );
        simulation.run_host_cycle(cycle)?;
        deliver_everything(simulation, cycle)?;

        let waiting = simulation
            .turns
            .first()
            .cloned()
            .expect("the turn should still wait");
        simulation.resolve_approval(waiting);
        simulation.run_host_cycle(cycle)?;
        let turn = simulation
            .turns
            .first()
            .cloned()
            .expect("the turn should run");
        simulation.finish_turn(&turn);
        simulation.run_host_cycle(cycle)?;
        deliver_everything(simulation, cycle)?;

        reset = Some(simulation.client_ops.len());
        simulation.reset_planning();
        simulation.run_host_cycle(cycle)?;
        deliver_everything(simulation, cycle)?;
        simulation.settle()
    });
    let decision = &operations[decision.expect("the kernel should take the first decision")];
    let second = &operations[second.expect("the script sent a second decision")];
    let reset = &operations[reset.expect("the script sent a planning reset")];
    assert!(
        matches!(
            operation_state_of(&records, decision.op_id),
            OperationState::Done | OperationState::Failed
        ),
        "the decision ends with its completion"
    );
    assert_eq!(
        operation_state_of(&records, second.op_id),
        OperationState::Rejected
    );
    assert_eq!(
        operation_state_of(&records, reset.op_id),
        OperationState::Done
    );
    let ends = records
        .iter()
        .filter(|record| {
            matches!(&record.event, KernelEvent::ApprovalResolved { approval_id, .. }
                if approval_id == "approval-1")
        })
        .count();
    assert_eq!(ends, 1, "the approval ends once");
}

/*
 * #2805: 늦은 steer 의 완료는 다른 턴에 적용되지 않는다. 순서: 턴 1 에 steer 를 보낸다 → 그 효과가 app-server 에 닿기
 * 전에 턴 1 이 끝나고 같은 thread 에서 턴 2 가 돈다 → 턴 2 에도 steer 를 보낸다 → 턴 1 의 steer 가 늦게 닿는다.
 * app-server 는 `expectedTurnId` 가 다르므로 거절하고, 커널은 그 실패를 턴 1 의 steer 명령의 끝으로만 적는다.
 * 돌려주는 값은 기록, 클라이언트 명령, 늦은 steer 의 `client_ops` 자리이다.
 */
fn run_late_steer_script() -> (Vec<RecordedKernelEvent>, Vec<ClientOperation>, usize) {
    let cycle = Duration::from_millis(10);
    let mut late_steer = None;
    let (records, operations) = run_client_script(|simulation| {
        start_scripted_session(simulation, cycle)?;
        let first = start_scripted_turn(simulation, "task 1", false, cycle)?;
        late_steer = simulation.send_steer(&first.thread_id, &first.turn_id);
        assert!(
            late_steer.is_some(),
            "the kernel takes a steer for the running turn"
        );
        simulation.run_host_cycle(cycle)?;
        let held = simulation
            .pending
            .iter()
            .position(|pending| matches!(pending, PendingCompletion::Steer { .. }))
            .expect("the steer should leave the kernel");
        let held = simulation.pending.remove(held);
        simulation.finish_turn(&first);
        simulation.run_host_cycle(cycle)?;
        deliver_everything(simulation, cycle)?;

        let second = start_scripted_turn(simulation, "task 2", true, cycle)?;
        assert_eq!(second.thread_id, first.thread_id);
        simulation.send_steer(&second.thread_id, &second.turn_id);
        simulation.run_host_cycle(cycle)?;
        simulation.pending.push(held);
        deliver_everything(simulation, cycle)?;
        simulation.send_steer(&second.thread_id, &second.turn_id);
        simulation.run_host_cycle(cycle)?;
        deliver_everything(simulation, cycle)?;
        simulation.settle()
    });
    let late_steer = late_steer.expect("the script sent the late steer");
    (records, operations, late_steer)
}

#[test]
fn kernel_simulation_ends_a_late_steer_on_its_own_turn_only() {
    let (records, operations, late_steer) = run_late_steer_script();
    assert_eq!(operations[late_steer].steer_turn.as_deref(), Some("turn-1"));
    assert_eq!(
        operation_state_of(&records, operations[late_steer].op_id),
        OperationState::Failed
    );
    let steers_of_turn_two = operations
        .iter()
        .filter(|operation| operation.steer_turn.as_deref() == Some("turn-2"))
        .map(|operation| operation_state_of(&records, operation.op_id))
        .collect::<Vec<_>>();
    assert_eq!(steers_of_turn_two.len(), 2);
    assert!(
        steers_of_turn_two
            .iter()
            .any(|state| matches!(state, OperationState::Done | OperationState::Failed)),
        "a steer of turn 2 reaches the app-server: {steers_of_turn_two:?}"
    );
}

#[test]
fn kernel_simulation_catches_a_command_end_written_from_another_completion() {
    // 결함 주입 (#2805): 늦은 steer 의 실패가 턴 2 의 성공으로 적힌 기록을 I11 이 잡는다.
    let (mut records, operations, late_steer) = run_late_steer_script();
    let late_op = operations[late_steer].op_id;
    let injected = records.iter_mut().any(|record| match &mut record.event {
        KernelEvent::CommandCompleted { op_id, result, .. } if *op_id == late_op => {
            *result = crate::domain::kernel_event::OperationEnd::Done;
            true
        }
        _ => false,
    });
    assert!(injected, "the late steer should have a completion record");
    let error = check_client_operation_ends(&records, &operations)
        .expect_err("the invariant should catch the wrong end");
    assert!(error.starts_with("I11:"), "{error}");
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
        // plan 19 Q3-7: thread prompts and a restart. The restart with a queued prompt has its own scripted test.
        "(ManualIntake",
        "kernel restarted with 0",
        // #2806: a crash that loses records. Contracts C1 to C3 have their own scripted tests.
        "kernel crashed:",
        // plan 19 Q3-5: approvals. The restart with an open approval has its own scripted test.
        "requests approval-",
        "ended Accepted",
        "ended Declined",
        "ended TimedOut",
        // plan 19 Q3-6: a resend with the same idempotency key.
        "answered with the receipt of operation",
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

mod issue_collection;
mod issue_decision;
mod issue_triage;

mod replay;
