//! 커널의 판단 사건이다 (plan 19 Q3-1). 추가 전용 기록에 쓰고, 커널을 다시 조립할 때 판단 상태를 되돌린다.
//! 대화 원본, 화면 상태, 스트림 delta 는 담지 않는다 (plan 19 §5.1).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use crate::domain::kernel_spec::ReasonCode;
use crate::domain::kernel_spec::{
    CommandCompletion, KernelCommandKind, KernelRecordKind, OperationState,
};
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
    /// 이 프롬프트를 낸 명령의 연산 id 이다 (plan 18 K4-4). 다시 낸 턴도 같은 연산이다. K4-4 이전의 기록에는 없다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op_id: Option<i64>,
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
    /// 턴이 도는 채로 커널이 다시 조립되었다 (plan 18 K4-4). 새 커널은 그 턴의 끝을 받지 못한다.
    Lost,
}

/// 턴 기록 없이 끝난 명령의 결과이다 (plan 18 K4-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OperationEnd {
    Done,
    Failed,
    /// 연산이 열린 채로 커널이 다시 조립되었다.
    Lost,
}

/*
 * 턴을 내거나 돌리는 thread 하나이다 (plan 19 Q3-2, plan 18 K4-4). thread 요약에서 만들고, 두 값을 비교해 턴 기록을
 * 낸다. `turn_id` 는 app-server 가 턴을 시작한 뒤에 있다. `op_id` 는 그 턴을 낸 명령의 연산이고, 명령이 낸 턴이
 * 아니면(사람의 TUI 턴, 자동 후속) 비어 있다.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadTurn {
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub op_id: Option<i64>,
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
    /// 지우는 명령을 확인하고 보냈는가이다 (plan 18 K4-5). 커널은 확인이 없는 `destructive` 명령을 실행하지 않는다.
    pub confirmed: bool,
    /// 명령을 보낸 접속이 스스로 밝힌 이름이다 (plan 18 K4-5). 예: `akra-desktop`. 인증이 아니다. 이름이 없으면 비운다.
    pub caller: Option<String>,
}

/// 기록에 남기는 호출자 이름의 최대 길이이다. 이름은 클라이언트가 정하므로 길이를 막는다.
pub const CALLER_NAME_MAX_CHARS: usize = 64;

/// 명령의 영수증이다 (plan 19 Q3-6). `op_id` 는 그 명령을 받은 기록의 sequence 이다. 진행은 그 뒤의 기록이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationReceipt {
    pub op_id: i64,
    /// 커널이 명령을 바로 거절했으면 그 이유이다.
    pub rejection: Option<ReasonCode>,
    /// 같은 멱등 키의 명령을 전에 받았다. 다시 실행하지 않았고, 처음의 결과이다.
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[schemars(rename = "KernelRecord")]
pub enum KernelEvent {
    /// Repository runtime configuration survives an idle kernel unload.
    AutoFollowConfigured {
        max_auto_turns: usize,
        continuation_paused: bool,
    },
    /// 커널이 프롬프트를 남겼다. 쉴 때 한 번 다시 낸다.
    ThreadPromptQueued(QueuedThreadPrompt),
    /// 남긴 프롬프트가 끝났다. 작업 id 로 남김과 짝을 맞춘다.
    ThreadPromptSettled {
        thread_id: String,
        workspace_directory: String,
        task_id: String,
        settlement: ThreadPromptSettlement,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        op_id: Option<i64>,
    },
    /// thread 지정 프롬프트를 준비 전에 거절했다 (plan 19 Q3-2).
    ThreadPromptRejected {
        thread_id: String,
        reason: ReasonCode,
    },
    /// 턴 제출을 받아들였다. `submission` 은 그 제출의 세대 번호이다. 새 대화의 첫 턴은 thread id 가 비어 있다.
    /// `op_id` 는 그 턴을 낸 명령의 연산이다 (plan 18 K4-4). 명령이 낸 턴이 아니면 없다. 턴의 시작과 끝도 같다.
    TurnSubmitted {
        thread_id: String,
        submission: u64,
        origin: TurnOrigin,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        op_id: Option<i64>,
    },
    /// app-server 가 턴을 시작했다.
    TurnStarted {
        thread_id: String,
        turn_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        op_id: Option<i64>,
    },
    /// 턴이 끝났다. `outcome` 은 그 dispatch 의 입력이 끝을 알렸을 때만 있다 (Q3-3b). 대화 무효화처럼 다른 일로
    /// 요약에서 빠진 턴은 비어 있다. Q3-3b 이전의 기록에는 이 필드가 없다.
    TurnEnded {
        thread_id: String,
        turn_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcome: Option<TurnEndOutcome>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        op_id: Option<i64>,
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
        /*
         * 이 명령을 받은 커널이 그 끝을 어떻게 알리는가이다 (plan 18 K4-4). 받을 때의 규칙을 기록에 적는다. 그래야 뒤의
         * binary 가 규칙을 바꿔도 이 연산의 상태가 바뀌지 않는다. K4-4 이전의 기록에는 없다. 그 커널은 턴 기록에
         * 연산 id 를 싣지 않았으므로 그 연산의 끝은 알 수 없다. 끝을 알리지 않는 명령과 같이 다룬다.
         */
        #[serde(default, skip_serializing_if = "Option::is_none")]
        completion: Option<CommandCompletion>,
        /// 명령을 보낸 접속이 스스로 밝힌 이름이다 (plan 18 K4-5). 승인 결정도 명령이므로 누가 결정했는지 남는다.
        /// 인증이 아니다. K4-5 이전의 기록과 이름을 밝히지 않은 접속의 기록에는 없다.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caller: Option<String>,
    },
    /// 커널이 그 명령을 바로 거절했다.
    CommandRejected {
        op_id: i64,
        thread_id: String,
        reason: ReasonCode,
    },
    /// 명령이 턴 기록 없이 끝났다 (plan 18 K4-4). 턴이 끝나서 끝난 명령은 `turn_ended` 가 알린다.
    CommandCompleted {
        op_id: i64,
        thread_id: String,
        result: OperationEnd,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<ReasonCode>,
        /// 실패한 요청이 댄 원인이다 (plan 18 K4-9a). 사람이 읽는 글이다. 입구가 사용자에게 그대로 보인다.
        /// 성공한 명령과 이유 코드로 끝난 명령에는 없다.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    /*
     * 저장소의 git 상태가 지난 관찰과 다르다 (plan 20 §4.1). 커널은 주기적으로 본다. 그래서 관찰 사이의 변화
     * 여러 개가 사건 하나가 된다. 누가 바꿨는지는 모른다. Akra 의 턴일 수도 있고, Akra 가 열지 않은 에이전트나
     * 사용자일 수도 있다. 이전 값이 없으면 이 기록의 첫 관찰이다. 대화가 아니므로 thread id 가 없다.
     */
    RepositoryChanged {
        head: String,
        /// 체크아웃한 브랜치이다. 브랜치가 아닌 커밋을 체크아웃했으면 없다.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous_head: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous_branch: Option<String>,
    },
}

/// 작업 공간의 git 상태이다. 커밋이 하나도 없는 저장소와 저장소가 아닌 폴더는 상태가 없다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryState {
    pub head: String,
    pub branch: Option<String>,
}

impl RepositoryState {
    /// 지난 관찰과 다르면 그 사건이다. 지난 관찰이 없으면 첫 관찰로 기록한다.
    pub fn change_from(&self, previous: Option<&Self>) -> Option<KernelEvent> {
        if previous == Some(self) {
            return None;
        }
        Some(KernelEvent::RepositoryChanged {
            head: self.head.clone(),
            branch: self.branch.clone(),
            previous_head: previous.map(|state| state.head.clone()),
            previous_branch: previous.and_then(|state| state.branch.clone()),
        })
    }
}

/// 기록의 마지막 git 상태이다. 커널을 다시 조립할 때 읽는다. 그래야 커널이 꺼진 동안의 변화도 사건이 된다.
pub fn last_repository_state(
    events: impl IntoIterator<Item = KernelEvent>,
) -> Option<RepositoryState> {
    events
        .into_iter()
        .filter_map(|event| match event {
            KernelEvent::RepositoryChanged { head, branch, .. } => {
                Some(RepositoryState { head, branch })
            }
            _ => None,
        })
        .last()
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

/*
 * 연산 하나의 상태와 그 연산의 기록이다 (plan 18 K4-4). `operation` 조회의 답이다. 상태는 이 기록에서 계산한 값이다.
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OperationStatus {
    pub op_id: i64,
    pub state: OperationState,
    pub records: Vec<RecordedKernelEvent>,
}

impl OperationStatus {
    /// 기록 순서대로 읽은 사건에서 그 연산의 것만 골라 상태를 계산한다.
    pub fn from_records(
        op_id: i64,
        records: impl IntoIterator<Item = RecordedKernelEvent>,
    ) -> Self {
        let records = records
            .into_iter()
            .filter(|record| record.event.op_id() == Some(op_id))
            .collect::<Vec<_>>();
        Self {
            op_id,
            state: operation_state(op_id, records.iter().map(|record| &record.event)),
            records,
        }
    }
}

/// 남김 종류가 아닌 사건을 최근 이만큼만 남긴다 (plan 19 Q3-3). 남김 종류는 복구에 필요해서 지우지 않는다.
/// 마지막 `repository_changed` 하나도 지우지 않는다. 다음 커널이 그 상태에서 시작한다.
pub const RETAINED_KERNEL_EVENTS: i64 = 10_000;

/// 한 번에 읽는 사건 수의 한도이다.
pub const MAX_KERNEL_EVENT_PAGE: usize = 500;

/// 남긴 프롬프트를 되돌릴 때 읽는 사건 종류이다. 다른 사건은 읽지 않는다.
pub const THREAD_PROMPT_QUEUE_KINDS: [&str; 2] = [
    KernelRecordKind::ThreadPromptQueued.name(),
    KernelRecordKind::ThreadPromptSettled.name(),
];

/// 연산의 상태를 계산할 때 읽는 사건 종류이다 (plan 18 K4-4). 커널을 조립할 때 열린 턴과 연산을 찾는 데 쓴다.
pub const OPERATION_KINDS: [&str; 8] = [
    KernelRecordKind::CommandReceived.name(),
    KernelRecordKind::CommandRejected.name(),
    KernelRecordKind::CommandCompleted.name(),
    KernelRecordKind::ThreadPromptQueued.name(),
    KernelRecordKind::ThreadPromptSettled.name(),
    KernelRecordKind::TurnSubmitted.name(),
    KernelRecordKind::TurnStarted.name(),
    KernelRecordKind::TurnEnded.name(),
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

/// 마지막 git 상태를 찾을 때 읽는 사건 종류이다 (plan 20 §4.1).
pub const REPOSITORY_KINDS: [&str; 1] = [KernelRecordKind::RepositoryChanged.name()];
/// 자동 후속 설정의 기록 종류이다 (plan 18 K4-8). 다음 커널이 마지막 설정을 되돌린다.
pub const AUTO_FOLLOW_KINDS: [&str; 1] = [KernelRecordKind::AutoFollowConfigured.name()];

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
            op_id: self.op_id,
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
            Self::AutoFollowConfigured { .. } => KernelRecordKind::AutoFollowConfigured,
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
            Self::CommandCompleted { .. } => KernelRecordKind::CommandCompleted,
            Self::RepositoryChanged { .. } => KernelRecordKind::RepositoryChanged,
        }
    }

    /// 이 사건의 대화이다. 대화가 없는 사건은 비어 있다.
    pub fn thread_id(&self) -> &str {
        match self {
            Self::RepositoryChanged { .. } | Self::AutoFollowConfigured { .. } => "",
            Self::ThreadPromptQueued(prompt) => &prompt.thread_id,
            Self::ApprovalRequested(approval) => &approval.thread_id,
            Self::ApprovalResolved { thread_id, .. }
            | Self::CommandReceived { thread_id, .. }
            | Self::CommandRejected { thread_id, .. }
            | Self::CommandCompleted { thread_id, .. }
            | Self::ThreadPromptSettled { thread_id, .. }
            | Self::ThreadPromptRejected { thread_id, .. }
            | Self::TurnSubmitted { thread_id, .. }
            | Self::TurnStarted { thread_id, .. }
            | Self::TurnEnded { thread_id, .. }
            | Self::PostTurnRouted { thread_id, .. } => thread_id,
        }
    }
}

impl KernelEvent {
    /// 이 사건이 속한 연산이다 (plan 18 K4-4). 명령에서 나오지 않은 사건은 `None` 이다.
    pub fn op_id(&self) -> Option<i64> {
        match self {
            Self::CommandReceived { op_id, .. }
            | Self::CommandRejected { op_id, .. }
            | Self::CommandCompleted { op_id, .. } => Some(*op_id),
            Self::ThreadPromptQueued(prompt) => prompt.op_id,
            Self::ThreadPromptSettled { op_id, .. }
            | Self::TurnSubmitted { op_id, .. }
            | Self::TurnStarted { op_id, .. }
            | Self::TurnEnded { op_id, .. } => *op_id,
            Self::ThreadPromptRejected { .. }
            | Self::AutoFollowConfigured { .. }
            | Self::PostTurnRouted { .. }
            | Self::ApprovalRequested(_)
            | Self::ApprovalResolved { .. }
            | Self::RepositoryChanged { .. } => None,
        }
    }

    /// 이 사건 뒤의 그 연산의 상태이다. 상태를 바꾸지 않는 사건은 `None` 이다.
    fn operation_step(&self) -> Option<OperationState> {
        let state = match self {
            Self::CommandReceived { completion, .. } => match completion {
                Some(CommandCompletion::Accepted) => OperationState::Done,
                Some(CommandCompletion::Records | CommandCompletion::Unreported) | None => {
                    OperationState::Accepted
                }
            },
            Self::CommandRejected { .. } => OperationState::Rejected,
            Self::ThreadPromptQueued(_) | Self::TurnSubmitted { .. } | Self::TurnStarted { .. } => {
                OperationState::Running
            }
            Self::ThreadPromptSettled { settlement, .. } => match settlement {
                ThreadPromptSettlement::Submitted => OperationState::Running,
                ThreadPromptSettlement::ParallelSlots | ThreadPromptSettlement::TaskFinished => {
                    OperationState::Done
                }
                ThreadPromptSettlement::RejectedAgain => OperationState::Failed,
            },
            Self::TurnEnded { outcome, .. } => match outcome {
                Some(TurnEndOutcome::Failed) => OperationState::Failed,
                Some(TurnEndOutcome::Lost) => OperationState::Lost,
                Some(
                    TurnEndOutcome::Completed
                    | TurnEndOutcome::Interrupted
                    | TurnEndOutcome::Unknown,
                )
                | None => OperationState::Done,
            },
            Self::CommandCompleted { result, .. } => match result {
                OperationEnd::Done => OperationState::Done,
                OperationEnd::Failed => OperationState::Failed,
                OperationEnd::Lost => OperationState::Lost,
            },
            Self::ThreadPromptRejected { .. }
            | Self::AutoFollowConfigured { .. }
            | Self::PostTurnRouted { .. }
            | Self::ApprovalRequested(_)
            | Self::ApprovalResolved { .. }
            | Self::RepositoryChanged { .. } => return None,
        };
        Some(state)
    }
}

/*
 * 기록 순서대로 읽은 사건에서 연산마다의 상태를 계산한다 (plan 18 K4-4, 설계 §3.4). 상태는 저장하지 않는다. 그 연산의
 * 마지막 사건이 상태를 정한다. 받은 기록(`command_received`)이 없는 연산은 넣지 않는다. 그 기록이 보존 한도를 넘어
 * 지워졌으면 뒤의 사건만으로는 무슨 명령이었는지 알 수 없다.
 */
pub fn operation_states<'a>(
    events: impl IntoIterator<Item = &'a KernelEvent>,
) -> std::collections::BTreeMap<i64, OperationState> {
    let mut states = std::collections::BTreeMap::new();
    for event in events {
        let (Some(op_id), Some(state)) = (event.op_id(), event.operation_step()) else {
            continue;
        };
        if matches!(event, KernelEvent::CommandReceived { .. }) || states.contains_key(&op_id) {
            states.insert(op_id, state);
        }
    }
    states
}

/// 연산 하나의 상태이다. 받은 기록이 없으면 `unknown` 이다.
pub fn operation_state<'a>(
    op_id: i64,
    events: impl IntoIterator<Item = &'a KernelEvent>,
) -> OperationState {
    operation_states(events)
        .get(&op_id)
        .copied()
        .unwrap_or(OperationState::Unknown)
}

/*
 * 커널을 다시 조립할 때 이전 커널이 열어 둔 턴과 연산을 닫는 사건이다 (plan 18 K4-4). 승인의 `lost` (Q3-5)와 같은
 * 규칙이다. 새 커널은 이전 커널의 턴이 어떻게 끝났는지 받지 못한다.
 *
 * - 시작했지만 끝나지 않은 턴은 `turn_ended` 의 `lost` 로 닫는다.
 * - 그 뒤에도 열려 있는 연산은 `command_completed` 의 `lost` 로 닫는다. 큐에 남은 프롬프트의 연산은 닫지 않는다.
 *   새 커널이 그 프롬프트를 다시 내고, 그 턴도 같은 연산이다.
 * - 끝을 기록으로 알리는 명령(`CommandCompletion::Records`)의 연산만 닫는다. 끝을 알리지 않는 명령과 K4-4 이전의
 *   커널이 받은 명령은 닫지 않는다. 그 상태는 원래 `accepted` 로 남는다. 이전 커널의 명령은 턴 기록에 연산 id 가
 *   없어서, 끝난 명령도 열린 것처럼 보인다. 닫으면 끝난 명령을 `lost` 로 적게 된다.
 */
pub fn restart_closures(events: &[KernelEvent]) -> Vec<KernelEvent> {
    let mut closures = lost_turns(events);
    let queued = open_queued_thread_prompts(events.iter().cloned())
        .into_iter()
        .filter_map(|prompt| prompt.op_id)
        .collect::<Vec<_>>();
    let states = operation_states(events.iter().chain(&closures));
    for event in events {
        let KernelEvent::CommandReceived {
            op_id,
            thread_id,
            completion: Some(CommandCompletion::Records),
            ..
        } = event
        else {
            continue;
        };
        let open = states.get(op_id).is_some_and(|state| !state.is_end());
        if open && !queued.contains(op_id) {
            closures.push(KernelEvent::CommandCompleted {
                op_id: *op_id,
                thread_id: thread_id.clone(),
                result: OperationEnd::Lost,
                reason: None,
                detail: None,
            });
        }
    }
    closures
}

/// 시작했지만 끝나지 않은 턴을 `lost` 로 닫는 사건이다. 시작한 차례대로이다.
fn lost_turns(events: &[KernelEvent]) -> Vec<KernelEvent> {
    let mut open_turns: Vec<(&str, &str, Option<i64>)> = Vec::new();
    for event in events {
        match event {
            KernelEvent::TurnStarted {
                thread_id,
                turn_id,
                op_id,
            } => open_turns.push((thread_id, turn_id, *op_id)),
            KernelEvent::TurnEnded {
                thread_id, turn_id, ..
            } => open_turns.retain(|(thread, turn, _)| (*thread, *turn) != (thread_id, turn_id)),
            _ => {}
        }
    }
    open_turns
        .into_iter()
        .map(|(thread_id, turn_id, op_id)| KernelEvent::TurnEnded {
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
            outcome: Some(TurnEndOutcome::Lost),
            op_id,
        })
        .collect()
}

/*
 * thread 요약 두 개를 비교해 턴의 시작과 끝을 낸다 (plan 19 Q3-2). 요약은 보지 않는 thread 를 포함하므로 모든 thread 의
 * 턴을 본다. 끝을 시작보다 먼저 낸다. 같은 thread 에서 턴이 바뀌면 이전 턴이 먼저 끝났기 때문이다.
 *
 * 명령이 낸 턴 제출이 턴을 시작하지 못하고 사라지면(시작 전의 스트림 실패) 턴 기록이 없다. 그 연산이 끝나지 않으므로
 * `command_completed` 의 `failed` 를 낸다 (plan 18 K4-4).
 */
pub fn turn_events_between(before: &[ThreadTurn], after: &[ThreadTurn]) -> Vec<KernelEvent> {
    // 시작한 턴이다: `(thread id, 턴 id, 연산 id)`.
    let started_turns = |turns: &[ThreadTurn]| {
        turns
            .iter()
            .filter_map(|turn| Some((turn.thread_id.clone(), turn.turn_id.clone()?, turn.op_id)))
            .collect::<Vec<_>>()
    };
    let same_turn = |left: &(String, String, Option<i64>),
                     right: &(String, String, Option<i64>)| {
        left.0 == right.0 && left.1 == right.1
    };
    let (running_before, running_after) = (started_turns(before), started_turns(after));
    let ended = running_before
        .iter()
        .filter(|turn| !running_after.iter().any(|other| same_turn(turn, other)))
        .map(|(thread_id, turn_id, op_id)| KernelEvent::TurnEnded {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            outcome: None,
            op_id: *op_id,
        })
        .collect::<Vec<_>>();
    let started = running_after
        .iter()
        .filter(|turn| !running_before.iter().any(|other| same_turn(turn, other)))
        .map(|(thread_id, turn_id, op_id)| KernelEvent::TurnStarted {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            op_id: *op_id,
        })
        .collect::<Vec<_>>();
    let closed_without_a_turn = before
        .iter()
        .filter(|turn| turn.turn_id.is_none())
        .filter_map(|turn| Some((turn, turn.op_id?)))
        .filter(|(_, op_id)| !after.iter().any(|other| other.op_id == Some(*op_id)))
        .map(|(turn, op_id)| KernelEvent::CommandCompleted {
            op_id,
            thread_id: turn.thread_id.clone(),
            result: OperationEnd::Failed,
            reason: None,
            detail: None,
        });
    ended
        .into_iter()
        .chain(started)
        .chain(closed_without_a_turn)
        .collect()
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
    rejection: Option<ReasonCode>,
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
    pub fn reject(&mut self, op_id: i64, reason: ReasonCode) {
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
            op_id: None,
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

    fn turn(thread: &str, turn: Option<&str>) -> ThreadTurn {
        operation_turn(thread, turn, None)
    }

    fn operation_turn(thread: &str, turn: Option<&str>, op_id: Option<i64>) -> ThreadTurn {
        ThreadTurn {
            thread_id: thread.to_string(),
            turn_id: turn.map(str::to_string),
            op_id,
        }
    }

    /// 지금의 커널이 받은 명령이다. 그 명령의 끝 규칙을 함께 적는다.
    fn received_command(op_id: i64, command: &str) -> KernelEvent {
        let completion = KernelCommandKind::ALL
            .iter()
            .find(|kind| kind.name() == command)
            .map(|kind| kind.spec().completion);
        assert!(completion.is_some(), "unknown command {command}");
        received_with(op_id, command, completion)
    }

    /// K4-4 이전의 커널이 받은 명령이다. 끝 규칙이 없다.
    fn received_before_completion(op_id: i64, command: &str) -> KernelEvent {
        received_with(op_id, command, None)
    }

    fn received_with(
        op_id: i64,
        command: &str,
        completion: Option<CommandCompletion>,
    ) -> KernelEvent {
        KernelEvent::CommandReceived {
            op_id,
            command: command.to_string(),
            thread_id: "thread-1".to_string(),
            fingerprint: "fp".to_string(),
            idempotency_key: None,
            completion,
            caller: None,
        }
    }

    #[test]
    fn a_command_of_an_older_kernel_is_not_closed_as_lost() {
        // The older kernel put no operation id on its turn records. Its finished prompts look open.
        // Closing them at a restart would record finished commands as lost.
        let old: KernelEvent = serde_json::from_value(serde_json::json!({
            "kind": "command_received", "op_id": 1, "command": "submit_prompt",
            "thread_id": "thread-1", "fingerprint": "fp"
        }))
        .expect("a command record without `completion` should still decode");
        assert_eq!(old, received_before_completion(1, "submit_prompt"));
        let records = [
            old,
            turn_started("turn-1", None),
            turn_ended("turn-1", TurnEndOutcome::Completed, None),
            received_command(4, "submit_prompt"),
        ];
        assert_eq!(
            restart_closures(&records),
            vec![KernelEvent::CommandCompleted {
                op_id: 4,
                thread_id: "thread-1".to_string(),
                result: OperationEnd::Lost,
                reason: None,
                detail: None,
            }],
            "only the command of this kernel generation is closed"
        );
        assert_eq!(operation_state(1, &records), OperationState::Accepted);
        // The rule of a command is the rule the receiving kernel recorded, not the rule of this binary.
        let unreported_then =
            received_with(5, "submit_prompt", Some(CommandCompletion::Unreported));
        assert!(restart_closures(std::slice::from_ref(&unreported_then)).is_empty());
    }

    fn turn_started(turn: &str, op_id: Option<i64>) -> KernelEvent {
        KernelEvent::TurnStarted {
            thread_id: "thread-1".to_string(),
            turn_id: turn.to_string(),
            op_id,
        }
    }

    fn turn_ended(turn: &str, outcome: TurnEndOutcome, op_id: Option<i64>) -> KernelEvent {
        KernelEvent::TurnEnded {
            thread_id: "thread-1".to_string(),
            turn_id: turn.to_string(),
            outcome: Some(outcome),
            op_id,
        }
    }

    #[test]
    fn turn_records_carry_the_operation_of_the_command_that_submitted_the_turn() {
        // plan 18 K4-4: the start and the end of a turn name the operation, so the operation's end is known.
        let submitting = [operation_turn("thread-1", None, Some(7))];
        let running = [operation_turn("thread-1", Some("turn-1"), Some(7))];
        assert_eq!(
            turn_events_between(&submitting, &running),
            vec![turn_started("turn-1", Some(7))]
        );
        assert_eq!(
            turn_events_between(&running, &[]),
            vec![KernelEvent::TurnEnded {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                outcome: None,
                op_id: Some(7),
            }]
        );
        // A submission that closes before a turn started has no turn record. The operation still ends.
        assert_eq!(
            turn_events_between(&submitting, &[]),
            vec![KernelEvent::CommandCompleted {
                op_id: 7,
                thread_id: "thread-1".to_string(),
                result: OperationEnd::Failed,
                reason: None,
                detail: None,
            }]
        );
        // A turn that no command submitted (a TUI turn, automatic follow-up) closes without an operation record.
        assert!(turn_events_between(&[turn("thread-1", None)], &[]).is_empty());
    }

    #[test]
    fn the_operation_state_is_the_last_record_of_the_operation() {
        let queued_prompt = QueuedThreadPrompt {
            op_id: Some(7),
            ..queued("task-1")
        };
        let mut records = vec![received_command(7, "submit_prompt")];
        let state = |records: &[KernelEvent]| operation_state(7, records);
        assert_eq!(state(&records), OperationState::Accepted);
        records.push(KernelEvent::ThreadPromptQueued(queued_prompt.clone()));
        assert_eq!(
            state(&records),
            OperationState::Running,
            "queued is not an end"
        );
        records.push(queued_prompt.settled(ThreadPromptSettlement::Submitted));
        records.push(turn_started("turn-1", Some(7)));
        // Records of other operations and of no operation do not change the state.
        records.push(turn_ended("turn-9", TurnEndOutcome::Failed, None));
        records.push(received_command(8, "submit_prompt"));
        assert_eq!(state(&records), OperationState::Running);
        records.push(turn_ended("turn-1", TurnEndOutcome::Completed, Some(7)));
        assert_eq!(state(&records), OperationState::Done);
        assert_eq!(operation_state(8, &records), OperationState::Accepted);

        let ends = [
            (
                KernelEvent::CommandRejected {
                    op_id: 7,
                    thread_id: "thread-1".to_string(),
                    reason: ReasonCode::Busy,
                },
                OperationState::Rejected,
            ),
            (
                turn_ended("turn-1", TurnEndOutcome::Failed, Some(7)),
                OperationState::Failed,
            ),
            (
                turn_ended("turn-1", TurnEndOutcome::Interrupted, Some(7)),
                OperationState::Done,
            ),
            (
                turn_ended("turn-1", TurnEndOutcome::Lost, Some(7)),
                OperationState::Lost,
            ),
            (
                queued_prompt.settled(ThreadPromptSettlement::TaskFinished),
                OperationState::Done,
            ),
            (
                queued_prompt.settled(ThreadPromptSettlement::RejectedAgain),
                OperationState::Failed,
            ),
            (
                KernelEvent::CommandCompleted {
                    op_id: 7,
                    thread_id: "thread-1".to_string(),
                    result: OperationEnd::Failed,
                    reason: Some(ReasonCode::PlanningRefused),
                    detail: None,
                },
                OperationState::Failed,
            ),
        ];
        for (end, expected) in ends {
            assert_eq!(
                operation_state(7, &[received_command(7, "submit_prompt"), end.clone()]),
                expected,
                "{end:?}"
            );
        }
    }

    #[test]
    fn an_operation_without_its_received_record_is_unknown() {
        // The received record names the command. Without it the later records do not say what ended.
        assert_eq!(
            operation_state(
                7,
                &[turn_ended("turn-1", TurnEndOutcome::Completed, Some(7))]
            ),
            OperationState::Unknown
        );
        assert_eq!(operation_state(7, &[]), OperationState::Unknown);
        // A command that ends when the kernel accepts it is done at once.
        assert_eq!(
            operation_state(3, &[received_command(3, "pause_auto_follow")]),
            OperationState::Done
        );
        // A command that ends with a later record is accepted until that record comes.
        assert_eq!(
            operation_state(4, &[received_command(4, "steer_turn")]),
            OperationState::Accepted
        );
        assert_eq!(
            operation_state(
                4,
                &[
                    received_command(4, "steer_turn"),
                    KernelEvent::CommandCompleted {
                        op_id: 4,
                        thread_id: "thread-1".to_string(),
                        result: OperationEnd::Done,
                        reason: None,
                        detail: None,
                    },
                ]
            ),
            OperationState::Done
        );
    }

    #[test]
    fn a_restart_closes_open_turns_and_operations_but_not_a_queued_prompt() {
        let queued_prompt = QueuedThreadPrompt {
            op_id: Some(3),
            ..queued("task-3")
        };
        let records = [
            // 1: its turn runs. 2: still preparing. 3: queued. 4: ended. 5: received by a kernel that reported no end.
            received_command(1, "submit_prompt"),
            turn_started("turn-1", Some(1)),
            received_command(2, "submit_prompt"),
            received_command(3, "submit_prompt"),
            KernelEvent::ThreadPromptQueued(queued_prompt),
            received_command(4, "submit_prompt"),
            turn_started("turn-4", Some(4)),
            turn_ended("turn-4", TurnEndOutcome::Completed, Some(4)),
            received_with(5, "steer_turn", Some(CommandCompletion::Unreported)),
            // A turn that no command submitted.
            turn_started("turn-auto", None),
        ];
        let closures = restart_closures(&records);
        assert_eq!(
            closures,
            vec![
                turn_ended("turn-1", TurnEndOutcome::Lost, Some(1)),
                turn_ended("turn-auto", TurnEndOutcome::Lost, None),
                KernelEvent::CommandCompleted {
                    op_id: 2,
                    thread_id: "thread-1".to_string(),
                    result: OperationEnd::Lost,
                    reason: None,
                    detail: None,
                },
            ]
        );
        let after = records.iter().chain(&closures).cloned().collect::<Vec<_>>();
        assert_eq!(operation_state(1, &after), OperationState::Lost);
        assert_eq!(operation_state(2, &after), OperationState::Lost);
        assert_eq!(operation_state(3, &after), OperationState::Running);
        assert_eq!(operation_state(4, &after), OperationState::Done);
        assert_eq!(operation_state(5, &after), OperationState::Accepted);
        // The next restart finds nothing more to close.
        assert!(restart_closures(&after).is_empty());
    }

    #[test]
    fn records_written_before_the_operation_id_still_read() {
        let old: KernelEvent = serde_json::from_value(serde_json::json!({
            "kind": "turn_started", "thread_id": "thread-1", "turn_id": "turn-1"
        }))
        .expect("a record without `op_id` should still decode");
        assert_eq!(old, turn_started("turn-1", None));
        assert!(
            serde_json::to_value(&old).expect("event should serialize")["op_id"].is_null(),
            "a turn that no command submitted leaves the field out"
        );
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
                op_id: None,
            }
        );
        assert!(
            serde_json::to_value(&old).expect("event should serialize")["outcome"].is_null(),
            "an unknown outcome is left out"
        );
    }

    #[test]
    fn rejection_records_written_before_the_one_reason_list_still_read() {
        // plan 18 K4-3: the stored text of the two old reason enums is the text of `ReasonCode`.
        let prompt: KernelEvent = serde_json::from_value(serde_json::json!({
            "kind": "thread_prompt_rejected", "thread_id": "thread-1", "reason": "preparation_active"
        }))
        .expect("an old thread prompt rejection should still decode");
        assert_eq!(
            prompt,
            KernelEvent::ThreadPromptRejected {
                thread_id: "thread-1".to_string(),
                reason: ReasonCode::PreparationActive,
            }
        );
        let command: KernelEvent = serde_json::from_value(serde_json::json!({
            "kind": "command_rejected", "op_id": 7, "thread_id": "thread-1", "reason": "steer_in_flight"
        }))
        .expect("an old command rejection should still decode");
        assert_eq!(
            command,
            KernelEvent::CommandRejected {
                op_id: 7,
                thread_id: "thread-1".to_string(),
                reason: ReasonCode::SteerInFlight,
            }
        );
    }

    #[test]
    fn every_kind_matches_its_json_tag() {
        let events = [
            KernelEvent::AutoFollowConfigured {
                max_auto_turns: 3,
                continuation_paused: true,
            },
            KernelEvent::ThreadPromptQueued(queued("task-1")),
            settled("task-1"),
            KernelEvent::ThreadPromptRejected {
                thread_id: "thread-1".to_string(),
                reason: ReasonCode::Busy,
            },
            KernelEvent::TurnSubmitted {
                thread_id: String::new(),
                submission: 3,
                origin: TurnOrigin::AutoFollow,
                op_id: None,
            },
            KernelEvent::TurnStarted {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                op_id: None,
            },
            KernelEvent::TurnEnded {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                outcome: Some(TurnEndOutcome::Interrupted),
                op_id: None,
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
                reason: ReasonCode::NoMatchingTurn,
            },
            KernelEvent::CommandCompleted {
                op_id: 7,
                thread_id: "thread-1".to_string(),
                result: OperationEnd::Failed,
                reason: Some(ReasonCode::PlanningRefused),
                detail: None,
            },
            repository("b", Some("main"))
                .change_from(Some(&repository("a", None)))
                .expect("a different state is a change"),
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
            completion: None,
            caller: None,
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
                reason: ReasonCode::SteerInFlight,
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
                rejection: Some(ReasonCode::SteerInFlight),
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

    fn repository(head: &str, branch: Option<&str>) -> RepositoryState {
        RepositoryState {
            head: head.to_string(),
            branch: branch.map(str::to_string),
        }
    }

    #[test]
    fn a_repository_change_carries_the_previous_state_and_the_log_restores_the_last_one() {
        let first = repository("a", Some("main"));
        let baseline = first.change_from(None).expect("the first observation");
        assert_eq!(
            serde_json::to_value(&baseline).expect("event should serialize"),
            serde_json::json!({ "kind": "repository_changed", "head": "a", "branch": "main" })
        );
        assert_eq!(first.change_from(Some(&first)), None);

        // 같은 커밋에서 브랜치만 바뀌어도 변화이다. 브랜치가 없는 체크아웃은 `branch` 가 없다.
        let detached = repository("a", None);
        let moved = detached.change_from(Some(&first)).expect("a branch change");
        assert_eq!(
            moved,
            KernelEvent::RepositoryChanged {
                head: "a".to_string(),
                branch: None,
                previous_head: Some("a".to_string()),
                previous_branch: Some("main".to_string()),
            }
        );
        assert_eq!(moved.thread_id(), "");
        assert_eq!(moved.op_id(), None);

        assert_eq!(
            last_repository_state([baseline, settled("task-1"), moved]),
            Some(detached)
        );
        assert_eq!(last_repository_state([settled("task-1")]), None);
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
