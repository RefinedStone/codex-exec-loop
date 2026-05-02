// 학습 주석: option line도 최종 overlay renderer가 소비하는 `Line`입니다. 이 파일은 선택지 주변의
// 설명성 line을 만들지만 실제 key handling은 shell runtime 쪽에 남아 있습니다.
use super::super::super::super::super::super::Line;
// 학습 주석: copy DTO는 staged draft name과 staged file count처럼 option 설명에 필요한 presentation 값을
// 이미 추출해 둔 구조입니다. builder는 application state를 다시 읽지 않습니다.
use super::super::super::copy::PlanningSimpleReviewCopy;

// 학습 주석: option lines는 사용자가 promote 전에 검토해야 할 선택지/결과를 정리합니다. action handler와
// 직접 연결되지는 않지만, key line의 "promote/detail/edit" 행동이 무엇을 의미하는지 설명하는 영역입니다.
pub(super) fn build_simple_review_option_lines(
    // 학습 주석: shared reference로 copy를 받는 이유는 option text 생성이 draft metadata를 읽기만 하고
    // downstream assembly에서도 copy 소유권이 필요할 수 있기 때문입니다.
    copy: &PlanningSimpleReviewCopy,
) -> Vec<Line<'static>> {
    vec![
        // 학습 주석: staged draft 이름을 첫 줄에 노출해 사용자가 어떤 planning scaffold를 promote하는지 확인합니다.
        Line::from(format!("staged draft: {}", copy.draft_name)),
        // 학습 주석: staged_file_count는 review 대상이 단일 문구가 아니라 여러 support artifact를 포함할 수
        // 있음을 알려 주는 검토량 표시입니다.
        Line::from(format!(
            "reviewed artifacts: {} staged planning support files",
            copy.staged_file_count
        )),
        // 학습 주석: promote outcome line은 Enter/Ctrl+P가 실제로 만드는 planning baseline을 요약합니다.
        Line::from(
            "promote outcome: generic direction catalog, empty task ledger, and default queue-idle review prompt",
        ),
        // 학습 주석: advanced path line은 simple scaffold를 받아들이지 않고 detail-mode authoring으로 우회할 수
        // 있음을 보여 줍니다. key handling은 별도지만 UI 문구가 사용자의 선택지를 연결합니다.
        Line::from(
            "advanced path: press D to branch into detail-mode authoring instead of promoting the simple scaffold",
        ),
    ]
}
