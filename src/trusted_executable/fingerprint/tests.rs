use super::*;
use std::io::Write;

fn fixture() -> (PathBuf, PathBuf, PathBuf) {
    #[cfg(unix)]
    let base = PathBuf::from(std::env::var_os("HOME").unwrap()).join(".cache");
    #[cfg(windows)]
    let base = crate::private_fs::windows_local_app_data_path()
        .unwrap()
        .join("Akra")
        .join("tests");
    let root = base.join(format!(
        "native-fingerprint-{}-{:016x}",
        std::process::id(),
        rand::random::<u64>()
    ));
    crate::private_directory::ensure_private_directory(&root).unwrap();
    let context = std::env::current_dir().unwrap();
    let native = resolve_native_from_current_path("git", &context).unwrap();
    let path = root.join(if cfg!(windows) {
        "pinned.exe"
    } else {
        "pinned"
    });
    fs::copy(native, &path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    (root, path, context)
}

#[test]
fn pinned_native_format_and_digest_describe_the_same_bytes() {
    let (root, path, context) = fixture();
    let pinned = PinnedNativeExecutable::inspect(&path, &context).unwrap();
    assert_eq!(
        pinned.sha256(),
        format!("{:x}", Sha256::digest(fs::read(&path).unwrap()))
    );
    pinned.verify().unwrap();
    drop(pinned);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_modified_pinned_executable_cannot_keep_its_prepared_identity() {
    let (root, path, context) = fixture();
    let pinned = PinnedNativeExecutable::inspect(&path, &context).unwrap();
    let mut writer = fs::OpenOptions::new().write(true).open(&path).unwrap();
    writer.write_all(b"not a native executable").unwrap();
    writer.sync_all().unwrap();
    assert!(pinned.verify().is_err());
    assert!(PinnedNativeExecutable::inspect(&path, &context).is_err());
    drop(writer);
    drop(pinned);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_script_is_rejected_before_it_can_be_fingerprinted_as_a_native_runner() {
    let (root, path, context) = fixture();
    fs::write(&path, b"#!/bin/sh\nexit 0\n").unwrap();
    assert!(PinnedNativeExecutable::inspect(&path, &context).is_err());
    fs::remove_dir_all(root).unwrap();
}
