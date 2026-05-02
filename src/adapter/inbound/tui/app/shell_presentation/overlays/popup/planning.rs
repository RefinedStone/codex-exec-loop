// 학습 주석: planning_copy module은 app state에서 planning init overlay가 필요한 presentation copy를 뽑습니다.
#[path = "planning_copy.rs"]
mod copy;
// 학습 주석: planning_editor_copy module은 draft editor surface에 필요한 file/body/cursor 관련 copy를 만듭니다.
#[path = "planning_editor_copy.rs"]
mod editor_copy;
// 학습 주석: planning_editor_inputs module은 draft editor rendering에 넘길 input DTO를 정리합니다.
#[path = "planning_editor_inputs.rs"]
mod editor_inputs;
// 학습 주석: planning_editor_surface module은 draft editor copy/input을 최종 editor overlay view로 조립합니다.
#[path = "planning_editor_surface.rs"]
mod editor_surface;
// 학습 주석: planning_existing_workspace module은 기존 planning workspace를 감지했을 때 보여 줄 overlay variant입니다.
#[path = "planning_existing_workspace.rs"]
mod existing_workspace;
// 학습 주석: planning_existing_workspace_inputs module은 existing workspace 화면에 필요한 copy/input을 분리합니다.
#[path = "planning_existing_workspace_inputs.rs"]
mod existing_workspace_inputs;
// 학습 주석: planning_init_copy module은 simple/detail/manual init flow에서 공유하는 copy DTO와 title helper를 담습니다.
#[path = "planning_init_copy.rs"]
mod init_copy;
// 학습 주석: planning_init_router module은 app state를 보고 simple review, detail authoring, existing workspace
// 같은 init overlay variant 중 무엇을 만들지 결정합니다.
#[path = "planning_init_router.rs"]
mod init_router;
// 학습 주석: planning_projection module은 raw planning state를 renderer가 쓰기 쉬운 projection으로 바꿉니다.
#[path = "planning_projection.rs"]
mod projection;
// 학습 주석: planning_projection_lines module은 projection DTO를 실제 styled line들로 변환하는 text layer입니다.
#[path = "planning_projection_lines.rs"]
mod projection_lines;
// 학습 주석: planning_runtime module은 runtime status/progress를 planning overlay copy에 반영하는 path입니다.
#[path = "planning_runtime.rs"]
mod runtime;
// 학습 주석: planning_session module은 planning 관련 session context를 overlay projection으로 연결합니다.
#[path = "planning_session.rs"]
mod session;
// 학습 주석: planning_simple_review_inputs module은 simple review 화면에 필요한 input/copy를 init copy에서 뽑습니다.
#[path = "planning_simple_review_inputs.rs"]
mod simple_review_inputs;

// 학습 주석: planning overlay facade는 app 전체 state를 읽어 view DTO를 만들기 때문에 `NativeTuiApp`만
// public entry의 입력으로 받습니다.
use super::super::super::NativeTuiApp;
// 학습 주석: planning init overlay와 draft editor overlay는 popup renderer가 소비하는 서로 다른 DTO입니다.
use super::{PlanningDraftEditorOverlayView, PlanningInitOverlayView};
// 학습 주석: editor_surface는 editor-specific copy/input 조립을 숨기고 최종 editor view builder만 공개합니다.
use editor_surface::build_planning_draft_editor_overlay_view_for_app;
// 학습 주석: init_router는 planning init variant 선택을 캡슐화합니다. 이 facade는 router를 호출하는 얇은 entry입니다.
use init_router::build_planning_init_overlay_view_for_app;

// 학습 주석: planning init overlay의 top-level builder입니다. shell frontend는 하위 copy/router/projection
// module을 알 필요 없이 이 함수 하나로 현재 app state에 맞는 planning init popup을 얻습니다.
pub(crate) fn build_planning_init_overlay_view(app: &NativeTuiApp) -> PlanningInitOverlayView {
    // 학습 주석: variant selection과 line assembly는 init_router로 위임해 이 파일은 planning popup surface만 담당합니다.
    build_planning_init_overlay_view_for_app(app)
}

// 학습 주석: draft editor overlay의 top-level builder입니다. 반환값이 `Option`인 이유는 app state가 현재
// editor surface를 만들 수 없을 때 caller가 overlay를 그리지 않아야 하기 때문입니다.
pub(crate) fn build_planning_draft_editor_overlay_view(
    // 학습 주석: app은 staged draft content, selected file, editor mode 같은 presentation source를 제공합니다.
    app: &NativeTuiApp,
    // 학습 주석: editor_height는 visible editor body를 계산하는 layout input입니다. builder가 content slice와
    // cursor context를 이 높이에 맞춰 projection합니다.
    editor_height: u16,
) -> Option<PlanningDraftEditorOverlayView> {
    // 학습 주석: editor-specific copy extraction과 renderer-facing DTO assembly는 editor_surface에 위임합니다.
    build_planning_draft_editor_overlay_view_for_app(app, editor_height)
}
