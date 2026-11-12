use anyhow::Result;

use crate::domain::kernel_event::RepositoryState;

/*
 * 작업 공간의 git 상태를 읽는 바깥 경계다 (plan 20 §4.1). 커널의 관찰자가 주기적으로 부른다. 판단은 하지 않는다.
 * 지난 관찰과 견주는 일은 domain 이 한다.
 */
pub trait RepositoryStatePort: Send + Sync {
    /// 체크아웃한 커밋과 브랜치이다. 커밋이 없는 저장소와 저장소가 아닌 폴더는 `None` 이다.
    fn repository_state(&self, workspace_dir: &str) -> Result<Option<RepositoryState>>;
}
