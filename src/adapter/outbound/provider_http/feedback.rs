use super::*;
use crate::application::port::outbound::jira_issue_feedback_api_port::*;
use crate::application::port::outbound::provider_credentials_port::ProviderCredentialLease;

const PROPERTY_KEY: &str = "akra.issue-feedback.v1";
const PAGE_SIZE: u64 = 100;

fn id(value: &str) -> CredentialResult<()> {
    crate::domain::issue_automation::feedback::validate_jira_feedback_id(value)
        .map_err(|_| ProviderCredentialError::InvalidRequest)
}
fn page(start_at: u64) -> CredentialResult<Vec<(String, String)>> {
    if start_at > 10_000 {
        return Err(ProviderCredentialError::TooLarge);
    }
    Ok(vec![
        ("startAt".into(), start_at.to_string()),
        ("maxResults".into(), PAGE_SIZE.to_string()),
    ])
}

impl JiraIssueFeedbackApiPort for ProviderHttpAdapter {
    fn read(
        &self,
        lease: &ProviderCredentialLease,
        request: &JiraIssueFeedbackReadRequest,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<JiraIssueFeedbackDocument> {
        if lease.reference.provider != Provider::Jira {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        use JiraIssueFeedbackReadRequest::*;
        let (relative, query) = match request {
            TenantInfo => {
                return self
                    .jira_cloud_read(
                        &lease.metadata,
                        &lease.token,
                        JiraCloudRead::tenant_info(),
                        cancelled,
                        deadline,
                    )
                    .map(JiraIssueFeedbackDocument::new);
            }
            Actor => ("myself".to_owned(), vec![]),
            Issue { issue_id } => {
                id(issue_id)?;
                (
                    format!("issue/{issue_id}"),
                    vec![("fields".into(), "project,status,updated".into())],
                )
            }
            Comments { issue_id, start_at } => {
                id(issue_id)?;
                let mut query = page(*start_at)?;
                query.push(("expand".into(), "properties".into()));
                (format!("issue/{issue_id}/comment"), query)
            }
            Comment {
                issue_id,
                comment_id,
            } => {
                id(issue_id)?;
                id(comment_id)?;
                (
                    format!("issue/{issue_id}/comment/{comment_id}"),
                    vec![("expand".into(), "properties".into())],
                )
            }
            CommentProperty {
                issue_id,
                comment_id,
            } => {
                id(issue_id)?;
                id(comment_id)?;
                (
                    format!("comment/{comment_id}/properties/{PROPERTY_KEY}"),
                    vec![],
                )
            }
            Transitions { issue_id } => {
                id(issue_id)?;
                (
                    format!("issue/{issue_id}/transitions"),
                    vec![("expand".into(), "transitions.fields".into())],
                )
            }
            Changelog { issue_id, start_at } => {
                id(issue_id)?;
                (format!("issue/{issue_id}/changelog"), page(*start_at)?)
            }
        };
        self.jira_cloud_read(
            &lease.metadata,
            &lease.token,
            JiraCloudRead::get_rest(&relative, query),
            cancelled,
            deadline,
        )
        .map(JiraIssueFeedbackDocument::new)
    }

    fn write(
        &self,
        lease: &ProviderCredentialLease,
        request: &JiraIssueFeedbackWriteRequest,
        send_authorization: Arc<dyn ProviderMutationAuthorizationPort>,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<JiraIssueFeedbackWriteResponse> {
        if lease.reference.provider != Provider::Jira {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        let config = lease.metadata.configuration.normalized(Provider::Jira)?;
        if config.deployment != Some(JiraDeployment::Cloud) {
            return Err(ProviderCredentialError::Unsupported);
        }
        let (relative, method, expected, body) = match request {
            JiraIssueFeedbackWriteRequest::CreateComment {
                issue_id,
                body,
                property,
            } => {
                id(issue_id)?;
                (
                    format!("issue/{issue_id}/comment"),
                    HttpMethod::Post,
                    ExpectedResponse::Json(201),
                    comment_payload(body, property)?,
                )
            }
            JiraIssueFeedbackWriteRequest::UpdateComment {
                issue_id,
                comment_id,
                body,
                property,
            } => {
                id(issue_id)?;
                id(comment_id)?;
                (
                    format!("issue/{issue_id}/comment/{comment_id}"),
                    HttpMethod::Put,
                    ExpectedResponse::Json(200),
                    comment_payload(body, property)?,
                )
            }
            JiraIssueFeedbackWriteRequest::Transition {
                issue_id,
                transition_id,
            } => {
                id(issue_id)?;
                id(transition_id)?;
                (
                    format!("issue/{issue_id}/transitions"),
                    HttpMethod::Post,
                    ExpectedResponse::NoContent,
                    json!({"transition":{"id":transition_id}}),
                )
            }
        };
        let site = config
            .site_url
            .as_deref()
            .ok_or(ProviderCredentialError::InvalidRequest)?;
        let authorization = jira_authorization(&config, &lease.token)?;
        let response = self.transport.read(
            HttpRequest {
                url: format!("{site}/rest/api/3/{relative}"),
                method,
                expected,
                send_authorization: Some(send_authorization),
                preserve_client_errors: true,
                report_jira_feedback_rejection: true,
                query: vec![],
                body: Some(body),
                limit: RESPONSE_LIMIT,
                timeout: REQUEST_TIMEOUT,
            },
            &authorization,
            &lease.token,
            cancelled,
            deadline,
        )?;
        match (expected, response) {
            (_, HttpResponse::JiraRejected(reason)) => {
                Ok(JiraIssueFeedbackWriteResponse::Rejected(reason))
            }
            (ExpectedResponse::Json(_), HttpResponse::Json(value)) => {
                reject_reflected_credentials(&value, &lease.token, &authorization)?;
                Ok(JiraIssueFeedbackWriteResponse::Comment(
                    JiraIssueFeedbackDocument::new(value),
                ))
            }
            (ExpectedResponse::NoContent, HttpResponse::NoContent) => {
                Ok(JiraIssueFeedbackWriteResponse::Transitioned)
            }
            _ => Err(ProviderCredentialError::Corrupt),
        }
    }
}

fn comment_payload(body: &Value, property: &Value) -> CredentialResult<Value> {
    if body.get("type").and_then(Value::as_str) != Some("doc")
        || body.get("version").and_then(Value::as_u64) != Some(1)
        || !body.get("content").is_some_and(Value::is_array)
        || !property.is_object()
    {
        return Err(ProviderCredentialError::InvalidRequest);
    }
    let value = json!({"body":body,"properties":[{"key":PROPERTY_KEY,"value":property}]});
    if serde_json::to_vec(&value)
        .map_err(|_| ProviderCredentialError::InvalidRequest)?
        .len()
        > 256 * 1024
    {
        return Err(ProviderCredentialError::TooLarge);
    }
    Ok(value)
}
