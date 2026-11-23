use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use super::*;

pub(super) struct Fixture {
    pub(super) base: PathBuf,
    pub(super) root: PathBuf,
}

impl Fixture {
    pub(super) fn unprepared() -> Self {
        let base = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-vault-files-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        crate::private_directory::ensure_private_directory(&base).unwrap();
        let base = fs::canonicalize(base).unwrap();
        Self {
            root: base.join("vault"),
            base,
        }
    }

    pub(super) fn new() -> Self {
        let fixture = Self::unprepared();
        prepare_writer_root(&fixture.root).unwrap();
        fixture
    }

    pub(super) fn relative() -> PathBuf {
        Path::new("records").join(format!("{}.json", "a".repeat(64)))
    }

    pub(super) fn lock_name() -> String {
        format!("evidence-vault-binding-{}.lock", "b".repeat(64))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

#[test]
fn missing_reads_do_not_prepare_roots_records_or_locks() {
    let fixture = Fixture::unprepared();
    assert_eq!(
        read_optional_file(&fixture.root, &Fixture::relative()).unwrap(),
        None
    );
    assert!(!fixture.root.exists());
    prepare_writer_root(&fixture.root).unwrap();
    assert_eq!(
        read_optional_file(&fixture.root, &Fixture::relative()).unwrap(),
        None
    );
    assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 0);
}

#[test]
fn only_record_paths_and_bounded_bodies_reach_the_filesystem() {
    let fixture = Fixture::unprepared();
    for relative in [
        "../outside.json",
        "records/../outside.json",
        "records/./aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json",
        "records/short.json",
        "records/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA.json",
        "records/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json:stream",
        "/records/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json",
    ] {
        let relative = Path::new(relative);
        assert!(read_optional_file(&fixture.root, relative).is_err());
        assert!(write_file_atomic(&fixture.root, relative, b"synthetic").is_err());
        assert!(compare_and_swap_file(&fixture.root, relative, None, "synthetic").is_err());
    }
    assert!(
        write_file_atomic(
            &fixture.root,
            &Fixture::relative(),
            &vec![b'x'; MAX_FILE_BYTES + 1],
        )
        .is_err()
    );
    assert!(!fixture.root.exists());
}

#[test]
fn encrypted_bytes_round_trip_and_ciphertext_cas_preserves_conflicts() {
    let fixture = Fixture::new();
    let relative = Fixture::relative();
    let first = r#"{"ciphertext":"synthetic 한글"}"#;
    let second = r#"{"ciphertext":"synthetic second"}"#;
    assert!(compare_and_swap_file(&fixture.root, &relative, None, first).unwrap());
    assert!(!compare_and_swap_file(&fixture.root, &relative, None, second).unwrap());
    assert!(!compare_and_swap_file(&fixture.root, &relative, Some("stale"), second).unwrap());
    assert_eq!(
        read_optional_file(&fixture.root, &relative)
            .unwrap()
            .as_deref(),
        Some(first)
    );
    assert!(compare_and_swap_file(&fixture.root, &relative, Some(first), second).unwrap());
    assert!(compare_and_swap_file(&fixture.root, &relative, Some(second), second).unwrap());
    assert_eq!(
        read_optional_file(&fixture.root, &relative)
            .unwrap()
            .as_deref(),
        Some(second)
    );
    assert_eq!(
        fs::read_dir(fixture.root.join("records")).unwrap().count(),
        1
    );
}

#[test]
fn noncooperating_ciphertext_change_is_detected_before_installation() {
    let fixture = Fixture::new();
    let relative = Fixture::relative();
    write_file_atomic(&fixture.root, &relative, b"old ciphertext").unwrap();
    let path = fixture.root.join(&relative);
    install_before_atomic_replace_hook(move || fs::write(path, b"external ciphertext").unwrap());
    assert!(
        !compare_and_swap_file(
            &fixture.root,
            &relative,
            Some("old ciphertext"),
            "proposed ciphertext",
        )
        .unwrap()
    );
    assert_eq!(
        read_optional_file(&fixture.root, &relative)
            .unwrap()
            .as_deref(),
        Some("external ciphertext"),
    );
    assert_eq!(
        fs::read_dir(fixture.root.join("records")).unwrap().count(),
        1
    );
}

#[test]
fn post_installation_failure_keeps_new_ciphertext_and_reports_unknown() {
    let fixture = Fixture::new();
    let relative = Fixture::relative();
    write_file_atomic(&fixture.root, &relative, b"old ciphertext").unwrap();
    install_after_atomic_replace_error_hook("synthetic flush failure");
    let error = compare_and_swap_file(
        &fixture.root,
        &relative,
        Some("old ciphertext"),
        "new ciphertext",
    )
    .unwrap_err();
    assert!(write_outcome_unknown(&error));
    assert_eq!(
        read_optional_file(&fixture.root, &relative)
            .unwrap()
            .as_deref(),
        Some("new ciphertext"),
    );
    assert_eq!(
        fs::read_dir(fixture.root.join("records")).unwrap().count(),
        1
    );
}

#[test]
fn binding_lease_serializes_handles_and_retains_the_lock_file() {
    let fixture = Fixture::new();
    let name = Fixture::lock_name();
    let first =
        VaultBindingLease::acquire(fixture.root.clone(), name.clone(), Duration::ZERO).unwrap();
    first.verify_identity().unwrap();
    assert!(
        VaultBindingLease::acquire(
            fixture.root.clone(),
            name.clone(),
            Duration::from_millis(50),
        )
        .is_err()
    );
    drop(first);
    let second =
        VaultBindingLease::acquire(fixture.root.clone(), name.clone(), Duration::ZERO).unwrap();
    second.verify_identity().unwrap();
    drop(second);
    assert!(fixture.root.join(name).is_file());
}

#[test]
fn invalid_binding_lock_name_does_not_create_an_entry() {
    let fixture = Fixture::new();
    for name in [
        "../outside.lock",
        "other.lock",
        "evidence-vault-binding-short.lock",
    ] {
        assert!(VaultBindingLease::acquire(fixture.root.clone(), name, Duration::ZERO).is_err());
    }
    assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 0);
}
