/*
 * Evidence Vault 백업 계약의 로컬 파일 구현이다 (#2432). 보관 파일은 JSON 한 개이다.
 *
 * 보관 파일에는 레코드 파일의 내용(암호문)과 범위, 복구 키로 감싼 장치 키가 있다. 복구 키와 평문은 없다.
 * 감싸는 키는 `ring` 의 HKDF-SHA256 으로 복구 키에서 만든다 (salt 는 보관 파일의 식별자). 장치 키는 `ring` 의
 * ChaCha20-Poly1305 로 감싼다. 머리말과 레코드 목록의 SHA-256 은 연관 데이터이다. 그래서 복구 키가 다르거나
 * 머리말이나 레코드 목록이 바뀌면 장치 키가 풀리지 않는다.
 *
 * 가져오기는 먼저 모든 레코드를 원본 키와 원본 범위로 검증하고, 승인한 프로젝트의 레코드를 로컬 장치 키와 로컬
 * 범위로 다시 암호화한다. 읽을 수 있는 기존 연결은 보존하고, 읽지 못하는 연결은 명시적 승인으로만 복구한다. 이슈의 `CurrentVersion`
 * 레코드는 마지막에 쓴다. 그 전에 멈추면 그 이슈는 연결이 없는 상태로 남고, 다시 부르면 끝난다.
 */
use std::collections::HashSet;
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
    open_record, push_part, record_relative_path, scope_bytes, seal_record,
};
use super::secure_fs;
use crate::application::port::outbound::evidence_vault_backup_port::{
    EvidenceVaultBackupPort, VaultBackupError, VaultBackupExportRequest, VaultBackupImportReport,
    VaultBackupImportRequest, VaultBackupRecoveryMode, VaultRecoveryKey,
};
use crate::application::port::outbound::evidence_vault_port::{
    VaultKey, VaultRecordBinding, VaultRecordField, VaultRecordKey, VaultRecordScope,
};
use crate::domain::issue_work::IssueReference;

/// 보관 파일의 형식의 판이다. 필드, 알고리즘, KDF, 연관 데이터의 구성이 바뀌면 올린다.
const ARCHIVE_FORMAT_VERSION: u32 = 1;
const ARCHIVE_ALGORITHM: &str = "chacha20-poly1305";
const ARCHIVE_KDF: &str = "hkdf-sha256";
const WRAP_ASSOCIATED_DATA_DOMAIN: &str = "akra-evidence-vault-backup";
const WRAP_KEY_INFO: &[u8] = b"akra-evidence-vault-backup-wrap-key-v1";
const BACKUP_ID_LEN: usize = 16;
/// 밖에서 온 파일이 메모리와 디스크를 다 쓰지 못하게 하는 상한이다.
const MAX_ARCHIVE_BYTES: usize = secure_fs::MAX_FILE_BYTES;
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

/// 다시 암호화한 레코드 한 건이다. 아직 쓰지 않았다.
struct StagedRecord {
    issue: IssueReference,
    is_current_version: bool,
    relative_path: PathBuf,
    body: Vec<u8>,
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
        let mut records = Vec::new();
        for scope in &request.scopes {
            let scope_bytes = scope_bytes(scope);
            let Some(record) = secure_fs::read_optional_file(
                &self.vault_root,
                &record_relative_path(&scope_bytes),
            )
            .map_err(storage_error)?
            else {
                if scope.binding == VaultRecordBinding::CurrentVersion {
                    return Err(VaultBackupError::BindingChanged);
                }
                continue;
            };
            // 최초 선택한 판과 실제 담는 연결이 같아야 한다. 연결 파일은 이 한 번 읽은 내용을 담는다.
            let plaintext = open_record(&record, &request.key, &scope_bytes)
                .map_err(VaultBackupError::Record)?;
            if scope.binding == VaultRecordBinding::CurrentVersion
                && [VaultRecordField::Title, VaultRecordField::Body]
                    .iter()
                    .any(|field| {
                        !request.scopes.contains(&VaultRecordScope {
                            issue: scope.issue.clone(),
                            field: *field,
                            binding: VaultRecordBinding::ConfirmedVersion(
                                plaintext.expose().to_string(),
                            ),
                        })
                    })
            {
                return Err(VaultBackupError::BindingChanged);
            }
            records.push(StoredBackupRecord::new(scope, record));
        }
        if records.is_empty() {
            return Err(VaultBackupError::NothingToExport);
        }
        if records.len() > MAX_ARCHIVE_RECORDS {
            return Err(VaultBackupError::ArchiveTooLarge);
        }

        let random = SystemRandom::new();
        let mut backup_id = [0u8; BACKUP_ID_LEN];
        let mut nonce = [0u8; NONCE_LEN];
        random
            .fill(&mut backup_id)
            .and_then(|()| random.fill(&mut nonce))
            .map_err(|_| VaultBackupError::Storage("the OS random source failed".to_string()))?;
        let mut archive = StoredVaultBackup {
            format_version: ARCHIVE_FORMAT_VERSION,
            algorithm: ARCHIVE_ALGORITHM.to_string(),
            kdf: ARCHIVE_KDF.to_string(),
            backup_id: backup_id.iter().map(|byte| format!("{byte:02x}")).collect(),
            key_id: request.key.key_id.clone(),
            wrap_nonce: BASE64.encode(nonce),
            wrapped_key: String::new(),
            records,
        };

        let mut wrapped = request.key.key.expose().to_vec();
        let sealed = wrap_key(&request.recovery_key, &backup_id)?.seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(wrap_associated_data(&archive)),
            &mut wrapped,
        );
        if sealed.is_err() {
            wrapped.zeroize();
            return Err(VaultBackupError::Storage(
                "the device key cannot be wrapped".to_string(),
            ));
        }
        archive.wrapped_key = BASE64.encode(&wrapped);

        let body = serde_json::to_string(&archive).map_err(storage_error)?;
        if body.len() > MAX_ARCHIVE_BYTES {
            return Err(VaultBackupError::ArchiveTooLarge);
        }
        Ok(body)
    }

    fn import(
        &self,
        request: VaultBackupImportRequest,
    ) -> Result<VaultBackupImportReport, VaultBackupError> {
        if request.archive.len() > MAX_ARCHIVE_BYTES {
            return Err(VaultBackupError::ArchiveTooLarge);
        }
        let archive = parse_archive(&request.archive)?;
        let source_key = unwrap_source_key(&archive, &request.recovery_key)?;

        // 쓰기 전에 보관 파일 전체를 검증하고 다시 암호화한다. 평문은 메모리에만 있고 바로 지운다.
        let mut staged = Vec::new();
        let mut unapproved_projects = Vec::new();
        for record in &archive.records {
            let source_scope = record.scope();
            let plaintext = open_record(&record.record, &source_key, &scope_bytes(&source_scope))
                .map_err(|_| VaultBackupError::CorruptArchive)?;
            let source_project = &source_scope.issue.project_scope;
            let Some(approval) = request
                .projects
                .iter()
                .find(|approval| approval.source == *source_project)
            else {
                if !unapproved_projects.contains(source_project) {
                    unapproved_projects.push(source_project.clone());
                }
                continue;
            };
            // 프로젝트만 사용자가 고른 로컬 프로젝트로 바꾼다. 원격 이슈의 ID 는 그대로이다.
            let mut local_scope = source_scope;
            local_scope.issue.project_scope = approval.local.clone();
            let local_scope_bytes = scope_bytes(&local_scope);
            staged.push(StagedRecord {
                is_current_version: local_scope.binding == VaultRecordBinding::CurrentVersion,
                relative_path: record_relative_path(&local_scope_bytes),
                body: seal_record(&request.local_key, &local_scope_bytes, &plaintext)
                    .map_err(|error| VaultBackupError::Storage(error.to_string()))?,
                issue: local_scope.issue,
            });
        }

        // 읽을 수 있는 로컬 연결은 항상 보존한다. 키 분실 등의 복구는 명시적으로 선택한 경우만 허용한다.
        // shortcut: 다중 프로세스의 확인과 쓰기를 한 트랜잭션으로 묶는 저장 계약은 별도이다.
        let mut kept_local = Vec::new();
        let mut recovered = Vec::new();
        for record in staged.iter().filter(|record| record.is_current_version) {
            let Some(current) =
                secure_fs::read_optional_file(&self.vault_root, &record.relative_path)
                    .map_err(storage_error)?
            else {
                continue;
            };
            let scope = scope_bytes(&VaultRecordScope::current_version(record.issue.clone()));
            match open_record(&current, &request.local_key, &scope) {
                Ok(_) => kept_local.push(record.issue.clone()),
                Err(_) if request.recovery_mode == VaultBackupRecoveryMode::RecoverUnreadable => {
                    recovered.push(record.issue.clone());
                }
                Err(_) => return Err(VaultBackupError::LocalRecoveryRequired),
            }
        }
        let kept: HashSet<&IssueReference> = kept_local.iter().collect();

        // 연결(`CurrentVersion`)을 마지막에 쓴다. 그 전에 멈추면 그 이슈는 연결이 없는 상태로 남는다.
        staged.sort_by_key(|record| record.is_current_version);
        let mut imported = Vec::new();
        for record in &staged {
            if kept.contains(&record.issue) {
                continue;
            }
            secure_fs::write_file_atomic(&self.vault_root, &record.relative_path, &record.body)
                .map_err(storage_error)?;
            if record.is_current_version {
                imported.push(record.issue.clone());
            }
        }

        Ok(VaultBackupImportReport {
            backup_id: archive.backup_id,
            imported,
            kept_local,
            recovered,
            unapproved_projects,
        })
    }
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
    if format_version != u64::from(ARCHIVE_FORMAT_VERSION) {
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
