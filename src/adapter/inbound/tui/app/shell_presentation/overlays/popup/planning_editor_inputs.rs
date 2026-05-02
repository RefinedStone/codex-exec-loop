// 학습 주석: 편집기 입력 투영은 도메인 검증 결과를 새로 판단하지 않고, 이미 계산된
// `PlanningValidationReport`를 읽어 TUI가 한 줄 상태 문구로 보여 줄 수 있는 정보만 뽑습니다.
use crate::domain::planning::PlanningValidationReport;

// 학습 주석: 닫기 위험도는 위쪽 UI 런타임이 판단한 상태입니다. 이 파일은 위험도를 유지한 채
// copy DTO에 싣기만 해서, 입력 해석과 문구 조립 책임이 섞이지 않게 합니다.
use super::super::super::super::super::planning_draft_editor_ui::PlanningDraftEditorCloseRisk;
// 학습 주석: footer notice와 같은 제한 폭을 공유하면 staged path, dirty label, 검증 메시지가
// 서로 다른 규칙으로 잘려 보이는 일을 막을 수 있습니다.
use super::super::super::super::{FOOTER_NOTICE_DETAIL_LIMIT, compact_inline_detail};
// 학습 주석: 이 파일의 출력은 `copy` 모듈의 표시 전용 구조체입니다. lifetime을 빌려 쓰는
// 필드와 새로 만든 `String` 필드를 한곳에서 맞춰 surface builder가 단순 렌더링만 하게 둡니다.
use super::copy::{PlanningDraftEditorIssueCopy, PlanningDraftEditorStatusCopy};

// 학습 주석: 이 함수는 여러 상태 소스를 하나의 copy DTO로 모으는 어댑터 경계라 인자가 많습니다.
// 별도 설정 구조체로 감추면 호출자가 어떤 상태를 화면에 넘기는지 덜 보이므로, 여기서는 평평한
// 조립 함수로 두고 clippy 예외를 이 경계에만 좁게 겁니다.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_planning_draft_editor_status_copy<'a>(
    // 학습 주석: draft 이름과 active path는 원본 상태의 문자열을 그대로 빌려 씁니다. 표시 계층이
    // 소유권을 새로 만들지 않기 때문에 editor state와 status copy의 생명주기가 함께 묶입니다.
    draft_name: &'a str,
    active_path: &'a str,
    // 학습 주석: 선택 위치와 전체 파일 수는 본문 리스트의 커서 상태를 footer 쪽 문구와 연결합니다.
    // 여기서 숫자를 보정하지 않는 이유는 선택 계산 책임이 planning editor runtime에 있기 때문입니다.
    selected_file_position: usize,
    file_count: usize,
    // 학습 주석: 검증 보고서는 저장 가능 여부와 첫 번째 문제 문구의 원천입니다. 전체 이슈 목록은
    // 별도 검토 화면의 몫이고, 편집기 상태 줄은 가장 앞의 신호만 보여 주도록 축약합니다.
    validation_report: &PlanningValidationReport,
    // 학습 주석: staged path는 사용자에게 실제로 쓰일 임시 파일 위치를 알려 주는 값입니다. 경로가
    // 길 수 있으므로 DTO에 넣기 전 같은 compact 규칙으로 줄입니다.
    staged_path: &str,
    // 학습 주석: dirty label은 편집 중 바뀐 planning 파일 종류를 사람이 읽는 라벨로 받은 목록입니다.
    // 문자열 라벨로 받은 뒤 합치는 덕분에 이 모듈은 파일 종류 enum의 표시 규칙을 다시 알 필요가 없습니다.
    dirty_labels: &[String],
    // 학습 주석: next action은 키 입력 처리 쪽에서 이미 결정한 안내 문구입니다. copy 단계에서
    // 명령 가능 여부를 재계산하지 않아, 상태 전이와 표현 문구가 서로 다른 판단을 하지 않게 합니다.
    next_action: &'static str,
    // 학습 주석: close risk와 confirmation flag는 닫기/폐기 확인 흐름의 현재 단계입니다. 둘을 함께
    // 전달해야 copy builder가 평상시 안내와 확인 대기 안내를 구분할 수 있습니다.
    close_risk: Option<PlanningDraftEditorCloseRisk>,
    confirmation_pending: bool,
) -> PlanningDraftEditorStatusCopy<'a> {
    PlanningDraftEditorStatusCopy {
        draft_name,
        active_path,
        selected_file_position,
        file_count,
        // 학습 주석: `validation_ok`는 status copy에서 저장 가능/문제 있음 문구를 고르는 가장 거친
        // 신호입니다. 상세 내용은 아래 `first_issue`에 별도로 담아 두 신호를 독립적으로 테스트합니다.
        validation_ok: validation_report.is_valid(),
        // 학습 주석: 첫 이슈만 싣는 정책은 footer가 여러 검증 메시지로 넘치지 않게 하는 UI 선택입니다.
        // 더 자세한 목록이 필요하면 검증 보고서 자체를 소비하는 별도 화면에서 처리해야 합니다.
        first_issue: build_first_issue_copy(validation_report),
        // 학습 주석: staged path는 복사해서 축약 문자열로 만들기 때문에 status copy가 원본 path 길이에
        // 끌려 레이아웃을 흔들지 않습니다.
        staged_path_summary: compact_inline_detail(staged_path, FOOTER_NOTICE_DETAIL_LIMIT),
        // 학습 주석: dirty label 요약과 존재 여부를 따로 둡니다. 표시 문구는 `"none"`이어도,
        // 버튼/색상 같은 분기에는 빈 목록 여부가 더 직접적인 신호입니다.
        dirty_label_summary: summarize_dirty_labels(dirty_labels),
        has_dirty_labels: !dirty_labels.is_empty(),
        next_action,
        close_risk,
        confirmation_pending,
    }
}

// 학습 주석: 검증 보고서에서 화면에 바로 보여 줄 첫 문제만 표시 전용 구조체로 바꿉니다. 보고서의
// 정렬 순서를 존중하므로, "어떤 문제가 우선인가"라는 정책은 도메인 검증 단계에 남아 있습니다.
fn build_first_issue_copy(
    validation_report: &PlanningValidationReport,
) -> Option<PlanningDraftEditorIssueCopy> {
    validation_report
        // 학습 주석: `issues.first()`를 사용해 빈 보고서는 `None`, 문제가 있으면 가장 앞의 하나만
        // 선택합니다. `Option`으로 남겨 두면 호출부가 "문제 없음"을 별도 sentinel 문자열로 표현하지 않아도 됩니다.
        .issues
        .first()
        // 학습 주석: map 안에서는 도메인 이슈를 TUI copy 필드로 옮깁니다. severity는 색상/강조의
        // 입력이라 그대로 보존하고, message만 폭 제한에 맞춰 줄입니다.
        .map(|issue| PlanningDraftEditorIssueCopy {
            severity: issue.severity,
            detail: compact_inline_detail(&issue.message, FOOTER_NOTICE_DETAIL_LIMIT),
        })
}

// 학습 주석: dirty label 목록을 footer 한 칸에 들어갈 문자열로 바꿉니다. 빈 목록도 빈 문자열이 아니라
// `"none"`으로 표현해, 화면에서 "정보가 누락됨"과 "변경 없음"이 다르게 읽히도록 합니다.
fn summarize_dirty_labels(dirty_labels: &[String]) -> String {
    if dirty_labels.is_empty() {
        "none".to_string()
    } else {
        // 학습 주석: 여러 라벨은 쉼표로 연결한 뒤 같은 footer 폭으로 축약합니다. 먼저 join한 다음
        // compact해야 전체 문장이 한 번만 잘리고, 라벨별 축약이 겹쳐 읽기 어려워지는 일을 피합니다.
        compact_inline_detail(&dirty_labels.join(", "), FOOTER_NOTICE_DETAIL_LIMIT)
    }
}

// 학습 주석: 이 테스트 모듈은 런타임 전체를 띄우지 않고 copy projection의 작은 정책만 고정합니다.
// 그래서 수동 편집기 회귀 테스트가 깨지기 전에도 요약 문자열 정책 변화를 빠르게 잡을 수 있습니다.
#[cfg(test)]
mod tests {
    // 학습 주석: 테스트는 공개 surface가 아니라 이 파일의 내부 조립 함수를 직접 검증합니다. copy
    // projection의 정책이 바뀌면 화면 렌더링과 독립적으로 원인을 좁힐 수 있습니다.
    use super::{build_planning_draft_editor_status_copy, summarize_dirty_labels};
    // 학습 주석: 도메인 검증 타입을 직접 만들어 warning/error 순서가 copy에 어떻게 반영되는지 확인합니다.
    use crate::domain::planning::{
        PlanningFileKind, PlanningValidationReport, PlanningValidationSeverity,
    };

    // 학습 주석: clean 상태는 dirty label summary가 빈 문자열이 아니라 `"none"`이어야 합니다. 이 값은
    // footer에서 사용자가 변경 없음 상태를 명확히 읽도록 하는 표시 계약입니다.
    #[test]
    fn dirty_label_summary_reports_none_when_clean() {
        assert_eq!(summarize_dirty_labels(&[]), "none");
    }

    // 학습 주석: 첫 검증 이슈 우선 정책을 고정합니다. warning 뒤에 error가 오더라도 이 함수는 심각도
    // 재정렬을 하지 않고 보고서가 준 첫 항목을 그대로 표시해야 합니다.
    #[test]
    fn status_copy_prefers_first_validation_issue() {
        // 학습 주석: warning을 먼저 넣고 error를 나중에 넣어, copy projection이 severity 기준으로
        // 임의 재정렬하지 않는다는 조건을 눈에 보이게 만듭니다.
        let mut validation_report = PlanningValidationReport::default();
        validation_report.push_warning(
            PlanningFileKind::Directions,
            "first-warning",
            "first issue should be clearer",
        );
        validation_report.push_error(
            PlanningFileKind::Directions,
            "second-error",
            "second issue is critical",
        );

        // 학습 주석: 나머지 인자는 status copy 조립에 필요한 최소 상태를 채우는 고정값입니다. 여기서
        // 관심 있는 축은 validation report가 first_issue로 어떻게 내려가는지입니다.
        let status_copy = build_planning_draft_editor_status_copy(
            "draft-1",
            "planning/result-output.md",
            1,
            1,
            &validation_report,
            ".draft/planning/result-output.md",
            &[],
            "next action: inspect",
            None,
            false,
        );

        // 학습 주석: `expect`는 이 테스트의 전제인 "검증 이슈가 하나 이상 있다"를 문서화합니다. None이면
        // copy projection이 보고서를 잃어버린 것이므로 이후 severity 검사보다 먼저 실패해야 합니다.
        let first_issue = status_copy
            .first_issue
            .expect("first issue should exist for warnings");
        assert_eq!(first_issue.severity, PlanningValidationSeverity::Warning);
        assert!(first_issue.detail.contains("first issue should be clearer"));
    }
}
