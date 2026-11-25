// Linear issue automation adapters: team-bound discovery/resync and status feedback (#2564).
// Credentials stay in the Gateway; only fixed GraphQL documents leave this process.
pub mod issue_feedback;
pub mod issue_source;

pub use self::issue_feedback::LinearIssueFeedbackAdapter;
pub use self::issue_source::LinearIssueSourceAdapter;
#[cfg(test)]
pub(crate) mod test_support;
