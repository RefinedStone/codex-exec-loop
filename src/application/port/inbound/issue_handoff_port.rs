use anyhow::Result;

use crate::domain::issue_work::{IssueFieldHash, IssueLocator, IssueSharedCopy};

// 이슈 인계의 inbound 계약이다 (#2428). Gateway 의 `akra/issue/*` 가 쓴다. 답은 공유본뿐이다.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueHandoffRequest {
    pub locator: IssueLocator,
    /// 이슈를 쓰는 로컬 프로젝트의 작업 폴더이다. 있으면 그 프로젝트의 정책 파일도 읽는다.
    pub workspace_dir: Option<String>,
    /// 승인한 공유본의 필드 해시이다. 있으면 원격 값이 이 해시와 같을 때만 인계한다.
    pub approved: Option<IssueFieldHash>,
}

pub trait IssueHandoffPort: Send + Sync {
    /// AI 에 넘길 제목과 본문이다. 원격 값을 읽고 AI 전달 정책으로 가린 뒤의 글이다. 댓글과 복원본은 없다.
    fn handoff_copy(&self, request: IssueHandoffRequest) -> Result<IssueSharedCopy>;
}
