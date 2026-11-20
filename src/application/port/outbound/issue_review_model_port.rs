use anyhow::Result;

/*
 * 이슈 초안 품질 검토에 쓰는 모델의 경계이다 (#2430). application service 가 지시문과 입력을 만들고 답을
 * 해석한다. 모델을 부르는 방법은 outbound adapter 가 맡는다.
 *
 * 요청에는 검증된 공유 초안만 있다. 구현은 이 요청 밖의 이슈 글, Vault 값, 복원본을 모델에 주지 않는다.
 */

/// 모델 한 번의 검토 요청이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueReviewModelRequest {
    /// 검토 스킬의 지시문이다.
    pub instructions: String,
    /// 검증된 공유 초안의 JSON 이다 (`title`, `body`).
    pub draft_json: String,
}

pub trait IssueReviewModelPort: Send + Sync {
    /// 모델의 답 글이다. 모델을 부르지 못하면 오류이다.
    fn review(&self, request: IssueReviewModelRequest) -> Result<String>;
}
