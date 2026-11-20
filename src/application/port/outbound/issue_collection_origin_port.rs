//! 대표 checkout의 origin에서 이슈 출처의 GitHub 좌표만 읽는다.
//! 인증 정보가 들어갈 수 있는 원격 URL은 이 경계를 넘지 않는다.
use anyhow::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueCollectionOrigin {
    pub owner: String,
    pub name: String,
}

pub trait IssueCollectionOriginPort: Send + Sync {
    fn github_origin(&self, workspace_dir: &str) -> Result<IssueCollectionOrigin>;
}
