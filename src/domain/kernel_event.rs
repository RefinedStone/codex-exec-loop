//! 커널의 판단 사건이다 (plan 19 Q3-1). 추가 전용 기록에 쓰고, 커널을 다시 조립할 때 판단 상태를 되돌린다.
//! 대화 원본, 화면 상태, 스트림 delta 는 담지 않는다 (plan 19 §5.1).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::kernel_spec::{KernelCommandKind, KernelRecordKind, spec_names};
use crate::domain::planning::ManualPromptMainSessionHandoff;

/*
 * 준비가 끝났지만 제출이 거절된 thread 지정 프롬프트이다 (plan 19 Q2-3). 준비가 만든 작업은 이미 큐에 있다. 커널이
 * 쉬면 같은 handoff 로 한 번 다시 낸다. 기록에서 되돌릴 수 있게 직렬화한다 (Q3-1).
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct QueuedThreadPrompt {
    pub thread_id: String,
    pub workspace_directory: String,
    /// 커널이 되돌릴 때만 쓰는 값이다. 스펙에는 싣지 않는다.
    #[schemars(skip)]
    pub handoff: ManualPromptMainSessionHandoff,
    pub task_id: String,
}

/// 남긴 프롬프트가 어떻게 끝났는가이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThreadPromptSettlement {
    /// 다시 내서 턴을 제출했다.
    Submitted,
    /// 병렬 모드가 켜져 내지 않았다. 작업은 병렬 slot 이 맡는다.
    ParallelSlots,
    /// planning 투영이 작업을 완료나 취소로 보여 내지 않았다.
    TaskFinished,
    /// 다시 냈지만 또 거절되었다. 작업은 큐에 남는다.
    RejectedAgain,
}

/// 턴을 낸 쪽이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TurnOrigin {
    /// 사람이 낸 프롬프트이다.
    Manual,
    /// planning 준비를 거친 프롬프트이다 (TUI 의 intake, 원격 thread 지정 프롬프트).
    ManualIntake,
    /// 턴 뒤 평가가 낸 자동 후속 프롬프트이다.
    AutoFollow,
}

/// 턴이 어떻게 끝났는가이다 (plan 19 Q3-3b). app-server 의 terminal receipt 에서 온다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TurnEndOutcome {
    Completed,
    Interrupted,
    Failed,
    /// app-server 가 끝을 확정하지 못했다.
    Unknown,
}

/// 턴 뒤 평가가 고른 다음 경로이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PostTurnRoute {
    /// 자동 후속 턴을 낸다.
    AutoSubmit,
    /// 병렬 제어부가 자동 후속 프롬프트를 썼다.
    ParallelConsumed,
    /// 이어 가지 않는다.
    NoContinuation,
}

/// thread 지정 프롬프트를 준비 전에 거절한 이유이다. planning 작업은 만들어지지 않았다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThreadPromptRejection {
    Busy,
    PreparationActive,
    UnknownThread,
    ParallelMode,
    ParallelModeUnknown,
}

/// 승인을 요청한 일의 종류이다 (plan 19 Q3-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRequestKind {
    CommandExecution,
    FileChange,
    Permissions,
}

/// 승인이 어떻게 끝났는가이다 (plan 19 Q3-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalResolution {
    Accepted,
    Declined,
    /// 결정이 없어서 app-server 가 요청을 닫았다.
    TimedOut,
    /// 결정 없이 사라졌다. 턴이 끝났거나 멈췄거나 대화가 닫혔다.
    Withdrawn,
    /// 승인이 열린 채로 커널이 다시 조립되었다. 새 커널은 그 요청에 답할 수 없다.
    Lost,
}

/*
 * 결정을 기다리는 승인이다 (plan 19 Q3-5). 요청의 내용(명령 원문, patch, 경로)은 담지 않는다. `summary` 는 클라이언트에
 * 보이는 짧은 설명과 같은 값이다. 요청과 끝은 `(thread id, 승인 id)` 로 짝을 맞춘다.
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PendingApproval {
    pub thread_id: String,
    /// 승인을 요청한 턴이다. 턴 id 를 아직 모르면 비어 있다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub approval_id: String,
    pub request_kind: ApprovalRequestKind,
    pub summary: String,
}

spec_names! {
    /// 커널이 명령을 바로 거절한 이유이다 (plan 18 K3, plan 19 Q3-6). 설명이 다음 행동을 알려 준다. 뒤의 다섯은 thread
    /// 지정 프롬프트(`submit_prompt`)를 준비 전에 거절한 이유이고 `ThreadPromptRejection` 과 같은 값이다.
    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum CommandRejectionReason {
        SteerInFlight => "steer_in_flight",
            "Another steer for the same turn is still in flight. Send again with a new idempotency key after it ends.";
        NoMatchingTurn => "no_matching_turn",
            "The thread has no running turn with `expected_turn_id`. Read `active_turn_id` from `thread_activity`.";
        DecisionInFlight => "decision_in_flight",
            "A decision for the same approval is still in flight.";
        NoMatchingApproval => "no_matching_approval",
            "No pending approval has that id. Read `approval` from `thread_activity`.";
        Busy => "busy",
            "A turn is running or the thread still has work. Send again with a new idempotency key when `thread_activity` is empty.";
        PreparationActive => "preparation_active",
            "Another prompt is being prepared. Send again with a new idempotency key after that preparation ends.";
        UnknownThread => "unknown_thread",
            "The kernel does not know the thread. Send `load_sessions` first.";
        ParallelMode => "parallel_mode",
            "Parallel mode is on. The kernel starts no thread prompt until it is off.";
        ParallelModeUnknown => "parallel_mode_unknown",
            "The kernel does not know the parallel mode yet. Send again with a new idempotency key shortly.";
    }
}

/*
 * 클라이언트가 보낸 명령을 기록하기 위한 설명이다 (plan 19 Q3-6). 명령의 내용(프롬프트 글 등)은 담지 않는다.
 * `fingerprint` 는 내용의 지문이다. 같은 멱등 키로 온 명령이 같은 명령인지 볼 때만 쓴다.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelCommandDescriptor {
    pub command: KernelCommandKind,
    /// 명령이 가리키는 thread 이다. thread 를 가리키지 않는 명령은 비어 있다.
    pub thread_id: String,
    pub fingerprint: String,
    pub idempotency_key: Option<String>,
}

/// 명령의 영수증이다 (plan 19 Q3-6). `op_id` 는 그 명령을 받은 기록의 sequence 이다. 진행은 그 뒤의 기록이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationReceipt {
    pub op_id: i64,
    /// 커널이 명령을 바로 거절했으면 그 이유이다.
    pub rejection: Option<CommandRejectionReason>,
    /// 같은 멱등 키의 명령을 전에 받았다. 다시 실행하지 않았고, 처음의 결과이다.
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[schemars(rename = "KernelRecord")]
pub enum KernelEvent {
    /// 커널이 프롬프트를 남겼다. 쉴 때 한 번 다시 낸다.
    ThreadPromptQueued(QueuedThreadPrompt),
    /// 남긴 프롬프트가 끝났다. 작업 id 로 남김과 짝을 맞춘다.
    ThreadPromptSettled {
        thread_id: String,
        workspace_directory: String,
        task_id: String,
        settlement: ThreadPromptSettlement,
    },
    /// thread 지정 프롬프트를 준비 전에 거절했다 (plan 19 Q3-2).
    ThreadPromptRejected {
        thread_id: String,
        reason: ThreadPromptRejection,
    },
    /// 턴 제출을 받아들였다. `submission` 은 그 제출의 세대 번호이다. 새 대화의 첫 턴은 thread id 가 비어 있다.
    TurnSubmitted {
        thread_id: String,
        submission: u64,
        origin: TurnOrigin,
    },
    /// app-server 가 턴을 시작했다.
    TurnStarted { thread_id: String, turn_id: String },
    /// 턴이 끝났다. `outcome` 은 그 dispatch 의 입력이 끝을 알렸을 때만 있다 (Q3-3b). 대화 무효화처럼 다른 일로
    /// 요약에서 빠진 턴은 비어 있다. Q3-3b 이전의 기록에는 이 필드가 없다.
    TurnEnded {
        thread_id: String,
        turn_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcome: Option<TurnEndOutcome>,
    },
    /// 턴 뒤 평가의 경로를 정했다.
    PostTurnRouted {
        thread_id: String,
        completed_turn_id: String,
        route: PostTurnRoute,
    },
    /// 턴이 승인을 요청했다 (plan 19 Q3-5). 결정을 기다린다.
    ApprovalRequested(PendingApproval),
    /// 승인이 끝났다. `(thread id, 승인 id)` 로 요청과 짝을 맞춘다.
    ApprovalResolved {
        thread_id: String,
        approval_id: String,
        resolution: ApprovalResolution,
    },
    /// 클라이언트의 명령을 받았다 (plan 19 Q3-6). 처리하기 전에 기록한다. `op_id` 는 이 기록의 sequence 이다.
    CommandReceived {
        op_id: i64,
        command: String,
        thread_id: String,
        fingerprint: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        idempotency_key: Option<String>,
    },
    /// 커널이 그 명령을 바로 거절했다.
    CommandRejected {
        op_id: i64,
        thread_id: String,
        reason: CommandRejectionReason,
    },
}

/// 기록에서 읽은 사건 하나이다 (plan 19 Q3-3). `sequence` 는 기록 순서이고 cursor 로 쓴다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RecordedKernelEvent {
    pub sequence: i64,
    pub recorded_at: String,
    pub event: KernelEvent,
}

/*
 * cursor 로 읽은 기록 한 쪽이다 (plan 19 Q3-3). `next_after` 는 다음에 보낼 cursor 이다. 이 binary 가 모르는 사건을
 * 건너뛰어도 cursor 는 그 뒤로 간다. 더 읽을 것이 없으면 받은 cursor 를 그대로 돌려준다.
 */
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct KernelEventPage {
    pub records: Vec<RecordedKernelEvent>,
    pub next_after: i64,
}

/// 남김 종류가 아닌 사건을 최근 이만큼만 남긴다 (plan 19 Q3-3). 남김 종류는 복구에 필요해서 지우지 않는다.
pub const RETAINED_KERNEL_EVENTS: i64 = 10_000;

/// 한 번에 읽는 사건 수의 한도이다.
pub const MAX_KERNEL_EVENT_PAGE: usize = 500;

/// 남긴 프롬프트를 되돌릴 때 읽는 사건 종류이다. 다른 사건은 읽지 않는다.
pub const THREAD_PROMPT_QUEUE_KINDS: [&str; 2] = [
    KernelRecordKind::ThreadPromptQueued.name(),
    KernelRecordKind::ThreadPromptSettled.name(),
];

/// 처리한 명령을 되돌릴 때 읽는 사건 종류이다 (plan 19 Q3-6).
pub const COMMAND_KINDS: [&str; 2] = [
    KernelRecordKind::CommandReceived.name(),
    KernelRecordKind::CommandRejected.name(),
];

/// 열린 승인을 찾을 때 읽는 사건 종류이다 (plan 19 Q3-5).
pub const APPROVAL_KINDS: [&str; 2] = [
    KernelRecordKind::ApprovalRequested.name(),
    KernelRecordKind::ApprovalResolved.name(),
];

/// 기록에 남기는 승인 설명의 최대 길이이다. 기록은 대화의 사본이 아니다.
const APPROVAL_SUMMARY_MAX_CHARS: usize = 200;

impl QueuedThreadPrompt {
    /// 이 남김이 끝났다는 사건이다.
    pub fn settled(&self, settlement: ThreadPromptSettlement) -> KernelEvent {
        KernelEvent::ThreadPromptSettled {
            thread_id: self.thread_id.clone(),
            workspace_directory: self.workspace_directory.clone(),
            task_id: self.task_id.clone(),
            settlement,
        }
    }
}

impl PendingApproval {
    pub fn new(
        thread_id: &str,
        turn_id: Option<&str>,
        approval_id: &str,
        request_kind: ApprovalRequestKind,
        summary: &str,
    ) -> Self {
        Self {
            thread_id: thread_id.to_string(),
            turn_id: turn_id.map(str::to_string),
            approval_id: approval_id.to_string(),
            request_kind,
            summary: summary.chars().take(APPROVAL_SUMMARY_MAX_CHARS).collect(),
        }
    }

    /// 이 승인이 끝났다는 사건이다.
    pub fn resolved(&self, resolution: ApprovalResolution) -> KernelEvent {
        KernelEvent::ApprovalResolved {
            thread_id: self.thread_id.clone(),
            approval_id: self.approval_id.clone(),
            resolution,
        }
    }

    fn is_same_request(&self, other: &Self) -> bool {
        self.thread_id == other.thread_id && self.approval_id == other.approval_id
    }
}

impl KernelEvent {
    /// 기록 표의 `kind` 열 값이다. 직렬화의 `kind` 와 같다.
    pub fn kind(&self) -> &'static str {
        self.record_kind().name()
    }

    /// 이 사건의 종류이다. 종류의 이름과 설명은 `KernelRecordKind` 가 가진다 (plan 18 K4-1).
    pub fn record_kind(&self) -> KernelRecordKind {
        match self {
            Self::ThreadPromptQueued(_) => KernelRecordKind::ThreadPromptQueued,
            Self::ThreadPromptSettled { .. } => KernelRecordKind::ThreadPromptSettled,
            Self::ThreadPromptRejected { .. } => KernelRecordKind::ThreadPromptRejected,
            Self::TurnSubmitted { .. } => KernelRecordKind::TurnSubmitted,
            Self::TurnStarted { .. } => KernelRecordKind::TurnStarted,
            Self::TurnEnded { .. } => KernelRecordKind::TurnEnded,
            Self::PostTurnRouted { .. } => KernelRecordKind::PostTurnRouted,
            Self::ApprovalRequested(_) => KernelRecordKind::ApprovalRequested,
            Self::ApprovalResolved { .. } => KernelRecordKind::ApprovalResolved,
            Self::CommandReceived { .. } => KernelRecordKind::CommandReceived,
            Self::CommandRejected { .. } => KernelRecordKind::CommandRejected,
        }
    }

    pub fn thread_id(&self) -> &str {
        match self {
            Self::ThreadPromptQueued(prompt) => &prompt.thread_id,
            Self::ApprovalRequested(approval) => &approval.thread_id,
            Self::ApprovalResolved { thread_id, .. }
            | Self::CommandReceived { thread_id, .. }
            | Self::CommandRejected { thread_id, .. }
            | Self::ThreadPromptSettled { thread_id, .. }
            | Self::ThreadPromptRejected { thread_id, .. }
            | Self::TurnSubmitted { thread_id, .. }
            | Self::TurnStarted { thread_id, .. }
            | Self::TurnEnded { thread_id, .. }
            | Self::PostTurnRouted { thread_id, .. } => thread_id,
        }
    }
}

/*
 * thread 요약 두 개를 비교해 턴의 시작과 끝을 낸다 (plan 19 Q3-2). 요약은 보지 않는 thread 를 포함하므로 모든 thread 의
 * 턴을 본다. 끝을 시작보다 먼저 낸다. 같은 thread 에서 턴이 바뀌면 이전 턴이 먼저 끝났기 때문이다. 값은
 * `(thread id, 진행 중 턴 id)` 이다.
 */
pub fn turn_events_between(
    before: &[(String, Option<String>)],
    after: &[(String, Option<String>)],
) -> Vec<KernelEvent> {
    let running = |turns: &[(String, Option<String>)]| {
        turns
            .iter()
            .filter_map(|(thread_id, turn_id)| Some((thread_id.clone(), turn_id.clone()?)))
            .collect::<Vec<_>>()
    };
    let (before, after) = (running(before), running(after));
    let ended = before
        .iter()
        .filter(|turn| !after.contains(turn))
        .map(|(thread_id, turn_id)| KernelEvent::TurnEnded {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            outcome: None,
        });
    let started = after
        .iter()
        .filter(|turn| !before.contains(turn))
        .map(|(thread_id, turn_id)| KernelEvent::TurnStarted {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
        });
    ended.chain(started).collect()
}

/*
 * thread 요약 두 개를 비교해 승인의 요청과 끝을 낸다 (plan 19 Q3-5). 턴과 같은 방식이다. 끝을 요청보다 먼저 낸다.
 * 끝의 종류는 여기서 모른다. `Withdrawn` 으로 내고, 그 dispatch 의 입력이 결정을 알렸으면 부르는 쪽이 바꾼다.
 */
pub fn approval_events_between(
    before: &[PendingApproval],
    after: &[PendingApproval],
) -> Vec<KernelEvent> {
    let missing_from = |approvals: &[PendingApproval], approval: &PendingApproval| {
        !approvals
            .iter()
            .any(|known| known.is_same_request(approval))
    };
    let resolved = before
        .iter()
        .filter(|approval| missing_from(after, approval))
        .map(|approval| approval.resolved(ApprovalResolution::Withdrawn));
    let requested = after
        .iter()
        .filter(|approval| missing_from(before, approval))
        .map(|approval| KernelEvent::ApprovalRequested(approval.clone()));
    resolved.chain(requested).collect()
}

/// 기록 순서대로 읽은 사건에서 아직 끝나지 않은 승인을 요청한 차례대로 돌려준다 (plan 19 Q3-5).
pub fn open_approvals(events: impl IntoIterator<Item = KernelEvent>) -> Vec<PendingApproval> {
    let mut open: Vec<PendingApproval> = Vec::new();
    for event in events {
        match event {
            KernelEvent::ApprovalRequested(approval) => {
                open.retain(|pending| !pending.is_same_request(&approval));
                open.push(approval);
            }
            KernelEvent::ApprovalResolved {
                thread_id,
                approval_id,
                ..
            } => {
                open.retain(|pending| {
                    pending.thread_id != thread_id || pending.approval_id != approval_id
                });
            }
            _ => {}
        }
    }
    open
}

/// 멱등 키로 기억한 명령 하나이다.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RememberedCommand {
    op_id: i64,
    fingerprint: String,
    rejection: Option<CommandRejectionReason>,
}

/// 멱등 키로 명령을 찾은 결과이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandRecall {
    /// 이 키는 처음이다.
    Unknown,
    /// 같은 명령을 전에 받았다. 다시 실행하지 않는다.
    Same(OperationReceipt),
    /// 이 키는 다른 명령에 썼다.
    Different,
}

/*
 * 멱등 키로 기억하는 처리한 명령이다 (plan 19 Q3-6). 기록의 투영이다. 따로 저장하지 않고, 커널을 조립할 때 기록의
 * 명령 종류(`COMMAND_KINDS`)에서 다시 만든다. 그래서 기록에 남아 있는 동안만 기억한다.
 */
#[derive(Debug, Default)]
pub struct CommandMemory {
    by_key: std::collections::BTreeMap<String, RememberedCommand>,
}

impl CommandMemory {
    /// 기록 순서대로 읽은 사건에서 다시 만든다. 키가 없는 명령은 기억하지 않는다.
    pub fn from_events(events: impl IntoIterator<Item = KernelEvent>) -> Self {
        let mut memory = Self::default();
        for event in events {
            match event {
                KernelEvent::CommandReceived {
                    op_id,
                    fingerprint,
                    idempotency_key: Some(key),
                    ..
                } => memory.remember(&key, op_id, &fingerprint),
                KernelEvent::CommandRejected { op_id, reason, .. } => memory.reject(op_id, reason),
                _ => {}
            }
        }
        memory
    }

    pub fn recall(&self, key: &str, fingerprint: &str) -> CommandRecall {
        match self.by_key.get(key) {
            None => CommandRecall::Unknown,
            Some(remembered) if remembered.fingerprint == fingerprint => {
                CommandRecall::Same(OperationReceipt {
                    op_id: remembered.op_id,
                    rejection: remembered.rejection,
                    replayed: true,
                })
            }
            Some(_) => CommandRecall::Different,
        }
    }

    pub fn remember(&mut self, key: &str, op_id: i64, fingerprint: &str) {
        self.by_key.insert(
            key.to_string(),
            RememberedCommand {
                op_id,
                fingerprint: fingerprint.to_string(),
                rejection: None,
            },
        );
    }

    /// 그 연산이 바로 거절되었다. 같은 키로 다시 오면 같은 거절을 답한다.
    pub fn reject(&mut self, op_id: i64, reason: CommandRejectionReason) {
        if let Some(remembered) = self
            .by_key
            .values_mut()
            .find(|remembered| remembered.op_id == op_id)
        {
            remembered.rejection = Some(reason);
        }
    }

    /*
     * 기록에서 지워진 범위의 명령은 잊는다. 기록은 쓸 때마다 가장 새 sequence 에서 `RETAINED_KERNEL_EVENTS` 보다
     * 오래된 사건을 지운다. 같은 규칙으로 잊어야 실행 중인 커널과 다시 조립한 커널이 같은 키를 기억한다.
     */
    pub fn forget_trimmed(&mut self, newest_sequence: i64) {
        self.by_key
            .retain(|_, remembered| remembered.op_id > newest_sequence - RETAINED_KERNEL_EVENTS);
    }
}

/// 기록 순서대로 읽은 사건에서 아직 끝나지 않은 남김을 남긴 차례대로 돌려준다.
pub fn open_queued_thread_prompts(
    events: impl IntoIterator<Item = KernelEvent>,
) -> Vec<QueuedThreadPrompt> {
    let mut open: Vec<QueuedThreadPrompt> = Vec::new();
    for event in events {
        match event {
            KernelEvent::ThreadPromptQueued(prompt) => {
                open.retain(|queued| queued.task_id != prompt.task_id);
                open.push(prompt);
            }
            KernelEvent::ThreadPromptSettled { task_id, .. } => {
                open.retain(|queued| queued.task_id != task_id);
            }
            _ => {}
        }
    }
    open
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queued(task_id: &str) -> QueuedThreadPrompt {
        QueuedThreadPrompt {
            thread_id: "thread-1".to_string(),
            workspace_directory: "/workspace".to_string(),
            handoff: ManualPromptMainSessionHandoff {
                prompt: "planned prompt".to_string(),
                transcript_text: "fix the test".to_string(),
                task: None,
            },
            task_id: task_id.to_string(),
        }
    }

    fn settled(task_id: &str) -> KernelEvent {
        queued(task_id).settled(ThreadPromptSettlement::Submitted)
    }

    #[test]
    fn only_unsettled_queued_prompts_stay_open_in_queue_order() {
        let open = open_queued_thread_prompts([
            KernelEvent::ThreadPromptQueued(queued("task-1")),
            KernelEvent::ThreadPromptQueued(queued("task-2")),
            settled("task-1"),
            KernelEvent::ThreadPromptQueued(queued("task-3")),
            settled("task-9"),
        ]);
        assert_eq!(open, vec![queued("task-2"), queued("task-3")]);
    }

    fn turn(thread: &str, turn: Option<&str>) -> (String, Option<String>) {
        (thread.to_string(), turn.map(str::to_string))
    }

    #[test]
    fn a_turn_change_ends_the_old_turn_before_it_starts_the_new_one() {
        let events = turn_events_between(
            &[turn("thread-1", Some("turn-1")), turn("thread-2", None)],
            &[
                turn("thread-1", Some("turn-2")),
                turn("thread-2", Some("turn-3")),
            ],
        );
        let ids = events
            .iter()
            .map(|event| match event {
                KernelEvent::TurnEnded { turn_id, .. } => format!("end {turn_id}"),
                KernelEvent::TurnStarted { turn_id, .. } => format!("start {turn_id}"),
                other => panic!("unexpected event {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(ids, ["end turn-1", "start turn-2", "start turn-3"]);
        let same = [turn("thread-1", Some("turn-1"))];
        assert!(turn_events_between(&same, &same).is_empty());
    }

    #[test]
    fn a_turn_end_recorded_before_the_outcome_field_still_reads() {
        let old: KernelEvent = serde_json::from_value(serde_json::json!({
            "kind": "turn_ended", "thread_id": "thread-1", "turn_id": "turn-1"
        }))
        .expect("a Q3-2 record should still decode");
        assert_eq!(
            old,
            KernelEvent::TurnEnded {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                outcome: None,
            }
        );
        assert!(
            serde_json::to_value(&old).expect("event should serialize")["outcome"].is_null(),
            "an unknown outcome is left out"
        );
    }

    #[test]
    fn every_kind_matches_its_json_tag() {
        let events = [
            KernelEvent::ThreadPromptQueued(queued("task-1")),
            settled("task-1"),
            KernelEvent::ThreadPromptRejected {
                thread_id: "thread-1".to_string(),
                reason: ThreadPromptRejection::Busy,
            },
            KernelEvent::TurnSubmitted {
                thread_id: String::new(),
                submission: 3,
                origin: TurnOrigin::AutoFollow,
            },
            KernelEvent::TurnStarted {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
            },
            KernelEvent::TurnEnded {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                outcome: Some(TurnEndOutcome::Interrupted),
            },
            KernelEvent::PostTurnRouted {
                thread_id: "thread-1".to_string(),
                completed_turn_id: "turn-1".to_string(),
                route: PostTurnRoute::AutoSubmit,
            },
            KernelEvent::ApprovalRequested(approval("thread-1", "a")),
            approval("thread-1", "a").resolved(ApprovalResolution::TimedOut),
            received(7, Some("key-1"), "fp-1"),
            KernelEvent::CommandRejected {
                op_id: 7,
                thread_id: "thread-1".to_string(),
                reason: CommandRejectionReason::NoMatchingTurn,
            },
        ];
        // 종류 하나에 사건 하나이다. 새 사건을 더하면 `record_kind` 가 컴파일되지 않고, 이 표본이 빠지면 여기서 걸린다.
        assert_eq!(
            events.iter().map(KernelEvent::kind).collect::<Vec<_>>(),
            KernelRecordKind::ALL
                .iter()
                .map(|kind| kind.name())
                .collect::<Vec<_>>()
        );
        for event in events {
            let json = serde_json::to_value(&event).expect("event should serialize");
            assert_eq!(json["kind"], event.kind());
            let decoded: KernelEvent = serde_json::from_value(json).expect("event should decode");
            assert_eq!(decoded, event);
        }
    }

    fn approval(thread: &str, approval_id: &str) -> PendingApproval {
        PendingApproval::new(
            thread,
            Some("turn-1"),
            approval_id,
            ApprovalRequestKind::CommandExecution,
            "Command execution requested.",
        )
    }

    #[test]
    fn an_approval_change_ends_the_old_request_before_the_new_one() {
        let events = approval_events_between(
            &[approval("thread-1", "a"), approval("thread-2", "b")],
            &[approval("thread-1", "c"), approval("thread-2", "b")],
        );
        assert_eq!(
            events,
            vec![
                approval("thread-1", "a").resolved(ApprovalResolution::Withdrawn),
                KernelEvent::ApprovalRequested(approval("thread-1", "c")),
            ]
        );
        // 같은 승인 id 라도 thread 가 다르면 다른 요청이다.
        assert_eq!(
            approval_events_between(&[approval("thread-1", "a")], &[approval("thread-2", "a")])
                .len(),
            2
        );
    }

    #[test]
    fn only_approvals_without_an_end_are_open() {
        let open = open_approvals([
            KernelEvent::ApprovalRequested(approval("thread-1", "a")),
            KernelEvent::ApprovalRequested(approval("thread-1", "b")),
            approval("thread-1", "a").resolved(ApprovalResolution::Accepted),
            KernelEvent::ApprovalRequested(approval("thread-2", "a")),
            approval("thread-9", "b").resolved(ApprovalResolution::Lost),
            settled("task-1"),
        ]);
        assert_eq!(
            open,
            vec![approval("thread-1", "b"), approval("thread-2", "a")]
        );
    }

    #[test]
    fn an_approval_record_keeps_a_bounded_summary_and_no_request_content() {
        let long = "x".repeat(APPROVAL_SUMMARY_MAX_CHARS + 50);
        let pending = PendingApproval::new(
            "thread-1",
            None,
            "a",
            ApprovalRequestKind::FileChange,
            &long,
        );
        assert_eq!(pending.summary.chars().count(), APPROVAL_SUMMARY_MAX_CHARS);
        let json = serde_json::to_value(KernelEvent::ApprovalRequested(pending))
            .expect("event should serialize");
        assert_eq!(json["kind"], "approval_requested");
        assert_eq!(json["request_kind"], "file_change");
        assert!(json.get("turn_id").is_none(), "an unknown turn is left out");
        let mut fields = json
            .as_object()
            .expect("an event is an object")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        fields.sort_unstable();
        assert_eq!(
            fields,
            [
                "approval_id",
                "kind",
                "request_kind",
                "summary",
                "thread_id"
            ]
        );
    }

    fn received(op_id: i64, key: Option<&str>, fingerprint: &str) -> KernelEvent {
        KernelEvent::CommandReceived {
            op_id,
            command: "steer_turn".to_string(),
            thread_id: "thread-1".to_string(),
            fingerprint: fingerprint.to_string(),
            idempotency_key: key.map(str::to_string),
        }
    }

    #[test]
    fn the_command_memory_is_rebuilt_from_the_record() {
        // plan 19 Q3-6: the memory is a projection of the log. Only commands with a key are remembered.
        let mut memory = CommandMemory::from_events([
            received(3, Some("key-a"), "fp-a"),
            received(4, None, "fp-b"),
            received(9, Some("key-c"), "fp-c"),
            KernelEvent::CommandRejected {
                op_id: 9,
                thread_id: "thread-1".to_string(),
                reason: CommandRejectionReason::SteerInFlight,
            },
            settled("task-1"),
        ]);
        assert_eq!(
            memory.recall("key-a", "fp-a"),
            CommandRecall::Same(OperationReceipt {
                op_id: 3,
                rejection: None,
                replayed: true,
            })
        );
        assert_eq!(
            memory.recall("key-c", "fp-c"),
            CommandRecall::Same(OperationReceipt {
                op_id: 9,
                rejection: Some(CommandRejectionReason::SteerInFlight),
                replayed: true,
            }),
            "a resend gets the same rejection"
        );
        assert_eq!(memory.recall("key-a", "fp-other"), CommandRecall::Different);
        assert_eq!(memory.recall("key-z", "fp-a"), CommandRecall::Unknown);

        // 기록에서 지워진 범위는 잊는다. 기록은 `sequence <= 가장 새 sequence - 보존 한도` 를 지운다.
        memory.forget_trimmed(RETAINED_KERNEL_EVENTS + 2);
        assert!(matches!(
            memory.recall("key-a", "fp-a"),
            CommandRecall::Same(_)
        ));
        memory.forget_trimmed(RETAINED_KERNEL_EVENTS + 3);
        assert_eq!(memory.recall("key-a", "fp-a"), CommandRecall::Unknown);
        assert!(matches!(
            memory.recall("key-c", "fp-c"),
            CommandRecall::Same(_)
        ));
    }

    #[test]
    fn a_command_record_has_no_content_and_leaves_out_a_missing_key() {
        let json = serde_json::to_value(received(4, None, "fp-b")).expect("event should serialize");
        let mut fields = json
            .as_object()
            .expect("an event is an object")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        fields.sort_unstable();
        assert_eq!(
            fields,
            ["command", "fingerprint", "kind", "op_id", "thread_id"]
        );
    }

    #[test]
    fn an_event_round_trips_through_json_with_its_kind() {
        let event = KernelEvent::ThreadPromptQueued(queued("task-1"));
        let json = serde_json::to_value(&event).expect("event should serialize");
        assert_eq!(json["kind"], event.kind());
        let decoded: KernelEvent = serde_json::from_value(json).expect("event should decode");
        assert_eq!(decoded, event);
        assert_eq!(
            serde_json::to_value(settled("task-1")).expect("event should serialize")["kind"],
            settled("task-1").kind()
        );
    }
}
