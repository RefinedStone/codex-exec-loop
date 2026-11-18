use std::collections::HashSet;

use anyhow::Result;

use crate::domain::git_graph::{GitCommitDetail, GitGraphCommit, GitGraphRef, GitGraphWorktree};

/*
 * 저장소의 커밋 이력을 읽는 바깥 경계다 (plan 27 G1, G2). 읽기만 한다. 순서와 레인 배치는 정하지 않는다. git 이 준
 * 순서(자식이 부모보다 먼저)를 그대로 돌려준다.
 */

/// 한 시점의 ref 들이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHistoryRefs {
    /// 이 작업 트리의 HEAD 커밋이다. 커밋이 없는 저장소는 `None` 이다.
    pub head: Option<String>,
    /// 로컬 브랜치, 원격 브랜치, 태그, 백업 ref 이다. git 이 준 순서이다.
    pub refs: Vec<GitGraphRef>,
    /// 저장소의 작업 트리들이다. 첫째가 대표 작업 트리이다.
    pub worktrees: Vec<GitGraphWorktree>,
}

pub trait GitHistoryPort: Send + Sync {
    /// 저장소의 ref 들이다. 저장소가 아닌 폴더는 `None` 이다.
    fn read_refs(&self, repository_dir: &str) -> Result<Option<GitHistoryRefs>>;

    /// 모든 브랜치, 태그, 백업 ref 에서 닿는 커밋을 날짜순으로 읽는다. 자식은 항상 부모보다 먼저 나온다.
    /// `include_head` 이면 HEAD 에서 닿는 커밋도 읽는다 (브랜치가 아닌 커밋을 체크아웃한 경우). 앞의 `skip` 개를
    /// 건너뛰고 `limit` 개까지 읽는다.
    fn read_commits(
        &self,
        repository_dir: &str,
        include_head: bool,
        skip: usize,
        limit: usize,
    ) -> Result<Vec<GitGraphCommit>>;

    /// 백업 ref 에서만 닿는 커밋들이다. 브랜치, 태그, HEAD(`include_head` 일 때)에서 닿는 커밋은 뺀다.
    fn read_backup_only_commits(
        &self,
        repository_dir: &str,
        include_head: bool,
    ) -> Result<HashSet<String>>;

    /// 커밋 하나의 상세이다. `commit` 은 전체 sha 이다 (`is_git_commit_id`). 그 커밋이 없으면 `None` 이다.
    fn read_commit_detail(
        &self,
        repository_dir: &str,
        commit: &str,
    ) -> Result<Option<GitCommitDetail>>;
}
