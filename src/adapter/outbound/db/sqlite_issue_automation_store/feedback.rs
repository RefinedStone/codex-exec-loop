//! Durable issue feedback state, publication claims, and an independent UI outbox.
mod completion;
mod intents;
mod outbox;
mod schema;
mod state;

use anyhow::{Result, anyhow, bail};
use rusqlite::Connection;
use serde::{Serialize, de::DeserializeOwned};

use super::{SqliteIssueAutomationStoreAdapter, validate_reference_scope};
use crate::application::port::outbound::issue_feedback_store_port::{
    IssueFeedbackClaimResult, IssueFeedbackCompletion, IssueFeedbackFence,
    IssueFeedbackGrantResult, IssueFeedbackRecordResult, IssueFeedbackSaveResult,
    IssueFeedbackSnapshot, IssueFeedbackStorePort,
};
use crate::domain::issue_automation::feedback::{
    IssueFeedbackDesired, IssueFeedbackEvent, IssueFeedbackIntent, IssueFeedbackIntentRecord,
    validate_feedback_issue,
};
use crate::domain::issue_work::IssueReference;

const MAX_RECORD_BYTES: usize = 256 * 1024;

pub(super) fn schema_objects() -> impl Iterator<Item = (&'static str, &'static str)> {
    schema::TABLES.into_iter().chain(schema::INDEXES)
}

pub(super) fn create_schema(connection: &Connection) -> Result<()> {
    schema::create(connection)
}

pub(super) fn seed_existing_grants(connection: &Connection) -> Result<()> {
    schema::seed_existing_grants(connection)
}

fn validate_issue(issue: &IssueReference) -> Result<()> {
    validate_feedback_issue(issue).map_err(|reason| anyhow!(reason))?;
    validate_reference_scope(&issue.project_scope.workspace_dir, issue)
}

fn encode<T: Serialize>(value: &T) -> Result<String> {
    let encoded = serde_json::to_string(value)?;
    if encoded.len() > MAX_RECORD_BYTES {
        bail!("the feedback record exceeds its storage bound");
    }
    Ok(encoded)
}

fn decode<T: DeserializeOwned>(encoded: String) -> Result<T> {
    if encoded.len() > MAX_RECORD_BYTES {
        bail!("the stored feedback record exceeds its storage bound");
    }
    serde_json::from_str(&encoded).map_err(|_| anyhow!("the stored feedback record is invalid"))
}

impl IssueFeedbackStorePort for SqliteIssueAutomationStoreAdapter {
    fn snapshot(&self, issue: &IssueReference) -> Result<Option<IssueFeedbackSnapshot>> {
        state::snapshot(self, issue)
    }

    fn save_desired(
        &self,
        issue: &IssueReference,
        expected: IssueFeedbackFence,
        desired: &IssueFeedbackDesired,
    ) -> Result<IssueFeedbackSaveResult> {
        state::save_desired(self, issue, expected, desired)
    }

    fn record(
        &self,
        expected: IssueFeedbackFence,
        intent: &IssueFeedbackIntent,
    ) -> Result<IssueFeedbackRecordResult> {
        intents::record(self, expected, intent)
    }

    fn claim_sent(
        &self,
        issue: &IssueReference,
        intent_id: &str,
        expected_revision: u64,
        expected: IssueFeedbackFence,
    ) -> Result<IssueFeedbackClaimResult> {
        intents::claim_sent(self, issue, intent_id, expected_revision, expected)
    }

    fn complete(
        &self,
        issue: &IssueReference,
        intent_id: &str,
        expected_revision: u64,
        completion: &IssueFeedbackCompletion,
    ) -> Result<Option<IssueFeedbackIntentRecord>> {
        completion::complete(self, issue, intent_id, expected_revision, completion)
    }

    fn unresolved(&self, issue: &IssueReference) -> Result<Vec<IssueFeedbackIntentRecord>> {
        intents::unresolved(self, issue)
    }

    fn pending(&self, workspace: &str) -> Result<Vec<IssueFeedbackIntentRecord>> {
        intents::pending(self, workspace)
    }

    fn mark_first_grant(
        &self,
        issue: &IssueReference,
        work_order_id: &str,
        expected: IssueFeedbackFence,
    ) -> Result<IssueFeedbackGrantResult> {
        state::mark_first_grant(self, issue, work_order_id, expected)
    }

    fn pending_events(&self, workspace: &str) -> Result<Vec<IssueFeedbackEvent>> {
        outbox::pending(self, workspace)
    }

    fn acknowledge_event(&self, workspace: &str, event_id: &str) -> Result<()> {
        outbox::acknowledge(self, workspace, event_id)
    }
}

#[cfg(test)]
mod tests;
