#[path = "directions_copy.rs"]
// 학습 주석: copy 모듈은 이미 선택/요약이 끝난 presentation 값을 실제 Line 묶음으로 바꿉니다.
// 이 파일은 copy 문구를 직접 만들지 않고 overlay step별 view builder에 위임합니다.
mod copy;
#[path = "directions_projection.rs"]
// 학습 주석: projection 모듈은 detail-doc 대상 direction 목록을 선택 상태와 함께 표시용 row로 낮춥니다.
// overview/confirm은 단순 값만 넘기지만, selection step은 list projection이 필요해 별도 모듈로 분리합니다.
mod projection;

// 학습 주석: 이 overlay builder는 NativeTuiApp의 directions maintenance UI state만 읽습니다. Line과
// compact helper는 최종 renderer 계약과 status 문구 폭 제한을 맞추기 위해 가져옵니다.
use super::super::{
    DirectionsMaintenanceOverlayStep, Line, NativeTuiApp, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT,
    compact_whitespace_detail,
};
// 학습 주석: copy builders는 step별 최종 view shape를 만듭니다. 이 파일의 역할은 app state에서 필요한
// 입력만 골라 이 함수들에 넘기는 overlay router입니다.
use copy::{
    build_detail_doc_confirm_overlay_view, build_detail_doc_selection_overlay_view,
    build_manual_editor_overlay_view, build_overview_overlay_view,
};
// 학습 주석: selection step에서는 actionable direction 목록과 현재 선택 항목을 projection으로 묶은 뒤
// copy builder에 넘깁니다. 목록 표시 정책을 라우터와 분리해 테스트/확장이 쉬워집니다.
use projection::build_detail_doc_selection_projection;

// 학습 주석: directions maintenance overlay renderer가 소비하는 최종 DTO입니다. header/summary/options/status/keys를
// 분리해 shell_rendering이 모든 directions step을 같은 panel layout에 배치할 수 있게 합니다.
pub(crate) struct DirectionsMaintenanceOverlayView {
    // 학습 주석: header는 overlay 목적과 현재 maintenance context를 설명하는 상단 copy입니다.
    pub(crate) header_lines: Vec<Line<'static>>,
    // 학습 주석: summary는 overview counts나 선택된 direction 설명처럼 현재 step의 주요 상태입니다.
    pub(crate) summary_lines: Vec<Line<'static>>,
    // 학습 주석: option_lines는 사용자가 고르거나 검토할 선택지 rows입니다. overview action list와
    // detail-doc selection list가 같은 field를 공유합니다.
    pub(crate) option_lines: Vec<Line<'static>>,
    // 학습 주석: status_lines는 parse error, staged file 안내, confirmation 상태처럼 action 전후의 보조 진단입니다.
    pub(crate) status_lines: Vec<Line<'static>>,
    // 학습 주석: key_lines는 step별 허용 키를 보여 줍니다. 입력 처리 상태와 copy가 어긋나지 않도록 step router에서 같이 고릅니다.
    pub(crate) key_lines: Vec<Line<'static>>,
}

// 학습 주석: `build_directions_maintenance_overlay_view`는 directions maintenance UI state를 renderer-ready
// view로 낮추는 최상위 presentation boundary입니다. state mutation은 input/runtime 쪽 책임이고,
// 여기서는 현재 step을 읽어 필요한 값만 copy/projection builder에 전달합니다.
pub(crate) fn build_directions_maintenance_overlay_view(
    // 학습 주석: app은 shell 전체 state container지만, 이 함수는 directions_maintenance_overlay_ui_state만
    // 읽어 directions overlay가 다른 TUI 상태와 결합하지 않게 합니다.
    app: &NativeTuiApp,
) -> DirectionsMaintenanceOverlayView {
    // 학습 주석: step enum은 overlay의 작은 flow router입니다. overview, detail-doc selection,
    // confirmation, manual editor가 서로 다른 입력을 필요로 하므로 여기에서 분기해 view DTO를 만듭니다.
    match app.directions_maintenance_overlay_ui_state.step() {
        // 학습 주석: Overview는 accepted directions summary와 queue-idle prompt health를 압축해 보여 줍니다.
        // summary가 아직 없으면 loading/error 초기 상태로 보고 unknown/0 fallback을 사용합니다.
        DirectionsMaintenanceOverlayStep::Overview => {
            // 학습 주석: summary는 optional snapshot입니다. 같은 Option에서 여러 표시값을 읽기 때문에
            // 각 field별 fallback을 명확히 두어 copy builder가 None을 직접 알 필요가 없게 합니다.
            let summary = app.directions_maintenance_overlay_ui_state.summary();
            // 학습 주석: missing/broken detail doc count는 operator가 먼저 어떤 maintenance action을
            // 해야 하는지 판단하는 핵심 수치입니다.
            let missing_doc_count = summary
                .map(|summary| summary.missing_detail_doc_count)
                .unwrap_or_default();
            let broken_doc_count = summary
                .map(|summary| summary.broken_detail_doc_count)
                .unwrap_or_default();
            // 학습 주석: total_direction_count는 overview가 "문제가 몇 개"뿐 아니라 전체 규모를 같이 보여 주게 합니다.
            let total_direction_count =
                summary.map(|summary| summary.directions.len()).unwrap_or(0);
            // 학습 주석: queue idle policy는 directions maintenance와 직접 연결됩니다. queue가 비었을 때
            // 어떤 prompt/detail doc 흐름이 필요한지 overview에서 즉시 보이게 합니다.
            let queue_idle_policy = summary
                .map(|summary| summary.queue_idle_policy.label().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            // 학습 주석: prompt path는 길거나 공백이 섞일 수 있어 queue inspection과 같은 compact 규칙으로 줄입니다.
            // path가 없으면 copy에는 `<none>`을 넘겨 missing과 empty string을 구분합니다.
            let queue_idle_prompt = summary
                .and_then(|summary| summary.queue_idle_prompt_path.as_deref())
                .map(|path| compact_whitespace_detail(path, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT))
                .unwrap_or_else(|| "<none>".to_string());
            // 학습 주석: prompt status는 enum label만 넘깁니다. copy builder는 표시 문구만 담당하고,
            // status 판정은 summary 생성 쪽에 남습니다.
            let queue_idle_prompt_status = summary
                .map(|summary| summary.queue_idle_prompt_status.label())
                .unwrap_or("unknown");
            // 학습 주석: parse error는 overview status panel에 짧게 들어가야 하므로 path와 같은 compact
            // helper를 씁니다. None이면 copy builder가 error line을 생략합니다.
            let parse_error_summary = summary
                .and_then(|summary| summary.parse_error.as_deref())
                .map(|error| compact_whitespace_detail(error, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT));

            build_overview_overlay_view(
                missing_doc_count,
                broken_doc_count,
                total_direction_count,
                &queue_idle_policy,
                queue_idle_prompt_status,
                &queue_idle_prompt,
                parse_error_summary.as_deref(),
            )
        }
        // 학습 주석: DetailDocSelection은 missing/broken detail doc mapping이 있는 directions만 목록화합니다.
        // projection 단계가 selected row와 selected title을 계산하고 copy 단계가 panel lines를 만듭니다.
        DirectionsMaintenanceOverlayStep::DetailDocSelection => {
            let actionable_directions = app
                .directions_maintenance_overlay_ui_state
                .actionable_detail_doc_directions();
            // 학습 주석: selected_direction은 UI cursor가 가리키는 actionable item입니다. 목록이 비었거나
            // cursor가 아직 정렬되지 않았으면 None이 될 수 있어 projection이 fallback copy를 만듭니다.
            let selected_direction = app
                .directions_maintenance_overlay_ui_state
                .selected_actionable_detail_doc_direction();
            // 학습 주석: projection은 Vec을 소유하지 않고 slice로 읽습니다. app state가 제공한 목록을
            // 표시용 row와 selected title로 낮추는 순수 변환 경계입니다.
            let projection = build_detail_doc_selection_projection(
                actionable_directions.as_slice(),
                selected_direction,
            );

            build_detail_doc_selection_overlay_view(
                projection.option_lines,
                projection.selected_direction_title.as_deref(),
            )
        }
        // 학습 주석: DetailDocConfirm은 선택한 direction에 대해 detail doc staging을 실행할지 확인합니다.
        // pending이 없으면 unknown fallback으로 렌더링해 잘못된 state도 깨진 UI 대신 진단 가능한 copy를 보여 줍니다.
        DirectionsMaintenanceOverlayStep::DetailDocConfirm => {
            let pending = app
                .directions_maintenance_overlay_ui_state
                .pending_detail_doc_creation();
            // 학습 주석: confirm copy에는 사람이 읽는 title과 안정적인 id가 모두 필요합니다. title은
            // 사용자가 확인할 대상이고, id는 생성될 mapping/doc path의 기준입니다.
            let direction_id = pending
                .map(|pending| pending.direction_id())
                .unwrap_or("unknown");
            let direction_title = pending
                .map(|pending| pending.direction_title())
                .unwrap_or("unknown");

            build_detail_doc_confirm_overlay_view(
                direction_title,
                direction_id,
                app.directions_maintenance_overlay_ui_state
                    // 학습 주석: confirm choice는 yes/no cursor 상태입니다. copy builder가 이 값을 이용해
                    // 선택된 action row와 key guidance를 같은 기준으로 강조합니다.
                    .detail_doc_confirm_choice(),
            )
        }
        // 학습 주석: ManualEditor는 별도 planning draft editor로 넘어가기 전 안내만 보여 주는 정적 step입니다.
        // 읽어야 할 app state가 없으므로 copy builder를 바로 호출합니다.
        DirectionsMaintenanceOverlayStep::ManualEditor => build_manual_editor_overlay_view(),
    }
}
