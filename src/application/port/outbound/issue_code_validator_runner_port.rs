use std::time::Duration;

/*
 * 사용자가 승인한 코드 검증기를 실행하는 경계이다 (#2431). application service 는 어떤 검증기를 무슨
 * 입력으로 실행할지 정한다. 내용 해시 확인, 격리, 환경 변수 제거, 제한 시간은 outbound adapter 가 맡는다.
 *
 * 구현은 격리를 적용할 수 없으면 실행하지 않고 `IsolationUnavailable` 을 돌려준다. 격리 없는 실행은 없다.
 */

/// 검증기 한 번의 실행 요청이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueCodeValidatorRunRequest {
    /// 사용자가 승인한 실행 파일의 절대 경로이다.
    pub command: String,
    /// 승인한 파일 내용의 SHA-256 이다. 소문자 16진수이다.
    pub sha256: String,
    pub timeout: Duration,
    /// 검증기의 stdin 이다. 검증된 공유 초안의 JSON 이다.
    pub input_json: String,
}

/// 검증기가 실행되지 않은 이유이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueCodeValidatorNotRun {
    /// 정책이 고른 ID 가 사용자가 승인한 목록에 없다.
    NotApproved,
    /// 실행 파일이 없거나 읽을 수 없다.
    Missing,
    /// 파일 내용이 승인한 해시와 다르다.
    HashMismatch,
    /// 이 플랫폼에서 필요한 격리를 적용할 수 없다.
    IsolationUnavailable,
    TimedOut,
    /// 시작하지 못했거나, 신호로 끝났거나, 종료 코드가 0 과 1 이 아니다.
    Crashed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueCodeValidatorRunOutcome {
    /// 종료 코드 0 이다.
    Passed,
    /// 종료 코드 1 이다. 검증기가 초안을 거부했다. `message` 는 stdout 의 앞부분이다.
    Rejected {
        message: String,
    },
    NotRun(IssueCodeValidatorNotRun),
}

pub trait IssueCodeValidatorRunnerPort: Send + Sync {
    fn run(&self, request: IssueCodeValidatorRunRequest) -> IssueCodeValidatorRunOutcome;
}
