/*
 * Evidence Vault 의 백업 서비스이다 (#2432). 이슈의 연결 레코드를 보관 파일로 내보내고, 사용자가 보관한 복구 키와
 * 승인으로 다른 장치의 Vault 에 가져온다.
 *
 * 내보내기는 복구 키를 OS 난수로 새로 만들어 한 번만 돌려준다. 이 서비스는 복구 키를 저장하지 않는다.
 * 가져오기는 사용자가 확인했고 복구 키가 맞을 때만 쓴다. 사용자가 승인한 프로젝트의 레코드만, 사용자가 고른
 * 로컬 프로젝트에 쓴다. 원격 이슈는 바꾸지 않는다. 가져온 레코드도 복원 서비스(`evidence_vault_restore`)가 최신
 * 게시본과 맞는지 확인한 뒤에만 평문을 준다.
 *
 * 이 파일의 타입은 `Serialize` 를 구현하지 않는다. Kernel 이벤트와 Gateway 응답에 넣지 않는다.
 */
use std::fmt;
use std::sync::Arc;

use rand::{RngCore, rngs::OsRng};

use crate::application::port::outbound::evidence_vault_backup_port::{
    EvidenceVaultBackupPort, VAULT_RECOVERY_KEY_LEN, VaultBackupError, VaultBackupExportRequest,
    VaultBackupImportReport, VaultBackupImportRequest, VaultBackupProjectApproval,
    VaultRecoveryKey,
};
use crate::application::port::outbound::evidence_vault_port::{
    EvidenceVaultStorePort, VaultRecordBinding, VaultRecordField, VaultRecordLoadRequest,
    VaultRecordScope, VaultStoreError,
};
use crate::application::service::evidence_vault_unlock::{
    EvidenceVaultUnlockService, VaultSharedCopyReason, VaultUserDecision,
};
use crate::domain::issue_work::IssueReference;

/// 내보내기의 결과이다. 보관 파일의 내용에는 복구 키가 없다. 복구 키는 사용자에게 한 번만 보여준다.
#[derive(Debug, PartialEq, Eq)]
pub struct VaultBackupExport {
    pub archive: String,
    pub recovery_key: VaultRecoveryKey,
}

/// 신뢰된 로컬 UI 에서 온 가져오기 요청이다.
#[derive(Debug)]
pub struct VaultBackupApprovedImport {
    /// 보관 파일의 내용이다.
    pub archive: String,
    pub recovery_key: VaultRecoveryKey,
    pub decision: VaultUserDecision,
    /// 사용자가 승인한 프로젝트이다. 여기에 없는 프로젝트의 레코드는 쓰지 않는다.
    pub projects: Vec<VaultBackupProjectApproval>,
}

/// 백업을 못 한 이유이다. 어떤 값도 평문이나 키를 담지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultBackupFailure {
    /// 사용자가 가져오기를 승인하지 않았다. 아무것도 읽거나 쓰지 않았다.
    NotApproved,
    /// 이 장치의 키를 쓸 수 없다.
    Locked(VaultSharedCopyReason),
    Backup(VaultBackupError),
}

impl fmt::Display for VaultBackupFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotApproved => formatter.write_str("the user did not approve the vault import"),
            Self::Locked(reason) => write!(formatter, "the vault is locked: {reason:?}"),
            Self::Backup(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for VaultBackupFailure {}

pub struct EvidenceVaultBackupService {
    vault: Arc<dyn EvidenceVaultStorePort>,
    backup: Arc<dyn EvidenceVaultBackupPort>,
    unlock: Arc<EvidenceVaultUnlockService>,
}

impl EvidenceVaultBackupService {
    pub fn new(
        vault: Arc<dyn EvidenceVaultStorePort>,
        backup: Arc<dyn EvidenceVaultBackupPort>,
        unlock: Arc<EvidenceVaultUnlockService>,
    ) -> Self {
        Self {
            vault,
            backup,
            unlock,
        }
    }

    /// 이 이슈들의 연결(`CurrentVersion`)과 그 판의 레코드를 내보낸다. 연결이 없는 이슈는 건너뛴다.
    /// 평문을 돌려주지 않는다. 게시 의도의 레코드는 내보내지 않는다.
    pub fn export(
        &self,
        issues: &[IssueReference],
    ) -> Result<VaultBackupExport, VaultBackupFailure> {
        let key = self
            .unlock
            .sealing_key()
            .map_err(VaultBackupFailure::Locked)?;
        let mut scopes = Vec::new();
        for issue in issues {
            let current = VaultRecordScope::current_version(issue.clone());
            let version = match self.vault.load(VaultRecordLoadRequest {
                scope: current.clone(),
                key: key.clone(),
            }) {
                Ok(version) => version,
                Err(VaultStoreError::RecordNotFound) => continue,
                Err(error) => {
                    return Err(VaultBackupFailure::Backup(VaultBackupError::Record(error)));
                }
            };
            scopes.push(current);
            for field in [VaultRecordField::Title, VaultRecordField::Body] {
                scopes.push(VaultRecordScope {
                    issue: issue.clone(),
                    field,
                    binding: VaultRecordBinding::ConfirmedVersion(version.expose().to_string()),
                });
            }
        }

        let mut recovery_key = vec![0u8; VAULT_RECOVERY_KEY_LEN];
        OsRng.fill_bytes(&mut recovery_key);
        let recovery_key = VaultRecoveryKey::new(recovery_key);
        let archive = self
            .backup
            .export(VaultBackupExportRequest {
                scopes,
                key,
                recovery_key: recovery_key.clone(),
            })
            .map_err(VaultBackupFailure::Backup)?;
        Ok(VaultBackupExport {
            archive,
            recovery_key,
        })
    }

    /// 사용자가 확인한 보관 파일을 가져온다. 취소하면 키 저장소와 Vault 를 건드리지 않는다.
    /// 이 장치에 키가 없으면 새 키를 만든다. 이미 있는 키는 바꾸지 않는다.
    pub fn import(
        &self,
        request: VaultBackupApprovedImport,
    ) -> Result<VaultBackupImportReport, VaultBackupFailure> {
        if request.decision == VaultUserDecision::Cancelled {
            return Err(VaultBackupFailure::NotApproved);
        }
        let local_key = self
            .unlock
            .sealing_key()
            .map_err(VaultBackupFailure::Locked)?;
        self.backup
            .import(VaultBackupImportRequest {
                archive: request.archive,
                recovery_key: request.recovery_key,
                local_key,
                projects: request.projects,
            })
            .map_err(VaultBackupFailure::Backup)
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use serde::Serialize;

    use super::*;
    use crate::adapter::outbound::filesystem::evidence_vault_backup::FilesystemEvidenceVaultBackupAdapter;
    use crate::adapter::outbound::filesystem::evidence_vault_store::FilesystemEvidenceVaultStoreAdapter;
    use crate::adapter::outbound::filesystem::secure_fs;
    use crate::adapter::outbound::vault_key_store::FakeVaultKeyStoreAdapter;
    use crate::application::port::outbound::evidence_vault_port::{
        VaultKey, VaultPlaintext, VaultRecordSaveRequest,
    };
    use crate::domain::issue_work::{IssueProjectScope, IssueProvider};

    // 합성 데이터이다. 실제 키, 사람, 경로가 아니다.
    const SOURCE_KEY: [u8; 32] = [7; 32];
    const LOCAL_KEY: [u8; 32] = [9; 32];
    const SOURCE_PROJECT: &str = "/work/project";
    const LOCAL_PROJECT: &str = "/other/checkout";
    const VERSION: &str = "1:synthetic-version";
    const TITLE_SPANS: &str = "synthetic-title-spans-sentinel";
    const BODY_SPANS: &str = "synthetic-body-spans-sentinel";

    /// 장치 한 대이다: 임시 Vault 폴더와 가짜 키 저장소.
    struct Device {
        vault_root: PathBuf,
        key_store: Arc<FakeVaultKeyStoreAdapter>,
        vault: Arc<FilesystemEvidenceVaultStoreAdapter>,
        unlock: Arc<EvidenceVaultUnlockService>,
        service: EvidenceVaultBackupService,
    }

    impl Device {
        fn new(label: &str, key: [u8; 32]) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos();
            let vault_root = std::env::temp_dir().join(format!(
                "akra-evidence-vault-backup-{label}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&vault_root).expect("temp directory");
            let key_store = Arc::new(FakeVaultKeyStoreAdapter::with_key(VaultKey::new(
                key.to_vec(),
            )));
            let vault = Arc::new(FilesystemEvidenceVaultStoreAdapter::new(vault_root.clone()));
            let unlock = Arc::new(EvidenceVaultUnlockService::new(key_store.clone()));
            Self {
                service: EvidenceVaultBackupService::new(
                    vault.clone(),
                    Arc::new(FilesystemEvidenceVaultBackupAdapter::new(
                        vault_root.clone(),
                    )),
                    unlock.clone(),
                ),
                vault_root,
                key_store,
                vault,
                unlock,
            }
        }

        fn save(&self, scope: VaultRecordScope, text: &str) {
            self.vault
                .save(VaultRecordSaveRequest {
                    scope,
                    key: self.unlock.sealing_key().expect("device key"),
                    plaintext: VaultPlaintext::new(text.to_string()),
                })
                .expect("save should succeed");
        }

        /// 이슈를 `version` 에 연결한다: 두 필드의 레코드와 `CurrentVersion` 이다.
        fn bind(&self, issue: &IssueReference, version: &str) {
            self.save(
                confirmed(issue, VaultRecordField::Title, version),
                TITLE_SPANS,
            );
            self.save(
                confirmed(issue, VaultRecordField::Body, version),
                BODY_SPANS,
            );
            self.save(VaultRecordScope::current_version(issue.clone()), version);
        }

        fn load(&self, scope: VaultRecordScope) -> Result<String, VaultStoreError> {
            self.vault
                .load(VaultRecordLoadRequest {
                    scope,
                    key: self.unlock.sealing_key().expect("device key"),
                })
                .map(|plaintext| plaintext.expose().to_string())
        }

        /// 레코드 파일의 이름과 내용이다. 이름 순서이다.
        fn record_files(&self) -> Vec<(String, Vec<u8>)> {
            let Ok(entries) = fs::read_dir(self.vault_root.join("records")) else {
                return Vec::new();
            };
            let mut files = entries
                .map(|entry| entry.expect("record entry"))
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
                .map(|entry| {
                    (
                        entry.file_name().to_string_lossy().to_string(),
                        fs::read(entry.path()).expect("record file"),
                    )
                })
                .collect::<Vec<_>>();
            files.sort();
            files
        }

        fn import(
            &self,
            archive: &str,
            recovery_key: &VaultRecoveryKey,
        ) -> Result<VaultBackupImportReport, VaultBackupFailure> {
            self.service.import(VaultBackupApprovedImport {
                archive: archive.to_string(),
                recovery_key: recovery_key.clone(),
                decision: VaultUserDecision::Confirmed,
                projects: vec![approval()],
            })
        }
    }

    fn issue(workspace_dir: &str, issue_id: &str) -> IssueReference {
        IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".to_string(),
            repository_id: "R_1".to_string(),
            issue_id: issue_id.to_string(),
            project_scope: project(workspace_dir),
        }
    }

    fn project(workspace_dir: &str) -> IssueProjectScope {
        IssueProjectScope {
            workspace_dir: workspace_dir.to_string(),
        }
    }

    fn approval() -> VaultBackupProjectApproval {
        VaultBackupProjectApproval {
            source: project(SOURCE_PROJECT),
            local: project(LOCAL_PROJECT),
        }
    }

    fn confirmed(
        issue: &IssueReference,
        field: VaultRecordField,
        version: &str,
    ) -> VaultRecordScope {
        VaultRecordScope {
            issue: issue.clone(),
            field,
            binding: VaultRecordBinding::ConfirmedVersion(version.to_string()),
        }
    }

    /// 원본 장치에서 이슈 두 건을 연결하고 내보낸다.
    fn exported(label: &str) -> (Device, VaultBackupExport) {
        let source = Device::new(label, SOURCE_KEY);
        let issues = [issue(SOURCE_PROJECT, "I_1"), issue(SOURCE_PROJECT, "I_2")];
        for issue in &issues {
            source.bind(issue, VERSION);
        }
        let export = source
            .service
            .export(&issues)
            .expect("export should succeed");
        (source, export)
    }

    fn archive_json(archive: &str) -> serde_json::Value {
        serde_json::from_str(archive).expect("archive json")
    }

    fn backup_error<T: fmt::Debug>(
        result: Result<T, VaultBackupFailure>,
    ) -> Result<T, VaultBackupError> {
        result.map_err(|failure| match failure {
            VaultBackupFailure::Backup(error) => error,
            other => panic!("not a backup error: {other:?}"),
        })
    }

    #[test]
    fn export_holds_versioned_ciphertext_and_no_recovery_key_or_plaintext() {
        let (source, export) = exported("export");
        let archive = archive_json(&export.archive);

        assert_eq!(archive["formatVersion"], 1);
        assert_eq!(archive["algorithm"], "chacha20-poly1305");
        assert_eq!(archive["kdf"], "hkdf-sha256");
        assert_eq!(archive["records"].as_array().expect("records").len(), 6);
        assert_eq!(
            archive["records"][0]["record"].as_str().map(str::is_empty),
            Some(false)
        );
        let recovery_text = export.recovery_key.expose_text();
        for secret in [
            recovery_text.clone(),
            recovery_text.replace('-', ""),
            BASE64.encode(export.recovery_key.expose()),
            BASE64.encode(SOURCE_KEY),
            TITLE_SPANS.to_string(),
            BODY_SPANS.to_string(),
            BASE64.encode(TITLE_SPANS),
        ] {
            assert!(!export.archive.contains(&secret));
        }
        assert_eq!(
            VaultRecoveryKey::parse(&recovery_text),
            Some(export.recovery_key.clone())
        );
        assert_eq!(VaultRecoveryKey::parse("not-a-key"), None);
        assert_eq!(VaultRecoveryKey::parse(&recovery_text[..20]), None);

        // 연결이 없는 이슈만 고르면 내보낼 것이 없다.
        assert_eq!(
            backup_error(source.service.export(&[issue(SOURCE_PROJECT, "I_9")])),
            Err(VaultBackupError::NothingToExport)
        );
    }

    #[test]
    fn import_on_another_device_writes_only_the_approved_project_under_the_local_key() {
        let (_source, export) = exported("import-source");
        let local = Device::new("import-local", LOCAL_KEY);

        // 승인한 프로젝트가 없으면 아무것도 쓰지 않고, 보관 파일의 프로젝트만 알려준다.
        let preview = local
            .service
            .import(VaultBackupApprovedImport {
                archive: export.archive.clone(),
                recovery_key: export.recovery_key.clone(),
                decision: VaultUserDecision::Confirmed,
                projects: Vec::new(),
            })
            .expect("preview should succeed");
        assert_eq!(preview.unapproved_projects, vec![project(SOURCE_PROJECT)]);
        assert!(preview.imported.is_empty());
        assert!(local.record_files().is_empty());

        let report = local
            .import(&export.archive, &export.recovery_key)
            .expect("import should succeed");

        let local_issue = issue(LOCAL_PROJECT, "I_1");
        assert_eq!(
            report.imported,
            vec![local_issue.clone(), issue(LOCAL_PROJECT, "I_2")]
        );
        assert!(report.kept_local.is_empty());
        assert!(report.unapproved_projects.is_empty());
        assert_eq!(
            report.backup_id,
            archive_json(&export.archive)["backupId"]
                .as_str()
                .expect("id")
        );
        // 로컬 장치 키와 로컬 프로젝트로 열린다. 로컬 장치 키는 그대로이다.
        assert_eq!(
            local.load(VaultRecordScope::current_version(local_issue.clone())),
            Ok(VERSION.to_string())
        );
        assert_eq!(
            local.load(confirmed(&local_issue, VaultRecordField::Body, VERSION)),
            Ok(BODY_SPANS.to_string())
        );
        assert_eq!(
            local.key_store.stored_key(),
            Some(VaultKey::new(LOCAL_KEY.to_vec()))
        );
        // 원본 프로젝트의 경로에는 연결이 생기지 않는다.
        assert_eq!(
            local.load(VaultRecordScope::current_version(issue(
                SOURCE_PROJECT,
                "I_1"
            ))),
            Err(VaultStoreError::RecordNotFound)
        );
    }

    #[test]
    fn cancelled_import_touches_neither_the_key_store_nor_the_vault() {
        let (_source, export) = exported("cancel-source");
        let local = Device::new("cancel-local", LOCAL_KEY);

        let result = local.service.import(VaultBackupApprovedImport {
            archive: export.archive,
            recovery_key: export.recovery_key,
            decision: VaultUserDecision::Cancelled,
            projects: vec![approval()],
        });

        assert_eq!(result, Err(VaultBackupFailure::NotApproved));
        assert_eq!(local.key_store.load_count(), 0);
        assert!(local.record_files().is_empty());
    }

    #[test]
    fn wrong_recovery_key_imports_nothing() {
        let (_source, export) = exported("wrong-key-source");
        let local = Device::new("wrong-key-local", LOCAL_KEY);
        let mut other = export.recovery_key.expose().to_vec();
        other[0] ^= 1;

        assert_eq!(
            backup_error(local.import(&export.archive, &VaultRecoveryKey::new(other))),
            Err(VaultBackupError::RecoveryKeyRejected)
        );
        assert!(local.record_files().is_empty());
    }

    #[test]
    fn corrupted_archive_imports_nothing() {
        let (_source, export) = exported("corrupt-source");
        let local = Device::new("corrupt-local", LOCAL_KEY);
        let archive = archive_json(&export.archive);
        let edited = |edit: &dyn Fn(&mut serde_json::Value)| {
            let mut archive = archive.clone();
            edit(&mut archive);
            backup_error(local.import(&archive.to_string(), &export.recovery_key))
        };

        // 잘린 파일이다.
        assert_eq!(
            backup_error(local.import(
                &export.archive[..export.archive.len() / 2],
                &export.recovery_key
            )),
            Err(VaultBackupError::CorruptArchive)
        );
        assert_eq!(
            edited(&|archive| archive["formatVersion"] = 2.into()),
            Err(VaultBackupError::UnsupportedFormat)
        );
        // 레코드의 암호문, 레코드 목록, 범위, 머리말, 감싼 키는 모두 감싼 키의 인증에 묶여 있다.
        assert_eq!(
            edited(&|archive| {
                let record: serde_json::Value =
                    serde_json::from_str(archive["records"][0]["record"].as_str().expect("record"))
                        .expect("record json");
                let mut sealed = BASE64
                    .decode(record["ciphertext"].as_str().expect("ciphertext"))
                    .expect("base64");
                sealed[0] ^= 1;
                let mut record = record.clone();
                record["ciphertext"] = BASE64.encode(&sealed).into();
                archive["records"][0]["record"] = record.to_string().into();
            }),
            Err(VaultBackupError::RecoveryKeyRejected)
        );
        assert_eq!(
            edited(&|archive| {
                archive["records"].as_array_mut().expect("records").pop();
            }),
            Err(VaultBackupError::RecoveryKeyRejected)
        );
        assert_eq!(
            edited(&|archive| archive["records"][0]["issue"]["issueId"] = "I_3".into()),
            Err(VaultBackupError::RecoveryKeyRejected)
        );
        assert_eq!(
            edited(&|archive| archive["keyId"] = "device-0000000000000000".into()),
            Err(VaultBackupError::RecoveryKeyRejected)
        );
        assert_eq!(
            edited(&|archive| {
                let mut wrapped = BASE64
                    .decode(archive["wrappedKey"].as_str().expect("wrapped key"))
                    .expect("base64");
                wrapped[0] ^= 1;
                archive["wrappedKey"] = BASE64.encode(&wrapped).into();
            }),
            Err(VaultBackupError::RecoveryKeyRejected)
        );
        assert!(local.record_files().is_empty());
    }

    #[test]
    fn repeated_import_changes_nothing_and_keeps_a_newer_local_binding() {
        let (_source, export) = exported("repeat-source");
        let local = Device::new("repeat-local", LOCAL_KEY);
        local
            .import(&export.archive, &export.recovery_key)
            .expect("first import");
        // 가져온 뒤에 이 장치에서 이슈 한 건을 새 판에 연결했다.
        let newer = issue(LOCAL_PROJECT, "I_1");
        local.bind(&newer, "1:synthetic-newer-version");
        let before = local.record_files();

        let report = local
            .import(&export.archive, &export.recovery_key)
            .expect("second import");

        assert!(report.imported.is_empty());
        assert_eq!(
            report.kept_local,
            vec![newer.clone(), issue(LOCAL_PROJECT, "I_2")]
        );
        assert_eq!(local.record_files(), before);
        assert_eq!(
            local.load(VaultRecordScope::current_version(newer)),
            Ok("1:synthetic-newer-version".to_string())
        );
    }

    #[test]
    fn import_into_the_exporting_device_keeps_every_local_record() {
        let (source, export) = exported("same-device");
        let before = source.record_files();

        let report = source
            .service
            .import(VaultBackupApprovedImport {
                archive: export.archive,
                recovery_key: export.recovery_key,
                decision: VaultUserDecision::Confirmed,
                projects: vec![VaultBackupProjectApproval {
                    source: project(SOURCE_PROJECT),
                    local: project(SOURCE_PROJECT),
                }],
            })
            .expect("import should succeed");

        assert!(report.imported.is_empty());
        assert_eq!(report.kept_local.len(), 2);
        assert_eq!(source.record_files(), before);
    }

    #[test]
    fn interrupted_import_leaves_no_binding_and_a_retry_finishes_it() {
        let (_source, export) = exported("interrupt-source");
        let local = Device::new("interrupt-local", LOCAL_KEY);
        let current =
            |issue_id: &str| VaultRecordScope::current_version(issue(LOCAL_PROJECT, issue_id));

        // 첫 레코드를 쓴 직후에 멈춘 가져오기이다.
        secure_fs::install_after_atomic_replace_error_hook("synthetic interruption");
        assert!(matches!(
            backup_error(local.import(&export.archive, &export.recovery_key)),
            Err(VaultBackupError::Storage(_))
        ));
        assert_eq!(local.record_files().len(), 1);
        for issue_id in ["I_1", "I_2"] {
            assert_eq!(
                local.load(current(issue_id)),
                Err(VaultStoreError::RecordNotFound)
            );
        }

        let report = local
            .import(&export.archive, &export.recovery_key)
            .expect("retry should succeed");

        assert_eq!(report.imported.len(), 2);
        assert!(report.kept_local.is_empty());
        assert_eq!(local.record_files().len(), 6);
        for issue_id in ["I_1", "I_2"] {
            assert_eq!(local.load(current(issue_id)), Ok(VERSION.to_string()));
            assert_eq!(
                local.load(confirmed(
                    &issue(LOCAL_PROJECT, issue_id),
                    VaultRecordField::Title,
                    VERSION
                )),
                Ok(TITLE_SPANS.to_string())
            );
        }
    }

    // `T: Serialize` 이면 두 impl 이 겹쳐서 `_` 를 정할 수 없다. 그러면 컴파일이 실패한다.
    trait NotSerialize<Marker> {
        fn proven() {}
    }
    impl<T: ?Sized> NotSerialize<()> for T {}
    impl<T: ?Sized + Serialize> NotSerialize<u8> for T {}

    #[test]
    fn backup_types_stay_out_of_serialized_surfaces_and_debug_output() {
        <VaultRecoveryKey as NotSerialize<_>>::proven();
        <VaultBackupExport as NotSerialize<_>>::proven();
        <VaultBackupApprovedImport as NotSerialize<_>>::proven();
        <VaultBackupExportRequest as NotSerialize<_>>::proven();
        <VaultBackupImportRequest as NotSerialize<_>>::proven();

        let (_source, export) = exported("debug");
        let recovery_text = export.recovery_key.expose_text();
        let recovery_bytes = format!("{:?}", export.recovery_key.expose());
        let debug = format!("{export:?}");

        assert!(!debug.contains(&recovery_text));
        assert!(!debug.contains(&recovery_bytes));
    }
}
