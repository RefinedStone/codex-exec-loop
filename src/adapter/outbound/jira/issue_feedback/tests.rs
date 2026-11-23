use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use serde_json::json;

use super::*;
use crate::application::port::outbound::provider_credential_mutation_port::{
    ProviderMutationAuthorizationPort, ProviderMutationSendGuardPort,
};
use crate::application::port::outbound::provider_credentials_port::ProviderCredentialReadPort;
use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_work::IssueProjectScope;
use crate::domain::provider_credentials::*;

mod comment_cases;
mod transition_cases;

const ACTOR: &str = "account-1";
const SITE: &str = "https://example.atlassian.net";

struct Credentials {
    active: Arc<Mutex<Option<ConnectionRef>>>,
}
impl Credentials {
    fn new() -> Self {
        Self {
            active: Arc::new(Mutex::new(Some(ConnectionRef {
                profile_id: "a".repeat(64),
                provider: Provider::Jira,
                connection_id: "b".repeat(32),
                revision: 1,
            }))),
        }
    }
    fn rotate(&self) {
        let mut active = self.active.lock().unwrap();
        active.as_mut().unwrap().revision += 1;
    }
    fn metadata() -> ConnectionMetadata {
        ConnectionMetadata {
            configuration: ConnectionConfiguration {
                site_url: Some(SITE.into()),
                deployment: Some(JiraDeployment::Cloud),
                email: Some("account@example.invalid".into()),
            },
            account_id: ACTOR.into(),
            account_name: "Fixture".into(),
        }
    }
}
impl ProviderCredentialReadPort for Credentials {
    fn current_state(&self, provider: Provider) -> CredentialResult<ProviderCredentialState> {
        assert_eq!(provider, Provider::Jira);
        let active = self.active.lock().unwrap().clone();
        Ok(ProviderCredentialState {
            profile_id: "a".repeat(64),
            provider,
            revision: active.as_ref().map_or(2, |reference| reference.revision),
            status: if active.is_some() {
                ProviderConnectionStatus::Connected
            } else {
                ProviderConnectionStatus::Disconnected
            },
            active_connection: active.map(|reference| ConnectionSummary {
                reference,
                metadata: Self::metadata(),
            }),
            operation: None,
            capabilities: ProviderCredentialCapabilities {
                native_input: false,
                legacy_import: false,
                keyring: false,
            },
        })
    }
    fn validate_current(&self, expected: &ConnectionRef) -> CredentialResult<()> {
        if self.active.lock().unwrap().as_ref() == Some(expected) {
            Ok(())
        } else {
            Err(ProviderCredentialError::Conflict)
        }
    }
    fn acquire(&self, expected: &ConnectionRef) -> CredentialResult<ProviderCredentialLease> {
        self.validate_current(expected)?;
        Ok(ProviderCredentialLease {
            reference: expected.clone(),
            metadata: Self::metadata(),
            token: SecretToken::from_bytes(
                Provider::Jira,
                b"synthetic-jira-feedback-token".to_vec(),
            )?,
        })
    }
}
struct SendAuthorization {
    active: Arc<Mutex<Option<ConnectionRef>>>,
    expected: ConnectionRef,
    used: AtomicBool,
}
struct SendGuard;
impl ProviderMutationSendGuardPort for SendGuard {}
impl ProviderMutationAuthorizationPort for SendAuthorization {
    fn begin_send(&self) -> CredentialResult<Box<dyn ProviderMutationSendGuardPort>> {
        if self.used.swap(true, Ordering::SeqCst)
            || self.active.lock().unwrap().as_ref() != Some(&self.expected)
        {
            return Err(ProviderCredentialError::Conflict);
        }
        Ok(Box::new(SendGuard))
    }
}
impl ProviderCredentialMutationPort for Credentials {
    fn authorize_mutation(
        &self,
        expected: &ConnectionRef,
    ) -> CredentialResult<Arc<dyn ProviderMutationAuthorizationPort>> {
        self.validate_current(expected)?;
        Ok(Arc::new(SendAuthorization {
            active: self.active.clone(),
            expected: expected.clone(),
            used: AtomicBool::new(false),
        }))
    }
}

struct Remote {
    comments: Vec<Value>,
    status: String,
    done: bool,
    transitions: Vec<Value>,
    history: Vec<Value>,
    page_size: usize,
    incomplete: bool,
    parent: String,
    actor: String,
}
struct Api {
    remote: Mutex<Remote>,
    writes: AtomicUsize,
    reads: AtomicUsize,
    lose_ack: AtomicBool,
    before_send: Mutex<Option<Box<dyn Fn() + Send>>>,
}
impl Api {
    fn new() -> Self {
        Self {
            remote: Mutex::new(Remote {
                comments: vec![],
                status: "1".into(),
                done: false,
                transitions: vec![
                    json!({"id":"31","to":{"id":"3","statusCategory":{"key":"done"}},"fields":{}}),
                ],
                history: vec![],
                page_size: 100,
                incomplete: false,
                parent: "10".into(),
                actor: ACTOR.into(),
            }),
            writes: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
            lose_ack: AtomicBool::new(false),
            before_send: Mutex::new(None),
        }
    }
}
impl JiraIssueFeedbackApiPort for Api {
    fn read(
        &self,
        _lease: &ProviderCredentialLease,
        request: &JiraIssueFeedbackReadRequest,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<JiraIssueFeedbackDocument> {
        assert!(!cancelled.load(Ordering::SeqCst) && deadline > Instant::now());
        self.reads.fetch_add(1, Ordering::SeqCst);
        let remote = self.remote.lock().unwrap();
        let value = match request {
            JiraIssueFeedbackReadRequest::TenantInfo => json!({"cloudId":"cloud-1"}),
            JiraIssueFeedbackReadRequest::Actor => json!({"accountId":remote.actor,"active":true}),
            JiraIssueFeedbackReadRequest::Issue { issue_id } => {
                json!({"id":issue_id,"fields":{"project":{"id":remote.parent},
                "status":{"id":remote.status,"statusCategory":{"key":if remote.done {"done"}else{"indeterminate"}}}}})
            }
            JiraIssueFeedbackReadRequest::Comments { start_at, .. } => page(
                "comments",
                &remote.comments,
                *start_at,
                remote.page_size,
                remote.incomplete,
            ),
            JiraIssueFeedbackReadRequest::Comment { comment_id, .. } => remote
                .comments
                .iter()
                .find(|comment| comment["id"].as_str() == Some(comment_id))
                .cloned()
                .ok_or(ProviderCredentialError::NotFound)?,
            JiraIssueFeedbackReadRequest::CommentProperty { comment_id, .. } => {
                let comment = remote
                    .comments
                    .iter()
                    .find(|comment| comment["id"].as_str() == Some(comment_id))
                    .ok_or(ProviderCredentialError::NotFound)?;
                comment["properties"]
                    .as_array()
                    .and_then(|properties| properties.first())
                    .cloned()
                    .ok_or(ProviderCredentialError::NotFound)?
            }
            JiraIssueFeedbackReadRequest::Transitions { .. } => {
                json!({"transitions":remote.transitions})
            }
            JiraIssueFeedbackReadRequest::Changelog { start_at, .. } => page(
                "values",
                &remote.history,
                *start_at,
                remote.page_size,
                remote.incomplete,
            ),
        };
        Ok(JiraIssueFeedbackDocument::new(value))
    }
    fn write(
        &self,
        _lease: &ProviderCredentialLease,
        request: &JiraIssueFeedbackWriteRequest,
        authorization: Arc<dyn ProviderMutationAuthorizationPort>,
        _cancelled: &AtomicBool,
        _deadline: Instant,
    ) -> CredentialResult<JiraIssueFeedbackWriteResponse> {
        if let Some(hook) = self.before_send.lock().unwrap().take() {
            hook();
        }
        let _send = authorization.begin_send()?;
        self.writes.fetch_add(1, Ordering::SeqCst);
        let mut remote = self.remote.lock().unwrap();
        let response = match request {
            JiraIssueFeedbackWriteRequest::CreateComment {
                issue_id,
                body,
                property,
            } => {
                let id = (1000 + remote.comments.len()).to_string();
                let value = comment(issue_id, &id, body.clone(), property.clone(), &remote.actor);
                remote.comments.push(value.clone());
                JiraIssueFeedbackWriteResponse::Comment(JiraIssueFeedbackDocument::new(value))
            }
            JiraIssueFeedbackWriteRequest::UpdateComment {
                issue_id,
                comment_id,
                body,
                property,
            } => {
                let value = comment(
                    issue_id,
                    comment_id,
                    body.clone(),
                    property.clone(),
                    &remote.actor,
                );
                let existing = remote
                    .comments
                    .iter_mut()
                    .find(|comment| comment["id"].as_str() == Some(comment_id))
                    .ok_or(ProviderCredentialError::NotFound)?;
                *existing = value.clone();
                JiraIssueFeedbackWriteResponse::Comment(JiraIssueFeedbackDocument::new(value))
            }
            JiraIssueFeedbackWriteRequest::Transition { transition_id, .. } => {
                assert_eq!(transition_id, "31");
                let id = (2000 + remote.history.len()).to_string();
                let event = history(&id, &remote.status, "3", &remote.actor);
                remote.history.push(event);
                remote.status = "3".into();
                remote.done = true;
                JiraIssueFeedbackWriteResponse::Transitioned
            }
        };
        if self.lose_ack.load(Ordering::SeqCst) {
            Err(ProviderCredentialError::OutcomeUnknown)
        } else {
            Ok(response)
        }
    }
}
fn page(field: &str, values: &[Value], start: u64, size: usize, incomplete: bool) -> Value {
    let mut page =
        json!({"startAt":start,"maxResults":size,"total":values.len() + usize::from(incomplete)});
    page[field] = json!(
        values
            .iter()
            .skip(start as usize)
            .take(size)
            .cloned()
            .collect::<Vec<_>>()
    );
    page
}
fn comment(issue: &str, id: &str, body: Value, property: Value, actor: &str) -> Value {
    json!({"id":id,"self":format!("{SITE}/rest/api/3/issue/{issue}/comment/{id}"), "author":{"accountId":actor},
        "updateAuthor":{"accountId":actor},"body":body,"properties":[{"key":"akra.issue-feedback.v1","value":property}]})
}
fn history(id: &str, from: &str, to: &str, actor: &str) -> Value {
    json!({"id":id,"created":"2026-11-23T12:00:00.000+0000","author":{"accountId":actor},
        "items":[{"fieldId":"status","from":from,"to":to}]})
}
fn binding() -> IssueAutomationJiraBinding {
    IssueAutomationJiraBinding {
        site_url: SITE.into(),
        cloud_id: "cloud-1".into(),
        project_id: "10".into(),
        expected_account_id: ACTOR.into(),
        jql: "assignee = currentUser()".into(),
        mode: IssueAutomationAutonomy::Gated,
        close_transition: Some(IssueAutomationJiraCloseTransition {
            transition_id: "31".into(),
            from_status_ids: vec!["1".into()],
            to_status_id: "3".into(),
        }),
    }
}
fn issue() -> IssueReference {
    IssueReference {
        provider: IssueProvider::Jira,
        server_id: "cloud-1".into(),
        repository_id: "10".into(),
        issue_id: "100".into(),
        project_scope: IssueProjectScope {
            workspace_dir: "/fixture/workspace".into(),
        },
    }
}
struct Fixture {
    adapter: JiraIssueFeedbackAdapter,
    credentials: Arc<Credentials>,
    api: Arc<Api>,
    policy: Arc<Mutex<IssueAutomationJiraBinding>>,
}
impl Fixture {
    fn new() -> Self {
        let credentials = Arc::new(Credentials::new());
        let api = Arc::new(Api::new());
        let policy = Arc::new(Mutex::new(binding()));
        let reader = policy.clone();
        let adapter = JiraIssueFeedbackAdapter::new(
            credentials.clone(),
            api.clone(),
            Arc::new(move || Ok(reader.lock().unwrap().clone())),
        );
        Self {
            adapter,
            credentials,
            api,
            policy,
        }
    }
    fn intent(&self, id: &str, content: &str) -> IssueFeedbackIntent {
        let actor = self.adapter.actor_context(&issue()).unwrap();
        let body = format!("{content}\n\n{}", feedback_marker(id));
        let intent = IssueFeedbackIntent {
            intent_id: id.into(),
            issue: issue(),
            actor_id: actor.actor_id,
            connection: actor.connection,
            automation_policy_version: "a".repeat(64),
            disclosure_policy_version: "b".repeat(64),
            desired_revision: 1,
            operation: IssueFeedbackOperation::CreateComment {
                body_sha256: feedback_body_hash(&body),
                body,
                content_sha256: feedback_body_hash(content),
            },
            recorded_at: "2026-11-23T12:00:00Z".into(),
        };
        intent.validate().unwrap();
        intent
    }
    fn close_intent(&self) -> IssueFeedbackIntent {
        let mut intent = self.intent("close-1", "Status");
        let actor = IssueFeedbackActorContext {
            actor_id: intent.actor_id.clone(),
            connection: intent.connection.clone(),
        };
        let snapshot = self
            .adapter
            .read_jira_close_transition(
                &issue(),
                &actor,
                binding().close_transition.as_ref().unwrap(),
            )
            .unwrap();
        intent.operation = IssueFeedbackOperation::CloseIssue {
            integration_record_id: "integration-1".into(),
            integration_sha: "a".repeat(40),
            decision_id: "human-close-1".into(),
            previous_event_id: snapshot.previous_event_id,
            jira_transition: Some(snapshot.target),
        };
        intent.validate().unwrap();
        intent
    }
}
