//! Test-only staging: keep the product trust boundary around repository binaries.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

use crate::private_fs::{
    WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT, WINDOWS_GENERIC_READ, WINDOWS_GENERIC_WRITE,
    WINDOWS_READ_CONTROL, WINDOWS_WRITE_DAC, set_windows_private_acl,
    validate_windows_path_identity, validate_windows_private_owner_and_acl,
    validate_windows_trusted_executable_path_identity,
};

pub(super) fn stage(source: &Path, directory: &Path, cwd: &Path) -> Result<PathBuf> {
    ensure!(
        source.is_absolute(),
        "manual helper source must be absolute"
    );
    // An existing writer or a concurrent write/delete makes this open fail.
    // Hold the source handle until the copied bytes have been verified.
    let mut original = OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
        .open(source)
        .context("open the manual helper source without write/delete sharing")?;
    // Cargo may hard-link target/debug to deps; the held handle still fixes its bytes.
    validate_windows_trusted_executable_path_identity(source, &original, false)?;
    crate::private_directory::ensure_private_directory(directory)?;
    let destination = directory.join("akra-gateway.exe");
    let mut copied = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .access_mode(
            WINDOWS_GENERIC_READ | WINDOWS_GENERIC_WRITE | WINDOWS_READ_CONTROL | WINDOWS_WRITE_DAC,
        )
        .share_mode(FILE_SHARE_READ)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&destination)
        .context("create a new manual helper; never replace an existing file")?;
    set_windows_private_acl(&copied, false)?;
    std::io::copy(&mut original, &mut copied)?;
    copied.sync_all()?;
    ensure!(
        digest(&mut original)? == digest(&mut copied)?,
        "manual helper copy differs from its opened source"
    );
    validate_windows_path_identity(&destination, &copied, false)?;
    validate_windows_private_owner_and_acl(&destination, &copied)?;
    let validated = crate::trusted_executable::validate_absolute(&destination, cwd)?;
    crate::trusted_executable::validate_native_executable(&validated)?;
    Ok(validated)
}

fn digest(file: &mut File) -> Result<[u8; 32]> {
    file.rewind()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize().into())
}

#[test]
fn repository_helper_is_staged_without_weakening_trust_or_replacing_a_copy() {
    let folder = super::Folder::new();
    let repository = folder.0.join("repository");
    let target = repository.join("target").join("debug");
    crate::private_directory::ensure_private_directory(&target).unwrap();
    std::fs::create_dir(repository.join(".git")).unwrap();
    let artifact = target.join("akra-gateway-build.exe");
    crate::trusted_executable::copy_native_executable_fixture(&artifact).unwrap();
    let source = target.join("akra-gateway.exe");
    std::fs::hard_link(artifact, &source).unwrap();
    assert!(crate::trusted_executable::validate_absolute(&source, &repository).is_err());

    let destination = folder.0.join("helper");
    let staged = stage(&source, &destination, &repository).unwrap();
    let expected = digest(&mut File::open(&source).unwrap()).unwrap();
    assert_eq!(digest(&mut File::open(&staged).unwrap()).unwrap(), expected);
    assert_eq!(
        crate::trusted_executable::validate_absolute(&staged, &repository).unwrap(),
        staged
    );
    crate::trusted_executable::validate_native_executable(&staged).unwrap();
    assert!(stage(&source, &destination, &repository).is_err());
    assert_eq!(digest(&mut File::open(staged).unwrap()).unwrap(), expected);
}

#[test]
fn invalid_native_source_is_rejected_without_running_a_helper() {
    let folder = super::Folder::new();
    let source = folder.0.join("not-a-native-helper.exe");
    std::fs::write(&source, b"not a native executable").unwrap();
    let cwd = std::env::current_dir().unwrap();
    assert!(stage(&source, &folder.0.join("helper"), &cwd).is_err());
    assert!(stage(Path::new("relative.exe"), &folder.0.join("relative"), &cwd).is_err());
}
