//! An actual Windows destination-sharing failure preserves the existing binding.
//! This exercises the native rename, rather than the after-replace error stub.
use super::*;
use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;
use windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION;
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

#[test]
fn windows_denied_pointer_replace_keeps_original_and_allows_exact_retry() {
    let fixture = Fixture::new("sharing-failure");
    let store = fixture.store();
    let expected = seed_legacy(&store);
    let operation = request(expected, "sharing-retry", desired_fields("next"));
    let path = pointer_path(&fixture.root);
    let before = all_records(&fixture.root);
    let before_pointer = fs::read(&path).unwrap();

    // Reads and writes remain shareable, but this private handle refuses deletion
    // and replacement of the current pointer until explicitly released.
    let blocker = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&path)
        .unwrap();
    let denied = OpenOptions::new()
        .access_mode(DELETE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(&path)
        .unwrap_err();
    assert_eq!(denied.raw_os_error(), Some(ERROR_SHARING_VIOLATION as i32));

    let result = store
        .begin_binding(load_request())
        .unwrap()
        .commit(operation.clone());
    assert!(
        matches!(&result, Err(VaultStoreError::Storage(message))
            if message.contains("the vault record could not be installed by its source handle")),
        "a failed native rename must be a known storage failure, not an acknowledged or unknown commit: {result:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), before_pointer);
    assert_eq!(current_fields(&store), old_fields());
    let retained = all_records(&fixture.root);
    for (name, bytes) in before {
        assert_eq!(
            retained.get(&name),
            Some(&bytes),
            "existing ciphertext changed: {name}"
        );
    }
    assert!(
        retained.keys().all(|name| !name.starts_with(".vault-")),
        "a known, unchanged destination permits cleanup of this attempt's temporary file"
    );
    drop(blocker);
    drop(store);

    // Reopen the actual filesystem store and explicitly retry the same immutable
    // operation. Partial staged fields must be reused, and old versions survive.
    let restarted = fixture.store();
    assert_eq!(current_fields(&restarted), old_fields());
    assert!(matches!(
        restarted
            .begin_binding(load_request())
            .unwrap()
            .commit(operation.clone())
            .unwrap(),
        VaultBindingCommitResult::Committed(_)
    ));
    assert_eq!(current_fields(&restarted), operation.fields);
    let mut tx = restarted.begin_binding(load_request()).unwrap();
    assert_eq!(
        tx.load_version_fields(&base_version()).unwrap(),
        old_fields()
    );
    drop(tx);
    let committed = all_records(&fixture.root);
    assert!(matches!(
        restarted
            .begin_binding(load_request())
            .unwrap()
            .commit(operation)
            .unwrap(),
        VaultBindingCommitResult::AlreadyCommitted(_)
    ));
    assert_eq!(all_records(&fixture.root), committed);
}
