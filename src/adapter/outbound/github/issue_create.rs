//! 승인한 저장소에 새 이슈를 한 번 생성한다 (#2436).
//! clientMutationId 는 상관관계용이며 GitHub 의 멱등 키가 아니다. 오류를 재전송하지 않는다.
//! 입력 계약: https://docs.github.com/en/graphql/reference/issues#createissueinput
use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;

use super::issue_snapshot::{GhApiRunner, ensure_node_id, graphql_arguments, run_gh_api};
use super::resource_version::{GITHUB_HOSTNAME, GithubIssueTarget};
use crate::application::port::outbound::issue_create_port::{
    IssueCreatePort, IssueCreateRequest, IssueCreateResponse, IssueRepositoryResolveRequest,
};
use crate::domain::issue_creation::{IssueCreatedLocation, IssueRepositoryReference};
use crate::domain::issue_work::{IssueLocator, IssueProvider, IssueReference, IssueSharedCopy};

const RESOLVE_REPOSITORY: &str = "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { __typename id nameWithOwner } }";
const CREATE_ISSUE: &str = "mutation($repositoryId: ID!, $title: String!, $body: String!, $clientMutationId: String!) { createIssue(input: { repositoryId: $repositoryId, title: $title, body: $body, clientMutationId: $clientMutationId }) { clientMutationId issue { __typename id number url title body repository { id nameWithOwner } } } }";

pub struct GithubIssueCreateAdapter {
    run_gh_api: GhApiRunner,
}

impl GithubIssueCreateAdapter {
    pub fn new(working_directory: PathBuf) -> Self {
        Self::with_runner(Box::new(move |arguments| {
            run_gh_api(&working_directory, arguments)
        }))
    }

    fn with_runner(run_gh_api: GhApiRunner) -> Self {
        Self { run_gh_api }
    }

    fn query<T: for<'de> Deserialize<'de>>(&self, arguments: &[String]) -> Result<T> {
        let output = (self.run_gh_api)(arguments)
            .map_err(|_| anyhow!("GitHub did not confirm the issue creation request"))?;
        // Unexpected remote text and GraphQL error messages must not appear in errors or logs.
        let envelope = serde_json::from_slice::<Envelope<T>>(&output)
            .map_err(|_| anyhow!("GitHub returned an invalid issue creation response"))?;
        if envelope.errors.is_some_and(|errors| !errors.is_empty()) {
            bail!("GitHub did not confirm the issue creation request")
        }
        envelope
            .data
            .ok_or_else(|| anyhow!("GitHub did not return issue creation data"))
    }
}

impl IssueCreatePort for GithubIssueCreateAdapter {
    fn resolve_repository(
        &self,
        request: IssueRepositoryResolveRequest,
    ) -> Result<IssueRepositoryReference> {
        let requested_name = format!("{}/{}", request.owner, request.repository);
        let key = format!("{requested_name}#1");
        let target = GithubIssueTarget::parse(&key)?;
        let data: RepositoryData = self.query(&graphql_arguments(
            RESOLVE_REPOSITORY,
            &[("owner", target.owner), ("name", target.repository)],
            None,
        ))?;
        let repository = data
            .repository
            .ok_or_else(|| anyhow!("GitHub did not return the requested repository"))?;
        ensure_node_id(&repository.id)?;
        if repository.typename != "Repository"
            || !repository
                .name_with_owner
                .eq_ignore_ascii_case(&requested_name)
        {
            bail!("GitHub did not return the requested repository")
        }
        parse_repository_name(&repository.name_with_owner)?;
        Ok(IssueRepositoryReference {
            provider: IssueProvider::GitHub,
            server_id: GITHUB_HOSTNAME.to_string(),
            repository_id: repository.id,
            project_scope: request.project_scope,
        })
    }

    fn create_issue(&self, request: IssueCreateRequest) -> Result<IssueCreateResponse> {
        request.ensure_authorized()?;
        if request.repository.provider != IssueProvider::GitHub
            || request.repository.server_id != GITHUB_HOSTNAME
        {
            bail!("the creation repository is not on github.com")
        }
        ensure_node_id(&request.repository.repository_id)?;
        ensure_node_id(&request.intent_id)?;
        let data: CreationData = self.query(&graphql_arguments(
            CREATE_ISSUE,
            &[
                ("repositoryId", &request.repository.repository_id),
                ("title", &request.shared_copy.title),
                ("body", &request.shared_copy.body),
                ("clientMutationId", &request.intent_id),
            ],
            None,
        ))?;
        let payload = data
            .create_issue
            .ok_or_else(|| anyhow!("GitHub did not confirm the created issue"))?;
        if payload.client_mutation_id.as_deref() != Some(request.intent_id.as_str()) {
            bail!("GitHub did not confirm the creation intent")
        }
        let issue = payload
            .issue
            .ok_or_else(|| anyhow!("GitHub did not confirm the created issue"))?;
        ensure_node_id(&issue.id)?;
        ensure_node_id(&issue.repository.id)?;
        if issue.typename != "Issue"
            || issue.repository.id != request.repository.repository_id
            || issue.number == 0
            || issue.number > i32::MAX as u64
        {
            bail!("GitHub returned a different creation target")
        }
        let (owner, repository) = parse_repository_name(&issue.repository.name_with_owner)?;
        let expected_url = format!(
            "https://{GITHUB_HOSTNAME}/{owner}/{repository}/issues/{}",
            issue.number
        );
        if issue.url != expected_url {
            bail!("GitHub returned an inconsistent created issue URL")
        }
        let created = IssueCreatedLocation {
            issue: IssueReference {
                provider: request.repository.provider,
                server_id: request.repository.server_id,
                repository_id: request.repository.repository_id,
                issue_id: issue.id,
                project_scope: request.repository.project_scope,
            },
            locator: IssueLocator {
                owner: owner.to_string(),
                repository: repository.to_string(),
                number: issue.number,
            },
            url: issue.url,
        };
        // The service compares this answer with the approval. It journals only its hash if different.
        Ok(IssueCreateResponse {
            created,
            published_copy: IssueSharedCopy {
                title: issue.title,
                body: issue.body,
            },
        })
    }
}

fn parse_repository_name(name: &str) -> Result<(&str, &str)> {
    let Some((owner, repository)) = name.split_once('/') else {
        bail!("GitHub returned an invalid repository name")
    };
    let key = format!("{owner}/{repository}#1");
    GithubIssueTarget::parse(&key)
        .map_err(|_| anyhow!("GitHub returned an invalid repository name"))?;
    Ok((owner, repository))
}

#[derive(Deserialize)]
struct Envelope<T> {
    data: Option<T>,
    errors: Option<Vec<serde_json::Value>>,
}

#[derive(Deserialize)]
struct RepositoryData {
    repository: Option<ResolvedRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolvedRepository {
    #[serde(rename = "__typename")]
    typename: String,
    id: String,
    name_with_owner: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreationData {
    create_issue: Option<CreationPayload>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreationPayload {
    client_mutation_id: Option<String>,
    issue: Option<CreatedIssue>,
}

#[derive(Deserialize)]
struct CreatedIssue {
    #[serde(rename = "__typename")]
    typename: String,
    id: String,
    number: u64,
    url: String,
    title: String,
    body: String,
    repository: CreatedRepository,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatedRepository {
    id: String,
    name_with_owner: String,
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use serde_json::{Value, json};

    use super::*;
    use crate::domain::issue_creation::IssueCreationApproval;
    use crate::domain::issue_work::{IssueFieldHash, IssueProjectScope};

    type Calls = Arc<Mutex<Vec<Vec<String>>>>;

    fn adapter(responses: Vec<Result<Value>>) -> (GithubIssueCreateAdapter, Calls) {
        let calls = Calls::default();
        let recorded = calls.clone();
        let responses = Mutex::new(VecDeque::from(responses));
        let adapter = GithubIssueCreateAdapter::with_runner(Box::new(move |arguments| {
            recorded.lock().unwrap().push(arguments.to_vec());
            responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("no fake answer")
                .map(|value| serde_json::to_vec(&value).unwrap())
        }));
        (adapter, calls)
    }

    fn repository() -> IssueRepositoryReference {
        IssueRepositoryReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".to_string(),
            repository_id: "R_1".to_string(),
            project_scope: IssueProjectScope {
                workspace_dir: "/work/project".to_string(),
            },
        }
    }

    fn request() -> IssueCreateRequest {
        let shared_copy = IssueSharedCopy {
            title: "@new title".to_string(),
            body: "@body\n[[akra-mask:1]]".to_string(),
        };
        IssueCreateRequest {
            repository: repository(),
            intent_id: "intent-1".to_string(),
            approval: IssueCreationApproval {
                repository: repository(),
                intent_id: "intent-1".to_string(),
                shared_copy_hash: IssueFieldHash::of_fields(&shared_copy.title, &shared_copy.body),
                policy_version: "policy-1".to_string(),
                template_version: Some("template-1".to_string()),
            },
            shared_copy,
        }
    }

    fn response() -> Value {
        let copy = request().shared_copy;
        json!({
            "data": {
                "createIssue": {
                    "clientMutationId": "intent-1",
                    "issue": {
                        "__typename": "Issue", "id": "I_1", "number": 7,
                        "url": "https://github.com/sample/widgets/issues/7",
                        "title": copy.title, "body": copy.body,
                        "repository": { "id": "R_1", "nameWithOwner": "sample/widgets" }
                    }
                }
            }
        })
    }

    #[test]
    fn resolves_the_named_repository_to_a_stable_id() {
        let (adapter, calls) = adapter(vec![Ok(json!({
            "data": { "repository": {
                "__typename": "Repository", "id": "R_1", "nameWithOwner": "Sample/Widgets"
            } }
        }))]);
        let resolved = adapter
            .resolve_repository(IssueRepositoryResolveRequest {
                owner: "sample".to_string(),
                repository: "widgets".to_string(),
                project_scope: repository().project_scope,
            })
            .unwrap();
        assert_eq!(resolved, repository());
        assert_eq!(calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn repository_resolution_rejects_redirects_and_invalid_names() {
        let (adapter, calls) = adapter(vec![Ok(json!({
            "data": { "repository": {
                "__typename": "Repository", "id": "R_2", "nameWithOwner": "other/widgets"
            } }
        }))]);
        let mut input = IssueRepositoryResolveRequest {
            owner: "sample".to_string(),
            repository: "widgets".to_string(),
            project_scope: repository().project_scope,
        };
        assert!(adapter.resolve_repository(input.clone()).is_err());
        input.owner = "../other".to_string();
        assert!(adapter.resolve_repository(input).is_err());
        assert_eq!(calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn sends_exactly_one_creation_with_only_approved_fields() {
        let (adapter, calls) = adapter(vec![Ok(response())]);
        let result = adapter.create_issue(request()).unwrap();
        assert_eq!(result.created.issue.issue_id, "I_1");
        assert_eq!(result.created.issue.repository_id, "R_1");
        assert_eq!(result.created.locator.number, 7);
        assert_eq!(result.published_copy, request().shared_copy);
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0],
            graphql_arguments(
                CREATE_ISSUE,
                &[
                    ("repositoryId", "R_1"),
                    ("title", "@new title"),
                    ("body", "@body\n[[akra-mask:1]]"),
                    ("clientMutationId", "intent-1"),
                ],
                None,
            )
        );
        for other in [
            "labelIds",
            "assigneeIds",
            "projectIds",
            "issueTemplate",
            "updateIssue",
        ] {
            assert!(!CREATE_ISSUE.contains(other));
        }
    }

    #[test]
    fn approval_or_provider_mismatch_sends_nothing() {
        let (adapter, calls) = adapter(Vec::new());
        let mut input = request();
        input.shared_copy.body.push_str("changed");
        assert!(adapter.create_issue(input).is_err());
        let mut input = request();
        input.repository.server_id = "other.example".to_string();
        input.approval.repository = input.repository.clone();
        assert!(adapter.create_issue(input).is_err());
        let mut input = request();
        input.intent_id = "other-intent".to_string();
        assert!(adapter.create_issue(input).is_err());
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn ambiguous_or_malformed_results_are_never_retried_and_do_not_leak_text() {
        let mut cases = Vec::new();
        for (field, value) in [
            ("__typename", json!("PullRequest")),
            ("id", json!("invalid id")),
            ("number", json!(0)),
            (
                "url",
                json!("https://other.example/sample/widgets/issues/7"),
            ),
            ("url", json!("https://github.com/sample/widgets/issues/8")),
        ] {
            let mut value_with_error = response();
            value_with_error["data"]["createIssue"]["issue"][field] = value;
            cases.push(Ok(value_with_error));
        }
        let mut wrong_repository = response();
        wrong_repository["data"]["createIssue"]["issue"]["repository"]["id"] = json!("R_2");
        cases.push(Ok(wrong_repository));
        let mut wrong_name = response();
        wrong_name["data"]["createIssue"]["issue"]["repository"]["nameWithOwner"] =
            json!("sample/../../other");
        cases.push(Ok(wrong_name));
        let mut wrong_intent = response();
        wrong_intent["data"]["createIssue"]["clientMutationId"] = json!("other-intent");
        cases.push(Ok(wrong_intent));
        cases.push(Ok(json!({
            "data": null, "errors": [{ "message": "unexpected-remote-plaintext" }]
        })));
        cases.push(Err(anyhow!("unexpected-remote-plaintext")));
        for reply in cases {
            let (adapter, calls) = adapter(vec![reply]);
            let error = adapter.create_issue(request()).unwrap_err();
            assert!(!format!("{error:#}").contains("unexpected-remote-plaintext"));
            assert_eq!(calls.lock().unwrap().len(), 1);
        }
    }

    #[test]
    fn a_different_returned_copy_keeps_the_stable_candidate_for_service_reconciliation() {
        let mut answer = response();
        answer["data"]["createIssue"]["issue"]["body"] = json!("changed remotely");
        let (adapter, calls) = adapter(vec![Ok(answer)]);
        let result = adapter.create_issue(request()).unwrap();
        assert_eq!(result.created.issue.issue_id, "I_1");
        assert_eq!(result.published_copy.body, "changed remotely");
        assert_eq!(calls.lock().unwrap().len(), 1);
    }
}
