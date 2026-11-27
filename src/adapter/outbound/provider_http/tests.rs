use super::*;
use std::collections::VecDeque;
use std::sync::Mutex;
mod feedback;

struct Captured {
    url: String,
    method: HttpMethod,
    expected: ExpectedResponse,
    query: Vec<(String, String)>,
    body: Option<Value>,
    authorization: String,
    limit: usize,
    timeout: Duration,
}
#[derive(Default)]
struct FakeHttp {
    answers: Mutex<VecDeque<CredentialResult<Value>>>,
    calls: Mutex<Vec<Captured>>,
}
impl FakeHttp {
    fn answer(&self, value: Value) {
        self.answers.lock().unwrap().push_back(Ok(value));
    }
}
impl HttpTransport for FakeHttp {
    fn read(
        &self,
        request: HttpRequest,
        authorization: &str,
        _token: &SecretToken,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<HttpResponse> {
        ensure_live(cancelled, deadline)?;
        let _send_guard = request
            .send_authorization
            .as_ref()
            .map(|authorization| authorization.begin_send())
            .transpose()?;
        self.calls.lock().unwrap().push(Captured {
            url: request.url,
            method: request.method,
            expected: request.expected,
            query: request.query,
            body: request.body,
            authorization: authorization.to_owned(),
            limit: request.limit,
            timeout: request.timeout,
        });
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(ProviderCredentialError::Unavailable))
            .map(|value| match request.expected {
                ExpectedResponse::NoContent if value.is_null() => HttpResponse::NoContent,
                _ => HttpResponse::Json(value),
            })
    }
}
fn fixture() -> (ProviderHttpAdapter, Arc<FakeHttp>, SecretToken) {
    let http = Arc::new(FakeHttp::default());
    let adapter = ProviderHttpAdapter {
        transport: http.clone(),
    };
    (
        adapter,
        http,
        SecretToken::from_bytes(Provider::Jira, b"synthetic-provider-secret-canary".to_vec())
            .unwrap(),
    )
}
fn request(deployment: JiraDeployment) -> ConnectRequest {
    ConnectRequest {
        operation_id: "connect-test".to_owned(),
        profile_id: "a".repeat(64),
        provider: Provider::Jira,
        expected_revision: 0,
        configuration: ConnectionConfiguration {
            site_url: Some("https://jira.example:8443/jira".to_owned()),
            deployment: Some(deployment),
            email: Some("person@example.invalid".to_owned()),
        },
    }
}
fn metadata(deployment: JiraDeployment) -> ConnectionMetadata {
    ConnectionMetadata {
        configuration: request(deployment)
            .configuration
            .normalized(Provider::Jira)
            .unwrap(),
        account_id: "user-id".to_owned(),
        account_name: "User".to_owned(),
    }
}
fn issue_value() -> Value {
    json!({"id":"issue-1","identifier":"APP-42","title":"Synthetic title",
        "url":"https://linear.app/team/issue/APP-42/title","priority":2,"updatedAt":"2026-11-23T01:02:03Z",
        "branchName":"team/app-42","state":{"id":"state-1","name":"Started","type":"started"},
        "team":{"id":"team-1","key":"APP","name":"Team"},"project":{"id":"project-1","name":"Project"},
        "cycle":{"id":"cycle-1","number":1,"name":null,"isActive":true}})
}
#[test]
fn jira_cloud_and_server_verify_with_distinct_bounded_auth() {
    for deployment in [JiraDeployment::Cloud, JiraDeployment::Server] {
        let (adapter, http, token) = fixture();
        http.answer(json!({"accountId":"account-1","key":"server-1","displayName":"Person"}));
        let result = adapter
            .verify(
                &request(deployment),
                &token,
                &AtomicBool::new(false),
                Instant::now() + REQUEST_TIMEOUT,
            )
            .unwrap();
        assert_eq!(result.account_name, "Person");
        let calls = http.calls.lock().unwrap();
        let call = &calls[0];
        assert_eq!(
            call.url,
            format!(
                "https://jira.example:8443/jira/rest/api/{}/myself",
                if deployment == JiraDeployment::Cloud {
                    "3"
                } else {
                    "2"
                }
            )
        );
        assert!(call.body.is_none());
        assert_eq!(call.method, HttpMethod::Get);
        assert_eq!(call.expected, ExpectedResponse::Json(200));
        assert_eq!(call.limit, RESPONSE_LIMIT);
        match deployment {
            JiraDeployment::Cloud => {
                let encoded = call.authorization.strip_prefix("Basic ").unwrap();
                assert_eq!(
                    base64::engine::general_purpose::STANDARD
                        .decode(encoded)
                        .unwrap(),
                    b"person@example.invalid:synthetic-provider-secret-canary"
                );
            }
            JiraDeployment::Server => assert_eq!(
                call.authorization,
                "Bearer synthetic-provider-secret-canary"
            ),
        }
    }
}
#[test]
fn invalid_sites_and_cancelled_owners_never_call_http() {
    let (adapter, http, token) = fixture();
    for site in [
        "http://jira.example",
        "https://user:pass@jira.example",
        "https://jira.example/../other",
    ] {
        let mut request = request(JiraDeployment::Cloud);
        request.configuration.site_url = Some(site.to_owned());
        assert!(
            adapter
                .verify(
                    &request,
                    &token,
                    &AtomicBool::new(false),
                    Instant::now() + REQUEST_TIMEOUT
                )
                .is_err()
        );
    }
    assert!(
        adapter
            .verify(
                &request(JiraDeployment::Cloud),
                &token,
                &AtomicBool::new(true),
                Instant::now() + REQUEST_TIMEOUT
            )
            .is_err()
    );
    assert!(http.calls.lock().unwrap().is_empty());
}
#[test]
fn reflected_tokens_and_graphql_errors_are_fixed_errors() {
    let (adapter, http, token) = fixture();
    http.answer(json!({"accountId":"account-1","displayName":token.as_str()}));
    let error = adapter
        .verify(
            &request(JiraDeployment::Cloud),
            &token,
            &AtomicBool::new(false),
            Instant::now() + REQUEST_TIMEOUT,
        )
        .unwrap_err();
    assert_eq!(error, ProviderCredentialError::Corrupt);
    assert!(!error.to_string().contains(token.as_str()));
    http.answer(json!({"errors":[{"message":format!("rejected {}",token.as_str())}]}));
    assert_eq!(
        adapter.linear_assigned_issues(&token).unwrap_err(),
        ProviderCredentialError::Corrupt
    );
    let escaped = br#"{"name":"synthetic-provider-secret-\u0063anary"}"#;
    assert_eq!(
        parse_response(escaped, &token, RESPONSE_LIMIT).unwrap_err(),
        ProviderCredentialError::Corrupt
    );
    assert_eq!(
        parse_response(&[b' '; 65], &token, 64).unwrap_err(),
        ProviderCredentialError::TooLarge
    );
}
#[test]
fn jira_summary_is_capped_and_version_uses_small_fixed_request() {
    let (adapter, http, token) = fixture();
    http.answer(json!({"issues":[{"key":"APP-42","fields":{"summary":"Example","updated":"2026-11-23T01:02:03Z"}}]}));
    assert_eq!(
        adapter
            .jira_summary(&metadata(JiraDeployment::Cloud), &token)
            .unwrap()
            .issues[0]
            .key,
        "APP-42"
    );
    http.answer(json!({"fields":{"updated":"2026-11-23T10:02:03.000+0900"}}));
    assert_eq!(
        adapter
            .jira_current_version(&metadata(JiraDeployment::Cloud), &token, "APP-42")
            .unwrap(),
        "2026-11-23T01:02:03.000Z"
    );
    let calls = http.calls.lock().unwrap();
    assert_eq!(
        calls[0].url,
        "https://jira.example:8443/jira/rest/api/3/search/jql"
    );
    assert!(
        calls[0]
            .query
            .contains(&("maxResults".to_owned(), "30".to_owned()))
    );
    assert!(
        calls[0]
            .query
            .iter()
            .any(|(key, value)| key == "jql" && value.starts_with("assignee = currentUser()"))
    );
    assert_eq!(
        calls[1].url,
        "https://jira.example:8443/jira/rest/api/3/issue/APP-42"
    );
    assert_eq!(
        calls[1].query,
        vec![("fields".to_owned(), "updated".to_owned())]
    );
    assert_eq!(calls[1].limit, VERSION_RESPONSE_LIMIT);
    assert_eq!(calls[1].timeout, VERSION_TIMEOUT);
    drop(calls);
    assert_eq!(
        jira::summary(
            &json!({"issues":vec![json!({"key":"APP-42","fields":{}});31]}),
            "https://jira.example"
        )
        .unwrap_err(),
        ProviderCredentialError::TooLarge
    );
    assert!(
        adapter
            .jira_issue_prompt(&metadata(JiraDeployment::Cloud), &token, "../myself")
            .is_err()
    );
}
#[test]
fn linear_resource_version_checks_fixed_query_and_workspace_identity() {
    let (adapter, http, _) = fixture();
    let token = SecretToken::from_bytes(Provider::Linear, b"synthetic-linear-version-key".to_vec())
        .unwrap();
    for (url, identifier, updated) in [
        (
            "https://linear.app/team/issue/APP-42/title",
            "APP-42",
            "2026-11-23T01:02:03.123456Z",
        ),
        (
            "https://linear.app/other/issue/APP-42/title",
            "APP-42",
            "2026-11-23T01:02:03Z",
        ),
        (
            "https://linear.app/team/issue/APP-42/title",
            "APP-43",
            "2026-11-23T01:02:03Z",
        ),
        (
            "https://linear.app/team/issue/APP-42/title",
            "APP-42",
            "bad-timestamp",
        ),
    ] {
        http.answer(
            json!({"data":{"issue":{"url":url,"identifier":identifier,"updatedAt":updated}}}),
        );
    }
    assert_eq!(
        adapter
            .linear_current_version(&token, "team/APP-42")
            .unwrap(),
        "2026-11-23T01:02:03.123456Z"
    );
    for _ in 0..3 {
        assert_eq!(
            adapter
                .linear_current_version(&token, "team/APP-42")
                .unwrap_err(),
            ProviderCredentialError::Corrupt
        );
    }
    assert!(
        adapter
            .linear_current_version(&token, "team/APP-42?bad")
            .is_err()
    );
    let calls = http.calls.lock().unwrap();
    assert_eq!(calls.len(), 4);
    for call in calls.iter() {
        assert_eq!(call.url, LINEAR_ENDPOINT);
        assert_eq!(
            call.body,
            Some(json!({"query":linear::VERSION_QUERY,"variables":{"id":"APP-42"}}))
        );
        assert_eq!(call.limit, VERSION_RESPONSE_LIMIT);
        assert_eq!(call.timeout, VERSION_TIMEOUT);
    }
}

#[test]
fn linear_query_is_fixed_capped_and_filters_terminal_issues() {
    let (adapter, http, token) = fixture();
    let mut done = issue_value();
    done["id"] = json!("issue-2");
    done["state"]["type"] = json!("completed");
    http.answer(json!({"data":{"viewer":{"assignedIssues":{"nodes":[issue_value(),done]}}}}));
    let result = adapter.linear_assigned_issues(&token).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].identifier, "APP-42");
    let calls = http.calls.lock().unwrap();
    assert_eq!(calls[0].url, LINEAR_ENDPOINT);
    assert_eq!(calls[0].authorization, token.as_str());
    assert_eq!(
        calls[0].body.as_ref().unwrap()["query"],
        linear::ISSUES_QUERY
    );
    assert_eq!(calls[0].body.as_ref().unwrap()["variables"], json!({}));
    drop(calls);
    assert!(
        linear::assigned_issues(
            &json!({"viewer":{"assignedIssues":{"nodes":[issue_value(),issue_value()]}}})
        )
        .is_err()
    );
    assert!(
        linear::assigned_issues(
            &json!({"viewer":{"assignedIssues":{"nodes":vec![issue_value();31]}}})
        )
        .is_err()
    );
}
#[test]
fn detail_must_match_requested_identity_and_only_returns_a_bounded_draft() {
    let (adapter, http, token) = fixture();
    let mut issue = issue_value();
    issue["description"] = json!("한".repeat(30_000));
    issue["comments"] = json!({"nodes":[{"body":"Body","createdAt":"2026-11-23T01:02:03Z","user":{"name":"Author"}}],"pageInfo":{"hasNextPage":true}});
    http.answer(json!({"data":{"issue":issue}}));
    let text = adapter.linear_issue_prompt(&token, "issue-1").unwrap();
    assert!(text.contains("Work on this Linear issue."));
    assert!(text.contains("Create the working branch with this suggested name."));
    assert!(text.contains("[Older comments omitted]"));
    assert!(text.chars().count() <= MAX_PROMPT_CHARS);
    http.answer(json!({"data":{"issue":issue_value()}}));
    assert!(adapter.linear_issue_prompt(&token, "different-id").is_err());
    let text=jira::prompt(&json!({"key":"APP-42","fields":{"summary":"Test",
        "description":{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Description"}]}]},
        "comment":{"comments":[{"author":{"displayName":"Person"},"body":"Comment"}],"total":20}}}),"https://jira.example").unwrap();
    assert!(text.contains("Description"));
    assert!(text.contains("댓글 20개 중 최근 1개"));
    assert!(text.chars().count() <= MAX_PROMPT_CHARS);
}

#[test]
fn missing_linear_viewer_fields_return_a_typed_error_without_panicking() {
    for viewer in [
        json!({}),
        json!({"id":"account"}),
        json!({"name":"Name"}),
        json!({"id":{},"name":null}),
    ] {
        let (adapter, http, token) = fixture();
        http.answer(json!({"data":{"viewer":viewer}}));
        let mut request = request(JiraDeployment::Cloud);
        request.provider = Provider::Linear;
        request.configuration = ConnectionConfiguration::default();
        assert_eq!(
            adapter
                .verify(
                    &request,
                    &token,
                    &AtomicBool::new(false),
                    Instant::now() + REQUEST_TIMEOUT
                )
                .unwrap_err(),
            ProviderCredentialError::Corrupt
        );
    }
}

#[test]
fn reflected_basic_payload_is_not_public_account_metadata() {
    let (adapter, http, token) = fixture();
    let configuration = request(JiraDeployment::Cloud).configuration;
    let authorization = jira_authorization(&configuration, &token).unwrap();
    let encoded = authorization.strip_prefix("Basic ").unwrap();
    http.answer(json!({"accountId":"account-1","displayName":encoded}));
    assert_eq!(
        adapter
            .verify(
                &request(JiraDeployment::Cloud),
                &token,
                &AtomicBool::new(false),
                Instant::now() + REQUEST_TIMEOUT
            )
            .unwrap_err(),
        ProviderCredentialError::Corrupt
    );
}
