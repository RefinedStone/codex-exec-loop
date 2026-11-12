//! 커널 스펙의 domain 쪽 표이다 (plan 18 K4-1, 설계 `docs/plan/18-k4-command-spec.md` §3.1).
//!
//! 명령의 종류와 성질, 이유 코드, 기록의 종류를 여기서 한 번 정한다. 커널은 이 표로 명령을 기록할지 정한다
//! (`is_recorded`). 효과와 전제 조건을 커널이 검사하는 일은 K4-5 이다. 전송 모양(serde 타입)과 조회·이벤트·오류의
//! 표는 inbound adapter `kernel_protocol` 에 있고, 그 adapter 가 두 표와 스키마를 묶어 `describe` 의 답을 만든다.
//! 설명(`summary`)은 클라이언트와 에이전트가 읽으므로 영어로 쓴다.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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

spec_names! {
    /// 명령이 바꾸는 상태의 구역이다. 권한 범위이다 (plan 18 §4.4).
    #[derive(Serialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum CommandScope {
        Session => "session", "The session list and session names.";
        Thread => "thread", "The turns of a thread.";
        Approval => "approval", "Pending approval requests.";
        AutoFollow => "auto_follow", "The automatic follow-up settings.";
        Planning => "planning", "The planning queue and directions.";
    }
}

spec_names! {
    /// 명령의 부작용이다.
    #[derive(Serialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum CommandEffect {
        Read => "read",
            "Changes no state. The command is not written to the decision log and has no operation receipt.";
        Write => "write",
            "Changes state. The command is written to the decision log.";
        Destructive => "destructive",
            "Deletes data. The command is written to the decision log and its input needs `confirm: true`.";
    }
}

spec_names! {
    /// 같은 명령을 두 번 보내면 어떻게 되는가이다.
    #[derive(Serialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum CommandIdempotency {
        Natural => "natural",
            "The same input gives the same state. Sending it again is safe.";
        Keyed => "keyed",
            "Sending it again runs it again. Send `idempotency_key` so that the kernel tells a resend apart.";
    }
}

spec_names! {
    /*
     * 이유 코드이다 (plan 18 K4-3, 설계 §3.3). 커널이 명령을 바로 거절한 이유(명령 답의 `rejection`, 기록
     * `command_rejected`), thread 지정 프롬프트를 준비 전에 거절한 이유(기록 `thread_prompt_rejected`), 그 프롬프트가
     * 턴 없이 멈춘 이유(`prompt_submission` 이벤트)가 모두 이 목록 하나에서 온다. 전송되는 글자는 이전의 세 enum 과
     * 같다. 다음 행동(`next`)은 전송 표가 붙인다 (`kernel_protocol`).
     */
    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum ReasonCode {
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
        ParallelModeStarted => "parallel_mode_started",
            "Parallel mode started while the prompt was prepared. The planning task stays queued and parallel slots take it. Do not send the prompt again.";
        TurnStartedDuringPreparation => "turn_started_during_preparation",
            "Another turn started while the prompt was prepared. The planning task stays queued and the kernel submits the prompt once when it is idle. The result arrives as a `prompt_submission` event with the same `task_id`. Do not send the prompt again.";
        StopPending => "stop_pending",
            "A stop request is still settling. The planning task stays queued and the kernel submits the prompt once when it is idle. The result arrives as a `prompt_submission` event with the same `task_id`. Do not send the prompt again.";
        Unavailable => "unavailable",
            "The thread cannot take a prompt now. The planning task stays queued and the kernel submits the prompt once when it is idle. The result arrives as a `prompt_submission` event with the same `task_id`. Do not send the prompt again.";
        Cancelled => "cancelled",
            "The prompt preparation was cancelled. No planning task was made.";
        TextMismatch => "text_mismatch",
            "The prepared prompt does not match the sent text. No planning task was made.";
        BootstrapReviewRequired => "bootstrap_review_required",
            "A person must review the planning draft first. `message` names the draft. No planning task was made.";
        PlanningRefused => "planning_refused",
            "Planning did not take the prompt. `message` gives the cause. No planning task was made.";
        TaskFinished => "task_finished",
            "The kernel dropped the queued prompt because its planning task is already done or cancelled.";
        RejectedAgain => "rejected_again",
            "The kernel sent the queued prompt once more and the thread rejected it again. The kernel does not send it again. The planning task stays queued.";
    }
}

spec_names! {
    /// 명령이 언제 끝나는가이다 (plan 18 K4-4, 설계 §3.4). `operation` 조회의 `state` 가 이 규칙으로 끝을 안다.
    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum CommandCompletion {
        Accepted => "accepted",
            "The command ends when the kernel accepts it. Its operation state is `done` at once.";
        Records => "records",
            "The command ends with a later record that carries its `op_id`. Read the `operation` query.";
        Unreported => "unreported",
            "The kernel that received the command did not report its end. Its operation state stays `accepted`. No command of this version has this rule; it is the rule of commands that an older kernel received.";
    }
}

spec_names! {
    /// 연산의 상태이다 (plan 18 K4-4). 기록에서 계산한다. 따로 저장하지 않는다.
    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum OperationState {
        Accepted => "accepted",
            "The kernel accepted the command and no later record of it exists yet.";
        Running => "running",
            "The command is in progress: a prompt is queued or its turn runs.";
        Done => "done",
            "The command ended.";
        Rejected => "rejected",
            "The kernel rejected the command at once. The reason is in the `command_rejected` record.";
        Failed => "failed",
            "The preparation or the turn of the command failed.";
        Lost => "lost",
            "The kernel was built again while the command was open. Its work did not continue.";
        Unknown => "unknown",
            "No command record has this id. The id is wrong or the record is older than the retention limit.";
    }
}

impl OperationState {
    /// 더 바뀌지 않는 상태인가이다.
    pub const fn is_end(self) -> bool {
        !matches!(self, Self::Accepted | Self::Running)
    }
}

/// 명령 하나의 성질이다. 종류마다 하나이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    pub scope: CommandScope,
    pub effect: CommandEffect,
    pub idempotency: CommandIdempotency,
    /// 커널이 이 명령을 바로 거절할 때 낼 수 있는 이유이다.
    pub rejections: &'static [ReasonCode],
    pub completion: CommandCompletion,
}

impl KernelCommandKind {
    pub const fn spec(self) -> CommandSpec {
        use CommandEffect::{Destructive, Read, Write};
        use CommandIdempotency::{Keyed, Natural};
        use ReasonCode as Reason;
        let completion = match self {
            Self::LoadSessions
            | Self::RefreshPlanning
            | Self::SetAutoFollowMaxTurns
            | Self::PauseAutoFollow => CommandCompletion::Accepted,
            Self::SubmitPrompt
            | Self::RenameSession
            | Self::SteerTurn
            | Self::StopAll
            | Self::DecideApproval
            | Self::ResetPlanning => CommandCompletion::Records,
        };
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
            completion,
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
        CommandCompleted => "command_completed",
            "A command ended without a turn record. For `submit_prompt`: its preparation failed, its work went to parallel slots, or its turn submission closed before a turn started. For the other commands: the request of the command returned (`done` or `failed`), or the command did nothing (`failed`). For every command: the kernel was built again (`lost`).";
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
    fn reason_codes_keep_the_text_of_the_three_old_lists() {
        // plan 18 K4-3: one list replaced `CommandRejectionReason`, `ThreadPromptRejection`,
        // and `PromptSubmissionReason`. Records and v0 clients read the same text, and serde must agree with the table.
        let names = ReasonCode::ALL
            .iter()
            .map(|reason| reason.name())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "steer_in_flight",
                "no_matching_turn",
                "decision_in_flight",
                "no_matching_approval",
                "busy",
                "preparation_active",
                "unknown_thread",
                "parallel_mode",
                "parallel_mode_unknown",
                "parallel_mode_started",
                "turn_started_during_preparation",
                "stop_pending",
                "unavailable",
                "cancelled",
                "text_mismatch",
                "bootstrap_review_required",
                "planning_refused",
                "task_finished",
                "rejected_again",
            ]
        );
        for reason in ReasonCode::ALL {
            assert_eq!(
                serde_json::to_value(reason).expect("a reason should serialize"),
                reason.name()
            );
        }
    }
}
