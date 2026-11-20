//! GitHub 이슈 한 건의 안정 ID 와 지금 게시본을 `gh api graphql` 로 읽는다 (#2420). 읽기만 한다. 답을 저장하지
//! 않는다. 읽지 못하면 오류이고, 이전 답은 없다.
//!
//! 안정 ID 는 GraphQL 의 node ID 이다. 답의 종류, 이슈 ID, 저장소 ID 가 요청과 다르면 (PR, 옮겨진 이슈) 오류이다.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;

use super::resource_version::{
    GITHUB_HOSTNAME, GithubIssueTarget, gh_api_command, version_from_output,
};
use crate::application::port::outbound::issue_snapshot_port::{
    IssueRawFields, IssueSnapshotPort, IssueSnapshotReadRequest, IssueSnapshotResolveRequest,
    IssueSnapshotResponse,
};
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueProjectScope, IssueProvider, IssueReference,
};
use crate::subprocess;

/// 이슈 하나를 묻는 최대 시간이다.
const SNAPSHOT_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_NODE_ID_CHARS: usize = 200;

const RESOLVE_QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { id issueOrPullRequest(number: $number) { __typename ... on Issue { id number title body updatedAt repository { id } } } } }";
const READ_QUERY: &str = "query($id: ID!) { node(id: $id) { __typename ... on Issue { id number title body updatedAt repository { id } } } }";

/// `gh api --hostname github.com` 뒤의 인자를 받아 stdout 을 준다. 시험은 가짜를 넣는다.
pub(super) type GhApiRunner = Box<dyn Fn(&[String]) -> Result<Vec<u8>> + Send + Sync>;

pub struct GithubIssueSnapshotAdapter {
    run_gh_api: GhApiRunner,
}

impl GithubIssueSnapshotAdapter {
    /// `working_directory` 는 `gh` 를 찾고 실행할 때 기준이 되는 폴더이다. 저장소가 아닌 Akra 의 폴더를 준다.
    pub fn new(working_directory: PathBuf) -> Self {
        Self::with_runner(Box::new(move |arguments| {
            run_gh_api(&working_directory, arguments)
        }))
    }

    fn with_runner(run_gh_api: GhApiRunner) -> Self {
        Self { run_gh_api }
    }

    fn query<T: for<'de> Deserialize<'de>>(&self, arguments: &[String]) -> Result<T> {
        let stdout = (self.run_gh_api)(arguments)?;
        // serde 의 오류 글에는 답의 값이 들어갈 수 있다. 남기지 않는다.
        serde_json::from_slice::<GraphqlEnvelope<T>>(&stdout)
            .ok()
            .and_then(|envelope| envelope.data)
            .ok_or_else(|| anyhow!("GitHub did not return issue data"))
    }
}

impl IssueSnapshotPort for GithubIssueSnapshotAdapter {
    fn resolve(&self, request: IssueSnapshotResolveRequest) -> Result<IssueSnapshotResponse> {
        let locator = &request.locator;
        let key = format!(
            "{}/{}#{}",
            locator.owner, locator.repository, locator.number
        );
        let target = GithubIssueTarget::parse(&key)?;
        let data: RepositoryData = self.query(&graphql_arguments(
            RESOLVE_QUERY,
            &[("owner", target.owner), ("name", target.repository)],
            Some(target.number),
        ))?;
        let repository = data.repository.ok_or_else(target_not_found)?;
        let issue = repository
            .issue_or_pull_request
            .ok_or_else(target_not_found)?
            .into_issue()?;
        // 옮겨진 이슈는 새 저장소의 이슈로 답이 올 수 있다.
        if issue.repository_id != repository.id || issue.number != locator.number {
            bail!("the target moved to another repository or issue")
        }
        snapshot(issue, request.project_scope)
    }

    fn read(&self, request: IssueSnapshotReadRequest) -> Result<IssueSnapshotResponse> {
        let expected = request.issue;
        if expected.provider != IssueProvider::GitHub || expected.server_id != GITHUB_HOSTNAME {
            bail!("the issue is not on github.com")
        }
        ensure_node_id(&expected.issue_id)?;
        let data: NodeData = self.query(&graphql_arguments(
            READ_QUERY,
            &[("id", &expected.issue_id)],
            None,
        ))?;
        let issue = data.node.ok_or_else(target_not_found)?.into_issue()?;
        if issue.id != expected.issue_id || issue.repository_id != expected.repository_id {
            bail!("the target moved to another repository or issue")
        }
        snapshot(issue, expected.project_scope)
    }
}

fn target_not_found() -> anyhow::Error {
    anyhow!("GitHub does not have this issue")
}

/// `-f` 는 값을 글자 그대로 보낸다. `-F` 는 번호를 정수로 보낸다.
pub(super) fn graphql_arguments(
    query: &str,
    fields: &[(&str, &str)],
    number: Option<&str>,
) -> Vec<String> {
    let mut arguments = vec![
        "graphql".to_string(),
        "-f".to_string(),
        format!("query={query}"),
    ];
    for (name, value) in fields {
        arguments.extend(["-f".to_string(), format!("{name}={value}")]);
    }
    if let Some(number) = number {
        arguments.extend(["-F".to_string(), format!("number={number}")]);
    }
    arguments
}

pub(super) fn run_gh_api(working_directory: &Path, arguments: &[String]) -> Result<Vec<u8>> {
    let gh = crate::trusted_executable::resolve_native_from_current_path("gh", working_directory)
        .context("gh is not available")?;
    let mut command = gh_api_command(working_directory, &gh)?;
    command.args(arguments);
    let output = subprocess::command_output_with_timeout(
        &mut command,
        "gh api issue snapshot",
        SNAPSHOT_REQUEST_TIMEOUT,
    )
    .context("gh did not answer")?;
    if !output.status.success() {
        // stderr 에는 저장소 이름과 계정이 들어갈 수 있다. 남기지 않는다.
        bail!("gh could not read the issue")
    }
    Ok(output.stdout)
}

/// node ID 는 `gh` 의 인자가 되고 직렬화되는 참조에 들어간다. GitHub 이 쓰는 글자만 받는다.
pub(super) fn ensure_node_id(id: &str) -> Result<()> {
    let is_valid = !id.is_empty()
        && id.len() <= MAX_NODE_ID_CHARS
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-=".contains(character));
    if !is_valid {
        bail!("the GitHub node ID is not valid")
    }
    Ok(())
}

fn snapshot(issue: RemoteIssue, project_scope: IssueProjectScope) -> Result<IssueSnapshotResponse> {
    ensure_node_id(&issue.id)?;
    ensure_node_id(&issue.repository_id)?;
    let provider_updated_at = version_from_output(issue.updated_at.as_bytes())?;
    let raw_fields = IssueRawFields::new(issue.title, issue.body);
    Ok(IssueSnapshotResponse {
        issue: IssueReference {
            provider: IssueProvider::GitHub,
            server_id: GITHUB_HOSTNAME.to_string(),
            repository_id: issue.repository_id,
            issue_id: issue.id,
            project_scope,
        },
        comparison_basis: IssueComparisonBasis {
            field_hash: raw_fields.field_hash(),
            provider_updated_at,
        },
        raw_fields,
    })
}

/// 이슈로 확인한 답이다.
struct RemoteIssue {
    id: String,
    repository_id: String,
    number: u64,
    title: String,
    body: String,
    updated_at: String,
}

// 답의 타입은 원시 값을 가진다. `Debug` 를 구현하지 않는다.
#[derive(Deserialize)]
pub(super) struct GraphqlEnvelope<T> {
    pub(super) data: Option<T>,
}

#[derive(Deserialize)]
struct NodeData {
    node: Option<IssueNode>,
}

#[derive(Deserialize)]
struct RepositoryData {
    repository: Option<RepositoryNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryNode {
    id: String,
    issue_or_pull_request: Option<IssueNode>,
}

#[derive(Deserialize)]
struct RepositoryId {
    id: String,
}

/// 이슈가 아닌 답은 `__typename` 만 가진다.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueNode {
    #[serde(rename = "__typename")]
    typename: String,
    id: Option<String>,
    number: Option<u64>,
    title: Option<String>,
    body: Option<String>,
    updated_at: Option<String>,
    repository: Option<RepositoryId>,
}

impl IssueNode {
    fn into_issue(self) -> Result<RemoteIssue> {
        if self.typename != "Issue" {
            bail!("the target is not an issue")
        }
        let incomplete = || anyhow!("GitHub did not return issue data");
        Ok(RemoteIssue {
            id: self.id.ok_or_else(incomplete)?,
            repository_id: self.repository.ok_or_else(incomplete)?.id,
            number: self.number.ok_or_else(incomplete)?,
            title: self.title.ok_or_else(incomplete)?,
            body: self.body.ok_or_else(incomplete)?,
            updated_at: self.updated_at.ok_or_else(incomplete)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::domain::issue_work::{IssueFieldHash, IssueLocator};

    const ISSUE_NODE: &str = r##""__typename":"Issue","id":"I_kwDO-1","number":7,"title":"Raw title ","body":"line 1\r\n<!-- note -->\r\n**bold**","updatedAt":"2026-11-19T07:19:56Z","repository":{"id":"R_kgDO-1"}"##;

    type Calls = Arc<Mutex<Vec<Vec<String>>>>;

    /// `gh` 의 답을 순서대로 주는 가짜이다. 네트워크와 프로세스가 없다.
    fn adapter_with(responses: Vec<Result<String>>) -> (GithubIssueSnapshotAdapter, Calls) {
        let calls = Calls::default();
        let recorded = calls.clone();
        let responses = Mutex::new(VecDeque::from(responses));
        let adapter = GithubIssueSnapshotAdapter::with_runner(Box::new(move |arguments| {
            recorded.lock().unwrap().push(arguments.to_vec());
            let response = responses.lock().unwrap().pop_front();
            response
                .expect("the fake has no more answers")
                .map(String::into_bytes)
        }));
        (adapter, calls)
    }

    fn resolve_answer(repository_id: &str, node: &str) -> Result<String> {
        Ok(format!(
            r#"{{"data":{{"repository":{{"id":"{repository_id}","issueOrPullRequest":{{{node}}}}}}}}}"#
        ))
    }

    fn read_answer(node: &str) -> Result<String> {
        Ok(format!(r#"{{"data":{{"node":{{{node}}}}}}}"#))
    }

    fn project_scope() -> IssueProjectScope {
        IssueProjectScope {
            workspace_dir: "/work/project".to_string(),
        }
    }

    fn resolve_request(owner: &str, repository: &str, number: u64) -> IssueSnapshotResolveRequest {
        IssueSnapshotResolveRequest {
            locator: IssueLocator {
                owner: owner.to_string(),
                repository: repository.to_string(),
                number,
            },
            project_scope: project_scope(),
        }
    }

    fn reference() -> IssueReference {
        IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".to_string(),
            repository_id: "R_kgDO-1".to_string(),
            issue_id: "I_kwDO-1".to_string(),
            project_scope: project_scope(),
        }
    }

    fn read_request() -> IssueSnapshotReadRequest {
        IssueSnapshotReadRequest { issue: reference() }
    }

    #[test]
    fn resolve_reads_stable_ids_raw_fields_and_the_comparison_basis() {
        let (adapter, calls) = adapter_with(vec![resolve_answer("R_kgDO-1", ISSUE_NODE)]);

        let snapshot = adapter.resolve(resolve_request("octo", "repo", 7)).unwrap();

        assert_eq!(snapshot.issue, reference());
        // 필드 값은 그대로이다: 끝의 공백, `\r\n`, Markdown 글자.
        assert_eq!(snapshot.raw_fields.expose_title(), "Raw title ");
        assert_eq!(
            snapshot.raw_fields.expose_body(),
            "line 1\r\n<!-- note -->\r\n**bold**"
        );
        assert_eq!(
            snapshot.comparison_basis,
            IssueComparisonBasis {
                field_hash: IssueFieldHash::of_fields(
                    "Raw title ",
                    "line 1\r\n<!-- note -->\r\n**bold**"
                ),
                provider_updated_at: "2026-11-19T07:19:56Z".to_string(),
            }
        );
        assert_eq!(
            *calls.lock().unwrap(),
            [[
                "graphql",
                "-f",
                &format!("query={RESOLVE_QUERY}"),
                "-f",
                "owner=octo",
                "-f",
                "name=repo",
                "-F",
                "number=7"
            ]]
        );
    }

    #[test]
    fn read_asks_by_stable_id_and_returns_the_same_snapshot() {
        let (adapter, calls) = adapter_with(vec![
            resolve_answer("R_kgDO-1", ISSUE_NODE),
            read_answer(ISSUE_NODE),
        ]);

        let resolved = adapter.resolve(resolve_request("octo", "repo", 7)).unwrap();
        let read = adapter.read(read_request()).unwrap();

        assert_eq!(read, resolved);
        assert_eq!(
            calls.lock().unwrap()[1],
            [
                "graphql",
                "-f",
                &format!("query={READ_QUERY}"),
                "-f",
                "id=I_kwDO-1"
            ]
        );
    }

    #[test]
    fn a_failed_remote_read_is_an_error_and_never_an_earlier_snapshot() {
        let (adapter, calls) = adapter_with(vec![
            read_answer(ISSUE_NODE),
            Err(anyhow!("gh could not read the issue")),
            Ok("not json".to_string()),
            Ok(r#"{"data":null,"errors":[{"type":"NOT_FOUND"}]}"#.to_string()),
        ]);

        assert!(adapter.read(read_request()).is_ok());
        for _ in 0..3 {
            assert!(adapter.read(read_request()).is_err());
        }
        // 실패마다 원격을 다시 불렀다. 저장한 답이 없다.
        assert_eq!(calls.lock().unwrap().len(), 4);
    }

    #[test]
    fn a_pull_request_is_not_an_issue_target() {
        let pull_request = r#""__typename":"PullRequest""#;
        let (adapter, _) = adapter_with(vec![
            resolve_answer("R_kgDO-1", pull_request),
            read_answer(pull_request),
        ]);

        let error = adapter
            .resolve(resolve_request("octo", "repo", 7))
            .unwrap_err();
        assert_eq!(error.to_string(), "the target is not an issue");
        let error = adapter.read(read_request()).unwrap_err();
        assert_eq!(error.to_string(), "the target is not an issue");
    }

    #[test]
    fn a_target_that_moved_to_another_repository_or_issue_is_rejected() {
        let other_repository = ISSUE_NODE.replace("R_kgDO-1", "R_kgDO-2");
        let other_issue = ISSUE_NODE.replace("I_kwDO-1", "I_kwDO-2");
        let other_number = ISSUE_NODE.replace(r#""number":7"#, r#""number":8"#);
        let (adapter, _) = adapter_with(vec![
            // 옮겨진 이슈: 물은 저장소와 답의 저장소가 다르다.
            resolve_answer("R_kgDO-1", &other_repository),
            resolve_answer("R_kgDO-1", &other_number),
            read_answer(&other_repository),
            read_answer(&other_issue),
        ]);

        for result in [
            adapter.resolve(resolve_request("octo", "repo", 7)),
            adapter.resolve(resolve_request("octo", "repo", 7)),
            adapter.read(read_request()),
            adapter.read(read_request()),
        ] {
            assert_eq!(
                result.unwrap_err().to_string(),
                "the target moved to another repository or issue"
            );
        }
    }

    #[test]
    fn a_missing_target_is_an_error() {
        let (adapter, _) = adapter_with(vec![
            Ok(r#"{"data":{"repository":null}}"#.to_string()),
            Ok(
                r#"{"data":{"repository":{"id":"R_kgDO-1","issueOrPullRequest":null}}}"#
                    .to_string(),
            ),
            Ok(r#"{"data":{"node":null}}"#.to_string()),
        ]);

        assert!(adapter.resolve(resolve_request("octo", "repo", 7)).is_err());
        assert!(adapter.resolve(resolve_request("octo", "repo", 7)).is_err());
        assert!(adapter.read(read_request()).is_err());
    }

    #[test]
    fn invalid_targets_never_reach_gh() {
        let (adapter, calls) = adapter_with(Vec::new());

        for (owner, repository) in [
            ("octo", "repo --hostname evil.example"),
            ("octo/other", "repo"),
            ("", "repo"),
            ("octo", ".."),
        ] {
            assert!(
                adapter
                    .resolve(resolve_request(owner, repository, 7))
                    .is_err()
            );
        }
        for (server_id, issue_id) in [
            ("github.example", "I_kwDO-1"),
            ("github.com", ""),
            ("github.com", "I_1 -f query=x"),
            ("github.com", "@file"),
        ] {
            let issue = IssueReference {
                server_id: server_id.to_string(),
                issue_id: issue_id.to_string(),
                ..reference()
            };
            assert!(adapter.read(IssueSnapshotReadRequest { issue }).is_err());
        }
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn provider_text_outside_the_raw_fields_is_validated() {
        let bad_time = ISSUE_NODE.replace("2026-11-19T07:19:56Z", "a secret, not a time");
        let bad_id = ISSUE_NODE.replace("R_kgDO-1", "R 1 secret");
        let (adapter, _) = adapter_with(vec![
            read_answer(&bad_time),
            resolve_answer("R 1 secret", &bad_id),
        ]);

        assert!(adapter.read(read_request()).is_err());
        assert!(adapter.resolve(resolve_request("octo", "repo", 7)).is_err());
    }

    #[test]
    fn the_production_command_is_the_shared_gh_api_command() {
        let command = gh_api_command(&std::env::temp_dir(), Path::new("gh")).unwrap();
        let arguments: Vec<_> = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert_eq!(arguments, ["api", "--hostname", "github.com"]);
    }
}
