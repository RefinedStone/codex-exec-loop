use std::os::unix::fs::symlink;

use super::*;

#[test]
fn unsafe_pointer_aliases_are_storage_failures_never_recovery_snapshots() {
    for alias in ["symlink", "hardlink", "fifo"] {
        let fixture = Fixture::new("pointer-alias");
        let store = fixture.store();
        seed_legacy(&store);
        let path = pointer_path(&fixture.root);
        let old = fs::read(&path).unwrap();
        fs::remove_file(&path).unwrap();
        let target = fixture.root.join("synthetic-alias-target");
        fs::write(&target, &old).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        match alias {
            "symlink" => symlink(&target, &path).unwrap(),
            "hardlink" => fs::hard_link(&target, &path).unwrap(),
            "fifo" => {
                use std::os::unix::ffi::OsStrExt;
                let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
                // SAFETY: the synthetic path is a live, NUL-terminated CString.
                assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
            }
            _ => unreachable!(),
        }
        assert!(matches!(
            store.begin_binding(load_request()),
            Err(VaultStoreError::Storage(_))
        ));
        assert_eq!(fs::read(&target).unwrap(), old);
    }
}

#[test]
fn unsafe_lock_aliases_and_public_roots_cannot_start_a_binding_transaction() {
    for alias in ["symlink", "hardlink", "public-root"] {
        let fixture = Fixture::new("lock-alias");
        let store = fixture.store();
        seed_legacy(&store);
        let before = all_records(&fixture.root);
        if alias == "public-root" {
            fs::set_permissions(&fixture.root, fs::Permissions::from_mode(0o755)).unwrap();
        } else {
            let path = fixture.root.join(binding_lock_name(&load_request().scope));
            fs::remove_file(&path).unwrap();
            let target = fixture.root.join("synthetic-lock-target");
            fs::write(&target, b"").unwrap();
            fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
            if alias == "symlink" {
                symlink(&target, &path).unwrap();
            } else {
                fs::hard_link(&target, &path).unwrap();
            }
        }
        assert!(matches!(
            store.begin_binding(load_request()),
            Err(VaultStoreError::Storage(_))
        ));
        assert_eq!(all_records(&fixture.root), before);
        fs::set_permissions(&fixture.root, fs::Permissions::from_mode(0o700)).unwrap();
    }
}

#[test]
fn replaced_lock_identity_cannot_read_or_commit_through_an_old_guard() {
    let fixture = Fixture::new("changed-lock");
    let store = fixture.store();
    let expected = seed_legacy(&store);
    let before = all_records(&fixture.root);
    let mut tx = store.begin_binding(load_request()).unwrap();
    let lock = fixture.root.join(binding_lock_name(&load_request().scope));
    fs::rename(&lock, fixture.root.join("retired-lock")).unwrap();
    fs::write(&lock, b"").unwrap();
    fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(matches!(
        tx.load_version_fields(&base_version()),
        Err(VaultStoreError::Storage(_))
    ));
    assert!(matches!(
        tx.commit(request(expected, "must-not-write", desired_fields("new"))),
        Err(VaultStoreError::Storage(_))
    ));
    assert_eq!(all_records(&fixture.root), before);
}

#[test]
fn replaced_root_identity_does_not_create_records_in_the_replacement_root() {
    let fixture = Fixture::new("changed-root");
    let store = fixture.store();
    let expected = seed_legacy(&store);
    let before = all_records(&fixture.root);
    let tx = store.begin_binding(load_request()).unwrap();
    let retired = fixture.root.with_extension("retired");
    fs::rename(&fixture.root, &retired).unwrap();
    fs::create_dir(&fixture.root).unwrap();
    fs::set_permissions(&fixture.root, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(
        tx.commit(request(expected, "must-not-write", desired_fields("new"))),
        Err(VaultStoreError::Storage(_))
    ));
    assert!(all_records(&fixture.root).is_empty());
    assert_eq!(all_records(&retired), before);
    fs::remove_dir_all(retired).unwrap();
}

#[test]
fn a_damaged_field_does_not_turn_a_readable_pointer_into_a_recovery_grant() {
    let fixture = Fixture::new("field-damaged");
    let store = fixture.store();
    let expected = seed_legacy(&store);
    fs::write(
        record_path(
            &fixture.root,
            &field_scope(&base_version(), VaultRecordField::Title),
        ),
        b"synthetic-broken-field",
    )
    .unwrap();
    let mut tx = store.begin_binding(load_request()).unwrap();
    assert!(matches!(
        tx.snapshot().state,
        VaultBindingState::Readable { .. }
    ));
    assert_eq!(
        tx.load_version_fields(&base_version()),
        Err(VaultStoreError::CorruptRecord)
    );
    let mut operation = request(expected, "recover-field-not-pointer", desired_fields("new"));
    operation.policy = VaultBindingCommitPolicy::RecoverUnreadable;
    let before = all_records(&fixture.root);
    assert_eq!(
        tx.commit(operation).unwrap(),
        VaultBindingCommitResult::Conflict
    );
    assert_eq!(all_records(&fixture.root), before);
}

#[test]
fn invalid_scope_and_key_are_rejected_before_a_lease_or_record_is_created() {
    let fixture = Fixture::new("invalid-input");
    let store = fixture.store();
    let mut invalid = load_request();
    invalid.scope.field = VaultRecordField::Title;
    assert!(matches!(
        store.begin_binding(invalid),
        Err(VaultStoreError::Storage(_))
    ));
    let mut invalid = load_request();
    invalid.key.key = VaultKey::new(vec![7; 16]);
    assert!(matches!(
        store.begin_binding(invalid),
        Err(VaultStoreError::InvalidKey)
    ));
    assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 0);
}
