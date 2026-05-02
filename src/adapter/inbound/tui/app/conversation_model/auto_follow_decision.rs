/*
 * 학습 주석: auto_follow_decision은 post-turn 평가 결과를 TUI conversation model의 결정 언어로
 * 바꾸는 작은 경계다. planning runtime은 queue head와 handoff prompt를 계산하지만, TUI는 그 결과를
 * "다음 prompt를 큐에 넣기" 또는 "이유를 남기고 멈추기"로만 소비하므로 이 enum들이 adapter 내부
 * protocol 역할을 한다.
 */
use crate::application::service::planning::PlanningRuntimeQueuedAutoFollowPrompt;

use super::super::turn_activity::TurnActivityState;
use super::AutoFollowState;

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * 학습 주석: `AutoFollowupDecision`은 view model이 post-turn runtime에 돌려주는 실행 계획이다.
 * `QueuePrompt`는 application service가 만든 queue-aware prompt를 그대로 운반하고, `Skip`은 왜
 * 자동 후속 턴을 만들면 안 되는지 기록해 status line, activity notice, runtime action이 같은 이유를 보게 한다.
 */
pub(crate) enum AutoFollowupDecision {
    QueuePrompt(PlanningRuntimeQueuedAutoFollowPrompt),
    Skip(AutoFollowupSkipReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/*
 * 학습 주석: skip reason은 단순 로그 문자열이 아니라 auto-follow safety contract다. 각 variant는
 * 서로 다른 guardrail의 이름이며, post_turn_execution/followup_controls/shell_runtime 테스트가 이 값을
 * 통해 "왜 queue가 진행되지 않았는지"를 동일하게 해석한다.
 */
pub(crate) enum AutoFollowupSkipReason {
    // 학습 주석: 내부 continuation cycle에서는 operator가 켠 pause를 존중해 재귀적인 자동 제출을 막는다.
    PostTurnContinuationPaused,
    // 학습 주석: max_auto_turns budget은 runaway loop를 막는 최종 수량 guard다.
    LimitReached,
    // 학습 주석: agent reply가 없으면 planning runtime에 넘길 근거 문장이 없으므로 queue 평가를 중단한다.
    NoAgentReply,
    // 학습 주석: operator가 정한 stop keyword는 planning queue보다 앞서는 명시적 종료 신호다.
    StopKeywordMatched,
    // 학습 주석: no-file-change stop rule은 직전 턴이 실제 산출물을 만들지 않았을 때 자동 반복을 멈춘다.
    NoFileChanges,
    // 학습 주석: directions/task authority가 invalid면 queue head 판단이 신뢰할 수 없어 planning repair를 기다린다.
    PlanningBlocked,
    // 학습 주석: queue_idle.policy=stop은 actionable task가 없을 때 자동 follow-up을 만들지 않는 운영 정책이다.
    PlanningQueueIdlePolicyStop,
    // 학습 주석: queue-driven mode는 다음 task가 있을 때만 handoff prompt를 만들 수 있다.
    PlanningQueueHeadRequired,
    // 학습 주석: 같은 queue head가 반복되면 agent가 task를 전진시키지 못한 상태라 재제출을 막는다.
    PlanningRepeatedQueueHead,
    // 학습 주석: parallel slot session은 완료 후 supervisor가 후속 분배를 맡아야 하므로 같은 세션을 재사용하지 않는다.
    ParallelSessionCompleted,
    // 학습 주석: post-turn evaluation timeout은 planning runtime 응답이 늦어 TUI 회복 경로로 빠졌다는 신호다.
    PostTurnEvaluationTimedOut,
}

/*
 * 학습 주석: 한 skip reason에서 세 종류의 copy를 만든다. detail은 overlay/log에 긴 설명을 주고,
 * activity_summary는 tail/status에 짧은 event label을 주며, runtime_status는 턴 완료 문구와 함께
 * conversation lifecycle에 남는 한 줄 상태를 만든다.
 */
impl AutoFollowupSkipReason {
    /*
     * 학습 주석: detail은 operator가 follow-up이 멈춘 원인을 진단할 때 읽는 가장 설명적인 문구다.
     * AutoFollowState의 budget/keyword와 TurnActivityState의 직전 파일 변경 수를 함께 받아, 단순 reason
     * enum만으로는 알 수 없는 runtime context를 문장에 포함한다.
     */
    pub(crate) fn detail(
        self,
        auto_follow_state: &AutoFollowState,
        turn_activity: &TurnActivityState,
    ) -> String {
        // 학습 주석: 모든 variant를 여기서 직접 매핑해 새 guardrail이 생길 때 operator copy 추가를 강제한다.
        match self {
            Self::PostTurnContinuationPaused => {
                "post-turn continuation is paused for this internal runtime cycle".to_string()
            }
            Self::LimitReached => format!(
                "reached the configured auto-turn budget ({})",
                auto_follow_state.progress_label()
            ),
            Self::NoAgentReply => {
                "a non-empty agent reply is required before the next auto turn can be queued"
                    .to_string()
            }
            Self::StopKeywordMatched => format!(
                "the latest agent reply matched the stop keyword {}",
                auto_follow_state.stop_rules.stop_keyword.value()
            ),
            Self::NoFileChanges => format!(
                "the last completed turn changed {} files while the no-file stop rule is on",
                turn_activity.last_completed_file_change_count()
            ),
            Self::PlanningBlocked => {
                "planning files are invalid or incomplete; auto follow-up stays paused until they validate"
                    .to_string()
            }
            Self::PlanningQueueIdlePolicyStop => {
                "the planning queue is idle and queue_idle.policy is stop".to_string()
            }
            Self::PlanningQueueHeadRequired => {
                "queue-driven auto follow-up requires an actionable planning queue head"
                    .to_string()
            }
            Self::PlanningRepeatedQueueHead => {
                "the planning queue selected the same task again; auto follow-up stays paused until the queue advances"
                    .to_string()
            }
            Self::ParallelSessionCompleted => {
                "parallel agent session completed its assigned task; follow-up stays with the supervisor instead of reusing the same slot session"
                    .to_string()
            }
            Self::PostTurnEvaluationTimedOut => {
                "post-turn planner evaluation did not finish before the recovery timeout"
                    .to_string()
            }
        }
    }

    /*
     * 학습 주석: activity summary는 compact tail notice와 test expectation에 쓰이는 짧은 label이다.
     * detail보다 덜 설명적이지만, pause/stopped/skipped prefix를 유지해 operator가 자동 follow-up 상태를
     * 빠르게 분류할 수 있게 한다.
     */
    pub(crate) fn activity_summary(self) -> &'static str {
        match self {
            Self::PostTurnContinuationPaused => "paused: internal continuation",
            Self::LimitReached => "stopped: turn limit reached",
            Self::NoAgentReply => "skipped: no agent reply",
            Self::StopKeywordMatched => "stopped: stop keyword matched",
            Self::NoFileChanges => "stopped: no file changes",
            Self::PlanningBlocked => "paused: planning files invalid",
            Self::PlanningQueueIdlePolicyStop => "stopped: queue idle policy stop",
            Self::PlanningQueueHeadRequired => "paused: planning queue empty",
            Self::PlanningRepeatedQueueHead => "paused: planning queue repeated the same task",
            Self::ParallelSessionCompleted => "stopped: parallel session completed",
            Self::PostTurnEvaluationTimedOut => "paused: post-turn planner timeout",
        }
    }

    /*
     * 학습 주석: runtime_status는 turn lifecycle status line에 붙는 operator-facing 문장이다.
     * post_turn_execution이 SkipAutoFollowup action을 만들면 conversation model이 이 문구를 상태에 기록해,
     * 화면과 로그가 같은 skip reason을 같은 어휘로 설명한다.
     */
    pub(crate) fn runtime_status(self, auto_follow_state: &AutoFollowState) -> String {
        match self {
            Self::PostTurnContinuationPaused => {
                "turn completed / internal continuation paused".to_string()
            }
            Self::LimitReached => format!(
                "turn completed / auto follow-up stopped: turn limit reached ({})",
                auto_follow_state.progress_label()
            ),
            Self::NoAgentReply => {
                "turn completed / auto follow-up skipped: no agent reply".to_string()
            }
            Self::StopKeywordMatched => format!(
                "turn completed / auto follow-up stopped: stop keyword matched ({})",
                auto_follow_state.stop_rules.stop_keyword.value()
            ),
            Self::NoFileChanges => {
                "turn completed / auto follow-up stopped: no file changes".to_string()
            }
            Self::PlanningBlocked => {
                "turn completed / auto follow-up paused: planning files invalid".to_string()
            }
            Self::PlanningQueueIdlePolicyStop => {
                "turn completed / auto follow-up stopped: planning queue idle policy is stop"
                    .to_string()
            }
            Self::PlanningQueueHeadRequired => {
                "turn completed / auto follow-up paused: planning queue has no next task"
                    .to_string()
            }
            Self::PlanningRepeatedQueueHead => {
                "turn completed / auto follow-up paused: planning queue repeated the previous task"
                    .to_string()
            }
            Self::ParallelSessionCompleted => {
                "turn completed / auto follow-up stopped: parallel session completion is handed back to the supervisor"
                    .to_string()
            }
            Self::PostTurnEvaluationTimedOut => {
                "turn completed / auto follow-up paused: post-turn planner timed out".to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AutoFollowState, AutoFollowupSkipReason};

    #[test]
    fn post_turn_timeout_skip_reason_has_operator_copy() {
        /*
         * 학습 주석: timeout reason은 recovery path에서만 만들어지기 쉬워 일반 queue decision 테스트로
         * 놓치기 쉽다. runtime status와 activity summary 둘 다 operator가 읽는 copy이므로 variant 추가 시
         * 적어도 이 경로의 문구 계약이 유지되는지 확인한다.
         */
        let reason = AutoFollowupSkipReason::PostTurnEvaluationTimedOut;
        let state = AutoFollowState::new();

        assert!(
            reason
                .runtime_status(&state)
                .contains("post-turn planner timed out")
        );
        assert_eq!(
            reason.activity_summary(),
            "paused: post-turn planner timeout"
        );
    }
}
