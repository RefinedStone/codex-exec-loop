use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};

use anyhow::{Context, Result, bail};
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateFileW, DELETE, FILE_ATTRIBUTE_NORMAL,
    FILE_FLAG_WRITE_THROUGH, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
};

use crate::private_fs::{
    WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT, WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ,
    WINDOWS_GENERIC_WRITE, WINDOWS_READ_CONTROL, WindowsFileIdentitySnapshot,
    WindowsOwnerOnlySecurityDescriptor, validate_windows_identity_snapshot,
    validate_windows_path_identity, validate_windows_private_owner_and_acl,
    windows_file_identity_snapshot,
};
use crate::private_sqlite::PinnedPrivateDirectory;

use super::super::MAX_FILE_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ObjectIdentity {
    volume: u32,
    index: u64,
}

impl ObjectIdentity {
    pub(super) fn of(file: &File) -> Result<Self> {
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: the live handle and initialized output structure are valid.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
            return Err(std::io::Error::last_os_error())
                .context("the vault file identity could not be inspected");
        }
        // This key is used only while the source handle remains open on admitted NTFS.
        // Rename may change other metadata; the handle prevents file ID reuse.
        Ok(Self {
            volume: information.dwVolumeSerialNumber,
            index: (u64::from(information.nFileIndexHigh) << 32)
                | u64::from(information.nFileIndexLow),
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct RecordSnapshot {
    pub(super) body: String,
    pub(super) object: ObjectIdentity,
    identity: WindowsFileIdentitySnapshot,
}

pub(super) fn read_snapshot(
    parent: &PinnedPrivateDirectory,
    name: &str,
) -> Result<Option<RecordSnapshot>> {
    parent.verify()?;
    let path = parent.file_path(name)?;
    let mut file = match OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            parent.verify()?;
            return Ok(None);
        }
        Err(error) => return Err(error).context("the encrypted vault record could not be opened"),
    };
    verify_named_file(parent, name, &file)?;
    let identity = windows_file_identity_snapshot(&file)?;
    let object = ObjectIdentity::of(&file)?;
    let length = file.metadata()?.len();
    if length > MAX_FILE_BYTES as u64 {
        bail!("the encrypted vault record exceeds its byte limit");
    }
    let mut bytes = Vec::with_capacity(length as usize);
    (&mut file)
        .take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .context("the encrypted vault record could not be read")?;
    if bytes.len() as u64 != length || bytes.len() > MAX_FILE_BYTES {
        bail!("the encrypted vault record changed size during its bounded read");
    }
    #[cfg(test)]
    super::test_hooks::after_record_read();
    validate_windows_identity_snapshot(identity, &file, false)?;
    verify_named_file(parent, name, &file)?;
    let body = String::from_utf8(bytes).context("the encrypted vault record is not UTF-8")?;
    Ok(Some(RecordSnapshot {
        body,
        object,
        identity,
    }))
}

pub(super) fn verify_named_file(
    parent: &PinnedPrivateDirectory,
    name: &str,
    file: &File,
) -> Result<()> {
    parent.verify()?;
    let path = parent.file_path(name)?;
    validate_windows_path_identity(&path, file, false)?;
    validate_windows_private_owner_and_acl(&path, file)?;
    parent.verify()
}

#[derive(Clone, Copy)]
pub(super) enum NewFilePurpose {
    Record,
    Lease,
}

pub(super) fn create_private_file(
    parent: &PinnedPrivateDirectory,
    name: &str,
    purpose: NewFilePurpose,
) -> Result<File> {
    parent.verify()?;
    parent.file_path(name)?;
    // Canonicalization supplies the extended Win32 spelling, after all ancestors
    // have been pinned without delete sharing. It does not change record/AAD keys.
    let path = std::fs::canonicalize(parent.path())
        .context("the pinned vault directory could not be resolved")?
        .join(name);
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let security = WindowsOwnerOnlySecurityDescriptor::for_current_user()?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: security.as_raw(),
        bInheritHandle: 0,
    };
    let (access, sharing) = match purpose {
        NewFilePurpose::Record => (DELETE, FILE_SHARE_READ),
        NewFilePurpose::Lease => (0, FILE_SHARE_READ | FILE_SHARE_WRITE),
    };
    // SAFETY: the NUL-terminated path and immutable owner-only descriptor remain
    // live. CREATE_NEW never opens or truncates an existing entry.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            WINDOWS_GENERIC_READ | WINDOWS_GENERIC_WRITE | WINDOWS_READ_CONTROL | access,
            sharing,
            &attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL | WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_WRITE_THROUGH,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error())
            .context("the private vault file could not be created");
    }
    // SAFETY: CreateFileW returned one owned, non-inheritable handle.
    let file = unsafe { File::from_raw_handle(handle) };
    verify_named_file(parent, name, &file)?;
    Ok(file)
}

pub(super) fn open_existing_lease(parent: &PinnedPrivateDirectory, name: &str) -> Result<File> {
    parent.verify()?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_GENERIC_WRITE | WINDOWS_READ_CONTROL)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
        .open(parent.file_path(name)?)
        .context("the existing vault lease could not be opened")?;
    verify_named_file(parent, name, &file)?;
    Ok(file)
}

pub(super) fn is_already_exists(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<std::io::Error>())
        .any(|cause| cause.kind() == std::io::ErrorKind::AlreadyExists)
}
