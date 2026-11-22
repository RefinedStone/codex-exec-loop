//! 이슈별 복원 연결의 원자 저장 계약. 평문과 키에는 전송 형식이 없다.
use ring::hmac;

use super::{
    VaultPlaintext, VaultRecordBinding, VaultRecordField, VaultRecordKey, VaultRecordScope,
    VaultStoreError,
};
use crate::domain::issue_work::IssueProvider;

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
