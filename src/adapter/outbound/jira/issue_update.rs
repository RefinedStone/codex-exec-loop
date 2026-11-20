//! Jira Cloud 이슈 한 건의 제목(`summary`)과 본문(`description`)을 REST API v3 로 고친다 (#2434). 게시 의도 기록과
//! 승인 검사는 GitHub 와 같은 `IssueJournaledPublishService` 가 한다. 이 어댑터는 그 뒤의 쓰기 한 번이다.
//!
//! 쓰기는 `PUT /rest/api/3/issue/{id}` 하나이고, 값은 `summary` 와 `description` 뿐이다. 상태 전환, 댓글, 첨부,
//! 사용자 필드는 쓰지 않는다. 두 필드는 한 요청이므로 한 필드만 바뀌는 답은 없다.
//!
//! 쓰기 전에 원격을 부르지 않고 거절하는 글: Jira 가 받지 않는 제목, 이 표현으로 보존하지 못하는 본문
//! (`adf::writable_adf`). 쓰기 전에 원격을 읽고 거절하는 경우: 다른 사이트, 옮겨진 이슈, 편집 메타데이터에
//! `summary` 나 `description` 이 없음 (권한이나 워크플로가 편집을 막는다), 지금 원격 본문에 옮기지 못한 노드나
//! 표시가 있음 (덮어쓰면 지워진다). 이 거절은 사용자 검토로 간다: 쓰기는 없었고 원격은 그대로이다.
//!
//! 남는 경쟁: Jira 의 쓰기는 조건이 없다. 위의 읽기와 `PUT` 사이에 다른 사람이 고친 글은 덮어쓴다.
//!
//! 답은 `PUT` 뒤에 이슈를 다시 읽은 값이다. `PUT` 의 답이나 그 읽기를 잃으면 오류이고, 결과를 모른다. 이 어댑터는
//! 쓰기를 다시 보내지 않는다. 아직 이 어댑터를 연결한 곳이 없다.

use anyhow::{Context, Result, bail};
use serde_json::json;

use super::adf::{adf_to_markdown, writable_adf};
use super::issue_snapshot::{EditMeta, JiraIssueSnapshotAdapter};
use crate::application::port::outbound::issue_update_port::{
    IssueUpdatePort, IssueUpdateRequest, IssueUpdateResponse,
};
use crate::domain::issue_work::IssueSharedCopy;

const MAX_SUMMARY_CHARS: usize = 255;
const WRITTEN_FIELDS: [&str; 2] = ["summary", "description"];

/// 사이트 주소 뒤의 경로와 JSON 본문을 받아 PUT 을 한 번 보낸다. 2xx 답만 `Ok` 이다. 다시 보내지 않는다: 시간
/// 초과나 끊긴 연결 뒤에 같은 요청을 자동으로 다시 보내는 전송 계층을 쓰면 안 된다. 시험은 가짜를 넣는다.
pub type JiraApiPutRunner = Box<dyn Fn(&str, &[u8]) -> Result<()> + Send + Sync>;

pub struct JiraIssueUpdateAdapter {
    reader: JiraIssueSnapshotAdapter,
    run_jira_put: JiraApiPutRunner,
}

impl JiraIssueUpdateAdapter {
    /// `reader` 와 `run_jira_put` 은 같은 사이트를 부른다.
    pub fn new(reader: JiraIssueSnapshotAdapter, run_jira_put: JiraApiPutRunner) -> Self {
        Self {
            reader,
            run_jira_put,
        }
    }
}

impl IssueUpdatePort for JiraIssueUpdateAdapter {
    fn update_title_and_body(&self, request: IssueUpdateRequest) -> Result<IssueUpdateResponse> {
        request.ensure_authorized()?;
        let issue = request.issue;
        let IssueSharedCopy { title, body } = request.shared_copy;
        if title.trim().is_empty()
            || title.chars().count() > MAX_SUMMARY_CHARS
            || title.contains(['\n', '\r'])
        {
            bail!("Jira does not accept this title as a summary: review needed")
        }
        let Some(description) = writable_adf(&body) else {
            bail!("the body cannot be preserved as a Jira description: review needed")
        };

        self.reader.ensure_this_site(&issue)?;
        let meta: EditMeta = self
            .reader
            .get(&format!("/rest/api/3/issue/{}/editmeta", issue.issue_id))?;
        if WRITTEN_FIELDS
            .iter()
            .any(|field| !meta.fields.contains_key(*field))
        {
            bail!("Jira does not allow a summary and description edit on this issue")
        }
        let current = self.reader.known_issue(&issue)?;
        if !adf_to_markdown(&current.fields.description)
            .unsupported
            .is_empty()
        {
            bail!(
                "the current Jira description has content that a write would remove: review needed"
            )
        }

        let fields = json!({ "fields": { "summary": title, "description": description } });
        (self.run_jira_put)(
            &format!("/rest/api/3/issue/{}", issue.issue_id),
            fields.to_string().as_bytes(),
        )
        .context("the issue update did not complete")?;

        // 실제로 반영된 필드를 읽는다. 공급자가 값을 다듬었으면 보낸 글과 다르다.
        let written = self.reader.known_issue(&issue)?.fields;
        Ok(IssueUpdateResponse {
            issue,
            published_copy: IssueSharedCopy {
                title: written.summary,
                body: adf_to_markdown(&written.description).markdown,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use anyhow::anyhow;
    use serde_json::Value;

    use super::*;
    use crate::domain::issue_work::{
        IssueProjectScope, IssueProvider, IssueReference, IssueWriteApproval, IssueWriteOperation,
    };

    const CLOUD_ID: &str = "11111111-2222-3333-4444-555555555555";
    const EDIT_META: &str = include_str!("fixtures/editmeta.json");
    const BODY: &str = "## Steps\n\nRun **the tool** on [[akra-mask:1]].\n\n- first\n- second";
    const TENANT_PATH: &str = "/_edge/tenant_info";
    const EDIT_META_PATH: &str = "/rest/api/3/issue/10002/editmeta";
    const ISSUE_PATH: &str =
        "/rest/api/3/issue/10002?fields=summary,description,updated,project,issuetype";

    /// `(방식, 경로, 본문)` 이다.
    type Calls = Arc<Mutex<Vec<(&'static str, String, String)>>>;

    /// Jira 의 GET 답을 순서대로 주는 가짜이다. PUT 은 `put_answer` 를 준다. 네트워크가 없다.
    fn adapter_with(
        responses: Vec<Result<String>>,
        put_fails: bool,
    ) -> (JiraIssueUpdateAdapter, Calls) {
        let calls = Calls::default();
        let (gets, puts) = (calls.clone(), calls.clone());
        let responses = Mutex::new(VecDeque::from(responses));
        let reader = JiraIssueSnapshotAdapter::new(
            "https://jira.example.invalid".to_string(),
            Box::new(move |path| {
                gets.lock()
                    .unwrap()
                    .push(("GET", path.to_string(), String::new()));
                let response = responses.lock().unwrap().pop_front();
                response
                    .expect("the fake has no more answers")
                    .map(String::into_bytes)
            }),
        );
        let adapter = JiraIssueUpdateAdapter::new(
            reader,
            Box::new(move |path, body| {
                let body = String::from_utf8(body.to_vec()).unwrap();
                puts.lock().unwrap().push(("PUT", path.to_string(), body));
                if put_fails {
                    bail!("the connection closed before the answer")
                }
                Ok(())
            }),
        );
        (adapter, calls)
    }

    fn tenant(cloud_id: &str) -> Result<String> {
        Ok(format!(r#"{{"cloudId":"{cloud_id}"}}"#))
    }

    fn edit_meta() -> Result<String> {
        Ok(EDIT_META.to_string())
    }

    /// 원격 이슈의 답이다. `description` 은 ADF 문서이다.
    fn remote_issue(summary: &str, description: Value) -> Result<String> {
        Ok(json!({
            "id": "10002",
            "key": "DEMO-7",
            "fields": {
                "summary": summary,
                "description": description,
                "updated": "2026-11-19T07:19:56.000+0000",
                "project": { "id": "10000", "key": "DEMO" }
            }
        })
        .to_string())
    }

    fn reference() -> IssueReference {
        IssueReference {
            provider: IssueProvider::Jira,
            server_id: CLOUD_ID.to_string(),
            repository_id: "10000".to_string(),
            issue_id: "10002".to_string(),
            project_scope: IssueProjectScope {
                workspace_dir: "/work/project".to_string(),
            },
        }
    }

    fn request_for(issue: IssueReference, title: &str, body: &str) -> IssueUpdateRequest {
        IssueUpdateRequest {
            approval: IssueWriteApproval {
                issue: issue.clone(),
                operation: IssueWriteOperation::Body,
            },
            issue,
            shared_copy: IssueSharedCopy {
                title: title.to_string(),
                body: body.to_string(),
            },
        }
    }

    fn request() -> IssueUpdateRequest {
        request_for(reference(), "New title", BODY)
    }

    fn put_count(calls: &Calls) -> usize {
        let calls = calls.lock().unwrap();
        calls.iter().filter(|call| call.0 == "PUT").count()
    }

    #[test]
    fn update_sends_one_put_with_only_the_summary_and_description_and_answers_with_a_reread() {
        let description = writable_adf(BODY).unwrap();
        let (adapter, calls) = adapter_with(
            vec![
                tenant(CLOUD_ID),
                edit_meta(),
                remote_issue("Old title", Value::Null),
                remote_issue("New title", description.clone()),
            ],
            false,
        );

        let response = adapter.update_title_and_body(request()).unwrap();

        assert_eq!(response.issue, reference());
        assert_eq!(response.published_copy, request().shared_copy);
        let calls = calls.lock().unwrap();
        let order: Vec<(&str, &str)> = calls
            .iter()
            .map(|(method, path, _)| (*method, path.as_str()))
            .collect();
        assert_eq!(
            order,
            [
                ("GET", TENANT_PATH),
                ("GET", EDIT_META_PATH),
                ("GET", ISSUE_PATH),
                ("PUT", "/rest/api/3/issue/10002"),
                ("GET", ISSUE_PATH),
            ]
        );
        // 보낸 값 전부이다. 상태 전환, 댓글, 다른 필드가 없다.
        assert_eq!(
            serde_json::from_str::<Value>(&calls[3].2).unwrap(),
            json!({ "fields": { "summary": "New title", "description": description } })
        );
    }

    #[test]
    fn a_wrong_approval_target_title_or_body_never_reaches_jira() {
        let (adapter, calls) = adapter_with(Vec::new(), false);
        let mut board_approval = request();
        board_approval.approval.operation = IssueWriteOperation::BoardField;
        let mut other_issue = request();
        other_issue.approval.issue.issue_id = "10003".to_string();
        let on = |issue: IssueReference| request_for(issue, "New title", BODY);

        for request in [
            board_approval,
            other_issue,
            on(IssueReference {
                provider: IssueProvider::GitHub,
                ..reference()
            }),
            on(IssueReference {
                issue_id: "10002/transitions".to_string(),
                ..reference()
            }),
            // Jira 가 받지 않는 제목이다.
            request_for(reference(), " ", BODY),
            request_for(reference(), "two\nlines", BODY),
            request_for(reference(), &"x".repeat(256), BODY),
            // 이 표현으로 보존하지 못하는 본문이다.
            request_for(
                reference(),
                "New title",
                "Ask [unsupported: mention] first.",
            ),
            request_for(reference(), "New title", "run *.rs"),
            request_for(reference(), "New title", "line\r\nline"),
        ] {
            assert!(adapter.update_title_and_body(request).is_err());
        }
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_remote_state_that_forbids_or_cannot_keep_the_edit_is_not_written() {
        let read_only_meta = r#"{"fields":{"summary":{"schema":{"type":"string"}}}}"#;
        let with_panel = json!({ "type": "doc", "version": 1, "content": [{ "type": "panel" }] });
        let with_underline = json!({ "type": "doc", "version": 1, "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "x", "marks": [{ "type": "underline" }] }]
        }] });
        let moved = remote_issue("Old title", Value::Null)
            .unwrap()
            .replace(r#""id":"10000""#, r#""id":"10009""#);
        let cases: Vec<(Vec<Result<String>>, &str)> = vec![
            // 다른 사이트로 복사한 대상이다.
            (
                vec![tenant("99999999-2222-3333-4444-555555555555")],
                "the issue is on another Jira site",
            ),
            // 권한이나 워크플로 상태가 본문 편집을 막는다.
            (
                vec![tenant(CLOUD_ID), Ok(read_only_meta.to_string())],
                "Jira does not allow a summary and description edit on this issue",
            ),
            (vec![tenant(CLOUD_ID), Err(anyhow!("403"))], "403"),
            (
                vec![tenant(CLOUD_ID), edit_meta(), Ok(moved)],
                "the target moved to another project or issue",
            ),
            // 지금 원격 본문에 옮기지 못한 노드나 표시가 있다. 덮어쓰지 않는다.
            (
                vec![
                    tenant(CLOUD_ID),
                    edit_meta(),
                    remote_issue("Old title", with_panel),
                ],
                "the current Jira description has content that a write would remove: review needed",
            ),
            (
                vec![
                    tenant(CLOUD_ID),
                    edit_meta(),
                    remote_issue("Old title", with_underline),
                ],
                "the current Jira description has content that a write would remove: review needed",
            ),
        ];

        for (responses, expected) in cases {
            let (adapter, calls) = adapter_with(responses, false);

            let error = adapter.update_title_and_body(request()).unwrap_err();

            assert_eq!(error.to_string(), expected);
            assert_eq!(put_count(&calls), 0, "{expected}");
        }
    }

    #[test]
    fn a_lost_answer_is_an_error_and_the_put_is_sent_once() {
        let before_put = || {
            vec![
                tenant(CLOUD_ID),
                edit_meta(),
                remote_issue("Old title", Value::Null),
            ]
        };
        // PUT 의 답을 잃었다.
        let (adapter, calls) = adapter_with(before_put(), true);
        assert!(adapter.update_title_and_body(request()).is_err());
        assert_eq!(put_count(&calls), 1);

        // PUT 뒤의 읽기를 잃었다.
        let mut responses = before_put();
        responses.push(Err(anyhow!("Jira could not read the issue")));
        let (adapter, calls) = adapter_with(responses, false);
        assert!(adapter.update_title_and_body(request()).is_err());
        assert_eq!(put_count(&calls), 1);
    }

    #[test]
    fn the_answer_is_what_jira_kept_and_not_what_was_sent() {
        let mut responses = vec![
            tenant(CLOUD_ID),
            edit_meta(),
            remote_issue("Old title", Value::Null),
        ];
        // 공급자가 제목의 끝 공백을 지웠다.
        responses.push(remote_issue("New title", writable_adf(BODY).unwrap()));
        let (adapter, _) = adapter_with(responses, false);

        let response = adapter
            .update_title_and_body(request_for(reference(), "New title ", BODY))
            .unwrap();

        assert_eq!(response.published_copy.title, "New title");
    }

    #[test]
    fn inline_code_with_literal_delimiters_cannot_be_silently_rewritten() {
        let description = json!({"type": "doc", "version": 1, "content": [{
            "type": "paragraph", "content": [{
                "type": "text", "text": "a\x60 b \x60c", "marks": [{"type": "code"}]
            }]
        }]});
        let body = adf_to_markdown(&description).markdown;
        let (adapter, calls) = adapter_with(
            vec![
                tenant(CLOUD_ID),
                edit_meta(),
                remote_issue("Old title", description),
            ],
            false,
        );
        assert!(
            adapter
                .update_title_and_body(request_for(reference(), "Old title", &body))
                .is_err()
        );
        assert_eq!(put_count(&calls), 0);
    }
}
