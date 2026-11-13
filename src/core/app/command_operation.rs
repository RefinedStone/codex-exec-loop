//! 명령의 끝을 기록하기 위한 추적이다 (plan 18 K4-4b, 설계 `docs/plan/18-k4-command-spec.md` §3.4).
//!
//! 이름 바꾸기, planning 초기화, 중지, steer, 승인 결정은 효과 하나를 내고, 그 효과의 완료가 core 입력으로 돌아온다.
//! 그 완료가 명령의 끝이다. 효과와 완료는 같은 correlation 을 가지므로, 명령이 낸 효과의 correlation 에 연산 id 를
//! 묶어 두었다가 완료가 오면 끝을 적는다. correlation 타입에는 연산 id 를 넣지 않는다. 그래서 다섯 기능의 reducer 는
//! 바뀌지 않는다.
//!
//! 완료는 core 가 그 결과를 쓰지 않을 때(이미 낡은 correlation)도 명령의 끝이다. 효과는 실행되었기 때문이다.

use super::{
    ApprovalDecisionCorrelation, CoreEffect, CoreEffectCompletion, CoreInput,
    PlanningWorkspaceOperationCorrelation, SessionRenameCorrelation, StopRequestAttempt,
    StopRequestCorrelation, TurnSteerCorrelation,
};

/// 명령이 낸 효과를 가리키는 값이다. 효과와 그 완료가 같은 값을 가진다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CommandEffectKey {
    Rename(SessionRenameCorrelation),
    Reset(PlanningWorkspaceOperationCorrelation),
    Stop(StopRequestCorrelation),
    Steer(TurnSteerCorrelation),
    ApprovalDecision(ApprovalDecisionCorrelation),
}

impl CommandEffectKey {
    /// 명령을 처리하며 낸 효과 중 그 명령의 효과이다. 명령이 효과를 내지 않았으면 `None` 이다.
    pub(super) fn of_effects(effects: &[CoreEffect]) -> Option<Self> {
        effects.iter().find_map(|effect| match effect {
            CoreEffect::RenameSession { correlation } => Some(Self::Rename(correlation.clone())),
            CoreEffect::ResetPlanningWorkspace { correlation } => {
                Some(Self::Reset(correlation.clone()))
            }
            // 중지의 끝은 처음 보낸 중지 요청의 완료이다. 턴이 시작한 뒤의 다시 보내기는 커널 안의 일이다.
            CoreEffect::RequestStopAllSessions {
                correlation,
                attempt: StopRequestAttempt::Initial,
            } => Some(Self::Stop(*correlation)),
            CoreEffect::SteerTurn { correlation, .. } => Some(Self::Steer(*correlation)),
            CoreEffect::SubmitApprovalDecision { correlation } => {
                Some(Self::ApprovalDecision(correlation.clone()))
            }
            _ => None,
        })
    }

    /// 이 입력이 명령의 효과의 완료이면 그 효과와, 효과가 실패했을 때 댄 원인이다 (성공이면 `None`).
    pub(super) fn of_completion(input: &CoreInput) -> Option<(Self, Option<String>)> {
        let CoreInput::EffectCompleted(completion) = input else {
            return None;
        };
        match completion {
            CoreEffectCompletion::SessionRenamed {
                correlation,
                result,
            } => Some((Self::Rename(correlation.clone()), failure_of(result))),
            CoreEffectCompletion::PlanningWorkspaceResetCompleted {
                correlation,
                result,
            } => Some((Self::Reset(correlation.clone()), failure_of(result))),
            CoreEffectCompletion::StopRequestAttemptCompleted {
                correlation,
                attempt: StopRequestAttempt::Initial,
                result,
            } => Some((Self::Stop(*correlation), failure_of(result))),
            CoreEffectCompletion::TurnSteered {
                correlation,
                result,
            } => Some((Self::Steer(*correlation), failure_of(result))),
            CoreEffectCompletion::ApprovalDecisionSubmitted {
                correlation,
                result,
            } => Some((
                Self::ApprovalDecision(correlation.clone()),
                failure_of(result),
            )),
            _ => None,
        }
    }
}

/// 효과의 결과에서 실패의 원인만 꺼낸다. 완료 기록의 `detail` 이 된다 (plan 18 K4-9a).
fn failure_of<T>(result: &Result<T, String>) -> Option<String> {
    result.as_ref().err().cloned()
}

/// 효과의 완료를 기다리는 명령 하나이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OpenCommandOperation {
    pub(super) op_id: i64,
    pub(super) thread_id: String,
}

/// 효과의 완료를 기다리는 명령들이다. 커널이 도는 동안만 있다. 커널을 다시 조립하면 그 연산은 `lost` 로 닫힌다.
#[derive(Debug, Clone, Default)]
pub(super) struct CommandOperationTracker {
    open: Vec<(CommandEffectKey, OpenCommandOperation)>,
}

impl CommandOperationTracker {
    pub(super) fn track(&mut self, key: CommandEffectKey, operation: OpenCommandOperation) {
        self.open.push((key, operation));
    }

    /// 그 효과를 기다리던 명령을 꺼낸다. 명령에서 오지 않은 효과(TUI 의 명령, 커널 안의 다시 보내기)는 `None` 이다.
    pub(super) fn complete(&mut self, key: &CommandEffectKey) -> Option<OpenCommandOperation> {
        let index = self.open.iter().position(|(open, _)| open == key)?;
        Some(self.open.remove(index).1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operation(op_id: i64) -> OpenCommandOperation {
        OpenCommandOperation {
            op_id,
            thread_id: "thread-1".to_string(),
        }
    }

    #[test]
    fn an_effect_and_its_completion_name_the_same_command() {
        let stop = StopRequestCorrelation::new(3, None);
        let effect = CoreEffect::RequestStopAllSessions {
            correlation: stop,
            attempt: StopRequestAttempt::Initial,
        };
        let key = CommandEffectKey::of_effects(&[effect]).expect("the stop effect is a key");
        let completion = |attempt, result| {
            CoreInput::EffectCompleted(CoreEffectCompletion::StopRequestAttemptCompleted {
                correlation: stop,
                attempt,
                result,
            })
        };
        assert_eq!(
            CommandEffectKey::of_completion(&completion(StopRequestAttempt::Initial, Ok(()))),
            Some((key.clone(), None))
        );
        assert_eq!(
            CommandEffectKey::of_completion(&completion(
                StopRequestAttempt::Initial,
                Err("no app-server".to_string())
            )),
            Some((key, Some("no app-server".to_string())))
        );
        // The kernel sends the stop again after the turn started. That is not the end of the command.
        assert_eq!(
            CommandEffectKey::of_completion(&completion(
                StopRequestAttempt::AfterTurnStarted,
                Ok(())
            )),
            None
        );
        assert_eq!(
            CommandEffectKey::of_effects(&[CoreEffect::RequestStopAllSessions {
                correlation: stop,
                attempt: StopRequestAttempt::AfterTurnStarted,
            }]),
            None
        );
    }

    #[test]
    fn a_completion_ends_its_command_once() {
        let mut tracker = CommandOperationTracker::default();
        let turn = super::super::TurnSubmissionCorrelation::new(1);
        let first = CommandEffectKey::Steer(TurnSteerCorrelation::new(1, turn));
        let second = CommandEffectKey::Steer(TurnSteerCorrelation::new(2, turn));
        tracker.track(first.clone(), operation(7));
        tracker.track(second.clone(), operation(8));
        assert_eq!(tracker.complete(&second), Some(operation(8)));
        assert_eq!(tracker.complete(&second), None, "the command ended already");
        assert_eq!(tracker.complete(&first), Some(operation(7)));
        // An effect that no recorded command made (a TUI command) ends no operation.
        assert_eq!(
            tracker.complete(&CommandEffectKey::Stop(StopRequestCorrelation::new(
                1, None
            ))),
            None
        );
    }
}
