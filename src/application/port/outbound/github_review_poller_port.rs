// GitHub activity polling은 network, credentials, JSON parsing 경계에 닿으므로 실패할 수 있다.
// service는 실패를 TUI polling notice로 바꾸기 위해 `anyhow::Result`를 그대로 받는다.
use anyhow::Result;

// target은 어떤 PR을 읽을지 나타내는 domain key이고, snapshot은 GitHub API 응답을
// review/comment/activity 중심으로 정규화한 domain read model이다.
use crate::domain::github_review::{GithubPullRequestActivitySnapshot, GithubPullRequestTarget};

// `GithubReviewPollerPort`는 application service가 GitHub PR activity를 읽기 위해 요구하는
// outbound 계약이다. adapter는 GitHub token, REST endpoint, pagination, response DTO를 처리하고,
// service는 이 trait이 돌려주는 snapshot을 이전 snapshot과 비교해 새 activity만 TUI에 알린다.
//
// 이 port를 `GithubAutomationPort`와 분리한 이유는 책임이 다르기 때문이다. automation port는
// push/PR 생성/close 같은 write side delivery를 담당하고, review poller port는 review/comment 읽기 side만 담당한다.
pub trait GithubReviewPollerPort: Send + Sync {
    // target PR의 최신 activity snapshot을 읽는다. 반환 snapshot은 시간순 비교와
    // notification 계산에 쓰이므로, adapter는 GitHub 원문을 application-friendly ordering과 shape로 정규화해야 한다.
    fn load_pull_request_activity(
        &self,
        // repository, PR number, branch/ref 같은 PR 식별 정보를 담은 domain target이다.
        target: &GithubPullRequestTarget,
    ) -> Result<GithubPullRequestActivitySnapshot>;
}
