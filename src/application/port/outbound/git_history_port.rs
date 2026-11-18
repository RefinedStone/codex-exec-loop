use anyhow::Result;

use crate::domain::git_graph::{GitGraphCommit, GitGraphRef};

/*
 * 저장소의 커밋 이력을 읽는 바깥 경계다 (plan 27 G1). 읽기만 한다. 순서와 레인 배치는 정하지 않는다. git 이 준
 * 순서(자식이 부모보다 먼저)를 그대로 돌려준다.
 */

/// 한 시점의 ref 들이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHistoryRefs {
    /// 이 작업 트리의 HEAD 커밋이다. 커밋이 없는 저장소는 `None` 이다.
    pub head: Option<String>,
    /// 로컬 브랜치, 원격 브랜치, 태그이다. 이름순이다.
    pub refs: Vec<GitGraphRef>,
}

pub trait GitHistoryPort: Send + Sync {
    /// 저장소의 ref 들이다. 저장소가 아닌 폴더는 `None` 이다.
    fn read_refs(&self, repository_dir: &str) -> Result<Option<GitHistoryRefs>>;

    /// 모든 브랜치와 태그에서 닿는 커밋을 날짜순으로 읽는다. 자식은 항상 부모보다 먼저 나온다. `include_head` 이면
    /// HEAD 에서 닿는 커밋도 읽는다 (브랜치가 아닌 커밋을 체크아웃한 경우). 앞의 `skip` 개를 건너뛰고 `limit`
    /// 개까지 읽는다.
    fn read_commits(
        &self,
        repository_dir: &str,
        include_head: bool,
        skip: usize,
        limit: usize,
    ) -> Result<Vec<GitGraphCommit>>;
}
