use anyhow::{Result, bail};

use crate::domain::issue_work::{
    IssueReference, IssueSharedCopy, IssueWriteApproval, IssueWriteOperation,
};

/*
 * 기존 이슈의 제목과 본문을 고치는 계약이다 (#2423). 이슈 한 건의 제목과 본문만 쓴다. 새 이슈, 댓글, 첨부,
 * 보드 필드는 이 계약에 없다.
 *
 * 이 계약은 조건 없는 쓰기이다. 공급자(GitHub)는 "원격이 이 값일 때만 고친다"는 조건을 받지 않는다. 그래서
 * 부르는 쪽(`IssuePublishService`)이 쓰기 직전에 원격을 읽어도, 그 읽기와 이 쓰기 사이에 다른 사람이 고친 글은
 * 덮어쓴다. 이 경쟁은 남는다.
 *
 * 오류는 "쓰지 않았다"는 뜻이 아니다. 요청이 공급자에 닿은 뒤 답을 잃으면 (시간 초과, 연결 끊김) 쓰기가 되었는지
 * 모른다. 부르는 쪽은 다시 쓰기 전에 원격을 읽어서 확인한다.
 */

/// 이 계약의 판이다. 요청이나 응답의 뜻이 바뀌면 올린다.
pub const ISSUE_UPDATE_CONTRACT_VERSION: u32 = 1;

/// 이슈 한 건의 제목과 본문을 공유본으로 바꾸는 요청이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueUpdateRequest {
    pub approval: IssueWriteApproval,
    pub issue: IssueReference,
    /// 공급자에 쓸 공유본이다. 가린 뒤의 글이다.
    pub shared_copy: IssueSharedCopy,
}

impl IssueUpdateRequest {
    /// 승인이 이 이슈의 본문 쓰기인지 본다. 구현은 원격을 부르기 전에 이 검사를 한다.
    pub fn ensure_authorized(&self) -> Result<()> {
        if !self
            .approval
            .authorizes(&self.issue, IssueWriteOperation::Body)
        {
            bail!("the approval does not allow a title and body write on this issue")
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueUpdateResponse {
    pub issue: IssueReference,
    /// 쓰기의 답으로 공급자가 준 제목과 본문이다. 공급자가 값을 다듬으면 보낸 글과 다를 수 있다.
    pub published_copy: IssueSharedCopy,
}

pub trait IssueUpdatePort: Send + Sync {
    /// 구현이 따르는 계약의 판이다.
    fn contract_version(&self) -> u32 {
        ISSUE_UPDATE_CONTRACT_VERSION
    }

    /// 이슈의 제목과 본문을 고친다. 승인이 맞지 않으면 원격을 부르지 않고 오류를 준다.
    fn update_title_and_body(&self, request: IssueUpdateRequest) -> Result<IssueUpdateResponse>;
}
