//! Akra 커널 응용 프로토콜 (plan 18 K3-1 의 v0, K4-1 부터 판 1). JSON-RPC `akra/kernel/*` 메서드의 직렬화 타입과, 그 타입과
//! core 타입 사이의 변환을 둔다. core 는 직렬화를 모르므로 변환은 이 inbound adapter 만 한다. 커널 호스트는
//! `src/composition/kernel_host.rs` 이다. 전송(Gateway)은 K3-5 에서 붙인다.

pub use crate::domain::kernel_event::ReasonCode;
use crate::domain::kernel_event::{
    KernelCommandDescriptor, KernelEventPage, OperationReceipt, OperationStatus,
    RecordedKernelEvent,
};
pub use crate::domain::kernel_spec::OperationState;
use crate::domain::kernel_spec::{KernelCommandKind, spec_names};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::app::{
    AppCommand, AppEvent, AppSnapshot, ConversationRuntimeSnapshot, PlanningWorkspaceResetIntent,
    PlanningWorkspaceResetTarget, SESSION_CATALOG_PAGE_SIZE, SessionCatalogLoadIntent,
    SessionCatalogSnapshot, ThreadActivitySummary, ThreadPromptAdmission, ThreadPromptOutcome,
    ThreadPromptPreparationFailure, ThreadPromptSubmission, TurnSubmissionAdmission,
};
use crate::domain::conversation::{
    ConversationApprovalDecision, ConversationApprovalRequestIdentity, ConversationTurnSteerRequest,
};
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterState, ParallelModeQueueItemState, ParallelModeSupervisorSnapshot,
    ParallelModeSupervisorState, PrValidationOperatorState, PrValidationPhase,
};
use crate::domain::planning::{
    PriorityQueueTask, RuntimeProjection, RuntimeWorkspaceStatus, TaskStatus,
};
use crate::domain::recent_sessions::{SessionCatalog, SessionRenameRequest};

#[cfg(test)]
mod tests;

mod next;
use next::error_next;
pub use next::{NextStep, reason_next};

mod spec;
pub use spec::{KernelSpec, describe};

/// 프로토콜 판이다. 전송 모양을 호환되지 않게 바꿀 때만 올린다. 필드를 더하는 것은 올리지 않는다. K4-1 에서
/// `describe` 의 답이 스펙 전체로 바뀌어 1 이 되었다.
pub const PROTOCOL_VERSION: u32 = 1;

pub const METHOD_ATTACH: &str = KernelMethodKind::Attach.name();
pub const METHOD_DESCRIBE: &str = KernelMethodKind::Describe.name();
pub const METHOD_COMMAND: &str = KernelMethodKind::Command.name();
pub const METHOD_QUERY: &str = KernelMethodKind::Query.name();
pub const METHOD_EVENT: &str = KernelMethodKind::Event.name();
pub const METHOD_DETACH: &str = KernelMethodKind::Detach.name();

spec_names! {
    /// 프로토콜의 메서드이다.
    pub(crate) enum KernelMethodKind {
        Attach => "akra/kernel/attach",
            "Attach this connection to the kernel of the repository that owns the workspace folder.";
        Describe => "akra/kernel/describe",
            "Return this specification.";
        Command => "akra/kernel/command",
            "Send one command. The answer carries the rejection, if any, and the operation receipt.";
        Query => "akra/kernel/query",
            "Read one current value or a page of the decision log.";
        Event => "akra/kernel/event",
            "Notification from the kernel to an attached client.";
        Detach => "akra/kernel/detach",
            "Detach this connection. No event arrives after the answer.";
    }
}

/// JSON-RPC 의 잘못된 인자 오류 코드이다.
pub const ERROR_INVALID_PARAMS: i64 = KernelErrorKind::InvalidParams.code();
/// 이 판의 `akra/kernel/*` 메서드가 아니다.
pub const ERROR_METHOD_NOT_FOUND: i64 = KernelErrorKind::MethodNotFound.code();
/// 확인이 필요한 명령을 확인 없이 보냈다 (K-D1).
pub const ERROR_CONFIRMATION_REQUIRED: i64 = KernelErrorKind::ConfirmationRequired.code();
/// 이 접속은 아직 커널에 붙지 않았다. `akra/kernel/attach` 를 먼저 보낸다.
pub const ERROR_NOT_ATTACHED: i64 = KernelErrorKind::NotAttached.code();
/// 커널에 붙거나 닿지 못했다. 예: 다른 프로세스(TUI)가 그 저장소의 커널을 돌린다, 커널이 끝났다.
pub const ERROR_KERNEL_UNAVAILABLE: i64 = KernelErrorKind::KernelUnavailable.code();
/// 커널이 정한 시간 안에 답하지 않았다. 명령은 그 뒤에 처리될 수 있으므로, 클라이언트는 조회로 결과를 확인한다.
pub const ERROR_KERNEL_TIMEOUT: i64 = KernelErrorKind::KernelTimeout.code();
/// 이 `idempotency_key` 는 다른 명령에 이미 썼다 (plan 19 Q3-6). 명령마다 새 키를 만든다.
pub const ERROR_IDEMPOTENCY_KEY_REUSED: i64 = KernelErrorKind::IdempotencyKeyReused.code();
/// 이 커널은 기록이 없어서 재전송을 가릴 수 없다 (plan 19 Q3-6). 키가 있는 명령을 실행하지 않았다. 키 없이 보내면
/// 실행하지만 재전송은 다시 실행된다.
pub const ERROR_IDEMPOTENCY_UNAVAILABLE: i64 = KernelErrorKind::IdempotencyUnavailable.code();

spec_names! {
    /// 프로토콜이 내는 JSON-RPC 오류이다. 코드는 `code` 가 정한다.
    pub(crate) enum KernelErrorKind {
        InvalidParams => "invalid_params",
            "The request does not match the input schema.";
        MethodNotFound => "method_not_found",
            "The method is not an `akra/kernel/*` method of this protocol version.";
        ConfirmationRequired => "confirmation_required",
            "A destructive command came without `confirm: true`.";
        NotAttached => "not_attached",
            "This connection is not attached. Send `akra/kernel/attach` first.";
        KernelUnavailable => "kernel_unavailable",
            "The kernel cannot be reached. Another process (a TUI) may run the kernel of that repository, or the kernel stopped.";
        KernelTimeout => "kernel_timeout",
            "The kernel did not answer in time. The command may still run; read the result with a query.";
        IdempotencyKeyReused => "idempotency_key_reused",
            "The `idempotency_key` was already used for a different command. Use a new key for each command.";
        IdempotencyUnavailable => "idempotency_unavailable",
            "This kernel has no decision log and cannot tell resends apart. The command did not run.";
    }
}

impl KernelErrorKind {
    pub(crate) const fn code(self) -> i64 {
        match self {
            Self::InvalidParams => -32602,
            Self::MethodNotFound => -32601,
            Self::ConfirmationRequired => -32010,
            Self::NotAttached => -32011,
            Self::KernelUnavailable => -32012,
            Self::KernelTimeout => -32013,
            Self::IdempotencyKeyReused => -32014,
            Self::IdempotencyUnavailable => -32015,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelProtocolError {
    pub code: i64,
    pub message: String,
}

impl KernelProtocolError {
    fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: ERROR_INVALID_PARAMS,
            message: message.into(),
        }
    }

    /// 확인이 필요한 명령을 확인 없이 보냈다 (K-D1, plan 18 K4-5). 커널이 실행하지 않았다.
    pub fn confirmation_required(command: &str) -> Self {
        Self {
            code: ERROR_CONFIRMATION_REQUIRED,
            message: format!("{command} deletes data; send \"confirm\": true"),
        }
    }

    pub fn idempotency_key_reused() -> Self {
        Self {
            code: ERROR_IDEMPOTENCY_KEY_REUSED,
            message: "idempotency_key was already used for a different command".to_string(),
        }
    }

    pub fn idempotency_unavailable() -> Self {
        Self {
            code: ERROR_IDEMPOTENCY_UNAVAILABLE,
            message:
                "this kernel has no decision log and cannot tell resends apart; the command did not run"
                    .to_string(),
        }
    }
}

// attach -------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttachRequest {
    /// 커널을 고르는 작업 공간 루트이다.
    pub workspace: String,
    /// 진단용 클라이언트 이름이다 (예: `akra-desktop`).
    pub client_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AttachResponse {
    pub client_id: String,
    pub protocol_version: u32,
    /// 붙은 커널의 저장소 신원이다. 같은 저장소의 worktree 들은 같은 값을 받는다 (§3.4).
    pub kernel: String,
}

// command -----------------------------------------------------------------------------------------

/// 멱등 키의 최대 길이이다.
const MAX_IDEMPOTENCY_KEY_CHARS: usize = 128;
/// 명령 지문의 길이이다 (SHA-256 의 앞 16바이트를 16진수로 쓴다).
const FINGERPRINT_HEX_CHARS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandRequest {
    pub command: KernelCommand,
    /*
     * 재전송을 가리는 키이다 (plan 19 Q3-6). 클라이언트가 명령마다 새로 만든다. 키의 범위는 저장소의 커널
     * 전체이므로(모든 클라이언트) 무작위 값(UUID 등)을 쓴다. 같은 키와 같은 명령을 다시 보내면 커널은 다시 실행하지
     * 않고 처음의 영수증을 답한다 (`operation.replayed`). 거절된 명령도 같다: 다시 하려면 새 키로 보낸다. 같은 키로
     * 다른 명령을 보내면 `ERROR_IDEMPOTENCY_KEY_REUSED` 이다. 커널은 그 명령의 기록이 남아 있는 동안 키를 기억한다.
     * 기록이 없는 커널은 키가 있는 명령을 실행하지 않고 `ERROR_IDEMPOTENCY_UNAVAILABLE` 로 답한다.
     */
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 128))]
    pub idempotency_key: Option<String>,
}

impl CommandRequest {
    /*
     * 커널이 이 명령을 기록할 때 쓰는 설명이다 (plan 19 Q3-6). 명령의 내용은 지문으로만 남는다. 읽기 명령
     * (스펙의 `effect: read`)은 기록하지 않으므로 `None` 이다. 그 명령은 바른 키가 있어도 그냥 실행한다. 잘못된
     * 키는 어느 명령이든 거절한다.
     */
    pub fn descriptor(&self) -> Result<Option<KernelCommandDescriptor>, KernelProtocolError> {
        let idempotency_key = self
            .idempotency_key
            .as_deref()
            .map(valid_idempotency_key)
            .transpose()?;
        let kind = self.command.kind();
        if !kind.is_recorded() {
            return Ok(None);
        }
        let thread_id = match &self.command {
            KernelCommand::RenameSession { thread_id, .. }
            | KernelCommand::SubmitPrompt { thread_id, .. }
            | KernelCommand::SteerTurn { thread_id, .. } => thread_id.clone(),
            KernelCommand::LoadSessions {}
            | KernelCommand::StopAll {}
            | KernelCommand::DecideApproval { .. }
            | KernelCommand::SetAutoFollowMaxTurns { .. }
            | KernelCommand::PauseAutoFollow {}
            | KernelCommand::ResetPlanning { .. }
            | KernelCommand::RefreshPlanning {} => String::new(),
        };
        let json = serde_json::to_value(&self.command).map_err(|error| {
            KernelProtocolError::invalid_params(format!("command cannot be described: {error}"))
        })?;
        Ok(Some(KernelCommandDescriptor {
            command: kind,
            thread_id,
            fingerprint: fingerprint(&json),
            idempotency_key,
            confirmed: self.command.confirmed(),
            // 접속의 이름은 전송 계층(Gateway)이 안다. 거기서 넣는다.
            caller: None,
        }))
    }
}

fn valid_idempotency_key(key: &str) -> Result<String, KernelProtocolError> {
    let valid = !key.trim().is_empty()
        && key.chars().count() <= MAX_IDEMPOTENCY_KEY_CHARS
        && !key.chars().any(char::is_control);
    if !valid {
        return Err(KernelProtocolError::invalid_params(format!(
            "idempotency_key must be 1 to {MAX_IDEMPOTENCY_KEY_CHARS} characters without control characters"
        )));
    }
    Ok(key.to_string())
}

/*
 * 명령 내용의 지문이다. 기록에 내용 대신 남는다. 같은 명령은 binary 가 바뀌어도 같은 지문이어야 하므로, 키를 정렬한
 * JSON 글자로 만든다 (`serde_json` 의 맵 순서 설정에 기대지 않는다). 명령에 필드를 더하면 그 명령의 지문이 바뀐다.
 * 지문은 비밀이 아니다: 짧은 프롬프트는 추측해서 같은 지문인지 확인할 수 있다.
 */
fn fingerprint(command: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};
    let mut canonical = String::new();
    write_canonical_json(command, &mut canonical);
    let digest = Sha256::digest(canonical.as_bytes());
    let mut hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    hex.truncate(FINGERPRINT_HEX_CHARS);
    hex
}

/// 객체의 키를 정렬하고 공백 없이 쓴다.
fn write_canonical_json(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Object(fields) => {
            let mut names = fields.keys().collect::<Vec<_>>();
            names.sort_unstable();
            out.push('{');
            for (index, name) in names.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::Value::String(name.clone()).to_string());
                out.push(':');
                write_canonical_json(&fields[name], out);
            }
            out.push('}');
        }
        serde_json::Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical_json(item, out);
            }
            out.push(']');
        }
        scalar => out.push_str(&scalar.to_string()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CommandResponse {
    /// 커널이 명령을 처리했다. `false` 는 커널이 끝나서 명령을 넣지 못한 경우이다.
    pub processed: bool,
    /// 커널이 이 명령을 바로 거절한 이유이다 (`steer_turn`, `decide_approval`, 준비 전에 거절한 `submit_prompt`).
    /// `submit_prompt` 의 준비 뒤 진행과 거절은 `prompt_submission` 이벤트로 온다. 받아들인 명령의 결과는
    /// 이벤트(`thread_activity` 등)로 온다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection: Option<CommandRejection>,
    /// 명령의 영수증이다 (plan 19 Q3-6). 읽기 명령과 기록이 없는 커널은 영수증이 없다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<Operation>,
}

/*
 * 명령의 영수증이다 (plan 19 Q3-6). `op_id` 는 커널이 그 명령을 받은 기록의 sequence 이다. 명령의 진행은 그 뒤의
 * 기록이다: `next` 대로 `history` 를 `after` 부터 읽거나, `sequence` 가 `op_id` 보다 큰 `recorded` 이벤트를 본다.
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Operation {
    pub op_id: i64,
    /// 받아들임의 결과이다. 지금 상태는 `operation` 조회가 알린다 (plan 18 K4-4).
    pub state: OperationAdmission,
    pub next: OperationNext,
    /// 같은 멱등 키의 명령을 전에 받았다. 다시 실행하지 않았고, 처음의 결과를 답한다.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub replayed: bool,
}

/*
 * 영수증의 `state` 이다. 커널이 명령을 받을 때의 결과만 알린다. 연산의 지금 상태(`OperationState`)와 타입을 나눈다.
 * 그래서 스펙만 읽는 클라이언트가 영수증에는 이 두 값만 온다는 것을 안다.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OperationAdmission {
    /// 커널이 명령을 받아들였다. 결과는 이벤트와 기록으로 온다.
    Accepted,
    /// 커널이 명령을 바로 거절했다. 이유는 `rejection` 에 있다.
    Rejected,
}

/// 영수증 뒤에 할 조회이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OperationNext {
    pub query: String,
    pub after: i64,
}

impl Operation {
    fn from_receipt(receipt: OperationReceipt) -> Self {
        Self {
            op_id: receipt.op_id,
            state: if receipt.rejection.is_some() {
                OperationAdmission::Rejected
            } else {
                OperationAdmission::Accepted
            },
            next: OperationNext {
                query: "history".to_string(),
                after: receipt.op_id,
            },
            replayed: receipt.replayed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CommandRejection {
    pub reason: ReasonCode,
    /// 사람이 읽는 설명이다. 판단에는 `reason` 을 쓴다.
    pub message: String,
    /// 다음 행동이다 (plan 18 K4-3). 스펙의 그 이유의 `next` 와 같다.
    pub next: Vec<NextStep>,
}

impl CommandResponse {
    /// 커널이 명령을 처리하며 낸 core 이벤트로 응답을 만든다. `None` 은 커널이 끝난 경우이다.
    pub fn from_core(events: Option<&[AppEvent]>) -> Self {
        let Some(events) = events else {
            return Self {
                processed: false,
                rejection: None,
                operation: None,
            };
        };
        Self {
            processed: true,
            rejection: events
                .iter()
                .find_map(AppEvent::command_rejection)
                .map(CommandRejection::new),
            operation: None,
        }
    }

    /// 커널이 준 영수증을 붙인다. 영수증이 없으면(읽기 명령, 기록이 없는 커널) 그대로이다.
    pub fn with_operation(mut self, receipt: Option<OperationReceipt>) -> Self {
        self.operation = receipt.map(Operation::from_receipt);
        self
    }

    /// 같은 멱등 키의 명령을 다시 받았다. 실행하지 않고 처음의 결과로 답한다.
    pub fn replayed(receipt: OperationReceipt) -> Self {
        Self {
            processed: true,
            rejection: receipt.rejection.map(CommandRejection::new),
            operation: Some(Operation::from_receipt(receipt)),
        }
    }
}

impl CommandRejection {
    fn new(reason: ReasonCode) -> Self {
        // 설명과 다음 행동은 이유의 표에서 온다. 스펙의 `reasons` 와 같은 값이다.
        Self {
            reason,
            message: reason.summary().to_string(),
            next: reason_next(reason),
        }
    }
}

/*
 * v0 명령이다. 화면 흐름 단계가 아니라 한 번에 끝나는 도메인 명령이다 (plan 18 §5.5). 원격 클라이언트는 보는
 * 자리를 바꾸지 않고 thread 나 승인 id 를 지정한다 (K3-3a). 프롬프트 제출은 준비와 제출을 하나로 한다 (K3-3b).
 */
/// 빈 글자와 공백만 있는 글자를 받지 않는 필드의 규칙이다. `non_blank` 가 같은 것을 검사한다.
const NON_BLANK: &str = r"\S";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
// 필드가 없는 명령도 `{}` 로 둔다. serde 는 unit variant 의 모르는 필드를 거절하지 못한다.
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum KernelCommand {
    LoadSessions {},
    RenameSession {
        #[schemars(length(min = 1), regex(pattern = NON_BLANK))]
        thread_id: String,
        #[schemars(length(min = 1), regex(pattern = NON_BLANK))]
        name: String,
    },
    /// thread 에 프롬프트를 보낸다. 커널이 planning 조각을 붙여 준비한 뒤 턴을 제출한다.
    SubmitPrompt {
        #[schemars(length(min = 1), regex(pattern = NON_BLANK))]
        thread_id: String,
        #[schemars(length(min = 1), regex(pattern = NON_BLANK))]
        prompt: String,
    },
    SteerTurn {
        #[schemars(length(min = 1), regex(pattern = NON_BLANK))]
        thread_id: String,
        #[schemars(length(min = 1), regex(pattern = NON_BLANK))]
        expected_turn_id: String,
        #[schemars(length(min = 1), regex(pattern = NON_BLANK))]
        prompt: String,
    },
    StopAll {},
    DecideApproval {
        #[schemars(length(min = 1), regex(pattern = NON_BLANK))]
        approval_id: String,
        #[schemars(length(min = 1), regex(pattern = NON_BLANK))]
        server_request_id: String,
        decision: ApprovalDecision,
    },
    SetAutoFollowMaxTurns {
        value: usize,
    },
    PauseAutoFollow {},
    ResetPlanning {
        target: ResetTarget,
        /// 지우는 명령이라 `true` 가 아니면 거절한다 (K-D1).
        #[serde(default)]
        confirm: bool,
    },
    /// planning 큐를 다시 읽는다. 결과는 `planning` 이벤트로 온다.
    RefreshPlanning {},
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Accept,
    Decline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResetTarget {
    Queue,
    Directions,
    All,
}

impl KernelCommand {
    /// 이 명령의 종류이다. 이름과 성질은 domain 의 표가 가진다 (plan 18 K4-1).
    pub fn kind(&self) -> KernelCommandKind {
        match self {
            Self::LoadSessions {} => KernelCommandKind::LoadSessions,
            Self::RenameSession { .. } => KernelCommandKind::RenameSession,
            Self::SubmitPrompt { .. } => KernelCommandKind::SubmitPrompt,
            Self::SteerTurn { .. } => KernelCommandKind::SteerTurn,
            Self::StopAll {} => KernelCommandKind::StopAll,
            Self::DecideApproval { .. } => KernelCommandKind::DecideApproval,
            Self::SetAutoFollowMaxTurns { .. } => KernelCommandKind::SetAutoFollowMaxTurns,
            Self::PauseAutoFollow {} => KernelCommandKind::PauseAutoFollow,
            Self::ResetPlanning { .. } => KernelCommandKind::ResetPlanning,
            Self::RefreshPlanning {} => KernelCommandKind::RefreshPlanning,
        }
    }

    /// 클라이언트가 지우는 명령을 확인하고 보냈는가이다. 확인 필드가 없는 명령은 `false` 이다. 커널이 이 값과
    /// 스펙의 `effect` 로 명령을 받을지 정한다 (plan 18 K4-5). 이 adapter 는 검사하지 않는다.
    pub fn confirmed(&self) -> bool {
        match self {
            Self::ResetPlanning { confirm, .. } => *confirm,
            Self::LoadSessions {}
            | Self::RenameSession { .. }
            | Self::SubmitPrompt { .. }
            | Self::SteerTurn { .. }
            | Self::StopAll {}
            | Self::DecideApproval { .. }
            | Self::SetAutoFollowMaxTurns { .. }
            | Self::PauseAutoFollow {}
            | Self::RefreshPlanning {} => false,
        }
    }

    /// 커널이 처리할 core 명령으로 바꾼다. 작업 공간은 attach 때 고른 커널의 것이다.
    pub fn into_core(self, workspace_directory: &str) -> Result<AppCommand, KernelProtocolError> {
        let command = match self {
            Self::LoadSessions {} => {
                AppCommand::LoadSessionCatalog(SessionCatalogLoadIntent::ensure_loaded(
                    SESSION_CATALOG_PAGE_SIZE,
                    workspace_directory,
                ))
            }
            Self::RenameSession { thread_id, name } => {
                AppCommand::RenameSession(SessionRenameRequest::new(
                    non_blank("thread_id", thread_id)?,
                    non_blank("name", name)?,
                ))
            }
            Self::SubmitPrompt { thread_id, prompt } => {
                AppCommand::SubmitThreadPrompt(Box::new(ThreadPromptSubmission {
                    workspace_directory: workspace_directory.to_string(),
                    thread_id: non_blank("thread_id", thread_id)?,
                    prompt: non_blank("prompt", prompt)?,
                    // 런타임이 명령을 기록한 뒤 연산 id 를 넣는다 (plan 18 K4-4).
                    op_id: None,
                }))
            }
            Self::SteerTurn {
                thread_id,
                expected_turn_id,
                prompt,
            } => AppCommand::SteerTurn(ConversationTurnSteerRequest {
                thread_id: non_blank("thread_id", thread_id)?,
                expected_turn_id: non_blank("expected_turn_id", expected_turn_id)?,
                prompt: non_blank("prompt", prompt)?,
            }),
            Self::StopAll {} => AppCommand::RequestStopAllSessions,
            Self::DecideApproval {
                approval_id,
                server_request_id,
                decision,
            } => AppCommand::SubmitApprovalDecision {
                request_identity: ConversationApprovalRequestIdentity {
                    approval_id: non_blank("approval_id", approval_id)?,
                    server_request_id: non_blank("server_request_id", server_request_id)?,
                },
                decision: match decision {
                    ApprovalDecision::Accept => ConversationApprovalDecision::Accept,
                    ApprovalDecision::Decline => ConversationApprovalDecision::Decline,
                },
            },
            Self::SetAutoFollowMaxTurns { value } => AppCommand::SetAutoFollowMaxTurns { value },
            Self::PauseAutoFollow {} => AppCommand::PausePostTurnContinuation,
            Self::ResetPlanning { target, confirm } => AppCommand::ResetPlanningWorkspace(
                PlanningWorkspaceResetIntent::new(
                    workspace_directory,
                    match target {
                        ResetTarget::Queue => PlanningWorkspaceResetTarget::Queue,
                        ResetTarget::Directions => PlanningWorkspaceResetTarget::Directions,
                        ResetTarget::All => PlanningWorkspaceResetTarget::All,
                    },
                )
                .with_confirmation(confirm),
            ),
            Self::RefreshPlanning {} => AppCommand::RefreshPlanningRuntime {
                workspace_directory: workspace_directory.to_string(),
            },
        };
        Ok(command)
    }
}

fn non_blank(field: &str, value: String) -> Result<String, KernelProtocolError> {
    if value.trim().is_empty() {
        return Err(KernelProtocolError::invalid_params(format!(
            "{field} must not be blank"
        )));
    }
    Ok(value)
}

// query -------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum KernelQuery {
    Sessions {},
    AutoFollow {},
    ThreadActivity {},
    Planning {},
    Parallel {},
    /*
     * 판단 사건 기록이다 (plan 19 Q3-3). `after` 보다 뒤의 사건을 기록 순서대로 최대 `limit` 개 준다. 처음에는
     * `after` 를 비운다. 답의 `next_after` 를 다음 조회의 `after` 로 보낸다.
     */
    History {
        #[serde(default)]
        after: Option<i64>,
        #[serde(default)]
        limit: Option<u32>,
    },
    /*
     * 연산 하나의 상태와 그 연산의 기록이다 (plan 18 K4-4). `op_id` 는 명령의 답에 있는 `operation.op_id` 이다.
     * 상태는 기록에서 계산한다. 명령이 언제 끝나는지는 스펙의 그 명령의 `completion` 이 알린다.
     */
    Operation {
        op_id: i64,
    },
}

/// 기록 조회의 한 쪽 기본 크기이다.
const DEFAULT_HISTORY_LIMIT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueryRequest {
    pub query: KernelQuery,
}

impl KernelQuery {
    /// 기록 조회이면 읽을 cursor 와 개수이다. 기록은 스냅샷이 아니라 커널의 사건 기록에서 읽는다.
    pub fn history_range(&self) -> Option<(i64, usize)> {
        match self {
            Self::History { after, limit } => Some((
                after.unwrap_or(0),
                limit.map_or(DEFAULT_HISTORY_LIMIT, |limit| limit as usize),
            )),
            _ => None,
        }
    }

    /// 연산 조회이면 그 연산 id 이다. 연산도 스냅샷이 아니라 커널의 사건 기록에서 읽는다.
    pub fn operation_id(&self) -> Option<i64> {
        match self {
            Self::Operation { op_id } => Some(*op_id),
            _ => None,
        }
    }

    /// 조회의 답은 같은 이름의 이벤트와 같은 모양이다. 클라이언트는 둘을 같은 코드로 그린다. 기록 조회와 연산
    /// 조회는 호출자가 기록에서 읽은 값(`history`, `operation`)을 그대로 담는다.
    pub fn answer(
        self,
        snapshot: &AppSnapshot,
        threads: &[ThreadActivitySummary],
        parallel: ParallelSource<'_>,
        history: Option<KernelEventPage>,
        operation: Option<OperationStatus>,
    ) -> KernelQueryAnswer {
        match self {
            Self::Sessions {} => {
                KernelQueryAnswer::Sessions(SessionsState::from_core(&snapshot.session_catalog))
            }
            Self::AutoFollow {} => KernelQueryAnswer::AutoFollow(AutoFollowSettings::from_core(
                &snapshot.conversation_runtime,
            )),
            Self::ThreadActivity {} => KernelQueryAnswer::ThreadActivity {
                threads: threads.iter().map(ThreadActivity::from_core).collect(),
            },
            Self::Planning {} => KernelQueryAnswer::Planning(PlanningState::from_core(
                &snapshot.planning_parallel.planning_runtime,
            )),
            Self::Parallel {} => KernelQueryAnswer::Parallel(ParallelState::from_core(parallel)),
            Self::History { after, .. } => {
                KernelQueryAnswer::History(history.unwrap_or_else(|| KernelEventPage {
                    records: Vec::new(),
                    next_after: after.unwrap_or(0),
                }))
            }
            Self::Operation { op_id } => {
                KernelQueryAnswer::Operation(operation.unwrap_or(OperationStatus {
                    op_id,
                    state: OperationState::Unknown,
                    records: Vec::new(),
                }))
            }
        }
    }
}

spec_names! {
    /// 조회의 종류이다.
    pub(crate) enum KernelQueryKind {
        Sessions => "sessions",
            "The session list of the workspace and its load state.";
        AutoFollow => "auto_follow",
            "The automatic follow-up settings. They are the same for every thread.";
        ThreadActivity => "thread_activity",
            "The threads that still have work: a running turn, a pending approval, or automatic follow-up.";
        Planning => "planning",
            "The planning queue.";
        Parallel => "parallel",
            "The parallel mode state: pool, agents, merge queue, and pull request validation.";
        History => "history",
            "A page of the decision log after a cursor. Send the answer's `next_after` as the next `after`.";
        Operation => "operation",
            "The state of one operation and its records. `op_id` comes from the `operation` of a command answer.";
    }
}

/*
 * 조회의 답이다 (plan 18 K4-1). 이벤트 enum 에서 나눴다. 조회마다 답의 모양이 하나라서 스펙이 조회의 출력 스키마를
 * 이 타입에서 얻는다. 전송 모양은 이전과 같다: 같은 이름의 이벤트와 같고, `history` 는 조회에만 있다.
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KernelQueryAnswer {
    Sessions(SessionsState),
    AutoFollow(AutoFollowSettings),
    ThreadActivity {
        threads: Vec<ThreadActivity>,
    },
    Planning(PlanningState),
    Parallel(ParallelState),
    /// 사건 하나는 `sequence`, `recorded_at`, `event` 이고 `event.kind` 가 종류이다 (`turn_started` 등).
    History(KernelEventPage),
    /// 연산 하나의 상태와 그 연산의 기록이다 (plan 18 K4-4).
    Operation(OperationStatus),
}

// event -------------------------------------------------------------------------------------------

/*
 * v0 이벤트는 굵은 단위이다 (plan 18 §5.5). 클라이언트가 지금 상태를 다시 그릴 수 있는 값을 통째로 보낸다.
 * 증분 스트림은 보내지 않는다. 같은 모양을 조회 결과로도 쓴다.
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KernelEvent {
    Sessions(SessionsState),
    AutoFollow(AutoFollowSettings),
    ThreadActivity {
        threads: Vec<ThreadActivity>,
    },
    /// `submit_prompt` 의 진행이다. 턴이 시작되면 `thread_activity` 에도 나타난다.
    PromptSubmission(PromptSubmissionState),
    /// planning 큐가 바뀌었다 (K3-6).
    Planning(PlanningState),
    /// 병렬 작업 상태가 바뀌었다 (K3-6). 클라이언트가 붙어 있는 동안 커널이 감독 스냅샷을 주기적으로 새로 고친다.
    Parallel(ParallelState),
    /// 이 클라이언트에게 보낼 이벤트가 넘쳐 버렸다. 클라이언트는 조회로 다시 맞춘다.
    Resync,
    /*
     * 커널이 판단 사건 하나를 기록했다 (plan 19 Q3-3b). `sequence` 는 기록과 같으므로 `history` 의 cursor 로 쓴다.
     * `resync` 를 받으면 마지막으로 받은 `sequence` 를 `after` 로 `history` 를 읽어 빠진 사건을 채운다. 붙은 뒤
     * `history` 와 겹칠 수 있으므로 클라이언트는 `sequence` 로 겹침을 지운다.
     */
    Recorded(RecordedKernelEvent),
}

/// `akra/kernel/event` 알림의 params 이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventNotification {
    pub event: KernelEvent,
}

spec_names! {
    /// 이벤트의 종류이다.
    pub(crate) enum KernelEventKind {
        Sessions => "sessions",
            "The session list or its load state changed.";
        AutoFollow => "auto_follow",
            "The automatic follow-up settings changed.";
        ThreadActivity => "thread_activity",
            "The set of threads with work changed. The event carries every such thread.";
        PromptSubmission => "prompt_submission",
            "Progress of a `submit_prompt` command: preparing, submitted, queued, rejected, or failed.";
        Planning => "planning",
            "The planning queue changed.";
        Parallel => "parallel",
            "The parallel mode state changed.";
        Resync => "resync",
            "Events for this client were dropped. Read the current values with queries and `history` after the last sequence seen.";
        Recorded => "recorded",
            "The kernel wrote one decision record. `sequence` is the cursor for `history`.";
    }
}

impl KernelEvent {
    /// 프로토콜로 내보낼 core 이벤트만 바꾼다. 나머지는 v0 에서 보내지 않는다.
    pub fn from_core(event: &AppEvent) -> Option<Self> {
        match event {
            AppEvent::SessionCatalogChanged(snapshot) => {
                Some(Self::Sessions(SessionsState::from_core(snapshot)))
            }
            AppEvent::ConversationRuntimeAuthorityChanged(runtime) => {
                Some(Self::AutoFollow(AutoFollowSettings::from_core(runtime)))
            }
            AppEvent::ThreadActivityChanged(threads) => Some(Self::ThreadActivity {
                threads: threads.iter().map(ThreadActivity::from_core).collect(),
            }),
            AppEvent::ThreadPromptAdmissionResolved {
                thread_id,
                admission,
            } => Some(Self::PromptSubmission(PromptSubmissionState::admitted(
                thread_id, admission,
            ))),
            AppEvent::ThreadPromptSubmitted { thread_id, outcome } => Some(Self::PromptSubmission(
                PromptSubmissionState::from_outcome(thread_id, outcome),
            )),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionsState {
    pub status: SessionsStatus,
    pub sessions: Vec<SessionEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionsStatus {
    Idle,
    Loading,
    Ready,
    Unavailable,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionEntry {
    pub thread_id: String,
    pub title: String,
    pub workspace: String,
    pub updated_at_epoch: i64,
}

impl SessionsState {
    pub fn from_core(snapshot: &SessionCatalogSnapshot) -> Self {
        let (status, sessions, message) = match snapshot {
            SessionCatalogSnapshot::Idle => (SessionsStatus::Idle, Vec::new(), None),
            SessionCatalogSnapshot::Loading => (SessionsStatus::Loading, Vec::new(), None),
            SessionCatalogSnapshot::Failed { message } => {
                (SessionsStatus::Failed, Vec::new(), Some(message.clone()))
            }
            SessionCatalogSnapshot::Ready(ready) => match ready.catalog.as_ref() {
                SessionCatalog::Ready {
                    recent_sessions, ..
                } => (
                    SessionsStatus::Ready,
                    recent_sessions
                        .items
                        .iter()
                        .map(|session| SessionEntry {
                            thread_id: session.id.clone(),
                            title: session.title(),
                            workspace: session.cwd.clone(),
                            updated_at_epoch: session.updated_at_epoch,
                        })
                        .collect(),
                    None,
                ),
                SessionCatalog::Unsupported(_) | SessionCatalog::Partial(_) => {
                    (SessionsStatus::Unavailable, Vec::new(), None)
                }
            },
        };
        Self {
            status,
            sessions,
            message,
        }
    }
}

/*
 * 자동 후속 설정이다. 설정은 모든 thread 에 같이 걸린다. thread 별 진행(자동 후속이 도는지, 턴, 승인)은
 * `thread_activity` 가 알린다. 보는 대화의 턴 상태는 TUI 화면용이라 v0 에서 보내지 않는다 (K3 §3.2).
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AutoFollowSettings {
    pub max_turns: usize,
    pub paused: bool,
}

impl AutoFollowSettings {
    pub fn from_core(runtime: &ConversationRuntimeSnapshot) -> Self {
        Self {
            max_turns: runtime.auto_follow.max_auto_turns,
            paused: runtime.auto_follow.continuation_paused,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ThreadActivity {
    pub thread_id: String,
    pub running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalRequest>,
    pub auto_follow_active: bool,
}

/// 결정을 기다리는 승인 요청이다. `decide_approval` 에 두 id 를 그대로 보낸다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalRequest {
    pub approval_id: String,
    pub server_request_id: String,
    pub summary: String,
    pub details: Vec<String>,
}

impl ThreadActivity {
    pub fn from_core(activity: &ThreadActivitySummary) -> Self {
        Self {
            thread_id: activity.thread_id.clone(),
            running: activity.running,
            active_turn_id: activity.active_turn_id.clone(),
            approval: activity.approval.as_ref().map(|request| ApprovalRequest {
                approval_id: request.approval_id.clone(),
                server_request_id: request.server_request_id.clone(),
                summary: request.summary.clone(),
                details: request.details.clone(),
            }),
            auto_follow_active: activity.auto_follow_active,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PromptSubmissionState {
    pub thread_id: String,
    pub status: PromptSubmissionStatus,
    /// `rejected`, `queued`, `failed`, `dropped` 의 이유 코드이다. 뜻은 스펙의 `reasons` 에 있다 (plan 18 K4-3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<ReasonCode>,
    /// 그 이유 뒤의 다음 행동이다. 스펙의 그 이유의 `next` 와 같다. 이유가 없으면 비어 있고 보내지 않는다.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next: Vec<NextStep>,
    /// 준비가 큐에 만들거나 고친 planning 작업이다. 이 값이 있으면 같은 프롬프트를 다시 보내지 않는다. 다시 보내면
    /// 작업이 하나 더 생긴다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// 사람이 읽는 설명이다. 판단에는 `reason` 을 쓴다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PromptSubmissionStatus {
    /// 받아들였고 준비 중이다.
    Preparing,
    /// 준비가 끝나고 턴을 제출했다.
    Submitted,
    /// 준비가 만든 작업이 큐에 남았고 턴은 시작하지 않았다. 이유는 `reason`, 작업은 `task_id` 이다.
    Queued,
    /// 받아들이지 않았다. 이유는 `reason` 이다. planning 작업은 만들어지지 않았다.
    Rejected,
    /// 준비가 턴을 만들지 못했다. planning 작업은 만들어지지 않았다.
    Failed,
    /// 커널이 큐에 남긴 프롬프트를 내지 않고 끝냈다 (plan 18 K4-3). 이유는 `reason`, 작업은 `task_id` 이다.
    Dropped,
}

impl PromptSubmissionState {
    fn admitted(thread_id: &str, admission: &ThreadPromptAdmission) -> Self {
        let Some(reason) = admission.rejection() else {
            return Self::progress(thread_id, PromptSubmissionStatus::Preparing);
        };
        Self::stopped(thread_id, PromptSubmissionStatus::Rejected, reason, None)
    }

    fn from_outcome(thread_id: &str, outcome: &ThreadPromptOutcome) -> Self {
        match outcome {
            ThreadPromptOutcome::Submitted { admission, task_id } => {
                let reason = match admission {
                    TurnSubmissionAdmission::Accepted { .. } => {
                        return Self {
                            task_id: Some(task_id.clone()),
                            ..Self::progress(thread_id, PromptSubmissionStatus::Submitted)
                        };
                    }
                    TurnSubmissionAdmission::RejectedActive { .. } => {
                        ReasonCode::TurnStartedDuringPreparation
                    }
                    TurnSubmissionAdmission::RejectedStopPending { .. } => ReasonCode::StopPending,
                    TurnSubmissionAdmission::RejectedUnavailable => ReasonCode::Unavailable,
                };
                Self::stopped(
                    thread_id,
                    PromptSubmissionStatus::Queued,
                    reason,
                    Some(task_id.clone()),
                )
            }
            ThreadPromptOutcome::ParallelModeStarted { task_id } => Self::stopped(
                thread_id,
                PromptSubmissionStatus::Queued,
                ReasonCode::ParallelModeStarted,
                Some(task_id.clone()),
            ),
            ThreadPromptOutcome::PreparationFailed(failure) => {
                let (reason, detail) = match failure {
                    ThreadPromptPreparationFailure::Cancelled => (ReasonCode::Cancelled, None),
                    ThreadPromptPreparationFailure::TextMismatch => {
                        (ReasonCode::TextMismatch, None)
                    }
                    ThreadPromptPreparationFailure::BootstrapReviewRequired { draft_name } => (
                        ReasonCode::BootstrapReviewRequired,
                        Some(format!(
                            "planning bootstrap needs review / draft: {draft_name}"
                        )),
                    ),
                    ThreadPromptPreparationFailure::PlanningRefused { reason } => {
                        (ReasonCode::PlanningRefused, Some(reason.clone()))
                    }
                };
                // 이 두 이유는 설명 대신 초안 이름이나 원인을 싣는다. 사람이 그것을 보고 정한다.
                Self::stopped(thread_id, PromptSubmissionStatus::Failed, reason, None)
                    .with_detail(detail)
            }
            ThreadPromptOutcome::QueuedTaskFinished { task_id } => Self::stopped(
                thread_id,
                PromptSubmissionStatus::Dropped,
                ReasonCode::TaskFinished,
                Some(task_id.clone()),
            ),
            ThreadPromptOutcome::QueuedRejectedAgain { task_id } => Self::stopped(
                thread_id,
                PromptSubmissionStatus::Dropped,
                ReasonCode::RejectedAgain,
                Some(task_id.clone()),
            ),
        }
    }

    fn progress(thread_id: &str, status: PromptSubmissionStatus) -> Self {
        Self {
            thread_id: thread_id.to_string(),
            status,
            reason: None,
            next: Vec::new(),
            task_id: None,
            message: None,
        }
    }

    /// 이유의 설명 대신 그 일의 내용(초안 이름, 원인)을 싣는다. 내용이 없으면 그대로이다.
    fn with_detail(self, detail: Option<String>) -> Self {
        Self {
            message: detail.or(self.message),
            ..self
        }
    }

    /// 이유가 있는 상태이다. 설명과 다음 행동은 이유의 표에서 온다.
    fn stopped(
        thread_id: &str,
        status: PromptSubmissionStatus,
        reason: ReasonCode,
        task_id: Option<String>,
    ) -> Self {
        Self {
            thread_id: thread_id.to_string(),
            status,
            reason: Some(reason),
            next: reason_next(reason),
            task_id,
            message: Some(reason.summary().to_string()),
        }
    }
}

// planning ----------------------------------------------------------------------------------------

/*
 * planning 큐이다 (K3-6). 커널 작업 공간(저장소의 대표 checkout)의 planning 투영을 보낸다. 커널이 아직 읽지 않았으면
 * `uninitialized` 이다. 클라이언트는 붙은 뒤 `refresh_planning` 을 보낸다.
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanningState {
    pub status: PlanningStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_summary: Option<String>,
    /// `invalid` 의 원인이다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    /// planning 이 자동 후속을 멈춘 이유이다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_follow_pause_reason: Option<String>,
    /// 다음에 실행할 작업이다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_task: Option<PlanningTask>,
    pub active_tasks: Vec<PlanningTask>,
    /// 아직 받아들이지 않은 후속 제안이다.
    pub proposed_tasks: Vec<PlanningTask>,
    pub skipped_tasks: Vec<SkippedPlanningTask>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanningStatus {
    /// planning 이 없거나 커널이 아직 읽지 않았다.
    Uninitialized,
    Invalid,
    ReadyWithoutTask,
    ReadyWithTask,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanningTask {
    pub rank: usize,
    pub task_id: String,
    pub direction_title: String,
    pub title: String,
    pub status: PlanningTaskStatus,
    pub priority: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkippedPlanningTask {
    pub task_id: String,
    pub title: String,
    pub status: PlanningTaskStatus,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanningTaskStatus {
    Ready,
    Blocked,
    InProgress,
    Done,
    Cancelled,
    AwaitingUser,
    Proposed,
}

impl PlanningTaskStatus {
    fn from_core(status: TaskStatus) -> Self {
        match status {
            TaskStatus::Ready => Self::Ready,
            TaskStatus::Blocked => Self::Blocked,
            TaskStatus::InProgress => Self::InProgress,
            TaskStatus::Done => Self::Done,
            TaskStatus::Cancelled => Self::Cancelled,
            TaskStatus::AwaitingUser => Self::AwaitingUser,
            TaskStatus::Proposed => Self::Proposed,
        }
    }
}

impl PlanningTask {
    fn from_core(task: &PriorityQueueTask) -> Self {
        Self {
            rank: task.rank,
            task_id: task.task_id.clone(),
            direction_title: task.direction_title.clone(),
            title: task.task_title.clone(),
            status: PlanningTaskStatus::from_core(task.status),
            priority: task.combined_priority,
        }
    }
}

impl PlanningState {
    pub fn from_core(projection: &RuntimeProjection) -> Self {
        let queue = projection.queue_projection();
        let tasks = |pick: fn(
            &crate::domain::planning::PriorityQueueProjection,
        ) -> &Vec<PriorityQueueTask>| {
            queue
                .map(|queue| pick(queue).iter().map(PlanningTask::from_core).collect())
                .unwrap_or_default()
        };
        Self {
            status: match projection.workspace_status() {
                RuntimeWorkspaceStatus::Uninitialized => PlanningStatus::Uninitialized,
                RuntimeWorkspaceStatus::Invalid => PlanningStatus::Invalid,
                RuntimeWorkspaceStatus::ReadyNoTask => PlanningStatus::ReadyWithoutTask,
                RuntimeWorkspaceStatus::ReadyWithTask => PlanningStatus::ReadyWithTask,
            },
            revision: projection.planning_revision(),
            queue_summary: projection.queue_summary().map(str::to_string),
            failure_reason: projection.failure_reason().map(str::to_string),
            auto_follow_pause_reason: projection.auto_follow_pause_reason().map(str::to_string),
            next_task: queue
                .and_then(|queue| queue.next_task.as_ref())
                .or(projection.queue_head())
                .map(PlanningTask::from_core),
            active_tasks: tasks(|queue| &queue.active_tasks),
            proposed_tasks: tasks(|queue| &queue.proposed_tasks),
            skipped_tasks: queue
                .map(|queue| {
                    queue
                        .skipped_tasks
                        .iter()
                        .map(|task| SkippedPlanningTask {
                            task_id: task.task_id.clone(),
                            title: task.task_title.clone(),
                            status: PlanningTaskStatus::from_core(task.status),
                            reason: task.reason.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

// parallel ----------------------------------------------------------------------------------------

/// 커널 호스트가 모은 병렬 작업 상태이다. 호스트 타입을 모르게 하려고 값만 빌린다.
#[derive(Debug, Clone, Copy)]
pub struct ParallelSource<'a> {
    pub enabled: bool,
    pub supervisor: Option<&'a ParallelModeSupervisorSnapshot>,
    pub status: Option<&'a str>,
}

/*
 * 병렬 작업 상태이다 (K3-6). 감독 스냅샷은 TUI 의 병렬 화면과 같은 값이다. 상태 값(`state`, `phase`)은 정해진
 * 값이다 (K4-2). 전송되는 글자는 이전과 같다: TUI 의 표시 이름을 snake_case 로 쓴 것이다.
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ParallelState {
    pub enabled: bool,
    /// 병렬 제어부가 마지막으로 알린 상태 문구이다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// 커널이 아직 감독 스냅샷을 읽지 않았으면 없다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supervisor: Option<ParallelSupervisor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ParallelSupervisor {
    pub state: ParallelSupervisorState,
    pub pool: ParallelPool,
    pub agents: Vec<ParallelAgent>,
    /// 병합 큐의 머리 요약이다.
    pub merge_head: String,
    pub merge_queue: Vec<ParallelMergeItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_validation: Option<ParallelPrValidation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParallelSupervisorState {
    Prepare,
    Supervise,
    Recover,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ParallelPool {
    pub configured: usize,
    pub idle: usize,
    pub leased: usize,
    pub running: usize,
    pub awaiting_cleanup: usize,
    pub blocked: usize,
    pub missing: usize,
    pub unavailable: usize,
    pub exhausted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ParallelAgent {
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub task_title: String,
    pub slot_id: String,
    pub branch: String,
    pub state: ParallelAgentState,
    pub duration: String,
    pub summary: String,
}

/// 에이전트의 상태이다 (plan 18 K4-2). domain 의 roster 상태와 같은 이름이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParallelAgentState {
    Starting,
    Running,
    CleanupPending,
    ReportedComplete,
    LedgerRefreshing,
    CommitReady,
    MergeQueued,
    Pushing,
    PrPending,
    MergePending,
    Integrating,
    OfficialRefreshRecoveryNeeded,
    Failed,
    // roster 가 만든 상태가 아니다. 커널이 만든 행에는 나오지 않는다. 문서 주석으로 쓰지 않는다: 문서 주석이 있는
    // variant 는 스키마에서 따로 나뉘어, 값의 목록이 하나의 `enum` 으로 나오지 않는다.
    Unknown,
}

impl ParallelAgentState {
    fn from_core(state: Option<ParallelModeAgentRosterState>) -> Self {
        use ParallelModeAgentRosterState as Roster;
        match state {
            None => Self::Unknown,
            Some(Roster::Starting) => Self::Starting,
            Some(Roster::Running) => Self::Running,
            Some(Roster::CleanupPending) => Self::CleanupPending,
            Some(Roster::ReportedComplete) => Self::ReportedComplete,
            Some(Roster::LedgerRefreshing) => Self::LedgerRefreshing,
            Some(Roster::CommitReady) => Self::CommitReady,
            Some(Roster::MergeQueued) => Self::MergeQueued,
            Some(Roster::Pushing) => Self::Pushing,
            Some(Roster::PrPending) => Self::PrPending,
            Some(Roster::MergePending) => Self::MergePending,
            Some(Roster::Integrating) => Self::Integrating,
            Some(Roster::OfficialRefreshRecoveryNeeded) => Self::OfficialRefreshRecoveryNeeded,
            Some(Roster::Failed) => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ParallelMergeItem {
    pub task_title: String,
    pub state: ParallelMergeState,
    pub branch: String,
    pub commit: String,
}

/// 병합 큐 항목의 상태이다 (plan 18 K4-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParallelMergeState {
    Idle,
    Queued,
    Pushing,
    PrPending,
    MergePending,
    Integrating,
    Cleaning,
    Done,
    Blocked,
    Failed,
}

impl ParallelMergeState {
    fn from_core(state: ParallelModeQueueItemState) -> Self {
        match state {
            ParallelModeQueueItemState::Idle => Self::Idle,
            ParallelModeQueueItemState::Queued => Self::Queued,
            ParallelModeQueueItemState::Pushing => Self::Pushing,
            ParallelModeQueueItemState::PrPending => Self::PrPending,
            ParallelModeQueueItemState::MergePending => Self::MergePending,
            ParallelModeQueueItemState::Integrating => Self::Integrating,
            ParallelModeQueueItemState::Cleaning => Self::Cleaning,
            ParallelModeQueueItemState::Done => Self::Done,
            ParallelModeQueueItemState::Blocked => Self::Blocked,
            ParallelModeQueueItemState::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ParallelPrValidation {
    pub pull_request_number: u64,
    pub url: String,
    pub state: ParallelPrValidationState,
    pub phase: ParallelPrValidationPhase,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
}

/// PR 검증이 사람에게 요구하는 것으로 본 상태이다 (plan 18 K4-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParallelPrValidationState {
    Pending,
    Blocking,
    Remediation,
    Terminal,
    Blocked,
    Failed,
}

impl ParallelPrValidationState {
    fn from_core(state: PrValidationOperatorState) -> Self {
        match state {
            PrValidationOperatorState::Pending => Self::Pending,
            PrValidationOperatorState::Blocking => Self::Blocking,
            PrValidationOperatorState::Remediation => Self::Remediation,
            PrValidationOperatorState::Terminal => Self::Terminal,
            PrValidationOperatorState::Blocked => Self::Blocked,
            PrValidationOperatorState::Failed => Self::Failed,
        }
    }
}

/// PR 검증의 단계이다 (plan 18 K4-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParallelPrValidationPhase {
    Registered,
    PreMergeObservation,
    RemediationQueued,
    RemediationRunning,
    PostMergeObservation,
    Settled,
    Blocked,
    Failed,
}

impl ParallelPrValidationPhase {
    fn from_core(phase: PrValidationPhase) -> Self {
        match phase {
            PrValidationPhase::Registered => Self::Registered,
            PrValidationPhase::PreMergeObservation => Self::PreMergeObservation,
            PrValidationPhase::RemediationQueued => Self::RemediationQueued,
            PrValidationPhase::RemediationRunning => Self::RemediationRunning,
            PrValidationPhase::PostMergeObservation => Self::PostMergeObservation,
            PrValidationPhase::Settled => Self::Settled,
            PrValidationPhase::Blocked => Self::Blocked,
            PrValidationPhase::Failed => Self::Failed,
        }
    }
}

impl ParallelState {
    pub fn from_core(source: ParallelSource<'_>) -> Self {
        Self {
            enabled: source.enabled,
            status: source.status.map(str::to_string),
            supervisor: source.supervisor.map(ParallelSupervisor::from_core),
        }
    }
}

impl ParallelSupervisor {
    fn from_core(snapshot: &ParallelModeSupervisorSnapshot) -> Self {
        let pool = &snapshot.pool;
        Self {
            state: match snapshot.state {
                ParallelModeSupervisorState::Prepare => ParallelSupervisorState::Prepare,
                ParallelModeSupervisorState::Supervise => ParallelSupervisorState::Supervise,
                ParallelModeSupervisorState::Recover => ParallelSupervisorState::Recover,
            },
            pool: ParallelPool {
                configured: pool.configured_size,
                idle: pool.idle_slots,
                leased: pool.leased_slots,
                running: pool.running_slots,
                awaiting_cleanup: pool.awaiting_cleanup_slots,
                blocked: pool.blocked_slots,
                missing: pool.missing_slots,
                unavailable: pool.unavailable_slots,
                exhausted: pool.exhausted,
            },
            agents: snapshot
                .roster
                .entries
                .iter()
                .map(|entry| ParallelAgent {
                    agent_id: entry.agent_id.clone(),
                    thread_id: entry.thread_id.clone(),
                    task_title: entry.task_title.clone(),
                    slot_id: entry.slot_id.clone(),
                    branch: entry.branch_name.clone(),
                    state: ParallelAgentState::from_core(entry.state()),
                    duration: entry.duration_label.clone(),
                    summary: entry.latest_summary.clone(),
                })
                .collect(),
            merge_head: snapshot.distributor.head_summary.clone(),
            merge_queue: snapshot
                .distributor
                .queue_items
                .iter()
                .map(|item| ParallelMergeItem {
                    task_title: item.task_title.clone(),
                    state: ParallelMergeState::from_core(item.queue_state),
                    branch: item.branch_name.clone(),
                    commit: item.commit_short_sha.clone(),
                })
                .collect(),
            pr_validation: snapshot
                .pr_validation
                .as_ref()
                .map(|summary| ParallelPrValidation {
                    pull_request_number: summary.pull_request_number,
                    url: summary.canonical_pr_url.clone(),
                    state: ParallelPrValidationState::from_core(summary.state),
                    phase: ParallelPrValidationPhase::from_core(summary.phase),
                    reason: summary.reason.clone(),
                    blocker: summary.blocker.clone(),
                }),
            notice: snapshot.top_notice.clone(),
        }
    }
}
