// 학습 주석: overlay list projection은 ratatui가 그릴 수 있는 `Line` 단위로 이미 변환된 데이터를
// 담습니다. domain/application 값이 아니라 shell presentation layer의 view DTO입니다.
use super::super::Line;

// 학습 주석: `OverlayListEntryView`는 overlay list의 한 행 또는 한 항목을 표현합니다. 항목 하나가
// 여러 줄로 렌더링될 수 있으므로 단일 String이 아니라 Line vector를 소유합니다.
pub(crate) struct OverlayListEntryView {
    // 학습 주석: lines는 이미 style/span 처리가 끝난 화면 줄 목록입니다. renderer는 이 값을 다시
    // 해석하지 않고 list item 영역에 순서대로 배치합니다.
    pub(crate) lines: Vec<Line<'static>>,
}

// 학습 주석: `OverlayListView`는 overlay 목록 전체의 presentation snapshot입니다. 상단 message,
// item rows, selection cursor를 함께 담아 renderer가 layout 계산에 필요한 값을 한 번에 받게 합니다.
pub(crate) struct OverlayListView {
    // 학습 주석: message_lines는 list 위에 보여 줄 안내/빈 상태/오류 문구입니다. None이면 별도
    // message band 없이 item list만 그립니다.
    pub(crate) message_lines: Option<Vec<Line<'static>>>,
    // 학습 주석: items는 실제 list entry들입니다. 각 entry가 자체 line vector를 가지므로 renderer는
    // item 간 간격과 선택 표시만 책임집니다.
    pub(crate) items: Vec<OverlayListEntryView>,
    // 학습 주석: selected_index는 keyboard focus가 가리키는 item 위치입니다. None이면 선택 가능한
    // 항목이 없거나 list가 단순 정보 표시 모드라는 뜻입니다.
    pub(crate) selected_index: Option<usize>,
}
