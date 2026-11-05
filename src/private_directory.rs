//! 현재 사용자만 접근할 수 있는 디렉터리를 만들고 확인한다.
//!
//! Gateway 의 실행 디렉터리(`$AKRA_HOME/run`)는 소켓, 잠금 파일, upstream 소켓을 담는다. codex 는 upstream
//! 소켓 디렉터리가 현재 사용자 전용이 아니면 시작하지 않는다. unix 는 0700, Windows 는 상속을 끊은
//! 사용자 전용 ACL 을 쓴다.

use std::path::Path;

use anyhow::{Context, Result, bail};

pub(crate) fn ensure_private_directory(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)
        .with_context(|| format!("failed to create private directory `{}`", path.display()))?;
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect private directory `{}`", path.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!(
            "private directory `{}` must be a real directory",
            path.display()
        )
    }
    secure_private_directory(path)
}

#[cfg(unix)]
fn secure_private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to secure private directory `{}`", path.display()))?;
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("failed to validate private directory `{}`", path.display()))?;
    // SAFETY: geteuid has no preconditions and does not retain pointers.
    let current = unsafe { libc::geteuid() };
    if metadata.uid() != current || metadata.mode() & 0o777 != 0o700 {
        bail!(
            "private directory `{}` ownership or permissions are unsafe",
            path.display()
        )
    }
    Ok(())
}

#[cfg(windows)]
fn secure_private_directory(path: &Path) -> Result<()> {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    use crate::private_fs::{
        WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
        WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ, WINDOWS_READ_CONTROL, WINDOWS_WRITE_DAC,
        set_windows_private_acl, validate_windows_path_identity,
        validate_windows_private_owner_and_acl,
    };

    let directory = OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL | WINDOWS_WRITE_DAC)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .with_context(|| format!("failed to open private directory `{}`", path.display()))?;
    validate_windows_path_identity(path, &directory, true)?;
    set_windows_private_acl(&directory, true)?;
    validate_windows_private_owner_and_acl(path, &directory)
}

#[cfg(test)]
mod tests {
    use super::ensure_private_directory;

    #[test]
    fn creates_nested_directory_private_to_current_user() {
        let root = tempfile_root("akra-private-directory");
        let target = root.join("run").join("nested");

        ensure_private_directory(&target).expect("private directory should be created");
        // Running twice keeps the directory valid instead of failing on an existing path.
        ensure_private_directory(&target).expect("existing private directory should stay valid");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&target).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    fn tempfile_root(prefix: &str) -> std::path::PathBuf {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }
}
