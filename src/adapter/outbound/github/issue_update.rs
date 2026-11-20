//! GitHub 이슈 한 건의 제목과 본문을 `gh api graphql` 로 고친다 (#2423). 이슈 스냅샷(#2420)과 같은 `gh` 경로를
//! 쓴다.
//!
//! 쓰기는 `updateIssue` 하나이다. 인자는 이슈 ID, 제목, 본문뿐이다. 새 이슈, 댓글, 첨부, 라벨, 보드 필드는 쓰지
//! 않는다. `updateIssue` 는 조건 없는 쓰기이다: 원격이 그 사이에 바뀌었어도 덮어쓴다. 쓰기 직전의 비교는
//! `IssuePublishService` 가 한다. 아직 이 어댑터를 연결한 곳이 없다 (게시 의도 기록 #2424 뒤에 연결한다).

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;

use super::issue_snapshot::{
    GhApiRunner, GraphqlEnvelope, ensure_node_id, graphql_arguments, run_gh_api,
};
use super::resource_version::GITHUB_HOSTNAME;
use crate::application::port::outbound::issue_update_port::{
    IssueUpdatePort, IssueUpdateRequest, IssueUpdateResponse,
};
use crate::domain::issue_work::{IssueProvider, IssueSharedCopy};

const UPDATE_MUTATION: &str = "mutation($id: ID!, $title: String!, $body: String!) { updateIssue(input: { id: $id, title: $title, body: $body }) { issue { id title body repository { id } } } }";

pub struct GithubIssueUpdateAdapter {
    run_gh_api: GhApiRunner,
}

impl GithubIssueUpdateAdapter {
    /// `working_directory` 는 `gh` 를 찾고 실행할 때 기준이 되는 폴더이다. 저장소가 아닌 Akra 의 폴더를 준다.
    pub fn new(working_directory: PathBuf) -> Self {
        Self::with_runner(Box::new(move |arguments| {
            run_gh_api(&working_directory, arguments)
        }))
    }

    fn with_runner(run_gh_api: GhApiRunner) -> Self {
        Self { run_gh_api }
    }
}

impl IssueUpdatePort for GithubIssueUpdateAdapter {
    fn update_title_and_body(&self, request: IssueUpdateRequest) -> Result<IssueUpdateResponse> {
        request.ensure_authorized()?;
        let issue = request.issue;
        if issue.provider != IssueProvider::GitHub || issue.server_id != GITHUB_HOSTNAME {
            bail!("the issue is not on github.com")
        }
        ensure_node_id(&issue.issue_id)?;
        // shortcut: 제목과 본문이 `gh` 의 인자로 간다 (같은 기기의 프로세스 목록에 보인다, 인자 길이 한도가 있다).
        // 공유본이라서 받아들인다. 한도에 닿으면 `gh api --input -` 로 바꾼다.
        // `-f` 는 값을 글자 그대로 보낸다. `@` 로 시작하는 값도 파일로 읽지 않는다.
        let stdout = (self.run_gh_api)(&graphql_arguments(
            UPDATE_MUTATION,
            &[
                ("id", &issue.issue_id),
                ("title", &request.shared_copy.title),
                ("body", &request.shared_copy.body),
            ],
            None,
        ))
        .context("the issue update did not complete")?;
        // serde 의 오류 글에는 답의 값이 들어갈 수 있다. 남기지 않는다.
        let updated = serde_json::from_slice::<GraphqlEnvelope<UpdateData>>(&stdout)
            .ok()
            .and_then(|envelope| envelope.data)
            .and_then(|data| data.update_issue)
            .and_then(|payload| payload.issue)
            .ok_or_else(|| anyhow!("GitHub did not confirm the issue update"))?;
        if updated.id != issue.issue_id || updated.repository.id != issue.repository_id {
            bail!("GitHub did not confirm the issue update")
        }
        Ok(IssueUpdateResponse {
            issue,
            published_copy: IssueSharedCopy {
                title: updated.title,
                body: updated.body,
            },
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateData {
    update_issue: Option<UpdatePayload>,
}

#[derive(Deserialize)]
struct UpdatePayload {
    issue: Option<UpdatedIssue>,
}

#[derive(Deserialize)]
struct UpdatedIssue {
    id: String,
    title: String,
    body: String,
    repository: RepositoryId,
}

#[derive(Deserialize)]
struct RepositoryId {
    id: String,
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::domain::issue_work::{
        IssueProjectScope, IssueReference, IssueWriteApproval, IssueWriteOperation,
    };

    type Calls = Arc<Mutex<Vec<Vec<String>>>>;

    /// `gh` 의 답을 순서대로 주는 가짜이다. 네트워크와 프로세스가 없다.
    fn adapter_with(responses: Vec<Result<String>>) -> (GithubIssueUpdateAdapter, Calls) {
        let calls = Calls::default();
        let recorded = calls.clone();
        let responses = Mutex::new(VecDeque::from(responses));
        let adapter = GithubIssueUpdateAdapter::with_runner(Box::new(move |arguments| {
            recorded.lock().unwrap().push(arguments.to_vec());
            let response = responses.lock().unwrap().pop_front();
            response
                .expect("the fake has no more answers")
                .map(String::into_bytes)
        }));
        (adapter, calls)
    }

    fn update_answer(issue_id: &str, repository_id: &str) -> Result<String> {
        Ok(format!(
            r#"{{"data":{{"updateIssue":{{"issue":{{"id":"{issue_id}","title":"New title","body":"@new\nbody","repository":{{"id":"{repository_id}"}}}}}}}}}}"#
        ))
    }

    fn reference() -> IssueReference {
        IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".to_string(),
            repository_id: "R_kgDO-1".to_string(),
            issue_id: "I_kwDO-1".to_string(),
            project_scope: IssueProjectScope {
                workspace_dir: "/work/project".to_string(),
            },
        }
    }

    fn request(operation: IssueWriteOperation) -> IssueUpdateRequest {
        IssueUpdateRequest {
            approval: IssueWriteApproval {
                issue: reference(),
                operation,
            },
            issue: reference(),
            shared_copy: IssueSharedCopy {
                title: "New title".to_string(),
                body: "@new\nbody".to_string(),
            },
        }
    }

    #[test]
    fn update_sends_one_mutation_with_only_the_issue_id_title_and_body() {
        let (adapter, calls) = adapter_with(vec![update_answer("I_kwDO-1", "R_kgDO-1")]);

        let response = adapter
            .update_title_and_body(request(IssueWriteOperation::Body))
            .unwrap();

        assert_eq!(response.issue, reference());
        assert_eq!(
            response.published_copy,
            request(IssueWriteOperation::Body).shared_copy
        );
        // 인자 전부이다. 값은 `-f` 로 글자 그대로 간다.
        assert_eq!(
            *calls.lock().unwrap(),
            [[
                "graphql",
                "-f",
                &format!("query={UPDATE_MUTATION}"),
                "-f",
                "id=I_kwDO-1",
                "-f",
                "title=New title",
                "-f",
                "body=@new\nbody"
            ]]
        );
        // 쓰기 문은 이슈의 제목과 본문만 고친다. 새 이슈, 댓글, 보드 필드를 쓰는 mutation 이 없다.
        assert_eq!(UPDATE_MUTATION.matches("mutation").count(), 1);
        assert!(
            UPDATE_MUTATION.contains("updateIssue(input: { id: $id, title: $title, body: $body })")
        );
        for other in ["createIssue", "addComment", "updateProjectV2", "labelIds"] {
            assert!(!UPDATE_MUTATION.contains(other));
        }
    }

    #[test]
    fn a_wrong_approval_or_target_never_reaches_gh() {
        let (adapter, calls) = adapter_with(Vec::new());
        let body = || request(IssueWriteOperation::Body);
        let mut other_issue = body();
        other_issue.approval.issue.issue_id = "I_kwDO-2".to_string();
        let with_issue = |server_id: &str, issue_id: &str| {
            let issue = IssueReference {
                server_id: server_id.to_string(),
                issue_id: issue_id.to_string(),
                ..reference()
            };
            IssueUpdateRequest {
                approval: IssueWriteApproval {
                    issue: issue.clone(),
                    operation: IssueWriteOperation::Body,
                },
                issue,
                ..body()
            }
        };

        for request in [
            // 보드 필드 승인, 다른 이슈의 승인, 다른 서버, 올바르지 않은 node ID.
            request(IssueWriteOperation::BoardField),
            other_issue,
            with_issue("github.example", "I_kwDO-1"),
            with_issue("github.com", "I_1 -f query=x"),
        ] {
            assert!(adapter.update_title_and_body(request).is_err());
        }
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_failed_or_unconfirmed_update_is_an_error() {
        let (adapter, _) = adapter_with(vec![
            Err(anyhow!("gh could not read the issue")),
            Ok(r#"{"data":{"updateIssue":null}}"#.to_string()),
            Ok(r#"{"errors":[{"message":"x"}]}"#.to_string()),
            Ok("not json".to_string()),
            // 답이 다른 이슈, 다른 저장소이다.
            update_answer("I_kwDO-2", "R_kgDO-1"),
            update_answer("I_kwDO-1", "R_kgDO-2"),
        ]);

        for _ in 0..6 {
            assert!(
                adapter
                    .update_title_and_body(request(IssueWriteOperation::Body))
                    .is_err()
            );
        }
    }
}
