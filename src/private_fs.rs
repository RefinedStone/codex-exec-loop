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

    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_LocalAppData, SHGetKnownFolderPath};

    let mut raw_path = std::ptr::null_mut();
    let status = unsafe {
        SHGetKnownFolderPath(
            &FOLDERID_LocalAppData,
            0,
            std::ptr::null_mut(),
            &mut raw_path,
        )
    };
    if status < 0 || raw_path.is_null() {
        return Err(anyhow!(
            "failed to resolve Windows LocalAppData known folder (HRESULT {status:#x})"
        ));
    }
    let mut length = 0usize;
    while unsafe { *raw_path.add(length) } != 0 {
        length = length
            .checked_add(1)
            .context("Windows LocalAppData path length overflowed")?;
    }
    let path =
        std::ffi::OsString::from_wide(unsafe { std::slice::from_raw_parts(raw_path, length) });
    unsafe { CoTaskMemFree(raw_path.cast()) };
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
        return Err(std::io::Error::from_raw_os_error(status as i32))
            .with_context(|| format!("failed to verify Windows security for {}", path.display()));
    }
    let _descriptor = WindowsLocalAllocation(descriptor.cast());
    let mut control = 0u16;
    let mut revision = 0u32;
    let mut acl_size = ACL_SIZE_INFORMATION::default();
    // SAFETY: owner, descriptor and ACL all belong to the live security descriptor allocation.
    let header_is_valid = !owner.is_null()
        && !acl.is_null()
        && !descriptor.is_null()
        && unsafe { EqualSid(owner, current_user.sid) } != 0
        && unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } != 0
        && control & SE_DACL_PROTECTED != 0
        && unsafe {
            GetAclInformation(
                acl,
                (&mut acl_size as *mut ACL_SIZE_INFORMATION).cast(),
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        } != 0
        && acl_size.AceCount == 1;
    if !header_is_valid {
        return Err(anyhow!(
            "private Windows ACL is not owner-bound: {}",
            path.display()
        ));
    }
    let mut raw_ace = std::ptr::null_mut();
    // SAFETY: the ACL was validated to contain exactly one ACE.
    if unsafe { GetAce(acl, 0, &mut raw_ace) } == 0 || raw_ace.is_null() {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("failed to inspect Windows ACL for {}", path.display()));
    }
    // SAFETY: GetAce returned the first ACE, whose fixed header is always readable.
    let ace = unsafe { &*(raw_ace.cast::<ACCESS_ALLOWED_ACE>()) };
    let ace_sid = (&ace.SidStart as *const u32).cast_mut().cast();
    // ACCESS_ALLOWED_ACE_TYPE is zero. Reject inherited, deny, or extra-principal entries.
    if ace.Header.AceType != 0
        || ace.Mask & FILE_ALL_ACCESS != FILE_ALL_ACCESS
        // SAFETY: SidStart is the documented inline SID start for ACCESS_ALLOWED_ACE.
        || unsafe { EqualSid(ace_sid, current_user.sid) } == 0
    {
        return Err(anyhow!(
            "private Windows ACL grants access beyond the current user: {}",
            path.display()
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
