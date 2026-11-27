use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::domain::git_graph::{
    GitCommitDetail, GitGraphLaneState, GitGraphRef, GitGraphRow, GitGraphWorktree,
};

// 커밋 그래프 inbound 계약이다 (plan 27 G1, G2). Gateway 의 `akra/git/*` 가 쓴다.

/// 다음 쪽을 읽는 자리이다. 클라이언트는 받은 값을 그대로 돌려준다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitGraphCursor {
    /// 이미 읽은 커밋 수이다.
    pub skip: usize,
    /// 앞 쪽의 마지막 줄 아래의 레인 상태이다.
    pub lanes: GitGraphLaneState,
    /// 첫 쪽을 읽을 때의 ref 들의 지문이다. ref 가 바뀌면 이어 읽지 않는다.
    pub refs_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitGraphPageRequest {
    /// 저장소 안의 폴더이다. 절대 경로이다.
    pub repository_dir: String,
    pub limit: Option<usize>,
    /// 없으면 첫 쪽이다.
    pub cursor: Option<GitGraphCursor>,
}

/// 그래프의 한 쪽이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitGraphPage {
    /// 폴더가 git 저장소이다. 아니면 다른 값은 모두 비어 있다.
    pub repository: bool,
    /// 요청의 자리가 낡았다: 첫 쪽 뒤에 ref 가 바뀌었다. 줄은 비어 있다. 클라이언트는 첫 쪽부터 다시 읽는다.
    pub stale: bool,
    /// HEAD 커밋의 sha 이다.
    pub head: Option<String>,
    /// 이 쪽을 읽을 때의 ref 들의 지문이다. `refs_state` 의 값과 다르면 저장소가 바뀌었다. 저장소가 아니거나 낡은
    /// 자리의 답에는 없다.
    pub refs_digest: Option<String>,
    /// 첫 쪽에만 있다.
    pub refs: Vec<GitGraphRef>,
    /// 첫 쪽에만 있다.
    pub worktrees: Vec<GitGraphWorktree>,
    pub rows: Vec<GitGraphRow>,
    /// 다음 쪽의 자리이다. 마지막 쪽이면 없다.
    pub next: Option<GitGraphCursor>,
}

/// 저장소의 ref 들의 지금 상태이다. 화면은 이 값을 주기적으로 읽어 그래프를 다시 읽을지 정한다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitGraphRefsState {
    pub repository: bool,
    /// HEAD, ref, 작업 트리 가운데 하나가 바뀌면 값이 바뀐다. 저장소가 아니면 없다.
    pub refs_digest: Option<String>,
    /// 이 작업 트리의 끝나지 않은 작업이다 (`merge`, `rebase`, `cherry-pick`, `revert`, `bisect`). 지문에 넣지 않는다.
    pub operation_in_progress: Option<String>,
    /// 충돌이 풀리지 않은 파일 수이다. 지문에 넣지 않는다.
    pub conflicted_files: usize,
}

pub trait GitGraphPort: Send + Sync {
    /// 그래프의 한 쪽을 읽는다. 저장소를 고치지 않는다.
    fn graph_page(&self, request: GitGraphPageRequest) -> Result<GitGraphPage>;

    /// ref 들의 지문만 읽는다. 커밋은 읽지 않는다.
    fn refs_state(&self, repository_dir: &str) -> Result<GitGraphRefsState>;

    /// 커밋 하나의 상세이다. 그 커밋이 없으면 `None` 이다. `commit` 이 전체 sha 가 아니면 오류이다.
    fn commit_detail(&self, repository_dir: &str, commit: &str) -> Result<Option<GitCommitDetail>>;
}
