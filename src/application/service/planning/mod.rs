// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
pub mod admin;
// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
pub(crate) mod authoring;
// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
mod composition;
// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
pub mod control;
// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
mod feature;
// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
pub(crate) mod repair;
// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
pub(crate) mod runtime;
// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
pub(crate) mod shared;
// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
pub(crate) mod task_mutation;
// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
pub(crate) mod task_tool;
// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
mod use_cases;
// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
pub(crate) mod worker;

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::admin::{
    PlanningAdminCrudOutcome, PlanningAdminDirectionDeleteRequest,
    PlanningAdminDirectionMutationRequest, PlanningAdminDraftFileUpdate, PlanningAdminDraftKind,
    PlanningAdminDraftLoadRequest, PlanningAdminDraftMutationRequest, PlanningAdminFacadeService,
    PlanningAdminFileKey, PlanningAdminFileSyncOutcome, PlanningAdminManagementView,
    PlanningAdminOverview, PlanningAdminResetOutcome, PlanningAdminSessionView,
    PlanningAdminTaskDeleteRequest, PlanningAdminTaskMutationRequest,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::authoring::bootstrap::{
    PlanningBootstrapArtifacts, PlanningBootstrapMode, PlanningBootstrapService,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::authoring::directions::{
    DirectionsMaintenanceDirectionSummary, DirectionsMaintenanceSummary,
    DirectionsSupportingFileStatus, PlanningDoctorOutcome, QueueIdleReviewContext,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::authoring::init::{
    PlanningDraftEditorFile, PlanningDraftEditorSession, PlanningDraftPromoteResult,
    PlanningDraftSaveResult, PlanningInitStageResult, PlanningWorkspaceInitResult,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::authoring::proposal_promotion::{
    PlanningProposalPromotionOutcome, PlanningProposalPromotionRequest,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::control::{PlanningControlCommand, PlanningControlReply, PlanningControlService};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::feature::PlanningFeature;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::feature::PlanningFeature as PlanningServices;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::repair::doctor::{PlanningDoctorReport, PlanningDoctorState};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::repair::reconciliation::{
    PlanningExecutionSnapshot, PlanningProtectedFileRestoration, PlanningQueueProjectionAction,
    PlanningReconciliationResult, PlanningRepairRequest, PlanningRepairRetryReason,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::repair::reset::{PlanningResetTarget, PlanningWorkspaceResetResult};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::runtime::facade::{
    PlanningMainSessionHandoff, PlanningRuntimeAutoFollowDecision,
    PlanningRuntimeAutoFollowRequest, PlanningRuntimePreviewRequest,
    PlanningRuntimeQueuedAutoFollowPrompt, PlanningRuntimeRenderedPreview,
    PlanningRuntimeRepairAttempt, PlanningRuntimeStatusProjection,
    PlanningRuntimeStatusProjectionRequest, PlanningRuntimeSummaryLineRequest,
    PlanningRuntimeSummaryRequest, PlanningSubSessionHandoff, PlanningTaskHandoff,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::runtime::intake::{
    LocalPromptTaskDraftGenerator, PlanningTaskDraftGenerator, PlanningTaskIntakeCommitResult,
    PlanningTaskIntakeDraft, PlanningTaskIntakeProposal, PlanningTaskIntakeRequest,
    PlanningTaskIntakeValidationError, PlanningTaskIntakeValidationService,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::runtime::policy::PlanningAutoFollowBlockReason;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::runtime::prompt::{PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::runtime::validation::PlanningValidationService;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::shared::auto_follow_copy::BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::shared::contract::{
    ACTIVE_PLANNING_FILE_PATHS, DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH,
    PLANNING_DIRECTION_DOCS_DIRECTORY, PLANNING_DRAFTS_DIRECTORY, PLANNING_PROMPTS_DIRECTORY,
    PLANNING_REJECTED_DIRECTORY, RESULT_OUTPUT_FILE_PATH, canonical_active_planning_file_path,
    default_direction_detail_doc_path,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::task_mutation::{
    PlanningTaskCommandExtraction, PlanningTaskCreateInput, PlanningTaskCreatePreview,
    PlanningTaskCreatePreviewRequest, PlanningTaskMutationCommand,
    PlanningTaskMutationCommitResult, PlanningTaskMutationRequest, PlanningTaskMutationService,
    PlanningTaskMutationSource, PlanningTaskUpdateInput, extract_planning_task_commands,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::task_tool::{
    PlanningTaskToolRequest, PlanningTaskToolResponse, PlanningTaskToolService,
    planning_task_tool_contract_json,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::use_cases::{
    PlanningRuntimeUseCases, PlanningTaskToolUseCases, PlanningWorkerUseCases,
    PlanningWorkspaceUseCases,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use self::worker::orchestration::{
    PlanningLedgerRepairRequest, PlanningOfficialCompletionRefreshRequest,
    PlanningQueueRefreshMode, PlanningQueueRefreshRequest, PlanningWorkerRunOutcome,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
pub use crate::domain::planning::{
    PlanningOfficialCompletionRefreshContract, PlanningOfficialCompletionRefreshPayload,
};
