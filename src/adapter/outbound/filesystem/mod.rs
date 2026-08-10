// filesystem outbound adapter 영역은 planning workspace를 로컬 파일 시스템에 매핑하는 구현을 담는다.
// 이 선언이 `planning_workspace.rs`를 outward-facing adapter module로 연결한다.
pub mod parallel_agent_profile;
pub mod planning_workspace;
pub mod pr_validation_rollout_evidence;
pub(crate) mod secure_fs;

// re-export를 두면 composition/wiring code가 파일명 세부 구조를 몰라도 adapter를 직접 주입할 수 있다.
pub use self::parallel_agent_profile::FilesystemParallelAgentProfileRepositoryAdapter;
pub use self::planning_workspace::FilesystemPlanningWorkspaceAdapter;
pub use self::pr_validation_rollout_evidence::FilesystemPrValidationRolloutEvidenceAdapter;
