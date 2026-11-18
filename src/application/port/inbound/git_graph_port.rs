use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::domain::git_graph::{GitGraphLaneState, GitGraphRef, GitGraphRow};

// 커밋 그래프 inbound 계약이다 (plan 27 G1). Gateway 의 `akra/git/graph` 가 쓴다.

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
    /// 첫 쪽에만 있다.
    pub refs: Vec<GitGraphRef>,
    pub rows: Vec<GitGraphRow>,
    /// 다음 쪽의 자리이다. 마지막 쪽이면 없다.
    pub next: Option<GitGraphCursor>,
}

pub trait GitGraphPort: Send + Sync {
    /// 그래프의 한 쪽을 읽는다. 저장소를 고치지 않는다.
    fn graph_page(&self, request: GitGraphPageRequest) -> Result<GitGraphPage>;
}
