use std::fmt;

use anyhow::Result;

use crate::domain::issue_work::IssueReference;

/*
 * Evidence Vault 의 복원 계약이다 (#2415). 가린 구간의 평문을 돌려주는 유일한 진입점이고, 사용자 확인이 있어야
 * 부를 수 있다. Issue Core 의 일반 계약(`issue_core_port`)과 trait 가 다르므로 일반 도구는 복원을 부를 수 없다.
 *
 * 이 파일의 타입은 `Serialize`, `Deserialize`, `Display` 를 구현하지 않고 `Debug` 는 값을 가린다. Kernel 이벤트와
 * Gateway 응답은 serde 로 만들므로 평문과 키를 담을 수 없다. 평문은 로컬 보호 경로에서만 `expose` 로 꺼낸다.
 */

/// 이 계약의 판이다. 요청이나 응답의 뜻이 바뀌면 올린다.
pub const EVIDENCE_VAULT_CONTRACT_VERSION: u32 = 1;

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
    /// 사용자가 `issue` 의 복원을 확인한 뒤에만 부른다.
    pub fn for_issue(issue: IssueReference) -> Self {
        Self { issue }
    }

    pub fn issue(&self) -> &IssueReference {
        &self.issue
    }
}

/// 복원 요청이다. 확인한 이슈만 복원한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultRestoreRequest {
    pub confirmation: VaultUserConfirmation,
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

    /// 확인한 이슈의 로컬 복원본을 돌려준다. 공급자에 쓰지 않는다.
    fn restore(&self, request: VaultRestoreRequest) -> Result<VaultRestoreResponse>;
}
