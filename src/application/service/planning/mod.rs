/*
 * planning/mod.rs는 planning application service의 공개 표면을 정한다. 하위 구현 모듈은 가능한 한
 * crate-local/private로 두고, adapter가 실제로 호출해야 하는 facade, request/response, projection,
 * domain payload만 이 파일에서 re-export한다. 그래서 TUI/CLI/admin/Telegram adapter는 내부 파일 구조가
 * 아니라 `application::service::planning::*` 계약에 의존한다.
 */

// admin은 관리 화면과 admin API가 쓰는 고수준 planning facade를 담는다. 외부 adapter가 직접 호출해야 하므로 public
// 모듈로 열어 둔다.
pub mod admin;
// authoring은 draft 작성, bootstrap, 방향 문서 생성처럼 planning 자료를 만드는 내부 application 계층이다. crate 안의
// 다른 service/test는 보되 외부 crate API로는 노출하지 않기 위해 pub(crate)다.
pub(crate) mod authoring;
// composition은 포트와 서비스를 엮어 PlanningFeature를 만드는 내부 조립 계층이다. 생성자는 feature.rs로만 열고,
// 조립 세부 타입은 이 모듈 밖으로 새지 않게 private으로 둔다.
mod composition;
// control은 planning 상태를 명령/응답 형태로 다루는 얇은 control surface다. CLI와 admin 쪽에서 직접 쓸 수 있어
// public 모듈로 유지한다.
pub mod control;
// feature는 PlanningFeature public facade의 실제 정의다. 아래에서 타입만 재수출하고 파일 경로 자체는 숨기기 위해
// private 모듈로 둔다.
mod feature;
// repair는 doctor, reset, reconciliation처럼 workspace와 authority를 정상 상태로 되돌리는 내부 서비스 묶음이다.
pub(crate) mod repair;
// runtime은 턴 실행 중 필요한 snapshot, prompt, intake, policy를 담당한다. adapter는 아래 재수출 타입을 통해 접근한다.
pub(crate) mod runtime;
// shared는 planning 전반에서 공유하는 계약, copy, authority seed 같은 공용 부품이다.
pub(crate) mod shared;
// task_mutation은 자연어/도구 입력을 task 생성과 수정 명령으로 바꾸는 application service다.
pub(crate) mod task_mutation;
// task_tool은 app-server tool contract와 task authority 조작을 연결하는 좁은 서비스 영역이다.
pub(crate) mod task_tool;
// use_cases는 PlanningFeature가 외부에 내보내는 workspace/runtime/worker/task_tool 묶음 타입을 정의한다.
mod use_cases;
// worker는 planning worker 실행, 큐 refresh, official completion 복구처럼 외부 worker boundary를 타는 orchestration이다.
pub(crate) mod worker;

// admin 재수출은 inbound admin UI/API가 `application::service::planning::*` 한 경로에서 관리 요청/응답 타입을 쓰게 하는
// 계약이다. CRUD, draft, reset, overview 타입이 여기로 모인다.
pub use self::admin::{
    PlanningAdminCrudOutcome, PlanningAdminDirectionDeleteRequest,
    PlanningAdminDirectionMutationRequest, PlanningAdminDraftFileUpdate, PlanningAdminDraftKind,
    PlanningAdminDraftLoadRequest, PlanningAdminDraftMutationRequest, PlanningAdminFacadeService,
    PlanningAdminFileKey, PlanningAdminFileSyncOutcome, PlanningAdminManagementView,
    PlanningAdminOverview, PlanningAdminResetOutcome, PlanningAdminSessionView,
    PlanningAdminTaskDeleteRequest, PlanningAdminTaskMutationRequest,
};
// bootstrap 타입은 새 planning workspace를 세팅할 때 필요한 산출물과 실행 모드를 외부 초기화 흐름에 제공한다.
pub use self::authoring::bootstrap::{
    PlanningBootstrapArtifacts, PlanningBootstrapMode, PlanningBootstrapService,
};
// directions 타입은 queue idle 검토와 supporting file 상태처럼 planning 방향 문서 작성/검토 화면이 쓰는 projection 계약이다.
pub use self::authoring::directions::{
    DirectionsMaintenanceDirectionSummary, DirectionsMaintenanceSummary,
    DirectionsSupportingFileStatus, QueueIdleReviewContext,
};
// init 타입은 draft editor와 workspace 초기화 흐름이 주고받는 파일 목록, 저장 결과, promote 결과를 공개한다.
pub use self::authoring::init::{
    PlanningDraftEditorFile, PlanningDraftEditorSession, PlanningDraftPromoteResult,
    PlanningDraftSaveResult, PlanningInitStageResult, PlanningWorkspaceInitResult,
};
// proposal promotion 타입은 작성된 제안 파일을 active planning 문서로 승격할 때 adapter와 service가 공유하는 계약이다.
pub use self::authoring::proposal_promotion::{
    PlanningProposalPromotionOutcome, PlanningProposalPromotionRequest,
};
// control 타입은 planning subsystem을 명령형으로 조작하는 경로를 제공한다. CLI/admin이 service 내부 구조를 몰라도
// command/reply 단위로 호출할 수 있다.
pub use self::control::{PlanningControlCommand, PlanningControlReply, PlanningControlService};
// PlanningFeature는 adapter가 가장 많이 받는 planning facade다. 내부 feature 모듈은 숨기고 타입 이름만 공개한다.
pub use self::feature::PlanningFeature;
// PlanningServices 별칭은 기존 호출자 호환용 이름이다. 새 구조는 PlanningFeature지만, TUI와 테스트의 점진적 이전을
// 위해 같은 타입을 옛 이름으로도 재수출한다.
pub use self::feature::PlanningFeature as PlanningServices;
// doctor 타입은 workspace 상태 진단 결과를 admin/runtime 화면으로 전달하는 공개 projection이다.
pub use self::repair::doctor::{PlanningDoctorReport, PlanningDoctorState};
// reconciliation 재수출은 턴 실행 후 planning 파일과 task authority를 맞추는 과정의 snapshot, repair request, queue action
// 계약을 한곳으로 올린다.
pub use self::repair::reconciliation::{
    PlanningExecutionSnapshot, PlanningProtectedFileRestoration, PlanningQueueProjectionAction,
    PlanningReconciliationResult, PlanningRepairRequest, PlanningRepairRetryReason,
};
// reset 타입은 admin reset 요청이 어떤 범위를 되돌렸고 어떤 결과를 냈는지 표현한다.
pub use self::repair::reset::{PlanningResetTarget, PlanningWorkspaceResetResult};
// runtime facade 타입은 TUI 턴 제출 경로가 task handoff, auto-follow 판단, preview, status projection을 다룰 때 쓰는
// 중심 계약이다.
pub use self::runtime::facade::{
    PlanningMainSessionHandoff, PlanningRuntimeAutoFollowDecision,
    PlanningRuntimeAutoFollowRequest, PlanningRuntimePreviewRequest,
    PlanningRuntimeQueuedAutoFollowPrompt, PlanningRuntimeRenderedPreview,
    PlanningRuntimeRepairAttempt, PlanningRuntimeStatusProjection,
    PlanningRuntimeStatusProjectionRequest, PlanningRuntimeSummaryLineRequest,
    PlanningRuntimeSummaryRequest, PlanningSubSessionHandoff, PlanningTaskHandoff,
};
// intake 타입은 사용자 prompt를 task draft/proposal/commit 결과로 바꾸는 task intake 흐름의 공개 입력과 출력이다.
pub use self::runtime::intake::{
    LocalPromptTaskDraftGenerator, PlanningTaskDraftGenerator, PlanningTaskIntakeCommitResult,
    PlanningTaskIntakeDraft, PlanningTaskIntakeProposal, PlanningTaskIntakeRequest,
    PlanningTaskIntakeValidationError, PlanningTaskIntakeValidationService,
};
// auto-follow block reason은 runtime policy가 자동 후속 실행을 막은 이유를 UI copy와 테스트가 분기할 수 있게 공개한다.
pub use self::runtime::policy::PlanningAutoFollowBlockReason;
// runtime snapshot/status는 현재 planning workspace가 실행 가능한지, task가 있는지, invalid인지 판단하는 핵심 projection이다.
pub use self::runtime::prompt::{PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus};
// validation service는 여러 planning service와 테스트가 같은 검증 규칙을 직접 재사용할 수 있게 공개한다.
pub use self::runtime::validation::PlanningValidationService;
// builtin transcript copy는 auto-follow가 다음 작업 지시를 만들 때 쓰는 고정 문구다. UI/worker 경로가 같은 문구를
// 쓰도록 planning 모듈 표면에서 재수출한다.
pub use self::shared::auto_follow_copy::BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT;
// shared contract 재수출은 planning 파일 경로와 디렉터리 이름을 한 계약으로 고정한다. adapter가 문자열을 직접
// 재정의하지 않고 이 상수를 쓰면 workspace layout drift를 줄일 수 있다.
pub use self::shared::contract::{
    ACTIVE_PLANNING_FILE_PATHS, DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH,
    PLANNING_DIRECTION_DOCS_DIRECTORY, PLANNING_DRAFTS_DIRECTORY, PLANNING_PROMPTS_DIRECTORY,
    PLANNING_REJECTED_DIRECTORY, RESULT_OUTPUT_FILE_PATH, canonical_active_planning_file_path,
    default_direction_detail_doc_path,
};
// task mutation 타입은 task 생성/수정 명령 추출, preview, commit 결과를 외부 입력 처리 경로에 제공한다.
pub use self::task_mutation::{
    PlanningTaskCommandExtraction, PlanningTaskCreateInput, PlanningTaskCreatePreview,
    PlanningTaskCreatePreviewRequest, PlanningTaskMutationCommand,
    PlanningTaskMutationCommitResult, PlanningTaskMutationRequest, PlanningTaskMutationService,
    PlanningTaskMutationSource, PlanningTaskUpdateInput, extract_planning_task_commands,
};
// task tool 재수출은 app-server tool schema와 요청/응답 타입을 함께 노출해 adapter가 같은 contract로 JSON tool 호출을
// 구성하게 한다.
pub use self::task_tool::{
    PlanningTaskToolRequest, PlanningTaskToolResponse, PlanningTaskToolService,
    planning_task_tool_contract_json,
};
// use case 묶음 재수출은 PlanningFeature의 필드 타입을 외부에서도 명시하거나 테스트 fixture에서 사용할 수 있게 한다.
pub use self::use_cases::{
    PlanningRuntimeUseCases, PlanningTaskToolUseCases, PlanningWorkerUseCases,
    PlanningWorkspaceUseCases,
};
// worker orchestration 타입은 queue refresh, official completion refresh, worker run outcome처럼 app-server worker 경로의
// 입력과 결과를 공개한다.
pub use self::worker::orchestration::{
    PlanningLedgerRepairRequest, PlanningOfficialCompletionRefreshRequest,
    PlanningQueueRefreshMode, PlanningQueueRefreshRequest, PlanningWorkerRunOutcome,
};
// official completion refresh contract는 domain 쪽 payload지만 planning service API 사용자에게 필요한 타입이다.
// 이곳에서 재수출해 adapter가 domain 모듈 경로까지 직접 의존하지 않아도 된다.
pub use crate::domain::planning::{
    PlanningOfficialCompletionRefreshContract, PlanningOfficialCompletionRefreshPayload,
};
