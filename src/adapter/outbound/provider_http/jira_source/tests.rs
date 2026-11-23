use super::super::{
    ExpectedResponse, HttpMethod, HttpRequest, HttpResponse, HttpTransport, REQUEST_TIMEOUT,
    RESPONSE_LIMIT,
};
use super::*;
use crate::domain::provider_credentials::{
    ConnectionConfiguration, ConnectionMetadata, ConnectionRef, JiraDeployment, SecretToken,
};
use serde_json::Value;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Capture(Mutex<Vec<HttpRequest>>);
impl HttpTransport for Capture {
    fn read(
        &self,
        request: HttpRequest,
        _authorization: &str,
        _token: &SecretToken,
        _cancelled: &AtomicBool,
        _deadline: Instant,
    ) -> CredentialResult<HttpResponse> {
        self.0.lock().unwrap().push(request);
        Ok(HttpResponse::Json(json!({})))
    }
}
fn fixture() -> (ProviderHttpAdapter, Arc<Capture>, ProviderCredentialLease) {
    let http = Arc::new(Capture::default());
    let lease = ProviderCredentialLease {
        reference: ConnectionRef {
            profile_id: "a".repeat(64),
            provider: Provider::Jira,
            connection_id: "b".repeat(32),
            revision: 1,
        },
        metadata: ConnectionMetadata {
            configuration: ConnectionConfiguration {
                site_url: Some("https://jira.example.invalid".into()),
                deployment: Some(JiraDeployment::Cloud),
                email: Some("person@example.invalid".into()),
            },
            account_id: "account-1".into(),
            account_name: "Test User".into(),
        },
        token: SecretToken::from_bytes(Provider::Jira, b"synthetic-source-token".to_vec()).unwrap(),
    };
    (
        ProviderHttpAdapter {
            transport: http.clone(),
        },
        http,
        lease,
    )
}
fn invoke(
    adapter: &ProviderHttpAdapter,
    lease: &ProviderCredentialLease,
    request: JiraIssueSourceApiRequest,
) -> CredentialResult<JiraIssueSourceDocument> {
    JiraIssueSourceApiPort::read(
        adapter,
        lease,
        &request,
        &AtomicBool::new(false),
        Instant::now() + REQUEST_TIMEOUT,
    )
}

#[test]
fn typed_source_operations_only_use_same_site_fixed_cloud_routes_and_fields() {
    let (adapter, http, lease) = fixture();
    for request in [
        JiraIssueSourceApiRequest::TenantInfo,
        JiraIssueSourceApiRequest::Project {
            project_id: "10001".into(),
        },
        JiraIssueSourceApiRequest::Search {
            jql: "project = 10001 ORDER BY updated ASC, id ASC".into(),
            next_page_token: Some("opaque-provider-token".into()),
            page_size: 25,
        },
        JiraIssueSourceApiRequest::Issue {
            issue_id: "10041".into(),
        },
        JiraIssueSourceApiRequest::EditMetadata {
            issue_id: "10041".into(),
        },
    ] {
        invoke(&adapter, &lease, request).unwrap();
    }
    let calls = http.0.lock().unwrap();
    assert_eq!(calls.len(), 5);
    let paths = [
        "/_edge/tenant_info",
        "/rest/api/3/project/10001",
        "/rest/api/3/search/jql",
        "/rest/api/3/issue/10041",
        "/rest/api/3/issue/10041/editmeta",
    ];
    for (index, call) in calls.iter().enumerate() {
        assert_eq!(
            call.url,
            format!("https://jira.example.invalid{}", paths[index])
        );
        assert_eq!(
            call.method,
            if index == 2 {
                HttpMethod::Post
            } else {
                HttpMethod::Get
            }
        );
        assert_eq!(call.expected, ExpectedResponse::Json(200));
        assert_eq!(call.timeout, REQUEST_TIMEOUT);
        assert_eq!(call.limit, RESPONSE_LIMIT);
        assert!(call.send_authorization.is_none());
        assert!(call.preserve_client_errors);
    }
    let body = calls[2].body.as_ref().unwrap();
    assert_eq!(body["nextPageToken"], "opaque-provider-token");
    assert_eq!(body["maxResults"], 25);
    assert_eq!(body["fields"], json!(FIELDS));
    assert!(
        !body["fields"]
            .as_array()
            .unwrap()
            .contains(&Value::String("comment".into()))
    );
    assert_eq!(calls[3].query, vec![("fields".into(), FIELDS.join(","))]);
}

#[test]
fn source_rejects_server_urls_path_injection_and_unbounded_pages_before_http() {
    let (adapter, http, mut lease) = fixture();
    let requests = [
        JiraIssueSourceApiRequest::Issue {
            issue_id: "10041/../../myself".into(),
        },
        JiraIssueSourceApiRequest::Project {
            project_id: "10001?expand=secrets".into(),
        },
        JiraIssueSourceApiRequest::Search {
            jql: "project = 1".into(),
            next_page_token: None,
            page_size: 101,
        },
        JiraIssueSourceApiRequest::Search {
            jql: "project = 1".into(),
            next_page_token: Some("x".repeat(2049)),
            page_size: 1,
        },
    ];
    for request in requests {
        assert!(matches!(
            invoke(&adapter, &lease, request),
            Err(ProviderCredentialError::InvalidRequest)
        ));
    }
    lease.metadata.configuration.deployment = Some(JiraDeployment::Server);
    lease.metadata.configuration.email = None;
    assert!(invoke(&adapter, &lease, JiraIssueSourceApiRequest::TenantInfo).is_err());
    assert!(http.0.lock().unwrap().is_empty());
}

#[test]
fn source_request_and_document_debug_never_include_jql_or_remote_content() {
    let request = JiraIssueSourceApiRequest::Search {
        jql: "summary ~ sensitive-source-canary".into(),
        next_page_token: Some("private-page".into()),
        page_size: 1,
    };
    let document = JiraIssueSourceDocument::new(json!({"description":"private-body"}));
    assert_eq!(
        format!("{request:?}"),
        "JiraIssueSourceApiRequest(<redacted>)"
    );
    assert_eq!(
        format!("{document:?}"),
        "JiraIssueSourceDocument(<redacted>)"
    );
}
