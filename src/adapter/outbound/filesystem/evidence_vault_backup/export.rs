use super::*;

pub(super) fn selected(
    vault_root: &std::path::Path,
    request: VaultBackupExportRequest,
) -> Result<String, VaultBackupError> {
    let random = SystemRandom::new();
    let mut backup_id = [0u8; BACKUP_ID_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    random
        .fill(&mut backup_id)
        .and_then(|()| random.fill(&mut nonce))
        .map_err(|_| VaultBackupError::Storage("the OS random source failed".to_string()))?;
    let backup_id_text: String = backup_id.iter().map(|byte| format!("{byte:02x}")).collect();
    let mut key_bytes = zeroize::Zeroizing::new(vec![0; CHACHA20_POLY1305.key_len()]);
    random
        .fill(&mut key_bytes)
        .map_err(|_| VaultBackupError::Storage("the OS random source failed".into()))?;
    let archive_key = VaultRecordKey {
        key_id: format!("archive-{backup_id_text}"),
        key: VaultKey::new(std::mem::take(&mut *key_bytes)),
    };
    let mut records = Vec::new();
    let mut exported_scopes = HashSet::new();
    let store = FilesystemEvidenceVaultStoreAdapter::new(vault_root.to_owned());
    let mut issues = Vec::new();
    for scope in &request.scopes {
        if !issues.contains(&scope.issue) {
            issues.push(scope.issue.clone());
        }
    }
    // Keep one issue's binding lease through every ciphertext read. Other
    // issues are independent and never require nested binding leases.
    for issue in issues {
        let mut transaction = store
            .begin_binding(VaultRecordLoadRequest {
                scope: VaultRecordScope::current_version(issue.clone()),
                key: request.key.clone(),
            })
            .map_err(VaultBackupError::Record)?;
        let snapshot = transaction.snapshot().clone();
        for scope in request.scopes.iter().filter(|scope| scope.issue == issue) {
            if !exported_scopes.insert(scope.clone()) {
                continue;
            }
            let scope_bytes = scope_bytes(scope);
            let Some(record) =
                vault_fs::read_optional_file(vault_root, &record_relative_path(&scope_bytes))
                    .map_err(storage_error)?
            else {
                if scope.binding == VaultRecordBinding::CurrentVersion {
                    return Err(VaultBackupError::BindingChanged);
                }
                continue;
            };
            let plaintext = open_record(&record, &request.key, &scope_bytes)
                .map_err(VaultBackupError::Record)?;
            if scope.binding == VaultRecordBinding::CurrentVersion {
                let digest: [u8; 32] = Sha256::digest(record.as_bytes()).into();
                if scope.field != VaultRecordField::Body
                    || snapshot.token.ciphertext_sha256() != Some(digest)
                    || [VaultRecordField::Title, VaultRecordField::Body]
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
            }
            let sealed = super::super::evidence_vault_store::seal_record(
                &archive_key,
                &scope_bytes,
                &plaintext,
            )
            .map_err(VaultBackupError::Record)?;
            let sealed = String::from_utf8(sealed).map_err(|_| VaultBackupError::CorruptArchive)?;
            records.push(StoredBackupRecord::new(scope, sealed));
        }
        if let VaultBindingState::Readable { version } = &snapshot.state {
            // Revalidate lease identity and all version records before releasing
            // the guard. Only archive-key ciphertext can leave the source.
            transaction
                .load_version_fields(version)
                .map_err(VaultBackupError::Record)?;
        }
    }
    if records.is_empty() {
        return Err(VaultBackupError::NothingToExport);
    }
    if records.len() > MAX_ARCHIVE_RECORDS {
        return Err(VaultBackupError::ArchiveTooLarge);
    }

    let mut archive = StoredVaultBackup {
        format_version: ARCHIVE_FORMAT_VERSION,
        algorithm: ARCHIVE_ALGORITHM.to_string(),
        kdf: ARCHIVE_KDF.to_string(),
        backup_id: backup_id_text,
        key_id: archive_key.key_id.clone(),
        wrap_nonce: BASE64.encode(nonce),
        wrapped_key: String::new(),
        records,
    };

    let mut wrapped = zeroize::Zeroizing::new(archive_key.key.expose().to_vec());
    let sealed = wrap_key(&request.recovery_key, &backup_id)?.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::from(wrap_associated_data(&archive)),
        &mut *wrapped,
    );
    if sealed.is_err() {
        wrapped.zeroize();
        return Err(VaultBackupError::Storage(
            "the archive key cannot be wrapped".to_string(),
        ));
    }
    archive.wrapped_key = BASE64.encode(wrapped.as_slice());

    let body = serde_json::to_string(&archive).map_err(storage_error)?;
    if body.len() > MAX_ARCHIVE_BYTES {
        return Err(VaultBackupError::ArchiveTooLarge);
    }
    Ok(body)
}
