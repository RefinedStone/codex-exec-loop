//! Linear feedback uses the bound team, the stable issue ID and one journal-owned send.
//! Reconciliation reads only; a lost response never grants a retry.
use std::sync::{Arc, atomic::AtomicBool};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use serde_json::Value;

use crate::application::port::outbound::issue_feedback_port::*;
use crate::application::port::outbound::linear_issue_api_port::*;
use crate::application::port::outbound::provider_credential_mutation_port::{
    ProviderCredentialMutationPort, ProviderMutationAuthorizationPort,
    ProviderMutationSendGuardPort,
};
use crate::application::port::outbound::provider_credentials_port::ProviderCredentialLease;
use crate::domain::issue_automation::feedback::*;
use crate::domain::issue_automation::policy::{
    IssueAutomationLinearBinding, IssueAutomationLinearCloseState,
};
use crate::domain::issue_automation::source::{LINEAR_SERVER_ID, LinearIssueStateType};
use crate::domain::issue_work::{IssueProvider, IssueReference};
use crate::domain::provider_credentials::{
    ConnectionRef, CredentialResult, Provider, ProviderCredentialError,
};

mod comments;
mod states;
#[cfg(test)]
mod tests;

type BindingReader = dyn Fn() -> Result<IssueAutomationLinearBinding> + Send + Sync;
type SendPolicyValidator = dyn Fn(&IssueFeedbackIntent) -> Result<()> + Send + Sync;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_PAGES: usize = 100;

pub struct LinearIssueFeedbackAdapter {
    credentials: Arc<dyn ProviderCredentialMutationPort>,
    api: Arc<dyn LinearIssueApiPort>,
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
        // Policy may change during remote preflight. Read it again at the sole send boundary,
        // while retaining the one-shot credential admission.
        (self.validate_policy)(&self.intent).map_err(|_| ProviderCredentialError::Conflict)?;
        Ok(send)
    }
}

struct FeedbackContext {
    binding: IssueAutomationLinearBinding,
    lease: ProviderCredentialLease,
    deadline: Instant,
}

#[derive(Clone, PartialEq, Eq)]
struct IssueState {
    id: String,
    closed: bool,
}

impl LinearIssueFeedbackAdapter {
    pub fn new(
        credentials: Arc<dyn ProviderCredentialMutationPort>,
        api: Arc<dyn LinearIssueApiPort>,
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
        if issue.provider != IssueProvider::Linear {
            bail!("unsupported Linear feedback provider");
        }
        let binding =
            (self.binding)().map_err(|_| anyhow!("Linear feedback binding is unavailable"))?;
        binding
            .validate()
            .map_err(|_| anyhow!("invalid Linear feedback binding"))?;
        if issue.server_id != LINEAR_SERVER_ID || issue.repository_id != binding.team_id {
            bail!("Linear feedback binding changed");
        }
        let reference = if let Some(expected) = expected {
            expected.validate().map_err(anyhow::Error::msg)?;
            ConnectionRef {
                profile_id: expected.profile_id.clone(),
                provider: Provider::Linear,
                connection_id: expected.connection_id.clone(),
                revision: expected.revision,
            }
        } else {
            self.credentials
                .current_state(Provider::Linear)?
                .active_connection
                .ok_or_else(|| anyhow!("Linear feedback connection is unavailable"))?
                .reference
        };
        let lease = self.credentials.acquire(&reference)?;
        if lease.reference != reference || lease.metadata.account_id != binding.expected_viewer_id {
            bail!("Linear feedback account changed");
        }
        let context = FeedbackContext {
            binding,
            lease,
            deadline: Instant::now() + OPERATION_TIMEOUT,
        };
        let viewer = self.read(&context, LinearIssueReadRequest::Viewer)?;
        if viewer.pointer("/viewer/id").and_then(Value::as_str)
            != Some(context.binding.expected_viewer_id.as_str())
            || viewer.pointer("/viewer/active").and_then(Value::as_bool) != Some(true)
        {
            bail!("Linear feedback account changed");
        }
        self.issue_state(&context, issue)?;
        Ok(context)
    }

    fn read(&self, context: &FeedbackContext, request: LinearIssueReadRequest) -> Result<Value> {
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

    fn issue_state(&self, context: &FeedbackContext, issue: &IssueReference) -> Result<IssueState> {
        let value = self.read(
            context,
            LinearIssueReadRequest::Issue {
                issue_id: issue.issue_id.clone(),
            },
        )?;
        if value.pointer("/issue/id").and_then(Value::as_str) != Some(issue.issue_id.as_str())
            || value.pointer("/issue/team/id").and_then(Value::as_str)
                != Some(issue.repository_id.as_str())
        {
            bail!("Linear returned a different feedback issue");
        }
        let id = string(&value["issue"]["state"], "id")?;
        validate_linear_feedback_id(id).map_err(anyhow::Error::msg)?;
        let state_type = value
            .pointer("/issue/state/type")
            .and_then(Value::as_str)
            .and_then(LinearIssueStateType::parse)
            .ok_or_else(|| anyhow!("unsupported Linear workflow state"))?;
        Ok(IssueState {
            id: id.to_owned(),
            closed: state_type.closed(),
        })
    }

    fn require_intent_actor(context: &FeedbackContext, intent: &IssueFeedbackIntent) -> Result<()> {
        if context.binding.expected_viewer_id != intent.actor_id {
            bail!("Linear feedback owner changed");
        }
        Ok(())
    }

    fn send(
        &self,
        context: &FeedbackContext,
        intent: &IssueFeedbackIntent,
        request: LinearIssueWriteRequest,
    ) -> Result<Value> {
        let authorization = Arc::new(FeedbackSendAuthorization {
            credential: self
                .credentials
                .authorize_mutation(&context.lease.reference)?,
            validate_policy: self.validate_send_policy.clone(),
            intent: intent.clone(),
        });
        Ok(self
            .api
            .write(
                &context.lease,
                &request,
                authorization,
                &AtomicBool::new(false),
                context.deadline,
            )?
            .into_value())
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

impl IssueFeedbackPort for LinearIssueFeedbackAdapter {
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
        bail!("Linear progress labels are unsupported")
    }

    fn read_linear_close_state(
        &self,
        issue: &IssueReference,
        actor: &IssueFeedbackActorContext,
        mapping: &IssueAutomationLinearCloseState,
    ) -> Result<IssueFeedbackLinearStateSnapshot> {
        let connection = actor
            .connection
            .as_ref()
            .ok_or_else(|| anyhow!("Linear feedback connection is missing"))?;
        let context = self.context(issue, Some(connection))?;
        if actor.actor_id != context.binding.expected_viewer_id
            || context.binding.close_state.as_ref() != Some(mapping)
        {
            bail!("Linear close approval context changed");
        }
        self.state_snapshot(&context, issue, mapping)
    }

    fn write(&self, intent: &IssueFeedbackIntent) -> Result<IssueFeedbackWriteResult> {
        intent.validate().map_err(anyhow::Error::msg)?;
        let expected = intent
            .connection
            .as_ref()
            .ok_or_else(|| anyhow!("Linear feedback connection is missing"))?;
        let context = self.context(&intent.issue, Some(expected))?;
        Self::require_intent_actor(&context, intent)?;
        match &intent.operation {
            IssueFeedbackOperation::CreateComment { .. }
            | IssueFeedbackOperation::UpdateComment { .. } => self.write_comment(&context, intent),
            IssueFeedbackOperation::CloseIssue { .. } => self.write_state(&context, intent),
            _ => bail!("unsupported Linear feedback operation"),
        }
    }

    fn reconcile(&self, intent: &IssueFeedbackIntent) -> Result<IssueFeedbackReadResult> {
        intent.validate().map_err(anyhow::Error::msg)?;
        // Reading old outcomes may use a rotated credential for the same account.
        // It can never authorize a send of the old intent.
        let result = (|| {
            let context = self.context(&intent.issue, None)?;
            Self::require_intent_actor(&context, intent)?;
            match &intent.operation {
                IssueFeedbackOperation::CreateComment { .. }
                | IssueFeedbackOperation::UpdateComment { .. } => {
                    self.reconcile_comment(&context, intent)
                }
                IssueFeedbackOperation::CloseIssue { .. } => self.reconcile_state(&context, intent),
                _ => bail!("unsupported Linear feedback operation"),
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
        .ok_or_else(|| anyhow!("Linear feedback response is incomplete"))
}

/// One forward page of a Linear connection: nodes and the next `endCursor`, if any.
fn page<'a>(connection: &'a Value, after: Option<&str>) -> Result<(&'a [Value], Option<String>)> {
    let nodes = connection
        .get("nodes")
        .and_then(Value::as_array)
        .filter(|nodes| nodes.len() <= 100)
        .ok_or_else(|| anyhow!("Linear page values are missing"))?;
    let has_next = connection
        .pointer("/pageInfo/hasNextPage")
        .and_then(Value::as_bool)
        .ok_or_else(|| anyhow!("Linear page state is missing"))?;
    if !has_next {
        return Ok((nodes, None));
    }
    let next = string(&connection["pageInfo"], "endCursor")?;
    if after == Some(next) {
        bail!("Linear page did not advance");
    }
    Ok((nodes, Some(next.to_owned())))
}
