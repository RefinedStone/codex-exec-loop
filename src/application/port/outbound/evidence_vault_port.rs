use std::fmt;

use anyhow::Result;
use ring::hmac;
use zeroize::Zeroize;

use crate::domain::issue_work::{IssueProvider, IssueReference};

/*
 * Evidence Vault 의 복원 계약이다 (#2415). 가린 구간의 평문을 돌려주는 유일한 진입점이고, 사용자 확인이 있어야
 * 부를 수 있다. Issue Core 의 일반 계약(`issue_core_port`)과 trait 가 다르므로 일반 도구는 복원을 부를 수 없다.
 *
 * 이 파일의 타입은 `Serialize`, `Deserialize`, `Display` 를 구현하지 않고 `Debug` 는 값을 가린다. Kernel 이벤트와
 * Gateway 응답은 serde 로 만들므로 평문과 키를 담을 수 없다. 평문은 로컬 보호 경로에서만 `expose` 로 꺼낸다.
 */

/// 이 계약의 판이다. 요청이나 응답의 뜻이 바뀌면 올린다.
pub const EVIDENCE_VAULT_CONTRACT_VERSION: u32 = 3;

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
    session_id: u64,
}

impl VaultUserConfirmation {
    /// 사용자가 `issue` 의 복원을 확인한 뒤에만 부른다. application 밖에서는 만들 수 없다: 잠금 해제 서비스
    /// (`evidence_vault_unlock`)가 살아 있는 열람 세션의 권한에서만 만든다 (#2422).
    pub(in crate::application) fn for_issue(issue: IssueReference, session_id: u64) -> Self {
        Self { issue, session_id }
    }

    pub(in crate::application) fn session_id(&self) -> u64 {
        self.session_id
    }

    /// 시험에서만 확인 없이 만든다.
    #[cfg(test)]
    pub(crate) fn for_issue_in_test(issue: IssueReference) -> Self {
        Self {
            issue,
            session_id: 0,
        }
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
    /// 요청의 열람 세션을 시작 전과 평문 반환 직전에 검증한다. 끝난 세션의 요청은 재사용할 수 없다.
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
pub const EVIDENCE_VAULT_STORE_CONTRACT_VERSION: u32 = 4;

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
    /// 교체는 끝났지만 동기화나 사후 검증에 실패했다. 이전 레코드 보존을 가정하지 말고 다시 읽어야 한다.
    WriteOutcomeUnknown,
    /// 저장소를 읽지 못했거나 교체 전에 쓰기가 실패했다.
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
            Self::WriteOutcomeUnknown => formatter.write_str(
                "the vault record was replaced but its outcome is unknown; reload before retrying",
            ),
            Self::Storage(message) => write!(formatter, "the vault storage failed: {message}"),
        }
    }
}

impl std::error::Error for VaultStoreError {}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VaultCreationRecordScope {
    pub repository: crate::domain::issue_creation::IssueRepositoryReference,
    pub intent_id: String,
    pub approved_hash: crate::domain::issue_work::IssueFieldHash,
    pub field: VaultRecordField,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultCreationSaveRequest {
    pub scope: VaultCreationRecordScope,
    pub key: VaultRecordKey,
    pub plaintext: VaultPlaintext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultCreationLoadRequest {
    pub scope: VaultCreationRecordScope,
    pub key: VaultRecordKey,
}

pub trait EvidenceVaultStorePort: Send + Sync {
    /// canonical Body/CurrentVersion 범위에 고정된 프로세스 공유 트랜잭션을 시작한다.
    /// 안전하지 않은 저장소나 경로는 오류이며 Unreadable snapshot으로 완화하지 않는다.
    fn begin_binding(
        &self,
        request: VaultRecordLoadRequest,
    ) -> std::result::Result<Box<dyn VaultBindingTransaction + '_>, VaultStoreError>;

    /// 구현이 따르는 계약의 판이다.
    fn contract_version(&self) -> u32 {
        EVIDENCE_VAULT_STORE_CONTRACT_VERSION
    }

    /// New-issue staging has no fabricated IssueReference. Legacy adapters fail closed.
    fn save_creation(
        &self,
        _request: VaultCreationSaveRequest,
    ) -> std::result::Result<(), VaultStoreError> {
        Err(VaultStoreError::Storage(
            "new-issue vault staging is unavailable".to_string(),
        ))
    }

    fn load_creation(
        &self,
        _request: VaultCreationLoadRequest,
    ) -> std::result::Result<VaultPlaintext, VaultStoreError> {
        Err(VaultStoreError::Storage(
            "new-issue vault staging is unavailable".to_string(),
        ))
    }

    /// PublishIntent staging만 저장한다. CurrentVersion/ConfirmedVersion은 begin_binding으로만 쓴다.
    /// staging 저장은 binding lease를 재취득하지 않아 transaction 안에서도 부를 수 있다.
    /// 교체 전 실패는 이전 레코드를 보존한다. 교체 후 실패는 `WriteOutcomeUnknown`이며 다시 읽어야 한다.
    fn save(&self, request: VaultRecordSaveRequest) -> std::result::Result<(), VaultStoreError>;

    /// 이 범위의 레코드를 검증하고 평문을 돌려준다. 검증에 실패하면 평문을 돌려주지 않는다.
    fn load(
        &self,
        request: VaultRecordLoadRequest,
    ) -> std::result::Result<VaultPlaintext, VaultStoreError>;
}

// 이슈별 복원 연결의 원자 저장 계약. 평문과 키에는 전송 형식이 없다.
/// 빠진 필드는 의도적으로 복원하지 않는 필드이다. 빈 평문과 구분한다.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VaultBindingFields {
    pub title: Option<VaultPlaintext>,
    pub body: Option<VaultPlaintext>,
}

/// 현재 포인터의 실제 암호문 관측값이다. 권한 증명이 아니라 범위가 고정된 CAS 토큰이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultBindingToken {
    scope: VaultRecordScope,
    ciphertext_sha256: Option<[u8; 32]>,
}

impl VaultBindingToken {
    pub fn observed(scope: VaultRecordScope, ciphertext_sha256: Option<[u8; 32]>) -> Self {
        Self {
            scope,
            ciphertext_sha256,
        }
    }

    pub fn scope(&self) -> &VaultRecordScope {
        &self.scope
    }

    pub fn ciphertext_sha256(&self) -> Option<[u8; 32]> {
        self.ciphertext_sha256
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultBindingSnapshot {
    pub token: VaultBindingToken,
    pub state: VaultBindingState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultBindingState {
    Missing,
    Readable {
        version: String,
    },
    /// 포인터 레코드 자체의 복호화/형식 오류만 해당한다. 안전하지 않은 파일 경로는 begin 오류이다.
    Unreadable {
        error: VaultStoreError,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultBindingCommitPolicy {
    /// 검토한 포인터를 교체한다. 없는 포인터는 허용하고 읽지 못하는 포인터는 보존한다.
    ReplaceExpected,
    InstallIfMissing,
    RecoverUnreadable,
}

/// expected와 승인한 전체 bundle을 재시도 동안 그대로 유지해야 한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultBindingCommitRequest {
    pub expected: VaultBindingToken,
    pub operation_id: String,
    pub base_version: String,
    pub approval_digest: [u8; 32],
    pub fields: VaultBindingFields,
    pub policy: VaultBindingCommitPolicy,
}

impl VaultBindingCommitRequest {
    /// 전체 작업을 키에 묶은 불변 generation 이름이다. 파일 이름에서 평문을 추측할 수 있도록
    /// 평문의 일반 해시를 노출하지 않는다. fake와 실제 저장소가 이 함수를 함께 사용한다.
    pub fn generation_version(&self, key: &VaultRecordKey) -> Result<String, VaultStoreError> {
        if key.key.expose().len() != 32 {
            return Err(VaultStoreError::InvalidKey);
        }
        if self.operation_id.is_empty() || self.base_version.is_empty() {
            return Err(VaultStoreError::Storage(
                "binding operation and base version must not be empty".to_string(),
            ));
        }
        let scope = self.expected.scope();
        if scope.field != VaultRecordField::Body
            || scope.binding != VaultRecordBinding::CurrentVersion
        {
            return Err(VaultStoreError::Storage(
                "binding transactions require the canonical current-version scope".to_string(),
            ));
        }

        let signing_key = hmac::Key::new(hmac::HMAC_SHA256, key.key.expose());
        let mut digest = hmac::Context::with_key(&signing_key);
        let issue = &scope.issue;
        let provider = match issue.provider {
            IssueProvider::GitHub => "github",
            IssueProvider::Jira => "jira",
        };
        for part in [
            "akra-evidence-vault-binding-v1",
            issue.project_scope.workspace_dir.as_str(),
            provider,
            issue.server_id.as_str(),
            issue.repository_id.as_str(),
            issue.issue_id.as_str(),
            "body",
            "current-version",
            key.key_id.as_str(),
        ] {
            hash_part(&mut digest, part.as_bytes());
        }
        match self.expected.ciphertext_sha256() {
            None => digest.update(&[0]),
            Some(value) => {
                digest.update(&[1]);
                hash_part(&mut digest, &value);
            }
        }
        hash_part(&mut digest, self.operation_id.as_bytes());
        // 표시용 suffix를 줄이더라도 검토한 원래 판 전체는 작업의 identity에 남는다.
        hash_part(&mut digest, self.base_version.as_bytes());
        hash_part(&mut digest, &self.approval_digest);
        digest.update(&[match self.policy {
            VaultBindingCommitPolicy::ReplaceExpected => 0,
            VaultBindingCommitPolicy::InstallIfMissing => 1,
            VaultBindingCommitPolicy::RecoverUnreadable => 2,
        }]);
        for value in [&self.fields.title, &self.fields.body] {
            match value {
                None => digest.update(&[0]),
                Some(value) => {
                    digest.update(&[1]);
                    hash_part(&mut digest, value.expose().as_bytes());
                }
            }
        }
        let tag = digest.sign();
        let suffix: String = tag
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let base = self
            .base_version
            .rsplit_once(":binding:")
            .filter(|(base, suffix)| {
                !base.is_empty()
                    && !base.contains(":binding:")
                    && suffix.len() == 64
                    && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            .map_or(self.base_version.as_str(), |(base, _)| base);
        Ok(format!("{base}:binding:{suffix}"))
    }
}

fn hash_part(digest: &mut hmac::Context, value: &[u8]) {
    digest.update(&(value.len() as u64).to_be_bytes());
    digest.update(value);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultBindingReceipt {
    pub version: String,
    pub token: VaultBindingToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultBindingCommitResult {
    Committed(VaultBindingReceipt),
    AlreadyCommitted(VaultBindingReceipt),
    Conflict,
}

/// 한 이슈와 키에 고정된 배타적 저장 세션이다. guard를 가진 채 원래 store.load를 재호출하지 않는다.
/// 읽기와 commit은 같은 프로세스 공유 lease 아래 수행하며, guard를 버리면 쓰기 없이 종료한다.
pub trait VaultBindingTransaction {
    fn snapshot(&self) -> &VaultBindingSnapshot;

    /// 승인 당시 source version도 읽을 수 있다. 각 필드의 부재와 빈 평문을 구분한다.
    fn load_version_fields(&mut self, version: &str)
    -> Result<VaultBindingFields, VaultStoreError>;

    /// 같은 전체 작업의 이미 완료된 결과를 CAS 충돌보다 먼저 확인한다. 새 generation을 모두 검증한
    /// 뒤 포인터만 CAS로 교체한다. 포인터 교체 전 실패는 기존 reachable bundle을 보존한다.
    /// 교체 후 동기화/검증 실패는 WriteOutcomeUnknown이며 다시 읽어야 한다.
    fn commit(
        self: Box<Self>,
        request: VaultBindingCommitRequest,
    ) -> Result<VaultBindingCommitResult, VaultStoreError>;
}

#[cfg(test)]
mod tests;
