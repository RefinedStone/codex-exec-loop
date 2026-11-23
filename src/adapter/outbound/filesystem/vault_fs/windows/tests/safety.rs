use std::fs::{self, File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SetKernelObjectSecurity,
};

use super::Fixture;
use crate::adapter::outbound::filesystem::vault_fs::{
    VaultBindingLease, compare_and_swap_file, prepare_writer_root, read_optional_file,
    write_file_atomic,
};
use crate::private_fs::{
    WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
    WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ, WINDOWS_READ_CONTROL, WINDOWS_WRITE_DAC,
    validate_windows_private_owner_and_acl,
};

fn open_acl_handle(path: &Path, directory: bool) -> File {
    OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL | WINDOWS_WRITE_DAC)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(
            WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT
                | if directory {
                    WINDOWS_FILE_FLAG_BACKUP_SEMANTICS
                } else {
                    0
                },
        )
        .open(path)
        .unwrap()
}

fn grant_world_access(path: &Path, directory: bool) {
    let file = open_acl_handle(path, directory);
    let sddl: Vec<u16> = "D:P(A;;FA;;;WD)"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut descriptor = std::ptr::null_mut();
    // SAFETY: the constant synthetic DACL and output pointer are valid.
    assert_ne!(
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        },
        0
    );
    // SAFETY: only this fixture object is modified, through its WRITE_DAC handle.
    let changed = unsafe {
        SetKernelObjectSecurity(
            file.as_raw_handle(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        )
    };
    // SAFETY: the descriptor was allocated by the conversion API above.
    unsafe {
        LocalFree(descriptor.cast());
    }
    assert_ne!(changed, 0);
    assert!(validate_windows_private_owner_and_acl(path, &file).is_err());
}

fn assert_still_unsafe(path: &Path, directory: bool) {
    let file = open_acl_handle(path, directory);
    assert!(validate_windows_private_owner_and_acl(path, &file).is_err());
}

#[test]
fn existing_unsafe_root_acl_is_rejected_without_repair_or_children() {
    let fixture = Fixture::new();
    grant_world_access(&fixture.root, true);
    assert!(prepare_writer_root(&fixture.root).is_err());
    assert!(read_optional_file(&fixture.root, &Fixture::relative()).is_err());
    assert!(write_file_atomic(&fixture.root, &Fixture::relative(), b"ciphertext").is_err());
    assert_still_unsafe(&fixture.root, true);
    assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 0);
}

#[test]
fn existing_unsafe_record_and_records_directory_acls_are_not_repaired() {
    let fixture = Fixture::new();
    let relative = Fixture::relative();
    write_file_atomic(&fixture.root, &relative, b"retained ciphertext").unwrap();
    let file = fixture.root.join(&relative);
    grant_world_access(&file, false);
    assert!(read_optional_file(&fixture.root, &relative).is_err());
    assert!(
        compare_and_swap_file(
            &fixture.root,
            &relative,
            Some("retained ciphertext"),
            "new ciphertext",
        )
        .is_err()
    );
    assert_still_unsafe(&file, false);
    assert_eq!(fs::read(&file).unwrap(), b"retained ciphertext");
    let records = fixture.root.join("records");
    grant_world_access(&records, true);
    assert!(write_file_atomic(&fixture.root, &relative, b"new ciphertext").is_err());
    assert_still_unsafe(&records, true);
    assert_eq!(fs::read(file).unwrap(), b"retained ciphertext");
}

#[test]
fn a_hard_link_binding_lock_cannot_create_a_second_lock_domain() {
    let fixture = Fixture::new();
    let name = Fixture::lock_name();
    let lease =
        VaultBindingLease::acquire(fixture.root.clone(), name.clone(), Duration::ZERO).unwrap();
    drop(lease);
    let path = fixture.root.join(&name);
    let alias = fixture.base.join("lock-alias");
    fs::hard_link(&path, &alias).unwrap();
    assert!(VaultBindingLease::acquire(fixture.root.clone(), name, Duration::ZERO).is_err());
    assert_eq!(fs::metadata(path).unwrap().len(), 0);
    assert_eq!(fs::metadata(alias).unwrap().len(), 0);
}

#[test]
fn network_device_and_relative_roots_are_rejected_before_lookup() {
    for root in [
        r"\\vault.invalid\share\vault",
        r"\\?\UNC\vault.invalid\share\vault",
        r"\\?\GLOBALROOT\Device\HarddiskVolume1\vault",
        r"relative\vault",
    ] {
        assert!(prepare_writer_root(Path::new(root)).is_err());
        assert!(read_optional_file(Path::new(root), &Fixture::relative()).is_err());
    }
}

fn create_junction(link: &Path, target: &Path) {
    let link = link
        .to_str()
        .unwrap()
        .strip_prefix(r"\\?\")
        .unwrap_or_else(|| link.to_str().unwrap());
    let target = target
        .to_str()
        .unwrap()
        .strip_prefix(r"\\?\")
        .unwrap_or_else(|| target.to_str().unwrap());
    // cmd expands percent variables even inside quotes. Synthetic fixture paths
    // must not contain them; /D and /V:OFF disable startup and delayed expansion.
    assert!(!link.contains(['"', '%', '\r', '\n']));
    assert!(!target.contains(['"', '%', '\r', '\n']));
    let mut command = Command::new("cmd");
    command
        .args(["/D", "/V:OFF", "/C"])
        .raw_arg(format!("mklink /J \"{link}\" \"{target}\""))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = crate::subprocess::spawn(&mut command).unwrap();
    let started = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "the synthetic directory junction could not be created"
            );
            return;
        }
        if started.elapsed() >= Duration::from_secs(10) {
            child.terminate_and_wait().unwrap();
            panic!("the synthetic directory junction helper did not exit");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_junction_ancestor_is_rejected_before_creating_the_vault_root() {
    let fixture = Fixture::unprepared();
    let target = fixture.base.join("retained-target");
    crate::private_directory::ensure_private_directory(&target).unwrap();
    fs::write(target.join("sentinel"), b"retained").unwrap();
    let junction = fixture.base.join("redirect");
    create_junction(&junction, &target);
    assert!(prepare_writer_root(&junction.join("new-vault")).is_err());
    assert!(read_optional_file(&junction.join("new-vault"), &Fixture::relative()).is_err());
    assert_eq!(fs::read(target.join("sentinel")).unwrap(), b"retained");
    assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
    fs::remove_dir(junction).unwrap();
}
