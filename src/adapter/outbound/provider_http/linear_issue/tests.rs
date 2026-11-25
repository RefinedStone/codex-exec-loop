use super::super::{HttpRequest, HttpResponse, HttpTransport, LINEAR_ENDPOINT, REQUEST_TIMEOUT};
use super::*;
use crate::application::port::outbound::provider_credential_mutation_port::ProviderMutationSendGuardPort;
use crate::domain::provider_credentials::{
    ConnectionConfiguration, ConnectionMetadata, ConnectionRef, SecretToken,
};
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;

const TEAM: &str = "1f6d7f0a-0000-4000-8000-000000000001";
const ISSUE: &str = "1f6d7f0a-0000-4000-8000-000000000002";

struct Captured {
    url: String,
    method: HttpMethod,
    body: Value,
    authorization: String,
    mutation: bool,
}
#[derive(Default)]
struct Capture {
    calls: Mutex<Vec<Captured>>,
    answer: Mutex<Option<Value>>,
}
impl HttpTransport for Capture {
    fn read(
        &self,
        request: HttpRequest,
        authorization: &str,
        _token: &SecretToken,
        _cancelled: &AtomicBool,
        _deadline: Instant,
    ) -> CredentialResult<HttpResponse> {
        let _guard = request
            .send_authorization
            .as_ref()
            .map(|authorization| authorization.begin_send())
            .transpose()?;
        self.calls.lock().unwrap().push(Captured {
            url: request.url,
            method: request.method,
            body: request.body.unwrap_or(Value::Null),
            authorization: authorization.to_owned(),
            mutation: request.send_authorization.is_some(),
        });
        Ok(HttpResponse::Json(
            self.answer
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(json!({"data": {}})),
        ))
    }
}
struct Guard;
impl ProviderMutationSendGuardPort for Guard {}
#[derive(Default)]
struct OneShot(AtomicUsize);
impl ProviderMutationAuthorizationPort for OneShot {
    fn begin_send(&self) -> CredentialResult<Box<dyn ProviderMutationSendGuardPort>> {
        if self.0.fetch_add(1, Ordering::SeqCst) != 0 {
            return Err(ProviderCredentialError::Conflict);
        }
        Ok(Box::new(Guard))
    }
}

fn fixture() -> (ProviderHttpAdapter, Arc<Capture>, ProviderCredentialLease) {
    let http = Arc::new(Capture::default());
    let lease = ProviderCredentialLease {
        reference: ConnectionRef {
            profile_id: "a".repeat(64),
            provider: Provider::Linear,
            connection_id: "b".repeat(32),
            revision: 1,
        },
        metadata: ConnectionMetadata {
            configuration: ConnectionConfiguration::default(),
            account_id: "viewer-1".into(),
            account_name: "Test User".into(),
        },
        token: SecretToken::from_bytes(Provider::Linear, b"lin_api_synthetic_token".to_vec())
            .unwrap(),
    };
    (
        ProviderHttpAdapter {
            transport: http.clone(),
        },
        http,
        lease,
    )
}

fn read(
    adapter: &ProviderHttpAdapter,
    lease: &ProviderCredentialLease,
    request: LinearIssueReadRequest,
) -> CredentialResult<LinearIssueDocument> {
    LinearIssueApiPort::read(
        adapter,
        lease,
        &request,
        &AtomicBool::new(false),
        Instant::now() + REQUEST_TIMEOUT,
    )
}

#[test]
fn reads_use_fixed_documents_and_typed_variables_on_the_linear_endpoint() {
    let (adapter, http, lease) = fixture();
    read(
        &adapter,
        &lease,
        LinearIssueReadRequest::Issues {
            team_id: TEAM.into(),
            updated_from: "2026-11-01T00:00:00.000Z".into(),
            updated_before: "2026-11-02T00:00:00.000Z".into(),
            after: Some("opaque-cursor".into()),
            page_size: 25,
        },
    )
    .unwrap();
    read(
        &adapter,
        &lease,
        LinearIssueReadRequest::Issue {
            issue_id: ISSUE.into(),
        },
    )
    .unwrap();
    let calls = http.calls.lock().unwrap();
    assert!(calls.iter().all(|call| call.url == LINEAR_ENDPOINT
        && call.method == HttpMethod::Post
        && call.authorization == "lin_api_synthetic_token"
        && !call.mutation));
    let search = &calls[0].body;
    assert!(
        search["query"]
            .as_str()
            .unwrap()
            .contains("orderBy: updatedAt")
    );
    assert_eq!(
        search["variables"]["filter"],
        json!({
            "team": {"id": {"eq": TEAM}},
            "updatedAt": {"gte": "2026-11-01T00:00:00.000Z", "lt": "2026-11-02T00:00:00.000Z"},
            "state": {"type": {"nin": ["completed", "canceled", "duplicate"]}},
        })
    );
    assert_eq!(search["variables"]["first"], 25);
    assert_eq!(search["variables"]["after"], "opaque-cursor");
    assert!(
        calls[1].body["query"]
            .as_str()
            .unwrap()
            .contains("branchName")
    );
    assert_eq!(calls[1].body["variables"], json!({"id": ISSUE}));
}

#[test]
fn malformed_values_and_other_providers_are_rejected_before_a_request() {
    let (adapter, http, mut lease) = fixture();
    for request in [
        LinearIssueReadRequest::Issue {
            issue_id: "../viewer".into(),
        },
        LinearIssueReadRequest::Issues {
            team_id: TEAM.into(),
            updated_from: "yesterday".into(),
            updated_before: "2026-11-02T00:00:00Z".into(),
            after: None,
            page_size: 25,
        },
        LinearIssueReadRequest::Issues {
            team_id: TEAM.into(),
            updated_from: "2026-11-01T00:00:00Z".into(),
            updated_before: "2026-11-02T00:00:00Z".into(),
            after: None,
            page_size: 101,
        },
        LinearIssueReadRequest::Comments {
            issue_id: ISSUE.into(),
            after: Some("line\nbreak".into()),
        },
    ] {
        assert_eq!(
            read(&adapter, &lease, request).unwrap_err(),
            ProviderCredentialError::InvalidRequest
        );
    }
    lease.reference.provider = Provider::Jira;
    assert_eq!(
        read(&adapter, &lease, LinearIssueReadRequest::Viewer).unwrap_err(),
        ProviderCredentialError::InvalidRequest
    );
    assert!(http.calls.lock().unwrap().is_empty());
}

#[test]
fn a_mutation_carries_its_one_shot_send_admission() {
    let (adapter, http, lease) = fixture();
    let authorization = Arc::new(OneShot::default());
    let request = LinearIssueWriteRequest::UpdateState {
        issue_id: ISSUE.into(),
        state_id: TEAM.into(),
    };
    LinearIssueApiPort::write(
        &adapter,
        &lease,
        &request,
        authorization.clone(),
        &AtomicBool::new(false),
        Instant::now() + REQUEST_TIMEOUT,
    )
    .unwrap();
    assert_eq!(
        LinearIssueApiPort::write(
            &adapter,
            &lease,
            &request,
            authorization,
            &AtomicBool::new(false),
            Instant::now() + REQUEST_TIMEOUT,
        )
        .unwrap_err(),
        ProviderCredentialError::Conflict
    );
    let calls = http.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].mutation);
    assert!(
        calls[0].body["query"]
            .as_str()
            .unwrap()
            .contains("issueUpdate")
    );
    assert_eq!(
        calls[0].body["variables"],
        json!({"id": ISSUE, "stateId": TEAM})
    );
}

#[test]
fn graphql_errors_never_become_a_document() {
    let (adapter, http, lease) = fixture();
    *http.answer.lock().unwrap() =
        Some(json!({"errors": [{"message": "RATELIMITED"}], "data": null}));
    assert_eq!(
        read(&adapter, &lease, LinearIssueReadRequest::Viewer).unwrap_err(),
        ProviderCredentialError::Unavailable
    );
}
