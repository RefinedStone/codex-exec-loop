use std::fmt;

use super::evidence_vault_port::VaultKey;

/*
 * Vault 키를 두는 OS 키 저장소의 계약이다 (#2422). 키는 이 장치의 이 사용자에게 묶인다 (초기 장치 종속 모드).
 * 잠금 해제 서비스(`evidence_vault_unlock`)만 이 계약을 부른다. 암호문 레코드는 이 계약으로 읽거나 바꿀 수 없다.
 */

/// 키 저장소의 오류이다. 어떤 오류도 키를 담지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultKeyStoreError {
    /// 이 플랫폼이나 이 환경에는 OS 키 저장소가 없다.
    Unsupported,
    /// 사용자가 OS 인증을 취소했거나 OS 가 접근을 거부했다.
    AuthenticationCancelled,
    /// 키 저장소가 있지만 요청에 실패했다.
    Unavailable(String),
}

impl fmt::Display for VaultKeyStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => formatter.write_str("this environment has no OS key store"),
            Self::AuthenticationCancelled => {
                formatter.write_str("the OS key store authentication was cancelled or denied")
            }
            Self::Unavailable(message) => write!(formatter, "the OS key store failed: {message}"),
        }
    }
}

impl std::error::Error for VaultKeyStoreError {}

pub trait VaultKeyStorePort: Send + Sync {
    /// 이 장치의 Vault 키를 읽는다. 저장소는 동작하지만 키가 없으면 `None` 이다.
    fn load_key(&self) -> Result<Option<VaultKey>, VaultKeyStoreError>;

    /// 이 장치의 Vault 키를 저장한다. 이미 있는 키는 바꾼다.
    fn store_key(&self, key: &VaultKey) -> Result<(), VaultKeyStoreError>;
}
