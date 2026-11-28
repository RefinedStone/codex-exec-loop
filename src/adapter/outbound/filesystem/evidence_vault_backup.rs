//! Evidence Vault archive import and export. V2 reseals each selected record under an independent
//! archive key. V1 is read-only compatibility for archives wrapping a device key.
mod export;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::hkdf;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use super::evidence_vault_store::{
    FilesystemEvidenceVaultStoreAdapter, open_record, push_part, record_relative_path, scope_bytes,
};
use super::vault_fs;
use crate::application::port::outbound::evidence_vault_backup_port::{
    EvidenceVaultBackupPort, VaultBackupError, VaultBackupExportRequest, VaultBackupImportPlan,
    VaultBackupImportReport, VaultBackupProjectApproval, VaultBackupRecoveryMode,
    VaultBackupSelection, VaultRecoveryKey,
};
use crate::application::port::outbound::evidence_vault_port::{
    EvidenceVaultStorePort, VaultBindingCommitPolicy, VaultBindingCommitRequest,
    VaultBindingCommitResult, VaultBindingFields, VaultBindingState, VaultKey, VaultPlaintext,
    VaultRecordBinding, VaultRecordField, VaultRecordKey, VaultRecordLoadRequest, VaultRecordScope,
};
use crate::domain::issue_work::{IssueProjectScope, IssueReference};

/// 보관 파일의 형식의 판이다. 필드, 알고리즘, KDF, 연관 데이터의 구성이 바뀌면 올린다.
const ARCHIVE_FORMAT_VERSION: u32 = 2;
const ARCHIVE_ALGORITHM: &str = "chacha20-poly1305";
const ARCHIVE_KDF: &str = "hkdf-sha256";
const WRAP_ASSOCIATED_DATA_DOMAIN: &str = "akra-evidence-vault-backup";
const WRAP_KEY_INFO: &[u8] = b"akra-evidence-vault-backup-wrap-key-v1";
const BACKUP_ID_LEN: usize = 16;
/// 밖에서 온 파일이 메모리와 디스크를 다 쓰지 못하게 하는 상한이다.
const MAX_ARCHIVE_BYTES: usize = vault_fs::MAX_FILE_BYTES;
const MAX_ARCHIVE_RECORDS: usize = 4_096;

/// 보관 파일의 내용이다. 복구 키와 평문은 없다.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredVaultBackup {
    format_version: u32,
    algorithm: String,
    kdf: String,
    /// 16 진수이다. 보관 파일의 안정 식별자이고 HKDF 의 salt 이다.
    backup_id: String,
    /// 레코드를 암호화한 장치 키의 식별자이다.
    key_id: String,
    /// base64 이다.
    wrap_nonce: String,
    /// base64 이다. 감싼 장치 키 뒤에 인증 태그가 붙어 있다.
    wrapped_key: String,
    records: Vec<StoredBackupRecord>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredBackupRecord {
    issue: IssueReference,
    field: StoredBackupField,
    binding: StoredBackupBinding,
    /// 레코드 파일의 내용 그대로이다 (암호문).
    record: String,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum StoredBackupField {
    Title,
    Body,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum StoredBackupBinding {
    PublishIntent(String),
    ConfirmedVersion(String),
    CurrentVersion,
}

impl StoredBackupRecord {
    fn new(scope: &VaultRecordScope, record: String) -> Self {
        Self {
            issue: scope.issue.clone(),
            field: match scope.field {
                VaultRecordField::Title => StoredBackupField::Title,
                VaultRecordField::Body => StoredBackupField::Body,
            },
            binding: match &scope.binding {
                VaultRecordBinding::PublishIntent(intent_id) => {
                    StoredBackupBinding::PublishIntent(intent_id.clone())
                }
                VaultRecordBinding::ConfirmedVersion(version) => {
                    StoredBackupBinding::ConfirmedVersion(version.clone())
                }
                VaultRecordBinding::CurrentVersion => StoredBackupBinding::CurrentVersion,
            },
            record,
        }
    }

    fn scope(&self) -> VaultRecordScope {
        VaultRecordScope {
            issue: self.issue.clone(),
            field: match self.field {
                StoredBackupField::Title => VaultRecordField::Title,
                StoredBackupField::Body => VaultRecordField::Body,
            },
            binding: match &self.binding {
                StoredBackupBinding::PublishIntent(intent_id) => {
                    VaultRecordBinding::PublishIntent(intent_id.clone())
                }
                StoredBackupBinding::ConfirmedVersion(version) => {
                    VaultRecordBinding::ConfirmedVersion(version.clone())
                }
                StoredBackupBinding::CurrentVersion => VaultRecordBinding::CurrentVersion,
            },
        }
    }
}

/// Fully authenticated archive data for one issue. Only its current field set can
/// become visible; unrelated historical records are not installed.
struct StagedBinding {
    issue: IssueReference,
    base_version: String,
    fields: VaultBindingFields,
}

struct StagedIssueRecords {
    issue: IssueReference,
    current_version: Option<String>,
    records: Vec<(VaultRecordField, VaultRecordBinding, VaultPlaintext)>,
}

#[derive(Default)]
struct ImportOutcomes {
    imported: Vec<IssueReference>,
    kept_local: Vec<IssueReference>,
    recovered: Vec<IssueReference>,
}

pub struct FilesystemEvidenceVaultBackupAdapter {
    vault_root: PathBuf,
}

impl FilesystemEvidenceVaultBackupAdapter {
    /// `vault_root` 는 `FilesystemEvidenceVaultStoreAdapter` 와 같은 폴더이다.
    pub fn new(vault_root: PathBuf) -> Self {
        Self { vault_root }
    }
}

impl EvidenceVaultBackupPort for FilesystemEvidenceVaultBackupAdapter {
    fn export(&self, request: VaultBackupExportRequest) -> Result<String, VaultBackupError> {
        export::selected(&self.vault_root, request)
    }

    fn prepare_import(
        &self,
        request: VaultBackupSelection<'_>,
    ) -> Result<VaultBackupImportPlan<'_>, VaultBackupError> {
        let (archive, staged, unapproved_projects) =
            authenticated_records(request.archive, request.recovery_key, request.projects)?;
        let mut report = VaultBackupImportReport {
            format_version: archive.format_version,
            backup_id: archive.backup_id.clone(),
            imported: Vec::new(),
            kept_local: Vec::new(),
            recovered: Vec::new(),
            unapproved_projects,
        };
        if staged.is_empty() {
            return Ok(VaultBackupImportPlan::ReadOnly(report));
        }
        let approval_digest = Sha256::digest(wrap_associated_data(&archive)).into();
        let recovery_mode = request.recovery_mode;
        Ok(VaultBackupImportPlan::Install(Box::new(move |local_key| {
            let store = FilesystemEvidenceVaultStoreAdapter::new(self.vault_root.clone());
            let outcomes = commit_import_bindings(
                &store,
                local_key,
                &report.backup_id,
                approval_digest,
                recovery_mode,
                staged,
            )?;
            report.imported = outcomes.imported;
            report.kept_local = outcomes.kept_local;
            report.recovered = outcomes.recovered;
            Ok(report)
        })))
    }
}

fn authenticated_records(
    bytes: &str,
    key: &VaultRecoveryKey,
    projects: &[VaultBackupProjectApproval],
) -> Result<
    (
        StoredVaultBackup,
        Vec<StagedBinding>,
        Vec<IssueProjectScope>,
    ),
    VaultBackupError,
> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(VaultBackupError::ArchiveTooLarge);
    }
    let archive = parse_archive(bytes)?;
    let record_key = unwrap_source_key(&archive, key)?;
    let (staged, unapproved) = stage_import(&archive, &record_key, projects)?;
    Ok((archive, staged, unapproved))
}

/// Authenticates records without resolving a destination path, reading credentials, or installing anything.
pub(crate) fn inspect_archive(
    bytes: &str,
    key: &VaultRecoveryKey,
) -> Result<(u32, Vec<IssueProjectScope>), VaultBackupError> {
    let (archive, _, projects) = authenticated_records(bytes, key, &[])?;
    Ok((archive.format_version, projects))
}

fn stage_import(
    archive: &StoredVaultBackup,
    source_key: &VaultRecordKey,
    projects: &[VaultBackupProjectApproval],
) -> Result<(Vec<StagedBinding>, Vec<IssueProjectScope>), VaultBackupError> {
    let mut issues = Vec::<StagedIssueRecords>::new();
    let mut issue_indexes = HashMap::new();
    let mut scopes = HashSet::new();
    let mut unapproved_projects = Vec::new();
    for record in &archive.records {
        let source_scope = record.scope();
        let plaintext = open_record(&record.record, source_key, &scope_bytes(&source_scope))
            .map_err(|_| VaultBackupError::CorruptArchive)?;
        let source_project = &source_scope.issue.project_scope;
        let approvals: Vec<_> = projects
            .iter()
            .filter(|approval| approval.source == *source_project)
            .collect();
        let Some(approval) = approvals.first() else {
            if !unapproved_projects.contains(source_project) {
                unapproved_projects.push(source_project.clone());
            }
            continue;
        };
        if approvals.iter().any(|other| other.local != approval.local) {
            return Err(VaultBackupError::CorruptArchive);
        }
        let mut scope = source_scope;
        scope.issue.project_scope = approval.local.clone();
        if !scopes.insert(scope.clone()) {
            // Colliding source-to-local mappings and duplicate records must never
            // pick an arbitrary pointer or field by archive order.
            return Err(VaultBackupError::CorruptArchive);
        }
        let next_index = issues.len();
        let index = *issue_indexes
            .entry(scope.issue.clone())
            .or_insert(next_index);
        if index == next_index {
            issues.push(StagedIssueRecords {
                issue: scope.issue.clone(),
                current_version: None,
                records: Vec::new(),
            });
        }
        let issue = &mut issues[index];
        if scope.binding == VaultRecordBinding::CurrentVersion {
            if scope.field != VaultRecordField::Body
                || issue.current_version.is_some()
                || plaintext.expose().is_empty()
            {
                return Err(VaultBackupError::CorruptArchive);
            }
            issue.current_version = Some(plaintext.expose().to_string());
        } else {
            issue.records.push((scope.field, scope.binding, plaintext));
        }
    }

    let mut staged = Vec::with_capacity(issues.len());
    for issue in issues {
        let version = issue
            .current_version
            .ok_or(VaultBackupError::CorruptArchive)?;
        let mut fields = VaultBindingFields::default();
        for (field, binding, plaintext) in issue.records {
            if binding != VaultRecordBinding::ConfirmedVersion(version.clone()) {
                continue;
            }
            match field {
                VaultRecordField::Title => fields.title = Some(plaintext),
                VaultRecordField::Body => fields.body = Some(plaintext),
            }
        }
        staged.push(StagedBinding {
            issue: issue.issue,
            base_version: version,
            fields,
        });
    }
    Ok((staged, unapproved_projects))
}

fn commit_import_bindings(
    store: &dyn EvidenceVaultStorePort,
    local_key: &VaultRecordKey,
    backup_id: &str,
    approval_digest: [u8; 32],
    recovery_mode: VaultBackupRecoveryMode,
    staged: Vec<StagedBinding>,
) -> Result<ImportOutcomes, VaultBackupError> {
    let mut outcomes = ImportOutcomes::default();
    for binding in staged {
        let transaction = store
            .begin_binding(VaultRecordLoadRequest {
                scope: VaultRecordScope::current_version(binding.issue.clone()),
                key: local_key.clone(),
            })
            .map_err(VaultBackupError::Record)?;
        let policy = match &transaction.snapshot().state {
            VaultBindingState::Readable { .. } => {
                outcomes.kept_local.push(binding.issue);
                continue;
            }
            VaultBindingState::Missing => VaultBindingCommitPolicy::InstallIfMissing,
            VaultBindingState::Unreadable { .. }
                if recovery_mode == VaultBackupRecoveryMode::RecoverUnreadable =>
            {
                VaultBindingCommitPolicy::RecoverUnreadable
            }
            VaultBindingState::Unreadable { .. } => {
                return Err(VaultBackupError::LocalRecoveryRequired);
            }
        };
        let expected = transaction.snapshot().token.clone();
        match transaction
            .commit(VaultBindingCommitRequest {
                expected,
                operation_id: format!("backup:{backup_id}"),
                base_version: binding.base_version,
                approval_digest,
                fields: binding.fields,
                policy,
            })
            .map_err(VaultBackupError::Record)?
        {
            VaultBindingCommitResult::Committed(_)
            | VaultBindingCommitResult::AlreadyCommitted(_) => {
                if policy == VaultBindingCommitPolicy::RecoverUnreadable {
                    outcomes.recovered.push(binding.issue.clone());
                }
                outcomes.imported.push(binding.issue);
            }
            VaultBindingCommitResult::Conflict => return Err(VaultBackupError::BindingChanged),
        }
    }
    Ok(outcomes)
}

fn storage_error(error: impl std::fmt::Display) -> VaultBackupError {
    VaultBackupError::Storage(error.to_string())
}

/// 판을 먼저 본다. 다른 판의 보관 파일은 필드가 달라도 `UnsupportedFormat` 으로 알린다.
fn parse_archive(body: &str) -> Result<StoredVaultBackup, VaultBackupError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|_| VaultBackupError::CorruptArchive)?;
    let format_version = value
        .get("formatVersion")
        .and_then(serde_json::Value::as_u64)
        .ok_or(VaultBackupError::CorruptArchive)?;
    if format_version != 1 && format_version != u64::from(ARCHIVE_FORMAT_VERSION) {
        return Err(VaultBackupError::UnsupportedFormat);
    }
    let archive: StoredVaultBackup =
        serde_json::from_value(value).map_err(|_| VaultBackupError::CorruptArchive)?;
    if archive.algorithm != ARCHIVE_ALGORITHM || archive.kdf != ARCHIVE_KDF {
        return Err(VaultBackupError::UnsupportedFormat);
    }
    if archive.records.len() > MAX_ARCHIVE_RECORDS {
        return Err(VaultBackupError::ArchiveTooLarge);
    }
    Ok(archive)
}

/// 복구 키에서 장치 키를 감싸는 키를 만든다. 복구 키는 OS 난수 256 bit 이므로 비용이 드는 KDF 는 쓰지 않는다.
fn wrap_key(
    recovery_key: &VaultRecoveryKey,
    backup_id: &[u8],
) -> Result<LessSafeKey, VaultBackupError> {
    hkdf::Salt::new(hkdf::HKDF_SHA256, backup_id)
        .extract(recovery_key.expose())
        .expand(&[WRAP_KEY_INFO], &CHACHA20_POLY1305)
        .map(|okm| LessSafeKey::new(UnboundKey::from(okm)))
        .map_err(|_| VaultBackupError::Storage("the wrap key cannot be derived".to_string()))
}

/// 감싼 장치 키에 묶는 값이다: 머리말과, 순서대로 놓은 레코드 목록의 SHA-256 이다.
fn wrap_associated_data(archive: &StoredVaultBackup) -> Vec<u8> {
    let mut records = Sha256::new();
    for record in &archive.records {
        let scope = scope_bytes(&record.scope());
        records.update((scope.len() as u64).to_be_bytes());
        records.update(&scope);
        records.update((record.record.len() as u64).to_be_bytes());
        records.update(record.record.as_bytes());
    }

    let mut bytes = Vec::new();
    push_part(&mut bytes, WRAP_ASSOCIATED_DATA_DOMAIN);
    bytes.extend_from_slice(&archive.format_version.to_be_bytes());
    push_part(&mut bytes, &archive.algorithm);
    push_part(&mut bytes, &archive.kdf);
    push_part(&mut bytes, &archive.backup_id);
    push_part(&mut bytes, &archive.key_id);
    bytes.extend_from_slice(&records.finalize());
    bytes
}

fn unwrap_source_key(
    archive: &StoredVaultBackup,
    recovery_key: &VaultRecoveryKey,
) -> Result<VaultRecordKey, VaultBackupError> {
    let backup_id = decode_hex(&archive.backup_id).ok_or(VaultBackupError::CorruptArchive)?;
    let nonce = BASE64
        .decode(&archive.wrap_nonce)
        .ok()
        .and_then(|nonce| Nonce::try_assume_unique_for_key(&nonce).ok())
        .ok_or(VaultBackupError::CorruptArchive)?;
    let mut wrapped = BASE64
        .decode(&archive.wrapped_key)
        .map_err(|_| VaultBackupError::CorruptArchive)?;

    let key_len = match wrap_key(recovery_key, &backup_id)?.open_in_place(
        nonce,
        Aad::from(wrap_associated_data(archive)),
        &mut wrapped,
    ) {
        Ok(key) => key.len(),
        Err(_) => {
            wrapped.zeroize();
            return Err(VaultBackupError::RecoveryKeyRejected);
        }
    };
    wrapped.truncate(key_len);
    Ok(VaultRecordKey {
        key_id: archive.key_id.clone(),
        key: VaultKey::new(wrapped),
    })
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() != BACKUP_ID_LEN * 2 || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests;
