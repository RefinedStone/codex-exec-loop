use std::fs::File;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
use windows_sys::Win32::Security::{
    DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION, GetSecurityDescriptorControl,
    GetSecurityDescriptorLength, IsValidSecurityDescriptor, OWNER_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, SE_SELF_RELATIVE,
};

use crate::private_fs::{
    WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
    WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ, WINDOWS_READ_CONTROL,
    WindowsFileIdentitySnapshot, validate_windows_ancestor_directory_identity,
    validate_windows_owner, windows_file_identity_snapshot,
};

// These anchors are only for the directories leading to the Telegram config. The final
// file still uses the complete version snapshot and protected single-owner ACL checks.
pub(super) struct WindowsTelegramConfigParentAnchor {
    path: PathBuf,
    file: File,
    identity: WindowsFileIdentitySnapshot,
    security: Vec<u8>,
    require_current_user_owner: bool,
}

impl WindowsTelegramConfigParentAnchor {
    fn open(path: &Path, require_current_user_owner: bool) -> Result<Self> {
        let file = open_directory(path)?;
        let identity = windows_file_identity_snapshot(&file)?;
        validate_windows_ancestor_directory_identity(identity, &file)
            .context("Windows Telegram config parent must not be a junction or reparse point")?;
        let security = directory_security_snapshot(&file)?;
        let anchor = Self {
            path: path.to_path_buf(),
            file,
            identity,
            security,
            require_current_user_owner,
        };
        anchor.validate()?;
        Ok(anchor)
    }

    pub(super) fn validate(&self) -> Result<()> {
        self.validate_handle(&self.file)?;
        let reopened = open_directory(&self.path)?;
        self.validate_handle(&reopened)?;
        // Security queries may race a path mutation. Check the original handle again
        // without treating unrelated new/deleted directory entries as replacement.
        self.validate_handle(&self.file)
    }

    fn validate_handle(&self, file: &File) -> Result<()> {
        validate_windows_ancestor_directory_identity(self.identity, file).with_context(|| {
            format!(
                "Windows Telegram config parent changed or became a reparse point: {}",
                self.path.display()
            )
        })?;
        if directory_security_snapshot(file)? != self.security {
            bail!(
                "Windows Telegram config parent security changed while reading: {}",
                self.path.display()
            );
        }
        if self.require_current_user_owner {
            validate_windows_owner(&self.path, file)
                .context("Windows Telegram config parent must be owned by the current user")?;
        }
        validate_windows_ancestor_directory_identity(self.identity, file)
            .context("Windows Telegram config parent changed during security verification")
    }
}

pub(super) fn open_windows_telegram_config_parent_anchors(
    path: &Path,
) -> Result<Vec<WindowsTelegramConfigParentAnchor>> {
    let parent = path
        .parent()
        .context("Windows Telegram config path has no parent")?;
    let mut paths = parent.ancestors().collect::<Vec<_>>();
    paths.reverse();
    paths
        .into_iter()
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| WindowsTelegramConfigParentAnchor::open(path, path == parent))
        .collect()
}

fn open_directory(path: &Path) -> Result<File> {
    std::fs::OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .with_context(|| {
            format!(
                "failed to securely open Windows Telegram config parent {}",
                path.display()
            )
        })
}

fn directory_security_snapshot(file: &File) -> Result<Vec<u8>> {
    struct SecurityDescriptorAllocation(PSECURITY_DESCRIPTOR);
    impl Drop for SecurityDescriptorAllocation {
        fn drop(&mut self) {
            // SAFETY: GetSecurityInfo allocates this descriptor with LocalAlloc.
            unsafe { windows_sys::Win32::Foundation::LocalFree(self.0.cast()) };
        }
    }

    let mut descriptor = std::ptr::null_mut();
    // READ_CONTROL permits owner, group and DACL queries without the privileged SACL.
    // SAFETY: the handle is open and the descriptor out-pointer remains valid.
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle() as HANDLE,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(status as i32))
            .context("failed to read Windows Telegram config parent security");
    }
    let _allocation = SecurityDescriptorAllocation(descriptor);
    // SAFETY: a non-null successful GetSecurityInfo result belongs to the live allocation.
    if descriptor.is_null() || unsafe { IsValidSecurityDescriptor(descriptor) } == 0 {
        bail!("Windows Telegram config parent security descriptor is invalid");
    }
    let mut control = 0u16;
    let mut revision = 0u32;
    // SAFETY: the validated descriptor and both output values remain live for the call.
    if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
        return Err(std::io::Error::last_os_error())
            .context("failed to inspect Windows Telegram config parent security control");
    }
    if control & SE_SELF_RELATIVE == 0 {
        bail!("Windows Telegram config parent security descriptor must be self-relative");
    }
    // SAFETY: IsValidSecurityDescriptor checked the descriptor; self-relative descriptors
    // contain their complete SID/ACL data in the same allocation rather than raw pointers.
    let length = unsafe { GetSecurityDescriptorLength(descriptor) } as usize;
    if length == 0 {
        bail!("Windows Telegram config parent security descriptor is empty");
    }
    // Copy only the documented descriptor length while the LocalAlloc buffer is alive.
    // Do not log the descriptor bytes or SIDs.
    // SAFETY: GetSecurityDescriptorLength establishes this live self-relative buffer's extent.
    Ok(unsafe { std::slice::from_raw_parts(descriptor.cast::<u8>(), length) }.to_vec())
}
