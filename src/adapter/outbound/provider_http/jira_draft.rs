//! One fixed Jira Cloud mutation, invoked only by the journaled draft publisher. No caller-supplied URL or method.
use super::*;
use crate::application::port::outbound::provider_credentials_port::ProviderCredentialLease;

impl ProviderHttpAdapter {
    pub(crate) fn put_jira_draft(
        &self,
        lease: &ProviderCredentialLease,
        issue_id: &str,
        payload: &[u8],
        send_authorization: Arc<dyn ProviderMutationAuthorizationPort>,
    ) -> CredentialResult<()> {
        if lease.reference.provider != Provider::Jira
            || issue_id.is_empty()
            || issue_id.len() > 32
            || !issue_id.bytes().all(|b| b.is_ascii_digit())
            || payload.len() > RESPONSE_LIMIT
        {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        let config = lease.metadata.configuration.normalized(Provider::Jira)?;
        if config.deployment != Some(JiraDeployment::Cloud) {
            return Err(ProviderCredentialError::Unsupported);
        }
        let site = config
            .site_url
            .as_deref()
            .ok_or(ProviderCredentialError::InvalidRequest)?;
        let authorization = jira_authorization(&config, &lease.token)?;
        let body: Value =
            serde_json::from_slice(payload).map_err(|_| ProviderCredentialError::InvalidRequest)?;
        let fields = body
            .get("fields")
            .and_then(Value::as_object)
            .filter(|f| {
                f.len() == 2
                    && f.get("summary").is_some_and(Value::is_string)
                    && f.get("description").is_some()
            })
            .ok_or(ProviderCredentialError::InvalidRequest)?;
        let response = self.transport.read(
            HttpRequest {
                url: format!("{site}/rest/api/3/issue/{issue_id}"),
                method: HttpMethod::Put,
                expected: ExpectedResponse::NoContent,
                send_authorization: Some(send_authorization),
                preserve_client_errors: true,
                report_jira_feedback_rejection: true,
                query: vec![],
                body: Some(json!({"fields":fields})),
                limit: RESPONSE_LIMIT,
                timeout: REQUEST_TIMEOUT,
            },
            &authorization,
            &lease.token,
            &AtomicBool::new(false),
            Instant::now() + REQUEST_TIMEOUT,
        )?;
        match response {
            HttpResponse::NoContent => Ok(()),
            _ => Err(ProviderCredentialError::Corrupt),
        }
    }
}
