//! 신뢰하는 사용자 계정의 명령 댓글만 고른다 (IA8-2, #2567).
//! 작성자와 수정 여부를 먼저 본다. 고른 댓글이 아니면 글을 해석하지 않고 버린다.
use anyhow::{Result, bail};

use super::GithubIssueFeedbackAdapter;
use crate::adapter::outbound::github::issue_snapshot::ensure_node_id;
use crate::application::port::outbound::issue_comment_command_port::IssueCommentCommandSourcePort;
use crate::domain::issue_automation::comment_command::{
    IssueCommentCommand, IssueCommentCommandKind,
};
use crate::domain::issue_work::IssueReference;

impl IssueCommentCommandSourcePort for GithubIssueFeedbackAdapter {
    fn read_comment_commands(
        &self,
        issue: &IssueReference,
        trusted_actor_ids: &[String],
    ) -> Result<Vec<IssueCommentCommand>> {
        Self::validate_issue(issue)?;
        let mut commands = Vec::new();
        for comment in self.read_comments(issue)? {
            Self::require_issue(&comment.issue, issue)?;
            if comment.repository.id != issue.repository_id || comment.pull_request.is_some() {
                bail!("GitHub returned a comment from a different issue")
            }
            let Some(author_id) = comment
                .author
                .as_ref()
                .filter(|author| author.typename == "User")
                .and_then(|author| author.id.as_deref())
            else {
                continue;
            };
            // 고친 댓글은 다른 계정이 고쳤을 수 있고 이미 처리한 명령을 바꿀 수 있다. 명령으로 보지 않는다.
            if !trusted_actor_ids.iter().any(|id| id == author_id)
                || comment.last_edited_at.is_some()
                || comment.editor.is_some()
            {
                continue;
            }
            let Some(kind) = IssueCommentCommandKind::parse(&comment.body) else {
                continue;
            };
            ensure_node_id(&comment.id)?;
            commands.push(IssueCommentCommand {
                comment_id: comment.id,
                author_id: author_id.to_owned(),
                kind,
            });
        }
        Ok(commands)
    }
}
