/*
 * 이슈 작업의 값 타입이다 (#2414, #2415). 외부 이슈 한 건을 가리키는 참조와, 밖으로 나가도 되는 공유본이다.
 *
 * 공유본은 가린 뒤의 글이다. 가린 구간의 평문과 키는 이 모듈에 없다. 그 값은 Evidence Vault 의 복원 계약
 * (`evidence_vault_port`)에만 있다.
 */
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 이슈를 가진 서비스의 종류이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueProvider {
    GitHub,
}

/// 이슈를 쓰는 로컬 프로젝트이다. 같은 이슈도 프로젝트가 다르면 규약, 정책, Vault 가 다르다.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueProjectScope {
    /// 저장소 커널의 작업 폴더이다 (저장소의 대표 checkout). 절대 경로이다.
    pub workspace_dir: String,
}

/// 외부 이슈 한 건의 참조이다. ID 는 공급자가 준 안정 ID 이다. 이름이나 번호가 바뀌어도 값이 같다.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueReference {
    pub provider: IssueProvider,
    /// 공급자의 서버이다. 같은 공급자의 다른 설치(예: 직접 운영하는 서버)를 구분한다.
    pub server_id: String,
    pub repository_id: String,
    pub issue_id: String,
    pub project_scope: IssueProjectScope,
}

/// 이슈의 공유본이다. 가린 뒤의 제목과 본문이다. 일반 Kernel 이벤트와 Gateway 응답에 넣어도 된다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueSharedCopy {
    pub title: String,
    pub body: String,
}

/// 사람이 고르는 이슈의 주소이다. 이름과 번호는 바뀔 수 있다. 안정 ID 를 찾을 때만 쓴다.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueLocator {
    pub owner: String,
    pub repository: String,
    pub number: u64,
}

/// 필드 해시 표현의 판이다. `issue_field_hash_input` 의 바이트가 바뀌면 올린다.
pub const ISSUE_FIELD_HASH_VERSION: u32 = 1;

/// 제목과 본문의 필드 값으로 만든 해시이다. 판이 다른 해시는 서로 다른 값이다.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueFieldHash {
    pub version: u32,
    /// SHA-256 의 소문자 16진수이다.
    pub sha256: String,
}

impl IssueFieldHash {
    pub fn of_fields(title: &str, body: &str) -> Self {
        Self {
            version: ISSUE_FIELD_HASH_VERSION,
            sha256: format!("{:x}", Sha256::digest(issue_field_hash_input(title, body))),
        }
    }
}

/// 해시의 입력이다 (1판): 머리글, 그다음 제목과 본문 각각의 바이트 길이(8바이트 big-endian)와 UTF-8 바이트이다.
/// 필드 값을 그대로 쓴다. 다듬기, 줄 끝 바꾸기, Markdown 표시 변환을 하지 않는다.
pub fn issue_field_hash_input(title: &str, body: &str) -> Vec<u8> {
    let mut input = b"akra.issue.fields.v1\n".to_vec();
    for field in [title, body] {
        input.extend_from_slice(&(field.len() as u64).to_be_bytes());
        input.extend_from_slice(field.as_bytes());
    }
    input
}

/// 게시 전후를 비교하는 기준이다. 필드 해시가 같으면 제목과 본문이 같다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueComparisonBasis {
    pub field_hash: IssueFieldHash,
    /// 공급자가 준 마지막 변경 시각이다. 댓글과 라벨도 이 값을 바꾼다.
    pub provider_updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_hash_representation_and_version_are_pinned() {
        assert_eq!(ISSUE_FIELD_HASH_VERSION, 1);
        assert_eq!(
            issue_field_hash_input("T", "a\r\nb"),
            b"akra.issue.fields.v1\n\0\0\0\0\0\0\0\x01T\0\0\0\0\0\0\0\x04a\r\nb"
        );
        assert_eq!(
            IssueFieldHash::of_fields("T", "a\r\nb"),
            IssueFieldHash {
                version: 1,
                sha256: "ef7c239a3f4e3371b0067a465e6390027105fdf0fe3dae19f6bfa129d653f0ac"
                    .to_string(),
            }
        );
        assert_eq!(
            IssueFieldHash::of_fields("", "").sha256,
            "288df23891fd8aa7b05789427511ccef781fbc92462087ab6742a51eac8f5f6c"
        );
    }

    #[test]
    fn field_hash_uses_exact_field_values() {
        let hash = IssueFieldHash::of_fields("T", "a\r\nb");
        // 줄 끝, 공백, 필드 경계가 다르면 해시가 다르다.
        assert_ne!(hash, IssueFieldHash::of_fields("T", "a\nb"));
        assert_ne!(hash, IssueFieldHash::of_fields("T", "a\r\nb\n"));
        assert_ne!(
            IssueFieldHash::of_fields("ab", "c"),
            IssueFieldHash::of_fields("a", "bc")
        );
    }
}
