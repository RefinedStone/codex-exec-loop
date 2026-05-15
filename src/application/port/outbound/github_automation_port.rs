// GitHub automation은 git push, gh CLI/API, network/auth 경계에 닿으므로 실패가 정상적인 결과이다.
// distributor는 이 오류를 blocked delivery state로 바꾸기 위해 `anyhow::Result`를 받는다.
use anyhow::Result;
// capability snapshot은 supervisor/session detail에 저장되므로 serde round-trip이 필요하다.
use serde::{Deserialize, Serialize};

// GitHub automation readiness도 parallel mode capability projection vocabulary를 공유한다.
// push remote, gh binary, auth 상태를 같은 snapshot 구조로 표현하면 supervisor UI가 일관된 readiness line을 만들 수 있다.
use crate::domain::parallel_mode::{ParallelModeCapabilitySnapshot, ParallelModeCapabilityState};

pub const DEFAULT_GITHUB_PUSH_REMOTE_NAME: &str = "origin";
pub const GITHUB_AUTOMATION_SCRIPT_RELATIVE_PATH: &str = "scripts/gh-akra.sh";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// `GithubAutomationCapabilities`는 distributor delivery가 GitHub write side를 사용할 수 있는지
// 판단하는 세 축의 readiness snapshot이다. push는 git remote 권한이고, PR 생성/조회/close는 gh binary와 auth가 필요하다.
pub struct GithubAutomationCapabilities {
    // agent branch를 origin에 push할 수 있는지 나타낸다.
    pub push_remote: ParallelModeCapabilitySnapshot,
    // `gh` CLI 또는 adapter가 요구하는 GitHub command surface가 있는지 나타낸다.
    pub gh_binary: ParallelModeCapabilitySnapshot,
    // GitHub auth가 현재 repo에서 유효한지 나타낸다.
    pub gh_auth: ParallelModeCapabilitySnapshot,
}

impl GithubAutomationCapabilities {
    // capability snapshots를 하나의 value object로 묶는 생성자이다.
    // 테스트 fake와 production adapter가 같은 생성자를 쓰면 readiness 축 순서가 어긋나지 않는다.
    pub fn new(
        // git push 가능성이다.
        push_remote: ParallelModeCapabilitySnapshot,
        // gh binary/command surface 가능성이다.
        gh_binary: ParallelModeCapabilitySnapshot,
        // GitHub authentication 가능성이다.
        gh_auth: ParallelModeCapabilitySnapshot,
    ) -> Self {
        Self {
            push_remote,
            gh_binary,
            gh_auth,
        }
    }

    // push readiness만 빠르게 확인하는 helper이다. distributor는 PR 단계 이전에
    // branch push가 막혀 있는지 별도로 판단해야 한다.
    pub fn push_ready(&self) -> bool {
        self.push_remote.state == ParallelModeCapabilityState::Ready
    }

    // PR 생성/조회/close 같은 GitHub API/CLI 작업 가능성을 판단한다.
    // push remote가 ready여도 gh binary/auth가 없으면 PR delivery는 별도 정책으로 처리해야 한다.
    pub fn pull_request_workflow_ready(&self) -> bool {
        self.gh_binary.state == ParallelModeCapabilityState::Ready
            && self.gh_auth.state == ParallelModeCapabilityState::Ready
    }

    // 기존 호출부와 serialized 의미를 깨지 않기 위한 호환 helper다.
    pub fn github_ready(&self) -> bool {
        self.pull_request_workflow_ready()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
// `GithubAutomationPullRequest`는 adapter가 GitHub PR 원문을 delivery orchestration이 쓰는 최소 shape로
// 정규화한 값이다. distributor는 이 값으로 PR 번호/URL을 session detail에 남기고, draft/open/base/head 상태를 검사한다.
pub struct GithubAutomationPullRequest {
    // GitHub PR number이다. 이후 inspect/close 호출의 key로 사용된다.
    pub number: u64,
    // 사람이 열 수 있는 PR URL이다. supervisor/detail UI에 노출된다.
    pub url: String,
    // GitHub가 돌려준 open/closed/merged 등 상태 문자열이다.
    pub state: String,
    // PR target branch이다. 이 repo의 delivery 흐름에서는 보통 `prerelease`여야 한다.
    pub base_branch: String,
    // PR source branch이다. slot/agent branch와 일치해야 distributor가 올바른 작업을 추적할 수 있다.
    pub head_branch: String,
    // draft PR 여부이다. delivery가 reviewable 상태인지 판단하는 데 사용된다.
    pub is_draft: bool,
}

impl GithubAutomationPullRequest {
    // PR projection 생성자이다. 문자열 필드를 `Into<String>`으로 받아 production JSON mapping과
    // 테스트 fixture가 모두 간결하게 값을 만들 수 있게 한다.
    pub fn new(
        // GitHub PR number이다.
        number: u64,
        // PR URL이다.
        url: impl Into<String>,
        // PR state이다.
        state: impl Into<String>,
        // target/base branch이다.
        base_branch: impl Into<String>,
        // source/head branch이다.
        head_branch: impl Into<String>,
        // draft flag이다.
        is_draft: bool,
    ) -> Self {
        Self {
            number,
            url: url.into(),
            state: state.into(),
            base_branch: base_branch.into(),
            head_branch: head_branch.into(),
            is_draft,
        }
    }
}

// `GithubAutomationPort`는 parallel distributor가 GitHub write-side delivery를 수행하기 위해
// 사용하는 outbound 계약이다. push, PR ensure/inspect, integration branch push, PR close를 service 정책에서
// 순서대로 호출하고, adapter는 git/gh 명령과 GitHub 응답 parsing을 소유한다.
pub trait GithubAutomationPort: Send + Sync {
    // repo에서 delivery capability를 점검한다. distributor snapshot은 이 값을 이용해
    // push blocked, GitHub unavailable, auth missing 같은 상태를 operator에게 보여 준다.
    fn inspect_capabilities(&self, repo_root: &str) -> GithubAutomationCapabilities;

    // agent/slot branch를 remote에 push한다. rebase recovery나 retry에서는
    // `force_with_lease`를 사용해 원격 변경을 무작정 덮어쓰지 않는 안전장치를 유지한다.
    fn push_branch(&self, repo_root: &str, branch_name: &str, force_with_lease: bool)
    -> Result<()>;

    // source branch에 대한 PR을 보장한다. 이미 열려 있으면 기존 PR을 반환하고,
    // 없으면 title/body로 새 PR을 만들어 delivery state가 PR number를 추적할 수 있게 한다.
    fn ensure_pull_request(
        &self,
        // GitHub remote가 설정된 repository root이다.
        repo_root: &str,
        // PR target branch이다.
        base_branch: &str,
        // PR source branch이다.
        head_branch: &str,
        // PR title이다.
        title: &str,
        // PR body이다. distributor는 worker result와 validation summary를 여기에 담는다.
        body: &str,
    ) -> Result<GithubAutomationPullRequest>;

    // 이미 알고 있는 PR number의 현재 상태를 다시 읽는다. blocked/retry/recovery 흐름은
    // 이 값을 통해 PR이 여전히 같은 head/base를 가리키는지 확인한다.
    fn inspect_pull_request(
        &self,
        // repository root이다.
        repo_root: &str,
        // 조회할 PR number이다.
        pr_number: u64,
    ) -> Result<GithubAutomationPullRequest>;

    // integration branch를 push한다. distributor가 prerelease 통합 branch를 갱신한 뒤
    // 원격에도 같은 상태를 반영하기 위해 사용한다.
    fn push_integration_branch(&self, repo_root: &str, branch_name: &str) -> Result<()>;

    // 더 이상 필요 없는 PR을 닫는다. 통합 완료나 recovery cleanup에서 중복 PR을 정리할 때 쓰인다.
    fn close_pull_request(&self, repo_root: &str, pr_number: u64) -> Result<()>;
}
