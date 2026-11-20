use std::fmt;

use anyhow::Result;

use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssueLocator, IssueProjectScope, IssueReference,
    IssueSharedCopy,
};

/*
 * 원격 이슈 스냅샷의 계약이다 (#2420). 공급자가 지금 가진 제목과 본문의 원시 값, 안정 ID, 비교 기준을 읽는다.
 * 읽기만 한다. 구현은 이전 답을 저장하지 않는다. 읽지 못하면 오류이고, 이전 답을 복원 근거로 쓰지 않는다.
 *
 * 원시 값은 다른 사람이 고친 글일 수 있다. 그래서 `IssueRawFields` 는 `Serialize`, `Deserialize`, `Display` 를
 * 구현하지 않고 `Debug` 는 값을 가린다. 신뢰된 처리 경로만 값을 꺼낸다. 일반 도구에는 `verified_shared_copy` 가
 * 확인한 공유본만 준다.
 */

/// 이 계약의 판이다. 요청이나 응답의 뜻이 바뀌면 올린다.
pub const ISSUE_SNAPSHOT_CONTRACT_VERSION: u32 = 1;

/// 공급자가 준 제목과 본문의 필드 값이다. 바꾸지 않은 값이다.
#[derive(Clone, PartialEq, Eq)]
pub struct IssueRawFields {
    title: String,
    body: String,
}

impl IssueRawFields {
    pub fn new(title: String, body: String) -> Self {
        Self { title, body }
    }

    /// 제목의 필드 값을 꺼낸다. 신뢰된 처리 경로만 부른다.
    pub fn expose_title(&self) -> &str {
        &self.title
    }

    /// 본문의 필드 값을 꺼낸다. 신뢰된 처리 경로만 부른다.
    pub fn expose_body(&self) -> &str {
        &self.body
    }

    /// 버전 계산의 입력으로 만든 해시이다. 필드 값을 그대로 쓴다.
    pub fn field_hash(&self) -> IssueFieldHash {
        IssueFieldHash::of_fields(&self.title, &self.body)
    }

    /// 로컬 화면에 보이는 Markdown 이다. 줄 끝을 `\n` 으로 맞춘다. 버전 계산에는 쓰지 않는다.
    /// 원시 값으로 만들므로 신뢰된 처리 경로만 부른다.
    pub fn display_markdown(&self) -> String {
        format!("# {}\n\n{}", self.title, self.body).replace("\r\n", "\n")
    }
}

impl fmt::Debug for IssueRawFields {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("IssueRawFields(<redacted>)")
    }
}

/// 사람이 고른 주소로 안정 ID 를 찾는 요청이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueSnapshotResolveRequest {
    pub locator: IssueLocator,
    pub project_scope: IssueProjectScope,
}

/// 이미 아는 이슈를 안정 ID 로 다시 읽는 요청이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueSnapshotReadRequest {
    pub issue: IssueReference,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueSnapshotResponse {
    pub issue: IssueReference,
    pub raw_fields: IssueRawFields,
    pub comparison_basis: IssueComparisonBasis,
}

impl IssueSnapshotResponse {
    /// 원격 값이 승인한 공유본과 같을 때만 공유본을 준다. `approved` 는 승인한 공유본의 필드 해시이다.
    /// 다르면 다른 사람이 고친 글이다. 그 글은 가림 검증 뒤에만 밖으로 나간다.
    pub fn verified_shared_copy(&self, approved: &IssueFieldHash) -> Option<IssueSharedCopy> {
        (self.comparison_basis.field_hash == *approved).then(|| IssueSharedCopy {
            title: self.raw_fields.title.clone(),
            body: self.raw_fields.body.clone(),
        })
    }
}

pub trait IssueSnapshotPort: Send + Sync {
    /// 구현이 따르는 계약의 판이다.
    fn contract_version(&self) -> u32 {
        ISSUE_SNAPSHOT_CONTRACT_VERSION
    }

    /// 주소가 가리키는 이슈의 안정 ID 와 지금 게시본을 읽는다. PR 과 옮겨진 이슈는 오류이다.
    fn resolve(&self, request: IssueSnapshotResolveRequest) -> Result<IssueSnapshotResponse>;

    /// 아는 이슈의 지금 게시본을 읽는다. 공급자의 답이 다른 저장소, 다른 이슈, PR 이면 오류이다.
    fn read(&self, request: IssueSnapshotReadRequest) -> Result<IssueSnapshotResponse>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::issue_work::IssueProvider;

    fn snapshot(title: &str, body: &str) -> IssueSnapshotResponse {
        let raw_fields = IssueRawFields::new(title.to_string(), body.to_string());
        IssueSnapshotResponse {
            issue: IssueReference {
                provider: IssueProvider::GitHub,
                server_id: "github.com".to_string(),
                repository_id: "R_1".to_string(),
                issue_id: "I_1".to_string(),
                project_scope: IssueProjectScope {
                    workspace_dir: "/work/project".to_string(),
                },
            },
            comparison_basis: IssueComparisonBasis {
                field_hash: raw_fields.field_hash(),
                provider_updated_at: "2026-11-19T07:19:56Z".to_string(),
            },
            raw_fields,
        }
    }

    #[test]
    fn debug_output_hides_raw_field_values() {
        let debug = format!("{:?}", snapshot("raw-title-sentinel", "raw-body-sentinel"));
        assert!(debug.contains("IssueRawFields(<redacted>)"));
        assert!(!debug.contains("sentinel"));
    }

    #[test]
    fn only_a_remote_copy_equal_to_the_approved_copy_becomes_a_shared_copy() {
        let approved = IssueFieldHash::of_fields("title", "masked body");
        assert_eq!(
            snapshot("title", "masked body").verified_shared_copy(&approved),
            Some(IssueSharedCopy {
                title: "title".to_string(),
                body: "masked body".to_string(),
            })
        );
        // 다른 사람이 고친 글은 공유본이 아니다.
        assert_eq!(
            snapshot("title", "masked body and a secret").verified_shared_copy(&approved),
            None
        );
        // 판이 다른 해시는 같은 값으로 보지 않는다.
        let other_version = IssueFieldHash {
            version: approved.version + 1,
            ..approved
        };
        assert_eq!(
            snapshot("title", "masked body").verified_shared_copy(&other_version),
            None
        );
    }

    #[test]
    fn display_markdown_is_not_the_version_input() {
        let crlf = IssueRawFields::new("T".to_string(), "a\r\nb".to_string());
        let lf = IssueRawFields::new("T".to_string(), "a\nb".to_string());
        assert_eq!(crlf.display_markdown(), "# T\n\na\nb");
        assert_eq!(crlf.display_markdown(), lf.display_markdown());
        assert_ne!(crlf.field_hash(), lf.field_hash());
    }
}
