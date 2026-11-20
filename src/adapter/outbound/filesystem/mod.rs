// filesystem outbound adapter 영역은 planning workspace를 로컬 파일 시스템에 매핑하는 구현을 담는다.
// 이 선언이 `planning_workspace.rs`를 outward-facing adapter module로 연결한다.
pub mod parallel_agent_profile;
pub mod planning_workspace;
pub mod pr_validation_rollout_evidence;
pub(crate) mod secure_fs;
// kernel_lock 은 저장소당 커널 하나를 지키는 OS 파일 잠금이다 (plan 18 K3-4).
pub(crate) mod kernel_lock;
// git_operation_journal 은 저장소마다 하나인 git 작업 일지 파일이다 (plan 27 G3).
pub(crate) mod git_operation_journal;
// issue_policy 는 사용자와 프로젝트의 이슈 정책 파일을 읽는다 (#2416).
pub mod issue_policy;
// evidence_vault_store 는 가린 구간을 암호화한 레코드 파일로 저장한다 (#2421).
pub mod evidence_vault_store;
// evidence_vault_backup 은 암호화한 레코드와 복구 키로 감싼 장치 키를 보관 파일로 내보내고 가져온다 (#2432).
pub mod evidence_vault_backup;
// issue_publish_intent_journal 은 이슈 게시 의도 일지 파일이다 (#2424).
pub mod issue_publish_intent_journal;
// issue_template 은 작업 폴더의 이슈 템플릿 파일을 읽는다 (#2417).
pub mod issue_template;

// re-export를 두면 composition/wiring code가 파일명 세부 구조를 몰라도 adapter를 직접 주입할 수 있다.
pub(crate) use self::kernel_lock::{
    RepositoryKernelLock, RepositoryKernelLockAdapter, RepositoryKernelLockHeld,
    RepositoryKernelOwner,
};
pub use self::parallel_agent_profile::FilesystemParallelAgentProfileRepositoryAdapter;
pub use self::planning_workspace::FilesystemPlanningWorkspaceAdapter;
pub use self::pr_validation_rollout_evidence::FilesystemPrValidationRolloutEvidenceAdapter;

pub mod issue_creation_journal;
pub use self::issue_creation_journal::IssueCreationJournalAdapter;

pub(crate) mod private_runtime_lease;
