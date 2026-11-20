use std::fmt;

use anyhow::Result;
use zeroize::Zeroize;

use crate::domain::issue_work::IssueReference;

/*
 * Evidence Vault 의 복원 계약이다 (#2415). 가린 구간의 평문을 돌려주는 유일한 진입점이고, 사용자 확인이 있어야
 * 부를 수 있다. Issue Core 의 일반 계약(`issue_core_port`)과 trait 가 다르므로 일반 도구는 복원을 부를 수 없다.
 *
 * 이 파일의 타입은 `Serialize`, `Deserialize`, `Display` 를 구현하지 않고 `Debug` 는 값을 가린다. Kernel 이벤트와
 * Gateway 응답은 serde 로 만들므로 평문과 키를 담을 수 없다. 평문은 로컬 보호 경로에서만 `expose` 로 꺼낸다.
 */

/// 이 계약의 판이다. 요청이나 응답의 뜻이 바뀌면 올린다.
pub const EVIDENCE_VAULT_CONTRACT_VERSION: u32 = 2;

/// 복원한 평문이다.
#[derive(Clone, PartialEq, Eq)]
pub struct VaultPlaintext(String);

impl VaultPlaintext {
    pub fn new(plaintext: String) -> Self {
        Self(plaintext)
    }

    /// 평문을 꺼낸다. 로컬 보호 경로만 부른다.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl Drop for VaultPlaintext {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for VaultPlaintext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VaultPlaintext(<redacted>)")
    }
}

/// Vault 의 키 재료이다. 이 계약의 요청과 응답에는 없다. Vault 구현 안에서만 쓴다.
#[derive(Clone, PartialEq, Eq)]
pub struct VaultKey(Vec<u8>);

impl VaultKey {
    pub fn new(key: Vec<u8>) -> Self {
        Self(key)
    }

    /// 키를 꺼낸다. Vault 구현만 부른다.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for VaultKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for VaultKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VaultKey(<redacted>)")
    }
}

/// 사용자가 이 이슈의 복원을 확인했다는 증거이다. 전송 형식에서 만들 수 없다. 로컬 확인 경로만 만든다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultUserConfirmation {
    issue: IssueReference,
}

impl VaultUserConfirmation {
    /// 사용자가 `issue` 의 복원을 확인한 뒤에만 부른다. application 밖에서는 만들 수 없다: 잠금 해제 서비스
    /// (`evidence_vault_unlock`)가 살아 있는 열람 세션의 권한에서만 만든다 (#2422).
    pub(in crate::application) fn for_issue(issue: IssueReference) -> Self {
        Self { issue }
    }

    /// 시험에서만 확인 없이 만든다.
    #[cfg(test)]
    pub(crate) fn for_issue_in_test(issue: IssueReference) -> Self {
        Self { issue }
    }

    pub fn issue(&self) -> &IssueReference {
        &self.issue
    }
}

/// 복원 요청이다. 확인한 이슈만 복원한다. 확인과 키는 살아 있는 잠금 해제 권한에서 함께 나온다
/// (`EvidenceVaultUnlockService::access`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultRestoreRequest {
    pub confirmation: VaultUserConfirmation,
    /// 이 이슈의 레코드를 여는 키이다.
    pub key: VaultRecordKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultRestoreResponse {
    /// 가린 구간을 되돌린 제목이다.
    pub title: VaultPlaintext,
    /// 가린 구간을 되돌린 본문이다.
    pub body: VaultPlaintext,
}

pub trait EvidenceVaultRestorePort: Send + Sync {
    /// 구현이 따르는 계약의 판이다.
    fn contract_version(&self) -> u32 {
        EVIDENCE_VAULT_CONTRACT_VERSION
    }

    /// 확인한 이슈의 로컬 복원본을 돌려준다. 공급자에 쓰지 않는다. 최신 게시본이 저장한 연결과 다르면 오류이다.
    fn restore(&self, request: VaultRestoreRequest) -> Result<VaultRestoreResponse>;
}

/*
 * Evidence Vault 의 저장 계약이다 (#2421). 가린 구간의 평문을 암호화한 레코드로 저장하고 다시 읽는다.
 * 키는 부르는 쪽이 준다. OS 키 저장소와 사용자 잠금 해제는 이 계약 밖이다 (#2422).
 *
 * 레코드는 `VaultRecordScope` 에 묶인다. 범위나 키가 다르거나 레코드가 바뀌었으면 읽기는 실패하고 평문을
 * 돌려주지 않는다.
 */

/// 저장 계약의 판이다. 요청, 응답, 오류의 뜻이 바뀌면 올린다.
pub const EVIDENCE_VAULT_STORE_CONTRACT_VERSION: u32 = 1;

/// 가린 구간이 있는 이슈의 필드이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VaultRecordField {
    Title,
    Body,
}

/// 레코드가 속한 게시 상태이다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VaultRecordBinding {
    /// 아직 확인하지 않은 게시 의도의 ID 이다.
    PublishIntent(String),
    /// 공급자가 확인한 게시본의 판이다.
    ConfirmedVersion(String),
    /// 이 이슈에서 지금 복원에 쓰는 확인된 판의 이름을 담은 레코드이다 (#2425). 이슈마다 하나이다.
    CurrentVersion,
}

/// 레코드 한 건의 범위이다: 로컬 프로젝트와 이슈(`issue`), 필드, 게시 의도 또는 확인된 판이다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VaultRecordScope {
    pub issue: IssueReference,
    pub field: VaultRecordField,
    pub binding: VaultRecordBinding,
}

impl VaultRecordScope {
    /// 이슈의 `CurrentVersion` 레코드의 범위이다. 필드는 뜻이 없으므로 `Body` 로 고정한다.
    pub fn current_version(issue: IssueReference) -> Self {
        Self {
            issue,
            field: VaultRecordField::Body,
            binding: VaultRecordBinding::CurrentVersion,
        }
    }
}

/// 레코드를 암호화하는 키와 그 식별자이다. 식별자는 레코드에 평문으로 남는다. 키는 남지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultRecordKey {
    pub key_id: String,
    pub key: VaultKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultRecordSaveRequest {
    pub scope: VaultRecordScope,
    pub key: VaultRecordKey,
    /// 이 필드의 가린 구간 평문이다.
    pub plaintext: VaultPlaintext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultRecordLoadRequest {
    pub scope: VaultRecordScope,
    pub key: VaultRecordKey,
}

/// 저장 계약의 오류이다. 어떤 오류도 평문이나 키를 담지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultStoreError {
    /// 이 범위의 레코드가 없다.
    RecordNotFound,
    /// 레코드를 해석할 수 없다 (예: 잘린 파일).
    CorruptRecord,
    /// 레코드의 암호화 형식을 이 구현이 모른다.
    UnsupportedFormat,
    /// 레코드를 만든 키의 식별자가 요청의 식별자와 다르다.
    KeyMismatch,
    /// 키의 길이가 암호에 맞지 않는다.
    InvalidKey,
    /// 인증에 실패했다: 암호문이나 머리말이 바뀌었거나, 다른 범위의 레코드이거나, 키가 다르다.
    IntegrityFailure,
    /// 저장소를 읽거나 쓰지 못했다.
    Storage(String),
}

impl fmt::Display for VaultStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RecordNotFound => formatter.write_str("the vault has no record for this scope"),
            Self::CorruptRecord => formatter.write_str("the vault record cannot be parsed"),
            Self::UnsupportedFormat => {
                formatter.write_str("the vault record uses an unsupported encryption format")
            }
            Self::KeyMismatch => {
                formatter.write_str("the vault record was sealed with a different key identifier")
            }
            Self::InvalidKey => formatter.write_str("the vault key has the wrong length"),
            Self::IntegrityFailure => {
                formatter.write_str("the vault record failed its integrity check")
            }
            Self::Storage(message) => write!(formatter, "the vault storage failed: {message}"),
        }
    }
}

impl std::error::Error for VaultStoreError {}

pub trait EvidenceVaultStorePort: Send + Sync {
    /// 구현이 따르는 계약의 판이다.
    fn contract_version(&self) -> u32 {
        EVIDENCE_VAULT_STORE_CONTRACT_VERSION
    }

    /// 평문을 암호화해 이 범위의 레코드로 저장한다. 같은 범위의 레코드는 바꾼다. 실패하면 이전 레코드가 남는다.
    fn save(&self, request: VaultRecordSaveRequest) -> std::result::Result<(), VaultStoreError>;

    /// 이 범위의 레코드를 검증하고 평문을 돌려준다. 검증에 실패하면 평문을 돌려주지 않는다.
    fn load(
        &self,
        request: VaultRecordLoadRequest,
    ) -> std::result::Result<VaultPlaintext, VaultStoreError>;
}
