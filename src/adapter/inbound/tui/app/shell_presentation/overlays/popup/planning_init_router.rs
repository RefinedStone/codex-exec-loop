use super::super::super::super::{NativeTuiApp, PlanningInitOverlayStep};
use super::super::PlanningInitOverlayView;
use super::existing_workspace::build_existing_workspace_overlay_view_for_app;
use super::init_copy::{
    build_detail_selection_overlay_view, build_manual_editor_overlay_view,
    build_mode_selection_overlay_view, build_simple_review_overlay_view,
};
use super::simple_review_inputs::build_simple_review_copy;

// planning init overlay는 여러 wizard step을 갖지만 popup renderer는 단일
// `PlanningInitOverlayView`만 소비한다. 이 router는 shell frontend와 step-specific
// builder 사이의 adapter로, app/UI state 중 각 step에 필요한 입력만 아래로 넘긴다.
pub(super) fn build_planning_init_overlay_view_for_app(
    app: &NativeTuiApp,
) -> PlanningInitOverlayView {
    // 이 state는 planning service domain state가 아니라 modal-local cursor와 선택값이다.
    // 따라서 mode/detail selection builder에는 app 전체 대신 이 projection만 전달한다.
    let state = &app.planning_init_overlay_ui_state;

    // step enum을 exhaustive match로 둬 새 init step이 생길 때 compile 단계에서
    // presentation routing을 갱신하게 한다. fallback view를 만들면 wizard state와
    // 화면 copy가 어긋나는 오류가 늦게 드러난다.
    match state.step() {
        // 기존 planning artifact 감지는 runtime projection과 workspace path를 함께 읽는다.
        // 이 단계만 app-level copy builder를 거쳐야 guard 화면의 상태 문구가 최신이다.
        PlanningInitOverlayStep::ExistingWorkspace => {
            build_existing_workspace_overlay_view_for_app(app)
        }
        // mode selection은 고정 선택지와 현재 highlight만 필요하다. app을 넘기지 않아
        // 순수 copy builder가 planning runtime state에 새 의존성을 만들지 못하게 한다.
        PlanningInitOverlayStep::ModeSelection => {
            build_mode_selection_overlay_view(state.selected_mode())
        }
        // detail selection도 선택 cursor가 유일한 동적 입력이다. detail-mode authoring의
        // 나머지 상태는 다음 editor/review 단계에서 별도 copy로 들어간다.
        PlanningInitOverlayStep::DetailSelection => {
            build_detail_selection_overlay_view(state.selected_detail())
        }
        // simple review는 staged scaffold 이름과 file count 같은 app-derived copy가 필요하다.
        // copy 추출을 여기서 끝내 review layout module은 renderer DTO 조립만 맡는다.
        PlanningInitOverlayStep::SimpleReview => {
            build_simple_review_overlay_view(build_simple_review_copy(app))
        }
        // manual editor step은 별도 draft editor surface를 전제로 한 고정 안내다.
        // app-derived 값이 없으므로 builder를 바로 호출해 데이터 의존성을 명시적으로 비운다.
        PlanningInitOverlayStep::ManualEditor => build_manual_editor_overlay_view(),
    }
}
