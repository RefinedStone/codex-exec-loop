// 학습 주석: 보호 파일 복원 결과는 repair service가 "건드리면 안 되는 planning 파일"을 되살렸는지
// caller에게 보고하는 값입니다. derive trait들은 복원 결과를 로그/테스트/비교에서 그대로 다루게 합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: `PlanningProtectedFileRestoration`은 특정 보호 파일의 logical path와, 복원에 사용된
// archive 후보가 있었는지를 함께 담습니다. repair summary는 이 DTO 목록으로 사용자에게 어떤 파일이
// 되살아났는지 설명할 수 있습니다.
pub struct PlanningProtectedFileRestoration {
    // 학습 주석: relative_path는 workspace root 기준 보호 파일 경로입니다. static str인 이유는 보호
    // 대상 파일 목록이 코드가 아는 고정 contract라 런타임 문자열 ownership이 필요 없기 때문입니다.
    pub relative_path: &'static str,
    // 학습 주석: archived_candidate_path는 실제 복원에 사용했거나 참고한 archive 파일 경로입니다.
    // None이면 archive 후보 없이 기본 보호 내용을 재생성했거나 후보가 없었다는 뜻을 표현합니다.
    pub archived_candidate_path: Option<String>,
}
