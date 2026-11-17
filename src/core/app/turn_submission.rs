use crate::domain::conversation::ConversationTurnOptions;
use crate::domain::planning::{ParallelTurnHandoff, TaskHandoff};

use super::{PostTurnEvaluationCorrelation, StopRequestCorrelation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnSubmissionCorrelation {
    pub generation: u64,
}

impl TurnSubmissionCorrelation {
    pub const fn new(generation: u64) -> Self {
        Self { generation }
    }
}

/*
 * 함께 도는 대화 턴의 수의 정책이다 (plan 19 P4, plan 18 Q5-1). 값은 모든 thread 를 통튼 수이다. thread 하나의 턴은
 * 이 값과 관계없이 하나이다. 기본값은 1 이다: 대화 턴은 한 번에 하나만 돈다.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnConcurrencyPolicy {
    max_concurrent_turns: usize,
}

impl TurnConcurrencyPolicy {
    /// 0 은 1 로 읽는다. 턴을 하나도 받지 않는 커널은 없다.
    pub const fn new(max_concurrent_turns: usize) -> Self {
        Self {
            max_concurrent_turns: if max_concurrent_turns == 0 {
                1
            } else {
                max_concurrent_turns
            },
        }
    }

    /// 다른 thread 에서 도는 턴이 `running_turns` 개일 때 턴을 하나 더 받을 자리가 있다.
    pub const fn has_room(self, running_turns: usize) -> bool {
        running_turns < self.max_concurrent_turns
    }
}

impl Default for TurnConcurrencyPolicy {
    fn default() -> Self {
        Self::new(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnSubmissionAdmission {
    Accepted {
        correlation: TurnSubmissionCorrelation,
    },
    RejectedActive {
        active_correlation: TurnSubmissionCorrelation,
    },
    RejectedStopPending {
        stop_correlation: StopRequestCorrelation,
    },
    RejectedUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorePromptOrigin {
    Manual,
    ManualIntake,
    AutoFollow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnSubmissionRequest {
    pub workspace_directory: String,
    pub thread_id: Option<String>,
    pub prompt: String,
    // 대화 기록에 보이는 사용자 문장이다. 수동 intake 와 자동 후속은 실제 prompt 와 다르다. core 는 이 값을
    // 턴 종료 뒤 평가의 "최근 사용자 메시지"로 쓴다 (plan 18 K1-4).
    pub transcript_text: String,
    /*
     * Staged image file paths submitted beside the prompt text. Composition maps
     * them to `localImage` turn input items; empty for internal prompts.
     */
    pub image_paths: Vec<String>,
    pub prompt_origin: CorePromptOrigin,
    pub auto_follow_source: Option<PostTurnEvaluationCorrelation>,
    /*
     * Manual-intake handoff identity enters Core with the turn intent. Plain
     * manual turns must not carry one, while auto-follow derives its handoff
     * only from the exact Core-owned post-turn route.
     */
    pub planning_handoff: Option<TaskHandoff>,
    pub turn_options: ConversationTurnOptions,
    pub slot_lease_handoff: Option<ParallelTurnHandoff>,
}

// 자동 후속 모드 이름이다. 평가 문맥과 화면이 같은 값을 쓴다.
pub const AUTO_FOLLOW_MODE_LABEL: &str = "planning queue";

impl TurnSubmissionRequest {
    pub(crate) fn request_label(&self) -> &'static str {
        if self.thread_id.is_some() {
            "turn stream"
        } else {
            "new-thread stream"
        }
    }
}
