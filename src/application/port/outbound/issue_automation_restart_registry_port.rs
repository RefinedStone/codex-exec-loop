//! 사용자 profile 안에서 다시 확인할 저장소 후보다. 실행 권한은 저장하지 않는다.
use anyhow::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueAutomationRestartEntry {
    pub repository_key: String,
    pub workspace_root: String,
}

/// 호출자는 같은 저장소의 수준 변경을 소유 잠금 아래 직렬화한다.
/// 목록은 현재 선택·정책을 다시 읽을 후보일 뿐 수집이나 실행을 허가하지 않는다.
pub trait IssueAutomationRestartRegistryPort: Send + Sync {
    /// 같은 key와 root의 재시도는 추가 변경 없이 성공한다.
    fn register(&self, entry: &IssueAutomationRestartEntry) -> Result<()>;

    /// 없는 key와 아직 생성되지 않은 등록부의 삭제는 성공하며 파일을 만들지 않는다.
    fn unregister(&self, repository_key: &str) -> Result<()>;

    /// 없는 등록부는 빈 목록이다. 손상·권한 오류는 빈 목록으로 숨기지 않는다.
    fn list(&self) -> Result<Vec<IssueAutomationRestartEntry>>;
}
