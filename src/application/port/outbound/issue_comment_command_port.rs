//! 이슈 댓글 명령의 읽기와 처리 기록이다 (IA8-2, #2567).
use anyhow::Result;

use crate::domain::issue_automation::collection::IssueCollectionItem;
use crate::domain::issue_automation::comment_command::{
    IssueCommentCommand, IssueCommentCommandRecord, IssueCommentRequestAcceptance,
    IssueCommentRequestBasis, IssueCommentRequestKey, StoredIssueCommentRequest,
};
use crate::domain::issue_work::IssueReference;

pub trait IssueCommentCommandSourcePort: Send + Sync {
    /// 신뢰하는 사용자 계정이 쓰고 아무도 고치지 않은 명령 댓글만 돌려준다.
    /// 다른 댓글의 글은 해석하지 않고 돌려주지 않는다. 읽기 실패는 빈 목록이 아니라 오류이다.
    fn read_comment_commands(
        &self,
        issue: &IssueReference,
        trusted_actor_ids: &[String],
    ) -> Result<Vec<IssueCommentCommand>>;
}

pub trait IssueCommentCommandStorePort: Send + Sync {
    fn current_request_issue(
        &self,
        _key: &IssueCommentRequestKey,
    ) -> Result<Option<IssueCollectionItem>> {
        anyhow::bail!("comment request issue observations are unavailable")
    }
    fn record_comment_request(
        &self,
        record: &IssueCommentCommandRecord,
        _basis: &IssueCommentRequestBasis,
    ) -> Result<bool> {
        self.record_issue_comment_command(&record.issue.project_scope.workspace_dir, record)
    }
    fn request_state(
        &self,
        _key: &IssueCommentRequestKey,
    ) -> Result<Option<(IssueCommentRequestBasis, IssueCommentRequestAcceptance)>> {
        Ok(None)
    }
    fn claim_request(&self, _request: &StoredIssueCommentRequest) -> Result<bool> {
        anyhow::bail!("comment request claims are unavailable")
    }
    fn accepted_requests(&self, _workspace: &str) -> Result<Vec<StoredIssueCommentRequest>> {
        Ok(Vec::new())
    }
    fn close_accepted_request(
        &self,
        _key: &IssueCommentRequestKey,
        _state: IssueCommentRequestAcceptance,
    ) -> Result<()> {
        anyhow::bail!("comment request claims are unavailable")
    }
    /// Bounded read of the immutable command records for this exact issue. Does not fetch comment bodies.
    fn recorded_requests(
        &self,
        workspace: &str,
        issue: &IssueReference,
    ) -> Result<Vec<IssueCommentCommandRecord>>;
    fn issue_comment_command(
        &self,
        workspace: &str,
        issue: &IssueReference,
        comment_id: &str,
    ) -> Result<Option<IssueCommentCommandRecord>>;

    /// 같은 이슈와 댓글 ID 의 기록이 이미 있으면 그 기록을 지키고 false 를 돌려준다.
    fn record_issue_comment_command(
        &self,
        workspace: &str,
        record: &IssueCommentCommandRecord,
    ) -> Result<bool>;
}
