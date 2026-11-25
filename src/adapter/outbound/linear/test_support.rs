//! A fake Linear GraphQL server and Gateway credential owner. No network or real token is used.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{Value, json};

use crate::application::port::outbound::linear_issue_api_port::*;
use crate::application::port::outbound::provider_credential_mutation_port::{
    ProviderCredentialMutationPort, ProviderMutationAuthorizationPort,
    ProviderMutationSendGuardPort,
};
use crate::application::port::outbound::provider_credentials_port::{
    ProviderCredentialLease, ProviderCredentialReadPort,
};
use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_automation::policy::IssueAutomationLinearBinding;
use crate::domain::provider_credentials::*;

pub(crate) const TEAM: &str = "6f1c1a52-0000-4000-8000-00000000a001";
pub(crate) const VIEWER: &str = "6f1c1a52-0000-4000-8000-00000000b001";
pub(crate) const ISSUE: &str = "6f1c1a52-0000-4000-8000-00000000c001";
pub(crate) const STARTED: &str = "6f1c1a52-0000-4000-8000-00000000d001";
pub(crate) const DONE: &str = "6f1c1a52-0000-4000-8000-00000000d002";

pub(crate) fn binding() -> IssueAutomationLinearBinding {
    IssueAutomationLinearBinding {
        team_id: TEAM.into(),
        expected_viewer_id: VIEWER.into(),
        mode: IssueAutomationAutonomy::Propose,
        close_state: None,
    }
}

/// One Linear `Issue` node with the fields the fixed documents select.
pub(crate) fn issue(id: &str, identifier: &str) -> Value {
    json!({
        "id": id,
        "identifier": identifier,
        "title": "실행 설명을 보존한다",
        "description": "## 완료 기준\n- 결과가 저장된다\n",
        "url": format!("https://linear.app/example/issue/{identifier}/keep-the-description"),
        "branchName": format!("operator/{}-keep-the-description", identifier.to_ascii_lowercase()),
        "updatedAt": "2026-11-20T12:00:00.123Z",
        "state": {"id": STARTED, "type": "started"},
        "team": {"id": TEAM},
        "creator": {"id": "6f1c1a52-0000-4000-8000-00000000b002", "app": false},
        "assignee": null,
        "labels": {"nodes": [{"id": "6f1c1a52-0000-4000-8000-00000000e001", "name": "akra:gated"}],
                   "pageInfo": {"hasNextPage": false}},
    })
}

pub(crate) struct Credentials {
    pub summary: Mutex<ConnectionSummary>,
    pub reads: AtomicUsize,
    pub sends: AtomicUsize,
}
impl Credentials {
    pub fn new() -> Self {
        Self {
            reads: AtomicUsize::new(0),
            sends: AtomicUsize::new(0),
            summary: Mutex::new(ConnectionSummary {
                reference: ConnectionRef {
                    profile_id: "a".repeat(64),
                    provider: Provider::Linear,
                    connection_id: "b".repeat(32),
                    revision: 1,
                },
                metadata: ConnectionMetadata {
                    configuration: ConnectionConfiguration::default(),
                    account_id: VIEWER.into(),
                    account_name: "Synthetic User".into(),
                },
            }),
        }
    }
    pub fn reference(&self) -> ConnectionRef {
        self.summary.lock().unwrap().reference.clone()
    }
    pub fn rotate(&self) {
        self.summary.lock().unwrap().reference.revision += 1;
    }
}
impl ProviderCredentialReadPort for Credentials {
    fn current_state(&self, provider: Provider) -> CredentialResult<ProviderCredentialState> {
        assert_eq!(provider, Provider::Linear);
        self.reads.fetch_add(1, Ordering::SeqCst);
        let summary = self.summary.lock().unwrap().clone();
        Ok(ProviderCredentialState {
            profile_id: summary.reference.profile_id.clone(),
            provider,
            revision: summary.reference.revision,
            status: ProviderConnectionStatus::Connected,
            active_connection: Some(summary),
            operation: None,
            capabilities: ProviderCredentialCapabilities {
                native_input: false,
                legacy_import: false,
                keyring: false,
            },
        })
    }
    fn acquire(&self, expected: &ConnectionRef) -> CredentialResult<ProviderCredentialLease> {
        self.validate_current(expected)?;
        Ok(ProviderCredentialLease {
            reference: expected.clone(),
            metadata: self.summary.lock().unwrap().metadata.clone(),
            token: SecretToken::from_bytes(Provider::Linear, b"lin_api_synthetic_canary".to_vec())?,
        })
    }
    fn validate_current(&self, expected: &ConnectionRef) -> CredentialResult<()> {
        if *expected == self.reference() {
            Ok(())
        } else {
            Err(ProviderCredentialError::Conflict)
        }
    }
}
struct SendGuard;
impl ProviderMutationSendGuardPort for SendGuard {}
struct OneShot {
    credentials: Arc<Credentials>,
    reference: ConnectionRef,
    used: AtomicBool,
}
impl ProviderMutationAuthorizationPort for OneShot {
    fn begin_send(&self) -> CredentialResult<Box<dyn ProviderMutationSendGuardPort>> {
        if self.used.swap(true, Ordering::SeqCst) {
            return Err(ProviderCredentialError::Conflict);
        }
        self.credentials.validate_current(&self.reference)?;
        self.credentials.sends.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(SendGuard))
    }
}
/// `Arc<Credentials>` is the owner; the trait needs the shared handle for one-shot admissions.
pub(crate) struct SharedCredentials(pub Arc<Credentials>);
impl ProviderCredentialReadPort for SharedCredentials {
    fn current_state(&self, provider: Provider) -> CredentialResult<ProviderCredentialState> {
        self.0.current_state(provider)
    }
    fn acquire(&self, expected: &ConnectionRef) -> CredentialResult<ProviderCredentialLease> {
        self.0.acquire(expected)
    }
    fn validate_current(&self, expected: &ConnectionRef) -> CredentialResult<()> {
        self.0.validate_current(expected)
    }
}
impl ProviderCredentialMutationPort for SharedCredentials {
    fn authorize_mutation(
        &self,
        expected: &ConnectionRef,
    ) -> CredentialResult<Arc<dyn ProviderMutationAuthorizationPort>> {
        self.0.validate_current(expected)?;
        Ok(Arc::new(OneShot {
            credentials: self.0.clone(),
            reference: expected.clone(),
            used: AtomicBool::new(false),
        }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Call {
    Viewer,
    Team,
    Issues {
        from: String,
        before: String,
        after: Option<String>,
    },
    Issue(String),
    Comments(Option<String>),
    Comment(String),
    History(Option<String>),
    WorkflowState(String),
    CreateComment(String),
    UpdateComment(String, String),
    UpdateState(String),
}

/// Answers are consumed in call order; an unexpected call fails the test.
pub(crate) struct Api {
    pub answers: Mutex<VecDeque<CredentialResult<Value>>>,
    pub calls: Mutex<Vec<Call>>,
    pub rotate_on_call: AtomicUsize,
    pub credentials: Arc<Credentials>,
}
impl Api {
    pub fn new(credentials: Arc<Credentials>) -> Self {
        Self {
            answers: Mutex::default(),
            calls: Mutex::default(),
            rotate_on_call: AtomicUsize::new(0),
            credentials,
        }
    }
    pub fn answer(&self, value: Value) {
        self.answers.lock().unwrap().push_back(Ok(value));
    }
    pub fn fail(&self, error: ProviderCredentialError) {
        self.answers.lock().unwrap().push_back(Err(error));
    }
    pub fn identity(&self) {
        self.answer(json!({"viewer": {"id": VIEWER, "active": true}}));
        self.answer(json!({"team": {"id": TEAM}}));
    }
    pub fn page(&self, issues: Vec<Value>, end_cursor: Option<&str>) {
        self.identity();
        self.answer(json!({"issues": {"nodes": issues, "pageInfo": {
            "hasNextPage": end_cursor.is_some(), "endCursor": end_cursor}}}));
    }
    pub fn resync(&self, issue: Value) {
        self.identity();
        self.answer(json!({ "issue": issue }));
    }
    fn record(&self, call: Call) -> CredentialResult<LinearIssueDocument> {
        let mut calls = self.calls.lock().unwrap();
        calls.push(call);
        if self.rotate_on_call.load(Ordering::SeqCst) == calls.len() {
            self.credentials.rotate();
        }
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected Linear GraphQL call")
            .map(LinearIssueDocument::new)
    }
}
impl LinearIssueApiPort for Api {
    fn read(
        &self,
        _lease: &ProviderCredentialLease,
        request: &LinearIssueReadRequest,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<LinearIssueDocument> {
        assert!(!cancelled.load(Ordering::SeqCst));
        assert!(deadline > Instant::now());
        self.record(match request {
            LinearIssueReadRequest::Viewer => Call::Viewer,
            LinearIssueReadRequest::Team { team_id } => {
                assert_eq!(team_id, TEAM);
                Call::Team
            }
            LinearIssueReadRequest::Issues {
                team_id,
                updated_from,
                updated_before,
                after,
                ..
            } => {
                assert_eq!(team_id, TEAM);
                Call::Issues {
                    from: updated_from.clone(),
                    before: updated_before.clone(),
                    after: after.clone(),
                }
            }
            LinearIssueReadRequest::Issue { issue_id } => Call::Issue(issue_id.clone()),
            LinearIssueReadRequest::Comments { after, .. } => Call::Comments(after.clone()),
            LinearIssueReadRequest::Comment { comment_id } => Call::Comment(comment_id.clone()),
            LinearIssueReadRequest::History { after, .. } => Call::History(after.clone()),
            LinearIssueReadRequest::WorkflowState { state_id } => {
                Call::WorkflowState(state_id.clone())
            }
        })
    }
    fn write(
        &self,
        _lease: &ProviderCredentialLease,
        request: &LinearIssueWriteRequest,
        authorization: Arc<dyn ProviderMutationAuthorizationPort>,
        _cancelled: &AtomicBool,
        _deadline: Instant,
    ) -> CredentialResult<LinearIssueDocument> {
        let _guard = authorization.begin_send()?;
        self.record(match request {
            LinearIssueWriteRequest::CreateComment { body, .. } => {
                Call::CreateComment(body.clone())
            }
            LinearIssueWriteRequest::UpdateComment { comment_id, body } => {
                Call::UpdateComment(comment_id.clone(), body.clone())
            }
            LinearIssueWriteRequest::UpdateState { state_id, .. } => {
                Call::UpdateState(state_id.clone())
            }
        })
    }
}
