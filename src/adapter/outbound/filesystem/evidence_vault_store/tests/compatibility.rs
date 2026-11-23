use super::*;

/// Frozen synthetic v1 bytes, encoded independently with a fixed test nonce and key.
/// Changes to the scope spelling, length prefixes, AAD, or file format must not silently migrate it.
const FROZEN_RECORD: &str = r#"{"formatVersion":1,"algorithm":"chacha20-poly1305","keyId":"synthetic-key-1","nonce":"AAECAwQFBgcICQoL","ciphertext":"XwyfVwT+20V0W2e40GAeG8yg1dwLoUCUom49LvRCjmFWcauyDsTFJ+Pt"}"#;
const FROZEN_FILE: &str = "1607a080f59ea7dfbe87fb9ac184469fa1a83ceefc362d309056d83050a6a1a0.json";
const FROZEN_PLAINTEXT: &str = "synthetic-frozen-v1-원문";

#[test]
fn frozen_format_one_ciphertext_keeps_the_exact_windows_workspace_scope() {
    let root = temp_vault("v1-한글");
    let adapter = FilesystemEvidenceVaultStoreAdapter::new(root.clone());
    let original = scope(r"C:\Synthetic\프로젝트");
    let relative = record_relative_path(&scope_bytes(&original));
    assert_eq!(relative, Path::new("records").join(FROZEN_FILE));
    vault_fs::write_file_atomic(&root, &relative, FROZEN_RECORD.as_bytes()).unwrap();

    assert_eq!(
        load(&adapter, original.clone()).unwrap().expose(),
        FROZEN_PLAINTEXT
    );
    assert_eq!(
        fs::read_to_string(root.join(&relative)).unwrap(),
        FROZEN_RECORD
    );

    // Filesystem aliases must not change the historical AEAD identity of a project.
    // The composition layer chooses the canonical workspace before creating a scope.
    for workspace in [r"c:\Synthetic\프로젝트", r"\\?\C:\Synthetic\프로젝트"] {
        let changed = scope(workspace);
        let changed_path = record_relative_path(&scope_bytes(&changed));
        assert_ne!(changed_path, relative);
        vault_fs::write_file_atomic(&root, &changed_path, FROZEN_RECORD.as_bytes()).unwrap();
        assert_eq!(
            load(&adapter, changed),
            Err(VaultStoreError::IntegrityFailure)
        );
    }
    assert_eq!(load(&adapter, original).unwrap().expose(), FROZEN_PLAINTEXT);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reading_missing_records_or_a_missing_root_never_initializes_storage() {
    let root = temp_vault("missing-read");
    let adapter = FilesystemEvidenceVaultStoreAdapter::new(root.clone());
    assert_eq!(
        load(&adapter, project_scope()),
        Err(VaultStoreError::RecordNotFound)
    );
    assert_eq!(
        load_creation(&adapter, creation_scope()),
        Err(VaultStoreError::RecordNotFound)
    );
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);

    fs::remove_dir(&root).unwrap();
    // Unix preserves its existing missing-root Storage error; Windows can report NotFound.
    // Neither read is allowed to prepare a writer root or its records directory.
    for result in [
        load(&adapter, project_scope()),
        load_creation(&adapter, creation_scope()),
    ] {
        assert!(matches!(
            result,
            Err(VaultStoreError::RecordNotFound | VaultStoreError::Storage(_))
        ));
    }
    assert!(!root.exists());
}
