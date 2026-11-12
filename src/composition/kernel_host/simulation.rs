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
//! 다 쓴 뒤), 승인 요청과 그 끝(app-server 가 알린 결정, 결정 없이 끝난 턴, 재시작), 멱등 키가 있는 thread 지정
//! 프롬프트의 재전송, 시계. 병렬 모드, 클라이언트의 승인
//! 결정 명령, steer, 기록을 다 쓰지 못한 강제 종료는 아직 모델하지 않는다.

use std::collections::{BTreeMap, BTreeSet};
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
    AppCommand, AppEvent, CoreEffect, CoreEffectCompletion, CoreInput, CorePromptOrigin,
    PostTurnEvaluationCorrelation, SessionCatalogLoadCorrelation, SessionCatalogLoadIntent,
    SessionCatalogReadySnapshot, StopRequestAttempt, StopRequestCorrelation,
    ThreadPromptSubmission, TurnStreamEvent, TurnSubmissionCorrelation, TurnSubmissionRequest,
};
use crate::core::runtime::{CoreEffectExecutor, CoreInputSender};
use crate::domain::conversation::{
    ConversationApprovalRequest, ConversationApprovalRequestIdentity,
    ConversationApprovalRequestKind, ConversationApprovalResolution,
};
use crate::domain::conversation_runtime_envelope::{
    ConversationRuntimeConfigurationRequest, ConversationRuntimeEnvelope,
};
use crate::domain::kernel_event::{
    ApprovalResolution, KernelCommandDescriptor, KernelEvent, RecordedKernelEvent,
    ThreadPromptSettlement, open_queued_thread_prompts,
};
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
    /// 이 턴이 결정을 기다리는 승인의 id 이다.
    approval: Option<String>,
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
    },
    SessionCatalog {
        correlation: SessionCatalogLoadCorrelation,
    },
    /// thread 지정 프롬프트의 준비이다. 가짜 planner 가 작업 하나와 실행할 프롬프트를 답한다.
    PromptPreparation {
        correlation: ManualPromptCorrelation,
        raw_prompt: String,
    },
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

    /// I7: thread 지정 프롬프트는 커널을 다시 켜도 많아야 한 번, 자기 thread 에만 닿는다.
    fn record_planned_prompt_submission(
        &mut self,
        request: &TurnSubmissionRequest,
    ) -> Result<(), String> {
        let Some(planned) = self.planned_prompts.get_mut(&request.prompt) else {
            return Ok(());
        };
        planned.submissions += 1;
        if planned.submissions > 1 {
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
     * 프롬프트를 정확히 한 번 받았어야 한다. 기록은 커널을 버린 뒤에 읽는다.
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
            let submissions = self
                .planned_prompts
                .values()
                .find(|planned| planned.task_id == task_id)
                .map_or(0, |planned| planned.submissions);
            if submissions != 1 {
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
    let effects = RecordedEffects::default();
    let planning = test_planning_services(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));
    let parallel = test_parallel_mode_control_plane_composition(planning);
    /*
     * 실행 중인 커널과 달리 병렬 보기의 작업 공간을 정하지 않는다 (`show_workspace_to_parallel_control`). 정하면
     * 클라이언트가 붙을 때마다 병렬 제어부가 감독 점검을 실제 작업 스레드에서 돌리고, 그 완료 시점이 seed 를
     * 따르지 않는다. 병렬 제어부의 효과를 가짜로 바꾸는 일은 병렬 모드를 모델할 때 한다.
     */
    let (runtime, core_inputs) =
        NativeClientRuntime::new_with_effect_executor_for_test(parallel, Box::new(effects.clone()));
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
    // 가짜 app-server 와 가짜 평가기의 상태이다.
    turn: Option<FakeTurn>,
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
}

impl Simulation {
    fn new(seed: u64) -> Self {
        let mut rng = SimRng::new(seed);
        let settings = ScenarioSettings::from_rng(&mut rng);
        let SimKernel {
            mut host,
            inbox,
            effects,
            core_inputs,
        } = build_sim_kernel();
        let kernel_log = Arc::new(SimKernelLog::default());
        host.runtime
            .attach_kernel_event_log(kernel_log.clone(), "/sim");
        let mut simulation = Self {
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
            turn: None,
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
            next_task: 1,
            next_approval: 1,
            restarts: 0,
            lifetime_starts: Vec::new(),
            carried_prompts: BTreeSet::new(),
            thread_prompt_targets: BTreeMap::new(),
            prompt_prepared_behind_turn: false,
            prompt_probably_queued: false,
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
                .map(|(_, prompt, op_id)| (*op_id, prompt.as_str()))
                .collect::<Vec<_>>();
            check_kernel_records(&records, &self.lifetime_starts)
                .and_then(|()| self.ledger.check_queued_prompt_records(&records))
                .and_then(|()| self.ledger.check_approval_records(&records))
                .and_then(|()| check_operations_ended(&records, &operations))
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
            let weight = if preparing && self.turn.is_none() {
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
            let queued = self.prompt_probably_queued && self.turn.is_some();
            let weight = if queued { 48 } else { 1 };
            choices.push((SimAction::RestartKernel, weight));
        }
        if self.turn.is_some() {
            choices.push((SimAction::AdvanceTurn, 12));
        }
        if self.turn.is_some() && self.settings.stop_requests {
            choices.push((SimAction::RequestStop, 4));
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
            SimAction::RestartKernel => return self.restart_kernel(),
            SimAction::SubmitManualTurn => self.submit_manual_turn(),
            SimAction::SubmitThreadPrompt => self.submit_thread_prompt(),
            SimAction::ResendThreadPrompt => return self.resend_thread_prompt(),
            SimAction::RequestStop => {
                self.record("client: stop all sessions".to_string());
                self.send_recorded_stop();
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
            "I4: the kernel did not settle: remaining work {}, fake turn {:?}, pending completions {}, busy threads {:?}, viewed runtime {:?}",
            self.host.runtime.has_remaining_work(),
            self.turn,
            self.pending.len(),
            self.host.thread_activity,
            self.host.runtime.snapshot().conversation_runtime
        )))
    }

    fn is_quiet(&self) -> bool {
        self.turn.is_none() && self.pending.is_empty() && !self.host.runtime.has_remaining_work()
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
        self.restarts += 1;
        let SimKernel {
            host,
            inbox,
            effects,
            core_inputs,
        } = build_sim_kernel();
        // 이전 커널을 버려야 기록기가 남은 사건을 다 쓴다. 새 커널은 그 뒤에 기록을 읽는다.
        drop(std::mem::replace(&mut self.host, host));
        self.inbox = inbox;
        self.effects = effects;
        self.core_inputs = core_inputs;
        self.client = None;
        self.viewed_thread = None;
        self.turn = None;
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
            command: AppCommand::SubmitThreadPrompt(Box::new(ThreadPromptSubmission {
                workspace_directory: WORKSPACE.to_string(),
                thread_id: thread_id.to_string(),
                prompt: prompt.to_string(),
                op_id: None,
            })),
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
     */
    fn resend_thread_prompt(&mut self) -> Result<(), Violation> {
        self.prompt_resends += 1;
        let index = self.rng.index(self.sent_thread_prompts.len());
        let (thread_id, prompt, first_op_id) = self.sent_thread_prompts[index].clone();
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
            command: AppCommand::RequestStopAllSessions,
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
            command,
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
        self.manual_turns_sent += 1;
        let continues_viewed = self.viewed_thread.is_some() && self.rng.percent(50);
        if !continues_viewed && self.viewed_thread.take().is_some() {
            self.record("client: open a new conversation".to_string());
            // TUI 는 새 대화를 열 때 자기 프롬프트 준비를 먼저 취소한다. thread 지정 준비는 취소되면 안 된다.
            self.send_command(AppCommand::CancelManualPromptPreparation);
            self.send_command(AppCommand::InvalidateConversationLoad);
        }
        let thread_id = self.viewed_thread.clone();
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
        // 커널이 준비 완료를 처리한 뒤에야 프롬프트가 큐에 있다. 턴이 없으면 커널은 큐를 바로 비운다.
        self.prompt_probably_queued = self.turn.is_some()
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
        if (self.turn.is_some() || evaluating || preparing)
            && !self.host.runtime.has_remaining_work()
        {
            return Err(self.violation(format!(
                "I5: the kernel reports no remaining work while turn {:?} runs, an evaluation is pending ({evaluating}), or a prompt is being prepared ({preparing})",
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
            approval: None,
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
        let Some(turn) = self.turn.clone() else {
            return;
        };
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
        self.turn = Some(turn);
    }

    /// app-server 가 승인의 끝을 알린다. 다른 클라이언트의 결정이거나 시간 초과이다.
    fn resolve_approval(&mut self, mut turn: FakeTurn) {
        let Some(approval_id) = turn.approval.take() else {
            return;
        };
        let (sent, expected) = match self.rng.below(3) {
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
        self.turn = Some(turn);
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
        if let Some(approval_id) = &turn.approval {
            self.ledger
                .end_approval(approval_id, self.restarts, ApprovalResolution::Withdrawn);
        }
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
            PendingCompletion::PostTurnEvaluation { correlation, .. } => {
                self.complete_evaluation(correlation);
            }
            PendingCompletion::StopAttempt {
                correlation,
                attempt,
            } => self.complete_stop_attempt(correlation, attempt),
            PendingCompletion::SessionCatalog { correlation } => {
                self.complete_session_catalog(correlation);
            }
            PendingCompletion::PromptPreparation {
                correlation,
                raw_prompt,
            } => self.complete_prompt_preparation(correlation, raw_prompt),
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
        self.prompt_prepared_behind_turn |= self.turn.is_some();
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
        let invalidated = self
            .invalidated_stop_requests
            .contains(&correlation.generation);
        let result = if invalidated && self.rng.percent(70) {
            Err("stop request was superseded before provider execution".to_string())
        } else if invalidated {
            // 무효가 되기 전에 worker 가 이미 요청을 보냈다. 정지는 그대로 app-server 에 닿는다.
            self.interrupt_running_turn();
            Ok(())
        } else if self.rng.percent(10) {
            Err("simulated stop failure".to_string())
        } else {
            self.interrupt_running_turn();
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

    /// 실제 app-server adapter 처럼 지금 도는 턴을 중단한다 (모든 스트림). 그 턴은 보지 않는 thread 의 것일 수 있다.
    fn interrupt_running_turn(&mut self) {
        if let Some(turn) = self
            .turn
            .as_mut()
            .filter(|turn| turn.stage == TurnStage::Started)
        {
            turn.interrupt_requested = true;
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
    // `AKRA_SIM_TRACE_TAIL` 로 보고에 넣는 기록 줄 수를 늘린다 (실패를 조사할 때).
    let tail = env_number("AKRA_SIM_TRACE_TAIL").map_or(TRACE_TAIL, |lines| lines as usize);
    let tail_start = trace.len().saturating_sub(tail);
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
    }
}

#[test]
fn kernel_simulation_restores_a_queued_prompt_after_a_restart() {
    /*
     * plan 19 Q3-7 의 중심 경로를 정해진 순서로 지난다. 무작위 시나리오에서는 이 경로가 seed 의 일부에서만 나온다.
     * 순서: 세션 목록을 읽는다 → thread 지정 프롬프트를 보낸다 → 다른 대화에서 턴이 돈다 → 준비가 끝나 프롬프트가
     * 큐에 남는다 → 커널을 다시 켠다 → 새 커널이 그 프롬프트를 한 번 낸다.
     */
    let mut simulation = Simulation::new(0);
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
            .turn
            .clone()
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
    let mut simulation = Simulation::new(0);
    let script = |simulation: &mut Simulation| -> Result<(), Violation> {
        let cycle = Duration::from_millis(10);
        let run_turn_to_approval = |simulation: &mut Simulation| -> Result<FakeTurn, Violation> {
            simulation.submit_manual_turn();
            simulation.run_host_cycle(cycle)?;
            let turn = simulation
                .turn
                .clone()
                .expect("the manual turn should reach the app-server");
            simulation.start_turn(turn);
            simulation.run_host_cycle(cycle)?;
            let turn = simulation.turn.clone().expect("the turn should run");
            simulation.request_approval(turn);
            simulation.run_host_cycle(cycle)?;
            Ok(simulation.turn.clone().expect("the turn should wait"))
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
    let mut simulation = Simulation::new(0);
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
