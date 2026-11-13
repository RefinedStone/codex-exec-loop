//! 이유와 오류 뒤의 다음 행동이다 (plan 18 K4-3, 설계 `docs/plan/18-k4-command-spec.md` §3.2, §3.3).
//!
//! `next` 는 글이 아니라 데이터이다. 클라이언트가 차례대로 그대로 실행할 수 있는 단계만 둔다. 이유 코드는 domain 의
//! 표이고, 단계가 가리키는 조회와 이벤트의 이름은 이 adapter 의 표이므로 이 표는 adapter 에 둔다. 빠짐없는 `match`
//! 이므로 이유나 오류를 더하면 여기를 채울 때까지 컴파일되지 않는다.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{KernelErrorKind, KernelEventKind, KernelMethodKind, KernelQueryKind};
use crate::domain::kernel_spec::{KernelCommandKind, ReasonCode};

/// 이유가 `next` 로 기다리라고 할 때의 시간이다. 그 일의 끝을 알리는 이벤트가 없는 경우에만 쓴다.
const SHORT_WAIT_MILLISECONDS: u64 = 1_000;

/// 다음 행동의 한 단계이다. 목록의 차례대로 한다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum NextStep {
    /// 그 이름의 이벤트가 다음에 올 때까지 기다린다. 조건은 없다. 온 뒤에 다음 단계를 한다.
    WaitFor { event: String },
    /// 그만큼 기다린다. 그 일의 끝을 알리는 이벤트가 없을 때 쓴다.
    Wait { milliseconds: u64 },
    /// 그 조회로 지금 값을 읽는다.
    Query { name: String },
    /// 그 명령을 보낸다.
    Command { name: String },
    /// 그 메서드를 보낸다 (예: `akra/kernel/attach`).
    Method { name: String },
    /// 같은 요청을 다시 보낸다. 명령이면 새 멱등 키로 보낸다.
    Resend,
    /// 사람이 정해야 한다.
    AskUser,
    /// 다시 하지 않는다.
    Stop,
}

impl NextStep {
    fn wait_for(event: KernelEventKind) -> Self {
        Self::WaitFor {
            event: event.name().to_string(),
        }
    }

    fn query(query: KernelQueryKind) -> Self {
        Self::Query {
            name: query.name().to_string(),
        }
    }

    fn command(command: KernelCommandKind) -> Self {
        Self::Command {
            name: command.name().to_string(),
        }
    }

    fn method(method: KernelMethodKind) -> Self {
        Self::Method {
            name: method.name().to_string(),
        }
    }

    fn short_wait() -> Self {
        Self::Wait {
            milliseconds: SHORT_WAIT_MILLISECONDS,
        }
    }
}

/// 그 이유 뒤의 다음 행동이다. 명령 답의 `rejection.next`, `prompt_submission` 의 `next`, 스펙의 `reasons[].next` 가
/// 모두 이 값이다.
pub fn reason_next(reason: ReasonCode) -> Vec<NextStep> {
    use NextStep as Step;
    match reason {
        ReasonCode::KernelNotRunning => vec![Step::AskUser],
        // 진행 중인 steer 의 끝을 알리는 이벤트는 없다.
        ReasonCode::SteerInFlight => vec![Step::short_wait(), Step::Resend],
        ReasonCode::NoMatchingTurn => vec![Step::query(KernelQueryKind::ThreadActivity)],
        // 다른 결정이 그 승인을 닫았을 수 있다. 같은 결정을 다시 보내지 않고 승인이 남았는지 본다.
        ReasonCode::DecisionInFlight => vec![
            Step::short_wait(),
            Step::query(KernelQueryKind::ThreadActivity),
        ],
        ReasonCode::NoMatchingApproval => vec![Step::query(KernelQueryKind::ThreadActivity)],
        ReasonCode::Busy => vec![
            Step::wait_for(KernelEventKind::ThreadActivity),
            Step::Resend,
        ],
        // 이 거절도 `prompt_submission` 이벤트를 낸다. 그 이벤트를 기다리면 자기 거절에서 바로 풀린다.
        ReasonCode::PreparationActive => vec![Step::short_wait(), Step::Resend],
        ReasonCode::UnknownThread => {
            vec![Step::command(KernelCommandKind::LoadSessions), Step::Resend]
        }
        // 병렬 모드를 끄는 명령은 아직 프로토콜에 없다 (K4-9b). 끌지는 사람이 정한다.
        ReasonCode::ParallelMode => vec![Step::AskUser],
        ReasonCode::ParallelModeUnknown => vec![Step::short_wait(), Step::Resend],
        ReasonCode::ParallelModeStarted => vec![Step::Stop],
        // 커널이 쉴 때 한 번 다시 내고 그 결과를 `prompt_submission` 으로 알린다. 단계에는 조건이 없으므로 클라이언트가
        // 그 이벤트의 `task_id` 로 자기 프롬프트인지 가린다 (이유의 설명이 그것을 적는다).
        ReasonCode::TurnStartedDuringPreparation
        | ReasonCode::StopPending
        | ReasonCode::Unavailable => vec![Step::wait_for(KernelEventKind::PromptSubmission)],
        ReasonCode::Cancelled | ReasonCode::TextMismatch => vec![Step::Stop],
        ReasonCode::BootstrapReviewRequired | ReasonCode::PlanningRefused => vec![Step::AskUser],
        ReasonCode::TaskFinished | ReasonCode::RejectedAgain => vec![Step::Stop],
    }
}

/// 그 JSON-RPC 오류 뒤의 다음 행동이다. 스펙의 `errors[].next` 이다.
pub(crate) fn error_next(error: KernelErrorKind) -> Vec<NextStep> {
    use NextStep as Step;
    match error {
        // 입력이나 메서드를 고쳐야 한다. 같은 요청을 다시 보내도 같은 오류이다.
        KernelErrorKind::InvalidParams | KernelErrorKind::MethodNotFound => vec![Step::Stop],
        KernelErrorKind::NotAttached => {
            vec![Step::method(KernelMethodKind::Attach), Step::Resend]
        }
        KernelErrorKind::ConfirmationRequired => vec![Step::AskUser],
        KernelErrorKind::KernelUnavailable => vec![Step::AskUser],
        KernelErrorKind::KernelTimeout => vec![Step::query(KernelQueryKind::History)],
        KernelErrorKind::IdempotencyKeyReused => vec![Step::Resend],
        // 키 없이 보내면 실행되지만 재전송이 다시 실행된다. 그 위험을 받을지는 사람이 정한다.
        KernelErrorKind::IdempotencyUnavailable => vec![Step::AskUser],
    }
}
