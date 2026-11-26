use std::sync::atomic::{AtomicU8, Ordering};
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
    /// 사용자가 실행을 취소했다. 실행 중이면 제한 시간과 같은 정리 경로로 프로세스 트리를 끝낸다.
    Cancelled,
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

const VALIDATION_RUNNING: u8 = 0;
const VALIDATION_CANCELLED: u8 = 1;
const VALIDATION_FINISHED: u8 = 2;

/// 게시 검증 한 번의 취소 상태이다. 취소와 완료 중 먼저 받은 쪽만 성립한다: 취소를 받은 뒤에 끝난 성공은 버린다.
#[derive(Debug, Default)]
pub struct IssueCodeValidatorCancellation {
    state: AtomicU8,
}

impl IssueCodeValidatorCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    /// 실행기는 이 값을 제한 시간과 함께 확인한다.
    pub fn is_cancelled(&self) -> bool {
        self.state.load(Ordering::Acquire) == VALIDATION_CANCELLED
    }

    /// 취소를 받으면 `true` 이다. 이미 끝난 검증은 바꾸지 않고 `false` 이다.
    pub fn cancel(&self) -> bool {
        match self.state.compare_exchange(
            VALIDATION_RUNNING,
            VALIDATION_CANCELLED,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => true,
            Err(current) => current == VALIDATION_CANCELLED,
        }
    }

    /// 검증 결과를 확정한다. 취소를 먼저 받았으면 `false` 이고 결과는 게시에 쓰지 않는다.
    pub fn finish(&self) -> bool {
        self.state
            .compare_exchange(
                VALIDATION_RUNNING,
                VALIDATION_FINISHED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
}

pub trait IssueCodeValidatorRunnerPort: Send + Sync {
    /// `cancellation` 이 취소되면 실행하지 않거나 실행 중인 프로세스 트리를 끝내고 `Cancelled` 를 준다.
    fn run(
        &self,
        request: IssueCodeValidatorRunRequest,
        cancellation: &IssueCodeValidatorCancellation,
    ) -> IssueCodeValidatorRunOutcome;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_of_cancel_and_finish_wins() {
        let cancelled = IssueCodeValidatorCancellation::new();
        assert!(cancelled.cancel());
        assert!(cancelled.cancel());
        assert!(!cancelled.finish());
        assert!(cancelled.is_cancelled());

        let finished = IssueCodeValidatorCancellation::new();
        assert!(finished.finish());
        assert!(!finished.cancel());
        assert!(!finished.is_cancelled());
    }
}
