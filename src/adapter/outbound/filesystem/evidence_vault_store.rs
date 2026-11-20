/*
 * Evidence Vault 저장 계약의 로컬 파일 구현이다 (#2421). 레코드 한 건이 파일 하나이다.
 *
 * 암호는 `ring` 의 ChaCha20-Poly1305 (AEAD) 이다. 레코드마다 OS 난수로 새 nonce 를 만든다. 형식의 판, 알고리즘,
 * 키 식별자, 레코드의 범위(로컬 프로젝트, 이슈, 필드, 게시 의도 또는 확인된 판)는 연관 데이터로 암호문에 묶는다.
 * 그래서 머리말이나 암호문을 바꾼 레코드, 다른 범위의 자리로 복사한 레코드는 인증에 실패한다.
 *
 * 파일은 `secure_fs` 로 원자적으로 바꾼다. 저장이 중간에 멈추면 이전 레코드가 남는다.
 */
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use super::secure_fs;
use crate::application::port::outbound::evidence_vault_port::{
    EvidenceVaultStorePort, VaultPlaintext, VaultRecordBinding, VaultRecordField,
    VaultRecordLoadRequest, VaultRecordSaveRequest, VaultRecordScope, VaultStoreError,
};
use crate::domain::issue_work::IssueProvider;

/// 레코드 파일의 암호화 형식의 판이다. 파일의 필드, 알고리즘, 연관 데이터의 구성이 바뀌면 올린다.
const RECORD_FORMAT_VERSION: u32 = 1;
const RECORD_ALGORITHM: &str = "chacha20-poly1305";
const ASSOCIATED_DATA_DOMAIN: &str = "akra-evidence-vault-record";

/// 레코드 파일의 내용이다. 키와 평문은 없다.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredVaultRecord {
    format_version: u32,
    algorithm: String,
    key_id: String,
    /// base64 이다.
    nonce: String,
    /// base64 이다. 암호문 뒤에 인증 태그가 붙어 있다.
    ciphertext: String,
}

pub struct FilesystemEvidenceVaultStoreAdapter {
    vault_root: PathBuf,
}

impl FilesystemEvidenceVaultStoreAdapter {
    /// `vault_root` 는 이미 있는 사용자 전용 폴더이다. 프로젝트의 작업 폴더 안에 두지 않는다.
    pub fn new(vault_root: PathBuf) -> Self {
        Self { vault_root }
    }
}

impl EvidenceVaultStorePort for FilesystemEvidenceVaultStoreAdapter {
    fn save(&self, request: VaultRecordSaveRequest) -> Result<(), VaultStoreError> {
        let scope = scope_bytes(&request.scope);
        let key = aead_key(request.key.key.expose())?;
        // 96 bit 난수 nonce 는 키 하나에 레코드 2^32 건까지 안전하다. 이슈 레코드는 그보다 훨씬 적다.
        let mut nonce = [0u8; NONCE_LEN];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| VaultStoreError::Storage("the OS random source failed".to_string()))?;

        let mut sealed = request.plaintext.expose().as_bytes().to_vec();
        let associated_data = associated_data(&request.key.key_id, &scope);
        if key
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(associated_data),
                &mut sealed,
            )
            .is_err()
        {
            sealed.zeroize();
            return Err(VaultStoreError::Storage(
                "the record is too large to seal".to_string(),
            ));
        }

        let record = StoredVaultRecord {
            format_version: RECORD_FORMAT_VERSION,
            algorithm: RECORD_ALGORITHM.to_string(),
            key_id: request.key.key_id.clone(),
            nonce: BASE64.encode(nonce),
            ciphertext: BASE64.encode(&sealed),
        };
        let body = serde_json::to_vec(&record).map_err(storage_error)?;
        secure_fs::write_file_atomic(&self.vault_root, &record_relative_path(&scope), &body)
            .map_err(storage_error)
    }

    fn load(&self, request: VaultRecordLoadRequest) -> Result<VaultPlaintext, VaultStoreError> {
        let scope = scope_bytes(&request.scope);
        let body = secure_fs::read_optional_file(&self.vault_root, &record_relative_path(&scope))
            .map_err(storage_error)?
            .ok_or(VaultStoreError::RecordNotFound)?;
        let record = parse_record(&body)?;
        if record.key_id != request.key.key_id {
            return Err(VaultStoreError::KeyMismatch);
        }
        let nonce = BASE64
            .decode(&record.nonce)
            .ok()
            .and_then(|nonce| Nonce::try_assume_unique_for_key(&nonce).ok())
            .ok_or(VaultStoreError::CorruptRecord)?;
        let mut opened = BASE64
            .decode(&record.ciphertext)
            .map_err(|_| VaultStoreError::CorruptRecord)?;

        let key = aead_key(request.key.key.expose())?;
        let associated_data = associated_data(&record.key_id, &scope);
        let plaintext_len = match key.open_in_place(nonce, Aad::from(associated_data), &mut opened)
        {
            Ok(plaintext) => plaintext.len(),
            Err(_) => {
                opened.zeroize();
                return Err(VaultStoreError::IntegrityFailure);
            }
        };
        opened.truncate(plaintext_len);
        String::from_utf8(opened)
            .map(VaultPlaintext::new)
            .map_err(|error| {
                error.into_bytes().zeroize();
                VaultStoreError::CorruptRecord
            })
    }
}

fn aead_key(key: &[u8]) -> Result<LessSafeKey, VaultStoreError> {
    UnboundKey::new(&CHACHA20_POLY1305, key)
        .map(LessSafeKey::new)
        .map_err(|_| VaultStoreError::InvalidKey)
}

fn storage_error(error: impl std::fmt::Display) -> VaultStoreError {
    VaultStoreError::Storage(error.to_string())
}

/// 판을 먼저 본다. 다른 판의 레코드는 필드가 달라도 `UnsupportedFormat` 으로 알린다.
fn parse_record(body: &str) -> Result<StoredVaultRecord, VaultStoreError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|_| VaultStoreError::CorruptRecord)?;
    let format_version = value
        .get("formatVersion")
        .and_then(serde_json::Value::as_u64)
        .ok_or(VaultStoreError::CorruptRecord)?;
    if format_version != u64::from(RECORD_FORMAT_VERSION) {
        return Err(VaultStoreError::UnsupportedFormat);
    }
    let record: StoredVaultRecord =
        serde_json::from_value(value).map_err(|_| VaultStoreError::CorruptRecord)?;
    if record.algorithm != RECORD_ALGORITHM {
        return Err(VaultStoreError::UnsupportedFormat);
    }
    Ok(record)
}

// 길이를 앞에 붙인다. 값의 경계가 달라지면 바이트도 달라진다.
fn push_part(bytes: &mut Vec<u8>, part: &str) {
    bytes.extend_from_slice(&(part.len() as u64).to_be_bytes());
    bytes.extend_from_slice(part.as_bytes());
}

/// 레코드의 범위를 정해진 순서의 바이트로 만든다. 파일 이름과 연관 데이터가 이 값을 쓴다.
fn scope_bytes(scope: &VaultRecordScope) -> Vec<u8> {
    let issue = &scope.issue;
    let provider = match issue.provider {
        IssueProvider::GitHub => "github",
        IssueProvider::Jira => "jira",
    };
    let field = match scope.field {
        VaultRecordField::Title => "title",
        VaultRecordField::Body => "body",
    };
    let (binding_kind, binding_value) = match &scope.binding {
        VaultRecordBinding::PublishIntent(intent_id) => ("publish-intent", intent_id.as_str()),
        VaultRecordBinding::ConfirmedVersion(version) => ("confirmed-version", version.as_str()),
        VaultRecordBinding::CurrentVersion => ("current-version", ""),
    };

    let mut bytes = Vec::new();
    for part in [
        issue.project_scope.workspace_dir.as_str(),
        provider,
        &issue.server_id,
        &issue.repository_id,
        &issue.issue_id,
        field,
        binding_kind,
        binding_value,
    ] {
        push_part(&mut bytes, part);
    }
    bytes
}

fn associated_data(key_id: &str, scope: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    push_part(&mut bytes, ASSOCIATED_DATA_DOMAIN);
    bytes.extend_from_slice(&RECORD_FORMAT_VERSION.to_be_bytes());
    push_part(&mut bytes, RECORD_ALGORITHM);
    push_part(&mut bytes, key_id);
    bytes.extend_from_slice(scope);
    bytes
}

/// 파일 이름은 범위의 SHA-256 이다. 이슈 ID 나 경로가 파일 이름에 드러나지 않는다.
fn record_relative_path(scope: &[u8]) -> PathBuf {
    let name: String = Sha256::digest(scope)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Path::new("records").join(format!("{name}.json"))
}

#[cfg(all(test, unix))]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::rc::Rc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::application::port::outbound::evidence_vault_port::{
        EVIDENCE_VAULT_STORE_CONTRACT_VERSION, VaultKey, VaultRecordKey,
    };
    use crate::domain::issue_work::{IssueProjectScope, IssueReference};

    // 합성 데이터이다. 실제 키나 이슈가 아니다.
    const PLAINTEXT: &str = "synthetic-masked-segment-sentinel";
    const KEY_ID: &str = "synthetic-key-1";
    const KEY: [u8; 32] = [7; 32];

    fn temp_vault(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "akra-evidence-vault-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temp directory");
        path
    }

    fn scope(workspace_dir: &str) -> VaultRecordScope {
        VaultRecordScope {
            issue: IssueReference {
                provider: IssueProvider::GitHub,
                server_id: "github.com".to_string(),
                repository_id: "R_1".to_string(),
                issue_id: "I_1".to_string(),
                project_scope: IssueProjectScope {
                    workspace_dir: workspace_dir.to_string(),
                },
            },
            field: VaultRecordField::Body,
            binding: VaultRecordBinding::PublishIntent("intent-1".to_string()),
        }
    }

    fn project_scope() -> VaultRecordScope {
        scope("/work/project")
    }

    fn key(key_id: &str, key: &[u8]) -> VaultRecordKey {
        VaultRecordKey {
            key_id: key_id.to_string(),
            key: VaultKey::new(key.to_vec()),
        }
    }

    fn save(adapter: &FilesystemEvidenceVaultStoreAdapter, scope: VaultRecordScope, text: &str) {
        adapter
            .save(VaultRecordSaveRequest {
                scope,
                key: key(KEY_ID, &KEY),
                plaintext: VaultPlaintext::new(text.to_string()),
            })
            .expect("save should succeed");
    }

    fn load(
        adapter: &FilesystemEvidenceVaultStoreAdapter,
        scope: VaultRecordScope,
    ) -> Result<VaultPlaintext, VaultStoreError> {
        adapter.load(VaultRecordLoadRequest {
            scope,
            key: key(KEY_ID, &KEY),
        })
    }

    fn record_path(vault_root: &Path, scope: &VaultRecordScope) -> PathBuf {
        vault_root.join(record_relative_path(&scope_bytes(scope)))
    }

    fn read_record(vault_root: &Path, scope: &VaultRecordScope) -> serde_json::Value {
        let body = fs::read_to_string(record_path(vault_root, scope)).expect("record file");
        serde_json::from_str(&body).expect("record json")
    }

    fn write_record(vault_root: &Path, scope: &VaultRecordScope, record: &serde_json::Value) {
        fs::write(record_path(vault_root, scope), record.to_string()).expect("record fixture");
    }

    fn seeded(label: &str) -> (PathBuf, FilesystemEvidenceVaultStoreAdapter) {
        let vault_root = temp_vault(label);
        let adapter = FilesystemEvidenceVaultStoreAdapter::new(vault_root.clone());
        save(&adapter, project_scope(), PLAINTEXT);
        (vault_root, adapter)
    }

    #[test]
    fn saved_record_restores_the_plaintext_and_stores_version_and_key_id_only() {
        let (vault_root, adapter) = seeded("round-trip");

        let restored = load(&adapter, project_scope()).expect("load should succeed");
        let body = fs::read_to_string(record_path(&vault_root, &project_scope())).expect("record");
        let record = read_record(&vault_root, &project_scope());

        assert_eq!(restored.expose(), PLAINTEXT);
        assert_eq!(record["formatVersion"], RECORD_FORMAT_VERSION);
        assert_eq!(record["algorithm"], RECORD_ALGORITHM);
        assert_eq!(record["keyId"], KEY_ID);
        assert!(!body.contains(PLAINTEXT));
        assert!(!body.contains(&BASE64.encode(PLAINTEXT)));
        assert!(!body.contains(&BASE64.encode(KEY)));
        assert!(!body.contains("/work/project"));
        assert_eq!(
            adapter.contract_version(),
            EVIDENCE_VAULT_STORE_CONTRACT_VERSION
        );
    }

    #[test]
    fn each_save_uses_a_fresh_nonce() {
        let (vault_root, adapter) = seeded("nonce");
        let first = read_record(&vault_root, &project_scope());

        save(&adapter, project_scope(), PLAINTEXT);
        let second = read_record(&vault_root, &project_scope());

        assert_ne!(first["nonce"], second["nonce"]);
        assert_ne!(first["ciphertext"], second["ciphertext"]);
    }

    #[test]
    fn tampered_ciphertext_fails_the_integrity_check() {
        let (vault_root, adapter) = seeded("tamper");
        let mut record = read_record(&vault_root, &project_scope());
        let ciphertext = record["ciphertext"].as_str().expect("ciphertext");
        let mut sealed = BASE64.decode(ciphertext).expect("base64");

        for index in [0, sealed.len() - 1] {
            sealed[index] ^= 1;
            record["ciphertext"] = BASE64.encode(&sealed).into();
            write_record(&vault_root, &project_scope(), &record);

            assert_eq!(
                load(&adapter, project_scope()),
                Err(VaultStoreError::IntegrityFailure)
            );
            sealed[index] ^= 1;
        }
    }

    #[test]
    fn tampered_nonce_fails_the_integrity_check() {
        let (vault_root, adapter) = seeded("nonce-tamper");
        let mut record = read_record(&vault_root, &project_scope());
        record["nonce"] = BASE64.encode([0u8; NONCE_LEN]).into();
        write_record(&vault_root, &project_scope(), &record);

        assert_eq!(
            load(&adapter, project_scope()),
            Err(VaultStoreError::IntegrityFailure)
        );
    }

    #[test]
    fn record_copied_to_another_scope_fails_the_integrity_check() {
        let (vault_root, adapter) = seeded("copy");
        let mut other_issue = project_scope();
        other_issue.issue.issue_id = "I_2".to_string();
        let mut other_field = project_scope();
        other_field.field = VaultRecordField::Title;
        let mut confirmed = project_scope();
        confirmed.binding = VaultRecordBinding::ConfirmedVersion("intent-1".to_string());

        for other in [
            scope("/work/other-project"),
            other_issue,
            other_field,
            confirmed,
        ] {
            assert_eq!(
                load(&adapter, other.clone()),
                Err(VaultStoreError::RecordNotFound)
            );
            fs::copy(
                record_path(&vault_root, &project_scope()),
                record_path(&vault_root, &other),
            )
            .expect("copy record");

            assert_eq!(
                load(&adapter, other),
                Err(VaultStoreError::IntegrityFailure)
            );
        }
        assert_eq!(
            load(&adapter, project_scope())
                .expect("the original record stays valid")
                .expose(),
            PLAINTEXT
        );
    }

    #[test]
    fn wrong_key_and_wrong_key_id_fail_with_distinct_errors() {
        let (vault_root, adapter) = seeded("key");
        let load_with = |key: VaultRecordKey| {
            adapter.load(VaultRecordLoadRequest {
                scope: project_scope(),
                key,
            })
        };

        assert_eq!(
            load_with(key(KEY_ID, &[8; 32])),
            Err(VaultStoreError::IntegrityFailure)
        );
        assert_eq!(
            load_with(key("synthetic-key-2", &KEY)),
            Err(VaultStoreError::KeyMismatch)
        );
        assert_eq!(
            load_with(key(KEY_ID, &[7; 16])),
            Err(VaultStoreError::InvalidKey)
        );

        // 머리말의 키 식별자만 바꿔도 인증에 실패한다.
        let mut record = read_record(&vault_root, &project_scope());
        record["keyId"] = "synthetic-key-2".into();
        write_record(&vault_root, &project_scope(), &record);
        assert_eq!(
            load_with(key("synthetic-key-2", &KEY)),
            Err(VaultStoreError::IntegrityFailure)
        );
    }

    #[test]
    fn truncated_or_unknown_format_record_is_rejected() {
        let (vault_root, adapter) = seeded("format");
        let path = record_path(&vault_root, &project_scope());
        let body = fs::read_to_string(&path).expect("record");
        let mut record = read_record(&vault_root, &project_scope());

        fs::write(&path, &body[..body.len() / 2]).expect("truncated fixture");
        assert_eq!(
            load(&adapter, project_scope()),
            Err(VaultStoreError::CorruptRecord)
        );

        let ciphertext = record["ciphertext"].as_str().expect("ciphertext");
        let mut short = record.clone();
        short["ciphertext"] = ciphertext[..4].into();
        write_record(&vault_root, &project_scope(), &short);
        assert_eq!(
            load(&adapter, project_scope()),
            Err(VaultStoreError::IntegrityFailure)
        );

        record["formatVersion"] = 2.into();
        write_record(&vault_root, &project_scope(), &record);
        assert_eq!(
            load(&adapter, project_scope()),
            Err(VaultStoreError::UnsupportedFormat)
        );
    }

    #[test]
    fn interrupted_save_keeps_the_previous_record() {
        let (vault_root, adapter) = seeded("interrupt");
        let path = record_path(&vault_root, &project_scope());
        let before = fs::read(&path).expect("record");

        // 교체 직전에 멈춘 저장이다: 임시 파일은 있고 레코드는 이전 그대로이다.
        let seen = Rc::new(RefCell::new(Vec::new()));
        let seen_in_hook = Rc::clone(&seen);
        let hook_path = path.clone();
        secure_fs::install_before_atomic_replace_hook(move || {
            *seen_in_hook.borrow_mut() = fs::read(hook_path).expect("record before replace");
        });
        save(&adapter, project_scope(), "synthetic-second-segment");
        assert_eq!(*seen.borrow(), before);

        // 교체 전에 실패한 저장은 이전 레코드를 바꾸지 않는다.
        let failed = adapter.save(VaultRecordSaveRequest {
            scope: project_scope(),
            key: key(KEY_ID, &[7; 16]),
            plaintext: VaultPlaintext::new("synthetic-third-segment".to_string()),
        });
        assert_eq!(failed, Err(VaultStoreError::InvalidKey));
        assert_eq!(
            load(&adapter, project_scope())
                .expect("load should succeed")
                .expose(),
            "synthetic-second-segment"
        );
    }

    // `T: Serialize` 이면 두 impl 이 겹쳐서 `_` 를 정할 수 없다. 그러면 컴파일이 실패한다.
    trait NotSerialize<Marker> {
        fn proven() {}
    }
    impl<T: ?Sized> NotSerialize<()> for T {}
    impl<T: ?Sized + Serialize> NotSerialize<u8> for T {}

    #[test]
    fn store_requests_stay_out_of_serialized_surfaces_and_debug_output() {
        <VaultRecordKey as NotSerialize<_>>::proven();
        <VaultRecordSaveRequest as NotSerialize<_>>::proven();
        <VaultRecordLoadRequest as NotSerialize<_>>::proven();

        let debug = format!(
            "{:?}",
            VaultRecordSaveRequest {
                scope: project_scope(),
                key: key(KEY_ID, &KEY),
                plaintext: VaultPlaintext::new(PLAINTEXT.to_string()),
            }
        );

        assert!(!debug.contains(PLAINTEXT));
        assert!(!debug.contains(&format!("{KEY:?}")));
    }
}
