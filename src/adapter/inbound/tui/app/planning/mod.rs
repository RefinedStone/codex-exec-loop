// 학습 주석: controller는 planning overlay 입력을 application use case 호출로 연결하는 TUI orchestration 계층입니다. 상위 app 모듈이
// 직접 호출해야 하므로 pub(super)로 엽니다.
pub(super) mod controller;
// 학습 주석: debug_panel_state는 planner worker panel의 접힘/상태/가시성 정보를 보관하는 작은 UI state 모델입니다.
mod debug_panel_state;
// 학습 주석: presentation은 planner panel state를 실제 TUI line 목록으로 바꾸는 표시 계층입니다. controller의 mutation 로직과 분리됩니다.
mod presentation;
// 학습 주석: status_projection은 planning runtime snapshot을 TUI 상태 표시 모델로 축약합니다. shell presentation이 planning domain 구조를
// 직접 알지 않게 하는 adapter 경계입니다.
pub(crate) mod status_projection;

// 학습 주석: app planning 하위 모듈은 panel state 타입만 상위 app에 재수출합니다. 실제 상태 정의 파일은 private로 숨겨 UI state 표면을 작게 유지합니다.
pub(super) use debug_panel_state::{
    PlannerVisibility, PlannerWorkerPanelState, PlannerWorkerStatus,
};
// 학습 주석: planner panel line builder는 shell rendering 쪽에서 필요하므로 상위 planning module 표면으로 올립니다.
pub(super) use presentation::build_planner_panel_lines;
