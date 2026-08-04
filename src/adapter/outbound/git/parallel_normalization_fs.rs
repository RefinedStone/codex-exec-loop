use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::application::port::outbound::parallel_mode_runtime_port::ParallelPinnedDirectory;

struct GitParallelPinnedDirectory {
    path: PathBuf,
    opened: File,
}

impl ParallelPinnedDirectory for GitParallelPinnedDirectory {
    fn verify(&self) -> Result<(), String> {
        verify_pinned_staging_directory(&self.path, &self.opened)
    }
}

pub(super) fn create_private_staging_directory(
    path: &Path,
) -> Result<Box<dyn ParallelPinnedDirectory>, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        builder.create(path).map_err(|error| {
            format!(
                "normalization replacement staging could not be claimed at `{}`: {error}",
                path.display()
            )
        })?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir(path).map_err(|error| {
        format!(
            "normalization replacement staging could not be claimed at `{}`: {error}",
            path.display()
        )
    })?;

    let staging = GitParallelPinnedDirectory {
        path: path.to_path_buf(),
        opened: open_pinned_staging_directory(path)?,
    };
    staging.verify()?;
    Ok(Box::new(staging))
}

#[cfg(unix)]
fn open_pinned_staging_directory(path: &Path) -> Result<File, String> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| {
            format!(
                "normalization replacement staging could not be pinned at `{}`: {error}",
                path.display()
            )
        })
}

#[cfg(windows)]
fn open_pinned_staging_directory(path: &Path) -> Result<File, String> {
    use std::os::windows::fs::OpenOptionsExt;

    use crate::private_fs::{
        WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
        WINDOWS_GENERIC_READ, WINDOWS_READ_CONTROL,
    };

    const FILE_SHARE_READ_WRITE: u32 = 0x0000_0001 | 0x0000_0002;
    OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        .share_mode(FILE_SHARE_READ_WRITE)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .map_err(|error| {
            format!(
                "normalization replacement staging could not be pinned at `{}`: {error}",
                path.display()
            )
        })
}

#[cfg(not(any(unix, windows)))]
fn open_pinned_staging_directory(path: &Path) -> Result<File, String> {
    Err(format!(
        "normalization replacement staging identity pinning is unsupported at `{}`",
        path.display()
    ))
}

#[cfg(unix)]
fn verify_pinned_staging_directory(path: &Path, opened: &File) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;

    let opened_metadata = opened.metadata().map_err(|error| {
        format!(
            "normalization replacement staging handle could not be inspected at `{}`: {error}",
            path.display()
        )
    })?;
    let path_metadata = path.symlink_metadata().map_err(|error| {
        format!(
            "normalization replacement staging path could not be reinspected at `{}`: {error}",
            path.display()
        )
    })?;
    if !opened_metadata.is_dir()
        || !path_metadata.is_dir()
        || path_metadata.file_type().is_symlink()
        || opened_metadata.dev() != path_metadata.dev()
        || opened_metadata.ino() != path_metadata.ino()
        || opened_metadata.uid() != unsafe { libc::geteuid() }
        || path_metadata.uid() != opened_metadata.uid()
        || opened_metadata.mode() & 0o077 != 0
        || path_metadata.mode() & 0o077 != 0
    {
        return Err(format!(
            "normalization replacement staging identity changed or is not private at `{}`",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn verify_pinned_staging_directory(path: &Path, opened: &File) -> Result<(), String> {
    crate::private_fs::validate_windows_path_identity_only(path, opened, true).map_err(|error| {
        format!(
            "normalization replacement staging identity changed at `{}`: {error}",
            path.display()
        )
    })
}

#[cfg(not(any(unix, windows)))]
fn verify_pinned_staging_directory(path: &Path, _opened: &File) -> Result<(), String> {
    Err(format!(
        "normalization replacement staging identity verification is unsupported at `{}`",
        path.display()
    ))
}

pub(super) fn read_bounded_unshared_regular_file(path: &Path, max_bytes: usize) -> Option<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }

    let file = options.open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.file_type().is_file()
        || metadata.len() > max_bytes as u64
        || !opened_file_has_one_link_and_no_reparse(&file, &metadata)
    {
        return None;
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= max_bytes).then_some(bytes)
}

#[cfg(unix)]
fn opened_file_has_one_link_and_no_reparse(_file: &File, metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    metadata.nlink() == 1
}

#[cfg(windows)]
fn opened_file_has_one_link_and_no_reparse(file: &File, metadata: &std::fs::Metadata) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    use std::os::windows::fs::MetadataExt;

    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
        && crate::private_fs::windows_file_link_count(file).is_ok_and(|links| links == 1)
}

#[cfg(not(any(unix, windows)))]
fn opened_file_has_one_link_and_no_reparse(_file: &File, _metadata: &std::fs::Metadata) -> bool {
    true
}

#[cfg(target_vendor = "apple")]
pub(super) fn atomic_rename_noreplace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let result =
        unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(any(all(target_os = "linux", target_env = "gnu"), target_os = "android"))]
pub(super) fn atomic_rename_noreplace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(all(target_os = "linux", not(target_env = "gnu")))]
pub(super) fn atomic_rename_noreplace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
pub(super) fn atomic_rename_noreplace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileW;

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    if unsafe { MoveFileW(source.as_ptr(), destination.as_ptr()) } != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(any(
    target_vendor = "apple",
    target_os = "linux",
    target_os = "android",
    windows
)))]
pub(super) fn atomic_rename_noreplace(_source: &Path, _destination: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "atomic no-replace worktree moves are unsupported on this platform",
    ))
}
