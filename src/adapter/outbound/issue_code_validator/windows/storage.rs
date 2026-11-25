use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::FromRawHandle;
use std::path::{Component, Path, PathBuf, Prefix};
use std::ptr::null_mut;

use sha2::{Digest, Sha256};
use windows_sys::Win32::Foundation::{GENERIC_WRITE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CREATE_NEW, CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, GetDriveTypeW,
};
use windows_sys::Win32::System::WindowsProgramming::DRIVE_FIXED;

use super::security::{Descriptor, Identity, wide};
use crate::application::port::outbound::issue_code_validator_runner_port::{
    IssueCodeValidatorNotRun as NotRun, IssueCodeValidatorRunRequest,
};

const MAX_VALIDATOR: u64 = 8 * 1024 * 1024;
pub(super) const MAX_INPUT: usize = 1024 * 1024;

pub(super) fn approved_content(request: &IssueCodeValidatorRunRequest) -> Result<Vec<u8>, NotRun> {
    let path = Path::new(&request.command);
    // A drive-qualified local path only: no UNC, device namespace, PATH search or relative path.
    if !path.is_absolute()
        || !matches!(path.components().next(), Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(letter) if letter.is_ascii_alphabetic()))
    {
        return Err(NotRun::Missing);
    }
    let Component::Prefix(prefix) = path.components().next().ok_or(NotRun::Missing)? else {
        return Err(NotRun::Missing);
    };
    let Prefix::Disk(letter) = prefix.kind() else {
        return Err(NotRun::Missing);
    };
    let drive = wide(format!("{}:\\", char::from(letter)).as_ref()).map_err(|_| NotRun::Missing)?;
    // SAFETY: a terminated drive root; mapped network/removable sources are unsupported.
    if unsafe { GetDriveTypeW(drive.as_ptr()) } != DRIVE_FIXED {
        return Err(NotRun::Missing);
    }
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| NotRun::Missing)?;
    let metadata = file.metadata().map_err(|_| NotRun::Missing)?;
    if !metadata.is_file()
        || metadata.len() > MAX_VALIDATOR
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || crate::private_fs::validate_windows_trusted_executable_path_identity(path, &file, false)
            .is_err()
    {
        return Err(NotRun::Missing);
    }
    let mut content = Vec::new();
    file.take(MAX_VALIDATOR + 1)
        .read_to_end(&mut content)
        .map_err(|_| NotRun::Missing)?;
    if content.len() as u64 > MAX_VALIDATOR
        || format!("{:x}", Sha256::digest(&content)) != request.sha256
    {
        return Err(NotRun::HashMismatch);
    }
    if !native_headless_pe(&content) {
        return Err(NotRun::IsolationUnavailable);
    }
    Ok(content)
}

/// Deliberately narrow format: AMD64 PE32+ Windows-subsystem executable, not DLL or managed CLR image.
/// Dependency access is still decided by the LPAC token, never by a host-side interpreter.
fn native_headless_pe(content: &[u8]) -> bool {
    let u16_at = |offset: usize| {
        content
            .get(offset..offset.saturating_add(2))
            .and_then(|value| <[u8; 2]>::try_from(value).ok())
            .map(u16::from_le_bytes)
    };
    let u32_at = |offset: usize| {
        content
            .get(offset..offset.saturating_add(4))
            .and_then(|value| <[u8; 4]>::try_from(value).ok())
            .map(u32::from_le_bytes)
    };
    if content.get(..2) != Some(b"MZ") {
        return false;
    }
    let Some(offset) = u32_at(0x3c).map(|value| value as usize) else {
        return false;
    };
    if offset > content.len().saturating_sub(24)
        || content.get(offset..offset + 4) != Some(b"PE\0\0")
        || u16_at(offset + 4) != Some(0x8664)
        || !u16_at(offset + 6).is_some_and(|count| (1..=96).contains(&count))
        || !u16_at(offset + 22).is_some_and(|flags| flags & 0x0002 != 0 && flags & 0x2000 == 0)
    {
        return false;
    }
    let header = offset + 24;
    let Some(size) = u16_at(offset + 20).map(usize::from) else {
        return false;
    };
    if size < 112
        || header
            .checked_add(size)
            .is_none_or(|end| end > content.len())
        || u16_at(header) != Some(0x20b)
        || u16_at(header + 68) != Some(2)
    {
        return false;
    }
    let Some(directories) = u32_at(header + 108) else {
        return false;
    };
    directories <= 14
        || (size >= 232
            && u32_at(header + 112 + 14 * 8) == Some(0)
            && u32_at(header + 116 + 14 * 8) == Some(0))
}

pub(super) struct Workspace {
    pub(super) path: PathBuf,
    pub(super) executable: PathBuf,
    pub(super) input: Option<File>,
    executable_lock: Option<File>,
    directory_lock: Option<File>,
}
impl Workspace {
    pub(super) fn create(identity: &Identity, content: &[u8], input: &str) -> io::Result<Self> {
        let temp = crate::private_fs::windows_local_app_data_path()
            .map_err(io::Error::other)?
            .join("Temp");
        let parent = OpenOptions::new()
            .read(true)
            .custom_flags(
                crate::private_fs::WINDOWS_FILE_FLAG_BACKUP_SEMANTICS
                    | FILE_FLAG_OPEN_REPARSE_POINT,
            )
            .open(&temp)?;
        crate::private_fs::validate_windows_path_identity(&temp, &parent, true)
            .map_err(io::Error::other)?;
        let path = temp.join(&identity.name);
        let descriptor = identity.workspace_descriptor()?;
        let attributes = descriptor.attributes(false);
        let path_wide = wide(path.as_os_str())?;
        // SAFETY: both security descriptor and terminated path live through creation.
        if unsafe { CreateDirectoryW(path_wide.as_ptr(), &attributes) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut workspace = Self {
            executable: path.join("validator.exe"),
            path,
            input: None,
            executable_lock: None,
            directory_lock: None,
        };
        workspace.directory_lock = Some(
            OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ)
                .custom_flags(
                    crate::private_fs::WINDOWS_FILE_FLAG_BACKUP_SEMANTICS
                        | FILE_FLAG_OPEN_REPARSE_POINT,
                )
                .open(&workspace.path)?,
        );
        write_new(&workspace.executable, content, &descriptor)?;
        workspace.executable_lock = Some(read_locked(&workspace.executable)?);
        let input_path = workspace.path.join("input.json");
        write_new(&input_path, input.as_bytes(), &descriptor)?;
        workspace.input = Some(read_locked(&input_path)?);
        Ok(workspace)
    }

    pub(super) fn cleanup(&mut self) -> io::Result<()> {
        self.input.take();
        self.executable_lock.take();
        self.directory_lock.take();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match fs::remove_dir_all(&self.path) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                // Windows can retain an image/file reference briefly after the Job is empty.
                // Retry only sharing/lock violations on this invocation's already-owned stage.
                Err(error)
                    if matches!(error.raw_os_error(), Some(32 | 33))
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub(super) fn input(&self) -> io::Result<&File> {
        self.input
            .as_ref()
            .ok_or_else(|| io::Error::other("validator input is unavailable"))
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        // Error-path fallback; successful outcomes already checked cleanup explicitly.
        let _ = self.cleanup();
    }
}

pub(super) fn write_new(path: &Path, bytes: &[u8], descriptor: &Descriptor) -> io::Result<()> {
    let text = wide(path.as_os_str())?;
    let attributes = descriptor.attributes(false);
    // SAFETY: CREATE_NEW prevents replacement; this new object gets its ACL atomically.
    let raw = unsafe {
        CreateFileW(
            text.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ,
            &attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: ownership of the unique file handle transfers to File.
    let mut file = unsafe { File::from_raw_handle(raw) };
    file.write_all(bytes)?;
    file.sync_all()
}

fn read_locked(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::native_headless_pe;

    #[test]
    fn scripts_dlls_managed_images_and_truncated_pe_are_not_native_validators() {
        assert!(!native_headless_pe(b"@echo off\r\nexit 0"));
        let mut content = vec![0u8; 512];
        content[..2].copy_from_slice(b"MZ");
        content[0x3c..0x40].copy_from_slice(&64u32.to_le_bytes());
        content[64..68].copy_from_slice(b"PE\0\0");
        content[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
        content[70..72].copy_from_slice(&1u16.to_le_bytes());
        content[84..86].copy_from_slice(&240u16.to_le_bytes());
        content[86..88].copy_from_slice(&2u16.to_le_bytes());
        content[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
        content[156..158].copy_from_slice(&2u16.to_le_bytes());
        content[196..200].copy_from_slice(&16u32.to_le_bytes());
        assert!(native_headless_pe(&content));
        content[156..158].copy_from_slice(&3u16.to_le_bytes());
        assert!(!native_headless_pe(&content));
        content[156..158].copy_from_slice(&2u16.to_le_bytes());
        assert!(!native_headless_pe(&content[..240]));
        content[86..88].copy_from_slice(&0x2002u16.to_le_bytes());
        assert!(!native_headless_pe(&content));
        content[86..88].copy_from_slice(&2u16.to_le_bytes());
        content[312..316].copy_from_slice(&4096u32.to_le_bytes());
        assert!(!native_headless_pe(&content));
    }
    #[test]
    fn owned_workspace_cleanup_retries_transient_sharing_without_leaving_files() {
        use std::fs::OpenOptions;
        use std::os::windows::fs::OpenOptionsExt;
        use std::time::Duration;
        let identity = super::Identity::create().unwrap();
        let mut workspace = super::Workspace::create(&identity, b"synthetic stage", "{}").unwrap();
        let path = workspace.path.clone();
        let blocker = OpenOptions::new()
            .read(true)
            .share_mode(super::FILE_SHARE_READ)
            .open(&workspace.executable)
            .unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            drop(blocker);
        });
        let cleanup = workspace.cleanup();
        release.join().unwrap();
        cleanup.unwrap();
        assert!(!path.exists());
    }
}
