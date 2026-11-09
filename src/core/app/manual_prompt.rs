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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadPromptOutcome {
    // 준비가 끝나고 턴을 제출했다. 값은 턴 제출 admission 이다.
    Submitted(super::TurnSubmissionAdmission),
    // 준비가 턴을 만들지 못했다.
    PreparationFailed { reason: String },
}
