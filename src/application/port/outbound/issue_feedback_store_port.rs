use crate::domain::issue_automation::autonomy::IssueAutonomySelection;
use crate::domain::issue_automation::collection::{IssueCollectionItem, StoredIssueCollection};
use crate::domain::issue_automation::coordinator::IssueWorkCheckpoint;
use crate::domain::issue_automation::execution::IssueExecutionRecord;
use crate::domain::issue_automation::feedback::{
    IssueFeedbackDesired, IssueFeedbackEvent, IssueFeedbackIntent, IssueFeedbackIntentRecord,
    IssueFeedbackReceipt, IssueFeedbackState,
};
use crate::domain::issue_work::IssueReference;
use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IssueFeedbackFence {
    pub collection_epoch: u64,
    pub observation_revision: Option<u64>,
    pub tracked_revision: Option<u64>,
    pub work_revision: Option<u64>,
    pub execution_revision: Option<u64>,
    pub autonomy_revision: u64,
    pub authorization_epoch: u64,
    pub feedback_revision: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueFeedbackSnapshot {
    pub collection: StoredIssueCollection,
    pub item: IssueCollectionItem,
    pub work: Option<IssueWorkCheckpoint>,
    pub execution: Option<IssueExecutionRecord>,
    pub autonomy: IssueAutonomySelection,
    pub authorization_epoch: u64,
    pub pending_intervention: bool,
    pub state: IssueFeedbackState,
    /// workspace 별칭과 관계없는 remote 4좌표의 영구 사실이다.
    pub first_grant_entered: bool,
    /// Sent 이후 의도의 수다. 확인 재조회나 semantic revision과 독립이다.
    pub publication_attempts: u64,
}
impl IssueFeedbackSnapshot {
    pub fn fence(&self) -> IssueFeedbackFence {
        IssueFeedbackFence {
            collection_epoch: self.collection.epoch,
            observation_revision: self.item.observation_revision,
            tracked_revision: self.item.tracked_revision,
            work_revision: self.work.as_ref().map(|w| w.revision),
            execution_revision: self.execution.as_ref().map(|e| e.progress.revision),
            autonomy_revision: self.autonomy.revision,
            authorization_epoch: self.authorization_epoch,
            feedback_revision: self.state.revision,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueFeedbackSaveResult {
    Saved(Box<IssueFeedbackState>),
    Conflict,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueFeedbackRecordResult {
    Created(IssueFeedbackIntentRecord),
    Existing(IssueFeedbackIntentRecord),
    Blocked,
    Conflict,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueFeedbackClaimResult {
    Claimed(IssueFeedbackIntentRecord),
    Existing(IssueFeedbackIntentRecord),
    Conflict,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueFeedbackCompletion {
    Confirmed(Box<IssueFeedbackReceipt>),
    NeedsConfirmation {
        contradictory: bool,
        /// None은 이미 저장한 ID를 지우지 않는다. 다른 Some으로 바꾸지 않는다.
        acknowledged_comment_id: Option<String>,
    },
    /// Recorded에서만 가능하다. Sent/NeedsConfirmation을 자동 폐기하면 안 된다.
    Abandoned,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueFeedbackGrantResult {
    Marked,
    AlreadyEntered,
    Conflict,
}

/// private issue DB의 독립 feedback 장부다. semantic triage revision을 증가시키지 않는다.
/// 모든 issue-wide 중복/미확정 검사는 같은 runtime의 remote 4좌표로 하되 owner full scope는 보존한다.
pub trait IssueFeedbackStorePort: Send + Sync {
    fn snapshot(&self, issue: &IssueReference) -> Result<Option<IssueFeedbackSnapshot>>;
    fn save_desired(
        &self,
        issue: &IssueReference,
        expected: IssueFeedbackFence,
        desired: &IssueFeedbackDesired,
    ) -> Result<IssueFeedbackSaveResult>;
    fn record(
        &self,
        expected: IssueFeedbackFence,
        intent: &IssueFeedbackIntent,
    ) -> Result<IssueFeedbackRecordResult>;
    /// 현재 observation/work/epoch/autonomy/owner와 immutable intent를 같은 transaction에서 재검사한다.
    fn claim_sent(
        &self,
        issue: &IssueReference,
        intent_id: &str,
        expected_revision: u64,
        expected: IssueFeedbackFence,
    ) -> Result<IssueFeedbackClaimResult>;
    fn complete(
        &self,
        issue: &IssueReference,
        intent_id: &str,
        expected_revision: u64,
        completion: &IssueFeedbackCompletion,
    ) -> Result<Option<IssueFeedbackIntentRecord>>;
    fn unresolved(&self, issue: &IssueReference) -> Result<Vec<IssueFeedbackIntentRecord>>;
    fn pending(&self, workspace: &str) -> Result<Vec<IssueFeedbackIntentRecord>>;
    /// 실제 work/full owner를 확인한다. 이미 허가가 있었던 upgrade row는 marker만 conservative seed한다.
    fn mark_first_grant(
        &self,
        issue: &IssueReference,
        work_order_id: &str,
        expected: IssueFeedbackFence,
    ) -> Result<IssueFeedbackGrantResult>;
    fn pending_events(&self, workspace: &str) -> Result<Vec<IssueFeedbackEvent>>;
    fn acknowledge_event(&self, workspace: &str, event_id: &str) -> Result<()>;
}
