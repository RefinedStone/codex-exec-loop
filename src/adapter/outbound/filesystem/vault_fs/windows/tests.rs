use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;

use super::super::tests::Fixture;
use super::super::{
    MAX_FILE_BYTES, compare_and_swap_file, prepare_writer_root, read_optional_file,
    write_file_atomic,
};
use super::{directory, files, test_hooks, write};
use crate::private_fs::{
    WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT, WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ,
    WINDOWS_GENERIC_WRITE, WINDOWS_READ_CONTROL,
};

#[path = "tests/processes.rs"]
mod processes;
#[path = "tests/safety.rs"]
mod safety;

#[test]
fn unsupported_volume_admission_precedes_every_creation() {
    let fixture = Fixture::unprepared();
    let root = fixture.root.join("nested").join("vault");
    test_hooks::reject_next_volume_admission();
    assert!(prepare_writer_root(&root).is_err());
    assert!(!fixture.root.exists());
    prepare_writer_root(&fixture.root).unwrap();
    test_hooks::reject_next_volume_admission();
    assert!(write_file_atomic(&fixture.root, &Fixture::relative(), b"ciphertext").is_err());
    assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 0);
}

#[test]
fn native_no_replace_keeps_both_open_source_and_existing_destination() {
    let fixture = Fixture::unprepared();
    let mut long_root = fixture.root.clone();
    for index in 0..3 {
        long_root.push(format!("한글 space {index}-{}", "x".repeat(80)));
    }
    assert!(long_root.as_os_str().encode_wide().count() > 260);
    prepare_writer_root(&long_root).unwrap();
    let root = directory::open_root(&long_root).unwrap().unwrap();
    let records = directory::open_records(&root, true).unwrap().unwrap();
    let source_name = ".vault-native-source.tmp";
    let destination = Fixture::relative()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let mut source =
        files::create_private_file(&records, source_name, files::NewFilePurpose::Record).unwrap();
    source.write_all(b"new ciphertext").unwrap();
    source.sync_all().unwrap();
    let mut existing =
        files::create_private_file(&records, &destination, files::NewFilePurpose::Record).unwrap();
    existing.write_all(b"old ciphertext").unwrap();
    existing.sync_all().unwrap();
    drop(existing);
    let source_identity = files::ObjectIdentity::of(&source).unwrap();
    let error = write::rename_handle(&source, &records, &destination, false).unwrap_err();
    assert!(files::is_already_exists(&error));
    assert_eq!(files::ObjectIdentity::of(&source).unwrap(), source_identity);
    assert_eq!(
        fs::read(records.path().join(source_name)).unwrap(),
        b"new ciphertext"
    );
    assert_eq!(
        fs::read(records.path().join(&destination)).unwrap(),
        b"old ciphertext"
    );
    write::rename_handle(&source, &records, &destination, true).unwrap();
    source.sync_all().unwrap();
    files::verify_named_file(&records, &destination, &source).unwrap();
    assert!(!records.path().join(source_name).exists());
    assert_eq!(files::ObjectIdentity::of(&source).unwrap(), source_identity);
    assert_eq!(
        files::read_snapshot(&records, &destination)
            .unwrap()
            .unwrap()
            .body,
        "new ciphertext",
    );
}

#[test]
fn bounded_reader_rejects_invalid_utf8_and_oversized_private_records() {
    let fixture = Fixture::new();
    let relative = Fixture::relative();
    write_file_atomic(&fixture.root, &relative, b"valid ciphertext").unwrap();
    let path = fixture.root.join(&relative);
    fs::write(&path, [0xff, 0xfe]).unwrap();
    assert!(read_optional_file(&fixture.root, &relative).is_err());
    let file = OpenOptions::new()
        .write(true)
        .access_mode(WINDOWS_GENERIC_WRITE | WINDOWS_READ_CONTROL)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
        .unwrap();
    file.set_len(MAX_FILE_BYTES as u64 + 1).unwrap();
    drop(file);
    assert!(read_optional_file(&fixture.root, &relative).is_err());
}

#[test]
fn mutation_during_a_read_cannot_return_an_obsolete_ciphertext_snapshot() {
    let fixture = Fixture::new();
    let relative = Fixture::relative();
    write_file_atomic(&fixture.root, &relative, b"old ciphertext").unwrap();
    let path = fixture.root.join(&relative);
    test_hooks::install_after_record_read_hook(move || {
        fs::write(path, b"new ciphertext").unwrap();
    });
    assert!(read_optional_file(&fixture.root, &relative).is_err());
    assert_eq!(
        read_optional_file(&fixture.root, &relative)
            .unwrap()
            .as_deref(),
        Some("new ciphertext"),
    );
}

#[test]
fn pinned_root_and_source_handles_reject_namespace_replacement() {
    let fixture = Fixture::new();
    let root = directory::open_root(&fixture.root).unwrap().unwrap();
    let records = directory::open_records(&root, true).unwrap().unwrap();
    let file =
        files::create_private_file(&records, ".vault-held.tmp", files::NewFilePurpose::Record)
            .unwrap();
    assert!(fs::rename(&fixture.root, fixture.base.join("moved")).is_err());
    assert!(
        fs::rename(
            records.path().join(".vault-held.tmp"),
            records.path().join("moved.tmp")
        )
        .is_err()
    );
    assert!(
        OpenOptions::new()
            .write(true)
            .access_mode(WINDOWS_GENERIC_WRITE | WINDOWS_GENERIC_READ)
            .share_mode(WINDOWS_FILE_SHARE_ALL)
            .open(records.path().join(".vault-held.tmp"))
            .is_err()
    );
    files::verify_named_file(&records, ".vault-held.tmp", &file).unwrap();
}

#[test]
fn replacing_a_hard_link_record_is_rejected_without_changing_either_name() {
    let fixture = Fixture::new();
    let relative = Fixture::relative();
    write_file_atomic(&fixture.root, &relative, b"retained ciphertext").unwrap();
    let path = fixture.root.join(&relative);
    let alias = fixture.base.join("alias.json");
    fs::hard_link(&path, &alias).unwrap();
    assert!(read_optional_file(&fixture.root, &relative).is_err());
    assert!(write_file_atomic(&fixture.root, &relative, b"new ciphertext").is_err());
    assert!(
        compare_and_swap_file(
            &fixture.root,
            &relative,
            Some("retained ciphertext"),
            "new ciphertext",
        )
        .is_err()
    );
    assert_eq!(fs::read(path).unwrap(), b"retained ciphertext");
    assert_eq!(fs::read(alias).unwrap(), b"retained ciphertext");
}
