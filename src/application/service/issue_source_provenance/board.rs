//! 정책의 보드 열 조건을 위해 이슈의 보드 항목 값을 관찰에 붙인다 (#2565).
use super::*;
use crate::application::port::outbound::issue_board_field_port::{
    IssueBoardItem, IssueBoardItemsReadRequest,
};
use crate::domain::issue_automation::feedback::IssueFeedbackEvidence;
use crate::domain::issue_automation::policy::IssueAutomationGitHubBoard;
use crate::domain::issue_automation::source::{IssueSourceBoardColumn, IssueSourceBoardRead};

impl IssueSourceProvenanceService {
    /// 열 조건이 있는 GitHub 정책의 보드이다. 정책을 읽지 못하면 판정이 정책 차단으로 멈추므로 읽지 않는다.
    pub(super) fn board_condition(&self, workspace: &str) -> Option<IssueAutomationGitHubBoard> {
        let (_, policy) = self.board.as_ref()?;
        let resolved = policy.resolve(workspace).ok()?;
        if resolved.policy.binding.jira.is_some() {
            return None;
        }
        resolved
            .policy
            .binding
            .github
            .board
            .filter(|board| !board.select_option_ids.is_empty())
    }

    pub(super) fn apply_board(
        &self,
        observation: &mut IssueSourceObservation,
        board: Option<&IssueAutomationGitHubBoard>,
    ) -> Result<(), IssueSourceFailure> {
        let (Some((port, _)), Some(board)) = (&self.board, board) else {
            return Ok(());
        };
        if observation.issue.provider != IssueProvider::GitHub {
            return Ok(());
        }
        // 읽기 실패는 불일치가 아니다. `read:project` 권한이 없는 로그인도 여기로 온다.
        let mut read = match port.read_items(IssueBoardItemsReadRequest {
            issue: observation.issue.clone(),
        }) {
            Ok(response) if response.issue == observation.issue => {
                column_read(&response.items, board)
            }
            _ => IssueSourceBoardRead::Unavailable,
        };
        self.suppress_owned_board_write(&observation.issue, board, &mut read)?;
        observation.board = Some(IssueSourceBoardColumn {
            project_id: board.project_id.clone(),
            field_id: board.field_id.clone(),
            read,
        });
        Ok(())
    }

    /// 확인된 자기 쓰기가 남긴 값은 쓰기 전 값으로 되돌려 판정에 넣는다. 자기 쓰기로 선택이 바뀌지 않는다.
    fn suppress_owned_board_write(
        &self,
        issue: &IssueReference,
        board: &IssueAutomationGitHubBoard,
        read: &mut IssueSourceBoardRead,
    ) -> Result<(), IssueSourceFailure> {
        let (Some((feedback, _)), IssueSourceBoardRead::Item { item_id, option_id }) =
            (&self.feedback, read)
        else {
            return Ok(());
        };
        let Some(snapshot) = feedback
            .snapshot(issue)
            .map_err(|_| IssueSourceFailure::ProviderUnavailable)?
        else {
            return Ok(());
        };
        if let Some(IssueFeedbackEvidence::BoardField {
            project_id,
            item_id: owned_item,
            field_id,
            option_id: owned_option,
            origin_option_id,
        }) = snapshot
            .state
            .board_proof
            .as_ref()
            .map(|proof| &proof.evidence)
            && *project_id == board.project_id
            && *field_id == board.field_id
            && owned_item == item_id
            && option_id.as_ref() == Some(owned_option)
        {
            *option_id = origin_option_id.clone();
        }
        Ok(())
    }
}

fn column_read(
    items: &[IssueBoardItem],
    board: &IssueAutomationGitHubBoard,
) -> IssueSourceBoardRead {
    let Some(item) = items
        .iter()
        .find(|item| item.project_id == board.project_id)
    else {
        return IssueSourceBoardRead::NotOnBoard;
    };
    // 정책의 필드가 보드에 없으면 설정이나 권한 문제이다. 불일치로 보지 않는다.
    match item
        .fields
        .iter()
        .find(|field| field.field_id == board.field_id)
    {
        Some(field) => IssueSourceBoardRead::Item {
            item_id: item.item_id.clone(),
            option_id: field.selected_option_id.clone(),
        },
        None => IssueSourceBoardRead::Unavailable,
    }
}
