// overlay list projection은 ratatui가 그릴 수 있는 `Line` 단위로 이미 변환된 데이터를 담는다.
// domain/application 값이 아니라 shell presentation layer의 view DTO다.
use super::super::Line;

// `OverlayListEntryView`는 overlay list의 한 행 또는 한 항목을 표현한다. 항목 하나가 여러 줄로 렌더링될 수
// 있으므로 단일 String이 아니라 Line vector를 소유한다.
pub(crate) struct OverlayListEntryView {
    // lines는 이미 style/span 처리가 끝난 화면 줄 목록이다. renderer는 이 값을 다시 해석하지 않고 list item
    // 영역에 순서대로 배치한다.
    pub(crate) lines: Vec<Line<'static>>,
}

// `OverlayListView`는 overlay 목록 전체의 presentation snapshot이다. 상단 message, item rows, selection
// cursor를 함께 담아 renderer가 layout 계산에 필요한 값을 한 번에 받게 한다.
pub(crate) struct OverlayListView {
    // message_lines는 list 위에 보여 줄 안내/빈 상태/오류 문구다. None이면 별도 message band 없이 item list만
    // 그린다.
    pub(crate) message_lines: Option<Vec<Line<'static>>>,
    // items는 실제 list entry들이다. 각 entry가 자체 line vector를 가지므로 renderer는 item 간 간격과 선택
    // 표시만 책임진다.
    pub(crate) items: Vec<OverlayListEntryView>,
    // selected_index는 keyboard focus가 가리키는 item 위치다. None이면 선택 가능한 항목이 없거나 list가 단순
    // 정보 표시 mode라는 뜻이다.
    pub(crate) selected_index: Option<usize>,
}
