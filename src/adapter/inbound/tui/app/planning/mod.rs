// controller는 planning overlay 입력을 application use case 호출로 연결하는 TUI orchestration 계층이다.
// 상위 app module이 planning command/action을 위임해야 하므로 pub(super) 표면으로 연다.
pub(super) mod controller;
// debug_panel_state는 planning worker debug detail의 TUI-local visibility만 보관한다.
mod debug_panel_state;
// presentation은 planning worker panel state를 실제 TUI line 목록으로 바꾸는 표시 계층이다.
// controller의 mutation logic과 분리해 rendering copy 변경이 use-case 호출 흐름에 번지지 않게 한다.
mod presentation;
// status_projection은 planning runtime projection을 TUI 상태 표시 model로 축약한다. shell presentation이 planning
// domain 구조를 직접 알지 않게 하는 adapter 경계다.
pub(crate) mod status_projection;

pub(super) use debug_panel_state::PlanningWorkerVisibility;
// planning worker panel line builder는 shell rendering 쪽에서 필요하므로 상위 planning module 표면으로 올린다.
pub(super) use presentation::{build_planning_worker_panel_lines, planning_worker_status_label};
