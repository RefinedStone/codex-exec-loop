use serde::{Deserialize, Serialize};

use super::{IssueAutomationError, IssueTriageResult};

/// 수명 상태의 닫힌 목록이다. 보류/낡음/조회 실패는 별도 표시다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueAutomationState {
    Discovered,
    Ineligible,
    NeedsInfo,
    AwaitingDecision,
    Ready,
    Queued,
    Running,
    InReview,
    Done,
    Failed,
    Cancelled,
}

/// 결정 요청이 닫히면 돌아갈 직전 단계이다. 작업을 새로 시작할 허가는 아니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueDecisionResumePhase {
    Discovered,
    Ready,
    Running,
    InReview,
    Failed,
}

impl IssueDecisionResumePhase {
    pub fn state(self) -> IssueAutomationState {
        match self {
            Self::Discovered => IssueAutomationState::Discovered,
            Self::Ready => IssueAutomationState::Ready,
            Self::Running => IssueAutomationState::Running,
            Self::InReview => IssueAutomationState::InReview,
            Self::Failed => IssueAutomationState::Failed,
        }
    }

    fn from_state(state: IssueAutomationState) -> Option<Self> {
        match state {
            IssueAutomationState::Discovered => Some(Self::Discovered),
            IssueAutomationState::Ready => Some(Self::Ready),
            IssueAutomationState::Running => Some(Self::Running),
            IssueAutomationState::InReview => Some(Self::InReview),
            IssueAutomationState::Failed => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueAutomationFlag {
    Held,
    Stale,
    Unknown,
}

/// 세 표시는 동시에 참일 수 있다. 하나를 바꾸어도 다른 표시와 수명 상태는 유지한다.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueAutomationFlags {
    pub held: bool,
    pub stale: bool,
    pub unknown: bool,
}

impl IssueTriageResult {
    fn state(self) -> IssueAutomationState {
        match self {
            Self::Ineligible => IssueAutomationState::Ineligible,
            Self::NeedsInfo => IssueAutomationState::NeedsInfo,
            Self::Ready => IssueAutomationState::Ready,
        }
    }
}

/// 이미 확인한 사실/결정에 따라 장부를 바꾸는 명령이다. 외부 행동이나 실행 허가를 내지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueTransition {
    Assess(IssueTriageResult),
    Reassess,
    AwaitDecision,
    ResumeDecision,
    /// 사람의 재시도 결정이 확인된 뒤 쓴다. 일반 재판정과 달리 Failed에서 나올 수 있다.
    RetryFailed,
    Queue,
    Start,
    OpenReview,
    Rework,
    /// 신뢰된 통합 완료 사실이다. 한도 소진 뒤 확인한 완료도 재실행 없이 반영한다.
    Complete,
    /// 일반 조회/CI 오류가 아니다. 실행 시도 한도가 소진된 사실이다.
    ExhaustAttempts,
    /// 실행 중인 작업의 취소 확인은 상위 경계의 책임이다. 외부 이슈의 닫힘을 직접 뜻하지 않는다.
    Cancel,
}

pub(super) fn validate_resume(
    state: IssueAutomationState,
    resume: Option<IssueDecisionResumePhase>,
) -> Result<(), IssueAutomationError> {
    if (state == IssueAutomationState::AwaitingDecision) != resume.is_some() {
        return Err(IssueAutomationError::InvalidDecisionResume);
    }
    Ok(())
}

pub(super) fn next_state(
    state: IssueAutomationState,
    resume: Option<IssueDecisionResumePhase>,
    transition: IssueTransition,
) -> Result<(IssueAutomationState, Option<IssueDecisionResumePhase>), IssueAutomationError> {
    use IssueAutomationState as State;
    use IssueTransition as Transition;

    validate_resume(state, resume)?;
    let refused = || IssueAutomationError::InvalidTransition {
        from: state,
        transition,
    };
    let plain = |next| Ok((next, None));
    match transition {
        Transition::Assess(result) => {
            let next = result.state();
            if state == State::Discovered || state == next {
                plain(next)
            } else {
                Err(refused())
            }
        }
        Transition::Reassess => match state {
            State::Discovered | State::Ineligible | State::NeedsInfo | State::Ready => {
                plain(State::Discovered)
            }
            State::AwaitingDecision
                if matches!(
                    resume,
                    Some(IssueDecisionResumePhase::Discovered | IssueDecisionResumePhase::Ready)
                ) =>
            {
                plain(State::Discovered)
            }
            _ => Err(refused()),
        },
        Transition::AwaitDecision => {
            if state == State::AwaitingDecision {
                return Ok((state, resume));
            }
            let origin = IssueDecisionResumePhase::from_state(state).ok_or_else(refused)?;
            Ok((State::AwaitingDecision, Some(origin)))
        }
        Transition::ResumeDecision => {
            if state != State::AwaitingDecision {
                return Err(refused());
            }
            let origin = resume.ok_or(IssueAutomationError::InvalidDecisionResume)?;
            plain(origin.state())
        }
        Transition::RetryFailed => {
            if state == State::Failed
                || (state == State::AwaitingDecision
                    && resume == Some(IssueDecisionResumePhase::Failed))
            {
                plain(State::Discovered)
            } else {
                Err(refused())
            }
        }
        Transition::Queue if matches!(state, State::Ready | State::Queued) => plain(State::Queued),
        Transition::Start if matches!(state, State::Queued | State::Running) => {
            plain(State::Running)
        }
        Transition::OpenReview if matches!(state, State::Running | State::InReview) => {
            plain(State::InReview)
        }
        Transition::Rework if matches!(state, State::InReview | State::Running) => {
            plain(State::Running)
        }
        Transition::Complete if matches!(state, State::InReview | State::Failed | State::Done) => {
            plain(State::Done)
        }
        Transition::ExhaustAttempts
            if matches!(state, State::Running | State::InReview | State::Failed) =>
        {
            plain(State::Failed)
        }
        Transition::Cancel if state != State::Done => plain(State::Cancelled),
        _ => Err(refused()),
    }
}

/// 저장 경계에서도 같은 한 단계 전이표를 적용한다. DTO 복원으로 금지된 점프를 우회하지 못한다.
pub(super) fn validate_state_replacement(
    current: IssueAutomationState,
    resume: Option<IssueDecisionResumePhase>,
    next: IssueAutomationState,
    next_resume: Option<IssueDecisionResumePhase>,
) -> Result<(), IssueAutomationError> {
    validate_resume(current, resume)?;
    validate_resume(next, next_resume)?;
    if (current, resume) == (next, next_resume) {
        return Ok(());
    }
    let transitions = [
        IssueTransition::Assess(IssueTriageResult::Ineligible),
        IssueTransition::Assess(IssueTriageResult::NeedsInfo),
        IssueTransition::Assess(IssueTriageResult::Ready),
        IssueTransition::Reassess,
        IssueTransition::AwaitDecision,
        IssueTransition::ResumeDecision,
        IssueTransition::RetryFailed,
        IssueTransition::Queue,
        IssueTransition::Start,
        IssueTransition::OpenReview,
        IssueTransition::Rework,
        IssueTransition::Complete,
        IssueTransition::ExhaustAttempts,
        IssueTransition::Cancel,
    ];
    if transitions
        .into_iter()
        .any(|transition| next_state(current, resume, transition).ok() == Some((next, next_resume)))
    {
        return Ok(());
    }
    Err(IssueAutomationError::InvalidStateReplacement {
        from: current,
        to: next,
    })
}
