//! Jira Cloud feedback uses stable cloud/project/issue IDs and one journal-owned send.
//! Reconciliation reads only; a timeout never grants a retry.
use std::sync::{Arc, atomic::AtomicBool};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use serde_json::Value;

use crate::application::port::outbound::issue_feedback_port::*;
use crate::application::port::outbound::jira_issue_feedback_api_port::*;
use crate::application::port::outbound::provider_credential_mutation_port::{
    ProviderCredentialMutationPort, ProviderMutationAuthorizationPort,
    ProviderMutationSendGuardPort,
};
use crate::application::port::outbound::provider_credentials_port::ProviderCredentialLease;
use crate::domain::issue_automation::feedback::*;
use crate::domain::issue_automation::policy::{
    IssueAutomationJiraBinding, IssueAutomationJiraCloseTransition,
};
use crate::domain::issue_work::{IssueProvider, IssueReference};
use crate::domain::provider_credentials::{
    ConnectionRef, CredentialResult, JiraDeployment, Provider, ProviderCredentialError,
};

mod comments;
#[cfg(test)]
mod tests;
mod transitions;

type BindingReader = dyn Fn() -> Result<IssueAutomationJiraBinding> + Send + Sync;
type SendPolicyValidator = dyn Fn(&IssueFeedbackIntent) -> Result<()> + Send + Sync;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_PAGES: usize = 100;
const MAX_ITEMS: usize = 10_000;

pub struct JiraIssueFeedbackAdapter {
    credentials: Arc<dyn ProviderCredentialMutationPort>,
    api: Arc<dyn JiraIssueFeedbackApiPort>,
    binding: Arc<BindingReader>,
    validate_send_policy: Arc<SendPolicyValidator>,
}

struct FeedbackSendAuthorization {
    credential: Arc<dyn ProviderMutationAuthorizationPort>,
    validate_policy: Arc<SendPolicyValidator>,
    intent: IssueFeedbackIntent,
}
impl ProviderMutationAuthorizationPort for FeedbackSendAuthorization {
    fn begin_send(&self) -> CredentialResult<Box<dyn ProviderMutationSendGuardPort>> {
        let send = self.credential.begin_send()?;
        // Policy may change during remote preflight or transport preparation. Read it again
        // at the sole HTTP send boundary, while retaining the one-shot credential admission.
        (self.validate_policy)(&self.intent).map_err(|_| ProviderCredentialError::Conflict)?;
        Ok(send)
    }
}
struct FeedbackContext {
    binding: IssueAutomationJiraBinding,
    lease: ProviderCredentialLease,
    deadline: Instant,
}
#[derive(Clone, PartialEq, Eq)]
struct IssueStatus {
    id: String,
    done: bool,
}

impl JiraIssueFeedbackAdapter {
    pub fn new(
        credentials: Arc<dyn ProviderCredentialMutationPort>,
        api: Arc<dyn JiraIssueFeedbackApiPort>,
        binding: Arc<BindingReader>,
        validate_send_policy: Arc<SendPolicyValidator>,
    ) -> Self {
        Self {
            credentials,
            api,
            binding,
            validate_send_policy,
        }
    }

    fn context(
        &self,
        issue: &IssueReference,
        expected: Option<&IssueFeedbackConnection>,
    ) -> Result<FeedbackContext> {
        validate_feedback_issue(issue).map_err(anyhow::Error::msg)?;
        if issue.provider != IssueProvider::Jira {
            bail!("unsupported Jira feedback provider");
        }
        let binding =
            (self.binding)().map_err(|_| anyhow!("Jira feedback binding is unavailable"))?;
        binding
            .validate()
            .map_err(|_| anyhow!("invalid Jira feedback binding"))?;
        if issue.server_id != binding.cloud_id || issue.repository_id != binding.project_id {
            bail!("Jira feedback binding changed");
        }
        let reference = if let Some(expected) = expected {
            expected.validate().map_err(anyhow::Error::msg)?;
            ConnectionRef {
                profile_id: expected.profile_id.clone(),
                provider: Provider::Jira,
                connection_id: expected.connection_id.clone(),
                revision: expected.revision,
            }
        } else {
            self.credentials
                .current_state(Provider::Jira)?
                .active_connection
                .ok_or_else(|| anyhow!("Jira feedback connection is unavailable"))?
                .reference
        };
        let lease = self.credentials.acquire(&reference)?;
        if lease.reference != reference
            || lease.metadata.configuration.deployment != Some(JiraDeployment::Cloud)
            || lease.metadata.configuration.site_url.as_deref() != Some(binding.site_url.as_str())
            || lease.metadata.account_id != binding.expected_account_id
        {
            bail!("Jira feedback account or site changed");
        }
        let context = FeedbackContext {
            binding,
            lease,
            deadline: Instant::now() + OPERATION_TIMEOUT,
        };
        let tenant = self.read(&context, JiraIssueFeedbackReadRequest::TenantInfo)?;
        if tenant.get("cloudId").and_then(Value::as_str) != Some(issue.server_id.as_str()) {
            bail!("Jira feedback cloud changed");
        }
        let actor = self.read(&context, JiraIssueFeedbackReadRequest::Actor)?;
        if actor.get("accountId").and_then(Value::as_str)
            != Some(context.binding.expected_account_id.as_str())
            || actor.get("active").and_then(Value::as_bool) == Some(false)
        {
            bail!("Jira feedback account changed");
        }
        self.issue_status(&context, issue)?;
        Ok(context)
    }

    fn read(
        &self,
        context: &FeedbackContext,
        request: JiraIssueFeedbackReadRequest,
    ) -> Result<Value> {
        self.credentials
            .validate_current(&context.lease.reference)?;
        let value = self
            .api
            .read(
                &context.lease,
                &request,
                &AtomicBool::new(false),
                context.deadline,
            )?
            .into_value();
        self.credentials
            .validate_current(&context.lease.reference)?;
        Ok(value)
    }

    fn issue_status(
        &self,
        context: &FeedbackContext,
        issue: &IssueReference,
    ) -> Result<IssueStatus> {
        let value = self.read(
            context,
            JiraIssueFeedbackReadRequest::Issue {
                issue_id: issue.issue_id.clone(),
            },
        )?;
        if value.get("id").and_then(Value::as_str) != Some(issue.issue_id.as_str())
            || value.pointer("/fields/project/id").and_then(Value::as_str)
                != Some(issue.repository_id.as_str())
        {
            bail!("Jira returned a different feedback issue");
        }
        let status = value
            .pointer("/fields/status/id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("Jira status identity is missing"))?;
        validate_jira_feedback_id(status).map_err(anyhow::Error::msg)?;
        let category = value
            .pointer("/fields/status/statusCategory/key")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("Jira status category is missing"))?;
        if !matches!(category, "new" | "indeterminate" | "done") {
            bail!("unsupported Jira status category");
        }
        Ok(IssueStatus {
            id: status.to_owned(),
            done: category == "done",
        })
    }

    fn require_intent_actor(context: &FeedbackContext, intent: &IssueFeedbackIntent) -> Result<()> {
        if context.binding.expected_account_id != intent.actor_id {
            bail!("Jira feedback owner changed");
        }
        Ok(())
    }

    fn send(
        &self,
        context: &FeedbackContext,
        intent: &IssueFeedbackIntent,
        request: JiraIssueFeedbackWriteRequest,
    ) -> Result<JiraIssueFeedbackWriteResponse> {
        let authorization = Arc::new(FeedbackSendAuthorization {
            credential: self
                .credentials
                .authorize_mutation(&context.lease.reference)?,
            validate_policy: self.validate_send_policy.clone(),
            intent: intent.clone(),
        });
        Ok(self.api.write(
            &context.lease,
            &request,
            authorization,
            &AtomicBool::new(false),
            context.deadline,
        )?)
    }

    fn confirmed(
        intent: &IssueFeedbackIntent,
        evidence: IssueFeedbackEvidence,
    ) -> Result<IssueFeedbackReadResult> {
        let receipt = IssueFeedbackReceipt {
            issue: intent.issue.clone(),
            intent_id: intent.intent_id.clone(),
            actor_id: intent.actor_id.clone(),
            evidence,
        };
        receipt.validate_for(intent).map_err(anyhow::Error::msg)?;
        Ok(IssueFeedbackReadResult::Confirmed(Box::new(receipt)))
    }
}

impl IssueFeedbackPort for JiraIssueFeedbackAdapter {
    fn actor_id(&self, issue: &IssueReference) -> Result<String> {
        Ok(self.actor_context(issue)?.actor_id)
    }

    fn actor_context(&self, issue: &IssueReference) -> Result<IssueFeedbackActorContext> {
        let context = self.context(issue, None)?;
        Ok(IssueFeedbackActorContext {
            actor_id: context.lease.metadata.account_id,
            connection: Some(IssueFeedbackConnection {
                profile_id: context.lease.reference.profile_id,
                connection_id: context.lease.reference.connection_id,
                revision: context.lease.reference.revision,
            }),
        })
    }

    fn read_label(
        &self,
        _issue: &IssueReference,
        _name: &str,
    ) -> Result<IssueFeedbackLabelSnapshot> {
        bail!("Jira progress labels are unsupported")
    }

    fn read_jira_close_transition(
        &self,
        issue: &IssueReference,
        actor: &IssueFeedbackActorContext,
        mapping: &IssueAutomationJiraCloseTransition,
    ) -> Result<IssueFeedbackJiraTransitionSnapshot> {
        let connection = actor
            .connection
            .as_ref()
            .ok_or_else(|| anyhow!("Jira feedback connection is missing"))?;
        let context = self.context(issue, Some(connection))?;
        if actor.actor_id != context.binding.expected_account_id
            || context.binding.close_transition.as_ref() != Some(mapping)
        {
            bail!("Jira close approval context changed");
        }
        self.transition_snapshot(&context, issue, mapping)
    }

    fn write(&self, intent: &IssueFeedbackIntent) -> Result<IssueFeedbackWriteResult> {
        intent.validate().map_err(anyhow::Error::msg)?;
        let expected = intent
            .connection
            .as_ref()
            .ok_or_else(|| anyhow!("Jira feedback connection is missing"))?;
        let context = self.context(&intent.issue, Some(expected))?;
        Self::require_intent_actor(&context, intent)?;
        match &intent.operation {
            IssueFeedbackOperation::CreateComment { .. }
            | IssueFeedbackOperation::UpdateComment { .. } => self.write_comment(&context, intent),
            IssueFeedbackOperation::CloseIssue { .. } => self.write_transition(&context, intent),
            _ => bail!("unsupported Jira feedback operation"),
        }
    }

    fn reconcile(&self, intent: &IssueFeedbackIntent) -> Result<IssueFeedbackReadResult> {
        intent.validate().map_err(anyhow::Error::msg)?;
        // Reading old outcomes may use a rotated credential for the same fixed site/account.
        // It can never authorize a send of the old intent.
        let result = (|| {
            let context = self.context(&intent.issue, None)?;
            Self::require_intent_actor(&context, intent)?;
            match &intent.operation {
                IssueFeedbackOperation::CreateComment { .. }
                | IssueFeedbackOperation::UpdateComment { .. } => {
                    self.reconcile_comment(&context, intent)
                }
                IssueFeedbackOperation::CloseIssue { .. } => {
                    self.reconcile_transition(&context, intent)
                }
                _ => bail!("unsupported Jira feedback operation"),
            }
        })();
        Ok(result.unwrap_or(IssueFeedbackReadResult::NeedsConfirmation))
    }
}

fn string<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty() && text.len() <= 2048)
        .ok_or_else(|| anyhow!("Jira feedback response is incomplete"))
}
