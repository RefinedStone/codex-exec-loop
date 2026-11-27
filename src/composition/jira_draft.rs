//! One packet of draft adapters is fenced to one authenticated connection revision and stable issue ID.
use super::provider_credentials::ProviderServices;
use crate::adapter::outbound::jira::{JiraIssueSnapshotAdapter, JiraIssueUpdateAdapter};
use crate::application::port::inbound::issue_draft_port::JiraDraftCoordinate;
use crate::application::port::outbound::evidence_vault_port::EvidenceVaultStorePort;
use crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort;
use crate::application::port::outbound::jira_issue_source_api_port::{
    JiraIssueSourceApiPort, JiraIssueSourceApiRequest,
};
use crate::application::port::outbound::provider_credential_mutation_port::ProviderCredentialMutationPort;
use crate::application::port::outbound::provider_credentials_port::ProviderCredentialReadPort;
use crate::application::service::issue_draft::jira::{JiraDraftEndpoints, JiraDraftRouter};
use crate::application::service::issue_publish_journal::IssueJournaledPublishService;
use crate::domain::provider_credentials::{JiraDeployment, Provider};
use anyhow::{Result, ensure};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

pub(super) fn router(
    services: ProviderServices,
    journal: Arc<dyn IssuePublishIntentJournalPort>,
    vault: Arc<dyn EvidenceVaultStorePort>,
) -> JiraDraftRouter {
    Arc::new(move |target| {
        ensure!(
            target.connection.provider == Provider::Jira
                && [&target.project_id, &target.issue_id]
                    .into_iter()
                    .all(|id| !id.is_empty()
                        && id.len() <= 32
                        && id.bytes().all(|b| b.is_ascii_digit())),
            "invalid Jira draft target"
        );
        let read = || snapshot(&services, target);
        let writer_services = services.clone();
        let coordinate = target.clone();
        let writer = JiraIssueUpdateAdapter::new(
            read()?,
            Box::new(move |path, body| {
                ensure!(
                    path == format!("/rest/api/3/issue/{}", coordinate.issue_id),
                    "Jira target mismatch"
                );
                let lease = writer_services
                    .credentials
                    .acquire(&coordinate.connection)?;
                let authorization = writer_services
                    .credentials
                    .authorize_mutation(&coordinate.connection)?;
                writer_services.api.put_jira_draft(
                    &lease,
                    &coordinate.issue_id,
                    body,
                    authorization,
                )?;
                writer_services
                    .credentials
                    .validate_current(&coordinate.connection)?;
                Ok(())
            }),
        );
        let reader = Arc::new(read()?);
        let publisher = Arc::new(
            IssueJournaledPublishService::new(
                reader.clone(),
                Arc::new(writer),
                journal.clone(),
                vault.clone(),
            )
            .with_feedback_store(super::issue_collection::build_feedback_store()),
        );
        Ok(JiraDraftEndpoints {
            snapshot: reader.clone(),
            requirements: reader,
            publisher,
        })
    })
}
fn snapshot(
    services: &ProviderServices,
    target: &JiraDraftCoordinate,
) -> Result<JiraIssueSnapshotAdapter> {
    let lease = services.credentials.acquire(&target.connection)?;
    ensure!(
        lease.metadata.configuration.deployment == Some(JiraDeployment::Cloud),
        "Jira Cloud connection required"
    );
    let site = lease
        .metadata
        .configuration
        .site_url
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Jira site unavailable"))?;
    let services = services.clone();
    let target = target.clone();
    Ok(JiraIssueSnapshotAdapter::new(
        site,
        Box::new(move |path| {
            let issue_id = &target.issue_id;
            let query = if path == "/_edge/tenant_info" {
                JiraIssueSourceApiRequest::TenantInfo
            } else if path == format!("/rest/api/3/issue/{issue_id}/editmeta") {
                JiraIssueSourceApiRequest::EditMetadata {
                    issue_id: issue_id.clone(),
                }
            } else if path
                == format!(
                    "/rest/api/3/issue/{issue_id}?fields=summary,description,updated,project,issuetype"
                )
            {
                JiraIssueSourceApiRequest::Issue {
                    issue_id: issue_id.clone(),
                }
            } else {
                anyhow::bail!("Jira draft read outside the selected issue")
            };
            let lease = services.credentials.acquire(&target.connection)?;
            let value = services
                .api
                .read(
                    &lease,
                    &query,
                    &AtomicBool::new(false),
                    Instant::now() + Duration::from_secs(20),
                )?
                .into_value();
            services.credentials.validate_current(&target.connection)?;
            Ok(serde_json::to_vec(&value)?)
        }),
    ))
}
