use anyhow::Result;

use crate::domain::issue_creation::IssueCreationIntentRecord;

/// A repository transaction holds its process-shared lease until it is dropped.
/// The service keeps it across intent lookup, durable state changes and one remote creation.
pub trait IssueCreationJournalTransaction {
    fn records(&mut self) -> Result<Vec<IssueCreationIntentRecord>>;
    fn save(&mut self, record: &IssueCreationIntentRecord) -> Result<()>;
}

pub trait IssueCreationJournalPort: Send + Sync {
    /// Return the canonical existing workspace identity without performing a remote write.
    fn canonical_workspace(&self, workspace_dir: &str) -> Result<String>;

    fn begin(&self, workspace_dir: &str) -> Result<Box<dyn IssueCreationJournalTransaction + '_>>;
}
