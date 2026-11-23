use super::*;
use crate::domain::provider_credentials::JiraDeployment;
use serde_json::json;
use std::sync::atomic::Ordering;

mod fixtures;
mod observations;
mod pagination;
use fixtures::*;

#[test]
fn construction_is_inert_and_wrong_cloud_account_or_site_never_issues_http() {
    let fixture = Fixture::new();
    assert_eq!(fixture.credentials.reads.load(Ordering::SeqCst), 0);
    assert!(fixture.api.calls.lock().unwrap().is_empty());
    for change in 0..3 {
        let fixture = Fixture::new();
        {
            let mut summary = fixture.credentials.summary.lock().unwrap();
            match change {
                0 => summary.metadata.account_id = "different-account".into(),
                1 => {
                    summary.metadata.configuration.site_url =
                        Some("https://other.example.invalid".into())
                }
                _ => {
                    summary.metadata.configuration.deployment = Some(JiraDeployment::Server);
                    summary.metadata.configuration.email = None;
                }
            }
        }
        assert_eq!(
            fixture.adapter.discover(request(None)),
            Err(IssueSourceFailure::IdentityMismatch)
        );
        assert!(fixture.api.calls.lock().unwrap().is_empty());
    }
}

#[test]
fn a_credential_change_on_the_last_read_discards_the_page_and_resync() {
    let fixture = Fixture::new();
    fixture.api.page(vec![issue("10041", "APP-41")], true, None);
    fixture.api.rotate_on_call.store(4, Ordering::SeqCst);
    assert_eq!(
        fixture.adapter.discover(request(None)),
        Err(IssueSourceFailure::ProviderUnavailable)
    );
    assert_eq!(fixture.api.calls.lock().unwrap().len(), 4);

    let fixture = Fixture::new();
    fixture.api.resync(issue("10041", "APP-41"), editmeta());
    fixture.api.rotate_on_call.store(3, Ordering::SeqCst);
    assert_eq!(
        fixture.adapter.resync(resync()),
        IssueSourceRead::Unknown {
            issue: resync().issue,
            reason: IssueSourceFailure::ProviderUnavailable,
        }
    );
}

#[test]
fn resync_uses_the_stable_id_and_follows_key_rename_without_reinterpreting_identity() {
    let fixture = Fixture::new();
    fixture.api.resync(issue("10041", "RENAMED-71"), editmeta());
    let current = observed(fixture.adapter.resync(resync()));
    assert_eq!(current.issue, resync().issue);
    let jira = current.jira.unwrap();
    assert_eq!(jira.key, "RENAMED-71");
    assert_eq!(jira.connection, fixture.credentials.reference());
    assert_eq!(
        fixture.api.calls.lock().unwrap().as_slice(),
        &[
            JiraIssueSourceApiRequest::TenantInfo,
            JiraIssueSourceApiRequest::Issue {
                issue_id: "10041".into()
            },
            JiraIssueSourceApiRequest::EditMetadata {
                issue_id: "10041".into()
            },
        ]
    );
}

#[test]
fn a_wrong_tenant_issue_or_project_is_unknown_and_never_a_closed_observation() {
    for mismatch in 0..3 {
        let fixture = Fixture::new();
        if mismatch == 0 {
            fixture.api.answer(json!({"cloudId":"recreated-cloud"}));
        } else {
            let mut value = issue("10041", "APP-41");
            if mismatch == 1 {
                value["id"] = json!("10099");
            } else {
                value["fields"]["project"]["id"] = json!("10002");
            }
            fixture.api.resync(value, editmeta());
        }
        assert_eq!(
            fixture.adapter.resync(resync()),
            IssueSourceRead::Unknown {
                issue: resync().issue,
                reason: IssueSourceFailure::IdentityMismatch,
            }
        );
    }
}

#[test]
fn failed_edit_metadata_does_not_turn_required_fields_into_an_empty_success() {
    let fixture = Fixture::new();
    fixture.api.answer(json!({"cloudId":binding().cloud_id}));
    fixture.api.answer(issue("10041", "APP-41"));
    fixture.api.fail(ProviderCredentialError::Unauthorized);
    assert_eq!(
        fixture.adapter.resync(resync()),
        IssueSourceRead::Unknown {
            issue: resync().issue,
            reason: IssueSourceFailure::ProviderUnavailable,
        }
    );
}

#[test]
fn grouping_or_order_injection_is_rejected_before_any_provider_read() {
    let fixture = Fixture::new();
    for jql in [
        "assignee = currentUser()) OR project = 2",
        "(summary ~ 'x'",
        "status = Open ORDER BY key",
        "summary ~ 'unterminated",
    ] {
        let mut configured = binding();
        configured.jql = jql.into();
        assert!(
            JiraIssueSourceAdapter::new(
                configured,
                fixture.credentials.clone(),
                fixture.api.clone()
            )
            .is_err()
        );
    }
    let mut configured = binding();
    configured.jql = format!("{}status = Open{}", "(".repeat(65), ")".repeat(65));
    assert!(
        JiraIssueSourceAdapter::new(configured, fixture.credentials.clone(), fixture.api.clone())
            .is_err()
    );
    assert_eq!(fixture.credentials.reads.load(Ordering::SeqCst), 0);
    assert!(fixture.api.calls.lock().unwrap().is_empty());
}
