//! Atomically created host-private temporary directories with validation-only shared parents.
use anyhow::{Context, Result, bail};
use rand::{RngCore, rngs::OsRng};
use std::fs;
#[cfg(windows)]
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

/// Allocate a private directory under the trusted system parent.
/// The now-public prefix is a single ASCII label, never a caller-supplied path.
pub fn create_private_temp_directory(prefix: &str) -> Result<PathBuf> {
    if prefix.is_empty()
        || !prefix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("private temporary directory prefix must be a nonempty ASCII label");
    }
    let base = trusted_temp_directory_base()?;
    for _ in 0..32_u32 {
        let mut random = [0_u8; 16];
        OsRng.fill_bytes(&mut random);
        let nonce = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let path = base.join(format!("{prefix}-{nonce}"));
        if create_private_directory(&path)? {
            #[cfg(unix)]
            secure_private_directory(&path)?;
            #[cfg(windows)]
            validate_private_directory(&path)
                .context("private temporary root verification after atomic creation failed")?;
            return Ok(path);
        }
    }
    bail!("failed to allocate a unique private temporary root")
}

#[cfg(unix)]
pub fn trusted_temp_directory_base() -> Result<PathBuf> {
    use std::os::unix::fs::MetadataExt;

    let base =
        fs::canonicalize("/tmp").context("trusted system temporary directory is unavailable")?;
    let metadata =
        fs::metadata(&base).context("failed to inspect trusted system temporary directory")?;
    if !metadata.is_dir() || metadata.uid() != 0 || metadata.mode() & 0o1000 == 0 {
        bail!("system temporary directory must be a root-owned sticky directory")
    }
    Ok(base)
}

#[cfg(windows)]
pub fn trusted_temp_directory_base() -> Result<PathBuf> {
    let local_app_data = crate::private_fs::windows_local_app_data_path()?;
    trusted_temp_directory_base_in(&local_app_data)
}

#[cfg(windows)]
fn trusted_temp_directory_base_in(local_app_data: &Path) -> Result<PathBuf> {
    // General application data may already contain inherited ACLs from normal initialization.
    // Keep transient helpers in their own private boundary; never repair or fall back to Akra.
    let base = local_app_data.join("AkraTrustedTemp");
    let created = create_private_directory(&base)?;
    // A concurrent creator publishes the private directory in one OS operation. Existing
    // roots are validation-only: resetting their inheritable ACL can race with children.
    validate_shared_private_directory(&base)
        .with_context(|| format!("trusted temp parent verification failed (created={created})"))?;
    Ok(base)
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> Result<bool> {
    use std::os::unix::fs::DirBuilderExt;

    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error).context("failed to create private temporary directory"),
    }
}

#[cfg(windows)]
fn create_private_directory(path: &Path) -> Result<bool> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;

    let parent = path
        .parent()
        .context("private directory parent is missing")?;
    let name = path
        .file_name()
        .context("private directory name is missing")?;
    let native_path = fs::canonicalize(parent)
        .context("failed to resolve private directory parent")?
        .join(name);
    let security =
        crate::private_fs::WindowsOwnerOnlySecurityDescriptor::for_current_user_directory()?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: security.as_raw(),
        bInheritHandle: 0,
    };
    let wide = native_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // SAFETY: the terminated path and private descriptor remain allocated for this call.
    if unsafe { CreateDirectoryW(wide.as_ptr(), &attributes) } != 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        Ok(false)
    } else {
        Err(error).context("CreateDirectoryW failed to create private temporary directory")
    }
}

#[cfg(unix)]
pub fn secure_private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect private directory `{}`", path.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("private directory must be a real directory")
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to secure private directory `{}`", path.display()))?;
    validate_private_directory(path)
}

#[cfg(unix)]
pub fn validate_private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to validate private directory `{}`", path.display()))?;
    // SAFETY: geteuid has no preconditions and does not retain pointers.
    let current = unsafe { libc::geteuid() };
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != current
        || metadata.mode() & 0o777 != 0o700
    {
        bail!("private directory ownership, type, or permissions are unsafe")
    }
    Ok(())
}

#[cfg(windows)]
pub fn validate_private_directory(path: &Path) -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    use crate::private_fs::{
        WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
        WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ, WINDOWS_READ_CONTROL,
        validate_windows_path_identity, validate_windows_private_owner_and_acl,
    };
    let directory = OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .with_context(|| format!("failed to validate private directory `{}`", path.display()))?;
    validate_windows_path_identity(path, &directory, true)?;
    validate_windows_private_owner_and_acl(path, &directory)
}

#[cfg(windows)]
fn validate_shared_private_directory(path: &Path) -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    use crate::private_fs::{
        WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
        WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ, WINDOWS_READ_CONTROL,
        validate_windows_shared_private_directory,
    };
    let directory = OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .context("failed to open shared private Windows directory")?;
    validate_windows_shared_private_directory(path, &directory)
}

#[cfg(all(test, windows))]
mod windows_tests;

#[cfg(test)]
mod tests {
    #[test]
    fn public_temp_prefix_cannot_select_a_path_or_stream() {
        for prefix in [
            "",
            "../outside",
            "/absolute",
            "C:/outside",
            "parent\\child",
            "name:stream",
        ] {
            let error = super::create_private_temp_directory(prefix).unwrap_err();
            assert!(error.to_string().contains("prefix must be"));
        }
    }
}
