use std::fmt;

use zeroize::Zeroize;

use super::evidence_vault_port::{VaultRecordKey, VaultRecordScope, VaultStoreError};
use crate::domain::issue_work::{IssueProjectScope, IssueReference};

/* V2 exports contain only explicitly selected records encrypted under an independent archive key.
 * V1 import remains supported. Only legacy archives wrap the original device key.
 * These request types are not serialized; their key values redact Debug output. */

/// 백업 계약의 판이다. 요청, 응답, 오류의 뜻이 바뀌면 올린다.
pub const EVIDENCE_VAULT_BACKUP_CONTRACT_VERSION: u32 = 5;

/// 복구 키의 길이이다 (256 bit).
pub const VAULT_RECOVERY_KEY_LEN: usize = 32;

/// 사용자가 보관하는 복구 키이다. OS 난수로 만든다. 내보낼 때 한 번만 보여주고 Akra 는 저장하지 않는다.
#[derive(Clone, PartialEq, Eq)]
pub struct VaultRecoveryKey(Vec<u8>);

impl VaultRecoveryKey {
    /// `key` 는 OS 난수 `VAULT_RECOVERY_KEY_LEN` 바이트이다.
    pub fn new(key: Vec<u8>) -> Self {
        Self(key)
    }

    /// 사용자가 입력한 글에서 만든다. `-` 와 공백은 무시한다. 16 진수 64 자가 아니면 `None` 이다.
    pub fn parse(text: &str) -> Option<Self> {
        let mut digits = text
            .chars()
            .filter(|character| *character != '-' && !character.is_whitespace())
            .map(|character| character.to_digit(16).map(|digit| digit as u8));
        let mut key = Vec::with_capacity(VAULT_RECOVERY_KEY_LEN);
        while let Some(high) = digits.next() {
            match (high, digits.next().flatten()) {
                (Some(high), Some(low)) => key.push(high << 4 | low),
                _ => {
                    key.zeroize();
                    return None;
                }
            }
        }
        if key.len() != VAULT_RECOVERY_KEY_LEN {
            key.zeroize();
            return None;
        }
        Some(Self(key))
    }

    /// 키를 꺼낸다. Vault 구현만 부른다.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    /// 사용자에게 한 번 보여주는 글이다: 16 진수를 8 자씩 `-` 로 나눈다. 신뢰된 로컬 UI 만 부른다.
    pub fn expose_text(&self) -> String {
        let hex: Vec<String> = self.0.iter().map(|byte| format!("{byte:02x}")).collect();
        hex.chunks(4)
            .map(|group| group.concat())
            .collect::<Vec<_>>()
            .join("-")
    }
}

impl Drop for VaultRecoveryKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for VaultRecoveryKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VaultRecoveryKey(<redacted>)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultBackupExportRequest {
    /// 내보낼 레코드의 범위이다. 레코드가 없는 범위는 건너뛴다.
    pub scopes: Vec<VaultRecordScope>,
    /// Source-only key: authenticates the selected records. It must not be serialized or wrapped on export.
    pub key: VaultRecordKey,
    pub recovery_key: VaultRecoveryKey,
}

/// 사용자가 승인한 프로젝트 한 개이다: 보관 파일의 프로젝트와, 그 레코드를 받을 이 장치의 프로젝트.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultBackupProjectApproval {
    pub source: IssueProjectScope,
    pub local: IssueProjectScope,
}

/// 기존 연결을 새 키로 열 수 없을 때의 처리이다. 복구는 신뢰된 로컬 UI가 명시적으로 승인한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultBackupRecoveryMode {
    PreserveLocal,
    RecoverUnreadable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultBackupImportRequest {
    /// 보관 파일의 내용이다.
    pub archive: String,
    pub recovery_key: VaultRecoveryKey,
    /// 가져온 레코드를 다시 암호화하는 이 장치의 키이다.
    pub local_key: VaultRecordKey,
    /// 승인하지 않은 프로젝트의 레코드는 쓰지 않는다.
    pub projects: Vec<VaultBackupProjectApproval>,
    pub recovery_mode: VaultBackupRecoveryMode,
}

/// 가져오기의 결과이다. 평문은 없다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultBackupImportReport {
    /// Authenticated archive format; v1 wraps a device key and v2 wraps only an archive key.
    pub format_version: u32,
    /// 보관 파일의 안정 식별자이다.
    pub backup_id: String,
    /// 이 호출이 연결(`CurrentVersion`)을 쓴 이슈이다. 프로젝트는 로컬 프로젝트이다.
    pub imported: Vec<IssueReference>,
    /// 이미 로컬 연결이 있어서 건드리지 않은 이슈이다.
    pub kept_local: Vec<IssueReference>,
    /// 명시적 복구 승인으로 읽을 수 없는 로컬 연결을 교체한 이슈이다.
    pub recovered: Vec<IssueReference>,
    /// 보관 파일에 있지만 승인이 없어서 쓰지 않은 프로젝트이다.
    pub unapproved_projects: Vec<IssueProjectScope>,
}

/// 백업 계약의 오류이다. 어떤 오류도 평문이나 키를 담지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultBackupError {
    /// 내보낼 레코드가 없다.
    NothingToExport,
    /// 범위를 고른 뒤 현재 연결이 바뀌었다. 새 범위로 다시 내보내야 한다.
    BindingChanged,
    /// 기존 연결을 현재 키로 열 수 없다. 명시적인 백업 복구 승인이 필요하다.
    LocalRecoveryRequired,
    /// 보관 파일이 크기나 레코드 수의 상한을 넘는다.
    ArchiveTooLarge,
    /// 보관 파일을 해석할 수 없거나, 안의 레코드가 검증에 실패했다.
    CorruptArchive,
    /// 보관 파일의 형식을 이 구현이 모른다.
    UnsupportedFormat,
    /// 복구 키로 장치 키를 풀지 못했다: 복구 키가 다르거나, 머리말이나 레코드 목록이 바뀌었다.
    RecoveryKeyRejected,
    /// 로컬 레코드 검증 또는 이슈별 binding commit이 실패했다. 교체 후 실패는 WriteOutcomeUnknown이다.
    Record(VaultStoreError),
    /// 저장소를 읽거나 쓰지 못했다. 앞서 끝난 이슈의 연결은 유지하며 다시 시도할 수 있다.
    Storage(String),
}

impl fmt::Display for VaultBackupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NothingToExport => formatter.write_str("the vault has no record to export"),
            Self::BindingChanged => formatter.write_str("the vault binding changed; retry the export"),
            Self::LocalRecoveryRequired => formatter.write_str(
                "the local vault binding is unreadable; explicit backup recovery approval is required",
            ),
            Self::ArchiveTooLarge => formatter.write_str("the vault backup exceeds the size limit"),
            Self::CorruptArchive => formatter.write_str("the vault backup is corrupt"),
            Self::UnsupportedFormat => {
                formatter.write_str("the vault backup uses an unsupported format")
            }
            Self::RecoveryKeyRejected => {
                formatter.write_str("the recovery key does not open this vault backup")
            }
            Self::Record(error) => write!(formatter, "a vault record cannot be exported: {error}"),
            Self::Storage(message) => write!(formatter, "the vault storage failed: {message}"),
        }
    }
}

impl std::error::Error for VaultBackupError {}

/// Single-use installation of an authenticated selection.
pub type VaultBackupInstallation<'a> =
    Box<dyn FnOnce(&VaultRecordKey) -> Result<VaultBackupImportReport, VaultBackupError> + 'a>;

/// Authenticated outcome before attempting to obtain (and possibly initialize) the destination key.
pub enum VaultBackupImportPlan<'a> {
    /// No approved issue will be written. Project selection never needs local credentials.
    ReadOnly(VaultBackupImportReport),
    /// Commits only the verified, approved records; cannot be replayed with different archive bytes.
    Install(VaultBackupInstallation<'a>),
}

/// Immutable archive bytes and the user's source-to-destination decisions.
pub struct VaultBackupSelection<'a> {
    pub archive: &'a str,
    pub recovery_key: &'a VaultRecoveryKey,
    pub projects: &'a [VaultBackupProjectApproval],
    pub recovery_mode: VaultBackupRecoveryMode,
}

pub trait EvidenceVaultBackupPort: Send + Sync {
    /// 구현이 따르는 계약의 판이다.
    fn contract_version(&self) -> u32 {
        EVIDENCE_VAULT_BACKUP_CONTRACT_VERSION
    }

    /// Reseal the selected records under a fresh archive-only key. Never wrap the source device key.
    fn export(&self, request: VaultBackupExportRequest) -> Result<String, VaultBackupError>;

    /// Authenticate and stage all records without creating a destination key or writing to the vault.
    fn prepare_import(
        &self,
        selection: VaultBackupSelection<'_>,
    ) -> Result<VaultBackupImportPlan<'_>, VaultBackupError>;

    /// Import with a key the caller already owns. Uses the same authenticated plan as first-time recovery.
    fn import(
        &self,
        request: VaultBackupImportRequest,
    ) -> Result<VaultBackupImportReport, VaultBackupError> {
        match self.prepare_import(VaultBackupSelection {
            archive: &request.archive,
            recovery_key: &request.recovery_key,
            projects: &request.projects,
            recovery_mode: request.recovery_mode,
        })? {
            VaultBackupImportPlan::ReadOnly(report) => Ok(report),
            VaultBackupImportPlan::Install(install) => install(&request.local_key),
        }
    }
}
