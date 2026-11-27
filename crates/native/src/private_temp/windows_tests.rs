use std::fs::{self, File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Barrier;
use std::time::Duration;

use super::{
    create_private_directory, trusted_temp_directory_base_in, validate_private_directory,
    validate_shared_private_directory,
};
use crate::private_fs::{
    WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
    WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ, WINDOWS_READ_CONTROL,
    windows_file_identity_snapshot,
};

struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "akra-trusted-temp-contract-{:032x}",
            rand::random::<u128>()
        ));
        assert!(create_private_directory(&path).unwrap());
        validate_private_directory(&path).unwrap();
        Self(path)
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn directory_handle(path: &Path) -> File {
    OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .unwrap()
}

fn run_fixture_command(command: &mut Command, label: &str) -> Output {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let output = crate::subprocess::wait_with_output_timeout(
        crate::subprocess::spawn(command).unwrap(),
        label,
        Duration::from_secs(10),
    )
    .unwrap();
    // ACL tools may print principals. Keep the fixture's failure report limited to the exit code.
    assert!(output.status.success(), "{label}: status={}", output.status);
    output
}

fn directory_acl(path: &Path) -> Vec<u8> {
    let mut command = Command::new("icacls.exe");
    command.arg(path);
    run_fixture_command(&mut command, "trusted-temp ACL snapshot").stdout
}

#[test]
fn existing_shared_temp_root_is_verified_without_acl_mutation() {
    let root = OwnedRoot::new();
    let base = trusted_temp_directory_base_in(&root.0).unwrap();
    let before = windows_file_identity_snapshot(&directory_handle(&base)).unwrap();
    let acl = directory_acl(&base);
    for _ in 0..4 {
        assert_eq!(trusted_temp_directory_base_in(&root.0).unwrap(), base);
    }
    assert_eq!(
        before,
        windows_file_identity_snapshot(&directory_handle(&base)).unwrap(),
        "existing root metadata must remain unchanged"
    );
    assert!(
        acl == directory_acl(&base),
        "existing root ACL must remain unchanged"
    );
}

#[test]
fn inherited_application_tree_is_compatible_and_legacy_temp_is_untouched() {
    let root = OwnedRoot::new();
    let akra = root.0.join("Akra");
    let extensions = akra.join("extensions");
    let local_user = extensions.join("local-user");
    // Match kernel_extensions::build_local: create ancestors, then protect only the leaf.
    crate::private_directory::ensure_private_directory(&local_user).unwrap();
    let legacy_temp = akra.join("trusted-temp");
    fs::create_dir(&legacy_temp).unwrap();
    fs::write(legacy_temp.join("sentinel"), b"legacy must stay untouched").unwrap();
    let inherited = validate_private_directory(&akra).unwrap_err();
    assert!(
        format!("{inherited:#}").contains("dacl_protected=false"),
        "normal application fixture must retain its inherited ACL"
    );
    let paths = [&akra, &extensions, &local_user, &legacy_temp];
    let before = paths.map(|path| windows_file_identity_snapshot(&directory_handle(path)).unwrap());
    let acls_before = paths.map(|path| directory_acl(path));

    let base = trusted_temp_directory_base_in(&root.0).unwrap();
    assert_eq!(base, root.0.join("AkraTrustedTemp"));
    validate_private_directory(&base).unwrap();
    assert_eq!(trusted_temp_directory_base_in(&root.0).unwrap(), base);

    let after = paths.map(|path| windows_file_identity_snapshot(&directory_handle(path)).unwrap());
    let acls_after = paths.map(|path| directory_acl(path));
    assert_eq!(
        before, after,
        "legacy application metadata must remain unchanged"
    );
    assert!(
        acls_before == acls_after,
        "legacy ACLs must remain unchanged"
    );
    assert_eq!(
        fs::read(legacy_temp.join("sentinel")).unwrap(),
        b"legacy must stay untouched"
    );
    assert_eq!(fs::read_dir(&legacy_temp).unwrap().count(), 1);
    assert_eq!(fs::read_dir(&base).unwrap().count(), 0);
}

#[test]
fn concurrent_first_use_publishes_private_parents_and_children_atomically() {
    let root = OwnedRoot::new();
    let barrier = Barrier::new(8);
    std::thread::scope(|scope| {
        let workers = (0..8)
            .map(|worker| {
                let root = &root.0;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    for iteration in 0..4 {
                        let base = trusted_temp_directory_base_in(root).unwrap();
                        let child = base.join(format!("worker-{worker}-{iteration}"));
                        assert!(create_private_directory(&child).unwrap());
                        validate_private_directory(&child).unwrap();
                        validate_shared_private_directory(&base).unwrap();
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker
                .join()
                .expect("private temp creation worker should finish");
        }
    });
    let base = trusted_temp_directory_base_in(&root.0).unwrap();
    assert_eq!(fs::read_dir(base).unwrap().count(), 32);
}

#[test]
fn shared_parent_verification_rejects_replacement_after_the_handle_is_opened() {
    let root = OwnedRoot::new();
    let base = trusted_temp_directory_base_in(&root.0).unwrap();
    let original = directory_handle(&base);
    let replaced = root.0.join("original-trusted-temp");
    fs::rename(&base, &replaced).unwrap();
    assert!(create_private_directory(&base).unwrap());
    validate_shared_private_directory(&base).unwrap();
    let error =
        crate::private_fs::validate_windows_shared_private_directory(&base, &original).unwrap_err();
    assert!(
        error.to_string().contains("same non-reparse object"),
        "{error:#}"
    );
    assert_eq!(fs::read_dir(&base).unwrap().count(), 0);
    assert_eq!(fs::read_dir(&replaced).unwrap().count(), 0);
}

#[test]
fn existing_inherited_root_is_rejected_without_repair_or_legacy_fallback() {
    let root = OwnedRoot::new();
    let akra = root.0.join("Akra");
    assert!(create_private_directory(&akra).unwrap());
    let legacy_temp = akra.join("trusted-temp");
    assert!(create_private_directory(&legacy_temp).unwrap());
    validate_private_directory(&legacy_temp).unwrap();
    let legacy_before = windows_file_identity_snapshot(&directory_handle(&legacy_temp)).unwrap();
    let untrusted = root.0.join("AkraTrustedTemp");
    fs::create_dir(&untrusted).unwrap();
    let before = windows_file_identity_snapshot(&directory_handle(&untrusted)).unwrap();
    let error = trusted_temp_directory_base_in(&root.0).unwrap_err();
    let diagnostic = format!("{error:#}");
    assert!(diagnostic.contains("created=false"), "{diagnostic}");
    assert!(diagnostic.contains("owner_matches=true"), "{diagnostic}");
    assert!(diagnostic.contains("dacl_protected=false"), "{diagnostic}");
    assert!(
        !diagnostic.contains("S-1-"),
        "diagnostics must not expose SID values"
    );
    assert!(validate_private_directory(&untrusted).is_err());
    assert_eq!(
        before,
        windows_file_identity_snapshot(&directory_handle(&untrusted)).unwrap()
    );
    assert_eq!(
        legacy_before,
        windows_file_identity_snapshot(&directory_handle(&legacy_temp)).unwrap()
    );
    assert_eq!(fs::read_dir(&untrusted).unwrap().count(), 0);
    assert_eq!(fs::read_dir(&legacy_temp).unwrap().count(), 0);
}

#[test]
fn existing_extra_principal_acl_is_rejected_with_safe_header_diagnostics() {
    let root = OwnedRoot::new();
    let base = trusted_temp_directory_base_in(&root.0).unwrap();
    let mut command = Command::new("icacls.exe");
    command.arg(&base).args(["/grant", "*S-1-1-0:(R)"]);
    run_fixture_command(&mut command, "trusted-temp extra ACE fixture");
    let before = windows_file_identity_snapshot(&directory_handle(&base)).unwrap();
    let error = trusted_temp_directory_base_in(&root.0).unwrap_err();
    let diagnostic = format!("{error:#}");
    for expected in [
        "owner_matches=true",
        "dacl_protected=true",
        "ace_count=Some(2)",
        "GetSecurityDescriptorControl_error=None",
        "GetAclInformation_error=None",
    ] {
        assert!(diagnostic.contains(expected), "{diagnostic}");
    }
    assert!(
        !diagnostic.contains("S-1-"),
        "diagnostics must not expose SID values"
    );
    assert!(validate_private_directory(&base).is_err());
    assert_eq!(
        before,
        windows_file_identity_snapshot(&directory_handle(&base)).unwrap()
    );
}

#[test]
fn existing_parent_junction_is_rejected_without_touching_the_target() {
    let root = OwnedRoot::new();
    let target = OwnedRoot::new();
    fs::write(target.0.join("sentinel"), b"unchanged").unwrap();
    let junction = root.0.join("AkraTrustedTemp");
    let mut command = Command::new("cmd");
    command
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&target.0);
    run_fixture_command(&mut command, "trusted-temp junction fixture");
    assert!(trusted_temp_directory_base_in(&root.0).is_err());
    assert_eq!(fs::read(target.0.join("sentinel")).unwrap(), b"unchanged");
    assert_eq!(fs::read_dir(&target.0).unwrap().count(), 1);
    fs::remove_dir(junction).unwrap();
}
