#![cfg(windows)]

use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result, anyhow};

pub(crate) const WINDOWS_FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
pub(crate) const WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
pub(crate) const WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
pub(crate) const WINDOWS_FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
pub(crate) const WINDOWS_FILE_SHARE_ALL: u32 = 0x0000_0001 | 0x0000_0002 | 0x0000_0004;
pub(crate) const WINDOWS_GENERIC_READ: u32 = 0x8000_0000;
pub(crate) const WINDOWS_GENERIC_WRITE: u32 = 0x4000_0000;
pub(crate) const WINDOWS_READ_CONTROL: u32 = 0x0002_0000;
pub(crate) const WINDOWS_WRITE_DAC: u32 = 0x0004_0000;

pub(crate) fn validate_windows_path_identity(
    path: &Path,
    opened: &File,
    directory: bool,
) -> Result<()> {
    validate_windows_path_identity_only(path, opened, directory)?;
    validate_windows_owner(path, opened)
}

/// A shared OS temporary parent may gain children during verification. Require the
/// current owner and stable directory identity without requiring an owner-only DACL.
/// This policy must not be used for files, stores, or owned temporary children.
pub(crate) fn validate_windows_shared_directory_path_identity(
    path: &Path,
    opened: &File,
) -> Result<()> {
    validate_windows_shared_directory_with_security(path, opened, validate_windows_owner)
}

/// Shared private temporary roots additionally require a protected owner-only DACL
/// on both handles. Keep this stricter boundary separate from the shared OS Temp parent.
pub(crate) fn validate_windows_shared_private_directory(path: &Path, opened: &File) -> Result<()> {
    validate_windows_shared_directory_with_security(
        path,
        opened,
        validate_windows_private_owner_and_acl,
    )
}

fn validate_windows_shared_directory_with_security(
    path: &Path,
    opened: &File,
    verify_security: fn(&Path, &File) -> Result<()>,
) -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    let before = windows_file_identity_snapshot(opened)?;
    validate_windows_shared_directory_snapshots(before, before)?;
    verify_security(path, opened)?;
    let reopened = std::fs::OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .with_context(|| {
            format!(
                "failed to reopen shared Windows directory {}",
                path.display()
            )
        })?;
    validate_windows_shared_directory_snapshots(
        before,
        windows_file_identity_snapshot(&reopened)?,
    )?;
    verify_security(path, &reopened)?;
    // Recheck both handles after security queries without treating a new child as replacement.
    validate_windows_shared_directory_snapshots(before, windows_file_identity_snapshot(opened)?)?;
    validate_windows_shared_directory_snapshots(before, windows_file_identity_snapshot(&reopened)?)
}

fn validate_windows_shared_directory_snapshots(
    expected: WindowsFileIdentitySnapshot,
    current: WindowsFileIdentitySnapshot,
) -> Result<()> {
    // Child creation can grow NTFS directory metadata (observed size 0 -> 4096 -> 8192),
    // in addition to updating write/change times. None of these three fields identifies
    // a shared directory. Keep volume/file ID, creation time, attributes and link count strict.
    // Existing file/store/owned-child validators still compare the complete snapshot.
    let content_metadata_ignored = WindowsFileIdentitySnapshot {
        size: expected.size,
        last_write_time: expected.last_write_time,
        change_time: expected.change_time,
        ..current
    };
    let expected_directory = expected.attributes & WINDOWS_FILE_ATTRIBUTE_DIRECTORY != 0;
    let current_directory = current.attributes & WINDOWS_FILE_ATTRIBUTE_DIRECTORY != 0;
    let expected_reparse = expected.attributes & WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT != 0;
    let current_reparse = current.attributes & WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT != 0;
    if !expected_directory
        || !current_directory
        || expected_reparse
        || current_reparse
        || expected != content_metadata_ignored
    {
        return Err(anyhow!(
            "shared Windows directory handles do not identify the same non-reparse object \
             (attributes_changed={}, volume_changed={}, file_id_changed={}, \
             creation_time_changed={}, link_count_changed={}, size_changed={}, \
             last_write_time_changed={}, change_time_changed={}, \
             expected_directory={}, current_directory={}, expected_reparse={}, current_reparse={})",
            expected.attributes != current.attributes,
            expected.volume_serial_number != current.volume_serial_number,
            expected.file_index != current.file_index,
            expected.creation_time != current.creation_time,
            expected.number_of_links != current.number_of_links,
            expected.size != current.size,
            expected.last_write_time != current.last_write_time,
            expected.change_time != current.change_time,
            expected_directory,
            current_directory,
            expected_reparse,
            current_reparse,
        ));
    }
    Ok(())
}

pub(crate) fn validate_windows_path_identity_only(
    path: &Path,
    opened: &File,
    directory: bool,
) -> Result<()> {
    validate_windows_path_identity_with_link_policy(path, opened, directory, true)
}

pub(crate) fn validate_windows_trusted_executable_path_identity(
    path: &Path,
    opened: &File,
    directory: bool,
) -> Result<()> {
    validate_windows_path_identity_with_link_policy(path, opened, directory, false)
}

fn validate_windows_path_identity_with_link_policy(
    path: &Path,
    opened: &File,
    directory: bool,
    require_single_link: bool,
) -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    let opened_identity = windows_file_identity_snapshot(opened)?;
    let path_handle = std::fs::OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
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
        .with_context(|| format!("failed to reopen Windows path {}", path.display()))?;
    validate_windows_handle_identity_with_link_policy(
        opened,
        &path_handle,
        directory,
        require_single_link,
    )
    .with_context(|| {
        if require_single_link {
            format!(
                "private Windows path must be a stable non-reparse, single-link object: {}",
                path.display()
            )
        } else {
            format!(
                "trusted Windows executable path must be a stable non-reparse object: {}",
                path.display()
            )
        }
    })?;
    validate_windows_identity_snapshot_with_link_policy(
        opened_identity,
        &path_handle,
        directory,
        require_single_link,
    )
    .with_context(|| {
        if require_single_link {
            format!(
                "private Windows path must be a stable non-reparse, single-link object: {}",
                path.display()
            )
        } else {
            format!(
                "trusted Windows executable path must be a stable non-reparse object: {}",
                path.display()
            )
        }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WindowsFileIdentitySnapshot {
    attributes: u32,
    volume_serial_number: u32,
    file_index: u64,
    number_of_links: u32,
    creation_time: u64,
    size: u64,
    last_write_time: u64,
    change_time: i64,
}

pub(crate) fn windows_file_identity_snapshot(file: &File) -> Result<WindowsFileIdentitySnapshot> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO, FileBasicInfo, GetFileInformationByHandle,
        GetFileInformationByHandleEx,
    };

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the file handle and output structure remain valid for the duration of the call.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut information) } == 0
    {
        return Err(std::io::Error::last_os_error())
            .context("failed to inspect private Windows file handle");
    }
    let mut basic = FILE_BASIC_INFO::default();
    // SAFETY: the file handle and FILE_BASIC_INFO output buffer remain valid for the call.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle() as HANDLE,
            FileBasicInfo,
            (&mut basic as *mut FILE_BASIC_INFO).cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error())
            .context("failed to inspect private Windows file change time");
    }
    Ok(WindowsFileIdentitySnapshot {
        attributes: information.dwFileAttributes,
        volume_serial_number: information.dwVolumeSerialNumber,
        file_index: (u64::from(information.nFileIndexHigh) << 32)
            | u64::from(information.nFileIndexLow),
        number_of_links: information.nNumberOfLinks,
        creation_time: (u64::from(information.ftCreationTime.dwHighDateTime) << 32)
            | u64::from(information.ftCreationTime.dwLowDateTime),
        size: (u64::from(information.nFileSizeHigh) << 32) | u64::from(information.nFileSizeLow),
        last_write_time: (u64::from(information.ftLastWriteTime.dwHighDateTime) << 32)
            | u64::from(information.ftLastWriteTime.dwLowDateTime),
        change_time: basic.ChangeTime,
    })
}

pub(crate) fn windows_file_identity_key(file: &File) -> Result<String> {
    let identity = windows_file_identity_snapshot(file)?;
    Ok(format!(
        "windows:{}:{}:{}",
        identity.volume_serial_number, identity.file_index, identity.creation_time
    ))
}

pub(crate) fn windows_file_link_count(file: &File) -> Result<u32> {
    Ok(windows_file_identity_snapshot(file)?.number_of_links)
}

pub(crate) fn windows_local_app_data_path() -> Result<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;

    use windows_sys::Win32::Foundation::RPC_E_CHANGED_MODE;
    use windows_sys::Win32::Security::{TOKEN_DUPLICATE, TOKEN_IMPERSONATE, TOKEN_QUERY};
    use windows_sys::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows_sys::Win32::UI::Shell::{FOLDERID_LocalAppData, SHGetKnownFolderPath};

    struct ComGuard(bool);
    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: this thread owes exactly one uninitialize for a successful call.
                unsafe { windows_sys::Win32::System::Com::CoUninitialize() };
            }
        }
    }
    struct ComPath(*mut u16);
    impl Drop for ComPath {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: SHGetKnownFolderPath returns CoTaskMem on success or failure.
                unsafe { windows_sys::Win32::System::Com::CoTaskMemFree(self.0.cast()) };
            }
        }
    }

    // Known-folder APIs require COM on the calling thread. An existing STA remains owned
    // by its caller; S_OK and S_FALSE both require balancing our own initialization.
    // SAFETY: the reserved pointer is null and the apartment choice is valid.
    let com_status = unsafe { CoInitializeEx(std::ptr::null(), COINIT_MULTITHREADED as u32) };
    let _com = if com_status >= 0 {
        ComGuard(true)
    } else if com_status == RPC_E_CHANGED_MODE {
        ComGuard(false)
    } else {
        return Err(anyhow!(
            "failed to initialize COM for Windows LocalAppData lookup (HRESULT {com_status:#x})"
        ));
    };

    let mut raw_token = std::ptr::null_mut();
    // Keep every Akra known-folder lookup on an explicit OS account token. A NULL-token
    // lookup can populate Shell's process-wide cache from a USERPROFILE override.
    // SAFETY: the pseudo process handle is valid and the output points to live storage.
    if unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY | TOKEN_IMPERSONATE | TOKEN_DUPLICATE,
            &mut raw_token,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error())
            .context("failed to open current user token for Windows LocalAppData lookup");
    }
    let token = WindowsHandle(raw_token);
    let mut raw_path = std::ptr::null_mut();
    // Keep flags at zero: retrieve the current, possibly redirected folder without creating it.
    // SAFETY: COM is initialized, token is owned, and all pointers remain valid for the call.
    let status = unsafe { SHGetKnownFolderPath(&FOLDERID_LocalAppData, 0, token.0, &mut raw_path) };
    let allocation = ComPath(raw_path);
    if status < 0 || allocation.0.is_null() {
        return Err(anyhow!(
            "failed to resolve Windows LocalAppData known folder (HRESULT {status:#x})"
        ));
    }
    let mut length = 0usize;
    // SAFETY: a successful query supplies a NUL-terminated UTF-16 string held by allocation.
    while unsafe { *allocation.0.add(length) } != 0 {
        length = length
            .checked_add(1)
            .context("Windows LocalAppData path length overflowed")?;
    }
    // SAFETY: the preceding scan established the live allocation's string length.
    let path =
        std::ffi::OsString::from_wide(unsafe { std::slice::from_raw_parts(allocation.0, length) });
    let path = std::path::PathBuf::from(path);
    if !path.is_absolute() {
        return Err(anyhow!(
            "Windows LocalAppData known folder must be absolute"
        ));
    }
    Ok(path)
}

fn validate_windows_handle_identity_with_link_policy(
    expected: &File,
    current: &File,
    directory: bool,
    require_single_link: bool,
) -> Result<()> {
    let expected = windows_file_identity_snapshot(expected)?;
    validate_windows_identity_snapshot_with_link_policy(
        expected,
        current,
        directory,
        require_single_link,
    )
}

pub(crate) fn validate_windows_identity_snapshot(
    expected: WindowsFileIdentitySnapshot,
    current: &File,
    directory: bool,
) -> Result<()> {
    validate_windows_identity_snapshot_with_link_policy(expected, current, directory, true)
}

fn validate_windows_identity_snapshot_with_link_policy(
    expected: WindowsFileIdentitySnapshot,
    current: &File,
    directory: bool,
    require_single_link: bool,
) -> Result<()> {
    let current = windows_file_identity_snapshot(current)?;
    let expected_type = if directory {
        expected.attributes & WINDOWS_FILE_ATTRIBUTE_DIRECTORY != 0
            && current.attributes & WINDOWS_FILE_ATTRIBUTE_DIRECTORY != 0
    } else {
        expected.attributes & WINDOWS_FILE_ATTRIBUTE_DIRECTORY == 0
            && current.attributes & WINDOWS_FILE_ATTRIBUTE_DIRECTORY == 0
            && (!require_single_link
                || expected.number_of_links == 1 && current.number_of_links == 1)
    };
    if expected.attributes & WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT != 0
        || current.attributes & WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT != 0
        || !expected_type
        || expected != current
    {
        return Err(anyhow!(
            "private Windows handles do not identify the same unchanged object"
        ));
    }
    Ok(())
}

struct WindowsHandle(windows_sys::Win32::Foundation::HANDLE);

impl Drop for WindowsHandle {
    fn drop(&mut self) {
        // SAFETY: this wrapper is created only for a successful owned token handle.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

struct WindowsLocalAllocation(windows_sys::Win32::Foundation::HLOCAL);

impl Drop for WindowsLocalAllocation {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: Windows security APIs allocate this pointer with LocalAlloc.
            unsafe {
                windows_sys::Win32::Foundation::LocalFree(self.0);
            }
        }
    }
}

struct WindowsCurrentUserSid {
    _buffer: Vec<usize>,
    sid: windows_sys::Win32::Security::PSID,
}

fn windows_current_user_sid() -> Result<WindowsCurrentUserSid> {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut raw_token: HANDLE = std::ptr::null_mut();
    // SAFETY: output points to valid storage and the pseudo process handle is always valid.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw_token) } == 0 {
        return Err(std::io::Error::last_os_error())
            .context("failed to open current process token");
    }
    let token = WindowsHandle(raw_token);
    let mut required = 0u32;
    // SAFETY: the null-buffer probe is the documented way to obtain the required size.
    unsafe {
        GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut required);
    }
    if required < std::mem::size_of::<TOKEN_USER>() as u32 {
        return Err(anyhow!(
            "Windows current-user token did not expose a user SID"
        ));
    }
    let word_count = (required as usize).div_ceil(std::mem::size_of::<usize>());
    let mut buffer = vec![0usize; word_count];
    // SAFETY: the aligned buffer is at least `required` bytes and remains alive in the result.
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error()).context("failed to read current-user SID");
    }
    // SAFETY: a successful TokenUser query initializes a TOKEN_USER at the buffer start.
    let sid = unsafe { (*(buffer.as_ptr().cast::<TOKEN_USER>())).User.Sid };
    if sid.is_null() {
        return Err(anyhow!("Windows current-user SID was null"));
    }
    Ok(WindowsCurrentUserSid {
        _buffer: buffer,
        sid,
    })
}

pub(crate) fn validate_windows_owner(path: &Path, file: &File) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
    use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{EqualSid, OWNER_SECURITY_INFORMATION, PSID};

    let current_user = windows_current_user_sid()?;
    let mut owner: PSID = std::ptr::null_mut();
    let mut descriptor = std::ptr::null_mut();
    // SAFETY: all out-pointers are valid and the file handle remains open for the call.
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle() as HANDLE,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(status as i32))
            .with_context(|| format!("failed to read Windows owner for {}", path.display()));
    }
    let _descriptor = WindowsLocalAllocation(descriptor.cast());
    // SAFETY: both SIDs come from validated Windows security API responses.
    if owner.is_null() || unsafe { EqualSid(owner, current_user.sid) } == 0 {
        return Err(anyhow!(
            "private Windows object must be owned by the current user: {}",
            path.display()
        ));
    }
    Ok(())
}

pub(crate) fn set_windows_private_acl(file: &File, directory: bool) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
    use windows_sys::Win32::Security::Authorization::{
        EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, SET_ACCESS, SetEntriesInAclW,
        SetSecurityInfo, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, NO_INHERITANCE, PROTECTED_DACL_SECURITY_INFORMATION,
        SUB_CONTAINERS_AND_OBJECTS_INHERIT,
    };
    use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;

    let current_user = windows_current_user_sid()?;
    let trustee = TRUSTEE_W {
        pMultipleTrustee: std::ptr::null_mut(),
        MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
        TrusteeForm: TRUSTEE_IS_SID,
        TrusteeType: TRUSTEE_IS_USER,
        ptstrName: current_user.sid.cast(),
    };
    let access = EXPLICIT_ACCESS_W {
        grfAccessPermissions: FILE_ALL_ACCESS,
        grfAccessMode: SET_ACCESS,
        grfInheritance: if directory {
            SUB_CONTAINERS_AND_OBJECTS_INHERIT
        } else {
            NO_INHERITANCE
        },
        Trustee: trustee,
    };
    let mut acl = std::ptr::null_mut();
    // SAFETY: the access entry and ACL out-pointer are valid for the duration of the call.
    let acl_status = unsafe { SetEntriesInAclW(1, &access, std::ptr::null(), &mut acl) };
    if acl_status != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(acl_status as i32))
            .context("failed to build private Windows ACL");
    }
    let _acl = WindowsLocalAllocation(acl.cast());
    // SAFETY: the file handle is open with WRITE_DAC and `acl` remains allocated here.
    let set_status = unsafe {
        SetSecurityInfo(
            file.as_raw_handle() as HANDLE,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            acl,
            std::ptr::null(),
        )
    };
    if set_status != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(set_status as i32))
            .context("failed to apply private Windows ACL");
    }
    Ok(())
}

pub(crate) fn validate_windows_private_owner_and_acl(path: &Path, file: &File) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
    use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation,
        GetSecurityDescriptorControl, OWNER_SECURITY_INFORMATION, PSID, SE_DACL_PROTECTED,
    };
    use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;

    let current_user = windows_current_user_sid()?;
    let mut owner: PSID = std::ptr::null_mut();
    let mut acl: *mut ACL = std::ptr::null_mut();
    let mut descriptor = std::ptr::null_mut();
    // SAFETY: all out-pointers are valid and the file handle remains open for the call.
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle() as HANDLE,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            &mut acl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(status as i32)).with_context(|| {
            format!(
                "GetSecurityInfo failed for private Windows ACL (win32_error={status}): {}",
                path.display()
            )
        });
    }
    let _descriptor = WindowsLocalAllocation(descriptor.cast());
    let mut control = 0u16;
    let mut revision = 0u32;
    let mut acl_size = ACL_SIZE_INFORMATION::default();
    let owner_present = !owner.is_null();
    let dacl_present = !acl.is_null();
    let descriptor_present = !descriptor.is_null();
    // SAFETY: non-null owner and current-user SIDs belong to live Windows allocations.
    let owner_matches = owner_present && unsafe { EqualSid(owner, current_user.sid) } != 0;
    let mut control_error = None;
    let mut acl_information_error = None;
    // Inspect independent fields even when another check fails, without logging any SID.
    // Capture GetLastError immediately; the next API call may overwrite it.
    let control_read = descriptor_present && {
        // SAFETY: descriptor belongs to the live GetSecurityInfo allocation.
        let success =
            unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } != 0;
        if !success {
            control_error = std::io::Error::last_os_error().raw_os_error();
        }
        success
    };
    let acl_information_read = dacl_present && {
        // SAFETY: ACL and output buffer remain live throughout this call.
        let success = unsafe {
            GetAclInformation(
                acl,
                (&mut acl_size as *mut ACL_SIZE_INFORMATION).cast(),
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        } != 0;
        if !success {
            acl_information_error = std::io::Error::last_os_error().raw_os_error();
        }
        success
    };
    let dacl_protected = control_read && control & SE_DACL_PROTECTED != 0;
    let ace_count = acl_information_read.then_some(acl_size.AceCount);
    if !descriptor_present || !owner_matches || !dacl_protected || ace_count != Some(1) {
        return Err(anyhow!(
            "private Windows ACL is not owner-bound: {} (descriptor_present={descriptor_present}, owner_present={owner_present}, dacl_present={dacl_present}, owner_matches={owner_matches}, dacl_protected={dacl_protected}, ace_count={ace_count:?}, GetSecurityDescriptorControl_error={control_error:?}, GetAclInformation_error={acl_information_error:?})",
            path.display()
        ));
    }
    let mut raw_ace = std::ptr::null_mut();
    // SAFETY: the ACL was validated to contain exactly one ACE.
    if unsafe { GetAce(acl, 0, &mut raw_ace) } == 0 {
        let error = std::io::Error::last_os_error();
        return Err(error)
            .with_context(|| format!("GetAce failed for private Windows ACL: {}", path.display()));
    }
    if raw_ace.is_null() {
        return Err(anyhow!(
            "GetAce returned a null entry for private Windows ACL: {}",
            path.display()
        ));
    }
    // SAFETY: GetAce returned the first ACE, whose fixed header is always readable.
    let ace = unsafe { &*(raw_ace.cast::<ACCESS_ALLOWED_ACE>()) };
    let ace_sid = (&ace.SidStart as *const u32).cast_mut().cast();
    // ACCESS_ALLOWED_ACE_TYPE is zero. Reject inherited, deny, or extra-principal entries.
    let allowed_ace = ace.Header.AceType == 0;
    let full_access = ace.Mask & FILE_ALL_ACCESS == FILE_ALL_ACCESS;
    // SAFETY: only an ACCESS_ALLOWED_ACE has the inline SID layout used here.
    let trustee_matches = allowed_ace && unsafe { EqualSid(ace_sid, current_user.sid) } != 0;
    if !allowed_ace || !full_access || !trustee_matches {
        return Err(anyhow!(
            "private Windows ACL grants access beyond the current user: {} (ace_type={}, full_access={full_access}, trustee_matches={trustee_matches})",
            path.display(),
            ace.Header.AceType
        ));
    }
    Ok(())
}

pub(crate) fn validate_windows_trusted_executable_acl(
    path: &Path,
    file: &File,
    directory: bool,
) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HANDLE, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, GetSecurityInfo, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation, IsWellKnownSid,
        OWNER_SECURITY_INFORMATION, PSID, WinBuiltinAdministratorsSid, WinLocalSystemSid,
    };

    let current_user = windows_current_user_sid()?;
    let mut owner: PSID = std::ptr::null_mut();
    let mut acl: *mut ACL = std::ptr::null_mut();
    let mut descriptor = std::ptr::null_mut();
    // SAFETY: all out-pointers are valid and the inspected handle remains open.
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle() as HANDLE,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            &mut acl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(status as i32))
            .with_context(|| format!("failed to read executable ACL for {}", path.display()));
    }
    let _descriptor = WindowsLocalAllocation(descriptor.cast());
    if owner.is_null() || acl.is_null() || descriptor.is_null() {
        return Err(anyhow!(
            "trusted Windows executable path must have an owner and a non-null DACL: {}",
            path.display()
        ));
    }
    if !windows_sid_is_trusted_owner(owner, current_user.sid) {
        return Err(anyhow!(
            "trusted Windows executable path has an untrusted owner: {}",
            path.display()
        ));
    }

    let mut acl_size = ACL_SIZE_INFORMATION::default();
    // SAFETY: `acl` belongs to the live descriptor allocation.
    if unsafe {
        GetAclInformation(
            acl,
            (&mut acl_size as *mut ACL_SIZE_INFORMATION).cast(),
            std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("failed to inspect executable ACL for {}", path.display()));
    }
    for index in 0..acl_size.AceCount {
        let mut raw_ace = std::ptr::null_mut();
        // SAFETY: the index is bounded by the ACL's reported ACE count.
        if unsafe { GetAce(acl, index, &mut raw_ace) } == 0 || raw_ace.is_null() {
            return Err(std::io::Error::last_os_error()).with_context(|| {
                format!(
                    "failed to inspect executable ACL entry for {}",
                    path.display()
                )
            });
        }
        // SAFETY: every ACE begins with ACE_HEADER followed by its type-specific mask.
        let header = unsafe { &*(raw_ace.cast::<windows_sys::Win32::Security::ACE_HEADER>()) };
        // INHERIT_ONLY_ACE does not grant rights on this object; an inherited copy is checked on
        // each concrete descendant in the canonical path chain.
        if header.AceFlags & 0x08 != 0 {
            continue;
        }
        let mask = unsafe { std::ptr::read_unaligned(raw_ace.cast::<u8>().add(4).cast::<u32>()) };
        match crate::trusted_executable::classify_windows_ace_mutation(
            header.AceType,
            mask,
            directory,
        ) {
            crate::trusted_executable::WindowsAceMutationAction::Ignore => continue,
            crate::trusted_executable::WindowsAceMutationAction::RejectUnrecognizedAllow => {
                return Err(anyhow!(
                    "trusted Windows executable ACL contains an unrecognized writable allow ACE: {}",
                    path.display()
                ));
            }
            crate::trusted_executable::WindowsAceMutationAction::InspectStandardAllowSid => {}
        }
        let ace = unsafe { &*(raw_ace.cast::<ACCESS_ALLOWED_ACE>()) };
        let sid = (&ace.SidStart as *const u32).cast_mut().cast();
        if !windows_sid_is_trusted_owner(sid, current_user.sid) {
            return Err(anyhow!(
                "trusted Windows executable ACL grants mutation rights to another principal: {}",
                path.display()
            ));
        }
    }

    fn windows_sid_is_trusted_owner(sid: PSID, current_user: PSID) -> bool {
        // SAFETY: both SID pointers originate from validated token/security descriptor data.
        if unsafe { EqualSid(sid, current_user) } != 0
            || unsafe { IsWellKnownSid(sid, WinLocalSystemSid) } != 0
            || unsafe { IsWellKnownSid(sid, WinBuiltinAdministratorsSid) } != 0
        {
            return true;
        }
        let mut raw = std::ptr::null_mut();
        // SAFETY: Windows allocates the returned null-terminated SID string with LocalAlloc.
        if unsafe { ConvertSidToStringSidW(sid, &mut raw) } == 0 || raw.is_null() {
            return false;
        }
        let mut length = 0usize;
        while unsafe { *raw.add(length) } != 0 {
            length = length.saturating_add(1);
        }
        let value = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(raw, length) });
        unsafe { LocalFree(raw.cast()) };
        value == "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464"
    }

    Ok(())
}

/// 현재 사용자 SID 의 문자열 형식(`S-1-5-21-...`)이다. named pipe 이름과 SDDL 에 쓴다.
pub(crate) fn windows_current_user_sid_string() -> Result<String> {
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;

    let current_user = windows_current_user_sid()?;
    let mut raw: *mut u16 = std::ptr::null_mut();
    // SAFETY: the SID stays alive in `current_user`; the output pointer is LocalAlloc'd by Windows.
    if unsafe { ConvertSidToStringSidW(current_user.sid, &mut raw) } == 0 {
        return Err(std::io::Error::last_os_error()).context("failed to format current-user SID");
    }
    let _allocation = WindowsLocalAllocation(raw.cast());
    // SAFETY: a successful call returns a NUL-terminated wide string.
    let length = unsafe { (0..).take_while(|&index| *raw.add(index) != 0).count() };
    // SAFETY: `length` UTF-16 units were just verified as readable.
    let units = unsafe { std::slice::from_raw_parts(raw, length) };
    String::from_utf16(units).context("current-user SID was not valid UTF-16")
}

/// 현재 사용자에게만 모든 권한을 주고 상속을 막은 보안 설명자이다 (`D:P(A;;GA;;;<SID>)`).
pub(crate) struct WindowsOwnerOnlySecurityDescriptor {
    descriptor: WindowsLocalAllocation,
}

impl WindowsOwnerOnlySecurityDescriptor {
    pub(crate) fn for_current_user() -> Result<Self> {
        Self::from_sddl(format!(
            "D:P(A;;GA;;;{})",
            windows_current_user_sid_string()?
        ))
    }

    pub(crate) fn for_current_user_directory() -> Result<Self> {
        let owner = windows_current_user_sid_string()?;
        // The owner and protected inheritable DACL exist before the directory is visible.
        // No other process needs to repair an ACL inherited during a concurrent creation.
        Self::from_sddl(format!("O:{owner}D:P(A;OICI;FA;;;{owner})"))
    }

    fn from_sddl(sddl: String) -> Result<Self> {
        use windows_sys::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
        };

        let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: `wide` is NUL-terminated; the descriptor is LocalAlloc'd and freed on drop.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error())
                .context("failed to build owner-only Windows security descriptor");
        }
        Ok(Self {
            descriptor: WindowsLocalAllocation(descriptor.cast()),
        })
    }

    /// `SECURITY_ATTRIBUTES` 를 만들 때 쓰는 원시 포인터이다. 이 값보다 오래 쓰면 안 된다.
    pub(crate) fn as_raw(&self) -> *mut std::ffi::c_void {
        self.descriptor.0.cast()
    }
}

// SAFETY: the descriptor is immutable after creation and only read by Windows APIs.
unsafe impl Send for WindowsOwnerOnlySecurityDescriptor {}
// SAFETY: see `Send`; no interior mutation happens through `as_raw`.
unsafe impl Sync for WindowsOwnerOnlySecurityDescriptor {}

#[cfg(test)]
mod shared_directory_tests {
    use super::*;

    #[test]
    fn shared_os_parent_accepts_inherited_acl_while_private_root_still_rejects_it() {
        use std::os::windows::fs::OpenOptionsExt;

        struct OwnedDirectory(std::path::PathBuf);
        impl Drop for OwnedDirectory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let path = std::env::temp_dir().join(format!(
            "akra-shared-directory-security-{:032x}",
            rand::random::<u128>()
        ));
        std::fs::create_dir(&path).unwrap();
        let directory = OwnedDirectory(path);
        let opened = std::fs::OpenOptions::new()
            .read(true)
            .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
            .share_mode(WINDOWS_FILE_SHARE_ALL)
            .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS)
            .open(&directory.0)
            .unwrap();
        let before = windows_file_identity_snapshot(&opened).unwrap();

        validate_windows_shared_directory_path_identity(&directory.0, &opened).unwrap();
        let error = validate_windows_shared_private_directory(&directory.0, &opened).unwrap_err();
        assert!(
            format!("{error:#}").contains("dacl_protected=false"),
            "an inherited OS-parent ACL must not satisfy the private-root policy"
        );
        assert_eq!(
            before,
            windows_file_identity_snapshot(&opened).unwrap(),
            "both shared-parent policies must be validation-only"
        );
    }

    fn directory_snapshot() -> WindowsFileIdentitySnapshot {
        WindowsFileIdentitySnapshot {
            attributes: WINDOWS_FILE_ATTRIBUTE_DIRECTORY,
            volume_serial_number: 17,
            file_index: 29,
            number_of_links: 1,
            creation_time: 31,
            size: 0,
            last_write_time: 37,
            change_time: 41,
        }
    }

    #[test]
    fn shared_directory_allows_content_size_and_timestamps_but_keeps_identity_fields_strict() {
        let expected = directory_snapshot();
        for size in [0, 4096, 8192] {
            let current = WindowsFileIdentitySnapshot {
                size,
                last_write_time: 43,
                change_time: 47,
                ..expected
            };
            assert!(validate_windows_shared_directory_snapshots(expected, current).is_ok());
            assert_ne!(expected, current, "ordinary snapshots remain fully strict");

            type IdentityMutation = fn(&mut WindowsFileIdentitySnapshot);
            let mutations: [(&str, IdentityMutation); 5] = [
                ("volume_changed", |value| value.volume_serial_number += 1),
                ("file_id_changed", |value| value.file_index += 1),
                ("creation_time_changed", |value| value.creation_time += 1),
                ("link_count_changed", |value| value.number_of_links += 1),
                ("attributes_changed", |value| value.attributes |= 0x2),
            ];
            for (field, mutate) in mutations {
                let mut changed = current;
                mutate(&mut changed);
                let diagnostic = validate_windows_shared_directory_snapshots(expected, changed)
                    .unwrap_err()
                    .to_string();
                assert!(
                    diagnostic.contains(&format!("{field}=true")),
                    "{diagnostic}"
                );
            }
        }
    }

    #[test]
    fn shared_directory_rejects_non_directories_and_reparse_points_in_either_snapshot() {
        let valid = directory_snapshot();
        for attributes in [
            0,
            WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT,
            WINDOWS_FILE_ATTRIBUTE_DIRECTORY | WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT,
        ] {
            let invalid = WindowsFileIdentitySnapshot {
                attributes,
                ..valid
            };
            assert!(validate_windows_shared_directory_snapshots(valid, invalid).is_err());
            assert!(validate_windows_shared_directory_snapshots(invalid, valid).is_err());
            assert!(validate_windows_shared_directory_snapshots(invalid, invalid).is_err());
        }
    }
}
