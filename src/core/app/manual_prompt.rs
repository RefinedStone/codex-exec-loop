use crate::domain::planning::ManualPromptCorrelation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualPromptPreparationIntent {
    pub workspace_directory: String,
    pub raw_prompt: String,
    pub parent_thread_id: Option<String>,
    pub parent_turn_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManualPromptPreparationAdmission {
    Accepted {
        correlation: ManualPromptCorrelation,
    },
    RejectedActive {
        active_correlation: ManualPromptCorrelation,
    },
}

/*
 * thread 를 지정한 프롬프트 제출이다 (plan 18 K3-3b). core 가 준비(planning 조각 붙이기) 뒤 그 thread 에 턴을
 * 제출한다. 원격 클라이언트가 쓴다. 보는 자리는 바꾸지 않는다.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadPromptSubmission {
    pub workspace_directory: String,
    pub thread_id: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadPromptAdmission {
    // 준비를 시작했다. 결과는 `AppEvent::ThreadPromptSubmitted` 로 온다.
    Accepted {
        correlation: ManualPromptCorrelation,
    },
    // 그 thread 에 일이 남아 있거나, 다른 thread 에서 턴이 돈다. 턴은 모든 thread 를 통틀어 하나만 돈다.
    RejectedBusy,
    // 다른 프롬프트 준비가 진행 중이다. 준비는 한 번에 하나이다.
    RejectedPreparationActive {
        active_correlation: ManualPromptCorrelation,
    },
    // core 가 모르는 thread 이다. 보는 thread, 일이 남은 thread, 읽은 세션 목록 어디에도 없다.
    RejectedUnknownThread,
    // 병렬 모드가 켜져 있다. 병렬 모드에서 수동 프롬프트는 병렬 작업 intake 로 가야 하는데, 그 경로는 아직 TUI
    // 에만 있다. main session 턴으로 내면 같은 작업이 병렬 slot 에도 나갈 수 있다.
    RejectedParallelMode,
    // 호스트가 병렬 모드 값을 아직 알리지 않았다. 값을 모르면 병렬 모드일 수 있으므로 받지 않는다.
    RejectedParallelModeUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadPromptOutcome {
    // 준비가 끝나고 턴 제출을 시도했다. `task_id` 는 준비가 큐에 만들거나 고친 planning 작업이다. 제출이
    // 거절되어도 그 작업은 큐에 남는다. 같은 프롬프트를 다시 보내면 작업이 하나 더 생긴다.
    Submitted {
        admission: super::TurnSubmissionAdmission,
        task_id: String,
    },
    // 준비하는 사이에 병렬 모드가 켜졌다. main session 턴으로 내지 않는다. 작업은 큐에 남고 병렬 slot 이 맡는다.
    ParallelModeStarted {
        task_id: String,
    },
    // 준비가 턴을 만들지 못했다. planning 작업은 만들어지지 않았다.
    PreparationFailed(ThreadPromptPreparationFailure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadPromptPreparationFailure {
    // 준비가 취소되었다.
    Cancelled,
    // 준비 결과가 보낸 프롬프트와 맞지 않는다.
    TextMismatch,
    // planning 초안을 사람이 검토해야 한다. 원격 클라이언트는 검토할 수 없다.
    BootstrapReviewRequired { draft_name: String },
    // planning 이 프롬프트를 받지 않았거나 준비가 실패했다.
    PlanningRefused { reason: String },
}
