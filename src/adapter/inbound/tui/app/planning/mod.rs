// controller는 planning overlay 입력을 application use case 호출로 연결하는 TUI orchestration 계층이다.
// 상위 app module이 planning command/action을 위임해야 하므로 pub(super) 표면으로 연다.
pub(super) mod controller;
// debug_panel_state는 planner worker panel의 접힘/상태/가시성 정보를 보관하는 작은 UI state model이다.
mod debug_panel_state;
// presentation은 planner panel state를 실제 TUI line 목록으로 바꾸는 표시 계층이다.
// controller의 mutation logic과 분리해 rendering copy 변경이 use-case 호출 흐름에 번지지 않게 한다.
mod presentation;
// status_projection은 planning runtime snapshot을 TUI 상태 표시 model로 축약한다. shell presentation이 planning
// domain 구조를 직접 알지 않게 하는 adapter 경계다.
pub(crate) mod status_projection;

// app planning 하위 module은 panel state 타입만 상위 app에 재수출한다. 실제 상태 정의 파일은 private로 숨겨
// UI state 표면을 작게 유지한다.
pub(super) use debug_panel_state::{
    PlannerVisibility, PlannerWorkerPanelState, PlannerWorkerStatus,
};
// planner panel line builder는 shell rendering 쪽에서 필요하므로 상위 planning module 표면으로 올린다.
pub(super) use presentation::build_planner_panel_lines;
