pub(crate) mod issue_assessment_evidence;
/*
 * git outbound adapter는 parallel mode가 필요로 하는 로컬 Git 런타임 조작을 구현한다.
 * public 표면은 worktree/branch 준비와 정리를 담당하는 parallel_mode_runtime에 집중되어 있다.
 */
pub(crate) mod history;
pub mod issue_form_repository;
pub(crate) mod issue_origin;
mod parallel_commit_diff;
pub mod parallel_mode_runtime;
mod parallel_normalization_fs;
mod parallel_pool_lock;
mod parallel_worker_commit;
mod progress_markers;
pub(crate) mod repository_state;
pub mod task_verification;
pub(crate) mod workspace;
