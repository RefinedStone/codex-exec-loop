// 학습 주석: filesystem outbound adapter 영역은 planning workspace를 로컬 파일 시스템에 매핑하는
// 구현을 담습니다. 이 선언이 `planning_workspace.rs`를 outward-facing adapter module로 연결합니다.
pub mod planning_workspace;

// 학습 주석: re-export를 두면 composition/wiring code가 파일명 세부 구조를 몰라도
// `adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter`를 직접 사용할 수 있습니다.
pub use self::planning_workspace::FilesystemPlanningWorkspaceAdapter;
