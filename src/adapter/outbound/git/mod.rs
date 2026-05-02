/*
 * git outbound adapter는 parallel mode가 필요로 하는 로컬 Git 런타임 조작을 구현한다.
 * public 표면은 worktree/branch 준비와 정리를 담당하는 parallel_mode_runtime에 집중되어 있다.
 */
pub mod parallel_mode_runtime;
