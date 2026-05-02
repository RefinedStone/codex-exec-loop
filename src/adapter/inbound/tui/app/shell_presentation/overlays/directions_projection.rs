// 학습 주석: DirectionsMaintenanceDirectionSummary는 application service가 계산한 direction별 detail-doc 상태 요약입니다.
// presentation layer는 이 summary를 읽기만 하고, 누락/깨짐 판단 자체는 service 결과를 신뢰합니다.
use crate::application::service::planning::DirectionsMaintenanceDirectionSummary;

// 학습 주석: selection list row는 marker, bold title, status/path detail을 Span으로 나눕니다.
// theme/style type을 가져와 directions overlay의 선택 표시를 다른 overlay list와 맞춥니다.
use super::super::super::{AkraTheme, Line, Modifier, Span, Style};

// 학습 주석: DetailDocSelectionProjection은 detail-doc 생성 대상 목록을 renderer-friendly 형태로 낮춘 DTO입니다.
// directions.rs는 이 projection을 받아 copy builder에 넘기고, popup renderer는 option_lines/status_lines만 그립니다.
pub(super) struct DetailDocSelectionProjection {
    // 학습 주석: option_lines는 사용자가 위/아래로 이동하며 선택할 direction rows입니다.
    pub(super) option_lines: Vec<Line<'static>>,
    // 학습 주석: selected_direction_title은 status 영역에서 "현재 선택된 대상"을 짧게 말하기 위한 값입니다.
    pub(super) selected_direction_title: Option<String>,
}

// 학습 주석: build_detail_doc_selection_projection은 actionable direction summaries를 selection list로 변환합니다.
// controller가 선택 index를 관리하고, service가 direction status를 계산하며, 이 함수는 그 둘을 화면 row로 연결합니다.
pub(super) fn build_detail_doc_selection_projection(
    // 학습 주석: actionable_directions는 detail-doc이 없거나 깨져서 사용자가 생성/복구 action을 할 수 있는 subset입니다.
    actionable_directions: &[&DirectionsMaintenanceDirectionSummary],
    // 학습 주석: selected_direction은 keyboard focus가 가리키는 direction입니다. 목록이 비었으면 None일 수 있습니다.
    selected_direction: Option<&DirectionsMaintenanceDirectionSummary>,
) -> DetailDocSelectionProjection {
    // 학습 주석: option_lines는 빈 목록과 일반 목록을 모두 표현합니다. 빈 경우도 Vec<Line>으로 반환해
    // downstream view builder가 별도 empty-state branch 없이 렌더링할 수 있습니다.
    let option_lines = if actionable_directions.is_empty() {
        // 학습 주석: actionable 대상이 없다는 것은 모든 direction의 detail-doc mapping이 healthy라는 뜻입니다.
        vec![Line::from(
            "Every direction already has a healthy detail-doc mapping.",
        )]
    } else {
        actionable_directions
            // 학습 주석: service/controller가 제공한 actionable ordering을 그대로 사용합니다.
            .iter()
            // 학습 주석: 각 direction summary를 title + diagnostic detail row로 바꿉니다.
            .map(|direction| {
                // 학습 주석: id 기준으로 선택 여부를 비교합니다. borrowed summary pointer가 달라도 같은 id면 같은 direction입니다.
                let selected =
                    selected_direction.is_some_and(|candidate| candidate.id == direction.id);
                // 학습 주석: selected row는 keyboard focus를 보여 주고, 나머지는 기본 text style로 둡니다.
                let style = if selected {
                    // 학습 주석: selected style은 사용자가 Enter로 confirm할 direction을 강조합니다.
                    AkraTheme::selected()
                } else {
                    // 학습 주석: 기본 style은 actionable이지만 현재 focus가 아닌 row입니다.
                    Style::default()
                };
                // 학습 주석: marker는 색상에 의존하지 않고 현재 row를 알려 주는 cursor column입니다.
                // idle marker도 같은 폭을 차지해 title alignment를 유지합니다.
                let marker = if selected {
                    // 학습 주석: selected marker는 overlay list 전반에서 공유하는 focus glyph입니다.
                    AkraTheme::selected_marker()
                } else {
                    // 학습 주석: idle marker는 선택되지 않은 row의 left padding 역할을 합니다.
                    AkraTheme::idle_marker()
                };
                // 학습 주석: 한 row 안에서 title만 bold하고, id/status/path는 detail text로 이어 붙입니다.
                // 이렇게 하면 사용자는 direction 이름을 먼저 보고, 필요하면 diagnosis metadata를 이어 읽습니다.
                Line::from(vec![
                    // 학습 주석: 첫 span은 focus marker column입니다.
                    Span::styled(marker, style),
                    // 학습 주석: direction title은 사람이 고르는 주 label이므로 bold로 표시합니다.
                    Span::styled(direction.title.clone(), style.add_modifier(Modifier::BOLD)),
                    // 학습 주석: detail span은 stable id, detail-doc health label, 현재 path를 함께 보여 줍니다.
                    // path가 없으면 <unset>으로 드러내 사용자가 생성 대상임을 바로 알 수 있게 합니다.
                    Span::styled(
                        format!(
                            "  id={} / status={} / path={}",
                            direction.id,
                            direction.detail_doc_status.label(),
                            direction.detail_doc_path.as_deref().unwrap_or("<unset>")
                        ),
                        style,
                    ),
                ])
            })
            // 학습 주석: renderer DTO는 Vec<Line>을 요구하므로 iterator를 materialize합니다.
            .collect()
    };
    // 학습 주석: status/header copy에는 full summary가 아니라 선택된 direction title만 필요합니다.
    // owned String으로 바꿔 projection DTO가 input borrow와 분리되게 합니다.
    let selected_direction_title = selected_direction.map(|direction| direction.title.clone());

    // 학습 주석: option lines와 selected title을 함께 반환해 caller가 같은 selection snapshot으로
    // list 영역과 status 영역을 구성하게 합니다.
    DetailDocSelectionProjection {
        option_lines,
        selected_direction_title,
    }
}
