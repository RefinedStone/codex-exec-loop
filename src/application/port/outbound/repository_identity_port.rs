use anyhow::Result;

/// 폴더가 속한 저장소의 안정 식별자를 찾는다 (A03). 같은 저장소의 worktree 들은 같은 값을 갖고, 지운 checkout
/// 자리에 새로 만든 저장소는 다른 값을 갖는다. 다른 저장소의 자료가 경로 글자만으로 섞이지 않게 한다.
pub trait RepositoryIdentityPort: Send + Sync {
    fn repository_identity(&self, cwd: &str) -> Result<String>;
}
