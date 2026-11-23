use super::*;
use crate::application::port::outbound::provider_credentials_port::ProviderCredentialLease;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

struct Credentials {
    revision: AtomicU64,
    reads: AtomicUsize,
    provider: Provider,
}
impl Credentials {
    fn reference(&self) -> ConnectionRef {
        ConnectionRef {
            profile_id: "a".repeat(64),
            provider: self.provider,
            connection_id: "b".repeat(32),
            revision: self.revision.load(Ordering::SeqCst),
        }
    }
    fn metadata(&self) -> ConnectionMetadata {
        ConnectionMetadata {
            configuration: if self.provider == Provider::Jira {
                ConnectionConfiguration {
                    site_url: Some("https://jira.example:8443/context".to_owned()),
                    deployment: Some(JiraDeployment::Cloud),
                    email: Some("user@example.invalid".to_owned()),
                }
            } else {
                ConnectionConfiguration::default()
            },
            account_id: "account".to_owned(),
            account_name: "Person".to_owned(),
        }
    }
}
impl ProviderCredentialReadPort for Credentials {
    fn current_state(&self, provider: Provider) -> CredentialResult<ProviderCredentialState> {
        if provider != self.provider {
            return Err(ProviderCredentialError::NotConnected);
        }
        let revision = self.revision.load(Ordering::SeqCst);
        Ok(ProviderCredentialState {
            profile_id: "a".repeat(64),
            provider,
            revision,
            status: ProviderConnectionStatus::Connected,
            active_connection: Some(ConnectionSummary {
                reference: self.reference(),
                metadata: self.metadata(),
            }),
            operation: None,
            capabilities: ProviderCredentialCapabilities {
                native_input: true,
                legacy_import: true,
                keyring: true,
            },
        })
    }
    fn acquire(&self, reference: &ConnectionRef) -> CredentialResult<ProviderCredentialLease> {
        self.validate_current(reference)?;
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(ProviderCredentialLease {
            reference: reference.clone(),
            metadata: self.metadata(),
            token: SecretToken::from_bytes(self.provider, b"synthetic-read-canary".to_vec())?,
        })
    }
    fn validate_current(&self, reference: &ConnectionRef) -> CredentialResult<()> {
        if reference != &self.reference() {
            return Err(ProviderCredentialError::Conflict);
        }
        Ok(())
    }
}
#[derive(Default)]
struct Gate {
    entered: Mutex<bool>,
    entered_cv: Condvar,
    released: Mutex<bool>,
    released_cv: Condvar,
}
impl Gate {
    fn enter(&self) {
        *self.entered.lock().unwrap() = true;
        self.entered_cv.notify_all();
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.released_cv.wait(released).unwrap();
        }
    }
    fn wait(&self) {
        let entered = self.entered.lock().unwrap();
        let (entered, timeout) = self
            .entered_cv
            .wait_timeout_while(entered, Duration::from_secs(5), |value| !*value)
            .unwrap();
        assert!(*entered && !timeout.timed_out());
    }
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.released_cv.notify_all();
    }
}
struct Api {
    calls: AtomicUsize,
    gate: Option<Arc<Gate>>,
    error: bool,
}
impl Api {
    fn begin(&self) -> CredentialResult<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.gate {
            gate.enter();
        }
        if self.error {
            Err(ProviderCredentialError::Unavailable)
        } else {
            Ok(())
        }
    }
}
impl ProviderWorkApiPort for Api {
    fn jira_summary(
        &self,
        metadata: &ConnectionMetadata,
        _token: &SecretToken,
    ) -> CredentialResult<JiraSummary> {
        self.begin()?;
        Ok(JiraSummary {
            site_url: metadata.configuration.site_url.clone().unwrap(),
            issues: vec![],
        })
    }
    fn jira_issue_prompt(
        &self,
        _metadata: &ConnectionMetadata,
        _token: &SecretToken,
        _key: &str,
    ) -> CredentialResult<String> {
        self.begin()?;
        Ok("draft".to_owned())
    }
    fn jira_current_version(
        &self,
        _metadata: &ConnectionMetadata,
        _token: &SecretToken,
        _key: &str,
    ) -> CredentialResult<String> {
        self.begin()?;
        Ok("2026-11-23T01:02:03.000Z".to_owned())
    }
    fn linear_assigned_issues(&self, _token: &SecretToken) -> CredentialResult<Vec<LinearIssue>> {
        self.begin()?;
        Ok(vec![linear_issue()])
    }
    fn linear_issue_prompt(&self, _token: &SecretToken, _id: &str) -> CredentialResult<String> {
        self.begin()?;
        Ok("draft".to_owned())
    }
}
fn linear_issue() -> LinearIssue {
    LinearIssue {
        id: "issue-1".to_owned(),
        identifier: "APP-42".to_owned(),
        title: "Synthetic".to_owned(),
        url: "https://linear.app/team/issue/APP-42/title".to_owned(),
        priority: 2,
        updated_at: "2026-11-23T01:02:03Z".to_owned(),
        branch_name: None,
        state: LinearState {
            id: "state".to_owned(),
            name: "Started".to_owned(),
            r#type: "started".to_owned(),
        },
        team: LinearTeam {
            id: "team".to_owned(),
            key: "APP".to_owned(),
            name: "Team".to_owned(),
        },
        project: Some(NamedEntity {
            id: "project".to_owned(),
            name: "Project".to_owned(),
        }),
        cycle: Some(LinearCycle {
            id: "cycle".to_owned(),
            number: 1,
            name: None,
            is_active: true,
        }),
    }
}
fn fixture(
    provider: Provider,
    gate: Option<Arc<Gate>>,
    error: bool,
) -> (Arc<ProviderWorkService>, Arc<Credentials>, Arc<Api>) {
    let credentials = Arc::new(Credentials {
        revision: AtomicU64::new(1),
        reads: AtomicUsize::new(0),
        provider,
    });
    let api = Arc::new(Api {
        calls: AtomicUsize::new(0),
        gate,
        error,
    });
    (
        Arc::new(ProviderWorkService::new(credentials.clone(), api.clone())),
        credentials,
        api,
    )
}
#[test]
fn headless_jira_current_version_needs_no_desktop_session() {
    let (service, credentials, api) = fixture(Provider::Jira, None, false);
    let resource = ExternalResourceRef {
        kind: "jira".to_owned(),
        key: "jira.example:8443/context/APP-42".to_owned(),
        url: "https://jira.example:8443/context/browse/APP-42".to_owned(),
        title_hint: None,
    };
    assert!(service.available(&resource));
    assert_eq!(credentials.reads.load(Ordering::SeqCst), 0);
    assert_eq!(
        service.current_version(&resource).unwrap(),
        "2026-11-23T01:02:03.000Z"
    );
    assert_eq!(api.calls.load(Ordering::SeqCst), 1);
    let mut wrong = resource;
    wrong.url = "https://evil.example/browse/APP-42".to_owned();
    assert!(!service.available(&wrong));
    assert!(service.current_version(&wrong).is_err());
    assert_eq!(api.calls.load(Ordering::SeqCst), 1);
}
#[test]
fn linear_filters_reuse_thirty_item_cache_without_keyring_or_http() {
    let (service, credentials, api) = fixture(Provider::Linear, None, false);
    let reference = credentials.reference();
    let initial = service
        .linear_summary(&reference, &LinearFilters::default(), false)
        .unwrap();
    assert_eq!(initial.total_cached, 1);
    assert_eq!(initial.limit, 30);
    assert_eq!(initial.teams.len(), 1);
    let filtered = service
        .linear_summary(
            &reference,
            &LinearFilters {
                team_id: Some("other".to_owned()),
                project_id: None,
                current_cycle: false,
            },
            false,
        )
        .unwrap();
    assert!(filtered.issues.is_empty());
    assert_eq!(filtered.total_cached, 1);
    assert_eq!(credentials.reads.load(Ordering::SeqCst), 1);
    assert_eq!(api.calls.load(Ordering::SeqCst), 1);
    service
        .linear_summary(&reference, &LinearFilters::default(), true)
        .unwrap();
    assert_eq!(api.calls.load(Ordering::SeqCst), 2);
}
#[test]
fn overlapping_refreshes_share_the_same_inflight_request() {
    let gate = Arc::new(Gate::default());
    let (service, credentials, api) = fixture(Provider::Linear, Some(gate.clone()), false);
    let reference = credentials.reference();
    let first = {
        let service = service.clone();
        let reference = reference.clone();
        std::thread::spawn(move || {
            service.linear_summary(&reference, &LinearFilters::default(), true)
        })
    };
    gate.wait();
    let second = {
        let service = service.clone();
        let reference = reference.clone();
        std::thread::spawn(move || {
            service.linear_summary(&reference, &LinearFilters::default(), true)
        })
    };
    // Wait for the second thread to acquire the shared flight rather than relying on a sleep.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let joined = service
            .linear
            .lock()
            .unwrap()
            .flight
            .as_ref()
            .is_some_and(|flight| Arc::strong_count(flight) >= 3);
        if joined {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    gate.release();
    assert_eq!(first.join().unwrap().unwrap().issues.len(), 1);
    assert_eq!(second.join().unwrap().unwrap().issues.len(), 1);
    assert_eq!(api.calls.load(Ordering::SeqCst), 1);
}
#[test]
fn disconnected_or_replaced_connection_cannot_publish_late_jira_or_linear_reads() {
    for provider in [Provider::Jira, Provider::Linear] {
        let gate = Arc::new(Gate::default());
        let (service, credentials, _api) = fixture(provider, Some(gate.clone()), false);
        let reference = credentials.reference();
        let task = {
            let service = service.clone();
            let reference = reference.clone();
            std::thread::spawn(move || {
                if provider == Provider::Linear {
                    service
                        .linear_summary(&reference, &LinearFilters::default(), false)
                        .map(|_| ())
                } else {
                    service.jira_summary(&reference).map(|_| ())
                }
            })
        };
        gate.wait();
        credentials.revision.store(2, Ordering::SeqCst);
        gate.release();
        assert_eq!(
            task.join().unwrap().unwrap_err(),
            ProviderCredentialError::Conflict
        );
        if provider == Provider::Linear {
            assert!(service.linear.lock().unwrap().cached.is_none());
        }
        assert!(
            service
                .validate_link(&ProviderLinkRequest {
                    connection: reference,
                    url: "https://linear.app/team/issue/APP-42".to_owned()
                })
                .is_err()
        );
    }
}
#[test]
fn only_current_connection_can_open_a_site_bound_link() {
    let (service, credentials, _) = fixture(Provider::Jira, None, false);
    let request = ProviderLinkRequest {
        connection: credentials.reference(),
        url: "https://jira.example:8443/context/browse/APP-42".to_owned(),
    };
    assert_eq!(service.validate_link(&request).unwrap(), request.url);
    assert_eq!(credentials.reads.load(Ordering::SeqCst), 0);
    assert!(
        service
            .validate_link(&ProviderLinkRequest {
                url: "https://jira.example/browse/APP-42".to_owned(),
                ..request.clone()
            })
            .is_err()
    );
    credentials.revision.store(2, Ordering::SeqCst);
    assert_eq!(
        service.validate_link(&request).unwrap_err(),
        ProviderCredentialError::Conflict
    );
}
