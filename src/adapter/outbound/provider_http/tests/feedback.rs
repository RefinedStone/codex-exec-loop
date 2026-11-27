use super::*;
use crate::application::port::outbound::jira_issue_feedback_api_port::*;
use crate::application::port::outbound::provider_credential_mutation_port::ProviderMutationSendGuardPort;
use crate::application::port::outbound::provider_credentials_port::ProviderCredentialLease;

struct Authorization {
    live: AtomicBool,
    used: AtomicBool,
}
struct Guard;
impl ProviderMutationSendGuardPort for Guard {}
impl ProviderMutationAuthorizationPort for Authorization {
    fn begin_send(&self) -> CredentialResult<Box<dyn ProviderMutationSendGuardPort>> {
        if self.used.swap(true, Ordering::SeqCst) || !self.live.load(Ordering::SeqCst) {
            return Err(ProviderCredentialError::Conflict);
        }
        Ok(Box::new(Guard))
    }
}
fn authorization(live: bool) -> Arc<Authorization> {
    Arc::new(Authorization {
        live: AtomicBool::new(live),
        used: AtomicBool::new(false),
    })
}
fn lease(token: SecretToken) -> ProviderCredentialLease {
    ProviderCredentialLease {
        reference: ConnectionRef {
            profile_id: "a".repeat(64),
            provider: Provider::Jira,
            connection_id: "b".repeat(32),
            revision: 1,
        },
        metadata: metadata(JiraDeployment::Cloud),
        token,
    }
}
fn body() -> Value {
    json!({"version":1,"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Shared status"}]}]})
}

#[test]
fn feedback_wire_uses_post_201_put_200_and_post_empty_204_without_retry() {
    let (adapter, http, token) = fixture();
    let lease = lease(token);
    let property = json!({"version":1,"intent_id":"intent","body_sha256":"a".repeat(64),"content_sha256":"b".repeat(64)});
    let requests = [
        JiraIssueFeedbackWriteRequest::CreateComment {
            issue_id: "100".into(),
            body: body(),
            property: property.clone(),
        },
        JiraIssueFeedbackWriteRequest::UpdateComment {
            issue_id: "100".into(),
            comment_id: "1000".into(),
            body: body(),
            property,
        },
        JiraIssueFeedbackWriteRequest::Transition {
            issue_id: "100".into(),
            transition_id: "31".into(),
        },
    ];
    for (index, request) in requests.iter().enumerate() {
        http.answer(if index == 2 {
            Value::Null
        } else {
            json!({"id":"1000"})
        });
        let response = JiraIssueFeedbackApiPort::write(
            &adapter,
            &lease,
            request,
            authorization(true),
            &AtomicBool::new(false),
            Instant::now() + REQUEST_TIMEOUT,
        )
        .unwrap();
        assert_eq!(
            matches!(response, JiraIssueFeedbackWriteResponse::Transitioned),
            index == 2
        );
    }
    let calls = http.calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    for (index, (method, expected, path)) in [
        (
            HttpMethod::Post,
            ExpectedResponse::Json(201),
            "issue/100/comment",
        ),
        (
            HttpMethod::Put,
            ExpectedResponse::Json(200),
            "issue/100/comment/1000",
        ),
        (
            HttpMethod::Post,
            ExpectedResponse::NoContent,
            "issue/100/transitions",
        ),
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(calls[index].method, *method);
        assert_eq!(calls[index].expected, *expected);
        assert_eq!(
            calls[index].url,
            format!("https://jira.example:8443/jira/rest/api/3/{path}")
        );
        assert_eq!(calls[index].timeout, REQUEST_TIMEOUT);
    }
    assert_eq!(
        calls[0].body.as_ref().unwrap()["properties"][0]["key"],
        "akra.issue-feedback.v1"
    );
    assert_eq!(calls[2].body, Some(json!({"transition":{"id":"31"}})));
}

#[test]
fn denied_reused_or_invalid_mutation_never_reaches_the_transport_effect() {
    let (adapter, http, token) = fixture();
    let lease = lease(token);
    let request = JiraIssueFeedbackWriteRequest::Transition {
        issue_id: "100".into(),
        transition_id: "31".into(),
    };
    assert!(
        JiraIssueFeedbackApiPort::write(
            &adapter,
            &lease,
            &request,
            authorization(false),
            &AtomicBool::new(false),
            Instant::now() + REQUEST_TIMEOUT
        )
        .is_err()
    );
    assert!(http.calls.lock().unwrap().is_empty());
    let once = authorization(true);
    http.answer(Value::Null);
    JiraIssueFeedbackApiPort::write(
        &adapter,
        &lease,
        &request,
        once.clone(),
        &AtomicBool::new(false),
        Instant::now() + REQUEST_TIMEOUT,
    )
    .unwrap();
    assert!(
        JiraIssueFeedbackApiPort::write(
            &adapter,
            &lease,
            &request,
            once,
            &AtomicBool::new(false),
            Instant::now() + REQUEST_TIMEOUT
        )
        .is_err()
    );
    let invalid = JiraIssueFeedbackWriteRequest::Transition {
        issue_id: "100/../myself".into(),
        transition_id: "31".into(),
    };
    assert!(
        JiraIssueFeedbackApiPort::write(
            &adapter,
            &lease,
            &invalid,
            authorization(true),
            &AtomicBool::new(false),
            Instant::now() + REQUEST_TIMEOUT
        )
        .is_err()
    );
    assert_eq!(http.calls.lock().unwrap().len(), 1);
}

#[test]
fn no_content_is_not_a_json_null_or_an_arbitrary_success_status() {
    let (_, _, token) = fixture();
    assert!(matches!(
        parse_expected_response(b"", &token, ExpectedResponse::NoContent, RESPONSE_LIMIT).unwrap(),
        HttpResponse::NoContent
    ));
    for body in [b"null".as_slice(), b"{}".as_slice(), b" ".as_slice()] {
        assert!(
            parse_expected_response(body, &token, ExpectedResponse::NoContent, RESPONSE_LIMIT)
                .is_err()
        );
    }
    assert!(
        parse_expected_response(b"", &token, ExpectedResponse::Json(200), RESPONSE_LIMIT).is_err()
    );
    assert!(check_response_status(200, ExpectedResponse::NoContent, true).is_err());
    assert!(check_response_status(204, ExpectedResponse::Json(201), true).is_err());
    assert_eq!(
        check_response_status(404, ExpectedResponse::Json(200), true),
        Err(ProviderCredentialError::NotFound)
    );
    assert_eq!(
        check_response_status(400, ExpectedResponse::Json(200), true),
        Err(ProviderCredentialError::InvalidRequest)
    );
    assert_eq!(
        check_response_status(400, ExpectedResponse::Json(200), false),
        Err(ProviderCredentialError::Unavailable)
    );
    assert_eq!(
        check_response_status(429, ExpectedResponse::NoContent, true),
        Err(ProviderCredentialError::Unavailable)
    );
}

/// A fake HTTP response crosses exactly the same status gate as native transport.
struct StatusHttp(u16);
impl HttpTransport for StatusHttp {
    fn read(
        &self,
        request: HttpRequest,
        _: &str,
        _: &SecretToken,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<HttpResponse> {
        ensure_live(cancelled, deadline)?;
        let _guard = request
            .send_authorization
            .as_ref()
            .map(|a| a.begin_send())
            .transpose()?;
        if let Some(reason) = jira_feedback_rejection(
            self.0,
            request.report_jira_feedback_rejection,
            request.send_authorization.is_some(),
        ) {
            return Ok(HttpResponse::JiraRejected(reason));
        }
        check_response_status(self.0, request.expected, request.preserve_client_errors)?;
        Ok(HttpResponse::NoContent)
    }
}
#[test]
fn jira_feedback_http_status_preserves_only_definitive_authorized_rejections() {
    let (_, _, token) = fixture();
    let lease = lease(token);
    let request = JiraIssueFeedbackWriteRequest::Transition {
        issue_id: "100".into(),
        transition_id: "31".into(),
    };
    for (status, reason) in [
        (400, JiraFeedbackRejection::BadRequest),
        (401, JiraFeedbackRejection::Unauthorized),
        (403, JiraFeedbackRejection::Forbidden),
        (404, JiraFeedbackRejection::NotFound),
    ] {
        let adapter = ProviderHttpAdapter {
            transport: Arc::new(StatusHttp(status)),
        };
        let result = JiraIssueFeedbackApiPort::write(
            &adapter,
            &lease,
            &request,
            authorization(true),
            &AtomicBool::new(false),
            Instant::now() + REQUEST_TIMEOUT,
        )
        .unwrap();
        assert!(
            matches!(result, JiraIssueFeedbackWriteResponse::Rejected(actual) if actual == reason)
        );
        assert!(jira_feedback_rejection(status, false, true).is_none());
        assert!(jira_feedback_rejection(status, true, false).is_none());
    }
    for status in [408, 409, 429, 500, 502, 503, 504] {
        let adapter = ProviderHttpAdapter {
            transport: Arc::new(StatusHttp(status)),
        };
        assert!(
            JiraIssueFeedbackApiPort::write(
                &adapter,
                &lease,
                &request,
                authorization(true),
                &AtomicBool::new(false),
                Instant::now() + REQUEST_TIMEOUT
            )
            .is_err()
        );
    }
}
