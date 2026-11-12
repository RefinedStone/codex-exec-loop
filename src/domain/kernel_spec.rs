//! 커널 스펙의 domain 쪽 표이다 (plan 18 K4-1, 설계 `docs/plan/18-k4-command-spec.md` §3.1).
//!
//! 명령의 종류와 성질, 기록의 종류를 여기서 한 번 정한다. 커널은 이 표로 명령을 기록하고 검사한다. 전송 모양(serde
//! 타입)과 조회·이벤트·오류의 표는 inbound adapter `kernel_protocol` 에 있고, 그 adapter 가 두 표와 스키마를 묶어
//! `describe` 의 답을 만든다. 설명(`summary`)은 클라이언트와 에이전트가 읽으므로 영어로 쓴다.

use schemars::JsonSchema;
use serde::Serialize;

use crate::domain::kernel_event::CommandRejectionReason;

/*
 * 이름 목록 하나를 한 번만 적는다. 줄 하나가 값, 전송되는 이름, 한 문장 설명이다. enum 과 `ALL`, `name`, `summary` 를
 * 함께 만든다. 그래서 값을 더하면 목록과 설명이 같이 생기고, 이 enum 을 `match` 하는 표는 채울 때까지 컴파일되지
 * 않는다.
 */
macro_rules! spec_names {
    (
        $(#[$meta:meta])*
        $visibility:vis enum $name:ident {
            $($variant:ident => $wire:literal, $summary:literal;)+
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        $visibility enum $name {
            $($variant,)+
        }

        impl $name {
            /// 모든 값이다. 적은 차례대로이다.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)+];

            /// 전송되는 이름이다.
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire,)+
                }
            }

            /// 한 문장 설명이다. 스펙이 그대로 싣는다.
            pub const fn summary(self) -> &'static str {
                match self {
                    $(Self::$variant => $summary,)+
                }
            }
        }
    };
}
pub(crate) use spec_names;

spec_names! {
    /// 클라이언트가 커널에 보내는 명령의 종류이다. 전송 enum `KernelCommand` 가 이 값으로 바뀐다.
    pub enum KernelCommandKind {
        LoadSessions => "load_sessions",
            "Load the session list of the workspace. The result arrives as the `sessions` event.";
        RenameSession => "rename_session",
            "Rename a session (thread).";
        SubmitPrompt => "submit_prompt",
            "Send a prompt to a thread. The kernel prepares it with planning context and submits a turn. Progress arrives as `prompt_submission` events.";
        SteerTurn => "steer_turn",
            "Add a prompt to the turn that is running on a thread.";
        StopAll => "stop_all",
            "Request a stop of every running turn.";
        DecideApproval => "decide_approval",
            "Accept or decline a pending approval request.";
        SetAutoFollowMaxTurns => "set_auto_follow_max_turns",
            "Set how many automatic follow-up turns may run after a manual turn.";
        PauseAutoFollow => "pause_auto_follow",
            "Pause automatic follow-up turns.";
        ResetPlanning => "reset_planning",
            "Delete planning data (the queue, the directions, or all of it).";
        RefreshPlanning => "refresh_planning",
            "Read the planning queue again. The result arrives as the `planning` event.";
    }
}

/// 명령이 바꾸는 상태의 구역이다. 권한 범위이다 (plan 18 §4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CommandScope {
    Session,
    Thread,
    Approval,
    AutoFollow,
    Planning,
}

/// 명령의 부작용이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CommandEffect {
    /// 상태를 바꾸지 않는다. 판단 사건 기록에 남지 않고 영수증도 없다.
    Read,
    /// 상태를 바꾼다. 기록에 남는다.
    Write,
    /// 지운다. 기록에 남고, 입력에 `confirm: true` 가 있어야 한다.
    Destructive,
}

/// 같은 명령을 두 번 보내면 어떻게 되는가이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CommandIdempotency {
    /// 같은 입력은 같은 상태가 된다. 다시 보내도 된다.
    Natural,
    /// 다시 보내면 한 번 더 실행된다. `idempotency_key` 로 재전송을 가린다.
    Keyed,
}

/// 명령 하나의 성질이다. 종류마다 하나이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    pub scope: CommandScope,
    pub effect: CommandEffect,
    pub idempotency: CommandIdempotency,
    /// 커널이 이 명령을 바로 거절할 때 낼 수 있는 이유이다.
    pub rejections: &'static [CommandRejectionReason],
}

impl KernelCommandKind {
    pub const fn spec(self) -> CommandSpec {
        use CommandEffect::{Destructive, Read, Write};
        use CommandIdempotency::{Keyed, Natural};
        use CommandRejectionReason as Reason;
        let (scope, effect, idempotency, rejections): (_, _, _, &'static [Reason]) = match self {
            Self::LoadSessions => (CommandScope::Session, Read, Natural, &[]),
            Self::RenameSession => (CommandScope::Session, Write, Natural, &[]),
            Self::SubmitPrompt => (
                CommandScope::Thread,
                Write,
                Keyed,
                &[
                    Reason::Busy,
                    Reason::PreparationActive,
                    Reason::UnknownThread,
                    Reason::ParallelMode,
                    Reason::ParallelModeUnknown,
                ],
            ),
            Self::SteerTurn => (
                CommandScope::Thread,
                Write,
                Keyed,
                &[Reason::SteerInFlight, Reason::NoMatchingTurn],
            ),
            Self::StopAll => (CommandScope::Thread, Write, Natural, &[]),
            Self::DecideApproval => (
                CommandScope::Approval,
                Write,
                Keyed,
                &[Reason::DecisionInFlight, Reason::NoMatchingApproval],
            ),
            Self::SetAutoFollowMaxTurns => (CommandScope::AutoFollow, Write, Natural, &[]),
            Self::PauseAutoFollow => (CommandScope::AutoFollow, Write, Natural, &[]),
            Self::ResetPlanning => (CommandScope::Planning, Destructive, Natural, &[]),
            Self::RefreshPlanning => (CommandScope::Planning, Read, Natural, &[]),
        };
        CommandSpec {
            scope,
            effect,
            idempotency,
            rejections,
        }
    }

    /// 판단 사건 기록에 남는 명령인가이다. 읽기 명령은 남지 않는다.
    pub const fn is_recorded(self) -> bool {
        !matches!(self.spec().effect, CommandEffect::Read)
    }
}

spec_names! {
    /// 판단 사건 기록의 종류이다. `KernelEvent::kind` 가 이 이름을 쓴다.
    pub enum KernelRecordKind {
        ThreadPromptQueued => "thread_prompt_queued",
            "The kernel kept a prepared prompt in its queue. It submits the prompt once when it is idle.";
        ThreadPromptSettled => "thread_prompt_settled",
            "A queued prompt ended: submitted, handed to parallel slots, dropped because its task finished, or rejected again.";
        ThreadPromptRejected => "thread_prompt_rejected",
            "The kernel rejected a thread prompt before preparation. No planning task was made.";
        TurnSubmitted => "turn_submitted",
            "The kernel accepted a turn submission (manual, prepared, or automatic follow-up).";
        TurnStarted => "turn_started",
            "The app-server started a turn.";
        TurnEnded => "turn_ended",
            "A turn ended. `outcome` is present when the kernel saw how it ended.";
        PostTurnRouted => "post_turn_routed",
            "The kernel chose what follows a completed turn.";
        ApprovalRequested => "approval_requested",
            "A turn asked for an approval. The request content is not recorded.";
        ApprovalResolved => "approval_resolved",
            "An approval ended: accepted, declined, timed out, withdrawn, or lost at a kernel restart.";
        CommandReceived => "command_received",
            "The kernel received a client command. `op_id` is the sequence of this record.";
        CommandRejected => "command_rejected",
            "The kernel rejected a received command at once.";
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_snake_case() {
        let commands = KernelCommandKind::ALL.iter().map(|kind| kind.name());
        let records = KernelRecordKind::ALL.iter().map(|kind| kind.name());
        for names in [commands.collect::<Vec<_>>(), records.collect::<Vec<_>>()] {
            let mut unique = names.clone();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(unique.len(), names.len(), "{names:?}");
            for name in names {
                assert!(
                    name.chars()
                        .all(|character| character.is_ascii_lowercase() || character == '_'),
                    "{name}"
                );
            }
        }
    }

    #[test]
    fn read_commands_are_not_recorded_and_carry_no_rejection() {
        // The recording rule is derived from the effect. It is not a second list.
        let unrecorded = KernelCommandKind::ALL
            .iter()
            .filter(|kind| !kind.is_recorded())
            .map(|kind| kind.name())
            .collect::<Vec<_>>();
        assert_eq!(unrecorded, ["load_sessions", "refresh_planning"]);
        for kind in KernelCommandKind::ALL {
            if !kind.is_recorded() {
                assert!(kind.spec().rejections.is_empty(), "{}", kind.name());
            }
        }
    }

    #[test]
    fn every_reason_belongs_to_a_command() {
        for reason in CommandRejectionReason::ALL {
            assert!(
                KernelCommandKind::ALL
                    .iter()
                    .any(|kind| kind.spec().rejections.contains(reason)),
                "{} is not a rejection of any command",
                reason.name()
            );
        }
    }
}
