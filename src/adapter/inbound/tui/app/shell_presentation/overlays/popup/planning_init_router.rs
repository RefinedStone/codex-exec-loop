// 학습 주석: router는 app 전체 state와 planning init step enum을 읽어 현재 단계에 맞는 overlay builder를 고릅니다.
use super::super::super::super::{NativeTuiApp, PlanningInitOverlayStep};
// 학습 주석: 모든 init step은 renderer-facing `PlanningInitOverlayView`로 수렴합니다. variant마다 builder가
// 달라도 최종 DTO shape를 통일해 popup renderer를 단순하게 유지합니다.
use super::super::PlanningInitOverlayView;
// 학습 주석: existing workspace builder는 app state에서 발견된 기존 planning workspace copy를 읽습니다.
use super::existing_workspace::build_existing_workspace_overlay_view_for_app;
// 학습 주석: init_copy builders는 selection/review/manual editor처럼 copy DTO만 있거나 고정 문구로 만들 수
// 있는 planning init screens를 담당합니다.
use super::init_copy::{
    build_detail_selection_overlay_view, build_manual_editor_overlay_view,
    build_mode_selection_overlay_view, build_simple_review_overlay_view,
};
// 학습 주석: simple review는 app state에서 staged draft name/file count 같은 copy를 먼저 뽑은 뒤 view로 만듭니다.
use super::simple_review_inputs::build_simple_review_copy;

// 학습 주석: 이 함수는 planning init overlay의 presentation router입니다. shell frontend는 "planning init
// overlay를 만들어 달라"고만 요청하고, 이 router가 step별 builder를 선택해 공통 view DTO로 돌려줍니다.
pub(super) fn build_planning_init_overlay_view_for_app(
    // 학습 주석: app은 current step, selected mode/detail, staged draft copy source 등 모든 routing input을 갖고 있습니다.
    app: &NativeTuiApp,
) -> PlanningInitOverlayView {
    // 학습 주석: planning_init_overlay_ui_state는 modal wizard의 UI-local state입니다. application planning
    // service state가 아니라, 사용자가 어느 init step을 보고 무엇을 선택했는지 담습니다.
    let state = &app.planning_init_overlay_ui_state;

    // 학습 주석: step enum을 exhaustively match하면 새 planning init step이 추가될 때 이 router가 컴파일 단계에서
    // 업데이트를 요구합니다. 따라서 renderer까지 잘못된 fallback view가 흘러가지 않습니다.
    match state.step() {
        // 학습 주석: ExistingWorkspace는 이미 `.codex-exec-loop/planning` artifact가 있는 경우의 안전 확인 화면입니다.
        // app state에서 기존 workspace detail을 읽어야 하므로 app 전체를 builder에 넘깁니다.
        PlanningInitOverlayStep::ExistingWorkspace => {
            build_existing_workspace_overlay_view_for_app(app)
        }
        // 학습 주석: ModeSelection은 simple/detail 같은 init mode를 고르는 단계입니다. selected_mode만 있으면
        // 고정 option lines를 만들 수 있어 state field만 넘깁니다.
        PlanningInitOverlayStep::ModeSelection => {
            build_mode_selection_overlay_view(state.selected_mode())
        }
        // 학습 주석: DetailSelection은 detail-mode authoring 경로의 세부 선택 단계입니다. selected_detail이
        // 현재 highlight/description을 결정합니다.
        PlanningInitOverlayStep::DetailSelection => {
            build_detail_selection_overlay_view(state.selected_detail())
        }
        // 학습 주석: SimpleReview는 staged simple scaffold를 promote하기 전 검토 화면입니다. app에서 copy를
        // 추출한 뒤 review builder에 넘겨 app 의존성을 아래 layer로 퍼뜨리지 않습니다.
        PlanningInitOverlayStep::SimpleReview => {
            build_simple_review_overlay_view(build_simple_review_copy(app))
        }
        // 학습 주석: ManualEditor는 dedicated draft editor surface와 함께 쓰는 안내 overlay입니다. 별도 app data가
        // 필요 없는 고정 state explanation이라 바로 builder를 호출합니다.
        PlanningInitOverlayStep::ManualEditor => build_manual_editor_overlay_view(),
    }
}
