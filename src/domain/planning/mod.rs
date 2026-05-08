/*
 * planning 도메인 모듈은 Akra의 작업 계획 원장을 표현하는 순수 도메인 계층이다. 이 파일은
 * JSON/TOML/DB에서 읽혀 온 계획 데이터를 Rust 타입으로 고정하고, application/service 계층이 같은
 * 단어로 대화하도록 공통 계약을 제공한다.
 *
 * 연결 흐름은 adapter가 `DirectionCatalogDocument`와 `TaskAuthorityDocument`를 읽고, validation
 * service가 이 타입들로 문서 의미를 검사한 뒤, `queue.rs`의 `PriorityQueueService`가 같은 타입을
 * 입력으로 다음 실행 후보를 계산하는 식이다. TUI와 app-server adapter는 `PriorityQueueProjection`을
 * 화면 문구나 하위 세션 handoff prompt로 변환한다.
 *
 * 그래서 이 파일의 enum/struct는 단순 데이터 묶음이 아니라, adapter -> application -> domain 경계를
 * 통과할 때 의미가 흐트러지지 않게 붙잡아 주는 중심 어휘다.
 */
use serde::{Deserialize, Serialize};

pub(crate) mod mutation;
pub(crate) mod promotion;
mod queue;
pub(crate) mod queue_follow;
mod validation;

pub(crate) use mutation::{PlanningTaskMutationPolicy, TaskDescriptionUpdateDecision};
pub(crate) use promotion::{PlanningProposalPromotionDecision, PlanningProposalPromotionPolicy};
pub use queue::PriorityQueueService;
pub(crate) use queue_follow::{
    PlanningQueueFollowBlockReason, PlanningQueueFollowDecision, PlanningQueueFollowFacts,
    PlanningQueueFollowPolicy, PlanningQueueFollowPromptMode,
};
pub use validation::PlanningSemanticValidationService;

// planning authority 문서의 schema version이다. adapter와 validation은 이 값을 기준으로 호환성을 판단한다.
pub const PLANNING_FORMAT_VERSION: u32 = 1;
// official completion refresh contract는 worker 완료 통지를 planning ledger에 반영하는 별도 wire contract다.
pub const PLANNING_OFFICIAL_COMPLETION_REFRESH_CONTRACT_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningWorkspaceState {
    /*
     * workspace state는 planning runtime이 "지금 operator에게 무엇을 보여줄지" 결정하는 큰 상태값이다.
     * Ready/Executing/Repairing/BlockedInvalid는 UI copy, 자동 후속 실행 정책, repair prompt 선택으로 이어진다.
     */
    Uninitialized,
    Ready,
    Executing,
    Repairing,
    BlockedInvalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningAuthorityLocation {
    // workspace root는 operator가 작업 중인 repo/worktree 기준점이다.
    pub workspace_root: String,
    // canonical repo root는 shadow store와 branch/worktree bookkeeping이 공유하는 정규화된 root다.
    pub canonical_repo_root: String,
    // runtime dir은 planning authority mirror와 transient runtime artifacts가 놓이는 위치다.
    pub runtime_dir: String,
    // authority store path는 DB/filesystem adapter가 실제 planning authority를 찾는 persistent boundary다.
    pub authority_store_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningAuthorityShadowStoreSyncState {
    // 새 shadow store가 만들어져 현재 authority를 처음 mirror한 상태다.
    Bootstrapped,
    // source authority와 shadow store가 이미 같은 상태였다.
    InSync,
    // drift나 누락을 발견해 다시 mirror한 상태다.
    Resynced,
}

impl PlanningAuthorityShadowStoreSyncState {
    // status copy와 logs가 enum 이름 대신 stable snake_case label을 쓰도록 고정한다.
    pub fn label(self) -> &'static str {
        match self {
            Self::Bootstrapped => "bootstrapped",
            Self::InSync => "in_sync",
            Self::Resynced => "resynced",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningAuthorityShadowStoreInspection {
    // inspection이 어떤 workspace/store를 봤는지 함께 싣는다.
    pub location: PlanningAuthorityLocation,
    // sync_state는 mirror가 만들어졌는지, 이미 동기였는지, 재동기화됐는지를 요약한다.
    pub sync_state: PlanningAuthorityShadowStoreSyncState,
    pub mirrored_document_count: usize,
    pub parity_issue_count: usize,
    // parity issue 전체를 UI에 다 싣지 않고 대표 예시만 보낸다.
    pub parity_issue_examples: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningFileKind {
    // direction authority file에서 나온 validation issue다.
    Directions,
    // task authority file에서 나온 validation issue다.
    TaskAuthority,
    // worker result/output markdown에서 나온 issue다.
    ResultOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningValidationSeverity {
    // promotion/execution을 막는 문제다.
    Error,
    // 실행은 가능하지만 operator가 봐야 하는 degraded state다.
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningValidationIssue {
    // UI와 mutation service가 block 여부를 판단하는 severity다.
    pub severity: PlanningValidationSeverity,
    // issue가 어느 authority artifact에 속하는지 나타낸다.
    pub file_kind: PlanningFileKind,
    // machine-readable issue code다. tests와 repair prompt가 이 값을 기준으로 분기할 수 있다.
    pub code: String,
    // operator-facing detail이다. validation service가 구체적인 id/path를 포함해 채운다.
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlanningValidationReport {
    // validation은 fail-fast가 아니라 report accumulation 방식이라 issue list를 그대로 보관한다.
    pub issues: Vec<PlanningValidationIssue>,
}

impl PlanningValidationReport {
    // 새 validation run마다 빈 report를 만들고 각 검사 pass가 issue를 추가한다.
    pub fn new() -> Self {
        Self { issues: Vec::new() }
    }

    // warning은 promotion을 막지 않으므로 error만 검사한다.
    pub fn has_errors(&self) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.severity == PlanningValidationSeverity::Error)
    }

    // application service와 UI가 promote 가능 여부를 읽는 가장 짧은 predicate다.
    pub fn is_valid(&self) -> bool {
        !self.has_errors()
    }

    // validation pass는 file kind와 code를 함께 넣어 repair/admin surface가 위치와 원인을 분리해서 보여 주게 한다.
    pub fn push_error(
        &mut self,
        file_kind: PlanningFileKind,
        code: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.issues.push(PlanningValidationIssue {
            severity: PlanningValidationSeverity::Error,
            file_kind,
            code: code.into(),
            message: message.into(),
        });
    }

    // warning은 report에 남지만 `is_valid`에는 영향을 주지 않는다.
    pub fn push_warning(
        &mut self,
        file_kind: PlanningFileKind,
        code: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.issues.push(PlanningValidationIssue {
            severity: PlanningValidationSeverity::Warning,
            file_kind,
            code: code.into(),
            message: message.into(),
        });
    }

    // callers that need blocking issues only can use this filtered view without copying messages manually.
    pub fn errors(&self) -> Vec<&PlanningValidationIssue> {
        self.issues
            .iter()
            .filter(|issue| issue.severity == PlanningValidationSeverity::Error)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectionCatalogDocument {
    /*
     * direction catalog는 "왜 이 일을 하는가"를 담는 상위 계획 문서다. 각 `DirectionDefinition`은
     * 여러 `TaskDefinition`의 부모가 되며, queue builder는 `task.direction_id`를 통해 이 문서의
     * direction과 연결한다. direction이 paused/done이면 하위 task가 ready여도 queue에서 제외된다.
     */
    // planning authority schema version이다.
    pub version: u32,
    #[serde(default)]
    // queue가 비었을 때 멈출지, review prompt로 새 작업을 제안할지 정하는 direction-level policy다.
    pub queue_idle: QueueIdleConfig,
    // operator-facing workstream definitions다.
    pub directions: Vec<DirectionDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectionDefinition {
    // task.direction_id가 참조하는 stable id다.
    pub id: String,
    // TUI/admin/prompt에서 direction을 짧게 식별하는 제목이다.
    pub title: String,
    // worker prompt가 방향성을 이해할 수 있게 하는 설명이다.
    pub summary: String,
    // completion/repair 판단에 쓰는 operator-authored success criteria다.
    pub success_criteria: Vec<String>,
    #[serde(default)]
    // worker에게 범위를 좁혀 주는 선택적 hint다.
    pub scope_hints: Vec<String>,
    #[serde(default)]
    // 자세한 direction markdown 문서의 상대 경로다.
    pub detail_doc_path: String,
    // queue inclusion을 결정하는 direction lifecycle state다.
    pub state: DirectionState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueIdleConfig {
    #[serde(default)]
    // executable task가 없을 때 runtime이 취할 policy다.
    pub policy: QueueIdlePolicy,
    #[serde(default)]
    // review-and-enqueue flow가 사용할 queue-idle prompt markdown 경로다.
    pub prompt_path: String,
}

impl Default for QueueIdleConfig {
    // 명시 policy가 없는 기존 authority 문서는 idle 상태에서 멈추는 쪽을 기본값으로 둔다.
    fn default() -> Self {
        Self {
            policy: QueueIdlePolicy::Stop,
            prompt_path: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueIdlePolicy {
    #[default]
    // queue가 비면 operator input을 기다린다.
    Stop,
    // queue가 비면 review prompt를 통해 후속 task proposal을 만들 수 있다.
    ReviewAndEnqueue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectionState {
    // active direction만 executable queue에 들어갈 수 있다.
    Active,
    // paused direction은 보존하되 실행 후보에서 제외한다.
    Paused,
    // done direction은 완료된 workstream이라 하위 ready task도 실행 후보에서 제외한다.
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskAuthorityDocument {
    // task authority schema version이다.
    pub version: u32,
    #[serde(default)]
    // 실행 단위의 source-of-truth list다.
    pub tasks: Vec<TaskDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDefinition {
    /*
     * `TaskDefinition`은 실제 실행 단위의 원본 authority다. queue builder가 `PriorityQueueTask`로
     * 복사하기 전까지는 이 타입이 source of truth이고, validation은 이 구조체의 필드 조합을 검사해
     * worker-authored task의 relation note, dependency/blocker reference, status semantics를 보장한다.
     */
    // task graph node id다.
    pub id: String,
    // parent direction id다.
    pub direction_id: String,
    #[serde(default)]
    // worker가 만든 task가 어떤 direction을 어떻게 만족시키는지 설명하는 audit note다.
    pub direction_relation_note: String,
    // queue/admin/prompt에 노출되는 task title이다.
    pub title: String,
    // worker handoff prompt의 중심 설명이다.
    pub description: String,
    // queue inclusion과 validation semantics를 결정하는 lifecycle state다.
    pub status: TaskStatus,
    // operator가 부여한 기본 우선순위다.
    pub base_priority: i32,
    #[serde(default)]
    // runtime이나 operator가 일시적으로 더하는 priority adjustment다.
    pub dynamic_priority_delta: i32,
    #[serde(default)]
    // dynamic priority가 0이 아닐 때 이유를 남기는 audit field다.
    pub priority_reason: String,
    #[serde(default)]
    // 완료되어야 이 task가 실행 가능한 dependency ids다.
    pub depends_on: Vec<String>,
    #[serde(default)]
    // 해소되어야 이 task가 막히지 않는 blocker ids다.
    pub blocked_by: Vec<String>,
    // 최초 생성 주체다. worker-authored relation note policy에 쓰인다.
    pub created_by: TaskActor,
    // 마지막 수정 주체다. worker가 수정한 task도 relation note를 요구한다.
    pub last_updated_by: TaskActor,
    #[serde(default)]
    // legacy 조회용 source turn id다. 새 감사 정보는 provider-neutral provenance.turn_id를 우선 사용한다.
    pub source_turn_id: Option<String>,
    #[serde(default)]
    // task를 생성하거나 마지막으로 의미 있게 수정한 runtime session/turn provenance다.
    pub provenance: TaskMutationProvenance,
    // RFC3339 timestamp string이다. validation이 형식을 검사한다.
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskMutationProvenance {
    /*
     * provenance는 provider 이름과 무관한 Akra runtime 감사 정보다.
     * thread_id/turn_id는 실제 mutation을 만든 session/turn이고, parent_*는 hidden/planning worker/parallel
     * mutation을 유발한 visible 또는 상위 실행 단위를 가리킨다.
     */
    #[serde(default)]
    pub origin_session_kind: Option<OriginSessionKind>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub parent_thread_id: Option<String>,
    #[serde(default)]
    pub parent_turn_id: Option<String>,
}

impl TaskMutationProvenance {
    pub fn new(origin_session_kind: OriginSessionKind) -> Self {
        Self {
            origin_session_kind: Some(origin_session_kind),
            ..Self::default()
        }
    }

    pub fn with_thread_turn(mut self, thread_id: Option<String>, turn_id: Option<String>) -> Self {
        self.thread_id = normalize_optional_id(thread_id);
        self.turn_id = normalize_optional_id(turn_id);
        self
    }

    pub fn with_parent(
        mut self,
        parent_thread_id: Option<String>,
        parent_turn_id: Option<String>,
    ) -> Self {
        self.parent_thread_id = normalize_optional_id(parent_thread_id);
        self.parent_turn_id = normalize_optional_id(parent_turn_id);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.origin_session_kind.is_none()
            && self.thread_id.is_none()
            && self.turn_id.is_none()
            && self.parent_thread_id.is_none()
            && self.parent_turn_id.is_none()
    }
}

fn normalize_optional_id(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginSessionKind {
    Main,
    ManualIntake,
    Planner,
    Parallel,
    System,
}

impl OriginSessionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::ManualIntake => "manual_intake",
            Self::Planner => "planner",
            Self::Parallel => "parallel",
            Self::System => "system",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    // dependency/blocker가 해소되면 실행 후보가 될 수 있다.
    Ready,
    // 명시적으로 막힌 task다.
    Blocked,
    // 현재 진행 중인 task다. queue rank에서 ready보다 우선한다.
    InProgress,
    // 완료되어 dependency를 만족시키는 task다.
    Done,
    // 더 진행하지 않는 task다. blocker 해소 관점에서는 막지 않는 상태로 취급한다.
    Cancelled,
    // 자동 worker가 아니라 사용자 응답을 기다리는 상태다.
    AwaitingUser,
    // worker가 제안했지만 아직 authority로 승격되지 않은 task다.
    Proposed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskActor {
    // operator나 explicit user action이 만든 변경이다.
    User,
    // worker가 만든 변경이다. 기존 task-authority JSON의 "llm" 값도 계속 읽는다.
    #[serde(alias = "llm")]
    Worker,
    // system bootstrap/repair가 만든 변경이다.
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriorityQueueProjection {
    // 지금 바로 실행할 하나의 후보다.
    pub next_task: Option<PriorityQueueTask>,
    // queue에서 visible한 executable/active tasks다.
    pub active_tasks: Vec<PriorityQueueTask>,
    // 아직 promote되지 않은 follow-up proposals다.
    pub proposed_tasks: Vec<PriorityQueueTask>,
    // queue에서 제외된 task와 그 이유다.
    pub skipped_tasks: Vec<PriorityQueueSkippedTask>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriorityQueueTask {
    // queue ordering 결과의 1-based rank다.
    pub rank: usize,
    pub task_id: String,
    pub direction_id: String,
    pub direction_title: String,
    pub task_title: String,
    pub status: TaskStatus,
    pub combined_priority: i32,
    pub updated_at: String,
    // queue builder가 왜 이 rank가 나왔는지 설명하는 audit trail이다.
    pub rank_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriorityQueueSkippedTask {
    pub task_id: String,
    pub task_title: String,
    pub direction_id: String,
    pub status: TaskStatus,
    // blocked/done/paused direction 같은 skip reason을 operator-facing text로 담는다.
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningRefreshContractKind {
    // official worker completion을 planning ledger에 다시 반영하는 refresh다.
    OfficialCompletion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningOfficialCompletionRefreshPayload {
    // 완료를 보고한 parallel/worker agent id다.
    pub agent_id: String,
    pub task_id: String,
    pub task_title: String,
    pub branch_name: String,
    pub worktree_path: String,
    pub commit_sha: String,
    // worker가 실행한 validation/test summary다.
    pub validation_summary: String,
    // ledger와 UI에 짧게 반영할 완료 요약이다.
    pub final_response_summary: String,
    #[serde(default)]
    // 필요할 때 더 긴 final response를 보존한다.
    pub final_response_text: Option<String>,
    #[serde(default)]
    // 실패/부분 완료의 맥락이다. 성공 payload에서는 비어 있을 수 있다.
    pub failure_context: Option<String>,
    // completion event timestamp다.
    pub completed_at: String,
}

impl PlanningOfficialCompletionRefreshPayload {
    #[allow(clippy::too_many_arguments)]
    // payload는 wire contract라 field가 많다. builder struct 대신 explicit constructor로 callsite intent를 보존한다.
    pub fn new(
        agent_id: impl Into<String>,
        task_id: impl Into<String>,
        task_title: impl Into<String>,
        branch_name: impl Into<String>,
        worktree_path: impl Into<String>,
        commit_sha: impl Into<String>,
        validation_summary: impl Into<String>,
        final_response_summary: impl Into<String>,
        final_response_text: Option<String>,
        failure_context: Option<String>,
        completed_at: impl Into<String>,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            task_id: task_id.into(),
            task_title: task_title.into(),
            branch_name: branch_name.into(),
            worktree_path: worktree_path.into(),
            commit_sha: commit_sha.into(),
            validation_summary: validation_summary.into(),
            final_response_summary: final_response_summary.into(),
            final_response_text,
            failure_context,
            completed_at: completed_at.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningOfficialCompletionRefreshContract {
    // refresh contract schema version이다.
    pub version: u32,
    // future refresh 종류 확장을 위한 discriminator다.
    pub refresh_kind: PlanningRefreshContractKind,
    // refresh를 유발한 완료 turn id다. 오래된 저장/로그 payload의 root_turn_id도 입력 호환한다.
    #[serde(alias = "root_turn_id")]
    pub completed_turn_id: String,
    // 같은 완료 turn에 여러 completion이 들어올 때 ordering을 고정한다.
    pub refresh_order: u64,
    pub completion: PlanningOfficialCompletionRefreshPayload,
}

impl PlanningOfficialCompletionRefreshContract {
    // current official completion contract의 version/kind를 한곳에서 고정한다.
    pub fn new(
        completed_turn_id: impl Into<String>,
        refresh_order: u64,
        completion: PlanningOfficialCompletionRefreshPayload,
    ) -> Self {
        Self {
            version: PLANNING_OFFICIAL_COMPLETION_REFRESH_CONTRACT_VERSION,
            refresh_kind: PlanningRefreshContractKind::OfficialCompletion,
            completed_turn_id: completed_turn_id.into(),
            refresh_order,
            completion,
        }
    }
}

impl DirectionState {
    // queue builder는 active direction만 실행 후보로 본다.
    pub fn allows_queue_execution(self) -> bool {
        self == Self::Active
    }

    // UI/admin copy가 serde spelling과 같은 label을 쓰게 한다.
    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Done => "done",
        }
    }
}

impl QueueIdlePolicy {
    // status line과 prompt copy에서 policy를 stable label로 보여 준다.
    pub fn label(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::ReviewAndEnqueue => "review_and_enqueue",
        }
    }
}

impl TaskStatus {
    // queue ordering에서 실행 가능한 status만 rank를 갖는다.
    pub fn queue_readiness_rank(self) -> Option<u8> {
        /*
         * InProgress가 0, Ready가 1인 이유는 이미 시작된 작업을 새 ready 작업보다 먼저 이어가야 하기
         * 때문이다. None을 반환하는 상태는 "queue에 올릴 수는 있지만 실행 후보는 아니다"라는 뜻이라
         * queue builder에서 skipped/proposed로 분기된다.
         */
        match self {
            Self::InProgress => Some(0),
            Self::Ready => Some(1),
            Self::Blocked | Self::Done | Self::Cancelled | Self::AwaitingUser | Self::Proposed => {
                None
            }
        }
    }

    // persisted snake_case와 UI label을 맞춘다.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Blocked => "blocked",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
            Self::AwaitingUser => "awaiting_user",
            Self::Proposed => "proposed",
        }
    }

    // terminal status는 historical record라 generic update path에서 재분류할 수 없다.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Cancelled)
    }

    // Done task만 dependency를 만족시킨다.
    pub fn is_dependency_complete(self) -> bool {
        self == Self::Done
    }

    // blocker는 "worker가 더 기다려야 하는가" 관점이라 dependency completion보다 넓게 해소 상태를 본다.
    pub fn clears_blocker(self) -> bool {
        /*
         * Done은 완료라서 막지 않고, Cancelled는 더 진행하지 않으므로 막지 않는다. AwaitingUser는 자동
         * 실행 관점에서 worker가 해결할 수 없는 사용자 대기 상태라 queue가 계속 막히지 않도록 해제
         * 상태로 취급한다.
         */
        matches!(self, Self::Done | Self::Cancelled | Self::AwaitingUser)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PLANNING_OFFICIAL_COMPLETION_REFRESH_CONTRACT_VERSION,
        PlanningOfficialCompletionRefreshContract, PlanningOfficialCompletionRefreshPayload,
        PlanningRefreshContractKind, TaskActor,
    };

    #[test]
    fn official_completion_refresh_contract_round_trips_as_versioned_json() {
        let contract = PlanningOfficialCompletionRefreshContract::new(
            "turn-42",
            7,
            PlanningOfficialCompletionRefreshPayload::new(
                "agent-2",
                "task-9",
                "Official completion pipeline 구현",
                "akra-agent/slot-1/official-completion",
                "/tmp/slot-1",
                "abc123def456",
                "cargo test passed",
                "official completion lifecycle wired",
                Some("Implemented official completion reporting.".to_string()),
                None,
                "2026-04-17T09:10:00Z",
            ),
        );

        let serialized =
            serde_json::to_string_pretty(&contract).expect("contract should serialize");
        assert!(serialized.contains("\"completed_turn_id\""));
        assert!(!serialized.contains("\"root_turn_id\""));
        let restored: PlanningOfficialCompletionRefreshContract =
            serde_json::from_str(&serialized).expect("contract should deserialize");

        assert_eq!(
            restored.version,
            PLANNING_OFFICIAL_COMPLETION_REFRESH_CONTRACT_VERSION
        );
        assert_eq!(
            restored.refresh_kind,
            PlanningRefreshContractKind::OfficialCompletion
        );
        assert_eq!(restored.completed_turn_id, "turn-42");
        assert_eq!(restored.refresh_order, 7);
        assert_eq!(restored.completion.task_id, "task-9");
        assert_eq!(
            restored.completion.final_response_text.as_deref(),
            Some("Implemented official completion reporting.")
        );

        let legacy_json = serialized.replace("\"completed_turn_id\"", "\"root_turn_id\"");
        let restored_legacy: PlanningOfficialCompletionRefreshContract =
            serde_json::from_str(&legacy_json).expect("legacy contract should deserialize");
        assert_eq!(restored_legacy.completed_turn_id, "turn-42");
    }

    #[test]
    fn legacy_llm_task_actor_deserializes_as_worker() {
        /*
         * 오래된 task-authority JSON은 actor를 "llm"으로 저장했다. 새 code vocabulary는
         * worker를 쓰지만 기존 authority snapshot은 migration 없이 읽을 수 있어야 한다.
         */
        let restored: TaskActor =
            serde_json::from_str("\"llm\"").expect("legacy actor should deserialize");

        assert_eq!(restored, TaskActor::Worker);
        assert_eq!(
            serde_json::to_string(&TaskActor::Worker).expect("actor should serialize"),
            "\"worker\""
        );
    }
}

impl TaskDefinition {
    // validation이 worker-authored relation note policy를 중복하지 않도록 이 domain helper를 쓴다.
    pub fn requires_relation_note(&self) -> bool {
        self.created_by == TaskActor::Worker || self.last_updated_by == TaskActor::Worker
    }

    // base priority와 runtime/operator adjustment를 합친 queue ordering 점수다.
    pub fn combined_priority(&self) -> i32 {
        self.base_priority + self.dynamic_priority_delta
    }

    // equality/diff에서 link ordering noise를 줄이기 위한 normalized copy다.
    pub fn normalized(&self) -> Self {
        let mut normalized = self.clone();
        normalized.depends_on.sort();
        normalized.blocked_by.sort();
        normalized
    }
}

#[derive(Debug, Clone)]
pub struct PlanningWorkspaceFiles<'a> {
    // parsed directions authority다.
    pub directions: &'a DirectionCatalogDocument,
    // task authority는 caller가 JSON text로 다시 저장/검증할 수 있게 raw text를 보존한다.
    pub task_authority_json: &'a str,
    // worker result markdown의 current raw contents다.
    pub result_output_markdown: &'a str,
}

#[derive(Debug, Clone)]
pub struct PlanningValidationResult {
    // parse가 성공한 directions document다. parse failure면 None이고 report에 issue가 남는다.
    pub directions: Option<DirectionCatalogDocument>,
    // parse가 성공한 task authority document다.
    pub task_authority: Option<TaskAuthorityDocument>,
    // parse/semantic validation issue를 누적한 report다.
    pub report: PlanningValidationReport,
}

impl PlanningValidationResult {
    // parsed documents가 있어도 report에 error가 있으면 promote/execution은 막힌다.
    pub fn is_valid(&self) -> bool {
        self.report.is_valid()
    }
}

impl PriorityQueueProjection {
    // shell/status copy에서 실행 가능한 queue head를 한 줄로 보여 주기 위한 summary다.
    pub fn queue_summary(&self) -> String {
        match self.next_task.as_ref() {
            Some(task) => format!(
                "queue head: rank {} / {} / {} / priority {}",
                task.rank,
                task.task_id.trim(),
                task.task_title.trim(),
                task.combined_priority,
            ),
            None => "queue idle: no executable planning task".to_string(),
        }
    }

    // proposed task가 있을 때 footer/status에 표시할 짧은 summary다.
    pub fn proposal_summary(&self, max_visible_titles: usize) -> Option<String> {
        if self.proposed_tasks.is_empty() {
            return None;
        }

        let task_titles = self
            .proposed_tasks
            .iter()
            .map(|task| task.task_title.trim())
            .filter(|title| !title.is_empty())
            .take(max_visible_titles)
            .collect::<Vec<_>>();
        let remaining_count = self.proposed_tasks.len().saturating_sub(task_titles.len());
        let title_segment = if task_titles.is_empty() {
            String::new()
        } else {
            let mut segment = format!(": {}", task_titles.join(" | "));
            if remaining_count > 0 {
                segment.push_str(&format!(" | +{remaining_count} more"));
            }
            segment
        };

        Some(format!(
            "{} promotable follow-up proposal{} available{}",
            self.proposed_tasks.len(),
            if self.proposed_tasks.len() == 1 {
                ""
            } else {
                "s"
            },
            title_segment,
        ))
    }

    // TUI list는 전체 queue 대신 limit만큼의 stable clone을 받는다.
    pub fn visible_tasks(&self, limit: usize) -> Vec<PriorityQueueTask> {
        self.active_tasks.iter().take(limit).cloned().collect()
    }

    // proposed task panel도 active queue와 같은 pagination contract를 쓴다.
    pub fn visible_proposed_tasks(&self, limit: usize) -> Vec<PriorityQueueTask> {
        self.proposed_tasks.iter().take(limit).cloned().collect()
    }
}

#[cfg(test)]
mod priority_queue_projection_tests {
    use super::{PriorityQueueProjection, PriorityQueueSkippedTask, PriorityQueueTask, TaskStatus};

    fn queue_task(rank: usize, task_id: &str, task_title: &str) -> PriorityQueueTask {
        PriorityQueueTask {
            rank,
            task_id: task_id.to_string(),
            direction_id: "general-workstream".to_string(),
            direction_title: "General workstream".to_string(),
            task_title: task_title.to_string(),
            status: TaskStatus::Ready,
            combined_priority: 80,
            updated_at: "2026-04-30T00:00:00Z".to_string(),
            rank_reasons: vec!["status=ready".to_string()],
        }
    }

    fn projection(
        next_task: Option<PriorityQueueTask>,
        proposed_tasks: Vec<PriorityQueueTask>,
    ) -> PriorityQueueProjection {
        PriorityQueueProjection {
            next_task,
            active_tasks: Vec::new(),
            proposed_tasks,
            skipped_tasks: Vec::<PriorityQueueSkippedTask>::new(),
        }
    }

    #[test]
    fn queue_summary_projects_queue_head_details() {
        let projection = projection(
            Some(queue_task(1, " task-1 ", " Extract domain summary ")),
            Vec::new(),
        );

        assert_eq!(
            projection.queue_summary(),
            "queue head: rank 1 / task-1 / Extract domain summary / priority 80"
        );
    }

    #[test]
    fn queue_summary_reports_idle_when_no_task_is_executable() {
        let projection = projection(None, Vec::new());

        assert_eq!(
            projection.queue_summary(),
            "queue idle: no executable planning task"
        );
    }

    #[test]
    fn proposal_summary_projects_count_titles_and_overflow() {
        let projection = projection(
            None,
            vec![
                queue_task(1, "proposal-1", " Plan A "),
                queue_task(2, "proposal-2", "Plan B"),
                queue_task(3, "proposal-3", "Plan C"),
            ],
        );

        assert_eq!(
            projection.proposal_summary(2).as_deref(),
            Some("3 promotable follow-up proposals available: Plan A | Plan B | +1 more")
        );
    }
}
