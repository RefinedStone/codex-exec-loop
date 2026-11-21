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
    /// The state that must be available to execute the command.
    #[derive(Serialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum CommandState {
        Store => "store", "Stored repository data. A command-only kernel can execute this command.";
        Runtime => "runtime", "Live kernel state. Start the Gateway before sending this command.";
    }
}

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
        CreateTask => "create_task",
            "Add a planning task as the operator. The answer carries the new task id and the new planning revision.";
        UpdateTask => "update_task",
            "Change a planning task as the operator. The title, the priority reason, and the references are replaced; a missing direction, description, status, or priority keeps its value. `expected_planning_revision` is the revision that the sender read; a different current revision rejects the command.";
        DeleteTask => "delete_task",
            "Delete a planning task and the references to it. `expected_planning_revision` is the revision that the sender read; a different current revision rejects the command.";
        CreateDirection => "create_direction",
            "Add a planning direction as the operator. The answer carries the new direction id and the new planning revision.";
        UpdateDirection => "update_direction",
            "Replace a planning direction as the operator. A missing summary becomes the title and a missing state becomes `active`. `expected_planning_revision` is the revision that the sender read; a different current revision rejects the command.";
        DeleteDirection => "delete_direction",
            "Delete a planning direction, its tasks, and the references to them. The default direction is kept. `expected_planning_revision` is the revision that the sender read; a different current revision rejects the command.";
        SetIssueAutomationMode => "set_issue_automation_mode",
            "Persist the issue collection mode: off or observe. Raising the mode requires Desktop human confirmation; lowering it does not. The answer follows the ledger commit.";
        SetIssueAutonomy => "set_issue_autonomy",
            "Persist the runtime issue autonomy level: off, observe, or propose within the file policy cap. Raising the level requires a current Desktop human confirmation. Reuse command_id and the idempotency key to recover the same result.";
        AnswerIssueDecision => "answer_issue_decision",
            "Record an answer to one open issue decision. Only a verified Desktop confirmation produces a human answer; human gates reject unverified answers. The current basis is checked atomically; an outdated request closes as superseded without applying its answer. Reuse the idempotency key to recover the same receipt.";
        HoldIssue => "hold_issue",
            "Hold one issue work order and revoke its unused work permits. Running turns are preserved. Reuse command_id to recover the same intervention receipt.";
        ReleaseIssue => "release_issue",
            "Release one issue hold. Previously revoked permits stay revoked until fresh approval and gate checks. Reuse command_id to recover the same intervention receipt.";
        SteerIssue => "steer_issue",
            "Queue guidance before execution, or steer the linked running thread once. An unknown delivery is not retried automatically. Reuse command_id to recover the same intervention receipt.";
        TakeOverIssue => "take_over_issue",
            "Take over one issue work order. Automatic continuation, delivery, and cleanup are disabled while its branch, worktree, and thread are preserved.";
        RetriageIssue => "retriage_issue",
            "Invalidate one issue assessment and revoke its existing work permits so that the current issue can be assessed again.";
        CancelIssueWork => "cancel_issue_work",
            "Cancel one issue work order and its task lineage. Requires confirm: true and revokes all unused work permits.";
        TrackIssue => "track_issue",
            "Track a stable issue reference in the attached workspace. Tracking does not grant work permits.";
        UntrackIssue => "untrack_issue",
            "Untrack one issue and stop its automatic work. Existing branches, worktrees, and threads are preserved.";
        SyncIssues => "sync_issues",
            "Request an immediate issue collection cycle. Concurrent requests coalesce. The command accepts scheduling; the issues event reports a later ledger revision. Off mode rejects the request.";
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
        Issue => "issue", "Issue collection settings and observations.";
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
        KernelUnreachable => "kernel_unreachable",
            "No kernel that the Akra Gateway runs can take the command. `message` gives the cause: the Gateway is not running, or another process (a TUI) runs the kernel of this repository.";
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
        ForeignWorkspace => "foreign_workspace",
            "The thread runs in a directory outside the workspace of this kernel client. The kernel makes no planning task for it. Send the prompt through the kernel of the thread's repository.";
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
        PlanningRevisionStale => "planning_revision_stale",
            "The planning data changed after the sender read it: `expected_planning_revision` is not the current revision. Nothing was changed. `message` gives both revisions.";
        PlanningLocked => "planning_locked",
            "Another change or parallel work holds the planning data that the command needs. Nothing was changed. `message` names the holder.";
        NotYetObserved => "not_yet_observed",
            "The issue has no current provider observation.";
        SourceUnavailable => "source_unavailable",
            "The current issue could not be read; missing data is not proof of eligibility.";
        MetadataIncomplete => "metadata_incomplete",
            "Required source metadata is incomplete or unavailable.";
        DisclosurePolicyStale => "disclosure_policy_stale",
            "The shared observation was produced under an older disclosure policy.";
        SelectionNotMatched => "selection_not_matched",
            "The issue does not match the configured selection.";
        IssueClosed => "issue_closed",
            "The provider reports that the issue is closed.";
        UntrustedActor => "untrusted_actor",
            "No trusted author or qualifying current label actor establishes trust.";
        AlreadyClaimed => "already_claimed",
            "The issue already has an assignee or a linked pull request.";
        TitleMissing => "title_missing",
            "The current shared issue has no usable title.";
        RequiredSectionMissing => "required_section_missing",
            "The current shared issue does not satisfy a required section.";
        CriteriaMissing => "criteria_missing",
            "The issue has no acceptance criteria.";
        CriteriaNotList => "criteria_not_list",
            "The acceptance criteria cannot be read as a supported Markdown list.";
        IssueTooLarge => "issue_too_large",
            "The shared issue exceeds the deterministic assessment size bound.";
        UnsupportedContent => "unsupported_content",
            "The issue contains content that cannot be assessed deterministically.";
        DisclosureConfirmationRequired => "disclosure_confirmation_required",
            "Disclosure requires explicit user confirmation; withheld content is not assessed as shared text.";
        MaskedContent => "masked_content",
            "Masking prevents a complete deterministic assessment.";
        HighRisk => "high_risk",
            "High risk requires a person at every gate.";
        HeldOrManual => "held_or_manual",
            "The issue is held or explicitly marked for manual handling.";
        ProtectedPath => "protected_path",
            "The declared scope includes a protected repository path.";
        PolicyHumanRequired => "policy_human_required",
            "An additional policy rule requires a person at the affected gates.";
        HumanSelectionRequired => "human_selection_required",
            "An agent-created or unresolved issue requires a trusted user account to apply a current selection label.";
        BudgetExhausted => "budget_exhausted",
            "A zero static budget cap prevents an automatic start and requires a person to review the budget.";
        ModeOff => "mode_off",
            "The effective automation policy is off.";
        ObservationOnly => "observation_only",
            "This assessment is observational and creates no work or execution permission.";
        ExecutionGrantRequired => "execution_grant_required",
            "The execution grant boundary is not connected; descriptive gates do not authorize work.";
        CloseDisabled => "close_disabled",
            "Issue closing is not enabled by this assessment.";
        HumanConfirmationRequired => "human_confirmation_required",
            "This action requires a current Desktop human confirmation. A client name or confirmation boolean is not evidence. Read the current state and ask the user in Desktop.";
        DecisionSuperseded => "decision_superseded",
            "The decision basis differs from the current issue assessment. The request was closed without applying the submitted answer; read the issue for current open decisions.";
        IssueAutomationOff => "issue_automation_off",
            "Issue collection is off. No source read was scheduled. Read issue_automation and decide whether to enable observe before sending sync_issues again.";
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

/*
 * 커널이 요청에 JSON-RPC 오류로 답했다 (plan 18 K4-8). `code` 로 스펙의 `errors` 항목을 찾고, 그 항목이 다음 행동을
 * 가진다. 커널에 요청을 보내는 쪽(Gateway 클라이언트, 명령 전용 조립)이 만들고, 입구(CLI)가 출력에 싣는다.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelCallError {
    pub code: i64,
    pub message: String,
}

impl std::fmt::Display for KernelCallError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "kernel error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for KernelCallError {}

/// 명령 하나의 성질이다. 종류마다 하나이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    pub scope: CommandScope,
    pub state: CommandState,
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
            | Self::PauseAutoFollow
            | Self::SyncIssues => CommandCompletion::Accepted,
            Self::SubmitPrompt
            | Self::RenameSession
            | Self::SteerTurn
            | Self::StopAll
            | Self::DecideApproval
            | Self::ResetPlanning
            | Self::CreateTask
            | Self::UpdateTask
            | Self::DeleteTask
            | Self::CreateDirection
            | Self::UpdateDirection
            | Self::DeleteDirection
            | Self::SetIssueAutomationMode
            | Self::SetIssueAutonomy
            | Self::AnswerIssueDecision
            | Self::HoldIssue
            | Self::ReleaseIssue
            | Self::SteerIssue
            | Self::TakeOverIssue
            | Self::RetriageIssue
            | Self::CancelIssueWork
            | Self::TrackIssue
            | Self::UntrackIssue => CommandCompletion::Records,
        };
        let (scope, effect, idempotency, rejections): (_, _, _, &'static [Reason]) = match self {
            Self::LoadSessions => (
                CommandScope::Session,
                Read,
                Natural,
                &[Reason::KernelUnreachable],
            ),
            Self::RenameSession => (
                CommandScope::Session,
                Write,
                Natural,
                &[Reason::KernelUnreachable],
            ),
            Self::SubmitPrompt => (
                CommandScope::Thread,
                Write,
                Keyed,
                &[
                    Reason::KernelUnreachable,
                    Reason::Busy,
                    Reason::PreparationActive,
                    Reason::UnknownThread,
                    Reason::ForeignWorkspace,
                    Reason::ParallelMode,
                    Reason::ParallelModeUnknown,
                ],
            ),
            Self::SteerTurn => (
                CommandScope::Thread,
                Write,
                Keyed,
                &[
                    Reason::KernelUnreachable,
                    Reason::SteerInFlight,
                    Reason::NoMatchingTurn,
                ],
            ),
            Self::StopAll => (
                CommandScope::Thread,
                Write,
                Natural,
                &[Reason::KernelUnreachable],
            ),
            Self::DecideApproval => (
                CommandScope::Approval,
                Write,
                Keyed,
                &[
                    Reason::KernelUnreachable,
                    Reason::DecisionInFlight,
                    Reason::NoMatchingApproval,
                ],
            ),
            Self::SetAutoFollowMaxTurns => (
                CommandScope::AutoFollow,
                Write,
                Natural,
                &[Reason::KernelUnreachable],
            ),
            Self::PauseAutoFollow => (
                CommandScope::AutoFollow,
                Write,
                Natural,
                &[Reason::KernelUnreachable],
            ),
            Self::SetIssueAutomationMode => (
                CommandScope::Issue,
                Write,
                Natural,
                &[Reason::HumanConfirmationRequired],
            ),
            Self::SetIssueAutonomy | Self::AnswerIssueDecision => (
                CommandScope::Issue,
                Write,
                Keyed,
                &[Reason::HumanConfirmationRequired],
            ),
            Self::HoldIssue
            | Self::ReleaseIssue
            | Self::SteerIssue
            | Self::TakeOverIssue
            | Self::RetriageIssue
            | Self::TrackIssue
            | Self::UntrackIssue => (CommandScope::Issue, Write, Keyed, &[]),
            Self::CancelIssueWork => (CommandScope::Issue, Destructive, Keyed, &[]),
            Self::SyncIssues => (
                CommandScope::Issue,
                Read,
                Natural,
                &[Reason::KernelUnreachable, Reason::IssueAutomationOff],
            ),
            Self::ResetPlanning => (CommandScope::Planning, Destructive, Natural, &[]),
            Self::RefreshPlanning => (
                CommandScope::Planning,
                Read,
                Natural,
                &[Reason::KernelUnreachable],
            ),
            // 저장 명령이다 (plan 18 K4-9d). 커널이 없어도 실행되므로 `kernel_unreachable` 이 없다.
            Self::CreateTask | Self::CreateDirection => (
                CommandScope::Planning,
                Write,
                Keyed,
                &[Reason::PlanningLocked],
            ),
            Self::UpdateTask | Self::UpdateDirection => (
                CommandScope::Planning,
                Write,
                Natural,
                &[Reason::PlanningRevisionStale, Reason::PlanningLocked],
            ),
            Self::DeleteTask | Self::DeleteDirection => (
                CommandScope::Planning,
                Destructive,
                Natural,
                &[Reason::PlanningRevisionStale, Reason::PlanningLocked],
            ),
        };
        CommandSpec {
            scope,
            state: match self {
                Self::ResetPlanning
                | Self::CreateTask
                | Self::UpdateTask
                | Self::DeleteTask
                | Self::CreateDirection
                | Self::UpdateDirection
                | Self::DeleteDirection
                | Self::SetIssueAutomationMode
                | Self::SetIssueAutonomy
                | Self::AnswerIssueDecision
                | Self::HoldIssue
                | Self::ReleaseIssue
                | Self::SteerIssue
                | Self::TakeOverIssue
                | Self::RetriageIssue
                | Self::CancelIssueWork
                | Self::TrackIssue
                | Self::UntrackIssue => CommandState::Store,
                Self::LoadSessions
                | Self::RenameSession
                | Self::SubmitPrompt
                | Self::SteerTurn
                | Self::StopAll
                | Self::DecideApproval
                | Self::SetAutoFollowMaxTurns
                | Self::PauseAutoFollow
                | Self::RefreshPlanning
                | Self::SyncIssues => CommandState::Runtime,
            },
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
        AutoFollowConfigured => "auto_follow_configured",
            "The repository auto-follow limit and pause state changed.";
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
            "A command ended without a turn record. For `submit_prompt`: its preparation failed, its work went to parallel slots, or its turn submission closed before a turn started. For the other commands: the request of the command returned (`done` or `failed`), or the command did nothing (`failed`). For every command: the kernel was built again (`lost`). `detail` is the cause that a failed request gave, as text for a person.";
        RepositoryChanged => "repository_changed",
            "The checked-out commit or branch of the workspace differs from the last observation. The kernel looks periodically, so one record can cover several changes. It does not say who made the change. Without `previous_head`, this is the first observation in this log. This record has no thread: `thread_id` is empty in history.";
        IssueTracked => "issue_tracked",
            "A stable issue entered the collection ledger for the first time. Metadata-only withheld observations count once; rediscovery does not repeat this record.";
        IssueTriaged => "issue_triaged",
            "A deterministic issue assessment was committed to the local ledger. The record carries stable identity and policy/gate revisions, with no issue text or execution permit.";
        IssueDecisionRequested => "issue_decision_requested",
            "An open issue decision was committed to the issue ledger. Only stable identity and decision metadata are recorded; question and note text are excluded.";
        IssueDecisionResolved => "issue_decision_resolved",
            "An issue decision was closed by its first answer or superseded by a changed basis. An agent answer is not human confirmation and does not grant execution.";
        IssueWorkSubmitted => "issue_work_submitted",
            "A durable issue work order was registered. Only stable identity and task IDs are recorded.";
        IssueHeld => "issue_held",
            "An issue hold was committed. New work is blocked while running turns and local work are preserved.";
        IssueReleased => "issue_released",
            "An issue hold was released. Previously revoked work permits are not restored.";
        IssueWorkPermitRevoked => "issue_work_permit_revoked",
            "An issue intervention advanced the authorization epoch and fenced previously granted work permits.";
        IssueWorkPermitGranted => "issue_work_permit_granted",
            "Work permits were bound to approved issue evidence and a frozen source SHA. Integration still requires human review.";
        IssueAutomationModeChanged => "issue_automation_mode_changed",
            "An issue collection mode change was committed. The ledger is authoritative; this audit record is not used to restore the mode.";
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
    fn read_commands_are_not_recorded_and_issue_sync_can_reject_when_off() {
        // The recording rule is derived from the effect. It is not a second list.
        let unrecorded = KernelCommandKind::ALL
            .iter()
            .filter(|kind| !kind.is_recorded())
            .map(|kind| kind.name())
            .collect::<Vec<_>>();
        assert_eq!(
            unrecorded,
            ["load_sessions", "refresh_planning", "sync_issues"]
        );
        for kind in KernelCommandKind::ALL {
            if !kind.is_recorded() {
                assert_eq!(
                    kind.spec().rejections,
                    if *kind == KernelCommandKind::SyncIssues {
                        &[
                            ReasonCode::KernelUnreachable,
                            ReasonCode::IssueAutomationOff,
                        ][..]
                    } else {
                        &[ReasonCode::KernelUnreachable][..]
                    },
                    "{}",
                    kind.name()
                );
            }
        }
    }

    #[test]
    fn issue_commands_keep_storage_and_runtime_boundaries_distinct() {
        let mode = KernelCommandKind::SetIssueAutomationMode.spec();
        assert_eq!(mode.scope, CommandScope::Issue);
        assert_eq!(mode.state, CommandState::Store);
        assert_eq!(mode.effect, CommandEffect::Write);
        assert_eq!(mode.idempotency, CommandIdempotency::Natural);
        assert_eq!(mode.completion, CommandCompletion::Records);
        assert_eq!(mode.rejections, &[ReasonCode::HumanConfirmationRequired]);
        let answer = KernelCommandKind::AnswerIssueDecision.spec();
        assert_eq!(answer.scope, CommandScope::Issue);
        assert_eq!(answer.state, CommandState::Store);
        assert_eq!(answer.effect, CommandEffect::Write);
        assert_eq!(answer.idempotency, CommandIdempotency::Keyed);
        assert_eq!(answer.completion, CommandCompletion::Records);
        assert_eq!(answer.rejections, &[ReasonCode::HumanConfirmationRequired]);
        let sync = KernelCommandKind::SyncIssues.spec();
        assert_eq!(sync.scope, CommandScope::Issue);
        assert_eq!(sync.state, CommandState::Runtime);
        assert_eq!(sync.effect, CommandEffect::Read);
        assert_eq!(sync.idempotency, CommandIdempotency::Natural);
        assert_eq!(sync.completion, CommandCompletion::Accepted);
        assert!(sync.rejections.contains(&ReasonCode::IssueAutomationOff));
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
                "kernel_unreachable",
                "steer_in_flight",
                "no_matching_turn",
                "decision_in_flight",
                "no_matching_approval",
                "busy",
                "preparation_active",
                "unknown_thread",
                "foreign_workspace",
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
                "planning_revision_stale",
                "planning_locked",
                "not_yet_observed",
                "source_unavailable",
                "metadata_incomplete",
                "disclosure_policy_stale",
                "selection_not_matched",
                "issue_closed",
                "untrusted_actor",
                "already_claimed",
                "title_missing",
                "required_section_missing",
                "criteria_missing",
                "criteria_not_list",
                "issue_too_large",
                "unsupported_content",
                "disclosure_confirmation_required",
                "masked_content",
                "high_risk",
                "held_or_manual",
                "protected_path",
                "policy_human_required",
                "human_selection_required",
                "budget_exhausted",
                "mode_off",
                "observation_only",
                "execution_grant_required",
                "close_disabled",
                "human_confirmation_required",
                "decision_superseded",
                "issue_automation_off",
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
