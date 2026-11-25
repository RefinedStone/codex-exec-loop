//! 진행 단계를 GitHub Projects 보드 필드 하나에 쓴다 (#2565).
//! 쓰기는 `feedback.board_field` 허가와 사용자 파일의 진행 선택지가 있을 때만 일지를 지나 한 번 보낸다.
use super::*;
use crate::application::port::outbound::issue_board_field_port::{
    IssueBoardField, IssueBoardFieldWriteRequest, IssueBoardItemsReadRequest,
};
use crate::domain::issue_automation::policy::IssueAutomationGitHubBoard;
use crate::domain::issue_work::{IssueWriteApproval, IssueWriteOperation};

/// 지금 진행 단계의 목표 선택지이다. 대기, 실패, 취소는 쓰지 않는다.
pub(super) fn board_target<'a>(
    snapshot: &IssueFeedbackSnapshot,
    policy: &'a IssueAutomationPolicy,
) -> Option<(&'a IssueAutomationGitHubBoard, &'a str)> {
    if !policy.feedback.board_field || snapshot.item.issue.provider != IssueProvider::GitHub {
        return None;
    }
    let board = policy.binding.github.board.as_ref()?;
    let progress = board.progress.as_ref()?;
    if snapshot
        .item
        .tracked
        .as_ref()
        .is_some_and(|tracked| tracked.state == IssueAutomationState::Cancelled)
    {
        return None;
    }
    let target = match snapshot.execution.as_ref()?.progress.state {
        IssueExecutionState::Running => progress.running.as_deref(),
        IssueExecutionState::InReview => progress.in_review.as_deref(),
        IssueExecutionState::Done => progress.done.as_deref(),
        IssueExecutionState::Queued | IssueExecutionState::Failed => None,
    }?;
    Some((board, target))
}

/// 마지막 자기 쓰기가 이 항목의 이 필드에 남긴 값이다.
fn owned_option<'a>(
    snapshot: &'a IssueFeedbackSnapshot,
    board: &IssueAutomationGitHubBoard,
    item_id: &str,
) -> Option<&'a str> {
    match snapshot
        .state
        .board_proof
        .as_ref()
        .map(|proof| &proof.evidence)
    {
        Some(IssueFeedbackEvidence::BoardField {
            project_id,
            item_id: owned_item,
            field_id,
            option_id,
            ..
        }) if *project_id == board.project_id
            && owned_item == item_id
            && *field_id == board.field_id =>
        {
            Some(option_id)
        }
        _ => None,
    }
}

impl IssueFeedbackService {
    pub(super) fn sync_board_field(
        &self,
        snapshot: &IssueFeedbackSnapshot,
        automation: &ResolvedIssueAutomationPolicy,
        disclosure: &IssuePolicy,
        now_unix_ms: u64,
        cancel: &IssueCollectionCancellation,
        lease: &dyn IssuePublicationLease,
    ) -> Result<()> {
        let Some(port) = &self.board else {
            return Ok(());
        };
        let Some((board, target)) = board_target(snapshot, &automation.policy) else {
            return Ok(());
        };
        let issue = &snapshot.item.issue;
        if self.blocked_by_unresolved(issue)? {
            return Ok(());
        }
        // 읽지 못한 보드는 쓰지 않는다 (`read:project` 권한 없음 포함). 다음 조정에서 다시 본다.
        let Ok(items) = port.read_items(IssueBoardItemsReadRequest {
            issue: issue.clone(),
        }) else {
            return Ok(());
        };
        if items.issue != *issue {
            return Ok(());
        }
        let Some((item, field)) = items
            .items
            .iter()
            .filter(|item| item.project_id == board.project_id)
            .find_map(|item| {
                item.fields
                    .iter()
                    .find(|field| field.field_id == board.field_id)
                    .map(|field| (item, field))
            })
        else {
            // 보드에 없는 이슈를 보드에 넣지 않는다.
            return Ok(());
        };
        let current = field.selected_option_id.as_deref();
        if current == Some(target) || !has_option(field, target) {
            return Ok(());
        }
        let may_write = match owned_option(snapshot, board, &item.item_id) {
            // 사람이 자기 쓰기 뒤의 값을 바꿨으면 존중한다.
            Some(owned) => current == Some(owned),
            None => {
                board.select_option_ids.is_empty()
                    || current.is_some_and(|id| board.select_option_ids.iter().any(|s| s == id))
            }
        };
        if !may_write {
            return Ok(());
        }
        let actor = self.provider.actor_context(issue)?;
        if !actor_matches_snapshot(&actor, snapshot) {
            return Ok(());
        }
        validate_feedback_id(&actor.actor_id).map_err(anyhow::Error::msg)?;
        let intent = IssueFeedbackIntent {
            intent_id: format!("{:032x}", rand::random::<u128>()),
            issue: issue.clone(),
            actor_id: actor.actor_id,
            connection: actor.connection,
            automation_policy_version: automation.version.clone(),
            disclosure_policy_version: issue_policy_version(disclosure),
            desired_revision: snapshot.state.desired_revision,
            operation: IssueFeedbackOperation::SetBoardField {
                project_id: board.project_id.clone(),
                item_id: item.item_id.clone(),
                field_id: board.field_id.clone(),
                option_id: target.to_string(),
                from_option_id: current.map(str::to_string),
                origin_option_id: snapshot.state.board_origin(
                    &board.project_id,
                    &item.item_id,
                    &board.field_id,
                    current,
                ),
            },
            recorded_at: format!("unix-ms:{now_unix_ms}"),
        };
        self.record_and_send(snapshot, intent, cancel, lease)?;
        Ok(())
    }

    /// Sent CAS 성공자만 부른다. 보드 쓰기 승인은 이 일지 의도에서만 만든다. 본문 쓰기 승인과 다른 동작이다.
    pub(super) fn write_board_field(
        &self,
        intent: &IssueFeedbackIntent,
    ) -> IssueFeedbackWriteResult {
        let (
            Some(port),
            IssueFeedbackOperation::SetBoardField {
                project_id,
                item_id,
                field_id,
                option_id,
                ..
            },
        ) = (&self.board, &intent.operation)
        else {
            return IssueFeedbackWriteResult::Unconfirmed;
        };
        let request = IssueBoardFieldWriteRequest {
            approval: IssueWriteApproval {
                issue: intent.issue.clone(),
                operation: IssueWriteOperation::BoardField,
            },
            issue: intent.issue.clone(),
            project_id: project_id.clone(),
            item_id: item_id.clone(),
            field_id: field_id.clone(),
            option_id: option_id.clone(),
        };
        match port.write_field(request) {
            Ok(response) if response.item_id == *item_id => {
                IssueFeedbackWriteResult::Acknowledged { comment_id: None }
            }
            Ok(_) => IssueFeedbackWriteResult::Contradictory,
            Err(_) => IssueFeedbackWriteResult::Unconfirmed,
        }
    }

    /// 읽기만 한다. 항목의 값이 목표와 같을 때만 확인이다. 읽기 실패는 결과 모름이다.
    pub(super) fn reconcile_board_field(
        &self,
        intent: &IssueFeedbackIntent,
    ) -> IssueFeedbackReadResult {
        let (
            Some(port),
            IssueFeedbackOperation::SetBoardField {
                project_id,
                item_id,
                field_id,
                option_id,
                origin_option_id,
                ..
            },
        ) = (&self.board, &intent.operation)
        else {
            return IssueFeedbackReadResult::NeedsConfirmation;
        };
        let Ok(items) = port.read_items(IssueBoardItemsReadRequest {
            issue: intent.issue.clone(),
        }) else {
            return IssueFeedbackReadResult::NeedsConfirmation;
        };
        let written = items.issue == intent.issue
            && items
                .items
                .iter()
                .filter(|item| item.project_id == *project_id && item.item_id == *item_id)
                .flat_map(|item| &item.fields)
                .any(|field| {
                    field.field_id == *field_id
                        && field.selected_option_id.as_ref() == Some(option_id)
                });
        if !written {
            return IssueFeedbackReadResult::NeedsConfirmation;
        }
        IssueFeedbackReadResult::Confirmed(Box::new(IssueFeedbackReceipt {
            issue: intent.issue.clone(),
            intent_id: intent.intent_id.clone(),
            actor_id: intent.actor_id.clone(),
            evidence: IssueFeedbackEvidence::BoardField {
                project_id: project_id.clone(),
                item_id: item_id.clone(),
                field_id: field_id.clone(),
                option_id: option_id.clone(),
                origin_option_id: origin_option_id.clone(),
            },
        }))
    }
}

fn has_option(field: &IssueBoardField, option_id: &str) -> bool {
    field
        .options
        .iter()
        .any(|option| option.option_id == option_id)
}
