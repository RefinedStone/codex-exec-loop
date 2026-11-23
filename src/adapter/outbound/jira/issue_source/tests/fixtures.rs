use super::super::*;
use crate::application::port::outbound::jira_issue_source_api_port::JiraIssueSourceDocument;
use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_policy::IssueDisclosurePolicy;
use crate::domain::issue_work::IssueProjectScope;
use crate::domain::provider_credentials::*;
use serde_json::json;
use std::collections::VecDeque;
use std::sync::{
    Mutex,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

pub(super) const NOW: u64 = 1_795_000_000_000;

pub(super) fn binding() -> IssueAutomationJiraBinding {
    IssueAutomationJiraBinding {
        site_url: "https://jira.example.invalid".into(),
        cloud_id: "cloud-1".into(),
        project_id: "10001".into(),
        expected_account_id: "account-1".into(),
        jql: "assignee = currentUser()".into(),
        mode: IssueAutomationAutonomy::Propose,
        close_transition: None,
    }
}

pub(super) fn repository() -> IssueSourceRepository {
    IssueSourceRepository {
        provider: IssueProvider::Jira,
        server_id: binding().cloud_id,
        repository_id: binding().project_id,
        project_scope: IssueProjectScope {
            workspace_dir: "/work/jira-project".into(),
        },
    }
}

pub(super) fn request(cursor: Option<IssueDiscoveryCursor>) -> IssueDiscoveryRequest {
    IssueDiscoveryRequest {
        target: IssueDiscoveryTarget::Repository(repository()),
        selection: IssueDiscoverySelection::AllOpen,
        cursor,
        page_size: 2,
        disclosure: IssueSourceDisclosure {
            policy: IssueDisclosurePolicy::builtin(),
            policy_version: "policy-v1".into(),
        },
    }
}

pub(super) fn resync() -> IssueResyncRequest {
    IssueResyncRequest {
        issue: crate::domain::issue_work::IssueReference {
            provider: IssueProvider::Jira,
            server_id: binding().cloud_id,
            repository_id: binding().project_id,
            issue_id: "10041".into(),
            project_scope: repository().project_scope,
        },
        disclosure: request(None).disclosure,
    }
}

pub(super) fn issue(id: &str, key: &str) -> Value {
    json!({"id":id,"key":key,"fields":{
        "summary":"실행 설명을 보존한다", "updated":"2026-11-20T12:00:00.123+0000",
        "description":{"type":"doc","version":1,"content":[
            {"type":"heading","attrs":{"level":2},"content":[{"type":"text","text":"완료 기준"}]},
            {"type":"bulletList","content":[{"type":"listItem","content":[
                {"type":"paragraph","content":[{"type":"text","text":"결과가 저장된다"}]}]}]}]},
        "project":{"id":"10001","key":key.split_once('-').unwrap().0},
        "issuetype":{"id":"10002"},
        "status":{"id":"3","statusCategory":{"key":"indeterminate"}},
        "labels":["akra:gated"],"assignee":{"accountId":"account-1","accountType":"atlassian"},
        "creator":{"accountId":"creator-1","accountType":"atlassian"},"priority":{"name":"Medium"}
    }})
}

pub(super) fn editmeta() -> Value {
    json!({"fields":{
        "summary":{"required":true,"name":"Summary","schema":{"type":"string","system":"summary"}},
        "description":{"required":true,"name":"Description","schema":{"type":"string","system":"description"}}
    }})
}

pub(super) struct Credentials {
    pub summary: Mutex<ConnectionSummary>,
    pub reads: AtomicUsize,
}
impl Credentials {
    pub fn new() -> Self {
        Self {
            reads: AtomicUsize::new(0),
            summary: Mutex::new(ConnectionSummary {
                reference: ConnectionRef {
                    profile_id: "a".repeat(64),
                    provider: Provider::Jira,
                    connection_id: "b".repeat(32),
                    revision: 1,
                },
                metadata: ConnectionMetadata {
                    configuration: ConnectionConfiguration {
                        site_url: Some(binding().site_url),
                        deployment: Some(JiraDeployment::Cloud),
                        email: Some("user@example.invalid".into()),
                    },
                    account_id: binding().expected_account_id,
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
        assert_eq!(provider, Provider::Jira);
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
            token: SecretToken::from_bytes(Provider::Jira, b"synthetic-source-canary".to_vec())?,
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

pub(super) struct Api {
    pub answers: Mutex<VecDeque<CredentialResult<Value>>>,
    pub calls: Mutex<Vec<JiraIssueSourceApiRequest>>,
    pub rotate_on_call: AtomicUsize,
    credentials: Arc<Credentials>,
}
impl Api {
    pub fn answer(&self, value: Value) {
        self.answers.lock().unwrap().push_back(Ok(value));
    }
    pub fn fail(&self, error: ProviderCredentialError) {
        self.answers.lock().unwrap().push_back(Err(error));
    }
    pub fn prefix(&self) {
        self.answer(json!({"cloudId":binding().cloud_id}));
        self.answer(json!({"id":binding().project_id}));
    }
    pub fn page(&self, issues: Vec<Value>, is_last: bool, token: Option<&str>) {
        self.prefix();
        self.answer(json!({"issues":issues,"isLast":is_last,"nextPageToken":token}));
        for _ in issues {
            self.answer(editmeta());
        }
    }
    pub fn resync(&self, issue: Value, meta: Value) {
        self.answer(json!({"cloudId":binding().cloud_id}));
        self.answer(issue);
        self.answer(meta);
    }
}
impl JiraIssueSourceApiPort for Api {
    fn read(
        &self,
        _lease: &ProviderCredentialLease,
        request: &JiraIssueSourceApiRequest,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<JiraIssueSourceDocument> {
        assert!(!cancelled.load(Ordering::SeqCst));
        assert!(deadline > Instant::now());
        let mut calls = self.calls.lock().unwrap();
        calls.push(request.clone());
        if self.rotate_on_call.load(Ordering::SeqCst) == calls.len() {
            self.credentials.rotate();
        }
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected source HTTP call")
            .map(JiraIssueSourceDocument::new)
    }
}

pub(super) struct Fixture {
    pub adapter: JiraIssueSourceAdapter,
    pub credentials: Arc<Credentials>,
    pub api: Arc<Api>,
    pub now: Arc<AtomicU64>,
}
impl Fixture {
    pub fn new() -> Self {
        let credentials = Arc::new(Credentials::new());
        let api = Arc::new(Api {
            answers: Mutex::default(),
            calls: Mutex::default(),
            rotate_on_call: AtomicUsize::new(0),
            credentials: credentials.clone(),
        });
        let now = Arc::new(AtomicU64::new(NOW));
        let clock = now.clone();
        let mut adapter =
            JiraIssueSourceAdapter::new(binding(), credentials.clone(), api.clone()).unwrap();
        adapter.clock = Arc::new(move || clock.load(Ordering::SeqCst));
        Self {
            adapter,
            credentials,
            api,
            now,
        }
    }
    pub fn queries(&self) -> Vec<(String, Option<String>)> {
        self.api
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter_map(|call| match call {
                JiraIssueSourceApiRequest::Search {
                    jql,
                    next_page_token,
                    ..
                } => Some((jql.clone(), next_page_token.clone())),
                _ => None,
            })
            .collect()
    }
}

pub(super) fn observed(read: IssueSourceRead) -> IssueSourceObservation {
    match read {
        IssueSourceRead::Observed(value) => *value,
        other => panic!("expected observation, got {other:?}"),
    }
}
