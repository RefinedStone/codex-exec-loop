//! 신뢰하는 사람의 이슈 댓글 명령이다 (IA8-2, #2567, plan 28 §10.2).
//! 실행 중인 에이전트도 사용자 계정으로 댓글을 쓸 수 있다. 그래서 댓글은 사람 확인의 증거가 아니다.
//! 더 보수적으로 만드는 `hold` 만 바로 적용한다. 다른 명령은 기록만 하고 적용하지 않는다.
use serde::{Deserialize, Serialize};

use crate::domain::issue_work::IssueReference;

/// 댓글 하나로 만든 개입의 command_id 접두사이다. 같은 댓글은 같은 개입 intent 를 다시 쓴다.
pub const ISSUE_COMMENT_COMMAND_PREFIX: &str = "issue-comment:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueCommentCommandKind {
    Hold,
    Release,
    Retriage,
    Approve,
}

/// 기록한 명령의 처리 결과이다. 명령의 종류만으로 정해진다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueCommentCommandOutcome {
    /// 보류를 기존 개입 경로로 적용한다.
    Applied,
    /// 실행을 다시 열 수 있다. Desktop 의 사람 확인 경로에서 다시 내려야 한다.
    AwaitingNativeConfirmation,
    /// 댓글은 `human` 관문의 증거가 아니다. 결정 요청에 답하지 않는다.
    NotHumanEvidence,
}

impl IssueCommentCommandKind {
    /// 댓글 전체가 `/akra <명령>` 하나일 때만 명령이다. 다른 글이 섞이면 명령이 아니다.
    pub fn parse(body: &str) -> Option<Self> {
        let mut words = body.split_whitespace();
        if words.next()? != "/akra" {
            return None;
        }
        let kind = match words.next()? {
            "hold" => Self::Hold,
            "release" => Self::Release,
            "retriage" => Self::Retriage,
            "approve" => Self::Approve,
            _ => return None,
        };
        words.next().is_none().then_some(kind)
    }

    pub fn outcome(self) -> IssueCommentCommandOutcome {
        match self {
            Self::Hold => IssueCommentCommandOutcome::Applied,
            Self::Release | Self::Retriage => {
                IssueCommentCommandOutcome::AwaitingNativeConfirmation
            }
            Self::Approve => IssueCommentCommandOutcome::NotHumanEvidence,
        }
    }
}

/// 공급자에서 읽은 명령 댓글이다. 댓글의 글은 담지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueCommentCommand {
    pub comment_id: String,
    pub author_id: String,
    pub kind: IssueCommentCommandKind,
}

/// 처리한 명령 댓글의 영속 기록이다. 이슈의 안정 좌표와 댓글 ID 로 한 번만 기록한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueCommentCommandRecord {
    pub issue: IssueReference,
    pub comment_id: String,
    pub author_id: String,
    pub kind: IssueCommentCommandKind,
    /// 적용한 개입의 command_id 이다. `hold` 만 가진다.
    pub intervention_command_id: Option<String>,
}

pub fn issue_comment_intervention_command_id(comment_id: &str) -> String {
    format!("{ISSUE_COMMENT_COMMAND_PREFIX}{comment_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_whole_command_body_is_a_command() {
        assert_eq!(
            IssueCommentCommandKind::parse(" /akra hold\n"),
            Some(IssueCommentCommandKind::Hold)
        );
        assert_eq!(
            IssueCommentCommandKind::parse("/akra approve"),
            Some(IssueCommentCommandKind::Approve)
        );
        for body in [
            "",
            "/akra",
            "/akra stop",
            "/akra hold now",
            "please /akra hold",
            "/AKRA hold",
            "/akra Hold",
        ] {
            assert_eq!(IssueCommentCommandKind::parse(body), None, "{body}");
        }
    }

    #[test]
    fn only_hold_is_applied_and_approve_is_never_human_evidence() {
        assert_eq!(
            IssueCommentCommandKind::Hold.outcome(),
            IssueCommentCommandOutcome::Applied
        );
        for kind in [
            IssueCommentCommandKind::Release,
            IssueCommentCommandKind::Retriage,
        ] {
            assert_eq!(
                kind.outcome(),
                IssueCommentCommandOutcome::AwaitingNativeConfirmation
            );
        }
        assert_eq!(
            IssueCommentCommandKind::Approve.outcome(),
            IssueCommentCommandOutcome::NotHumanEvidence
        );
    }
}
